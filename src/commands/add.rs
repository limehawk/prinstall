//! The `add` command — install a network or USB printer.
//!
//! Flow:
//! 1. Resolve the printer model (via `--model` or SNMP).
//! 2. Auto-pick a driver (via matcher) unless `--driver` overrides.
//! 3. Attempt to download + stage the driver if not already present.
//! 4. Run the standard three-step install (Add-PrinterPort → Add-PrinterDriver → Add-Printer).
//! 5. If that fails AND the target printer has IPP (port 631) open, fall back to
//!    `Microsoft IPP Class Driver` via `Add-Printer -ConnectionName`. The user gets a
//!    clearly-marked warning that this is a generic fallback and vendor-specific
//!    features (duplex, trays, finishing) may not be available.

use std::time::Duration;

use crate::core::executor::{PsExecutor, RealExecutor};
use crate::installer::powershell::escape_ps_string;
use crate::models::{InstallDetail, PrinterOpResult};
use crate::{discovery, drivers, installer};

/// Arguments for `prinstall add <ip>`.
pub struct AddArgs<'a> {
    pub ip: &'a str,
    pub driver_override: Option<&'a str>,
    pub name_override: Option<&'a str>,
    pub model_override: Option<&'a str>,
    pub usb: bool,
    pub community: &'a str,
    pub verbose: bool,
}

/// Run the `add` command.
///
/// Note: this function does NOT take a `PsExecutor` argument. The primary
/// install pipeline (`installer::install_printer`) currently calls PowerShell
/// through the legacy free functions, not the executor trait — threading the
/// executor through the installer module is future work. Only the IPP fallback
/// path uses the executor abstraction (for unit testability). Exposing an
/// executor parameter on this function would be a lying API: callers would
/// think they're mocking the whole flow when they're only mocking the fallback.
pub async fn run(args: AddArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;

    if verbose {
        eprintln!("[add] Checking reachability of {}...", args.ip);
    }

    // ── Parse IP ──────────────────────────────────────────────────────────────
    let addr: std::net::Ipv4Addr = match args.ip.parse() {
        Ok(a) => a,
        Err(e) => {
            return PrinterOpResult::err(format!("invalid IP address '{}': {e}", args.ip));
        }
    };

    // ── Step 1: resolve the printer model ────────────────────────────────────
    let model = if let Some(m) = args.model_override {
        m.to_string()
    } else {
        match discovery::snmp::identify_printer(addr, args.community, verbose).await {
            Some(p) => match p.model {
                Some(m) => m,
                None => {
                    return PrinterOpResult::err(format!(
                        "SNMP responded at {} but no model string. Use --model '...' to specify manually.",
                        args.ip
                    ));
                }
            },
            None => {
                return PrinterOpResult::err(format!(
                    "Could not identify printer at {} via SNMP. Check that SNMP is enabled, or use --model to bypass.",
                    args.ip
                ));
            }
        }
    };

    // ── Step 2: resolve the driver ────────────────────────────────────────────
    // Fetch the local driver store once and reuse it for both matching and the
    // "already staged?" check inside stage_driver_if_needed.
    let local_drivers = drivers::local_store::list_drivers(verbose);

    let driver_name = if let Some(d) = args.driver_override {
        d.to_string()
    } else {
        let results = drivers::matcher::match_drivers(&model, &local_drivers);
        match results.matched.first().or(results.universal.first()) {
            Some(best) => {
                if verbose {
                    eprintln!("[add] Auto-selected driver: {}", best.name);
                }
                best.name.clone()
            }
            None => {
                return PrinterOpResult::err(format!(
                    "No drivers found for '{model}'. Try --driver to specify one manually."
                ));
            }
        }
    };

    let printer_name = args.name_override.unwrap_or(&model).to_string();

    if verbose {
        eprintln!(
            "[add] Installing: printer='{printer_name}', driver='{driver_name}', ip={}",
            args.ip
        );
    }

    // ── Step 3: stage driver if not in local store ────────────────────────────
    if !args.usb {
        stage_driver_if_needed(&driver_name, &model, &local_drivers, verbose).await;
    }

    // ── Step 4: run the standard three-step install ───────────────────────────
    let primary_result = if args.usb {
        installer::update_printer_driver(&printer_name, &driver_name, &model, verbose)
    } else {
        installer::install_printer(args.ip, &driver_name, &printer_name, &model, verbose)
    };

    if primary_result.success {
        return primary_result;
    }

    // ── Step 5: IPP Class Driver fallback for network printers ────────────────
    if args.usb {
        return primary_result;
    }
    if !ipp_reachable(args.ip).await {
        if verbose {
            eprintln!("[add] Primary install failed and port 631 not reachable — no fallback available.");
        }
        return primary_result;
    }
    if verbose {
        eprintln!("[add] Primary install failed. Port 631 is open — attempting IPP Class Driver fallback.");
    }
    let executor = RealExecutor::new(verbose);
    try_ipp_fallback(&executor, args.ip, &driver_name, &model, verbose)
}

/// Look up the driver in the manifest and attempt to download + stage it.
/// Non-fatal — warnings are logged but the install proceeds regardless.
///
/// `local_drivers` is the already-fetched `Get-PrinterDriver` list from the
/// caller — avoids a second PowerShell round-trip.
async fn stage_driver_if_needed(
    driver_name: &str,
    model: &str,
    local_drivers: &[String],
    verbose: bool,
) {
    if local_drivers.iter().any(|d| d == driver_name) {
        return;
    }

    if verbose {
        eprintln!("[add] Driver not in local store, checking manufacturer downloads...");
    }

    let manifest = drivers::manifest::Manifest::load_embedded();
    let Some(mfr) = manifest.find_manufacturer(model) else {
        return;
    };
    let Some(ud) = mfr.universal_drivers.iter().find(|u| u.name == driver_name) else {
        return;
    };

    match drivers::downloader::download_and_stage(ud, verbose).await {
        Ok(extract_dir) => {
            let infs = drivers::downloader::find_inf_files(&extract_dir);
            for inf in &infs {
                if verbose {
                    eprintln!("[add] Staging driver: {}", inf.display());
                }
                let stage_result = installer::powershell::stage_driver_inf(
                    inf.to_str().unwrap_or_default(),
                    verbose,
                );
                if !stage_result.success {
                    eprintln!(
                        "[add] Warning: failed to stage {}: {}",
                        inf.display(),
                        stage_result.stderr
                    );
                }
            }
        }
        Err(e) => {
            if verbose {
                eprintln!("[add] Download failed: {e}");
                eprintln!("[add] Proceeding anyway — will fall back to IPP if available.");
            }
        }
    }
}

/// Check whether port 631 (IPP) is open on the target. Short timeout — this is
/// a fallback eligibility check, not a full scan.
async fn ipp_reachable(ip: &str) -> bool {
    let addr = format!("{ip}:631");
    tokio::time::timeout(
        Duration::from_millis(1500),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// Install the printer using Microsoft's built-in `IPP Class Driver`.
///
/// Uses the explicit port + driver approach, which is the most reliable path
/// on modern Windows: the `IP_<ip>` port was already created during the
/// primary install's port-creation step, and `Microsoft IPP Class Driver`
/// is pre-registered on Windows 8+ so no driver staging is needed. One
/// `Add-Printer` call with explicit `-DriverName` is all that's required.
///
/// The resulting printer is named `<model> (IPP)` so it's obvious in
/// `Get-Printer` output that it's on the generic fallback driver. A visible
/// warning is stored on the result for audit trails.
pub(crate) fn try_ipp_fallback(
    executor: &dyn PsExecutor,
    ip: &str,
    attempted_driver: &str,
    model: &str,
    verbose: bool,
) -> PrinterOpResult {
    let port_name = format!("IP_{ip}");
    let printer_name = format!("{model} (IPP)");
    let ps = format!(
        "Add-Printer -Name '{}' -DriverName 'Microsoft IPP Class Driver' -PortName '{}'",
        escape_ps_string(&printer_name),
        escape_ps_string(&port_name),
    );
    if verbose {
        eprintln!("[add] IPP fallback: {ps}");
    }

    let result = executor.run(&ps);
    if !result.success {
        return PrinterOpResult::err(format!(
            "Primary install failed and IPP Class Driver fallback also failed: {}",
            result.stderr
        ));
    }

    // Record in history so the audit trail shows IPP fallback was used.
    crate::history::record_install(model, "Microsoft IPP Class Driver", "install_ipp_fallback");

    PrinterOpResult::ok(InstallDetail {
        printer_name,
        driver_name: "Microsoft IPP Class Driver".to_string(),
        port_name,
        warning: Some(format!(
            "Installed via Microsoft IPP Class Driver (generic fallback). \
             Basic printing should work, but vendor-specific features \
             (duplex modes, tray selection, finishing options) may not be available. \
             The matched driver '{attempted_driver}' was not in the local store \
             and could not be downloaded."
        )),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use crate::installer::powershell::PsResult;

    #[test]
    fn ipp_fallback_success_wraps_with_warning() {
        let mock = MockExecutor::new().stub_contains(
            "Microsoft IPP Class Driver",
            PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let result = try_ipp_fallback(
            &mock,
            "10.10.20.16",
            "Brother Universal Printer",
            "Brother MFC-L2750DW series",
            false,
        );
        assert!(result.success);
        let detail = result.detail_as::<InstallDetail>().expect("has detail");
        assert_eq!(detail.printer_name, "Brother MFC-L2750DW series (IPP)");
        assert_eq!(detail.driver_name, "Microsoft IPP Class Driver");
        assert_eq!(detail.port_name, "IP_10.10.20.16");
        let warning = detail.warning.expect("warning present");
        assert!(warning.contains("Microsoft IPP Class Driver"));
        assert!(warning.contains("generic fallback"));
        assert!(warning.contains("Brother Universal Printer"));
    }

    #[test]
    fn ipp_fallback_failure_returns_error_result() {
        let mock = MockExecutor::new().stub_failure(
            "Microsoft IPP Class Driver",
            "Access denied",
        );
        let result = try_ipp_fallback(&mock, "10.10.20.16", "Brother", "foo", false);
        assert!(!result.success);
        let err = result.error.expect("error present");
        assert!(err.contains("IPP Class Driver fallback also failed"));
        assert!(err.contains("Access denied"));
    }

    #[test]
    fn ipp_fallback_uses_existing_tcpip_port() {
        let mock = MockExecutor::new().stub_contains(
            "Microsoft IPP Class Driver",
            PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let result = try_ipp_fallback(&mock, "10.20.30.40", "foo", "HP LaserJet 9999", false);
        let detail = result.detail_as::<InstallDetail>().unwrap();
        assert_eq!(detail.port_name, "IP_10.20.30.40");
        assert_eq!(detail.printer_name, "HP LaserJet 9999 (IPP)");
    }
}

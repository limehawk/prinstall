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
use crate::core::ps_error;
use crate::installer::powershell::escape_ps_string;
use crate::models::{InstallDetail, PrinterOpResult};
use crate::{discovery, drivers, installer};

/// Arguments for `prinstall add <target>`.
///
/// `target` is an IPv4 address for network printers, or an existing Windows
/// printer queue name when `usb` is true.
pub struct AddArgs<'a> {
    pub target: &'a str,
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
    if args.usb {
        run_usb(args).await
    } else {
        run_network(args).await
    }
}

/// Network-printer install path: SNMP identify → driver match → three-step
/// Add-PrinterPort/Driver/Printer pipeline → IPP Class Driver fallback if
/// the primary install fails and port 631 is open.
async fn run_network(args: AddArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;
    let target = args.target;

    if verbose {
        eprintln!("[add] Network mode — checking reachability of {target}...");
    }

    let addr: std::net::Ipv4Addr = match target.parse() {
        Ok(a) => a,
        Err(e) => {
            return PrinterOpResult::err(format!(
                "invalid IP address '{target}': {e}. For USB printers, pass --usb and use the printer queue name as the target."
            ));
        }
    };

    // ── Step 1: resolve the printer model via SNMP ────────────────────────
    let model = if let Some(m) = args.model_override {
        m.to_string()
    } else {
        match discovery::snmp::identify_printer(addr, args.community, verbose).await {
            Some(p) => match p.model {
                Some(m) => m,
                None => {
                    return PrinterOpResult::err(format!(
                        "SNMP responded at {target} but no model string. Use --model '...' to specify manually."
                    ));
                }
            },
            None => {
                return PrinterOpResult::err(format!(
                    "Could not identify printer at {target} via SNMP. Check that SNMP is enabled, or use --model to bypass."
                ));
            }
        }
    };

    // ── Step 2: resolve the driver ────────────────────────────────────────
    let local_drivers = drivers::local_store::list_drivers(verbose);
    let driver_name = match resolve_driver(&args, &model, &local_drivers, verbose) {
        Ok(name) => name,
        Err(result) => return result,
    };
    let printer_name = args.name_override.unwrap_or(&model).to_string();

    if verbose {
        eprintln!(
            "[add] Installing: printer='{printer_name}', driver='{driver_name}', ip={target}"
        );
    }

    // ── Step 3: stage the driver if not in local store ───────────────────
    stage_driver_if_needed(&driver_name, &model, &local_drivers, verbose).await;

    // ── Step 4: three-step install ───────────────────────────────────────
    let primary_result =
        installer::install_printer(target, &driver_name, &printer_name, &model, verbose);

    if primary_result.success {
        return primary_result;
    }

    // ── Step 5: IPP Class Driver fallback ────────────────────────────────
    if !ipp_reachable(target).await {
        if verbose {
            eprintln!(
                "[add] Primary install failed and port 631 not reachable — no fallback available."
            );
        }
        return primary_result;
    }
    if verbose {
        eprintln!(
            "[add] Primary install failed. Port 631 is open — attempting IPP Class Driver fallback."
        );
    }
    let executor = RealExecutor::new(verbose);
    try_ipp_fallback(&executor, target, &driver_name, &model, verbose)
}

/// USB-printer install path: verify queue exists → driver match → stage
/// driver → swap via Set-Printer. No port creation, no SNMP, no IPP fallback.
async fn run_usb(args: AddArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;
    let target = args.target;

    if verbose {
        eprintln!("[add] USB mode — target queue: '{target}'");
    }

    // Verify the USB printer queue exists. Windows auto-creates a queue
    // via PnP when a USB printer is plugged in; we're swapping its driver,
    // not creating it from scratch.
    if !installer::powershell::printer_exists(target, verbose) {
        return PrinterOpResult::err(format!(
            "USB printer queue '{target}' not found. Run `prinstall list` to see installed printers."
        ));
    }

    // ── Model resolution: --model wins, otherwise use the queue name ─────
    // The queue name is typically the model string Windows assigned during
    // PnP install (e.g. "Brother MFC-L2750DW"), which is a reasonable input
    // for the matcher.
    let model = args
        .model_override
        .map(|m| m.to_string())
        .unwrap_or_else(|| target.to_string());

    // ── Driver resolution ────────────────────────────────────────────────
    let local_drivers = drivers::local_store::list_drivers(verbose);
    let driver_name = match resolve_driver(&args, &model, &local_drivers, verbose) {
        Ok(name) => name,
        Err(result) => return result,
    };

    if verbose {
        eprintln!("[add] Swapping driver on '{target}' → '{driver_name}'");
    }

    // ── Stage the driver if not in local store ───────────────────────────
    stage_driver_if_needed(&driver_name, &model, &local_drivers, verbose).await;

    // ── Call Set-Printer -DriverName ─────────────────────────────────────
    installer::update_printer_driver(target, &driver_name, &model, verbose)
}

/// Shared driver-resolution logic for both USB and network paths.
/// Uses `--driver` if provided, otherwise runs the matcher and picks
/// the top candidate.
fn resolve_driver(
    args: &AddArgs<'_>,
    model: &str,
    local_drivers: &[String],
    verbose: bool,
) -> Result<String, PrinterOpResult> {
    if let Some(d) = args.driver_override {
        return Ok(d.to_string());
    }
    let results = drivers::matcher::match_drivers(model, local_drivers);
    match results.matched.first().or(results.universal.first()) {
        Some(best) => {
            if verbose {
                eprintln!("[add] Auto-selected driver: {}", best.name);
            }
            Ok(best.name.clone())
        }
        None => Err(PrinterOpResult::err(format!(
            "No drivers found for '{model}'. Try --driver to specify one manually."
        ))),
    }
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
                        ps_error::clean(&stage_result.stderr)
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

    // Idempotency: if this IPP-fallback printer was already installed
    // (e.g. from a previous `add` run on the same IP), return success
    // without calling Add-Printer a second time. Matches the same pattern
    // powershell::add_printer uses for the primary install path.
    if installer::powershell::printer_exists(&printer_name, verbose) {
        if verbose {
            eprintln!(
                "[add] IPP fallback target '{printer_name}' already exists — treating as success"
            );
        }
        return PrinterOpResult::ok(InstallDetail {
            printer_name,
            driver_name: "Microsoft IPP Class Driver".to_string(),
            port_name,
            warning: Some(format!(
                "Printer already installed via Microsoft IPP Class Driver fallback. \
                 The matched driver '{attempted_driver}' is still not in the local \
                 store. No changes made."
            )),
        });
    }

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
            ps_error::clean(&result.stderr)
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

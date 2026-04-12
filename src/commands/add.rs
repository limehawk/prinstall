//! The `add` command — install a network or USB printer.
//!
//! Network flow:
//! 1. Resolve the printer model (via `--model` or SNMP).
//! 2. Probe IPP for the device ID (used by the catalog resolver if needed).
//! 3. Auto-pick a driver (via matcher) unless `--driver` overrides.
//! 4. Attempt to download + stage the driver if not already present.
//! 5. Run the standard three-step install (Add-PrinterPort → Add-PrinterDriver → Add-Printer).
//! 6. If that fails AND we have an IPP device ID, try the catalog resolver
//!    (search Microsoft Update Catalog by CID → download → INF HWID match →
//!    stage → retry install). Deterministic match on the `1284_CID_*` form.
//! 7. If the catalog resolver also fails AND port 631 is open, fall back to
//!    `Microsoft IPP Class Driver`. The user gets a clearly-marked warning
//!    that this is a generic fallback and vendor-specific features (duplex,
//!    trays, finishing) may not be available.

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
    pub no_sdi: bool,
    pub no_catalog: bool,
    pub sdi_fetch: bool,
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

    // ── Step 2: IPP device ID (for the catalog resolver's CID query) ─────
    // Best-effort — many printers speak IPP even when SNMP is flaky, so this
    // is the most reliable path for the deterministic catalog match. If the
    // printer doesn't advertise a CID, we still install; we just won't be
    // able to escalate to Tier 3 on failure.
    let device_id = discovery::ipp::query_ipp_attributes(addr, verbose)
        .await
        .device_id;
    if verbose {
        match &device_id {
            Some(d) => eprintln!("[add] IPP device ID: {d}"),
            None => eprintln!("[add] IPP device ID: (not advertised)"),
        }
    }

    // ── Step 3: resolve the driver ────────────────────────────────────────
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

    // ── Step 4: stage the driver if not in local store ───────────────────
    stage_driver_if_needed(&driver_name, &model, &local_drivers, verbose).await;

    // ── Step 5: three-step install ───────────────────────────────────────
    let primary_result =
        installer::install_printer(target, &driver_name, &printer_name, &model, verbose);

    if primary_result.success {
        return primary_result;
    }

    // ── Step 6: Catalog resolver (Tier 3 — Microsoft Update Catalog) ────
    // Runs BEFORE SDI because the Catalog is faster (~10 sec for a 4 MB
    // CAB download) than SDI's first-run solid-LZMA2 decompression (~5
    // min). When both sources carry the same driver, Catalog wins on
    // speed. SDI only fires when the Catalog comes up empty — its value
    // is coverage (Brother, Canon, Epson, Ricoh drivers the Catalog
    // doesn't reliably carry), not speed.
    if args.no_catalog {
        if verbose {
            eprintln!("[add] Catalog resolver disabled (--no-catalog). Skipping.");
        }
    } else if let Some(ref dev_id) = device_id {
        if verbose {
            eprintln!(
                "[add] Primary install failed. Trying catalog resolver with device ID..."
            );
        }
        match drivers::resolver::resolve_driver_for_device(dev_id, verbose).await {
            Ok(resolved) => {
                if verbose {
                    eprintln!(
                        "[add] Catalog resolver matched '{}' from '{}' — staging INF and retrying install.",
                        resolved.display_name, resolved.catalog_title
                    );
                }
                let inf_str = resolved.inf_path.to_string_lossy().to_string();
                let stage_result = installer::powershell::stage_driver_inf(&inf_str, verbose);
                if !stage_result.success {
                    if verbose {
                        eprintln!(
                            "[add] INF staging failed: {} — falling through to SDI resolver.",
                            ps_error::clean(&stage_result.stderr)
                        );
                    }
                } else {
                    let retry = installer::install_printer(
                        target,
                        &resolved.display_name,
                        &printer_name,
                        &model,
                        verbose,
                    );
                    if retry.success {
                        return annotate_catalog_success(retry, &resolved);
                    }
                    if verbose {
                        eprintln!(
                            "[add] Retry install with catalog driver failed — falling through to SDI resolver."
                        );
                    }
                }
            }
            Err(e) => {
                if verbose {
                    eprintln!("[add] Catalog resolver: {e}");
                }
            }
        }
    }

    // ── Step 6.5: SDI resolver (Tier 4 — Snappy Driver Installer) ───────
    // Runs AFTER the Catalog because SDI's first extraction from a solid
    // LZMA2 pack takes minutes. Once the extraction cache is warm,
    // subsequent SDI installs are instant — but the Catalog should get
    // first crack at drivers it carries. SDI covers the gaps: vendors
    // the Catalog misses entirely.
    if !args.no_sdi {
        if let Some(ref dev_id) = device_id {
            if let Ok(mut cache) = drivers::sdi::cache::SdiCache::load() {
                // Auto-register any .7z packs that exist on disk but aren't
                // in metadata yet (e.g., manually copied by the tech).
                let newly_registered = cache.auto_register_packs();
                if newly_registered > 0 && verbose {
                    eprintln!("[sdi] Auto-registered {newly_registered} pack(s) from sdi/drivers/");
                }
                let candidates = drivers::sdi::resolver::enumerate_candidates(dev_id, &cache);
                if let Some(best) = pick_sdi_candidate(&candidates, args.sdi_fetch) {
                    if verbose {
                        eprintln!(
                            "[sdi] SDI match found: '{}' ({:?})",
                            best.driver_name,
                            best.source
                        );
                    }
                    match try_sdi_install(best, target, &printer_name, &model, verbose) {
                        Some(result) => return result,
                        None => {
                            if verbose {
                                eprintln!("[sdi] SDI install did not succeed — falling through to IPP fallback.");
                            }
                        }
                    }
                } else if !candidates.is_empty() && verbose {
                    // We have matches but all are uncached and --sdi-fetch wasn't set
                    eprintln!(
                        "[sdi] SDI has {} match(es) but the pack is not cached.",
                        candidates.len()
                    );
                    eprintln!(
                        "[sdi] Run `prinstall sdi prefetch` to pre-cache, or re-run with --sdi-fetch."
                    );
                }
            }
        }
    }

    // ── Step 7: IPP Class Driver fallback ────────────────────────────────
    if !ipp_reachable(target).await {
        if verbose {
            eprintln!(
                "[add] Catalog resolver did not succeed and port 631 not reachable — no fallback available."
            );
        }
        return primary_result;
    }
    if verbose {
        eprintln!(
            "[add] Falling back to Microsoft IPP Class Driver (port 631 open)."
        );
    }
    let executor = RealExecutor::new(verbose);
    try_ipp_fallback(&executor, target, &driver_name, &model, verbose)
}

/// Attach a catalog-success note to an otherwise-successful install result
/// so the CLI output and JSON payload show which tier actually landed the
/// driver. We overwrite the `warning` field because this message is an
/// informational breadcrumb, not a warning about degraded functionality.
fn annotate_catalog_success(
    mut result: PrinterOpResult,
    resolved: &drivers::resolver::ResolvedDriver,
) -> PrinterOpResult {
    if let Some(mut detail) = result.detail_as::<InstallDetail>() {
        let ver = resolved
            .driver_ver
            .as_deref()
            .map(|v| format!(" (DriverVer {v})"))
            .unwrap_or_default();
        detail.warning = Some(format!(
            "Installed via Microsoft Update Catalog: '{}' from '{}'{ver}. \
             Matched HWID: {}.",
            resolved.display_name,
            resolved.catalog_title,
            resolved.matched_hwid,
        ));
        // Re-wrap with the updated detail. Ignore serialization failures —
        // the install itself still succeeded so keep the original result.
        if let Ok(value) = serde_json::to_value(&detail) {
            result.detail = value;
        }
    }
    result
}

/// Pick the best SDI candidate from the enumeration. Prefers cached packs
/// (free, no download). Only considers uncached packs if `allow_uncached`
/// is true (`--sdi-fetch` flag).
fn pick_sdi_candidate<'a>(
    candidates: &'a [drivers::sources::SourceCandidate],
    allow_uncached: bool,
) -> Option<&'a drivers::sources::SourceCandidate> {
    // Prefer cached — effectively free, no network, no prompt.
    if let Some(c) = candidates
        .iter()
        .find(|c| c.source == drivers::sources::Source::SdiCached)
    {
        return Some(c);
    }
    // Uncached only if explicitly allowed via --sdi-fetch.
    if allow_uncached {
        candidates
            .iter()
            .find(|c| c.source == drivers::sources::Source::SdiUncached)
    } else {
        None
    }
}

/// Attempt to install a printer using a matched SDI candidate. Returns
/// `Some(result)` on success (install completed with SDI-annotated
/// result), `None` if the SDI path failed at any step (extraction,
/// staging, or retry install) and the caller should fall through to the
/// next tier.
fn try_sdi_install(
    candidate: &drivers::sources::SourceCandidate,
    target: &str,
    printer_name: &str,
    model: &str,
    verbose: bool,
) -> Option<PrinterOpResult> {
    let (pack_path, inf_dir_prefix, inf_filename) = match &candidate.install_hint {
        drivers::sources::InstallHint::SdiCached {
            pack_path,
            inf_dir_prefix,
            inf_filename,
        } => (pack_path.clone(), inf_dir_prefix.clone(), inf_filename.clone()),
        // SdiUncached would need fetcher calls here. Deferred until the
        // SDI mirror is published — for now only cached packs work.
        _ => {
            if verbose {
                eprintln!("[sdi] Skipping non-cached SDI candidate (pack fetch not yet implemented).");
            }
            return None;
        }
    };

    // Extract the driver's subdirectory from the cached pack. Uses a
    // PERSISTENT extraction cache under sdi/extracted/<pack_stem>/ so
    // the slow solid-LZMA2 decompression only happens once per pack.
    // Subsequent installs from the same pack (same or different driver)
    // read directly from the extracted tree — sub-second, no 7z touch.
    let pack_stem = pack_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let extract_dir = crate::paths::sdi_dir().join("extracted").join(pack_stem);

    // Check if this driver was already extracted in a previous run.
    // The persistent extraction cache means the slow 7z decompression
    // only happens once per pack — every subsequent install reads from
    // the extracted tree directly.
    let mut cached_inf = extract_dir.clone();
    for seg in inf_dir_prefix.split('/').filter(|s| !s.is_empty()) {
        cached_inf.push(seg);
    }
    cached_inf.push(&inf_filename);

    let extracted_inf = if cached_inf.is_file() {
        if verbose {
            eprintln!(
                "[sdi] Using cached extraction: {}",
                cached_inf.display()
            );
        }
        cached_inf
    } else {
        if verbose {
            eprintln!(
                "[sdi] Extracting {}{} from {} (first run — this takes a few minutes for solid LZMA2 packs)",
                inf_dir_prefix,
                inf_filename,
                pack_path.display()
            );
        }
        match drivers::sdi::pack::extract_driver_directory(
            &pack_path,
            &inf_dir_prefix,
            &inf_filename,
            &extract_dir,
        ) {
            Ok(p) => p,
            Err(e) => {
                if verbose {
                    eprintln!("[sdi] Extraction failed: {e}");
                }
                return None;
            }
        }
    };

    // Stage the extracted INF + siblings via pnputil /add-driver
    let inf_str = extracted_inf.to_string_lossy().to_string();
    if verbose {
        eprintln!("[sdi] Staging INF: {inf_str}");
    }
    let stage = installer::powershell::stage_driver_inf(&inf_str, verbose);
    if !stage.success {
        if verbose {
            eprintln!(
                "[sdi] INF staging failed: {} — falling through.",
                ps_error::clean(&stage.stderr)
            );
        }
        return None;
    }

    // Retry the three-step install with the SDI-resolved driver name.
    let retry = installer::install_printer(
        target,
        &candidate.driver_name,
        printer_name,
        model,
        verbose,
    );
    if retry.success {
        Some(annotate_sdi_success(retry, candidate))
    } else {
        if verbose {
            eprintln!("[sdi] Retry install with SDI driver failed — falling through.");
        }
        None
    }
}

/// Attach an SDI-success note to an otherwise-successful install result
/// so the CLI output and JSON payload show that the SDI tier landed the
/// driver. Mirrors the shape of [`annotate_catalog_success`].
fn annotate_sdi_success(
    mut result: PrinterOpResult,
    candidate: &drivers::sources::SourceCandidate,
) -> PrinterOpResult {
    if let Some(mut detail) = result.detail_as::<InstallDetail>() {
        let ver = candidate
            .driver_version
            .as_deref()
            .map(|v| format!(" (DriverVer {v})"))
            .unwrap_or_default();
        let provider = candidate
            .provider
            .as_deref()
            .unwrap_or("unknown");
        detail.warning = Some(format!(
            "Installed via SDI: '{}' from {} [{:?}]{ver}.",
            candidate.driver_name,
            provider,
            candidate.source,
        ));
        if let Ok(value) = serde_json::to_value(&detail) {
            result.detail = value;
        }
    }
    result
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

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

use std::time::{Duration, Instant};

use crate::core::executor::{PsExecutor, RealExecutor};
use crate::core::ps_error;
use crate::installer::powershell::escape_ps_string;
use crate::models::{InstallDetail, PrinterOpResult};
use crate::verbose::{InstallReport, TierStatus};
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
    pub force: bool,
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
///
/// Always renders a structured [`InstallReport`] to stderr showing the
/// Discovery → Resolution → Install → Summary phases. Raw PS commands
/// and implementation details only appear with `--verbose`.
async fn run_network(args: AddArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;
    let target = args.target;
    let start = Instant::now();
    let mut report = InstallReport::new(target);

    let addr: std::net::Ipv4Addr = match target.parse() {
        Ok(a) => a,
        Err(e) => {
            return PrinterOpResult::err(format!(
                "invalid IP address '{target}': {e}. For USB printers, pass --usb and use the printer queue name as the target."
            ));
        }
    };

    // ── Early check: printer already installed at this IP? ──────────────
    let port_name = format!("IP_{target}");
    if let Some(existing_queue) = installer::powershell::find_printer_on_port(&port_name, verbose) {
        if args.force {
            if verbose {
                eprintln!("[add] Printer already installed as '{existing_queue}' — removing before reinstall");
            }
            let executor = RealExecutor::new(verbose);
            let remove_result = crate::commands::remove::run(
                &executor,
                crate::commands::remove::RemoveArgs {
                    target: &existing_queue,
                    keep_driver: false,
                    keep_port: true, // keep the port — we're about to reuse it
                    verbose,
                },
            ).await;
            if !remove_result.success {
                return PrinterOpResult::err(format!(
                    "Failed to remove existing printer '{}' before reinstall: {}",
                    existing_queue,
                    remove_result.error.unwrap_or_default()
                ));
            }
        } else {
            return PrinterOpResult::err(format!(
                "Printer already installed at {target} as '{existing_queue}'. Use --force to reinstall."
            ));
        }
    }

    // ── Step 1: resolve the printer model via SNMP ────────────────────────
    let model = if let Some(m) = args.model_override {
        report.discovery.snmp_model = Some(m.to_string());
        m.to_string()
    } else {
        match discovery::snmp::identify_printer(addr, args.community, verbose).await {
            Some(p) => match p.model {
                Some(m) => {
                    report.discovery.snmp_model = Some(m.clone());
                    m
                }
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
    let ipp_attrs = discovery::ipp::query_ipp_attributes(addr, verbose).await;
    let device_id = ipp_attrs.device_id;
    report.discovery.ipp_model = ipp_attrs.make_and_model;
    report.discovery.device_id = device_id.clone();

    // Extract CID from the device ID if present
    if let Some(ref did) = device_id {
        if let Some(cid) = extract_cid(did) {
            report.discovery.ipp_cid = Some(cid);
        }
    }

    // ── Step 3: resolve the driver ────────────────────────────────────────
    let local_drivers = drivers::local_store::list_drivers(verbose);
    let driver_name = match resolve_driver(&args, &model, &local_drivers, verbose) {
        Ok(name) => name,
        Err(result) => return result,
    };
    let printer_name = args.name_override.unwrap_or(&model).to_string();

    // Track whether the auto-selected driver came from local store
    let driver_is_local = local_drivers.iter().any(|d| d == &driver_name);

    // ── Step 4: stage the driver if not in local store ───────────────────
    stage_driver_if_needed(&driver_name, &model, &local_drivers, verbose).await;

    // ── Step 5: three-step install ───────────────────────────────────────
    let primary_result =
        installer::install_printer(target, &driver_name, &printer_name, &model, verbose);

    if primary_result.success {
        // Populate the report for the happy path
        if driver_is_local {
            report.resolution.add_tier("Local store", TierStatus::Matched, &driver_name);
        } else {
            report.resolution.add_tier("Manufacturer", TierStatus::Matched, &driver_name);
        }
        populate_install_steps(&mut report, target, &driver_name, &printer_name, true);
        report.source_annotation = Some(if driver_is_local { "local driver store".into() } else { "manufacturer".into() });
        report.success = true;
        report.elapsed = start.elapsed();
        report.render();
        return primary_result;
    }

    // Primary failed — record the tier as failed
    if driver_is_local {
        report.resolution.add_tier("Local store", TierStatus::Failed, "install failed");
    } else {
        report.resolution.add_tier("Manufacturer", TierStatus::Failed, "install failed");
    }

    // ── Step 6: Catalog resolver (Tier 3 — Microsoft Update Catalog) ────
    //
    // Catalog-downloaded CABs get the same Authenticode gate as the SDI
    // tier (when built with `--features sdi`, which is the default). The
    // resolver extracts the CAB to `staging/catalog/<guid>-<idx>/`; we
    // find `.cat` files beside the matched INF and require every catalog
    // to carry a trusted signature before handing the driver to the
    // installer. Unsigned or tampered packs are skipped — the pipeline
    // falls through to SDI and then IPP Class Driver.
    //
    // In the `--no-default-features` (no-SDI) lean build, the
    // `sdi_verify` module isn't compiled in, so we keep the pre-v0.4.3
    // ungated behavior for that variant. The SDI-default build is where
    // Watson wants defense-in-depth anyway.
    if args.no_catalog {
        report.resolution.add_tier("Catalog", TierStatus::Disabled, "--no-catalog");
    } else if let Some(ref dev_id) = device_id {
        match drivers::resolver::resolve_driver_for_device(dev_id, verbose).await {
            Ok(resolved) => {
                // The extraction root holds both the .inf and the .cat files
                // that sign it. Walking upward from the matched INF to its
                // parent directory covers the typical layout where Windows
                // driver packages keep catalogs alongside their INFs.
                let cab_dir = resolved
                    .inf_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| resolved.inf_path.clone());

                if catalog_pack_safe_to_install(&cab_dir, verbose, &mut report) {
                    let signer_tag = catalog_signer_tag(&cab_dir, verbose);
                    let inf_str = resolved.inf_path.to_string_lossy().to_string();
                    let stage_result = installer::powershell::stage_driver_inf(&inf_str, verbose);
                    if stage_result.success {
                        let retry = installer::install_printer(
                            target,
                            &resolved.display_name,
                            &printer_name,
                            &model,
                            verbose,
                        );
                        if retry.success {
                            let (status, detail) = catalog_success_tier(&resolved, signer_tag.as_deref());
                            report.resolution.add_tier("Catalog", status, &detail);
                            populate_install_steps(&mut report, target, &resolved.display_name, &printer_name, true);
                            report.source_annotation = Some(catalog_source_annotation(signer_tag.as_deref()));
                            report.success = true;
                            report.elapsed = start.elapsed();
                            report.render();
                            return annotate_catalog_success(retry, &resolved);
                        }
                        report.resolution.add_tier("Catalog", TierStatus::Failed, "driver staged but install failed");
                    } else {
                        report.resolution.add_tier("Catalog", TierStatus::Failed,
                            &format!("staging failed: {}", stage_result.error_summary()));
                    }
                }
                // If the pack failed verification, `catalog_pack_safe_to_install`
                // already recorded the Failed tier with a descriptive reason —
                // fall through to the next tier.
            }
            Err(e) => {
                report.resolution.add_tier("Catalog", TierStatus::Failed, &e);
            }
        }
    } else {
        report.resolution.add_tier("Catalog", TierStatus::Skipped, "no device ID for CID query");
    }

    // ── Step 6.5: SDI resolver (Tier 4 — Snappy Driver Installer) ───────
    //
    // Cached candidates are gated by Authenticode verification: only packs
    // whose `.cat` catalogs all pass `Get-AuthenticodeSignature` install.
    // Unsigned, invalid, or catalog-less packs are skipped — the flow falls
    // through to the IPP Class Driver fallback instead.
    //
    // Uncached (`--sdi-fetch`) candidates install WITHOUT verification for
    // now; threading the verify gate through post-fetch extraction is
    // future work. Those installs are tagged `UNVERIFIED` in the report.
    #[cfg(feature = "sdi")]
    if args.no_sdi {
        report.resolution.add_tier("SDI Origin", TierStatus::Disabled, "--no-sdi");
    } else if let Some(ref dev_id) = device_id {
        if let Ok(mut cache) = drivers::sdi::cache::SdiCache::load() {
            let newly_registered = cache.auto_register_packs();
            if newly_registered > 0 && verbose {
                eprintln!("[sdi] Auto-registered {newly_registered} pack(s) from sdi/drivers/");
            }
            let candidates = drivers::sdi::resolver::enumerate_candidates(dev_id, &cache);
            if let Some(best) = pick_sdi_candidate(&candidates, args.sdi_fetch) {
                let cached = best.source == drivers::sources::Source::SdiCached;

                // Phase 1 — extract the driver subdirectory from the pack.
                // Persistent cache under sdi/extracted/<pack_stem>/ means
                // this is effectively free after the first install.
                match extract_sdi_driver(best, verbose) {
                    Some((extract_dir, extracted_inf)) => {
                        if cached {
                            // Phase 2 — Authenticode verification gate.
                            let verify_executor = RealExecutor::new(verbose);
                            let outcome = crate::commands::sdi_verify::verify_pack_directory(
                                &verify_executor,
                                &extract_dir,
                                verbose,
                            );

                            if outcome.is_safe_to_install() {
                                // Verified — proceed with install.
                                let signer = if let crate::commands::sdi_verify::PackVerifyOutcome::Verified { signers, .. } = &outcome {
                                    signers.first().cloned()
                                } else {
                                    None
                                };
                                match stage_and_install_sdi(
                                    best,
                                    &extracted_inf,
                                    target,
                                    &printer_name,
                                    &model,
                                    verbose,
                                ) {
                                    Some(result) => {
                                        let signer_tag = signer.as_deref().unwrap_or("unknown signer");
                                        report.resolution.add_tier(
                                            "SDI Origin",
                                            TierStatus::Verified,
                                            &format!("{} [verified: {signer_tag}]", best.driver_name),
                                        );
                                        populate_install_steps(&mut report, target, &best.driver_name, &printer_name, true);
                                        report.source_annotation = Some("SDI [verified]".into());
                                        report.success = true;
                                        report.elapsed = start.elapsed();
                                        report.render();
                                        return result;
                                    }
                                    None => {
                                        report.resolution.add_tier(
                                            "SDI Origin",
                                            TierStatus::Failed,
                                            "staging or install failed",
                                        );
                                    }
                                }
                            } else {
                                // Verification failed — skip and fall through.
                                let reason = match &outcome {
                                    crate::commands::sdi_verify::PackVerifyOutcome::Unsigned { unsigned, total } => {
                                        format!("verification failed: {unsigned}/{total} cats unsigned")
                                    }
                                    crate::commands::sdi_verify::PackVerifyOutcome::Invalid { first_reason, .. } => {
                                        format!("verification failed: {first_reason}")
                                    }
                                    crate::commands::sdi_verify::PackVerifyOutcome::NoCatalogs => {
                                        "verification failed: no .cat catalogs in pack".to_string()
                                    }
                                    crate::commands::sdi_verify::PackVerifyOutcome::Verified { .. } => {
                                        // unreachable given is_safe_to_install == false
                                        "verification failed".to_string()
                                    }
                                };
                                report.resolution.add_tier("SDI Origin", TierStatus::Failed, &reason);
                            }
                        } else {
                            // Uncached + --sdi-fetch path. Install WITHOUT verification
                            // for now. Future work: run the verify gate after fetch+extract
                            // inside this same flow.
                            match stage_and_install_sdi(
                                best,
                                &extracted_inf,
                                target,
                                &printer_name,
                                &model,
                                verbose,
                            ) {
                                Some(result) => {
                                    report.resolution.add_tier(
                                        "SDI Origin",
                                        TierStatus::Matched,
                                        &format!("{} [fetched, UNVERIFIED]", best.driver_name),
                                    );
                                    populate_install_steps(&mut report, target, &best.driver_name, &printer_name, true);
                                    report.source_annotation = Some("SDI [fetched, unverified]".into());
                                    report.success = true;
                                    report.elapsed = start.elapsed();
                                    report.render();
                                    return result;
                                }
                                None => {
                                    report.resolution.add_tier(
                                        "SDI Origin",
                                        TierStatus::Failed,
                                        "staging or install failed",
                                    );
                                }
                            }
                        }
                    }
                    None => {
                        report.resolution.add_tier("SDI Origin", TierStatus::Failed, "extraction failed");
                    }
                }
            } else if !candidates.is_empty() {
                report.resolution.add_tier("SDI Origin", TierStatus::Skipped,
                    "pack not cached (use --sdi-fetch)");
            } else {
                report.resolution.add_tier("SDI Origin", TierStatus::Failed, "no HWID match in indexes");
            }
        } else {
            report.resolution.add_tier("SDI Origin", TierStatus::Skipped, "cache not initialized");
        }
    } else {
        report.resolution.add_tier("SDI Origin", TierStatus::Skipped, "no device ID");
    }

    // ── Step 7: IPP Class Driver fallback ────────────────────────────────
    if !ipp_reachable(target).await {
        report.resolution.add_tier("IPP Class Driver", TierStatus::Failed, "port 631 not reachable");
        report.success = false;
        report.error = Some("all tiers exhausted, no fallback available".into());
        report.elapsed = start.elapsed();
        report.render();
        return primary_result;
    }

    let executor = RealExecutor::new(verbose);
    let ipp_result = try_ipp_fallback(&executor, target, &driver_name, &model, verbose);
    if ipp_result.success {
        report.resolution.add_tier("IPP Class Driver", TierStatus::Matched, "Microsoft IPP Class Driver");
        populate_install_steps(&mut report, target, "Microsoft IPP Class Driver", &format!("{model} (IPP)"), true);
        report.source_annotation = Some("IPP Class Driver (generic fallback)".into());
        report.success = true;
    } else {
        report.resolution.add_tier("IPP Class Driver", TierStatus::Failed, "Add-Printer failed");
        report.success = false;
        report.error = Some("all tiers exhausted".into());
    }
    report.elapsed = start.elapsed();
    report.render();
    ipp_result
}

/// Extract CID (Compatible ID) from a 1284 device ID string.
fn extract_cid(device_id: &str) -> Option<String> {
    for part in device_id.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("CID:").or_else(|| part.strip_prefix("COMPATIBLEID:")) {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Fill the install phase with port/driver/queue steps.
fn populate_install_steps(
    report: &mut InstallReport,
    ip: &str,
    driver_name: &str,
    printer_name: &str,
    all_ok: bool,
) {
    report.install.add_step("Port", &format!("IP_{ip}"), all_ok);
    report.install.add_step("Driver", driver_name, all_ok);
    report.install.add_step("Queue", printer_name, all_ok);
}

/// Run the Authenticode verification gate on a catalog-extracted pack.
///
/// Returns `true` if the pack is safe to install (all `.cat` catalogs carry a
/// trusted Authenticode signature). Returns `false` if the pack should be
/// skipped — in which case this helper has already recorded the appropriate
/// `Failed` tier entry on `report` with a descriptive reason, so the caller
/// just needs to fall through to the next tier.
///
/// In the no-SDI lean build this is a stub that always returns `true` — that
/// variant keeps the pre-v0.4.3 ungated behavior because the `sdi_verify`
/// module isn't compiled in.
#[cfg(feature = "sdi")]
fn catalog_pack_safe_to_install(
    cab_dir: &std::path::Path,
    verbose: bool,
    report: &mut InstallReport,
) -> bool {
    let verify_executor = RealExecutor::new(verbose);
    let outcome = crate::commands::sdi_verify::verify_pack_directory(
        &verify_executor,
        cab_dir,
        verbose,
    );
    if outcome.is_safe_to_install() {
        return true;
    }
    let reason = match &outcome {
        crate::commands::sdi_verify::PackVerifyOutcome::Unsigned { unsigned, total } => {
            format!("verification failed: {unsigned}/{total} cats unsigned")
        }
        crate::commands::sdi_verify::PackVerifyOutcome::Invalid { first_reason, .. } => {
            format!("verification failed: {first_reason}")
        }
        crate::commands::sdi_verify::PackVerifyOutcome::NoCatalogs => {
            "verification failed: no .cat catalogs in pack".to_string()
        }
        crate::commands::sdi_verify::PackVerifyOutcome::Verified { .. } => {
            // Unreachable: is_safe_to_install() is false.
            "verification failed".to_string()
        }
    };
    report.resolution.add_tier("Catalog", TierStatus::Failed, &reason);
    false
}

#[cfg(not(feature = "sdi"))]
fn catalog_pack_safe_to_install(
    _cab_dir: &std::path::Path,
    _verbose: bool,
    _report: &mut InstallReport,
) -> bool {
    // No verify gate in the lean build — preserve pre-v0.4.3 behavior.
    true
}

/// Re-run the verification to pull the leaf signer for audit display.
///
/// This is a second Get-AuthenticodeSignature pass — cheap enough to be worth
/// the clarity over threading the `PackVerifyOutcome` through the success
/// path. Returns `None` in the no-SDI lean build.
#[cfg(feature = "sdi")]
fn catalog_signer_tag(cab_dir: &std::path::Path, verbose: bool) -> Option<String> {
    let verify_executor = RealExecutor::new(verbose);
    let outcome = crate::commands::sdi_verify::verify_pack_directory(
        &verify_executor,
        cab_dir,
        verbose,
    );
    if let crate::commands::sdi_verify::PackVerifyOutcome::Verified { signers, .. } = outcome {
        signers.into_iter().next()
    } else {
        None
    }
}

#[cfg(not(feature = "sdi"))]
fn catalog_signer_tag(_cab_dir: &std::path::Path, _verbose: bool) -> Option<String> {
    None
}

/// Pick the right tier status + detail string for a catalog-tier success.
///
/// When we have a signer (SDI-default build with verification), use
/// `TierStatus::Verified` and include the signer CN. Otherwise fall back to
/// `TierStatus::Matched` so the lean build renders a clean tier without
/// pretending a verification happened.
fn catalog_success_tier(
    resolved: &drivers::resolver::ResolvedDriver,
    signer: Option<&str>,
) -> (TierStatus, String) {
    match signer {
        Some(s) => (
            TierStatus::Verified,
            format!("{} (from {}) [verified: {s}]", resolved.display_name, resolved.catalog_title),
        ),
        None => (
            TierStatus::Matched,
            format!("{} (from {})", resolved.display_name, resolved.catalog_title),
        ),
    }
}

/// Build the top-of-report source-annotation string for the catalog tier.
fn catalog_source_annotation(signer: Option<&str>) -> String {
    match signer {
        Some(_) => "Microsoft Update Catalog [verified]".to_string(),
        None => "Microsoft Update Catalog".to_string(),
    }
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
#[cfg(feature = "sdi")]
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

/// Extract an SDI candidate's driver subdirectory from its cached pack,
/// returning the root extract directory (for verification) and the specific
/// INF path (for staging). Uses the persistent extraction cache under
/// `sdi/extracted/<pack_stem>/` so the slow solid-LZMA2 decompression only
/// runs on the first install of a given pack. Returns `None` if the
/// candidate isn't cached or extraction fails.
#[cfg(feature = "sdi")]
fn extract_sdi_driver(
    candidate: &drivers::sources::SourceCandidate,
    verbose: bool,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
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

    let pack_stem = pack_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let extract_dir = crate::paths::sdi_dir().join("extracted").join(pack_stem);

    // Check if this driver was already extracted in a previous run.
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

    Some((extract_dir, extracted_inf))
}

/// Stage the extracted INF via pnputil /add-driver and run the three-step
/// install. Returns `Some(annotated_result)` on success, `None` if staging
/// or install failed.
#[cfg(feature = "sdi")]
fn stage_and_install_sdi(
    candidate: &drivers::sources::SourceCandidate,
    extracted_inf: &std::path::Path,
    target: &str,
    printer_name: &str,
    model: &str,
    verbose: bool,
) -> Option<PrinterOpResult> {
    let inf_str = extracted_inf.to_string_lossy().to_string();
    if verbose {
        eprintln!("[sdi] Staging INF: {inf_str}");
    }
    let stage = installer::powershell::stage_driver_inf(&inf_str, verbose);
    if !stage.success {
        if verbose {
            eprintln!(
                "[sdi] INF staging failed: {} — falling through.",
                stage.error_summary()
            );
        }
        return None;
    }

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
#[cfg(feature = "sdi")]
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

/// USB-printer install dispatcher. If the target queue already exists, swap
/// its driver (legacy flow). Otherwise, resolve a USB device by friendly
/// name and stage-and-install a driver for the orphan device.
async fn run_usb(args: AddArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;
    let target = args.target;

    if verbose {
        eprintln!("[add] USB mode — target: '{target}'");
    }

    // Existing queue? → driver swap (legacy behavior).
    if installer::powershell::printer_exists(target, verbose) {
        return run_usb_swap_driver(args).await;
    }

    // No queue — resolve USB device and stage-and-install.
    let executor = RealExecutor::new(verbose);
    let Some(device) = resolve_usb_device_by_name(&executor, target, verbose).await else {
        return PrinterOpResult::err(format!(
            "No USB printer matching '{target}' found. Run `prinstall scan --usb-only` to see attached devices."
        ));
    };

    run_usb_stage_and_install(args, device).await
}

/// Legacy USB flow: queue already exists, swap its driver via Set-Printer.
/// No port creation, no SNMP, no IPP fallback.
async fn run_usb_swap_driver(args: AddArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;
    let target = args.target;

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

/// New USB flow for orphan devices without a queue. Matches the driver,
/// stages it via pnputil, triggers a PnP rescan, polls for queue creation,
/// and falls back to an explicit Add-Printer if PnP doesn't bite.
async fn run_usb_stage_and_install(
    args: AddArgs<'_>,
    device: crate::models::UsbDevice,
) -> PrinterOpResult {
    let verbose = args.verbose;
    let friendly = device
        .friendly_name
        .as_deref()
        .unwrap_or(args.target)
        .to_string();
    let model = args
        .model_override
        .map(|m| m.to_string())
        .unwrap_or_else(|| friendly.clone());

    let local_drivers = drivers::local_store::list_drivers(verbose);
    let driver_name = match resolve_driver(&args, &model, &local_drivers, verbose) {
        Ok(name) => name,
        Err(result) => return result,
    };

    if verbose {
        eprintln!("[add] USB stage-and-install: device='{friendly}' driver='{driver_name}'");
    }

    // Stage the driver package (matches the network-path behavior).
    stage_driver_if_needed(&driver_name, &model, &local_drivers, verbose).await;

    // pnputil /add-driver + /scan-devices to make PnP pick it up.
    let executor = RealExecutor::new(verbose);
    let staging = crate::paths::staging_dir();
    let staging_str = staging.to_string_lossy().to_string();
    let add_result =
        installer::powershell::pnputil_add_driver(&executor, &staging_str, verbose).await;
    if !add_result.success {
        return PrinterOpResult::err(format!(
            "Failed to stage driver via pnputil: {}",
            add_result.error.unwrap_or_default()
        ));
    }
    let _ = installer::powershell::pnputil_scan_devices(&executor, verbose).await;

    // Poll for queue creation (~5s).
    let printer_name = args.name_override.unwrap_or(&friendly).to_string();
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if installer::powershell::printer_exists(&printer_name, verbose) {
            return PrinterOpResult::ok(InstallDetail {
                printer_name,
                driver_name,
                port_name: "USB (PnP auto)".into(),
                warning: None,
            });
        }
    }

    // Explicit Add-Printer fallback using the USB port.
    let Some(port) =
        installer::powershell::find_usb_port_for_device(&executor, &friendly, verbose).await
    else {
        return PrinterOpResult::err(format!(
            "USB driver staged but no matching USB port found for '{friendly}'. Replug the device."
        ));
    };

    let escaped_name = escape_ps_string(&printer_name);
    let escaped_driver = escape_ps_string(&driver_name);
    let escaped_port = escape_ps_string(&port);
    // Wrap in single quotes to match the rest of the codebase (see
    // try_ipp_fallback for the pattern). escape_ps_string doubles any
    // embedded single quotes so the quoting stays intact.
    let cmd = format!(
        "Add-Printer -Name '{escaped_name}' -DriverName '{escaped_driver}' -PortName '{escaped_port}'"
    );
    let ps_result = executor.run(&cmd);
    if ps_result.success {
        PrinterOpResult::ok(InstallDetail {
            printer_name,
            driver_name,
            port_name: port,
            warning: Some(
                "Installed via explicit Add-Printer fallback after PnP timeout".into(),
            ),
        })
    } else {
        PrinterOpResult::err(format!(
            "Add-Printer failed: {}",
            ps_result.error_summary()
        ))
    }
}

/// Find a USB device by case-insensitive friendly-name substring match.
/// Returns None when no device matches — caller falls back to an error
/// or to legacy queue-swap behavior depending on context.
async fn resolve_usb_device_by_name(
    exec: &dyn PsExecutor,
    name: &str,
    verbose: bool,
) -> Option<crate::models::UsbDevice> {
    let devices = discovery::usb::enumerate(exec, verbose).await;
    let needle = name.to_ascii_lowercase();
    devices.into_iter().find(|d| {
        d.friendly_name
            .as_deref()
            .map(|f| f.to_ascii_lowercase().contains(&needle))
            .unwrap_or(false)
    })
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
                        stage_result.error_summary()
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

    fn sample_resolved() -> drivers::resolver::ResolvedDriver {
        drivers::resolver::ResolvedDriver {
            inf_path: std::path::PathBuf::from("/tmp/abc/prnhp001.inf"),
            display_name: "HP Universal Printing PCL 6".to_string(),
            catalog_title: "HP - Printer - 61.325.1.24923".to_string(),
            catalog_date: "2024-01-15".to_string(),
            driver_ver: Some("10/24/2023,61.325.1.24923".to_string()),
            matched_hwid: "1284_CID_HP_UNIVERSAL_PCL6".to_string(),
        }
    }

    #[test]
    fn catalog_success_tier_verified_includes_signer() {
        let resolved = sample_resolved();
        let (status, detail) = catalog_success_tier(&resolved, Some("CN=HP Inc."));
        assert!(matches!(status, TierStatus::Verified));
        assert!(detail.contains("HP Universal Printing PCL 6"));
        assert!(detail.contains("verified: CN=HP Inc."));
    }

    #[test]
    fn catalog_success_tier_unverified_falls_back_to_matched() {
        let resolved = sample_resolved();
        let (status, detail) = catalog_success_tier(&resolved, None);
        assert!(matches!(status, TierStatus::Matched));
        assert!(detail.contains("HP Universal Printing PCL 6"));
        // No "verified" marker in the detail when we had no signer.
        assert!(!detail.contains("[verified"));
    }

    #[test]
    fn catalog_source_annotation_tags_verified_builds() {
        assert_eq!(
            catalog_source_annotation(Some("CN=HP Inc.")),
            "Microsoft Update Catalog [verified]"
        );
    }

    #[test]
    fn catalog_source_annotation_plain_without_signer() {
        assert_eq!(
            catalog_source_annotation(None),
            "Microsoft Update Catalog"
        );
    }
}

/// Tests for the catalog-tier Authenticode verification gate added in Task 25.
///
/// These only run with `--features sdi` because `verify_pack_directory` (and
/// the `PackVerifyOutcome` type it returns) lives in the `sdi_verify` module.
/// The no-SDI lean build falls through `catalog_pack_safe_to_install` as a
/// stub that always returns `true`, which is tested indirectly by the
/// compile-time cfg gates — no runtime tests needed for that branch.
#[cfg(test)]
#[cfg(feature = "sdi")]
mod catalog_gate_tests {
    use super::*;
    use crate::verbose::InstallReport;

    /// Empty pack → NoCatalogs → unsafe. The helper should record a Failed
    /// tier with the "no .cat catalogs" reason.
    #[test]
    fn gate_rejects_pack_with_no_cat_files() {
        let tmp = std::env::temp_dir().join(format!("prinstall-test-empty-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // Drop one .inf but no .cat to force the NoCatalogs verdict.
        std::fs::write(tmp.join("dummy.inf"), b"; not signed").unwrap();

        let mut report = InstallReport::new("10.0.0.5");
        let safe = catalog_pack_safe_to_install(&tmp, false, &mut report);
        assert!(!safe, "pack with no .cat files must be rejected");

        // Clean up.
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Missing directory → still returns NoCatalogs (find_cat_files yields an
    /// empty vec for unreadable dirs) → unsafe. Smoke test that the helper
    /// doesn't panic on a bogus path.
    #[test]
    fn gate_handles_missing_directory_without_panic() {
        let missing = std::path::PathBuf::from("/nonexistent/prinstall-test-missing");
        let mut report = InstallReport::new("10.0.0.6");
        let safe = catalog_pack_safe_to_install(&missing, false, &mut report);
        assert!(!safe);
    }
}

#[cfg(test)]
mod usb_install_tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use crate::installer::powershell::PsResult;

    fn ok(stdout: &str) -> PsResult {
        PsResult {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn resolve_usb_device_matches_by_friendly_name() {
        let mock = MockExecutor::new()
            .stub_contains(
                "Get-PnpDevice",
                ok(r#"[{"FriendlyName":"HP LaserJet 1320","InstanceId":"USB\\VID_03F0&PID_1D17\\ABC","Status":"Error"}]"#),
            )
            .stub_contains("Get-Printer", ok("[]"));
        let dev = resolve_usb_device_by_name(&mock, "HP LaserJet 1320", false).await;
        assert!(dev.is_some());
        assert_eq!(dev.unwrap().friendly_name.as_deref(), Some("HP LaserJet 1320"));
    }

    #[tokio::test]
    async fn resolve_usb_device_matches_substring_case_insensitive() {
        let mock = MockExecutor::new()
            .stub_contains(
                "Get-PnpDevice",
                ok(r#"[{"FriendlyName":"HP LaserJet 1320","InstanceId":"USB\\VID_03F0&PID_1D17\\ABC","Status":"Error"}]"#),
            )
            .stub_contains("Get-Printer", ok("[]"));
        let dev = resolve_usb_device_by_name(&mock, "laserjet 1320", false).await;
        assert!(dev.is_some());
    }

    #[tokio::test]
    async fn resolve_usb_device_returns_none_when_missing() {
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok("[]"))
            .stub_contains("Get-Printer", ok("[]"));
        let dev = resolve_usb_device_by_name(&mock, "Nonexistent", false).await;
        assert!(dev.is_none());
    }
}

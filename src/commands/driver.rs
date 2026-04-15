//! `prinstall driver ...` — noun-group commands for managing the Windows
//! driver store without touching printer queues.
//!
//! ## `driver add <target>`
//!
//! Stage a driver. `target` is either a path (INF file / folder) or a model
//! string:
//!
//! * **Path** — single INF or a folder containing INFs. Runs `pnputil
//!   /add-driver` (with `/subdirs` for folders). When verification is enabled,
//!   `Get-AuthenticodeSignature` runs on any `.cat` files in the target path
//!   before staging.
//!
//! * **Model string** — matches the model against embedded driver sources
//!   (`known_matches.toml`, `drivers.toml`). If a curated exact match exists,
//!   the manufacturer pack is downloaded and staged automatically. Otherwise
//!   the command prints ranked candidates and requires `--driver "<name>"`
//!   to pick one.
//!
//! Target type is auto-detected — paths contain separators or resolve on
//! disk; model strings don't.
//!
//! `--no-verify` bypasses the Authenticode gate and tags the success line
//! with `[UNVERIFIED]` for audit trails.
//!
//! ## `driver remove <target>`
//!
//! Remove a driver from the store. `target` is either an exact driver name
//! (as shown in `Get-PrinterDriver`) or a fuzzy string that resolves to one
//! staged driver. If the driver is bound to any printer queue, the command
//! refuses unless `--force` is passed — in which case the dependent queues
//! are removed first (via the standard `prinstall remove` pipeline) and
//! then the driver.
//!
//! Windows system drivers (Microsoft IPP Class Driver, etc.) are never
//! removable.
//!
//! ## `driver list`
//!
//! Pretty-print every driver in the store via `Get-PrinterDriver`. Useful
//! for discovery before `driver remove` and for RMM audits.

use std::path::{Path, PathBuf};

use crate::core::executor::{PsExecutor, RealExecutor};
use crate::installer::powershell::escape_ps_string;
use crate::models::MatchConfidence;
use crate::{drivers, installer};

/// Arguments for `prinstall driver add`.
pub struct DriverAddArgs<'a> {
    /// Path (INF / folder) OR a model string.
    pub target: &'a str,
    /// Explicit driver name (model-string flow only). Ignored for path targets.
    pub driver: Option<&'a str>,
    pub no_verify: bool,
    pub verbose: bool,
    pub json: bool,
}

/// Entry point for `prinstall driver add <target>`.
///
/// Returns the exit code (0 on success, 1 on failure).
pub async fn add(args: DriverAddArgs<'_>) -> i32 {
    if is_path_like(args.target) {
        add_from_path(&args).await
    } else {
        add_from_model(&args).await
    }
}

/// Decide whether the target looks like a filesystem path.
///
/// Path separators (`/` / `\`) are unambiguous. A target with no separator
/// is a path only if it resolves on disk — covers the `brother.inf`-in-CWD
/// case without mis-classifying a model string that happens to end in
/// something path-ish.
fn is_path_like(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || Path::new(s).is_file() || Path::new(s).is_dir()
}

// ── Path flow (pre-existing behavior) ──────────────────────────────────────

async fn add_from_path(args: &DriverAddArgs<'_>) -> i32 {
    let path = Path::new(args.target);
    if !path.exists() {
        emit_error(args.json, &format!("path not found: {}", args.target));
        return 1;
    }

    let executor = RealExecutor::new(args.verbose);

    #[cfg(feature = "sdi")]
    if !args.no_verify {
        let verify_dir = if path.is_file() {
            path.parent().unwrap_or(path).to_path_buf()
        } else {
            path.to_path_buf()
        };

        let outcome = crate::commands::sdi_verify::verify_pack_directory(
            &executor,
            &verify_dir,
            args.verbose,
        );

        use crate::commands::sdi_verify::PackVerifyOutcome;
        match &outcome {
            PackVerifyOutcome::Verified { valid, signers } => {
                if !args.json {
                    let signer = signers.first().map(|s| s.as_str()).unwrap_or("unknown");
                    println!("✓ Verified: {valid} .cat file(s), signed by {signer}");
                }
            }
            PackVerifyOutcome::Unsigned { unsigned, total } => {
                let msg = format!(
                    "verification failed: {unsigned}/{total} cats unsigned — use --no-verify to override"
                );
                emit_error(args.json, &msg);
                if !args.json {
                    eprintln!("Use --no-verify to stage this driver anyway.");
                }
                return 1;
            }
            PackVerifyOutcome::Invalid { first_reason, .. } => {
                let msg = format!("verification failed: {first_reason}");
                emit_error(args.json, &msg);
                if !args.json {
                    eprintln!("Use --no-verify to stage this driver anyway.");
                }
                return 1;
            }
            PackVerifyOutcome::NoCatalogs => {
                let msg = format!(
                    "no .cat catalog files in {} — use --no-verify to override",
                    args.target
                );
                emit_error(args.json, &msg);
                if !args.json {
                    eprintln!(
                        "Use --no-verify if this vendor ships drivers without signed catalogs."
                    );
                }
                return 1;
            }
        }
    }

    let (success, stdout, error) = if path.is_file() {
        let res = installer::powershell::stage_driver_inf(args.target, args.verbose);
        let err = if res.success { String::new() } else { res.error_summary() };
        (res.success, res.stdout, err)
    } else {
        let pnp = installer::powershell::pnputil_add_driver(
            &executor,
            args.target,
            args.verbose,
        )
        .await;
        (pnp.success, pnp.stdout, pnp.error.unwrap_or_default())
    };

    // Best-effort spooler registration for the path flow. pnputil writes to
    // the Windows driver store; Add-PrinterDriver registers in the spooler
    // so the driver shows up in `driver list` / `Get-PrinterDriver`. We walk
    // the path to enumerate all display names, and register each (quietly
    // skipping the ones that fail — duplicates are expected when multiple
    // INFs share names, and vendor packs sometimes have inert / filtered
    // entries that aren't valid to register on their own).
    let mut registered_count = 0usize;
    let mut discovered_total = 0usize;
    if success {
        let scan_infs: Vec<PathBuf> = if path.is_file() {
            vec![path.to_path_buf()]
        } else {
            drivers::downloader::find_inf_files(path)
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for inf in &scan_infs {
            if let Ok(data) = drivers::inf::parse_inf(inf) {
                for hw in &data.hwids {
                    if hw.display_name.is_empty() || !seen.insert(hw.display_name.clone()) {
                        continue;
                    }
                    discovered_total += 1;
                    let cmd = format!(
                        "Add-PrinterDriver -Name '{}'",
                        escape_ps_string(&hw.display_name)
                    );
                    let r = executor.run(&cmd);
                    if r.success {
                        registered_count += 1;
                    } else if args.verbose {
                        eprintln!(
                            "[driver add] Spooler register skipped for '{}': {}",
                            hw.display_name,
                            r.stderr.trim()
                        );
                    }
                }
            }
        }
    }

    if args.json {
        emit_path_result_json(args.target, success, &stdout, &error);
    } else if success {
        let unverified = if args.no_verify { " [UNVERIFIED]" } else { "" };
        let reg_suffix = if discovered_total == 0 {
            String::new()
        } else {
            format!(" ({registered_count}/{discovered_total} registered in spooler)")
        };
        println!("✓ Driver staged successfully{unverified}{reg_suffix}");
        if args.verbose && !stdout.is_empty() {
            println!();
            println!("{stdout}");
        }
    } else {
        eprintln!("Error: pnputil failed to stage driver");
        if !error.is_empty() {
            eprintln!("{error}");
        }
    }

    if success { 0 } else { 1 }
}

// ── Model flow (new — deviceless staging by model string) ───────────────────

async fn add_from_model(args: &DriverAddArgs<'_>) -> i32 {
    let model = args.target;

    // Empty local-drivers list — for pre-staging we care about manufacturer
    // sources (known_matches + universal), not what's already installed.
    // Same-model drivers already in the store are handled as a shortcut
    // below.
    let results = drivers::matcher::match_drivers(model, &[]);

    // Pick the driver name to stage.
    let driver_name = match resolve_driver_pick(args, &results) {
        Ok(name) => name,
        Err(exit) => return exit,
    };

    // Shortcut: already in the local store. Nothing to download.
    let local = drivers::local_store::list_drivers(args.verbose);
    if local.iter().any(|d| d == &driver_name) {
        if args.json {
            emit_model_result_json(model, &driver_name, true, args.no_verify, Some("already in local driver store"), None);
        } else {
            println!("✓ Driver '{driver_name}' already staged (local store)");
        }
        return 0;
    }

    // Find the manufacturer entry and the universal driver.
    let manifest = drivers::manifest::Manifest::load_embedded();
    let Some(mfr) = manifest.find_manufacturer(model) else {
        emit_error(
            args.json,
            &format!(
                "no manufacturer matched '{model}'. Check the model prefix or stage from a path instead."
            ),
        );
        return 1;
    };
    let Some(ud) = mfr.universal_drivers.iter().find(|u| u.name == driver_name) else {
        emit_error(
            args.json,
            &format!(
                "driver '{driver_name}' has no download URL in the manifest for {}. Try a different --driver or stage from a path.",
                mfr.name
            ),
        );
        return 1;
    };

    // Download + extract.
    if args.verbose {
        eprintln!("[driver add] Downloading {} from {}", ud.name, ud.url);
    }
    let extract_dir = match drivers::downloader::download_and_stage(ud, args.verbose).await {
        Ok(p) => p,
        Err(e) => {
            emit_error(args.json, &format!("download failed: {e}"));
            return 1;
        }
    };

    // Verification gate.
    let signer = if args.no_verify {
        if args.verbose {
            eprintln!("[driver add] --no-verify passed, skipping Authenticode check");
        }
        None
    } else {
        match run_verify(&extract_dir, args.verbose) {
            Ok(s) => s,
            Err(reason) => {
                emit_error(args.json, &format!("verification failed: {reason}"));
                if !args.json {
                    eprintln!("Use --no-verify to stage this driver anyway.");
                }
                return 1;
            }
        }
    };

    // Stage each INF in the extracted tree.
    let infs = drivers::downloader::find_inf_files(&extract_dir);
    if infs.is_empty() {
        emit_error(
            args.json,
            &format!("downloaded pack has no INF files (extracted to {})", extract_dir.display()),
        );
        return 1;
    }

    let mut any_ok = false;
    let mut first_err: Option<String> = None;
    let mut staged_infs: Vec<PathBuf> = Vec::new();
    for inf in &infs {
        let res = installer::powershell::stage_driver_inf(
            inf.to_str().unwrap_or_default(),
            args.verbose,
        );
        if res.success {
            any_ok = true;
            staged_infs.push(inf.clone());
        } else {
            let summary = res.error_summary();
            if args.verbose {
                eprintln!(
                    "[driver add] stage_driver_inf failed for {}: {summary}",
                    inf.display()
                );
            }
            if first_err.is_none() {
                first_err = Some(summary);
            }
        }
    }

    if !any_ok {
        emit_error(
            args.json,
            &format!(
                "pnputil /add-driver failed for all {} INF(s): {}",
                infs.len(),
                first_err.unwrap_or_else(|| "unknown error".to_string())
            ),
        );
        return 1;
    }

    // Register the driver in the print spooler. pnputil puts the INF in
    // the Windows driver store, but `Get-PrinterDriver` (and every other
    // print-subsystem cmdlet) won't see the driver until `Add-PrinterDriver`
    // registers it. Without this step, a later `prinstall driver list` or
    // `driver remove` finds nothing — the driver is effectively invisible
    // to the spooler even though it's on disk.
    //
    // The manifest hint (driver_name) may not match the INF's actual
    // [Models] display name — we walk the staged INFs and pick the best
    // match. Falls back to the hint if the match algorithm yields nothing.
    let registered_name = crate::commands::add::collect_actual_driver_name(
        &staged_infs,
        &driver_name,
        args.verbose,
    )
    .unwrap_or_else(|| driver_name.clone());
    let executor = RealExecutor::new(args.verbose);
    let register_cmd = format!(
        "Add-PrinterDriver -Name '{}'",
        escape_ps_string(&registered_name)
    );
    if args.verbose {
        eprintln!("[driver add] Registering in print spooler: {register_cmd}");
    }
    let register = executor.run(&register_cmd);
    let registered = register.success;
    if !registered {
        let reason = crate::core::ps_error::clean(&register.stderr);
        if !args.json {
            eprintln!(
                "Warning: driver staged in the store but spooler registration failed: {reason}"
            );
            eprintln!(
                "         The driver is still in the Windows driver store — PnP should bind it, \
                 but it won't appear in `prinstall driver list` until a queue is attached."
            );
        }
    }

    if args.json {
        let payload = serde_json::json!({
            "success": true,
            "model": model,
            "driver": registered_name,
            "staged": true,
            "registered": registered,
            "verified": !args.no_verify && signer.is_some(),
            "signer": signer,
        });
        println!("{payload}");
    } else {
        let unverified = if args.no_verify { " [UNVERIFIED]" } else { "" };
        let signer_tag = signer
            .as_deref()
            .map(|s| format!(" (signed by {s})"))
            .unwrap_or_default();
        let status = if registered { "Staged and registered" } else { "Staged (driver store only)" };
        println!("✓ {status} '{registered_name}'{unverified}{signer_tag}");
    }
    0
}

/// Decide which driver name to stage based on match results + explicit
/// `--driver` override.
///
/// * `--driver <name>` wins. Still verified against the manifest before
///   returning.
/// * Otherwise pick the top curated (exact) match from `known_matches.toml`.
/// * Otherwise (fuzzy / universal / nothing) — print the ranked list and
///   bail with exit 1, so the user re-runs with `--driver`.
fn resolve_driver_pick(
    args: &DriverAddArgs<'_>,
    results: &crate::models::DriverResults,
) -> Result<String, i32> {
    if let Some(explicit) = args.driver {
        return Ok(explicit.to_string());
    }

    // Auto-pick the top curated match (score 1000, from known_matches.toml).
    if let Some(exact) = results
        .matched
        .iter()
        .find(|m| matches!(m.confidence, MatchConfidence::Exact))
    {
        if !args.json {
            println!("★ Curated match: {}", exact.name);
        }
        return Ok(exact.name.clone());
    }

    // No exact match — surface candidates and force an explicit pick.
    if args.json {
        emit_candidates_json(args.target, results);
    } else {
        print_candidates(args.target, results);
    }
    Err(1)
}

/// Run Authenticode verification on a freshly-extracted pack.
/// Returns `Ok(Some(signer))` on pass, `Ok(None)` in lean builds (no gate),
/// `Err(reason)` on fail.
#[cfg(feature = "sdi")]
fn run_verify(extract_dir: &Path, verbose: bool) -> Result<Option<String>, String> {
    let executor = RealExecutor::new(verbose);
    let outcome =
        crate::commands::sdi_verify::verify_pack_directory(&executor, extract_dir, verbose);
    use crate::commands::sdi_verify::PackVerifyOutcome;
    match outcome {
        PackVerifyOutcome::Verified { signers, .. } => Ok(signers.into_iter().next()),
        PackVerifyOutcome::Unsigned { unsigned, total } => {
            Err(format!("{unsigned}/{total} cats unsigned"))
        }
        PackVerifyOutcome::Invalid { first_reason, .. } => Err(first_reason),
        PackVerifyOutcome::NoCatalogs => Err(
            "no .cat catalogs in pack — vendor hasn't included them".to_string(),
        ),
    }
}

#[cfg(not(feature = "sdi"))]
fn run_verify(_extract_dir: &Path, _verbose: bool) -> Result<Option<String>, String> {
    Ok(None)
}

// ── Remove flow ────────────────────────────────────────────────────────────

/// Arguments for `prinstall driver remove`.
pub struct DriverRemoveArgs<'a> {
    /// Exact driver name OR a fuzzy/model string.
    pub target: &'a str,
    /// Cascade: remove dependent printer queues first, then the driver.
    pub force: bool,
    pub verbose: bool,
    pub json: bool,
}

/// Windows system drivers are never removable — short-circuit so we don't
/// produce a confusing "still in use" error for the expected case.
/// Mirror of `commands::remove::is_system_driver` (kept in sync with that
/// list — see `src/commands/remove.rs::SYSTEM_DRIVERS`).
const SYSTEM_DRIVER_PREFIXES: &[&str] = &[
    "Microsoft IPP Class Driver",
    "Microsoft XPS Document Writer",
    "Microsoft Print To PDF",
    "Microsoft enhanced Point and Print compatibility driver",
    "Remote Desktop Easy Print",
    "Generic / Text Only",
];

fn is_system_driver_name(name: &str) -> bool {
    SYSTEM_DRIVER_PREFIXES.iter().any(|s| name == *s)
}

/// Entry point for `prinstall driver remove <target>`.
pub async fn remove(args: DriverRemoveArgs<'_>) -> i32 {
    let executor = RealExecutor::new(args.verbose);

    // 1. Resolve target to an exact driver name by consulting the local store.
    let local = drivers::local_store::list_drivers(args.verbose);
    let driver_name = match resolve_remove_target(args.target, &local, args.json) {
        Ok(name) => name,
        Err(exit) => return exit,
    };

    // 2. System driver short-circuit.
    if is_system_driver_name(&driver_name) {
        emit_error(
            args.json,
            &format!(
                "'{driver_name}' is a Windows system driver and cannot be removed."
            ),
        );
        return 1;
    }

    // 3. Find queues using this driver.
    let dependent = find_queues_using_driver(&executor, &driver_name, args.verbose);

    if !dependent.is_empty() {
        if !args.force {
            let queues = dependent.join("', '");
            emit_error(
                args.json,
                &format!(
                    "driver '{driver_name}' is in use by {} queue(s): '{queues}'. \
                     Pass --force to remove those queues first, or run `prinstall remove <queue>` yourself.",
                    dependent.len()
                ),
            );
            return 1;
        }

        // Cascade: remove each dependent queue first (full cleanup pipeline).
        if !args.json {
            println!(
                "Cascading: removing {} queue(s) before driver...",
                dependent.len()
            );
        }
        for queue in &dependent {
            let res = crate::commands::remove::run(
                &executor,
                crate::commands::remove::RemoveArgs {
                    target: queue,
                    keep_driver: true, // we're about to remove the driver ourselves
                    keep_port: false,
                    verbose: args.verbose,
                },
            )
            .await;
            if !res.success {
                emit_error(
                    args.json,
                    &format!(
                        "failed to remove dependent queue '{queue}': {}",
                        res.error.unwrap_or_default()
                    ),
                );
                return 1;
            }
            if !args.json {
                println!("  ✓ removed queue '{queue}'");
            }
        }
    }

    // 4. Remove the driver (try -RemoveFromDriverStore first, soft-fallback).
    let cmd_with_store = format!(
        "Remove-PrinterDriver -Name '{}' -RemoveFromDriverStore -Confirm:$false",
        escape_ps_string(&driver_name)
    );
    let mut result = executor.run(&cmd_with_store);
    let mut soft = false;
    if !result.success {
        if args.verbose {
            eprintln!(
                "[driver remove] -RemoveFromDriverStore failed: {}, retrying without it",
                result.stderr.trim()
            );
        }
        let cmd = format!(
            "Remove-PrinterDriver -Name '{}' -Confirm:$false",
            escape_ps_string(&driver_name)
        );
        result = executor.run(&cmd);
        soft = result.success;
    }

    if !result.success {
        emit_error(
            args.json,
            &format!(
                "Remove-PrinterDriver failed: {}",
                crate::core::ps_error::clean(&result.stderr)
            ),
        );
        return 1;
    }

    if args.json {
        let payload = serde_json::json!({
            "success": true,
            "driver": driver_name,
            "cascaded_queues": dependent,
            "driver_store_package_removed": !soft,
        });
        println!("{payload}");
    } else {
        let tag = if soft {
            " (unregistered, driver store package remains)"
        } else {
            " (including driver store package)"
        };
        println!("✓ Removed driver '{driver_name}'{tag}");
    }
    0
}

/// Resolve the user-supplied target to an exact driver name present in
/// the local store. Returns the name on success; prints candidates and
/// returns an exit code on ambiguity.
fn resolve_remove_target(
    target: &str,
    local: &[String],
    json: bool,
) -> Result<String, i32> {
    // Exact name match → use it directly.
    if let Some(hit) = local.iter().find(|d| *d == target) {
        return Ok(hit.clone());
    }

    // Case-insensitive exact match → use it (Windows driver names are CI).
    if let Some(hit) = local.iter().find(|d| d.eq_ignore_ascii_case(target)) {
        return Ok(hit.clone());
    }

    // Fuzzy match against the local store.
    let results = drivers::matcher::match_drivers(target, local);
    // Consider only LocalStore matches — we can't remove something not staged.
    let local_matches: Vec<&crate::models::DriverMatch> = results
        .matched
        .iter()
        .filter(|m| matches!(m.source, crate::models::DriverSource::LocalStore))
        .collect();

    if let [only] = local_matches.as_slice() {
        return Ok(only.name.clone());
    }

    if local_matches.is_empty() {
        if json {
            println!(
                r#"{{"success":false,"error":"no staged driver matches '{}'","target":"{}"}}"#,
                escape_json(target),
                escape_json(target)
            );
        } else {
            eprintln!("Error: no staged driver matches '{target}'.");
            eprintln!("Run `prinstall driver list` to see what's currently in the store.");
        }
        return Err(1);
    }

    // Ambiguous — show the list and force a more specific target.
    if json {
        let payload = serde_json::json!({
            "success": false,
            "error": "ambiguous — multiple staged drivers match",
            "target": target,
            "matches": local_matches.iter().map(|m| serde_json::json!({
                "name": m.name,
                "score": m.score,
            })).collect::<Vec<_>>(),
        });
        println!("{payload}");
    } else {
        println!("Multiple staged drivers match '{target}':");
        for m in &local_matches {
            println!("  ● {}  (score: {})", m.name, m.score);
        }
        println!();
        println!("Rerun with the exact driver name:");
        println!("  prinstall driver remove \"<name>\"");
    }
    Err(1)
}

/// Return queue names currently bound to `driver_name`, via
/// `Get-Printer | Where-Object DriverName -eq '...'`.
fn find_queues_using_driver(
    executor: &dyn PsExecutor,
    driver_name: &str,
    verbose: bool,
) -> Vec<String> {
    let cmd = format!(
        "Get-Printer | Where-Object {{ $_.DriverName -eq '{}' }} | Select-Object -ExpandProperty Name",
        escape_ps_string(driver_name)
    );
    if verbose {
        eprintln!("[driver remove] {cmd}");
    }
    let result = executor.run(&cmd);
    if !result.success {
        if verbose {
            eprintln!(
                "[driver remove] Could not enumerate queues: {}",
                result.stderr.trim()
            );
        }
        return Vec::new();
    }
    result
        .stdout
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

// ── List flow ──────────────────────────────────────────────────────────────

/// Arguments for `prinstall driver list`.
pub struct DriverListArgs {
    pub verbose: bool,
    pub json: bool,
}

/// Entry point for `prinstall driver list`.
pub fn list(args: DriverListArgs) -> i32 {
    let rows = drivers::local_store::list_drivers_with_dates(args.verbose);

    if args.json {
        let payload = serde_json::json!({
            "success": true,
            "count": rows.len(),
            "drivers": rows.iter().map(|(n, d)| serde_json::json!({
                "name": n,
                "driver_date": d,
            })).collect::<Vec<_>>(),
        });
        println!("{payload}");
        return 0;
    }

    if rows.is_empty() {
        println!("No drivers in the store.");
        return 0;
    }

    // Plain-text table. Keep it dense — techs read this in a small RMM
    // shell, not a wide terminal.
    let name_width = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(0).max(4);
    println!("{:<width$}  Date", "Name", width = name_width);
    println!("{:<width$}  ----", "----", width = name_width);
    for (name, date) in &rows {
        let date_str = date.as_deref().unwrap_or("unknown");
        println!("{:<width$}  {}", name, date_str, width = name_width);
    }
    println!();
    println!("{} driver(s) in the store.", rows.len());
    0
}

// ── Output helpers ─────────────────────────────────────────────────────────

fn print_candidates(model: &str, results: &crate::models::DriverResults) {
    println!("No curated match for '{model}'. Pick one with --driver \"<name>\":");
    println!();

    if !results.matched.is_empty() {
        println!("Matched drivers:");
        for m in &results.matched {
            let icon = match m.confidence {
                MatchConfidence::Exact => "★",
                MatchConfidence::Fuzzy => "●",
                MatchConfidence::Universal => "○",
            };
            println!("  {icon} {}  (score: {})", m.name, m.score);
        }
        println!();
    }

    if !results.universal.is_empty() {
        println!("Universal drivers:");
        for u in &results.universal {
            println!("  ○ {}", u.name);
        }
        println!();
    }

    if results.matched.is_empty() && results.universal.is_empty() {
        println!("  (no candidates found — check the model spelling or use a path)");
        println!();
    }

    println!(
        "Rerun: prinstall driver add \"{model}\" --driver \"<name from above>\""
    );
}

fn emit_candidates_json(model: &str, results: &crate::models::DriverResults) {
    // Minimal JSON shape — reuse DriverResults serialization.
    let payload = serde_json::json!({
        "success": false,
        "error": "explicit driver pick required",
        "model": model,
        "matches": results.matched.iter().map(|m| serde_json::json!({
            "name": m.name,
            "confidence": format!("{:?}", m.confidence).to_lowercase(),
            "score": m.score,
        })).collect::<Vec<_>>(),
        "universal": results.universal.iter().map(|u| u.name.clone()).collect::<Vec<_>>(),
    });
    println!("{payload}");
}

fn emit_error(json: bool, message: &str) {
    if json {
        println!(r#"{{"success":false,"error":"{}"}}"#, escape_json(message));
    } else {
        eprintln!("Error: {message}");
    }
}

fn emit_path_result_json(path: &str, success: bool, stdout: &str, error: &str) {
    println!(
        r#"{{"success":{success},"path":"{}","stdout":"{}","error":"{}"}}"#,
        escape_json(path),
        escape_json(stdout),
        escape_json(error),
    );
}

fn emit_model_result_json(
    model: &str,
    driver: &str,
    success: bool,
    no_verify: bool,
    note: Option<&str>,
    signer: Option<&str>,
) {
    let payload = serde_json::json!({
        "success": success,
        "model": model,
        "driver": driver,
        "verified": !no_verify && signer.is_some(),
        "signer": signer,
        "note": note,
    });
    println!("{payload}");
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

// Keep the unused-import warning quiet in the lean build where
// `run_verify` doesn't touch PathBuf.
const _: Option<PathBuf> = None;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_add_args_shape() {
        let args = DriverAddArgs {
            target: "C:\\test",
            driver: None,
            no_verify: false,
            verbose: false,
            json: false,
        };
        assert_eq!(args.target, "C:\\test");
        assert!(!args.no_verify);
        assert!(args.driver.is_none());
    }

    #[test]
    fn is_path_like_detects_separators() {
        assert!(is_path_like("/foo/bar"));
        assert!(is_path_like("C:\\Drivers\\HP"));
        assert!(is_path_like("./drivers/hp.inf"));
        assert!(is_path_like("drivers\\brother.inf"));
    }

    #[test]
    fn is_path_like_treats_bare_model_as_not_path() {
        assert!(!is_path_like("HP LaserJet 1320"));
        assert!(!is_path_like("hp 1320"));
        assert!(!is_path_like("Brother MFC-L2750DW"));
    }

    #[test]
    fn is_path_like_treats_existing_file_as_path() {
        // /etc/hostname exists on Linux hosts and has no separator in the basename.
        // The check is: does the string itself resolve as a file? We pass the
        // full path which contains a separator — that already trips the first
        // branch. Use a CWD-relative existing test asset instead.
        // Cargo.toml exists at the crate root during tests.
        assert!(is_path_like("Cargo.toml"));
    }

    #[test]
    fn is_path_like_treats_nonexistent_bare_name_as_not_path() {
        assert!(!is_path_like("nonexistent-model-string-xyz"));
    }

    #[tokio::test]
    async fn nonexistent_path_returns_1() {
        let args = DriverAddArgs {
            target: "/nonexistent/path/that/does/not/exist",
            driver: None,
            no_verify: true,
            verbose: false,
            json: true,
        };
        let exit = add(args).await;
        assert_eq!(exit, 1);
    }

    #[test]
    fn escape_json_handles_windows_paths() {
        let s = escape_json(r#"C:\Drivers\HP"LaserJet""#);
        assert_eq!(s, r#"C:\\Drivers\\HP\"LaserJet\""#);
    }

    #[test]
    fn escape_json_handles_newlines() {
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
    }

    // ── Remove-flow tests ─────────────────────────────────────────────────

    #[test]
    fn is_system_driver_name_catches_ipp_class() {
        assert!(is_system_driver_name("Microsoft IPP Class Driver"));
        assert!(is_system_driver_name("Microsoft XPS Document Writer"));
        assert!(is_system_driver_name("Microsoft Print To PDF"));
    }

    #[test]
    fn is_system_driver_name_rejects_vendor_drivers() {
        assert!(!is_system_driver_name("HP Universal Print Driver PCL6"));
        assert!(!is_system_driver_name("Brother MFC-L2750DW"));
        assert!(!is_system_driver_name(""));
    }

    #[test]
    fn resolve_remove_target_exact_match_wins() {
        let local = vec![
            "HP Universal Print Driver PCL6".to_string(),
            "Brother MFC-L2750DW".to_string(),
        ];
        let out = resolve_remove_target("HP Universal Print Driver PCL6", &local, false);
        assert_eq!(out.unwrap(), "HP Universal Print Driver PCL6");
    }

    #[test]
    fn resolve_remove_target_case_insensitive_exact_match() {
        let local = vec!["HP Universal Print Driver PCL6".to_string()];
        let out = resolve_remove_target("hp universal print driver pcl6", &local, false);
        assert_eq!(out.unwrap(), "HP Universal Print Driver PCL6");
    }

    #[test]
    fn resolve_remove_target_empty_local_store_errors() {
        let out = resolve_remove_target("HP 1320", &[], true);
        assert_eq!(out.unwrap_err(), 1);
    }

    #[test]
    fn resolve_remove_target_single_fuzzy_match_returns_it() {
        // Only one LocalStore-sourced driver scores above the threshold.
        let local = vec!["HP LaserJet 1320 PCL 5e".to_string()];
        let out = resolve_remove_target("hp 1320", &local, true);
        assert_eq!(out.unwrap(), "HP LaserJet 1320 PCL 5e");
    }

    #[test]
    fn find_queues_using_driver_parses_lines() {
        use crate::core::executor::MockExecutor;
        use crate::installer::powershell::PsResult;

        let mock = MockExecutor::new().stub_contains(
            "DriverName -eq",
            PsResult {
                success: true,
                stdout: "Front Desk Printer\nWarehouse Printer\n".to_string(),
                stderr: String::new(),
            },
        );
        let queues = find_queues_using_driver(&mock, "HP Universal Print Driver PCL6", false);
        assert_eq!(queues, vec!["Front Desk Printer", "Warehouse Printer"]);
    }

    #[test]
    fn find_queues_using_driver_empty_when_none_match() {
        use crate::core::executor::MockExecutor;
        use crate::installer::powershell::PsResult;

        let mock = MockExecutor::new().stub_contains(
            "DriverName -eq",
            PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let queues = find_queues_using_driver(&mock, "HP LaserJet 1320", false);
        assert!(queues.is_empty());
    }

    #[test]
    fn find_queues_using_driver_returns_empty_on_ps_failure() {
        use crate::core::executor::MockExecutor;
        let mock = MockExecutor::new().stub_failure("DriverName -eq", "Access denied");
        let queues = find_queues_using_driver(&mock, "Anything", false);
        assert!(queues.is_empty());
    }

    #[test]
    fn driver_list_args_shape() {
        let args = DriverListArgs { verbose: false, json: false };
        assert!(!args.verbose);
    }
}

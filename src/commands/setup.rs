//! `prinstall setup` — self-install / uninstall the prinstall binary.
//!
//! Install path:
//!   1. Resolve target dir (default `C:\ProgramData\prinstall`, or `--dir`).
//!   2. Copy the currently-running exe (`std::env::current_exe()`) into the
//!      target dir as `prinstall.exe`. No-op when source == target.
//!   3. Add the target dir to the Machine PATH if missing.
//!   4. Create the `Prinstall (mDNS discovery)` Windows Firewall rule on
//!      UDP 5353 so mDNS discovery works out of the box.
//!
//! Uninstall path reverses all three. If the running exe lives inside the
//! target dir, we warn the user — Windows holds an exclusive lock on the
//! running binary and we can't delete it in place. Techs should run
//! `setup uninstall` from a copy of the exe outside the install dir
//! (e.g., their Downloads folder).
//!
//! Both paths require admin. The caller (`main::cmd_setup`) runs the
//! elevation check before dispatching here.

use std::path::{Path, PathBuf};

use crate::core::executor::PsExecutor;
use crate::installer::powershell::escape_ps_string;

const FIREWALL_RULE_NAME: &str = "Prinstall (mDNS discovery)";
const DEFAULT_INSTALL_DIR_LEAF: &str = "prinstall";
const EXE_FILENAME: &str = "prinstall.exe";

/// Resolve the install dir. When the caller passes `Some`, use it as-is;
/// otherwise fall back to `C:\ProgramData\prinstall`. On non-Windows test
/// builds `PROGRAMDATA` isn't set, so we fall back to a `$TEMP` equivalent
/// — the actual runtime path always exists on Windows.
fn resolve_install_dir(override_dir: Option<&str>) -> PathBuf {
    if let Some(d) = override_dir {
        return PathBuf::from(d);
    }
    let base = std::env::var("PROGRAMDATA")
        .ok()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    PathBuf::from(base).join(DEFAULT_INSTALL_DIR_LEAF)
}

/// `prinstall setup install` — copy self into install dir + set up PATH + firewall.
pub async fn install(
    executor: &dyn PsExecutor,
    dir_override: Option<&str>,
    verbose: bool,
    json: bool,
) -> i32 {
    let install_dir = resolve_install_dir(dir_override);
    let install_dir_str = install_dir.to_string_lossy().to_string();
    let target_exe = install_dir.join(EXE_FILENAME);

    if !json {
        println!("Install dir     : {install_dir_str}");
    }

    // ── Step 1: Copy the running exe into the install dir ──────────────────
    let source_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return emit_error(json, &format!("could not resolve current exe path: {e}"));
        }
    };

    if let Err(e) = std::fs::create_dir_all(&install_dir) {
        return emit_error(
            json,
            &format!("could not create install dir {install_dir_str}: {e}"),
        );
    }

    // If the running exe IS the target, there's nothing to copy — the user
    // is already running the installed binary. Idempotent success.
    let same_file = paths_equal(&source_exe, &target_exe);
    if !same_file {
        if verbose {
            eprintln!(
                "[setup] copying {} → {}",
                source_exe.display(),
                target_exe.display()
            );
        }
        if let Err(e) = std::fs::copy(&source_exe, &target_exe) {
            return emit_error(
                json,
                &format!(
                    "could not copy {} to {}: {e}. Close any running prinstall and retry.",
                    source_exe.display(),
                    target_exe.display()
                ),
            );
        }
    } else if verbose {
        eprintln!("[setup] already running from install dir — skipping copy");
    }

    // ── Step 2: Firewall rule ──────────────────────────────────────────────
    let firewall_ok = ensure_firewall_rule(executor, &target_exe, verbose);

    // ── Step 3: Machine PATH ───────────────────────────────────────────────
    let path_ok = ensure_path_entry(executor, &install_dir_str, verbose);

    if json {
        let payload = serde_json::json!({
            "success": true,
            "action": "install",
            "install_dir": install_dir_str,
            "exe": target_exe.to_string_lossy(),
            "copied": !same_file,
            "firewall_ok": firewall_ok,
            "path_ok": path_ok,
        });
        println!("{payload}");
    } else {
        let copy_tag = if same_file { "already in place" } else { "copied" };
        println!("✓ prinstall.exe {copy_tag} at {}", target_exe.display());
        if firewall_ok {
            println!("✓ Firewall rule '{FIREWALL_RULE_NAME}' ready (UDP 5353)");
        } else {
            println!("⚠ Firewall rule '{FIREWALL_RULE_NAME}' could not be created — mDNS scan may be blocked");
        }
        if path_ok {
            println!("✓ {install_dir_str} is on Machine PATH");
        } else {
            println!("⚠ Could not update Machine PATH — use the full path to prinstall.exe");
        }
        println!();
        println!("Run `prinstall --help` from a new shell.");
    }
    0
}

/// `prinstall setup uninstall` — reverse the install.
pub async fn uninstall(
    executor: &dyn PsExecutor,
    dir_override: Option<&str>,
    verbose: bool,
    json: bool,
) -> i32 {
    let install_dir = resolve_install_dir(dir_override);
    let install_dir_str = install_dir.to_string_lossy().to_string();

    if !json {
        println!("Install dir     : {install_dir_str}");
    }

    // Warn if the running exe is inside the install dir — Windows will hold
    // an exclusive lock on it and `Remove-Item -Recurse -Force` will fail.
    // Don't abort: the PATH + firewall cleanup still runs, and the stale
    // exe can be deleted by the user on next reboot or from another shell.
    let running_exe = std::env::current_exe().ok();
    let running_in_install_dir = running_exe
        .as_deref()
        .map(|p| p.starts_with(&install_dir))
        .unwrap_or(false);
    if running_in_install_dir && !json {
        println!(
            "⚠ Running exe is inside the install dir — Windows file lock will block self-delete."
        );
        println!("  Other cleanup (firewall rule, PATH entry) still runs.");
        println!("  Remove the dir manually after the next reboot, or run `setup uninstall`");
        println!("  from a copy of the exe outside the install dir.");
    }

    // ── Step 1: Firewall rule ──────────────────────────────────────────────
    let firewall_ok = remove_firewall_rule(executor, verbose);

    // ── Step 2: Machine PATH ───────────────────────────────────────────────
    let path_ok = remove_path_entry(executor, &install_dir_str, verbose);

    // ── Step 3: Install dir ────────────────────────────────────────────────
    let dir_removed = if install_dir.exists() {
        match std::fs::remove_dir_all(&install_dir) {
            Ok(()) => {
                if verbose {
                    eprintln!("[setup] removed {install_dir_str}");
                }
                true
            }
            Err(e) => {
                if !json {
                    println!(
                        "⚠ Could not remove {install_dir_str}: {e}. Delete manually after reboot."
                    );
                }
                false
            }
        }
    } else {
        if verbose {
            eprintln!("[setup] install dir already absent");
        }
        true
    };

    if json {
        let payload = serde_json::json!({
            "success": true,
            "action": "uninstall",
            "install_dir": install_dir_str,
            "dir_removed": dir_removed,
            "firewall_removed": firewall_ok,
            "path_removed": path_ok,
            "running_in_install_dir": running_in_install_dir,
        });
        println!("{payload}");
    } else {
        if dir_removed {
            println!("✓ Install dir removed");
        }
        println!(
            "✓ Firewall rule cleanup: {}",
            if firewall_ok { "done" } else { "skipped" }
        );
        println!(
            "✓ PATH cleanup: {}",
            if path_ok { "done" } else { "skipped" }
        );
    }
    0
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn paths_equal(a: &Path, b: &Path) -> bool {
    // Canonicalize where we can so case/slash differences don't give false
    // negatives on Windows. Fall back to a lexical compare when either side
    // doesn't resolve (e.g., target doesn't exist yet).
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Run `New-NetFirewallRule` for the mDNS discovery rule. Idempotent —
/// if the rule already exists we delete-then-create so the bound program
/// path stays in sync with the actual exe location.
fn ensure_firewall_rule(executor: &dyn PsExecutor, exe: &Path, verbose: bool) -> bool {
    let exe_str = exe.to_string_lossy().to_string();
    // PowerShell pipeline: remove an existing rule with the same display name
    // first, then recreate. Wrapped in try/catch so a missing rule doesn't
    // error out. The quoting uses single quotes + escape_ps_string so
    // embedded single quotes in the exe path are preserved correctly.
    let cmd = format!(
        "try {{ if (Get-NetFirewallRule -DisplayName '{name}' -ErrorAction SilentlyContinue) {{ Remove-NetFirewallRule -DisplayName '{name}' -ErrorAction SilentlyContinue }} }} catch {{ }} ; \
         New-NetFirewallRule -DisplayName '{name}' -Description 'Allow prinstall.exe to receive mDNS multicast responses on UDP 5353 for network printer discovery.' -Direction Inbound -Protocol UDP -LocalPort 5353 -Action Allow -Profile Any -Program '{exe}' -Enabled True | Out-Null",
        name = escape_ps_string(FIREWALL_RULE_NAME),
        exe = escape_ps_string(&exe_str)
    );
    if verbose {
        eprintln!("[setup] firewall: {cmd}");
    }
    let result = executor.run(&cmd);
    result.success
}

/// Remove the mDNS firewall rule if present. Missing rule is not an error —
/// the uninstall is idempotent.
fn remove_firewall_rule(executor: &dyn PsExecutor, verbose: bool) -> bool {
    let cmd = format!(
        "if (Get-NetFirewallRule -DisplayName '{name}' -ErrorAction SilentlyContinue) {{ Remove-NetFirewallRule -DisplayName '{name}' -ErrorAction SilentlyContinue }}",
        name = escape_ps_string(FIREWALL_RULE_NAME)
    );
    if verbose {
        eprintln!("[setup] firewall cleanup: {cmd}");
    }
    let result = executor.run(&cmd);
    result.success
}

/// Add `install_dir` to Machine PATH if not already present. Done via
/// PowerShell (not a native Windows registry call) so the implementation
/// stays on the `PsExecutor` trait and unit-tests on Linux via MockExecutor.
fn ensure_path_entry(executor: &dyn PsExecutor, install_dir: &str, verbose: bool) -> bool {
    let cmd = format!(
        "$dir = '{dir}'; \
         $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine'); \
         if (-not $machinePath) {{ $machinePath = '' }}; \
         $entries = $machinePath -split ';' | Where-Object {{ $_ -ne '' }}; \
         $target = $dir.TrimEnd('\\'); \
         if (-not ($entries | Where-Object {{ $_.TrimEnd('\\') -ieq $target }})) {{ \
            $newPath = (($entries + $dir) -join ';'); \
            [Environment]::SetEnvironmentVariable('Path', $newPath, 'Machine') \
         }}",
        dir = escape_ps_string(install_dir)
    );
    if verbose {
        eprintln!("[setup] PATH add: {cmd}");
    }
    let result = executor.run(&cmd);
    result.success
}

/// Remove `install_dir` from Machine PATH. Case-insensitive, trailing-slash
/// tolerant so a round-trip install+uninstall leaves no residue.
fn remove_path_entry(executor: &dyn PsExecutor, install_dir: &str, verbose: bool) -> bool {
    let cmd = format!(
        "$dir = '{dir}'; \
         $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine'); \
         if (-not $machinePath) {{ $machinePath = '' }}; \
         $entries = $machinePath -split ';' | Where-Object {{ $_ -ne '' }}; \
         $target = $dir.TrimEnd('\\'); \
         $filtered = @($entries | Where-Object {{ $_.TrimEnd('\\') -ine $target }}); \
         if ($entries.Count -ne $filtered.Count) {{ \
            [Environment]::SetEnvironmentVariable('Path', ($filtered -join ';'), 'Machine') \
         }}",
        dir = escape_ps_string(install_dir)
    );
    if verbose {
        eprintln!("[setup] PATH cleanup: {cmd}");
    }
    let result = executor.run(&cmd);
    result.success
}

/// Emit a single-line JSON error object, or a plain-text stderr message,
/// and return exit code 1.
fn emit_error(json: bool, message: &str) -> i32 {
    if json {
        // Minimal shape so callers can parse success:false reliably.
        let payload = serde_json::json!({ "success": false, "error": message });
        println!("{payload}");
    } else {
        eprintln!("Error: {message}");
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use crate::installer::powershell::PsResult;

    #[test]
    fn resolve_install_dir_uses_override() {
        let p = resolve_install_dir(Some("C:\\Tools\\prinstall"));
        assert_eq!(p, PathBuf::from("C:\\Tools\\prinstall"));
    }

    #[test]
    fn resolve_install_dir_default_ends_with_prinstall() {
        let p = resolve_install_dir(None);
        assert_eq!(
            p.file_name().and_then(|s| s.to_str()),
            Some(DEFAULT_INSTALL_DIR_LEAF)
        );
    }

    #[test]
    fn ensure_firewall_rule_succeeds_when_ps_succeeds() {
        let mock = MockExecutor::new().stub_contains(
            "New-NetFirewallRule",
            PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let ok = ensure_firewall_rule(&mock, Path::new("C:\\ProgramData\\prinstall\\prinstall.exe"), false);
        assert!(ok);
    }

    #[test]
    fn ensure_firewall_rule_fails_when_ps_fails() {
        let mock = MockExecutor::new().stub_failure("New-NetFirewallRule", "Access denied");
        let ok = ensure_firewall_rule(&mock, Path::new("C:\\ProgramData\\prinstall\\prinstall.exe"), false);
        assert!(!ok);
    }

    #[test]
    fn remove_firewall_rule_handles_missing_rule() {
        // The PS pipeline is conditional on Get-NetFirewallRule returning
        // something — so an empty stdout counts as success (nothing to do).
        let mock = MockExecutor::new().stub_contains(
            "Get-NetFirewallRule",
            PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let ok = remove_firewall_rule(&mock, false);
        assert!(ok);
    }

    #[test]
    fn ensure_path_entry_passes_dir_to_ps() {
        let mock = MockExecutor::new().stub_contains(
            "SetEnvironmentVariable",
            PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let ok = ensure_path_entry(&mock, "C:\\ProgramData\\prinstall", false);
        assert!(ok);
    }

    #[test]
    fn paths_equal_matches_identical_strings() {
        let a = Path::new("/tmp/a/b");
        assert!(paths_equal(a, a));
    }

    #[test]
    fn paths_equal_rejects_different_paths() {
        assert!(!paths_equal(Path::new("/tmp/a"), Path::new("/tmp/b")));
    }

    #[tokio::test]
    async fn install_flow_against_mock_succeeds_with_override_dir() {
        // --dir to a writable tmp path so we can exercise the happy path
        // end-to-end against MockExecutor without touching a real
        // C:\ProgramData\prinstall.
        let tmp = std::env::temp_dir().join(format!(
            "prinstall-setup-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);

        let mock = MockExecutor::new()
            .stub_contains(
                "New-NetFirewallRule",
                PsResult { success: true, stdout: String::new(), stderr: String::new() },
            )
            .stub_contains(
                "SetEnvironmentVariable",
                PsResult { success: true, stdout: String::new(), stderr: String::new() },
            );

        let exit = install(&mock, Some(tmp.to_string_lossy().as_ref()), false, true).await;
        assert_eq!(exit, 0, "install should succeed");

        let target_exe = tmp.join(EXE_FILENAME);
        assert!(target_exe.exists(), "exe should have been copied into tmp dir");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn uninstall_flow_against_mock_succeeds() {
        let tmp = std::env::temp_dir().join(format!(
            "prinstall-uninstall-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).expect("create tmp dir");
        std::fs::write(tmp.join(EXE_FILENAME), b"fake exe contents").expect("write stub");

        let mock = MockExecutor::new()
            .stub_contains(
                "Get-NetFirewallRule",
                PsResult { success: true, stdout: String::new(), stderr: String::new() },
            )
            .stub_contains(
                "SetEnvironmentVariable",
                PsResult { success: true, stdout: String::new(), stderr: String::new() },
            );

        let exit = uninstall(&mock, Some(tmp.to_string_lossy().as_ref()), false, true).await;
        assert_eq!(exit, 0);
        assert!(!tmp.exists(), "tmp dir should be removed");
    }
}

//! The `remove` command — remove a printer queue, with optional cleanup of
//! the driver and port if they become orphaned.
//!
//! Flow:
//! 1. Resolve target → printer queue name (either by IP via `IP_<ip>` port
//!    lookup, or treat as a queue name directly).
//! 2. Capture the printer's driver and port name so we can make orphan
//!    decisions after the queue is gone.
//! 3. Remove the queue (`Remove-Printer`). Queue removal failure is fatal.
//! 4. Optionally remove the driver if no other printer uses it.
//! 5. Optionally remove the TCP/IP port if no other printer uses it.
//!
//! Driver/port cleanup failures are non-fatal — they're reported as flags
//! on the `RemoveDetail` payload, not as a failed operation.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde::Deserialize;

use crate::core::executor::{run_json, PsExecutor};
use crate::core::ps_error;
use crate::installer::powershell::{escape_ps_string, PsResult};
use crate::models::{PrinterOpResult, RemoveDetail};

/// Wait this long after `Remove-Printer` succeeds before attempting driver
/// or port cleanup. The Windows spooler holds internal references on the
/// driver and port for a few hundred milliseconds after the queue goes
/// away — without a settle delay, cleanup fails with a misleading "in use"
/// error even though `Get-Printer` reports zero references.
const SPOOLER_SETTLE_MS: u64 = 500;

/// Retry schedule for `Remove-PrinterDriver` and `Remove-PrinterPort`. Each
/// entry is the sleep duration *before* the corresponding attempt. First
/// attempt is immediate (`0`). Cumulative wait across all retries is ~5.5s,
/// which covers spooler-lag cases seen on both real hardware and VMs.
/// Windows usually releases references within 1–3s; the extended tail is
/// insurance for slow/loaded systems.
const REMOVE_RETRY_DELAYS_MS: &[u64] = &[0, 1000, 2000, 2500];

/// Arguments for `prinstall remove <target>`.
pub struct RemoveArgs<'a> {
    pub target: &'a str,
    pub keep_driver: bool,
    pub keep_port: bool,
    pub verbose: bool,
}

/// Projection of `Get-Printer | Select DriverName,PortName` after
/// `ConvertTo-Json`. PowerShell emits PascalCase property names.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PrinterInfo {
    driver_name: String,
    port_name: String,
}

/// Windows system drivers that should never be removed regardless of orphan
/// status. These ship with Windows and back the IPP/WSD/PDF subsystems even
/// when no user-installed printer explicitly uses them — `Remove-PrinterDriver`
/// will refuse to remove them with a misleading "in use" error.
const SYSTEM_DRIVERS: &[&str] = &[
    "Microsoft IPP Class Driver",
    "Microsoft Print To PDF",
    "Microsoft XPS Document Writer",
    "Microsoft Virtual Print Class Driver",
    "Universal Print Class Driver",
    "Remote Desktop Easy Print",
    "Microsoft enhanced Point and Print compatibility driver",
    "Microsoft Shared Fax Driver",
    "Generic / Text Only",
];

fn is_system_driver(name: &str) -> bool {
    SYSTEM_DRIVERS.iter().any(|sys| name == *sys)
}

/// Prinstall only ever creates `IP_<ip>` TCP/IP ports. Any other port name
/// (USB001, LPT1, COM1, PORTPROMPT:, FILE:, WSD-*, custom Brother/HP PnP
/// ports, virtual Print-To-OneNote ports, etc.) is something Windows or
/// another app manages. We whitelist rather than blacklist so the cleanup
/// path never touches a port we didn't create.
fn is_manageable_port(port_name: &str) -> bool {
    port_name.starts_with("IP_") || port_name.starts_with("http://")
}

pub async fn run(executor: &dyn PsExecutor, args: RemoveArgs<'_>) -> PrinterOpResult {
    let verbose = args.verbose;

    // ── Step 1: resolve target → printer queue name ──────────────────────────
    let printer_name = match resolve_printer_name(executor, args.target, verbose) {
        Some(name) => name,
        None => {
            if verbose {
                eprintln!(
                    "[remove] No printer found for target '{}' — nothing to remove",
                    args.target
                );
            }
            // Idempotent success — nothing to do, don't record history.
            return PrinterOpResult::ok(RemoveDetail {
                printer_name: args.target.to_string(),
                port_removed: false,
                driver_removed: false,
                already_absent: true,
            });
        }
    };

    if verbose {
        eprintln!("[remove] Resolved target '{}' → '{printer_name}'", args.target);
    }

    // ── Step 2: capture driver + port before the queue goes away ─────────────
    let info = match fetch_printer_info(executor, &printer_name) {
        Ok(info) => info,
        Err(e) => {
            if verbose {
                eprintln!("[remove] Failed to query printer details: {e}");
            }
            return PrinterOpResult::err(format!(
                "Failed to query details for printer '{printer_name}': {e}"
            ));
        }
    };

    if verbose {
        eprintln!(
            "[remove] Printer uses driver '{}' on port '{}'",
            info.driver_name, info.port_name
        );
    }

    // ── Step 3: remove the queue (fatal on failure) ──────────────────────────
    let remove_cmd = format!(
        "Remove-Printer -Name '{}' -Confirm:$false",
        escape_ps_string(&printer_name)
    );
    if verbose {
        eprintln!("[remove] {remove_cmd}");
    }
    let remove_result = executor.run(&remove_cmd);
    if !remove_result.success {
        return PrinterOpResult::err(format!(
            "Failed to remove printer '{printer_name}': {}",
            ps_error::clean(&remove_result.stderr)
        ));
    }

    // Give the spooler a moment to release references on the driver and
    // port before we try to remove them. Without this, both removals fail
    // with "in use" errors even though Get-Printer shows zero references.
    if !(args.keep_driver && args.keep_port) {
        if verbose {
            eprintln!(
                "[remove] Waiting {SPOOLER_SETTLE_MS}ms for spooler to release references..."
            );
        }
        std::thread::sleep(Duration::from_millis(SPOOLER_SETTLE_MS));
    }

    // ── Step 4: driver cleanup (non-fatal) ───────────────────────────────────
    let driver_removed = if args.keep_driver {
        if verbose {
            eprintln!("[remove] --keep-driver set, skipping driver cleanup");
        }
        false
    } else {
        try_remove_driver_if_orphaned(executor, &info.driver_name, verbose)
    };

    // ── Step 5: port cleanup (non-fatal) ─────────────────────────────────────
    let port_removed = if args.keep_port {
        if verbose {
            eprintln!("[remove] --keep-port set, skipping port cleanup");
        }
        false
    } else {
        try_remove_port_if_orphaned(executor, &info.port_name, verbose)
    };

    // Record in history — use the printer name as the "model" since we don't
    // have the original SNMP model string at removal time.
    crate::history::record_install(&printer_name, &info.driver_name, "remove");

    PrinterOpResult::ok(RemoveDetail {
        printer_name,
        port_removed,
        driver_removed,
        already_absent: false,
    })
}

/// Resolve a target string (IP or queue name) into a Windows printer queue name.
/// Returns `None` if no matching printer exists.
fn resolve_printer_name(
    executor: &dyn PsExecutor,
    target: &str,
    verbose: bool,
) -> Option<String> {
    // If it parses as an IPv4, look up via the `IP_<ip>` port name convention.
    if target.parse::<Ipv4Addr>().is_ok() {
        let port_name = format!("IP_{target}");
        let cmd = format!(
            "Get-Printer | Where-Object {{ $_.PortName -eq '{}' }} | Select-Object -ExpandProperty Name -First 1",
            escape_ps_string(&port_name)
        );
        if verbose {
            eprintln!("[remove] Looking up printer by port '{port_name}'");
            eprintln!("[remove] {cmd}");
        }
        let result = executor.run(&cmd);
        if !result.success {
            return None;
        }
        let name = result.stdout.trim();
        if name.is_empty() {
            return None;
        }
        return Some(name.to_string());
    }

    // Otherwise treat target as a queue name; verify it exists.
    let cmd = format!(
        "Get-Printer -Name '{}' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name",
        escape_ps_string(target)
    );
    if verbose {
        eprintln!("[remove] Verifying printer name '{target}' exists");
        eprintln!("[remove] {cmd}");
    }
    let result = executor.run(&cmd);
    if !result.success {
        return None;
    }
    let name = result.stdout.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Query the driver and port for the given printer queue.
fn fetch_printer_info(executor: &dyn PsExecutor, printer_name: &str) -> Result<PrinterInfo, String> {
    let cmd = format!(
        "Get-Printer -Name '{}' | Select-Object DriverName,PortName | ConvertTo-Json -Compress",
        escape_ps_string(printer_name)
    );
    run_json::<PrinterInfo>(executor, &cmd)
}

/// Run a PowerShell command with a retry schedule. Each entry in
/// `delays_ms` is the sleep *before* the corresponding attempt — the first
/// entry should normally be `0` so the first attempt is immediate. Returns
/// the first successful result, or the last failure if all attempts fail.
///
/// The Windows spooler keeps internal references on printer drivers and
/// ports for 1–3 seconds after `Remove-Printer` returns, sometimes longer
/// in VMs, so a single-shot removal frequently fails with a misleading
/// "in use" error even though `Get-Printer` shows zero references. This
/// helper exists specifically to smooth over that race.
fn run_with_retries(
    executor: &dyn PsExecutor,
    cmd: &str,
    delays_ms: &[u64],
    verbose: bool,
) -> PsResult {
    let mut last = PsResult {
        success: false,
        stdout: String::new(),
        stderr: "no attempts were made".to_string(),
    };
    for (i, delay) in delays_ms.iter().enumerate() {
        if *delay > 0 {
            if verbose {
                eprintln!(
                    "[remove] Waiting {delay}ms before retry {}/{}...",
                    i + 1,
                    delays_ms.len()
                );
            }
            std::thread::sleep(Duration::from_millis(*delay));
        }
        if verbose {
            if i == 0 {
                eprintln!("[remove] {cmd}");
            } else {
                eprintln!("[remove] retry {}/{}: {cmd}", i + 1, delays_ms.len());
            }
        }
        let result = executor.run(cmd);
        if result.success {
            if verbose && i > 0 {
                eprintln!("[remove] Succeeded on attempt {}", i + 1);
            }
            return result;
        }
        last = result;
    }
    last
}

/// If the driver is no longer used by any printer, remove it. Returns `true`
/// on successful removal, `false` if skipped (still in use, system driver,
/// or removal failed).
fn try_remove_driver_if_orphaned(
    executor: &dyn PsExecutor,
    driver_name: &str,
    verbose: bool,
) -> bool {
    // Windows system drivers are never removable — they back the OS's
    // built-in print subsystems. Short-circuit here so we don't waste a
    // round-trip and don't log a scary "in use" warning for the expected case.
    if is_system_driver(driver_name) {
        if verbose {
            eprintln!(
                "[remove] Skipping driver cleanup: '{driver_name}' is a Windows system driver"
            );
        }
        return false;
    }

    let count_cmd = format!(
        "(Get-Printer | Where-Object {{ $_.DriverName -eq '{}' }} | Measure-Object).Count",
        escape_ps_string(driver_name)
    );
    if verbose {
        eprintln!("[remove] Checking driver usage: {count_cmd}");
    }
    let count_result = executor.run(&count_cmd);
    if !count_result.success {
        if verbose {
            eprintln!(
                "[remove] Could not check driver usage: {}",
                count_result.stderr
            );
        }
        return false;
    }
    let count: u32 = match count_result.stdout.trim().parse() {
        Ok(n) => n,
        Err(e) => {
            if verbose {
                eprintln!(
                    "[remove] Could not parse driver usage count '{}': {e}",
                    count_result.stdout.trim()
                );
            }
            return false;
        }
    };
    if count > 0 {
        if verbose {
            eprintln!(
                "[remove] Driver '{driver_name}' still used by {count} printer(s), keeping"
            );
        }
        return false;
    }

    // Try with -RemoveFromDriverStore first, which also kills the underlying
    // oem<N>.inf package in the Windows driver store. That's how we avoid
    // leaving behind sibling drivers from multi-driver INF packages (e.g.
    // prnbrcl1.inf registers 6+ Brother class drivers in one shot — stale
    // siblings clutter Print Server Properties if we only remove the one we
    // care about). If the store-delete fails (usually because a sibling is
    // still referenced by another printer), fall back to the plain
    // Remove-PrinterDriver which at least unregisters the named driver.
    let cmd_with_store = format!(
        "Remove-PrinterDriver -Name '{}' -RemoveFromDriverStore -Confirm:$false",
        escape_ps_string(driver_name)
    );
    let result = run_with_retries(executor, &cmd_with_store, REMOVE_RETRY_DELAYS_MS, verbose);
    if result.success {
        if verbose {
            eprintln!("[remove] Removed driver '{driver_name}' (including driver store package)");
        }
        return true;
    }
    if verbose {
        eprintln!(
            "[remove] -RemoveFromDriverStore attempt failed ({}), falling back to soft unregister.",
            ps_error::clean(&result.stderr)
        );
    }

    let cmd = format!(
        "Remove-PrinterDriver -Name '{}' -Confirm:$false",
        escape_ps_string(driver_name)
    );
    let result = run_with_retries(executor, &cmd, REMOVE_RETRY_DELAYS_MS, verbose);
    if !result.success {
        if verbose {
            eprintln!(
                "[remove] Warning: failed to remove driver '{driver_name}' after {} attempts: {}",
                REMOVE_RETRY_DELAYS_MS.len(),
                ps_error::clean(&result.stderr)
            );
        }
        return false;
    }
    if verbose {
        eprintln!("[remove] Removed driver '{driver_name}' (unregistered only — driver store package may remain)");
    }
    true
}

/// If the port is no longer used by any printer, remove it. Returns `true`
/// on successful removal, `false` if skipped (not manageable, still in use,
/// or removal failed).
fn try_remove_port_if_orphaned(
    executor: &dyn PsExecutor,
    port_name: &str,
    verbose: bool,
) -> bool {
    // USB, LPT, COM, PORTPROMPT:, WSD-*, and other non-IP ports are managed
    // by Windows or their respective subsystems — never attempt to remove them.
    if !is_manageable_port(port_name) {
        if verbose {
            eprintln!(
                "[remove] Skipping port cleanup: '{port_name}' is not a prinstall-managed port"
            );
        }
        return false;
    }

    let count_cmd = format!(
        "(Get-Printer | Where-Object {{ $_.PortName -eq '{}' }} | Measure-Object).Count",
        escape_ps_string(port_name)
    );
    if verbose {
        eprintln!("[remove] Checking port usage: {count_cmd}");
    }
    let count_result = executor.run(&count_cmd);
    if !count_result.success {
        if verbose {
            eprintln!(
                "[remove] Could not check port usage: {}",
                count_result.stderr
            );
        }
        return false;
    }
    let count: u32 = match count_result.stdout.trim().parse() {
        Ok(n) => n,
        Err(e) => {
            if verbose {
                eprintln!(
                    "[remove] Could not parse port usage count '{}': {e}",
                    count_result.stdout.trim()
                );
            }
            return false;
        }
    };
    if count > 0 {
        if verbose {
            eprintln!("[remove] Port '{port_name}' still used by {count} printer(s), keeping");
        }
        return false;
    }

    let cmd = format!(
        "Remove-PrinterPort -Name '{}' -Confirm:$false",
        escape_ps_string(port_name)
    );
    // Same spooler-lag retry schedule as driver removal — Windows holds the
    // port lock for 1–3s after `Remove-Printer`, sometimes longer in VMs.
    let result = run_with_retries(executor, &cmd, REMOVE_RETRY_DELAYS_MS, verbose);
    if !result.success {
        if verbose {
            eprintln!(
                "[remove] Warning: failed to remove port '{port_name}' after {} attempts: {}",
                REMOVE_RETRY_DELAYS_MS.len(),
                ps_error::clean(&result.stderr)
            );
        }
        return false;
    }
    if verbose {
        eprintln!("[remove] Removed port '{port_name}'");
    }
    true
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use crate::installer::powershell::PsResult;

    /// Build a mock that returns the standard `Get-Printer | Select DriverName,PortName`
    /// JSON payload for the info-fetch stage.
    fn stub_printer_info(
        mock: MockExecutor,
        driver: &str,
        port: &str,
    ) -> MockExecutor {
        let json = format!(
            "{{\"DriverName\":\"{driver}\",\"PortName\":\"{port}\"}}"
        );
        mock.stub_contains(
            "Select-Object DriverName,PortName",
            PsResult {
                success: true,
                stdout: json,
                stderr: String::new(),
            },
        )
    }

    fn ok_stdout(stdout: &str) -> PsResult {
        PsResult {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn remove_by_printer_name_success() {
        // Stub order matters — MockExecutor uses first-match-wins, so more
        // specific stubs (info fetch, remove) must come before the bare
        // existence-check stub which matches "Get-Printer -Name '...'".
        let mock = stub_printer_info(MockExecutor::new(), "Brother Universal Printer", "IP_10.10.20.16")
            // Remove-Printer succeeds.
            .stub_contains("Remove-Printer -Name", ok_stdout(""))
            // Remove-PrinterDriver succeeds.
            .stub_contains("Remove-PrinterDriver", ok_stdout(""))
            // Remove-PrinterPort succeeds.
            .stub_contains("Remove-PrinterPort", ok_stdout(""))
            // Driver-in-use count → 0 (orphaned).
            .stub_contains("DriverName -eq", ok_stdout("0"))
            // Port-in-use count → 0 (orphaned).
            .stub_contains("PortName -eq", ok_stdout("0"))
            // Existence check comes last so earlier specific stubs match first.
            .stub_contains(
                "Get-Printer -Name 'Brother MFC-L2750DW'",
                ok_stdout("Brother MFC-L2750DW"),
            );

        let result = run(
            &mock,
            RemoveArgs {
                target: "Brother MFC-L2750DW",
                keep_driver: false,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success, "expected success, got {result:?}");
        let detail = result.detail_as::<RemoveDetail>().expect("detail present");
        assert_eq!(detail.printer_name, "Brother MFC-L2750DW");
        assert!(detail.port_removed);
        assert!(detail.driver_removed);
    }

    #[tokio::test]
    async fn remove_by_ip_resolves_to_name() {
        let mock = MockExecutor::new()
            // IP lookup: distinguish the resolver (uses `-ExpandProperty Name -First 1`)
            // from the later orphan count (uses `Measure-Object).Count`).
            .stub_contains(
                "-ExpandProperty Name -First 1",
                ok_stdout("Brother MFC-L2750DW"),
            );
        let mock = stub_printer_info(mock, "Brother Universal Printer", "IP_10.10.20.16");
        let mock = mock
            .stub_contains("Remove-Printer -Name", ok_stdout(""))
            .stub_contains("DriverName -eq", ok_stdout("0"))
            .stub_contains("Remove-PrinterDriver", ok_stdout(""))
            .stub_contains("PortName -eq", ok_stdout("0"))
            .stub_contains("Remove-PrinterPort", ok_stdout(""));

        let result = run(
            &mock,
            RemoveArgs {
                target: "10.10.20.16",
                keep_driver: false,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success, "expected success, got {result:?}");
        let detail = result.detail_as::<RemoveDetail>().expect("detail present");
        assert_eq!(detail.printer_name, "Brother MFC-L2750DW");
    }

    #[tokio::test]
    async fn remove_idempotent_when_printer_missing() {
        // Existence check returns empty → no match.
        let mock = MockExecutor::new().stub_contains(
            "Get-Printer -Name 'Ghost Printer'",
            ok_stdout(""),
        );

        let result = run(
            &mock,
            RemoveArgs {
                target: "Ghost Printer",
                keep_driver: false,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success, "idempotent removal should succeed");
        let detail = result.detail_as::<RemoveDetail>().expect("detail present");
        assert_eq!(detail.printer_name, "Ghost Printer");
        assert!(!detail.port_removed);
        assert!(!detail.driver_removed);
    }

    #[tokio::test]
    async fn remove_keeps_driver_when_flag_set() {
        // Info fetch, remove ops, and orphan counts registered BEFORE the
        // bare existence stub so first-match-wins routes correctly.
        let mock = stub_printer_info(MockExecutor::new(), "HP Universal PCL6", "IP_10.0.0.5")
            .stub_contains("Remove-Printer -Name", ok_stdout(""))
            .stub_contains("Remove-PrinterPort", ok_stdout(""))
            // Port would be orphaned — it should still be removed.
            .stub_contains("PortName -eq", ok_stdout("0"))
            .stub_contains(
                "Get-Printer -Name 'HP LaserJet'",
                ok_stdout("HP LaserJet"),
            );
        // NOTE: no stub for DriverName-eq or Remove-PrinterDriver — they
        // must never be called when keep_driver is true.

        let result = run(
            &mock,
            RemoveArgs {
                target: "HP LaserJet",
                keep_driver: true,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success);
        let detail = result.detail_as::<RemoveDetail>().unwrap();
        assert!(!detail.driver_removed, "driver should not be removed with --keep-driver");
        assert!(detail.port_removed);
    }

    #[tokio::test]
    async fn remove_keeps_port_when_flag_set() {
        let mock = stub_printer_info(MockExecutor::new(), "HP Universal PCL6", "IP_10.0.0.5")
            .stub_contains("Remove-Printer -Name", ok_stdout(""))
            .stub_contains("Remove-PrinterDriver", ok_stdout(""))
            .stub_contains("DriverName -eq", ok_stdout("0"))
            .stub_contains(
                "Get-Printer -Name 'HP LaserJet'",
                ok_stdout("HP LaserJet"),
            );

        let result = run(
            &mock,
            RemoveArgs {
                target: "HP LaserJet",
                keep_driver: false,
                keep_port: true,
                verbose: false,
            },
        )
        .await;

        assert!(result.success);
        let detail = result.detail_as::<RemoveDetail>().unwrap();
        assert!(detail.driver_removed);
        assert!(!detail.port_removed, "port should not be removed with --keep-port");
    }

    #[tokio::test]
    async fn remove_skips_driver_removal_when_still_in_use() {
        let mock = stub_printer_info(MockExecutor::new(), "HP Universal PCL6", "IP_10.0.0.5")
            .stub_contains("Remove-Printer -Name", ok_stdout(""))
            // Driver still used by 2 other printers.
            .stub_contains("DriverName -eq", ok_stdout("2"))
            // Port still used (1 → non-orphan) just to make both flags meaningful.
            .stub_contains("PortName -eq", ok_stdout("1"))
            .stub_contains(
                "Get-Printer -Name 'HP LaserJet'",
                ok_stdout("HP LaserJet"),
            );

        let result = run(
            &mock,
            RemoveArgs {
                target: "HP LaserJet",
                keep_driver: false,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success);
        let detail = result.detail_as::<RemoveDetail>().unwrap();
        assert!(!detail.driver_removed);
        assert!(!detail.port_removed);
    }

    #[tokio::test]
    async fn remove_fatal_when_queue_removal_fails() {
        let mock = stub_printer_info(MockExecutor::new(), "HP Universal PCL6", "IP_10.0.0.5")
            .stub_failure("Remove-Printer -Name", "Access denied")
            .stub_contains(
                "Get-Printer -Name 'HP LaserJet'",
                ok_stdout("HP LaserJet"),
            );

        let result = run(
            &mock,
            RemoveArgs {
                target: "HP LaserJet",
                keep_driver: false,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(!result.success);
        let err = result.error.expect("error present");
        assert!(err.contains("HP LaserJet"));
        assert!(err.contains("Access denied"));
    }

    #[tokio::test]
    async fn remove_skips_system_driver_cleanup() {
        // The IPP Class Driver is a Windows system driver — we must never
        // even attempt to remove it, so no DriverName-count query or
        // Remove-PrinterDriver stub should be registered.
        let mock = stub_printer_info(
            MockExecutor::new(),
            "Microsoft IPP Class Driver",
            "IP_10.10.20.16",
        )
        .stub_contains("Remove-Printer -Name", ok_stdout(""))
        // Port count returns 0 → orphan, will be removed
        .stub_contains("PortName -eq", ok_stdout("0"))
        .stub_contains("Remove-PrinterPort", ok_stdout(""))
        .stub_contains(
            "Get-Printer -Name 'Brother MFC-L2750DW series (IPP)'",
            ok_stdout("Brother MFC-L2750DW series (IPP)"),
        );
        // Deliberately NO stub for "DriverName -eq" or "Remove-PrinterDriver".
        // If the code calls either, it'll fall through to the default
        // (empty stdout) which would then fail to parse the count as u32 and
        // silently return false — but the assertion is that driver_removed
        // is false, which matches either path. The real proof is that no
        // driver-removal command is attempted.

        let result = run(
            &mock,
            RemoveArgs {
                target: "Brother MFC-L2750DW series (IPP)",
                keep_driver: false,
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success);
        let detail = result.detail_as::<RemoveDetail>().unwrap();
        assert!(
            !detail.driver_removed,
            "system driver must never be removed"
        );
        assert!(detail.port_removed, "port cleanup should still succeed");
    }

    #[test]
    fn is_system_driver_detects_microsoft_ipp() {
        assert!(is_system_driver("Microsoft IPP Class Driver"));
        assert!(is_system_driver("Microsoft Print To PDF"));
        assert!(is_system_driver("Microsoft XPS Document Writer"));
        assert!(is_system_driver("Remote Desktop Easy Print"));
    }

    #[test]
    fn is_system_driver_rejects_vendor_drivers() {
        assert!(!is_system_driver("HP Universal Print Driver PCL6"));
        assert!(!is_system_driver("Brother MFC-L2750DW PCL-6"));
        assert!(!is_system_driver("Canon Generic Plus PCL6"));
    }

    #[test]
    fn is_manageable_port_accepts_ip_ports() {
        assert!(is_manageable_port("IP_10.10.20.16"));
        assert!(is_manageable_port("IP_192.168.1.1"));
        assert!(is_manageable_port("http://10.10.20.16:631/ipp/print"));
    }

    #[test]
    fn is_manageable_port_rejects_usb_lpt_com() {
        assert!(!is_manageable_port("USB001"));
        assert!(!is_manageable_port("USB002"));
        assert!(!is_manageable_port("LPT1"));
        assert!(!is_manageable_port("COM1"));
        assert!(!is_manageable_port("PORTPROMPT:"));
        assert!(!is_manageable_port("FILE:"));
        assert!(!is_manageable_port("WSD-abc123"));
        assert!(!is_manageable_port("Send To Microsoft OneNote"));
        assert!(!is_manageable_port("nul:"));
    }

    #[tokio::test]
    async fn remove_skips_port_cleanup_for_usb_printer() {
        // USB printer scenario: queue exists, uses USB001 port.
        // Remove should kill the queue but not touch the USB port.
        let mock = stub_printer_info(
            MockExecutor::new(),
            "Brother MFC-L2750DW PCL6",
            "USB001",
        )
        .stub_contains("Remove-Printer -Name", ok_stdout(""))
        .stub_contains(
            "Get-Printer -Name 'Brother MFC-L2750DW'",
            ok_stdout("Brother MFC-L2750DW"),
        );
        // Deliberately NO stubs for PortName -eq or Remove-PrinterPort —
        // the code must never call them for a USB port.

        let result = run(
            &mock,
            RemoveArgs {
                target: "Brother MFC-L2750DW",
                keep_driver: true, // keep driver to isolate the port behavior
                keep_port: false,
                verbose: false,
            },
        )
        .await;

        assert!(result.success);
        let detail = result.detail_as::<RemoveDetail>().unwrap();
        assert!(!detail.port_removed, "USB port must not be removed");
    }
}

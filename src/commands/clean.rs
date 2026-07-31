//! The `clean` command — remove multiple printer queues in one pass.
//!
//! MSP desks often accumulate several queues for one physical printer
//! (vendor UPD, PS, IPP Class Driver, wrong model series, FAX). `remove`
//! resolves a single queue; `clean` enumerates matches and removes each,
//! then sweeps orphan ports for the target IP when applicable.
//!
//! Matching modes (at least one required):
//! - positional IP — every queue whose port name contains that address
//! - `--name-match <regex>` — queue name and/or driver name
//! - `--manufacturer <str>` — case-insensitive substring on driver/name

use std::net::Ipv4Addr;

use regex::Regex;
use serde::Serialize;

use crate::commands::remove::{self, RemoveArgs};
use crate::core::executor::PsExecutor;
use crate::discovery::local;
use crate::installer::powershell::escape_ps_string;
/// Arguments for `prinstall clean`.
pub struct CleanArgs<'a> {
    /// Optional IPv4 — match queues whose port name contains this address.
    pub ip: Option<&'a str>,
    /// Optional regex matched against queue name and driver name.
    pub name_match: Option<&'a str>,
    /// Optional manufacturer substring (case-insensitive).
    pub manufacturer: Option<&'a str>,
    pub keep_driver: bool,
    pub keep_port: bool,
    pub verbose: bool,
}

#[derive(Debug, Serialize)]
pub struct CleanResult {
    pub removed: Vec<CleanQueueResult>,
    pub skipped: Vec<String>,
    pub ports_swept: Vec<String>,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct CleanQueueResult {
    pub queue: String,
    pub success: bool,
    pub detail: String,
}

pub async fn run(executor: &dyn PsExecutor, args: CleanArgs<'_>) -> CleanResult {
    let verbose = args.verbose;

    if args.ip.is_none() && args.name_match.is_none() && args.manufacturer.is_none() {
        return CleanResult {
            removed: vec![],
            skipped: vec![],
            ports_swept: vec![],
            success: false,
            message: "clean requires an IP, --name-match, and/or --manufacturer".into(),
        };
    }

    let name_re = match args.name_match {
        Some(pat) => match Regex::new(&format!("(?i){pat}")) {
            Ok(r) => Some(r),
            Err(e) => {
                return CleanResult {
                    removed: vec![],
                    skipped: vec![],
                    ports_swept: vec![],
                    success: false,
                    message: format!("invalid --name-match regex: {e}"),
                };
            }
        },
        None => None,
    };

    let ip_filter: Option<Ipv4Addr> = match args.ip {
        Some(s) => match s.parse() {
            Ok(ip) => Some(ip),
            Err(_) => {
                return CleanResult {
                    removed: vec![],
                    skipped: vec![],
                    ports_swept: vec![],
                    success: false,
                    message: format!("'{s}' is not a valid IPv4 address"),
                };
            }
        },
        None => None,
    };

    let mfr_needle = args
        .manufacturer
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    // Enumerate local queues (no admin required for the read).
    let printers = local::list_local_printers(verbose);
    let mut targets: Vec<String> = Vec::new();

    for p in &printers {
        let Some(queue) = p.local_name.as_deref() else {
            continue;
        };
        let driver = p.driver_name.as_deref().unwrap_or("");
        let port = p.port_name.as_deref().unwrap_or("");

        let ip_hit = ip_filter.is_none_or(|ip| port_targets_ip(port, ip));
        let re_hit = name_re
            .as_ref()
            .is_none_or(|re| re.is_match(queue) || re.is_match(driver));
        let mfr_hit = mfr_needle
            .as_ref()
            .is_none_or(|n| queue.to_ascii_lowercase().contains(n) || driver.to_ascii_lowercase().contains(n));

        // When multiple filters are set, all must match (AND).
        if ip_hit && re_hit && mfr_hit {
            // At least one filter was requested — already validated above.
            targets.push(queue.to_string());
        }
    }

    targets.sort();
    targets.dedup();

    if targets.is_empty() {
        return CleanResult {
            removed: vec![],
            skipped: vec![],
            ports_swept: vec![],
            success: true,
            message: "No matching printer queues found.".into(),
        };
    }

    if verbose {
        eprintln!("[clean] Matched {} queue(s):", targets.len());
        for t in &targets {
            eprintln!("[clean]   • {t}");
        }
    }

    let mut removed = Vec::new();
    let mut all_ok = true;

    for queue in &targets {
        let result = remove::run(
            executor,
            RemoveArgs {
                target: queue,
                keep_driver: args.keep_driver,
                keep_port: true, // sweep ports once at the end for IP cleans
                verbose,
            },
        )
        .await;

        if !result.success {
            all_ok = false;
        }
        removed.push(CleanQueueResult {
            queue: queue.clone(),
            success: result.success,
            detail: result
                .error
                .clone()
                .unwrap_or_else(|| if result.success { "ok".into() } else { "failed".into() }),
        });
    }

    // After queues are gone, optionally clean orphan drivers per removed queue
    // was already handled inside remove (unless keep_driver). Port sweep for IP:
    let mut ports_swept = Vec::new();
    if let Some(ip) = ip_filter {
        if !args.keep_port {
            ports_swept = sweep_ports_for_ip(executor, ip, verbose);
        }
    }

    let ok_count = removed.iter().filter(|r| r.success).count();
    let message = format!(
        "Removed {ok_count}/{} matching queue(s).{}",
        removed.len(),
        if ports_swept.is_empty() {
            String::new()
        } else {
            format!(" Swept ports: {}.", ports_swept.join(", "))
        }
    );

    CleanResult {
        removed,
        skipped: vec![],
        ports_swept,
        success: all_ok,
        message,
    }
}

/// Port names that belong to a given host address.
///
/// Covers prinstall's `IP_<ip>` convention plus bare IP ports, `(1)` suffixes,
/// and `IP_<ip>_1` / `<ip>_1` variants vendors leave behind.
pub fn port_targets_ip(port_name: &str, ip: Ipv4Addr) -> bool {
    let ip_s = ip.to_string();
    if port_name == ip_s || port_name == format!("IP_{ip_s}") {
        return true;
    }
    if let Some(rest) = port_name.strip_prefix("IP_") {
        if rest == ip_s || rest.starts_with(&format!("{ip_s}(")) || rest.starts_with(&format!("{ip_s}_"))
        {
            return true;
        }
    }
    port_name.starts_with(&format!("{ip_s}("))
        || port_name.starts_with(&format!("{ip_s}_"))
        || port_name.contains(&format!("_{ip_s}"))
}

fn sweep_ports_for_ip(executor: &dyn PsExecutor, ip: Ipv4Addr, verbose: bool) -> Vec<String> {
    let ip_s = ip.to_string();
    // List ports whose name mentions the IP, remove if no queue references them.
    let list_cmd = format!(
        "Get-PrinterPort | Where-Object {{ $_.Name -like '*{ip_s}*' }} | Select-Object -ExpandProperty Name"
    );
    if verbose {
        eprintln!("[clean] Listing ports for {ip_s}: {list_cmd}");
    }
    let listed = executor.run(&list_cmd);
    if !listed.success {
        return vec![];
    }
    let mut swept = Vec::new();
    for port in listed.stdout.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let count_cmd = format!(
            "(Get-Printer | Where-Object {{ $_.PortName -eq '{}' }} | Measure-Object).Count",
            escape_ps_string(port)
        );
        let count_result = executor.run(&count_cmd);
        let count: u32 = count_result
            .stdout
            .trim()
            .parse()
            .unwrap_or(1);
        if count > 0 {
            if verbose {
                eprintln!("[clean] Port '{port}' still referenced ({count}); skipping");
            }
            continue;
        }
        let rm = format!(
            "Remove-PrinterPort -Name '{}' -ErrorAction Stop",
            escape_ps_string(port)
        );
        if verbose {
            eprintln!("[clean] Removing orphan port: {rm}");
        }
        let mut result = executor.run(&rm);
        // Ghost-port recovery: spooler claims "in use" with zero queues.
        if !result.success {
            if verbose {
                eprintln!(
                    "[clean] Port remove failed; restarting Spooler and retrying once: {}",
                    result.stderr.trim()
                );
            }
            let _ = executor.run("Restart-Service Spooler -Force");
            std::thread::sleep(std::time::Duration::from_secs(2));
            result = executor.run(&rm);
        }
        if result.success {
            swept.push(port.to_string());
        } else if verbose {
            eprintln!(
                "[clean] Could not remove port '{port}': {}",
                result.stderr.trim()
            );
        }
    }
    swept
}

/// Format a human-readable clean summary (non-JSON).
pub fn format_clean_result(result: &CleanResult) -> String {
    let mut out = String::new();
    out.push_str(&result.message);
    out.push('\n');
    for r in &result.removed {
        let mark = if r.success { "✓" } else { "✗" };
        out.push_str(&format!("  {mark} {}\n", r.queue));
        if !r.success && !r.detail.is_empty() {
            out.push_str(&format!("      {}\n", r.detail));
        }
    }
    for p in &result.ports_swept {
        out.push_str(&format!("  ✓ port {p}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn port_targets_ip_matches_common_variants() {
        let ip: Ipv4Addr = "192.168.0.199".parse().unwrap();
        assert!(port_targets_ip("IP_192.168.0.199", ip));
        assert!(port_targets_ip("192.168.0.199", ip));
        assert!(port_targets_ip("IP_192.168.0.199(1)", ip));
        assert!(port_targets_ip("192.168.0.199_1", ip));
        assert!(!port_targets_ip("IP_192.168.0.200", ip));
        assert!(!port_targets_ip("USB001", ip));
    }
}

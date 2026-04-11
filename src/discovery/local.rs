use std::net::Ipv4Addr;
use crate::installer::powershell;
use crate::models::*;

/// Extract an IPv4 address from a Windows printer port name.
pub fn extract_ip_from_port_name(port_name: &str) -> Option<Ipv4Addr> {
    let candidate = port_name
        .strip_prefix("IP_")
        .or_else(|| port_name.strip_prefix("TCPMON:"));
    candidate.and_then(|s| s.parse().ok())
}

/// Parse the structured key/value output produced by `list_local_printers`
/// into `Printer` records. Accepts the original three-field format
/// (Name / DriverName / PortName) for backwards compatibility with the
/// existing test fixtures, and the richer format emitted by the 0.3.2
/// list flow (adds Shared / Default / Status / Location).
pub fn parse_get_printer_output(output: &str) -> Vec<Printer> {
    let mut printers = Vec::new();

    for block in output.split("---") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut name = None;
        let mut driver = None;
        let mut port = None;
        let mut shared: Option<bool> = None;
        let mut is_default: Option<bool> = None;
        let mut status_raw: Option<String> = None;

        for line in block.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("Name: ") {
                name = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("DriverName: ") {
                driver = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("PortName: ") {
                port = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("Shared: ") {
                shared = Some(parse_bool(val));
            } else if let Some(val) = line.strip_prefix("Default: ") {
                is_default = Some(parse_bool(val));
            } else if let Some(val) = line.strip_prefix("Status: ") {
                status_raw = Some(val.trim().to_string());
            }
        }

        let Some(printer_name) = name else { continue };
        let port_str = port.as_deref().unwrap_or("");
        let ip = extract_ip_from_port_name(port_str);
        let is_usb = port_str.starts_with("USB") || port_str.starts_with("usb");

        let source = if is_usb {
            PrinterSource::Usb
        } else {
            PrinterSource::Installed
        };

        let status = status_raw
            .as_deref()
            .map(map_win32_printer_status)
            .unwrap_or(PrinterStatus::Ready);

        printers.push(Printer {
            ip,
            model: driver.clone(),
            serial: None,
            status,
            discovery_methods: vec![DiscoveryMethod::Local],
            ports: vec![],
            source,
            local_name: Some(printer_name),
            port_name: port.clone(),
            driver_name: driver,
            shared,
            is_default,
        });
    }

    printers
}

/// Parse a PowerShell-style boolean ("True"/"False", case insensitive).
fn parse_bool(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes")
}

/// Map a Win32_Printer PrinterStatus integer (as a string) to our
/// internal `PrinterStatus` enum. Ref:
/// https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-printer
///   1 = Other, 2 = Unknown, 3 = Idle, 4 = Printing, 5 = Warmup,
///   6 = Stopped Printing, 7 = Offline
/// Accepts either a numeric string or a human-readable label so the
/// parser tolerates both Get-Printer and Win32_Printer output shapes.
pub fn map_win32_printer_status(raw: &str) -> PrinterStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "3" | "idle" | "normal" | "ready" => PrinterStatus::Ready,
        "4" | "printing" | "warming up" | "warmup" | "5" => PrinterStatus::Ready,
        "6" | "stopped printing" | "stopped" | "paused" => PrinterStatus::Error,
        "7" | "offline" => PrinterStatus::Offline,
        "2" | "unknown" => PrinterStatus::Unknown,
        _ => PrinterStatus::Ready,
    }
}

/// List locally installed printers via `Get-CimInstance Win32_Printer`.
///
/// Win32_Printer is the CIM class backing the classic "Printers" control
/// panel. Compared to `Get-Printer` it gives us `Default` (whether the
/// queue is the Windows default), `Shared`, and a structured status
/// code in a single query. The output is serialized as the same simple
/// key: value blocks that the 0.3.x parser already understands, with a
/// few extra fields that land as optional data on the Printer struct.
pub fn list_local_printers(verbose: bool) -> Vec<Printer> {
    let cmd = "Get-CimInstance -ClassName Win32_Printer | ForEach-Object { \
        \"Name: $($_.Name)\"; \
        \"DriverName: $($_.DriverName)\"; \
        \"PortName: $($_.PortName)\"; \
        \"Shared: $($_.Shared)\"; \
        \"Default: $($_.Default)\"; \
        \"Status: $($_.PrinterStatus)\"; \
        '---' \
    }";
    let result = powershell::run_ps(cmd, verbose);
    if !result.success {
        if verbose {
            eprintln!("[local] Win32_Printer query failed: {}", result.stderr);
        }
        return Vec::new();
    }
    parse_get_printer_output(&result.stdout)
}

/// Deduplicate local printers against network-discovered printers.
pub fn deduplicate(local: Vec<Printer>, network: &[Printer]) -> Vec<Printer> {
    let network_ips: std::collections::HashSet<Ipv4Addr> = network
        .iter()
        .filter_map(|p| p.ip)
        .collect();

    local
        .into_iter()
        .filter(|p| {
            match p.ip {
                Some(ip) => !network_ips.contains(&ip),
                None => true,
            }
        })
        .collect()
}

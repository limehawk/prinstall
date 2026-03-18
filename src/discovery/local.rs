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

/// Parse the structured output from Get-Printer into Printer structs.
pub fn parse_get_printer_output(output: &str) -> Vec<Printer> {
    let mut printers = Vec::new();

    for block in output.split("---") {
        let block = block.trim();
        if block.is_empty() { continue; }

        let mut name = None;
        let mut driver = None;
        let mut port = None;

        for line in block.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("Name: ") {
                name = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("DriverName: ") {
                driver = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("PortName: ") {
                port = Some(val.trim().to_string());
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

        printers.push(Printer {
            ip,
            model: driver,
            serial: None,
            status: PrinterStatus::Ready,
            discovery_methods: vec![DiscoveryMethod::Local],
            ports: vec![],
            source,
            local_name: Some(printer_name),
        });
    }

    printers
}

/// List locally installed printers via PowerShell Get-Printer.
pub fn list_local_printers(verbose: bool) -> Vec<Printer> {
    let cmd = "Get-Printer | ForEach-Object { \
        \"Name: $($_.Name)\"; \
        \"DriverName: $($_.DriverName)\"; \
        \"PortName: $($_.PortName)\"; \
        \"Shared: $($_.Shared)\"; \
        '---' \
    }";
    let result = powershell::run_ps(cmd, verbose);
    if !result.success {
        if verbose {
            eprintln!("[local] Get-Printer failed: {}", result.stderr);
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

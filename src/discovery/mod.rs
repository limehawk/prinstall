pub mod ipp;
pub mod local;
pub mod mdns;
pub mod port_scan;
pub mod snmp;
pub mod subnet;
pub mod usb;

use std::net::Ipv4Addr;
use std::time::Duration;
use crate::core::executor::PsExecutor;
use crate::models::{DiscoveryMethod, Printer, PrinterSource, ScanResult};

const DEFAULT_CONCURRENCY: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum ScanMethod {
    /// Full multi-method scan: port probe + IPP + SNMP (unicast, per-host)
    /// plus an mDNS multicast browse. This is the default — running
    /// `prinstall scan` with no flags picks this mode.
    All,
    /// SNMP-only unicast probe against every host on the subnet.
    Snmp,
    /// TCP port-check probe against 9100/631/515 per host.
    Port,
    /// mDNS-only multicast browse. Ignores the subnet argument entirely
    /// since mDNS runs on the link, not on a specific host range.
    Mdns,
}

/// How long the mDNS browse pass waits for multicast responses before
/// giving up. Most responsive printers answer in under 1s; 3s is a
/// reasonable ceiling for a subnet scan without being noticeably slow.
const MDNS_BROWSE_TIMEOUT: Duration = Duration::from_secs(3);

/// Multi-method scan pipeline.
///
/// `ScanMethod::All` runs every discovery path we have — including the
/// mDNS multicast browse — so a bare `prinstall scan` surfaces as many
/// printers as possible. The narrower methods (`Snmp`, `Port`, `Mdns`)
/// run exactly what they name and nothing else.
pub async fn scan_subnet(
    hosts: Vec<Ipv4Addr>,
    community: &str,
    method: &ScanMethod,
    timeout: Duration,
    verbose: bool,
) -> Vec<Printer> {
    let mut printers = match method {
        ScanMethod::Snmp => scan_snmp_only(hosts, community, verbose).await,
        ScanMethod::Port => scan_port_only(hosts, timeout, verbose).await,
        ScanMethod::Mdns => Vec::new(),
        ScanMethod::All => scan_all(hosts, community, timeout, verbose).await,
    };

    if matches!(method, ScanMethod::All | ScanMethod::Mdns) {
        let mdns_printers = mdns::discover(MDNS_BROWSE_TIMEOUT, verbose).await;
        merge_mdns_results(&mut printers, mdns_printers);
    }

    printers.sort_by(|a, b| a.ip.cmp(&b.ip));
    printers
}

/// Merge mDNS-discovered printers into the main result list. For IPs
/// already known from port/SNMP/IPP, attach the `Mdns` method and
/// backfill any missing model name. For new IPs, append the mDNS entry
/// as-is so silent printers become visible.
fn merge_mdns_results(main: &mut Vec<Printer>, mdns_printers: Vec<Printer>) {
    for mdns_printer in mdns_printers {
        let Some(mdns_ip) = mdns_printer.ip else { continue };
        if let Some(existing) = main.iter_mut().find(|p| p.ip == Some(mdns_ip)) {
            if !existing.discovery_methods.contains(&DiscoveryMethod::Mdns) {
                existing.discovery_methods.push(DiscoveryMethod::Mdns);
            }
            if existing.model.is_none() {
                existing.model = mdns_printer.model;
            }
            if existing.local_name.is_none() {
                existing.local_name = mdns_printer.local_name;
            }
            for port in mdns_printer.ports {
                if !existing.ports.contains(&port) {
                    existing.ports.push(port);
                }
            }
        } else {
            main.push(mdns_printer);
        }
    }
}

/// SNMP-only scan (legacy behavior).
async fn scan_snmp_only(hosts: Vec<Ipv4Addr>, community: &str, verbose: bool) -> Vec<Printer> {
    use tokio::sync::Semaphore;
    use std::sync::Arc;

    let semaphore = Arc::new(Semaphore::new(64));
    let mut handles = Vec::new();
    let community = community.to_string();

    for ip in hosts {
        let sem = semaphore.clone();
        let comm = community.clone();
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            snmp::identify_printer(ip, &comm, verbose).await
        });
        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(printer)) = handle.await {
            results.push(printer);
        }
    }
    results
}

/// Port-scan only (no identification).
async fn scan_port_only(hosts: Vec<Ipv4Addr>, timeout: Duration, verbose: bool) -> Vec<Printer> {
    let candidates = port_scan::scan_ports(hosts, timeout, DEFAULT_CONCURRENCY, verbose).await;
    candidates
        .into_iter()
        .map(|c| Printer {
            ip: Some(c.ip),
            model: None,
            serial: None,
            status: crate::models::PrinterStatus::Unknown,
            discovery_methods: vec![DiscoveryMethod::PortScan],
            ports: c.open_ports,
            source: PrinterSource::Network,
            local_name: None,
            port_name: None,
            driver_name: None,
            shared: None,
            is_default: None,
        })
        .collect()
}

/// Full multi-method scan: port probe → IPP → SNMP.
async fn scan_all(
    hosts: Vec<Ipv4Addr>,
    community: &str,
    timeout: Duration,
    verbose: bool,
) -> Vec<Printer> {
    // Phase 1: Port scan
    let candidates = port_scan::scan_ports(hosts, timeout, DEFAULT_CONCURRENCY, verbose).await;

    if verbose {
        eprintln!("[scan] Port scan found {} candidates", candidates.len());
    }

    // Phase 2: Identify each candidate via IPP + SNMP
    use tokio::sync::Semaphore;
    use std::sync::Arc;

    let semaphore = Arc::new(Semaphore::new(32));
    let mut handles = Vec::new();
    let community = community.to_string();

    for candidate in candidates {
        let sem = semaphore.clone();
        let comm = community.clone();
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let ip = candidate.ip;
            let mut methods = vec![DiscoveryMethod::PortScan];
            let mut model: Option<String> = None;
            let mut serial: Option<String> = None;
            let mut status = crate::models::PrinterStatus::Unknown;

            // Try IPP first
            if candidate.open_ports.contains(&631)
                && let Some(ipp_model) = ipp::identify_printer_ipp(ip, verbose).await
            {
                model = Some(ipp_model);
                methods.push(DiscoveryMethod::Ipp);
            }

            // Try SNMP for enrichment
            if let Some(snmp_printer) = snmp::identify_printer(ip, &comm, verbose).await {
                methods.push(DiscoveryMethod::Snmp);
                if model.is_none() {
                    model = snmp_printer.model;
                }
                serial = snmp_printer.serial;
                status = snmp_printer.status;
            }

            Printer {
                ip: Some(ip),
                model,
                serial,
                status,
                discovery_methods: methods,
                ports: candidate.open_ports,
                source: PrinterSource::Network,
                local_name: None,
                port_name: None,
                driver_name: None,
                shared: None,
                is_default: None,
            }
        });
        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(printer) = handle.await {
            results.push(printer);
        }
    }
    results
}

/// Scan network + enumerate local printers, deduplicate.
pub async fn full_discovery(
    hosts: Vec<Ipv4Addr>,
    community: &str,
    method: &ScanMethod,
    timeout: Duration,
    verbose: bool,
) -> Vec<Printer> {
    let mut network = scan_subnet(hosts, community, method, timeout, verbose).await;

    let local = local::list_local_printers(verbose);
    let unique_local = local::deduplicate(local, &network);

    if verbose && !unique_local.is_empty() {
        eprintln!("[scan] Found {} local/USB printers", unique_local.len());
    }

    network.extend(unique_local);
    network
}

/// Full scan that combines a network subnet sweep with USB PnP enumeration
/// and returns both sections separately. Callers render them as two
/// distinct lists so an orphan USB printer (no queue) is obvious.
pub async fn full_scan_result(
    hosts: Vec<Ipv4Addr>,
    community: &str,
    method: &ScanMethod,
    timeout: Duration,
    exec: &dyn PsExecutor,
    verbose: bool,
) -> ScanResult {
    let network = scan_subnet(hosts, community, method, timeout, verbose).await;
    let usb = usb::enumerate(exec, verbose).await;
    ScanResult { network, usb }
}

#[cfg(test)]
mod full_scan_tests {
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
    async fn full_scan_result_includes_usb_section() {
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok("[]"))
            .stub_contains("Get-Printer", ok("[]"));
        let result = full_scan_result(
            vec![],
            "public",
            &ScanMethod::All,
            std::time::Duration::from_millis(50),
            &mock,
            false,
        ).await;
        assert!(result.network.is_empty());
        assert!(result.usb.is_empty());
    }
}

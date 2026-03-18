pub mod local;
pub mod snmp;
pub mod subnet;

use std::net::Ipv4Addr;
use crate::models::Printer;

/// Discovery method for subnet scanning.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanMethod {
    /// Run all discovery methods and merge results.
    All,
    /// SNMP-only discovery via UDP port 161.
    Snmp,
    /// TCP port-check discovery (port 9100).
    Port,
}

/// Scan a list of IPs for printers. Max 64 concurrent probes.
pub async fn scan_subnet(
    hosts: Vec<Ipv4Addr>,
    community: &str,
    _method: &ScanMethod,
    _timeout: std::time::Duration,
    _verbose: bool,
) -> Vec<Printer> {
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
            snmp::identify_printer(ip, &comm).await
        });
        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(printer)) = handle.await {
            results.push(printer);
        }
    }

    // Sort by IP numerically for consistent output
    results.sort_by(|a, b| {
        let a_ip: std::net::Ipv4Addr = a.ip.parse().unwrap_or(Ipv4Addr::UNSPECIFIED);
        let b_ip: std::net::Ipv4Addr = b.ip.parse().unwrap_or(Ipv4Addr::UNSPECIFIED);
        a_ip.cmp(&b_ip)
    });
    results
}

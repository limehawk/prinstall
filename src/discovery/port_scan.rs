use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use std::sync::Arc;

/// Printer-related TCP ports to probe.
pub const PRINTER_PORTS: &[u16] = &[9100, 631, 515];

/// Result of probing a single IP for open printer ports.
#[derive(Debug, Clone)]
pub struct PortScanResult {
    pub ip: Ipv4Addr,
    pub open_ports: Vec<u16>,
}

/// Probe a single port on a single IP. Returns true if port is open.
async fn probe_port(ip: Ipv4Addr, port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::new(ip.into(), port);
    tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

/// Scan a single IP for all printer ports. Returns None if no ports open.
async fn scan_host(ip: Ipv4Addr, timeout: Duration, verbose: bool) -> Option<PortScanResult> {
    let mut open_ports = Vec::new();
    for &port in PRINTER_PORTS {
        if probe_port(ip, port, timeout).await {
            if verbose {
                eprintln!("[scan] {ip}: port {port} open");
            }
            open_ports.push(port);
        }
    }
    if open_ports.is_empty() {
        if verbose {
            eprintln!("[scan] {ip}: all ports closed — skipping");
        }
        None
    } else {
        Some(PortScanResult { ip, open_ports })
    }
}

/// Scan a list of IPs for open printer ports. Max `max_concurrent` simultaneous connections.
pub async fn scan_ports(
    hosts: Vec<Ipv4Addr>,
    timeout: Duration,
    max_concurrent: usize,
    verbose: bool,
) -> Vec<PortScanResult> {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut handles = Vec::new();

    for ip in hosts {
        let sem = semaphore.clone();
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            scan_host(ip, timeout, verbose).await
        });
        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(result)) = handle.await {
            results.push(result);
        }
    }

    results.sort_by_key(|r| r.ip);
    results
}

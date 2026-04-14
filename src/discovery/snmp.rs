use std::net::Ipv4Addr;
use std::time::Duration;
use csnmp::{ObjectIdentifier, ObjectValue, Snmp2cClient};

use crate::models::{DiscoveryMethod, Printer, PrinterSource, PrinterStatus};

/// OID for hrDeviceDescr (device description / model)
const OID_DEVICE_DESCR: &str = "1.3.6.1.2.1.25.3.2.1.3.1";
/// OID for sysDescr (system description, fallback)
const OID_SYS_DESCR: &str = "1.3.6.1.2.1.1.1.0";
/// OID for serial number (prtGeneralSerialNumber)
const OID_SERIAL: &str = "1.3.6.1.2.1.43.5.1.1.17.1";
/// OID for hrPrinterStatus
const OID_PRINTER_STATUS: &str = "1.3.6.1.2.1.25.3.5.1.1.1";

const SNMP_TIMEOUT: Duration = Duration::from_secs(2);

/// Number of attempts per OID query (1 initial + 2 retries).
/// SNMP over UDP is lossy — packets get dropped on busy networks and the
/// "first scan misses, second scan hits" bug comes from exactly this.
/// Bumping to 3 tries per OID puts total worst-case latency at roughly
/// 3 × 500 ms timeout + 2 × 100 ms backoff = ~1.7 s per host. Still fits
/// comfortably inside the 64-way parallelism budget on a /24 scan.
const SNMP_MAX_ATTEMPTS: u32 = 3;

/// Backoff between retries. Short enough that we stay well under the
/// per-host budget, long enough to ride out transient congestion.
const SNMP_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Query a single printer via SNMP and return its details.
pub async fn identify_printer(ip: Ipv4Addr, community: &str, verbose: bool) -> Option<Printer> {
    let addr = format!("{ip}:161").parse().ok()?;
    let bind_addr: std::net::SocketAddr = "0.0.0.0:0".parse().unwrap();
    let client = match Snmp2cClient::new(addr, community.as_bytes().to_vec(), Some(bind_addr), Some(SNMP_TIMEOUT), 1).await {
        Ok(c) => c,
        Err(e) => {
            if verbose {
                eprintln!("[scan] {ip}: SNMP client error: {e}");
            }
            return None;
        }
    };

    let model = match snmp_get_string(&client, OID_DEVICE_DESCR, ip, verbose).await {
        Some(m) => Some(m),
        None => snmp_get_string(&client, OID_SYS_DESCR, ip, verbose).await,
    };

    if model.is_none() {
        if verbose {
            eprintln!("[scan] {ip}: SNMP → no model string");
        }
        return None;
    }

    if verbose {
        eprintln!("[scan] {ip}: SNMP → model {:?}", model.as_deref().unwrap_or("?"));
    }

    let serial = snmp_get_string(&client, OID_SERIAL, ip, verbose).await;
    let status = snmp_get_printer_status(&client, ip, verbose).await;

    Some(Printer {
        ip: Some(ip),
        model,
        serial,
        status,
        discovery_methods: vec![DiscoveryMethod::Snmp],
        ports: vec![],
        source: PrinterSource::Network,
        local_name: None,
        port_name: None,
        driver_name: None,
        shared: None,
        is_default: None,
    })
}

/// Retry wrapper around a csnmp `.get()`-style op. Tries up to
/// `SNMP_MAX_ATTEMPTS` with `SNMP_RETRY_DELAY` between attempts.
///
/// Designed for the common UDP-loss case: the socket timed out because
/// the packet was dropped, not because the device is offline. If every
/// attempt fails, returns `None` — callers treat that identically to the
/// pre-retry "no SNMP response" path.
async fn snmp_get_with_retry(
    client: &Snmp2cClient,
    oid: ObjectIdentifier,
    oid_label: &str,
    ip: Ipv4Addr,
    verbose: bool,
) -> Option<ObjectValue> {
    for attempt in 1..=SNMP_MAX_ATTEMPTS {
        match client.get(oid).await {
            Ok(v) => return Some(v),
            Err(e) => {
                if verbose && attempt < SNMP_MAX_ATTEMPTS {
                    eprintln!(
                        "[snmp] {ip}: {oid_label} attempt {attempt}/{SNMP_MAX_ATTEMPTS} failed: {e} — retrying"
                    );
                }
                if attempt < SNMP_MAX_ATTEMPTS {
                    tokio::time::sleep(SNMP_RETRY_DELAY).await;
                }
            }
        }
    }
    None
}

async fn snmp_get_string(
    client: &Snmp2cClient,
    oid_str: &str,
    ip: Ipv4Addr,
    verbose: bool,
) -> Option<String> {
    let oid: ObjectIdentifier = oid_str.parse().ok()?;
    let value = snmp_get_with_retry(client, oid, oid_str, ip, verbose).await?;
    if let ObjectValue::String(bytes) = value {
        let s = String::from_utf8_lossy(&bytes).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

async fn snmp_get_printer_status(
    client: &Snmp2cClient,
    ip: Ipv4Addr,
    verbose: bool,
) -> PrinterStatus {
    let oid: ObjectIdentifier = match OID_PRINTER_STATUS.parse() {
        Ok(o) => o,
        Err(_) => return PrinterStatus::Unknown,
    };
    let value = match snmp_get_with_retry(client, oid, OID_PRINTER_STATUS, ip, verbose).await {
        Some(v) => v,
        None => return PrinterStatus::Unknown,
    };
    if let ObjectValue::Integer(status_code) = value {
        return match status_code {
            // hrPrinterStatus: 1=other, 2=unknown, 3=idle, 4=printing, 5=warmup
            3..=5 => PrinterStatus::Ready,
            _ => PrinterStatus::Error,
        };
    }
    PrinterStatus::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generic retry helper that mirrors `snmp_get_with_retry`'s
    /// control flow but takes a plain closure so we can unit-test the
    /// try/sleep/try logic without opening a real UDP socket.
    async fn retry<F, Fut, T, E>(mut op: F) -> Option<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        for attempt in 1..=SNMP_MAX_ATTEMPTS {
            match op().await {
                Ok(v) => return Some(v),
                Err(_) => {
                    if attempt < SNMP_MAX_ATTEMPTS {
                        tokio::time::sleep(SNMP_RETRY_DELAY).await;
                    }
                }
            }
        }
        None
    }

    #[tokio::test]
    async fn retry_returns_first_success() {
        let mut calls = 0u32;
        let result: Option<u32> = retry(|| {
            calls += 1;
            async move { Ok::<u32, &'static str>(42) }
        })
        .await;
        assert_eq!(result, Some(42));
        assert_eq!(calls, 1, "should not retry on first success");
    }

    #[tokio::test]
    async fn retry_eventually_succeeds_after_packet_loss() {
        let mut calls = 0u32;
        let result: Option<u32> = retry(|| {
            calls += 1;
            let n = calls;
            async move {
                if n < 2 {
                    Err("timeout")
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(result, Some(7));
        assert_eq!(calls, 2, "should succeed on second attempt");
    }

    #[tokio::test]
    async fn retry_gives_up_after_max_attempts() {
        let mut calls = 0u32;
        let result: Option<u32> = retry(|| {
            calls += 1;
            async move { Err::<u32, &'static str>("timeout") }
        })
        .await;
        assert_eq!(result, None);
        assert_eq!(calls, SNMP_MAX_ATTEMPTS, "should try exactly MAX_ATTEMPTS times");
    }

    #[test]
    fn max_attempts_and_delay_are_sane() {
        // Sanity budget: 3 × 2s SNMP timeout + 2 × 100ms backoff = ~6.2s worst case.
        // Per-host; runs concurrently across a /24 with 64-way parallelism.
        assert_eq!(SNMP_MAX_ATTEMPTS, 3);
        assert_eq!(SNMP_RETRY_DELAY, Duration::from_millis(100));
    }
}

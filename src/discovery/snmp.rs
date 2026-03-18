use std::net::Ipv4Addr;
use std::time::Duration;
use csnmp::{ObjectIdentifier, Snmp2cClient};

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

/// Query a single printer via SNMP and return its details.
pub async fn identify_printer(ip: Ipv4Addr, community: &str) -> Option<Printer> {
    let addr = format!("{ip}:161").parse().ok()?;
    let client = Snmp2cClient::new(addr, community.as_bytes().to_vec(), None, Some(SNMP_TIMEOUT))
        .await
        .ok()?;

    let model = match snmp_get_string(&client, OID_DEVICE_DESCR).await {
        Some(m) => Some(m),
        None => snmp_get_string(&client, OID_SYS_DESCR).await,
    };

    // If we can't even get a model string, the device isn't a printer
    // or SNMP is misconfigured
    model.as_ref()?;

    let serial = snmp_get_string(&client, OID_SERIAL).await;
    let status = snmp_get_printer_status(&client).await;

    Some(Printer {
        ip: Some(ip),
        model,
        serial,
        status,
        discovery_methods: vec![DiscoveryMethod::Snmp],
        ports: vec![],
        source: PrinterSource::Network,
        local_name: None,
    })
}

async fn snmp_get_string(client: &Snmp2cClient, oid_str: &str) -> Option<String> {
    let oid: ObjectIdentifier = oid_str.parse().ok()?;
    let value = client.get(oid).await.ok()?;
    if let csnmp::ObjectValue::String(bytes) = value {
        let s = String::from_utf8_lossy(&bytes).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

async fn snmp_get_printer_status(client: &Snmp2cClient) -> PrinterStatus {
    let oid: ObjectIdentifier = match OID_PRINTER_STATUS.parse() {
        Ok(o) => o,
        Err(_) => return PrinterStatus::Unknown,
    };
    let value = match client.get(oid).await {
        Ok(v) => v,
        Err(_) => return PrinterStatus::Unknown,
    };
    if let csnmp::ObjectValue::Integer(status_code) = value {
        return match status_code {
            // hrPrinterStatus: 1=other, 2=unknown, 3=idle, 4=printing, 5=warmup
            3..=5 => PrinterStatus::Ready,
            _ => PrinterStatus::Error,
        };
    }
    PrinterStatus::Unknown
}

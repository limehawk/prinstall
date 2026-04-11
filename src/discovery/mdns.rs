//! mDNS / Bonjour service discovery for printers that don't respond to
//! SNMP or raw port probes. Many modern AirPrint-capable printers
//! advertise themselves via `_ipp._tcp.local.` (and friends) but ignore
//! SNMPv2c scans, so they stay invisible to the rest of the discovery
//! pipeline. This module runs a time-boxed mDNS browse pass against the
//! common printer service types and merges any results back into the
//! main `Printer` stream.
//!
//! Scope notes:
//! - Only IPv4 addresses are kept — the rest of prinstall operates on
//!   `Ipv4Addr` everywhere.
//! - Browsing is opt-in via the `--mdns` flag on `prinstall scan`.
//!   Enabling it by default would change discovery semantics and belongs
//!   in a minor release, not a patch.
//! - WS-Discovery is a separate protocol and lives behind a future
//!   roadmap item (no good pure-Rust crate exists today).

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use crate::models::{DiscoveryMethod, Printer, PrinterSource, PrinterStatus};

/// Service types advertised by network printers via mDNS. Order matters —
/// `_ipp._tcp.local.` is the canonical AirPrint service and takes
/// priority when multiple types resolve for the same IP.
pub const PRINTER_SERVICE_TYPES: &[&str] = &[
    "_ipp._tcp.local.",
    "_ipps._tcp.local.",
    "_pdl-datastream._tcp.local.",
    "_printer._tcp.local.",
];

/// Minimal printer record extracted from an mDNS service announcement.
/// Kept as a plain data struct so the parsing/merging logic is testable
/// without spinning up a real `ServiceDaemon`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsPrinter {
    pub ip: Ipv4Addr,
    pub hostname: String,
    pub service_type: String,
    pub port: u16,
    pub model: Option<String>,
    pub device_id: Option<String>,
}

impl MdnsPrinter {
    /// Convert the mDNS record into a `Printer` ready to merge into the
    /// scan pipeline.
    pub fn into_printer(self) -> Printer {
        let canonical_port = canonical_port_for(&self.service_type).unwrap_or(self.port);
        Printer {
            ip: Some(self.ip),
            model: self.model,
            serial: None,
            status: PrinterStatus::Unknown,
            discovery_methods: vec![DiscoveryMethod::Mdns],
            ports: vec![canonical_port],
            source: PrinterSource::Network,
            local_name: Some(self.hostname),
            port_name: None,
            driver_name: None,
            shared: None,
            is_default: None,
        }
    }
}

/// Map an mDNS service type to its canonical TCP port. Falls back to the
/// port advertised in the service record when the type is unrecognized.
pub fn canonical_port_for(service_type: &str) -> Option<u16> {
    match service_type {
        "_ipp._tcp.local." | "_ipps._tcp.local." => Some(631),
        "_pdl-datastream._tcp.local." => Some(9100),
        "_printer._tcp.local." => Some(515),
        _ => None,
    }
}

/// Preference score for picking the "best" service type when an IP
/// advertises multiple services. IPP wins because it carries the most
/// descriptive TXT metadata (model, device-id, PDL list).
pub fn service_priority(service_type: &str) -> u8 {
    match service_type {
        "_ipp._tcp.local." => 4,
        "_ipps._tcp.local." => 3,
        "_pdl-datastream._tcp.local." => 2,
        "_printer._tcp.local." => 1,
        _ => 0,
    }
}

/// Collapse multiple mDNS records for the same IP into a single entry,
/// keeping the highest-priority service type and the union of available
/// model/device-id metadata.
pub fn merge_by_ip(entries: Vec<MdnsPrinter>) -> Vec<MdnsPrinter> {
    let mut by_ip: HashMap<Ipv4Addr, MdnsPrinter> = HashMap::new();
    for entry in entries {
        by_ip
            .entry(entry.ip)
            .and_modify(|existing| {
                if service_priority(&entry.service_type)
                    > service_priority(&existing.service_type)
                {
                    existing.service_type = entry.service_type.clone();
                    existing.port = entry.port;
                }
                if existing.model.is_none() {
                    existing.model = entry.model.clone();
                }
                if existing.device_id.is_none() {
                    existing.device_id = entry.device_id.clone();
                }
            })
            .or_insert(entry);
    }
    let mut out: Vec<_> = by_ip.into_values().collect();
    out.sort_by_key(|p| p.ip);
    out
}

/// Build an IEEE-1284 style device-id string from mDNS TXT record
/// properties. AirPrint printers commonly publish `usb_MFG` and
/// `usb_MDL` keys that mirror the 1284 fields exactly. Returns `None`
/// when neither key is present.
pub fn device_id_from_txt(
    usb_mfg: Option<&str>,
    usb_mdl: Option<&str>,
    usb_cmd: Option<&str>,
) -> Option<String> {
    match (usb_mfg, usb_mdl) {
        (Some(mfg), Some(mdl)) => {
            let mut s = format!("MFG:{mfg};MDL:{mdl};");
            if let Some(cmd) = usb_cmd {
                s.push_str(&format!("CMD:{cmd};"));
            }
            Some(s)
        }
        _ => None,
    }
}

/// Pick the best available "model" string from the TXT record. AirPrint
/// printers prefer `ty` (the human-readable model name) over `product`
/// (which is usually the parenthesized USB-style name).
pub fn model_from_txt(ty: Option<&str>, product: Option<&str>) -> Option<String> {
    if let Some(s) = ty
        && !s.trim().is_empty()
    {
        return Some(s.trim().to_string());
    }
    if let Some(s) = product {
        let trimmed = s.trim().trim_start_matches('(').trim_end_matches(')');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Time-boxed mDNS browse across the standard printer service types.
/// Returns a deduplicated list of printers or an empty vec on any
/// initialization failure — we never want a broken multicast stack to
/// block a regular scan.
#[cfg(not(test))]
pub async fn discover(timeout: Duration, verbose: bool) -> Vec<Printer> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let mdns = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            if verbose {
                eprintln!("[mdns] failed to start daemon: {e}, skipping mDNS pass");
            }
            return Vec::new();
        }
    };

    // Move the blocking browse into tokio's blocking pool so we don't
    // starve the async runtime while waiting for multicast traffic.
    let verbose_inner = verbose;
    let raw = tokio::task::spawn_blocking(move || {
        let mut receivers = Vec::new();
        for stype in PRINTER_SERVICE_TYPES {
            match mdns.browse(stype) {
                Ok(recv) => receivers.push((stype.to_string(), recv)),
                Err(e) => {
                    if verbose_inner {
                        eprintln!("[mdns] browse({stype}) failed: {e}");
                    }
                }
            }
        }

        if receivers.is_empty() {
            let _ = mdns.shutdown();
            return Vec::new();
        }

        let mut out: Vec<MdnsPrinter> = Vec::new();
        let deadline = Instant::now() + timeout;

        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let per_poll = remaining.min(Duration::from_millis(100));
            let mut any_event = false;
            for (_stype, recv) in &receivers {
                match recv.recv_timeout(per_poll) {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        any_event = true;
                        let addrs = info.get_addresses_v4();
                        if addrs.is_empty() {
                            continue;
                        }
                        let service_type = info.ty_domain.clone();
                        let model = model_from_txt(
                            info.get_property_val_str("ty"),
                            info.get_property_val_str("product"),
                        );
                        let device_id = device_id_from_txt(
                            info.get_property_val_str("usb_MFG"),
                            info.get_property_val_str("usb_MDL"),
                            info.get_property_val_str("usb_CMD"),
                        );
                        let hostname = info
                            .get_hostname()
                            .trim_end_matches('.')
                            .to_string();
                        let port = info.get_port();
                        for ip in addrs {
                            if verbose_inner {
                                eprintln!(
                                    "[mdns] {} via {} -> {}",
                                    ip,
                                    service_type,
                                    model.as_deref().unwrap_or("?")
                                );
                            }
                            out.push(MdnsPrinter {
                                ip,
                                hostname: hostname.clone(),
                                service_type: service_type.clone(),
                                port,
                                model: model.clone(),
                                device_id: device_id.clone(),
                            });
                        }
                    }
                    Ok(_) => { any_event = true; }
                    Err(_) => {}
                }
            }
            if !any_event {
                // All channels quiet — give the daemon a tiny breather
                // so we don't spin at 100% CPU waiting for the deadline.
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        for stype in PRINTER_SERVICE_TYPES {
            let _ = mdns.stop_browse(stype);
        }
        let _ = mdns.shutdown();

        out
    })
    .await
    .unwrap_or_default();

    let merged = merge_by_ip(raw);
    if verbose && !merged.is_empty() {
        eprintln!("[mdns] resolved {} unique printer(s)", merged.len());
    }
    merged.into_iter().map(MdnsPrinter::into_printer).collect()
}

/// Test stub — the real `discover` spins up a live mDNS daemon which
/// we don't want to do inside `cargo test`. Integration against a real
/// printer happens on a Windows VM via the dev loop.
#[cfg(test)]
pub async fn discover(_timeout: Duration, _verbose: bool) -> Vec<Printer> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        ip: &str,
        stype: &str,
        model: Option<&str>,
    ) -> MdnsPrinter {
        MdnsPrinter {
            ip: ip.parse().unwrap(),
            hostname: "printer.local".to_string(),
            service_type: stype.to_string(),
            port: 631,
            model: model.map(str::to_string),
            device_id: None,
        }
    }

    #[test]
    fn canonical_port_maps_known_types() {
        assert_eq!(canonical_port_for("_ipp._tcp.local."), Some(631));
        assert_eq!(canonical_port_for("_ipps._tcp.local."), Some(631));
        assert_eq!(canonical_port_for("_pdl-datastream._tcp.local."), Some(9100));
        assert_eq!(canonical_port_for("_printer._tcp.local."), Some(515));
        assert_eq!(canonical_port_for("_unknown._tcp.local."), None);
    }

    #[test]
    fn service_priority_ipp_wins() {
        assert!(service_priority("_ipp._tcp.local.") > service_priority("_ipps._tcp.local."));
        assert!(service_priority("_ipps._tcp.local.")
            > service_priority("_pdl-datastream._tcp.local."));
        assert!(service_priority("_pdl-datastream._tcp.local.")
            > service_priority("_printer._tcp.local."));
        assert_eq!(service_priority("_unknown._tcp.local."), 0);
    }

    #[test]
    fn merge_by_ip_collapses_duplicates_and_keeps_best_service() {
        let entries = vec![
            entry("192.168.1.50", "_pdl-datastream._tcp.local.", None),
            entry("192.168.1.50", "_ipp._tcp.local.", Some("Brother MFC-L2750DW")),
            entry("192.168.1.51", "_ipps._tcp.local.", Some("HP LaserJet")),
        ];
        let merged = merge_by_ip(entries);
        assert_eq!(merged.len(), 2);
        let first = merged.iter().find(|e| e.ip.to_string() == "192.168.1.50").unwrap();
        assert_eq!(first.service_type, "_ipp._tcp.local.");
        assert_eq!(first.model.as_deref(), Some("Brother MFC-L2750DW"));
    }

    #[test]
    fn merge_by_ip_fills_missing_model_from_second_record() {
        let entries = vec![
            entry("192.168.1.60", "_ipp._tcp.local.", None),
            entry("192.168.1.60", "_pdl-datastream._tcp.local.", Some("Canon i-SENSYS")),
        ];
        let merged = merge_by_ip(entries);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].service_type, "_ipp._tcp.local.");
        assert_eq!(merged[0].model.as_deref(), Some("Canon i-SENSYS"));
    }

    #[test]
    fn device_id_from_txt_builds_1284_string() {
        assert_eq!(
            device_id_from_txt(Some("Brother"), Some("MFC-L2750DW"), None),
            Some("MFG:Brother;MDL:MFC-L2750DW;".to_string())
        );
        assert_eq!(
            device_id_from_txt(Some("HP"), Some("LaserJet"), Some("PCL6,POSTSCRIPT")),
            Some("MFG:HP;MDL:LaserJet;CMD:PCL6,POSTSCRIPT;".to_string())
        );
        assert_eq!(device_id_from_txt(None, Some("MFC-L2750DW"), None), None);
        assert_eq!(device_id_from_txt(Some("Brother"), None, None), None);
    }

    #[test]
    fn model_from_txt_prefers_ty_key() {
        assert_eq!(
            model_from_txt(Some("Brother MFC-L2750DW"), Some("(MFC-L2750DW)")),
            Some("Brother MFC-L2750DW".to_string())
        );
        assert_eq!(
            model_from_txt(None, Some("(HP LaserJet)")),
            Some("HP LaserJet".to_string())
        );
        assert_eq!(model_from_txt(Some("   "), Some("(Canon)")), Some("Canon".to_string()));
        assert_eq!(model_from_txt(None, None), None);
    }

    #[test]
    fn into_printer_uses_canonical_port() {
        let entry = MdnsPrinter {
            ip: "192.168.1.50".parse().unwrap(),
            hostname: "brother.local".to_string(),
            service_type: "_ipp._tcp.local.".to_string(),
            port: 80,
            model: Some("Brother".to_string()),
            device_id: None,
        };
        let printer = entry.into_printer();
        assert_eq!(printer.ports, vec![631]);
        assert_eq!(printer.discovery_methods, vec![DiscoveryMethod::Mdns]);
        assert_eq!(printer.local_name.as_deref(), Some("brother.local"));
    }

    #[test]
    fn merge_preserves_single_ipv4_entry() {
        let entries = vec![entry(
            "10.0.0.1",
            "_pdl-datastream._tcp.local.",
            Some("Zebra"),
        )];
        let merged = merge_by_ip(entries);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].ip.to_string(), "10.0.0.1");
        assert_eq!(merged[0].service_type, "_pdl-datastream._tcp.local.");
    }
}

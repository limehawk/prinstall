use std::net::Ipv4Addr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMethod {
    PortScan,
    Ipp,
    Snmp,
    Local,
    Mdns,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PrinterSource {
    Network,
    Usb,
    Installed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Printer {
    pub ip: Option<Ipv4Addr>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub status: PrinterStatus,
    pub discovery_methods: Vec<DiscoveryMethod>,
    pub ports: Vec<u16>,
    pub source: PrinterSource,
    pub local_name: Option<String>,
    /// Windows port name (e.g. `IP_192.168.1.50`, `USB001`, `PORTPROMPT:`).
    /// Populated by the `list` command from Get-Printer / Win32_Printer;
    /// None for printers discovered via network scans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_name: Option<String>,
    /// Driver name as reported by Windows. Distinct from `model`, which
    /// is the hardware model (SNMP sysDescr). For `list` results we
    /// populate this alongside the queue name; scan results leave it
    /// None since `model` already carries the relevant identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_name: Option<String>,
    /// Whether the queue is shared on the network. `list` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    /// Whether this is the Windows default printer. `list` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
}

impl Printer {
    pub fn display_ip(&self) -> String {
        if let Some(ip) = self.ip {
            ip.to_string()
        } else {
            match self.source {
                PrinterSource::Usb => "USB".to_string(),
                _ => {
                    if let Some(ref name) = self.local_name {
                        name.clone()
                    } else {
                        "Unknown".to_string()
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterStatus {
    Ready,
    Error,
    Offline,
    Unknown,
}

impl std::fmt::Display for PrinterStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => write!(f, "Ready"),
            Self::Error => write!(f, "Error"),
            Self::Offline => write!(f, "Offline"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverMatch {
    pub name: String,
    pub category: DriverCategory,
    pub confidence: MatchConfidence,
    pub source: DriverSource,
    /// Match score 0-1000. Higher is better. Used for ranking within a confidence tier.
    /// Exact matches get 1000. Universal drivers and unscored items are 0.
    #[serde(default)]
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DriverCategory {
    Matched,
    Universal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    Exact,
    Fuzzy,
    Universal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DriverSource {
    LocalStore,
    Manufacturer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverResults {
    pub printer_model: String,
    pub matched: Vec<DriverMatch>,
    pub universal: Vec<DriverMatch>,
    /// IEEE 1284 device ID advertised by the printer via IPP, if available.
    /// This is the string Windows Update matches drivers against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Result of the Windows Update install-rollback probe, if one was run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_update: Option<WindowsUpdateProbe>,
    /// Result of the Microsoft Update Catalog search, if one was run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CatalogSearchResult>,
}

/// Outcome of a Microsoft Update Catalog search for a printer model.
///
/// The catalog is the authoritative Windows-side source of driver packages
/// for network printers that don't advertise themselves to PnP. Searching
/// here gives us a list of candidate `.cab` driver packages we can download
/// via the catalog's download-dialog endpoint without ever touching Windows
/// Update Agent APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSearchResult {
    /// The search query we sent to the catalog (typically the printer model).
    pub query: String,
    /// Matching updates from the catalog, in the order returned.
    pub updates: Vec<CatalogEntry>,
    /// Present when the search could not complete. Graceful degradation — the
    /// rest of the driver report stays useful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CatalogSearchResult {
    /// Build a failure result carrying an error message.
    pub fn failure(query: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            updates: Vec::new(),
            error: Some(error.into()),
        }
    }
}

/// A single catalog update row, trimmed down to the fields we render in
/// the CLI output. Mirrors [`crate::drivers::catalog::CatalogUpdate`] but
/// lives in `models` so it can serialize through `--json` without pulling
/// in the catalog module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub title: String,
    pub products: String,
    pub classification: String,
    pub last_updated: String,
    pub version: String,
    pub size: String,
    pub size_bytes: u64,
    pub guid: String,
}

impl From<crate::drivers::catalog::CatalogUpdate> for CatalogEntry {
    fn from(u: crate::drivers::catalog::CatalogUpdate) -> Self {
        Self {
            title: u.title,
            products: u.products,
            classification: u.classification,
            last_updated: u.last_updated,
            version: u.version,
            size: u.size,
            size_bytes: u.size_bytes,
            guid: u.guid,
        }
    }
}

/// Outcome of an install-rollback probe against Windows Update.
///
/// We perform the probe by running `Add-Printer -ConnectionName` (which
/// triggers Windows Update's driver lookup), capturing the driver name
/// Windows chose, then immediately removing the probe queue. The driver
/// package stays in the driver store as a beneficial side effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsUpdateProbe {
    /// The driver name Windows Update selected (e.g. "Brother MFC-L2750DW series Class Driver").
    pub driver_name: String,
    /// The port name Windows assigned to the probe queue.
    pub port_name: String,
    /// The printer name Windows generated for the probe (usually the IPP-advertised name).
    pub resolved_printer_name: String,
    /// True if the selected driver is one of the in-box fallback drivers
    /// (e.g. "Microsoft IPP Class Driver"), meaning Windows Update had
    /// nothing vendor-specific to offer for this printer.
    pub from_in_box_fallback: bool,
    /// Present when the probe could not complete. The matched/universal
    /// sections remain valid even when probe_error is Some.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
}

impl WindowsUpdateProbe {
    /// Build a probe result representing a failed probe — carries the error
    /// message but no driver info. Used for graceful degradation.
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            driver_name: String::new(),
            port_name: String::new(),
            resolved_printer_name: String::new(),
            from_in_box_fallback: false,
            probe_error: Some(error.into()),
        }
    }

    /// True if this result represents a successful probe with a driver name.
    pub fn is_success(&self) -> bool {
        self.probe_error.is_none() && !self.driver_name.is_empty()
    }
}

/// Generic result type for any printer-manager operation (install, remove,
/// configure, etc). The typed per-operation payload lives in `detail` as a
/// serialized JSON value — decode it with `detail_as::<T>()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterOpResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
}

impl PrinterOpResult {
    /// Build a successful result with a typed detail payload.
    pub fn ok(detail: impl Serialize) -> Self {
        Self {
            success: true,
            error: None,
            detail: serde_json::to_value(detail).unwrap_or(serde_json::Value::Null),
        }
    }

    /// Build a successful result with no payload.
    pub fn ok_empty() -> Self {
        Self {
            success: true,
            error: None,
            detail: serde_json::Value::Null,
        }
    }

    /// Build a failure result with a human-readable error message.
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(msg.into()),
            detail: serde_json::Value::Null,
        }
    }

    /// Attempt to deserialize the detail into a specific type.
    pub fn detail_as<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.detail.clone()).ok()
    }
}

/// Payload for the `add`/install operation — the details of what was installed.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallDetail {
    pub printer_name: String,
    pub driver_name: String,
    pub port_name: String,
    /// Optional non-fatal warning (e.g., "installed via IPP Class Driver fallback").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Payload for the `remove` operation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoveDetail {
    pub printer_name: String,
    pub port_removed: bool,
    pub driver_removed: bool,
    /// True when the removal was a no-op because no matching printer existed.
    /// Callers can use this to distinguish "removed successfully" from
    /// "already gone" — both are reported as `success: true`.
    #[serde(default)]
    pub already_absent: bool,
}

/// Entry in the local install history (C:\ProgramData\prinstall\history.toml
/// on Windows — machine-wide so SYSTEM-run RMM installs and interactive
/// admin sessions share one audit log. See src/paths.rs for the rationale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub model: String,
    pub driver_name: String,
    pub source: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct History {
    #[serde(default)]
    pub installs: Vec<HistoryEntry>,
}

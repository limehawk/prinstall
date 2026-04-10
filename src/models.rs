use std::net::Ipv4Addr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMethod {
    PortScan,
    Ipp,
    Snmp,
    Local,
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

/// Entry in the local install history (C:\ProgramData\prinstall\history.toml)
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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Printer {
    pub ip: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub status: PrinterStatus,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub success: bool,
    pub printer_name: String,
    pub driver_name: String,
    pub port_name: String,
    pub error: Option<String>,
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

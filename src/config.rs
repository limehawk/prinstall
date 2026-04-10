//! Persistent application configuration.
//!
//! Stored as TOML at `paths::config_path()` (`%APPDATA%\prinstall\config.toml`
//! on Windows). Absent-file and malformed-file both degrade gracefully to
//! defaults — prinstall never fails to start because of a bad config.

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Default SNMP community string.
    pub community: String,
    /// Default subnet to scan; overrides auto-detect if set.
    pub default_subnet: Option<String>,
    /// Per-host timeout for TCP port probes, in milliseconds.
    pub scan_timeout_ms: u64,
    /// Discovery method: "all", "snmp", or "port".
    pub scan_method: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            community: "public".to_string(),
            default_subnet: None,
            scan_timeout_ms: 100,
            scan_method: "all".to_string(),
        }
    }
}

impl AppConfig {
    /// Load config from disk, returning default on any failure.
    pub fn load() -> Self {
        let path = paths::config_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist config to disk.
    pub fn save(&self) -> Result<(), String> {
        paths::ensure_data_dir().map_err(|e| format!("Failed to create data dir: {e}"))?;
        let contents = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        std::fs::write(paths::config_path(), contents)
            .map_err(|e| format!("Failed to write config: {e}"))
    }
}

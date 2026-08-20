//! Persistent application configuration.
//!
//! Stored as TOML at `paths::config_path()`. Absent or malformed files
//! degrade to defaults.

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// SDI (Snappy Driver Installer Origin) cache / mirror settings.
    #[serde(default)]
    pub sdi: SdiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sdi: SdiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SdiConfig {
    /// Mirror URL for SDI pack + index downloads.
    pub mirror_url: String,
    /// Warn if the last `sdi refresh` is older than this many days.
    pub index_refresh_days: u32,
    /// Maximum total cache size for SDI driver packs, in megabytes.
    pub max_cache_mb: u64,
    /// When true, the SDI tier never touches the network.
    pub offline_mode: bool,
}

impl Default for SdiConfig {
    fn default() -> Self {
        Self {
            mirror_url: "https://github.com/limehawk/prinstall/releases/download/sdi-printer-v1/"
                .to_string(),
            index_refresh_days: 30,
            max_cache_mb: 2048,
            offline_mode: false,
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
}

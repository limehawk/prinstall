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
    /// SDI (Snappy Driver Installer Origin) driver tier configuration.
    #[serde(default)]
    pub sdi: SdiConfig,
    /// Microsoft Update Catalog driver tier configuration.
    #[serde(default)]
    pub catalog: CatalogConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            community: "public".to_string(),
            default_subnet: None,
            scan_timeout_ms: 100,
            scan_method: "all".to_string(),
            sdi: SdiConfig::default(),
            catalog: CatalogConfig::default(),
        }
    }
}

/// Configuration for the SDI driver acquisition tier.
///
/// All fields use `#[serde(default)]` so an existing `config.toml` without
/// an `[sdi]` section silently gains defaults on load. Users who never
/// touch SDI-related flags see zero behavior change from the presence of
/// this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SdiConfig {
    /// Whether the SDI tier is enabled. When false, prinstall skips SDI
    /// during both `prinstall drivers` enumeration and `prinstall add`
    /// auto-pick. Same effect as `--no-sdi` per-run.
    pub enabled: bool,
    /// Mirror URL for SDI pack + index downloads. Default points at the
    /// prinstall GitHub Releases `sdi-printer-v1` tag. Users in air-gapped
    /// environments can redirect this to their own HTTP share that hosts
    /// a manifest.json + DP_Printer_*.bin + DP_Printer_*.7z assets.
    pub mirror_url: String,
    /// Stale index warning threshold. If the last `sdi refresh` was more
    /// than this many days ago, prinstall emits a one-line warning
    /// suggesting a refresh. Never hard-fails.
    pub index_refresh_days: u32,
    /// Maximum total cache size for SDI driver packs, in megabytes.
    /// `prinstall sdi clean` evicts least-recently-used packs past this
    /// budget. Default is 2 GB — enough for both printer packs plus
    /// some headroom.
    pub max_cache_mb: u64,
    /// When true, the SDI tier never touches the network. Only uses
    /// whatever is already cached locally. For air-gapped fleets or
    /// reference-image pre-staging.
    pub offline_mode: bool,
    /// When true, `prinstall add` auto-pick is allowed to trigger a
    /// first-run SDI pack download even for uncached packs. Symmetric
    /// with the `--sdi-fetch` per-run flag. Defaults to false so scripted
    /// RMM runs never silently pull a multi-hundred-MB pack.
    pub auto_fetch: bool,
}

impl Default for SdiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mirror_url: "https://github.com/limehawk/prinstall/releases/download/sdi-printer-v1/"
                .to_string(),
            index_refresh_days: 30,
            max_cache_mb: 2048,
            offline_mode: false,
            auto_fetch: false,
        }
    }
}

/// Configuration for the Microsoft Update Catalog driver acquisition
/// tier. Added alongside `SdiConfig` for symmetry — lets users disable
/// Tier 3 Catalog scraping explicitly without having to disable the
/// whole pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogConfig {
    /// Whether the Microsoft Update Catalog tier is enabled. When false,
    /// prinstall skips the catalog.update.microsoft.com HTTP scraper
    /// during both enumeration and auto-pick. Same effect as
    /// `--no-catalog` per-run.
    pub enabled: bool,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self { enabled: true }
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

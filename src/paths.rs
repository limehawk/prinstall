//! Canonical paths for all prinstall data files.
//!
//! Everything — install history, config, driver staging, future logs — lives
//! under a single root directory. On Windows that's `%APPDATA%\prinstall\`
//! (per-user, roaming). On Linux (dev builds) it's `$XDG_DATA_HOME/prinstall`
//! or `~/.local/share/prinstall`.

use std::path::PathBuf;

/// Returns the single root directory where prinstall stores all its files.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("prinstall");
        }
        // Should never happen on a real Windows session — fall back to ProgramData.
        PathBuf::from(r"C:\ProgramData").join("prinstall")
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("prinstall");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local/share/prinstall");
        }
        PathBuf::from("prinstall-data")
    }
}

/// Path to the install history TOML file.
pub fn history_path() -> PathBuf {
    data_dir().join("history.toml")
}

/// Path to the persistent config TOML file.
pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

/// Directory where downloaded driver packages are extracted and staged.
pub fn staging_dir() -> PathBuf {
    data_dir().join("staging")
}

/// Ensures the data directory exists. Idempotent.
pub fn ensure_data_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())
}

/// Legacy history location from versions prior to 0.2.2 (pre-APPDATA consolidation).
/// Used for one-time copy-forward migration on first run under the new layout.
#[cfg(target_os = "windows")]
pub fn legacy_history_path() -> PathBuf {
    PathBuf::from(r"C:\ProgramData\prinstall\history.toml")
}

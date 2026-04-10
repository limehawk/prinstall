//! Install history log.
//!
//! Records every successful install/update so techs can audit what's been
//! done on a machine. Stored as TOML at `paths::history_path()`
//! (`%APPDATA%\prinstall\history.toml` on Windows).

use crate::models::{History, HistoryEntry};
use crate::paths;

/// Load install history from disk.
///
/// On first run under the 0.2.2+ layout, migrates from the legacy
/// `C:\ProgramData\prinstall\history.toml` location if present.
pub fn load() -> History {
    let path = paths::history_path();
    if !path.exists() {
        migrate_legacy_if_present();
    }
    if !path.exists() {
        return History::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
        Err(_) => History::default(),
    }
}

/// Save install history to disk.
pub fn save(history: &History) {
    let _ = paths::ensure_data_dir();
    if let Ok(contents) = toml::to_string_pretty(history) {
        let _ = std::fs::write(paths::history_path(), contents);
    }
}

/// Record a successful install.
pub fn record_install(model: &str, driver_name: &str, source: &str) {
    let mut history = load();
    history.installs.push(HistoryEntry {
        model: model.to_string(),
        driver_name: driver_name.to_string(),
        source: source.to_string(),
        date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
    });
    save(&history);
}

/// One-time copy-forward of pre-0.2.2 history from
/// `C:\ProgramData\prinstall\history.toml` into the new APPDATA layout.
/// No-op if the new location already exists or the legacy file isn't present.
#[cfg(target_os = "windows")]
fn migrate_legacy_if_present() {
    let legacy = paths::legacy_history_path();
    let new = paths::history_path();
    if new.exists() || !legacy.exists() {
        return;
    }
    let _ = paths::ensure_data_dir();
    let _ = std::fs::copy(&legacy, &new);
}

#[cfg(not(target_os = "windows"))]
fn migrate_legacy_if_present() {}

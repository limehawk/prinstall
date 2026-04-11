//! Install history log.
//!
//! Records every successful install/update so techs can audit what's been
//! done on a machine. Stored as TOML at `paths::history_path()`
//! (`C:\ProgramData\prinstall\history.toml` on Windows) — see `paths.rs`
//! for the rationale behind the machine-wide ProgramData location.

use crate::models::{History, HistoryEntry};
use crate::paths;

/// Load install history from disk.
///
/// On first run under the 0.3.1+ layout, migrates forward from the
/// 0.2.2–0.3.0 `%APPDATA%\prinstall\history.toml` location if that file
/// exists and the ProgramData target doesn't.
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

/// One-time copy-forward of a 0.2.2–0.3.0 history file from
/// `%APPDATA%\prinstall\history.toml` into the machine-wide ProgramData
/// layout. No-op if the new location already exists, the legacy file
/// isn't present, or `%APPDATA%` isn't available for the current session
/// (e.g. SYSTEM contexts without a user profile).
///
/// Note: prior to 0.2.2 the history was already at
/// `C:\ProgramData\prinstall\history.toml` — that's the same location
/// we're returning to in 0.3.1+, so the pre-0.2.2 path is automatically
/// picked up by the regular load() without any explicit migration.
#[cfg(target_os = "windows")]
fn migrate_legacy_if_present() {
    let Some(legacy) = paths::legacy_appdata_history_path() else {
        return;
    };
    let new = paths::history_path();
    if new.exists() || !legacy.exists() {
        return;
    }
    let _ = paths::ensure_data_dir();
    let _ = std::fs::copy(&legacy, &new);
}

#[cfg(not(target_os = "windows"))]
fn migrate_legacy_if_present() {}

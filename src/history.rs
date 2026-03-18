use std::path::PathBuf;
use crate::models::{History, HistoryEntry};

const HISTORY_DIR: &str = r"C:\ProgramData\prinstall";
const HISTORY_FILE: &str = "history.toml";

fn history_path() -> PathBuf {
    PathBuf::from(HISTORY_DIR).join(HISTORY_FILE)
}

/// Load install history from disk.
pub fn load() -> History {
    let path = history_path();
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
    let path = history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(contents) = toml::to_string_pretty(history) {
        std::fs::write(path, contents).ok();
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

/// Look up a model in install history.
pub fn find_by_model(model: &str) -> Option<HistoryEntry> {
    let history = load();
    let model_lower = model.to_lowercase();
    history
        .installs
        .into_iter()
        .rev() // most recent first
        .find(|e| e.model.to_lowercase() == model_lower)
}

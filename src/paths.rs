//! Canonical paths for all prinstall data files.
//!
//! Everything — install history, config, driver staging, future logs — lives
//! under a single root directory. On Windows that's `%PROGRAMDATA%\prinstall\`
//! (machine-wide, shared across all user accounts and services including
//! SYSTEM). On Linux (dev builds) it's `$XDG_DATA_HOME/prinstall` or
//! `~/.local/share/prinstall`.
//!
//! ## Why ProgramData, not APPDATA
//!
//! prinstall is an MSP admin tool. It's commonly invoked by:
//! - Interactive admin sessions (techs on a remote session / local console)
//! - SYSTEM-level runbooks deployed via RMM (SuperOps, NinjaRMM, etc.)
//!
//! These two invocation paths have different `%APPDATA%` values — SYSTEM's
//! APPDATA is `C:\Windows\System32\config\systemprofile\AppData\Roaming\`,
//! which isn't accessible to interactive users, and interactive admins'
//! APPDATA isn't visible to SYSTEM. Putting the data dir at `%APPDATA%`
//! would split the install history across per-user silos and break the
//! MSP audit trail.
//!
//! `%PROGRAMDATA%` (typically `C:\ProgramData\`) is machine-wide and
//! writable by admin-privileged processes regardless of user context, so
//! every prinstall invocation — interactive or SYSTEM — reads and writes
//! the same history log. That's what an MSP tool needs.
//!
//! ## History of this decision
//!
//! - pre-0.2.2: data dir at `C:\ProgramData\prinstall\` (original)
//! - 0.2.2 through 0.3.0: data dir at `%APPDATA%\prinstall\` (mistake —
//!   made per-user, broke the shared audit trail)
//! - 0.3.1+: data dir back at `C:\ProgramData\prinstall\` (corrected)
//!
//! The 0.2.2→0.3.0 APPDATA history is migrated forward on first run under
//! 0.3.1+ via [`legacy_appdata_history_path`].

use std::path::PathBuf;

/// Returns the single root directory where prinstall stores all its files.
///
/// Windows: `C:\ProgramData\prinstall\` via the `%PROGRAMDATA%` env var.
/// Linux: `$XDG_DATA_HOME/prinstall` or `~/.local/share/prinstall`.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(programdata) = std::env::var("ProgramData") {
            return PathBuf::from(programdata).join("prinstall");
        }
        // Hard fallback — should never fire on a real Windows session.
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

/// Root directory for the SDI (Snappy Driver Installer Origin) cache.
///
/// Contains the printer-only slice of SDI: per-pack `.bin` indexes and
/// lazily-fetched `.7z` driver packs downloaded from the prinstall
/// GitHub Releases mirror.
pub fn sdi_dir() -> PathBuf {
    data_dir().join("sdi")
}

/// Directory containing cached SDW-format `.bin` index files.
///
/// Populated by `prinstall sdi refresh` (or transparently on the first
/// `prinstall add` that hits the SDI tier). Typical contents:
/// `DP_Printer_<version>.bin`, `DP_ThermoPrinter_<version>.bin`.
pub fn sdi_indexes_dir() -> PathBuf {
    sdi_dir().join("indexes")
}

/// Directory containing lazily-fetched SDI driver pack `.7z` files.
///
/// Packs are downloaded on first HWID match from the configured mirror
/// (default: prinstall's GitHub Releases `sdi-printer-v<N>` tag). Each
/// pack can be hundreds of megabytes; `prinstall sdi clean` evicts
/// least-recently-used packs past `config.sdi.max_cache_mb`.
pub fn sdi_drivers_dir() -> PathBuf {
    sdi_dir().join("drivers")
}

/// Path to the SDI cache metadata JSON file.
///
/// Tracks index version (from the mirror manifest), last refresh
/// timestamp, and per-pack usage stats (`size_bytes`, `sha256`,
/// `last_used`) used by the LRU prune logic.
pub fn sdi_metadata_path() -> PathBuf {
    sdi_dir().join("metadata.json")
}

/// Ensures the data directory exists. Idempotent.
pub fn ensure_data_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())
}

/// Ensures the SDI cache directories exist. Idempotent. Creates
/// `sdi/`, `sdi/indexes/`, and `sdi/drivers/`.
pub fn ensure_sdi_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(sdi_indexes_dir())?;
    std::fs::create_dir_all(sdi_drivers_dir())?;
    Ok(())
}

/// Path to the 0.2.2–0.3.0 APPDATA history file, used for one-time
/// copy-forward migration on first run under 0.3.1+.
///
/// Returns `None` when the `%APPDATA%` environment variable is missing
/// (shouldn't happen on a real interactive Windows session, but for
/// SYSTEM-run scripts it can be an unusual path or absent entirely).
#[cfg(target_os = "windows")]
pub fn legacy_appdata_history_path() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|p| PathBuf::from(p).join("prinstall").join("history.toml"))
}

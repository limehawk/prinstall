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

/// Resolve the active local driver bundle directory.
///
/// Priority order:
///   1. `PRINSTALL_BUNDLE_DIR` environment variable (verbatim).
///   2. `drivers/` adjacent to the prinstall executable
///      (`std::env::current_exe()` → parent).
///   3. `<data_dir>/drivers/` — on Windows that's
///      `C:\ProgramData\prinstall\drivers\`.
///
/// Returns the first location that exists and contains at least one file.
/// If none are populated, returns the ProgramData path — callers handle
/// "empty directory" gracefully.
pub fn bundle_dir() -> PathBuf {
    for candidate in bundle_dir_candidates() {
        if dir_has_files(&candidate) {
            return candidate;
        }
    }
    data_dir().join("drivers")
}

/// Return every candidate bundle directory in priority order. Callers that
/// want to inspect all three locations (for diagnostics, scanning, etc.)
/// use this instead of [`bundle_dir`].
pub fn bundle_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(3);

    // 1. Env var override — use verbatim.
    if let Ok(env_dir) = std::env::var("PRINSTALL_BUNDLE_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            out.push(PathBuf::from(trimmed));
        }
    }

    // 2. Exe-adjacent drivers/ directory. `current_exe` can fail in unusual
    //    environments (very old kernels, /proc unavailable); fall through
    //    silently when that happens.
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        out.push(parent.join("drivers"));
    }

    // 3. ProgramData fallback.
    out.push(data_dir().join("drivers"));

    out
}

/// Return true when `dir` exists and contains at least one entry. Used by
/// [`bundle_dir`] to skip past empty candidate directories.
fn dir_has_files(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .is_some_and(|mut it| it.next().is_some())
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

/// Shared mutex guarding tests that mutate `PRINSTALL_BUNDLE_DIR`. Lives
/// here (rather than inside each test module) because both `paths::tests`
/// and `drivers::bundle::tests` touch the same env var and need to
/// serialize against each other — not just among themselves.
#[cfg(test)]
pub(crate) static BUNDLE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // Alias to the shared lock so the existing test bodies stay readable.
    use super::BUNDLE_ENV_LOCK as ENV_LOCK;

    #[test]
    fn bundle_dir_candidates_respects_env_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join("prinstall-bundle-env-test");
        let _ = std::fs::create_dir_all(&tmp);
        // SAFETY: tests that touch env vars are gated by ENV_LOCK above.
        unsafe {
            std::env::set_var("PRINSTALL_BUNDLE_DIR", &tmp);
        }

        let candidates = bundle_dir_candidates();
        assert_eq!(candidates[0], tmp, "env var override should lead the list");

        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }
    }

    #[test]
    fn bundle_dir_candidates_includes_exe_adjacent_and_fallback() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: locked.
        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }

        let candidates = bundle_dir_candidates();
        // Expect at least the exe-adjacent + programdata candidates; when
        // current_exe() fails (rare), only the programdata fallback is there.
        assert!(
            !candidates.is_empty(),
            "bundle_dir_candidates should always return at least one entry"
        );

        // The final entry should always be the data_dir-based fallback.
        let last = candidates.last().unwrap();
        assert_eq!(last, &data_dir().join("drivers"));
    }

    #[test]
    fn bundle_dir_falls_back_to_programdata_when_nothing_populated() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Point the env override at a directory that definitely doesn't exist.
        let bogus = std::env::temp_dir().join("prinstall-bundle-definitely-missing-xyz");
        let _ = std::fs::remove_dir_all(&bogus);
        // SAFETY: locked.
        unsafe {
            std::env::set_var("PRINSTALL_BUNDLE_DIR", &bogus);
        }

        // With no populated bundle dir anywhere, the function returns the
        // ProgramData/XDG fallback — equal to data_dir().join("drivers").
        let resolved = bundle_dir();
        assert_eq!(resolved, data_dir().join("drivers"));

        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }
    }
}

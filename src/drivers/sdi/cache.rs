//! On-disk cache manager for the SDI tier.
//!
//! Tracks the state of `C:\ProgramData\prinstall\sdi\`: which SDW `.bin`
//! index files are present, which SDIO `.7z` driver packs are cached,
//! their byte sizes and SHA256 checksums, and per-pack `last_used`
//! timestamps for LRU eviction. Persists to [`crate::paths::sdi_metadata_path`]
//! as JSON. Loaded on every SDI tier invocation.
//!
//! ## Persistence format
//!
//! Metadata is stored as a single pretty-printed JSON document. The file
//! is written atomically: a temp sibling (`metadata.json.tmp`) is written
//! in full, then `rename`d over the target so a crash mid-write never
//! leaves a half-parsed file on disk.
//!
//! ## Graceful degradation
//!
//! [`SdiCache::load`] never fails on recoverable situations — a missing
//! metadata file, a corrupt JSON body, or a not-yet-created cache
//! directory all degrade to a fresh [`CacheMetadata::default`]. The only
//! errors bubbled up are hard filesystem failures (e.g. the parent data
//! directory can't be created at all). This matches [`crate::history`]
//! and [`crate::config::AppConfig`] patterns — prinstall never fails to
//! start because of a bad cache state file.
//!
//! ## LRU prune
//!
//! [`SdiCache::prune`] walks the `packs` map sorted by `last_used`
//! ascending and evicts entries from the head until the total cached
//! size drops at or below the configured budget. Eviction removes both
//! the on-disk `.7z` file and the metadata entry. Packs that are
//! registered in metadata but whose files are missing on disk are still
//! considered for eviction so the map never drifts out of sync.
//!
//! ## Path-traversal safety
//!
//! Pack names are treated as opaque filenames — all public methods that
//! accept a `pack_name` parameter validate it through [`validate_pack_name`]
//! before touching the filesystem. A name containing `/`, `\`, `..`, a
//! drive letter, or any other path separator is rejected outright. This
//! keeps [`SdiCache::pack_path`] from being weaponised into a
//! cache-escape primitive.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths;

/// Filename used for the persistent metadata document, relative to the
/// SDI cache root. Kept as a constant so both the production path
/// (via [`paths::sdi_metadata_path`]) and the test harness derive the
/// same name from a given root.
const METADATA_FILENAME: &str = "metadata.json";

/// On-disk state tracker for the SDI cache directory.
///
/// Owns a loaded snapshot of [`CacheMetadata`] plus the absolute root
/// path the cache lives under. Every mutating method re-persists
/// metadata to disk before returning so an unclean shutdown cannot lose
/// LRU info mid-session.
pub struct SdiCache {
    /// Absolute path to the SDI root directory (e.g.
    /// `C:\ProgramData\prinstall\sdi\`).
    root: PathBuf,
    /// Loaded metadata (last_refresh, per-pack usage stats, index version).
    pub metadata: CacheMetadata,
}

/// Persistent metadata for the SDI cache. Serialized to JSON at
/// `paths::sdi_metadata_path()`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheMetadata {
    /// Version string from the mirror manifest (e.g. "sdi-printer-v1").
    /// None until first sdi refresh.
    pub index_version: Option<String>,
    /// Timestamp of the last successful sdi refresh. None until first
    /// refresh. Used by the stale-index warning logic.
    pub last_refresh: Option<chrono::DateTime<chrono::Utc>>,
    /// Per-pack stats, keyed by pack filename (e.g. "DP_Printer_26000.7z").
    pub packs: HashMap<String, PackMeta>,
}

/// Per-pack usage and integrity metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMeta {
    /// Byte size of the `.7z` pack on disk at registration time.
    pub size_bytes: u64,
    /// Lowercase hex SHA256 of the pack contents at registration time.
    pub sha256: String,
    /// Timestamp of the most recent read of this pack by a SDI resolve.
    /// Used as the LRU sort key during [`SdiCache::prune`].
    pub last_used: chrono::DateTime<chrono::Utc>,
    /// Timestamp of the initial download of this pack. Never updated.
    pub first_cached: chrono::DateTime<chrono::Utc>,
}

impl SdiCache {
    /// Load the cache rooted at [`paths::sdi_dir`].
    ///
    /// Creates `sdi/`, `sdi/indexes/`, and `sdi/drivers/` if missing.
    /// If the metadata file is absent or corrupt, initialises fresh
    /// defaults rather than failing — see the module-level
    /// "Graceful degradation" section.
    pub fn load() -> Result<Self, String> {
        paths::ensure_sdi_dirs()
            .map_err(|e| format!("Failed to create SDI cache directories: {e}"))?;
        Self::load_from_root(paths::sdi_dir())
    }

    /// Load the cache rooted at an arbitrary directory.
    ///
    /// Public so integration tests can point at a tempdir without having
    /// to override [`paths::sdi_dir`] globally. Production code should
    /// call [`SdiCache::load`]; this entry point is primarily a test seam.
    ///
    /// Creates `<root>/indexes/` and `<root>/drivers/` if missing, then
    /// reads `<root>/metadata.json` with the same graceful-degradation
    /// semantics as [`SdiCache::load`].
    pub fn load_from_root(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(root.join("indexes"))
            .map_err(|e| format!("Failed to create SDI indexes dir: {e}"))?;
        fs::create_dir_all(root.join("drivers"))
            .map_err(|e| format!("Failed to create SDI drivers dir: {e}"))?;

        let metadata_path = root.join(METADATA_FILENAME);
        let metadata = match fs::read_to_string(&metadata_path) {
            Ok(contents) => match serde_json::from_str::<CacheMetadata>(&contents) {
                Ok(meta) => meta,
                Err(e) => {
                    eprintln!(
                        "warning: SDI cache metadata at {} is corrupt ({}); using fresh defaults",
                        metadata_path.display(),
                        e
                    );
                    CacheMetadata::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CacheMetadata::default(),
            Err(e) => {
                eprintln!(
                    "warning: unable to read SDI cache metadata at {} ({}); using fresh defaults",
                    metadata_path.display(),
                    e
                );
                CacheMetadata::default()
            }
        };

        Ok(Self { root, metadata })
    }

    /// Check whether a specific pack file is cached on disk AND has a
    /// metadata entry.
    ///
    /// Returns false if the file exists on disk without a metadata
    /// entry (unexpected state — caller should treat as uncached and
    /// re-register via [`SdiCache::register_pack`]) or vice versa.
    /// Returns false for any pack name that fails path-traversal
    /// validation.
    pub fn has_pack(&self, pack_name: &str) -> bool {
        if validate_pack_name(pack_name).is_err() {
            return false;
        }
        if !self.metadata.packs.contains_key(pack_name) {
            return false;
        }
        self.pack_path_unchecked(pack_name).is_file()
    }

    /// Returns the absolute path where a given pack would live in the
    /// cache, regardless of whether it actually exists.
    ///
    /// Rejects names that contain path separators, traversal
    /// components, or anything else that would escape the cache root.
    pub fn pack_path(&self, pack_name: &str) -> Result<PathBuf, String> {
        validate_pack_name(pack_name)?;
        Ok(self.pack_path_unchecked(pack_name))
    }

    /// Internal helper that skips validation — callers must have
    /// already validated the pack name.
    fn pack_path_unchecked(&self, pack_name: &str) -> PathBuf {
        self.root.join("drivers").join(pack_name)
    }

    /// Update the `last_used` timestamp for a pack. Call this every
    /// time a pack is read during a SDI resolve. Persists metadata
    /// immediately — an unclean shutdown shouldn't lose LRU info.
    ///
    /// Returns an error if the pack name fails validation or if no
    /// metadata entry exists for the pack.
    pub fn record_pack_used(&mut self, pack_name: &str) -> Result<(), String> {
        validate_pack_name(pack_name)?;
        let entry = self
            .metadata
            .packs
            .get_mut(pack_name)
            .ok_or_else(|| format!("Pack '{pack_name}' is not registered in the SDI cache"))?;
        entry.last_used = chrono::Utc::now();
        self.save_metadata()
    }

    /// Register a newly-downloaded pack. Computes and stores the SHA256,
    /// the size, and the initial timestamps. If the pack was previously
    /// registered, the `first_cached` timestamp is preserved and
    /// `last_used` is reset to now (treating a re-download as a fresh
    /// access).
    ///
    /// Errors if the pack name fails validation, the file is unreadable,
    /// or the metadata save fails.
    pub fn register_pack(&mut self, pack_name: &str, pack_path: &Path) -> Result<(), String> {
        validate_pack_name(pack_name)?;

        let (size_bytes, sha256) = hash_file(pack_path).map_err(|e| {
            format!(
                "Failed to hash SDI pack at {}: {e}",
                pack_path.display()
            )
        })?;

        let now = chrono::Utc::now();
        let first_cached = self
            .metadata
            .packs
            .get(pack_name)
            .map(|p| p.first_cached)
            .unwrap_or(now);

        self.metadata.packs.insert(
            pack_name.to_string(),
            PackMeta {
                size_bytes,
                sha256,
                last_used: now,
                first_cached,
            },
        );
        self.save_metadata()
    }

    /// Mark an index-bundle refresh as complete. Updates
    /// `metadata.index_version` and `metadata.last_refresh`.
    pub fn record_refresh(&mut self, version: &str) -> Result<(), String> {
        self.metadata.index_version = Some(version.to_string());
        self.metadata.last_refresh = Some(chrono::Utc::now());
        self.save_metadata()
    }

    /// Evict least-recently-used packs past the given byte budget
    /// (expressed in MB). Returns the list of pack names that were
    /// removed. If the total cache is already under budget, returns an
    /// empty vec and touches nothing.
    ///
    /// Packs whose on-disk file is already missing but still present in
    /// metadata are treated as 0-byte entries for budget purposes, but
    /// are still eligible for eviction so the map self-heals toward the
    /// real filesystem state.
    pub fn prune(&mut self, budget_mb: u64) -> Result<Vec<String>, String> {
        let budget_bytes = budget_mb.saturating_mul(1024 * 1024);
        let current = self.total_cache_size_bytes();
        if current <= budget_bytes {
            return Ok(Vec::new());
        }

        // Sort pack names by last_used ascending — oldest first.
        let mut candidates: Vec<(String, chrono::DateTime<chrono::Utc>, u64)> = self
            .metadata
            .packs
            .iter()
            .map(|(name, meta)| (name.clone(), meta.last_used, meta.size_bytes))
            .collect();
        candidates.sort_by_key(|(_, last_used, _)| *last_used);

        let mut removed: Vec<String> = Vec::new();
        let mut running = current;
        for (name, _, size) in candidates {
            if running <= budget_bytes {
                break;
            }
            // Delete the on-disk file (best-effort; missing files are fine).
            let path = self.pack_path_unchecked(&name);
            if path.exists()
                && let Err(e) = fs::remove_file(&path)
            {
                return Err(format!(
                    "Failed to evict SDI pack {}: {e}",
                    path.display()
                ));
            }
            self.metadata.packs.remove(&name);
            running = running.saturating_sub(size);
            removed.push(name);
        }

        self.save_metadata()?;
        Ok(removed)
    }

    /// Returns true if the cache is stale — that is, `last_refresh` is
    /// either `None` (never refreshed) or older than
    /// `refresh_threshold_days` days ago.
    ///
    /// A never-refreshed cache counts as stale so the warning fires at
    /// least once on first use, nudging the user toward running
    /// `prinstall sdi refresh`.
    pub fn is_stale(&self, refresh_threshold_days: u32) -> bool {
        let Some(last) = self.metadata.last_refresh else {
            return true;
        };
        let threshold = chrono::Duration::days(i64::from(refresh_threshold_days));
        chrono::Utc::now().signed_duration_since(last) > threshold
    }

    /// Persist the current metadata to disk via an atomic
    /// write-and-rename pattern. Never called externally; every
    /// mutating method invokes it internally before returning.
    ///
    /// The temp sibling is `metadata.json.tmp` in the same directory so
    /// the `rename` stays on a single filesystem (required for
    /// atomicity on every platform prinstall supports).
    fn save_metadata(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root)
            .map_err(|e| format!("Failed to ensure SDI root exists: {e}"))?;

        let metadata_path = self.root.join(METADATA_FILENAME);
        let tmp_path = self.root.join(format!("{METADATA_FILENAME}.tmp"));

        let contents = serde_json::to_string_pretty(&self.metadata)
            .map_err(|e| format!("Failed to serialize SDI cache metadata: {e}"))?;

        fs::write(&tmp_path, contents.as_bytes()).map_err(|e| {
            format!(
                "Failed to write SDI cache metadata tempfile {}: {e}",
                tmp_path.display()
            )
        })?;

        fs::rename(&tmp_path, &metadata_path).map_err(|e| {
            // Clean up the temp file if the rename failed, so we don't
            // leave stale `.tmp` turds behind on the next load.
            let _ = fs::remove_file(&tmp_path);
            format!(
                "Failed to rename SDI cache metadata into place at {}: {e}",
                metadata_path.display()
            )
        })?;

        Ok(())
    }

    /// Return paths to every `.bin` index file currently cached.
    ///
    /// Enumerates the on-disk `indexes/` directory rather than reading
    /// metadata — index files are managed by the fetcher/refresh flow,
    /// not tracked in `packs`. Returns an empty vec if the directory
    /// is empty, missing, or unreadable.
    pub fn list_cached_indexes(&self) -> Vec<PathBuf> {
        let dir = self.root.join("indexes");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("bin") && path.is_file() {
                out.push(path);
            }
        }
        out.sort();
        out
    }

    /// Return the total size of cached packs in bytes, summed from the
    /// metadata (not re-stat'd off disk). Used by [`SdiCache::prune`]
    /// and by the `prinstall sdi clean` verbose output.
    pub fn total_cache_size_bytes(&self) -> u64 {
        self.metadata
            .packs
            .values()
            .map(|p| p.size_bytes)
            .fold(0u64, |acc, n| acc.saturating_add(n))
    }

    /// Return the absolute cache root — primarily useful in tests that
    /// want to poke the filesystem directly.
    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Validate that `pack_name` is a bare filename safe to join onto the
/// cache root. Rejects:
///
/// - empty strings
/// - anything containing `/` or `\`
/// - any component equal to `.` or `..`
/// - absolute paths and Windows drive-qualified paths
/// - names containing a NUL byte
///
/// Returns `Ok(())` if the name is safe; the caller can then
/// unconditionally join it to `<root>/drivers/`.
fn validate_pack_name(pack_name: &str) -> Result<(), String> {
    if pack_name.is_empty() {
        return Err("SDI pack name is empty".to_string());
    }
    if pack_name.contains('\0') {
        return Err("SDI pack name contains a NUL byte".to_string());
    }
    if pack_name.contains('/') || pack_name.contains('\\') {
        return Err(format!(
            "SDI pack name '{pack_name}' contains a path separator"
        ));
    }
    if pack_name == "." || pack_name == ".." {
        return Err(format!("SDI pack name '{pack_name}' is a path component"));
    }
    // Reject Windows drive prefixes (`C:foo.7z`) and any other colon —
    // NTFS alternate data streams also embed colons in filenames and we
    // don't want them anywhere near the cache path.
    if pack_name.contains(':') {
        return Err(format!(
            "SDI pack name '{pack_name}' contains a colon"
        ));
    }
    // PathBuf-level sanity check: the name must be a single Normal
    // component (no RootDir, Prefix, ParentDir, CurDir).
    let p = Path::new(pack_name);
    let mut components = p.components();
    let first = components
        .next()
        .ok_or_else(|| format!("SDI pack name '{pack_name}' has no components"))?;
    if components.next().is_some() {
        return Err(format!(
            "SDI pack name '{pack_name}' resolves to multiple path components"
        ));
    }
    match first {
        std::path::Component::Normal(_) => Ok(()),
        _ => Err(format!(
            "SDI pack name '{pack_name}' is not a normal filename"
        )),
    }
}

/// Stream `path` through SHA256, returning the total byte length and
/// the lowercase hex digest. Used by [`SdiCache::register_pack`] to
/// fingerprint a just-downloaded `.7z`.
fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut f = fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total = total.saturating_add(n as u64);
    }
    let digest = hasher.finalize();
    let hex = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok((total, hex))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_pack_name_accepts_normal_filename() {
        assert!(validate_pack_name("DP_Printer_26000.7z").is_ok());
    }

    #[test]
    fn validate_pack_name_rejects_traversal() {
        assert!(validate_pack_name("../etc/passwd").is_err());
        assert!(validate_pack_name("..\\etc\\passwd").is_err());
        assert!(validate_pack_name("/etc/passwd").is_err());
        assert!(validate_pack_name("..").is_err());
        assert!(validate_pack_name(".").is_err());
        assert!(validate_pack_name("").is_err());
    }

    #[test]
    fn validate_pack_name_rejects_drive_prefix() {
        assert!(validate_pack_name("C:foo.7z").is_err());
        assert!(validate_pack_name("C:\\foo.7z").is_err());
    }
}

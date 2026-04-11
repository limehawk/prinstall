//! Integration tests for the SDI cache state manager.
//!
//! Every test uses its own tempdir so the suite can run in parallel.
//! No global state is touched: [`SdiCache::load_from_root`] accepts an
//! explicit root path so tests never have to clobber `paths::sdi_dir()`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use prinstall::drivers::sdi::cache::{CacheMetadata, PackMeta, SdiCache};
use sha2::{Digest, Sha256};

/// Monotonic counter so each tempdir in a process run gets a unique
/// name even if two tests grab the same millisecond.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a fresh tempdir for a single test. Not cleaned up on failure
/// so a debugger can still poke at the cache state if a test breaks,
/// but on pass the test finishes fast and the OS tmp cleaner handles it.
fn fresh_tempdir(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("prinstall-sdi-cache-{tag}-{pid}-{millis}-{seq}"));
    fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Write `bytes` to `<dir>/drivers/<name>` and return the full path.
/// Used to synthesize pack files without actually producing a real 7z.
fn write_pack(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let drivers = root.join("drivers");
    fs::create_dir_all(&drivers).expect("create drivers dir");
    let p = drivers.join(name);
    fs::write(&p, bytes).expect("write pack file");
    p
}

/// Compute the expected SHA256 of a byte slice, lowercase hex.
fn expected_sha(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    d.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

// ---------- tests ----------

#[test]
fn load_on_empty_dir_initializes_fresh_metadata() {
    let dir = fresh_tempdir("empty");
    let cache = SdiCache::load_from_root(dir.clone()).expect("load");
    assert!(cache.metadata.last_refresh.is_none());
    assert!(cache.metadata.index_version.is_none());
    assert!(cache.metadata.packs.is_empty());

    // load_from_root should have materialised the standard subdirs.
    assert!(dir.join("indexes").is_dir());
    assert!(dir.join("drivers").is_dir());
}

#[test]
fn load_with_corrupt_metadata_falls_back_to_defaults() {
    let dir = fresh_tempdir("corrupt");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("metadata.json"), b"not valid json {").unwrap();

    let cache = SdiCache::load_from_root(dir).expect("load should not fail");
    assert!(cache.metadata.last_refresh.is_none());
    assert!(cache.metadata.packs.is_empty());
}

#[test]
fn load_with_missing_metadata_file_returns_defaults() {
    // Directory exists, but no metadata.json inside it.
    let dir = fresh_tempdir("missing");
    let cache = SdiCache::load_from_root(dir).expect("load");
    assert!(cache.metadata.packs.is_empty());
    assert!(cache.metadata.index_version.is_none());
}

#[test]
fn register_pack_stores_size_and_sha256() {
    let dir = fresh_tempdir("register");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();

    let payload = b"hello sdi cache, this is a fake pack body";
    let path = write_pack(&dir, "fake.7z", payload);
    cache
        .register_pack("fake.7z", &path)
        .expect("register succeeds");

    assert!(cache.has_pack("fake.7z"));
    let meta = cache
        .metadata
        .packs
        .get("fake.7z")
        .expect("entry present");
    assert_eq!(meta.size_bytes, payload.len() as u64);
    assert_eq!(meta.sha256, expected_sha(payload));
    assert_eq!(meta.first_cached, meta.last_used);

    // Round-trip through save/load to confirm persistence.
    let reloaded = SdiCache::load_from_root(dir).unwrap();
    let meta2 = reloaded.metadata.packs.get("fake.7z").unwrap();
    assert_eq!(meta2.sha256, expected_sha(payload));
    assert_eq!(meta2.size_bytes, payload.len() as u64);
}

#[test]
fn register_pack_preserves_first_cached_on_reregister() {
    let dir = fresh_tempdir("reregister");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();
    let path = write_pack(&dir, "p.7z", b"v1");
    cache.register_pack("p.7z", &path).unwrap();
    let original_first = cache.metadata.packs.get("p.7z").unwrap().first_cached;

    // Manually age the last_used timestamp so the re-register has
    // something visible to advance.
    {
        let m = cache.metadata.packs.get_mut("p.7z").unwrap();
        m.last_used = chrono::Utc::now() - chrono::Duration::days(1);
    }

    let path2 = write_pack(&dir, "p.7z", b"v2-different-bytes");
    cache.register_pack("p.7z", &path2).unwrap();
    let after = cache.metadata.packs.get("p.7z").unwrap();
    assert_eq!(after.first_cached, original_first);
    assert!(after.last_used > original_first - chrono::Duration::seconds(1));
}

#[test]
fn record_pack_used_updates_last_used_and_round_trips() {
    let dir = fresh_tempdir("record_used");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();
    let path = write_pack(&dir, "touched.7z", b"abc");
    cache.register_pack("touched.7z", &path).unwrap();

    // Force the last_used back in time so the record_pack_used advance
    // is observable without relying on sub-millisecond resolution.
    let old = chrono::Utc::now() - chrono::Duration::hours(1);
    cache.metadata.packs.get_mut("touched.7z").unwrap().last_used = old;

    cache.record_pack_used("touched.7z").unwrap();
    let fresh_ts = cache.metadata.packs.get("touched.7z").unwrap().last_used;
    assert!(fresh_ts > old);

    // Round-trip.
    let reloaded = SdiCache::load_from_root(dir).unwrap();
    let persisted = reloaded
        .metadata
        .packs
        .get("touched.7z")
        .unwrap()
        .last_used;
    assert_eq!(persisted, fresh_ts);
}

#[test]
fn record_pack_used_errors_on_unknown_pack() {
    let dir = fresh_tempdir("unknown_used");
    let mut cache = SdiCache::load_from_root(dir).unwrap();
    let err = cache.record_pack_used("ghost.7z").unwrap_err();
    assert!(err.contains("ghost.7z"));
}

#[test]
fn prune_evicts_lru_pack_past_budget() {
    let dir = fresh_tempdir("prune_lru");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();

    // Three real files — tiny byte contents, but the metadata size
    // field is what prune actually consults. We'll stamp synthetic
    // sizes directly after register_pack so the total adds up to 500
    // MB without needing gigabytes of real disk.
    for name in ["old.7z", "mid.7z", "new.7z"] {
        let path = write_pack(&dir, name, b"x");
        cache.register_pack(name, &path).unwrap();
    }

    let mb = 1024 * 1024;
    let now = chrono::Utc::now();

    // Rewrite metadata: 3 packs totalling 500 MB with distinct ages.
    let o = cache.metadata.packs.get_mut("old.7z").unwrap();
    o.size_bytes = 200 * mb;
    o.last_used = now - chrono::Duration::days(10);

    let m = cache.metadata.packs.get_mut("mid.7z").unwrap();
    m.size_bytes = 200 * mb;
    m.last_used = now - chrono::Duration::days(5);

    let n = cache.metadata.packs.get_mut("new.7z").unwrap();
    n.size_bytes = 100 * mb;
    n.last_used = now;

    assert_eq!(cache.total_cache_size_bytes(), 500 * mb);

    // Budget of 300 MB — must evict at least the oldest (200 MB) to
    // get under, and should leave mid + new (300 MB total) intact.
    let removed = cache.prune(300).unwrap();
    assert!(!removed.is_empty(), "expected at least one eviction");
    assert!(removed.contains(&"old.7z".to_string()));
    assert!(!removed.contains(&"new.7z".to_string()));
    assert!(cache.total_cache_size_bytes() <= 300 * mb);

    // old.7z's on-disk file should be gone too.
    assert!(!dir.join("drivers").join("old.7z").exists());
    assert!(dir.join("drivers").join("new.7z").exists());
}

#[test]
fn prune_noop_when_under_budget() {
    let dir = fresh_tempdir("prune_noop");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();
    let path = write_pack(&dir, "small.7z", b"tiny");
    cache.register_pack("small.7z", &path).unwrap();

    let removed = cache.prune(1024).unwrap();
    assert!(removed.is_empty());
    assert!(dir.join("drivers").join("small.7z").exists());
    assert!(cache.metadata.packs.contains_key("small.7z"));
}

#[test]
fn is_stale_returns_true_past_threshold() {
    let dir = fresh_tempdir("stale_past");
    let mut cache = SdiCache::load_from_root(dir).unwrap();
    cache.metadata.last_refresh = Some(chrono::Utc::now() - chrono::Duration::days(45));
    assert!(cache.is_stale(30));
}

#[test]
fn is_stale_returns_false_fresh() {
    let dir = fresh_tempdir("stale_fresh");
    let mut cache = SdiCache::load_from_root(dir).unwrap();
    cache.metadata.last_refresh = Some(chrono::Utc::now());
    assert!(!cache.is_stale(30));
}

#[test]
fn is_stale_returns_true_when_never_refreshed() {
    let dir = fresh_tempdir("stale_never");
    let cache = SdiCache::load_from_root(dir).unwrap();
    assert!(cache.metadata.last_refresh.is_none());
    assert!(cache.is_stale(30));
}

#[test]
fn record_refresh_updates_version_and_timestamp() {
    let dir = fresh_tempdir("refresh");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();
    assert!(cache.metadata.index_version.is_none());
    cache.record_refresh("sdi-printer-v42").unwrap();
    assert_eq!(
        cache.metadata.index_version.as_deref(),
        Some("sdi-printer-v42")
    );
    assert!(cache.metadata.last_refresh.is_some());

    let reloaded = SdiCache::load_from_root(dir).unwrap();
    assert_eq!(
        reloaded.metadata.index_version.as_deref(),
        Some("sdi-printer-v42")
    );
    assert!(reloaded.metadata.last_refresh.is_some());
}

#[test]
fn rejects_path_traversal_in_pack_name() {
    let dir = fresh_tempdir("traversal");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();

    assert!(!cache.has_pack("../../etc/passwd"));
    assert!(cache.pack_path("../../etc/passwd").is_err());
    assert!(cache.pack_path("/etc/passwd").is_err());
    assert!(cache.pack_path("sub/dir.7z").is_err());
    assert!(cache.pack_path("..\\win.7z").is_err());
    assert!(cache.pack_path("C:evil.7z").is_err());
    assert!(cache.pack_path("").is_err());

    // record_pack_used and register_pack should also refuse the name.
    assert!(cache.record_pack_used("../evil.7z").is_err());
    let real = write_pack(&dir, "real.7z", b"ok");
    assert!(cache.register_pack("../boom.7z", &real).is_err());
}

#[test]
fn save_metadata_leaves_no_tempfile_and_round_trips() {
    let dir = fresh_tempdir("atomic_save");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();
    let path = write_pack(&dir, "one.7z", b"one");
    cache.register_pack("one.7z", &path).unwrap();

    // No .tmp sibling left behind after a successful save.
    assert!(dir.join("metadata.json").is_file());
    assert!(!dir.join("metadata.json.tmp").exists());

    // File should be valid JSON that deserialises into CacheMetadata.
    let contents = fs::read_to_string(dir.join("metadata.json")).unwrap();
    let parsed: CacheMetadata =
        serde_json::from_str(&contents).expect("metadata.json is valid JSON");
    assert!(parsed.packs.contains_key("one.7z"));
    let pm: &PackMeta = parsed.packs.get("one.7z").unwrap();
    assert_eq!(pm.size_bytes, 3);
}

#[test]
fn has_pack_false_when_file_missing_but_metadata_present() {
    let dir = fresh_tempdir("orphan_meta");
    let mut cache = SdiCache::load_from_root(dir.clone()).unwrap();
    let path = write_pack(&dir, "phantom.7z", b"bytes");
    cache.register_pack("phantom.7z", &path).unwrap();
    // Nuke the on-disk file, but leave metadata in place.
    fs::remove_file(&path).unwrap();
    assert!(!cache.has_pack("phantom.7z"));
}

#[test]
fn list_cached_indexes_enumerates_bin_files() {
    let dir = fresh_tempdir("indexes");
    let cache = SdiCache::load_from_root(dir.clone()).unwrap();
    assert!(cache.list_cached_indexes().is_empty());

    let ix = dir.join("indexes");
    fs::write(ix.join("DP_Printer_26000.bin"), b"fake").unwrap();
    fs::write(ix.join("DP_ThermoPrinter_26000.bin"), b"fake").unwrap();
    fs::write(ix.join("README.txt"), b"skip me").unwrap();

    let list = cache.list_cached_indexes();
    assert_eq!(list.len(), 2);
    assert!(list
        .iter()
        .all(|p| p.extension().and_then(|s| s.to_str()) == Some("bin")));
}

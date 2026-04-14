#![cfg(feature = "sdi")]

//! Integration tests for `src/drivers/sdi/pack.rs`.
//!
//! These tests exercise both the synthetic-7z path (built with
//! `sevenz_rust2::ArchiveWriter`) and, when the real SDIO Printer pack
//! fixture is present on disk, the 1.48 GB production pack at
//! `/tmp/sdio-check/packs/DP_Printer_26000.7z`.
//!
//! The real-pack test asserts that the prefix filter actually filters
//! — it extracts a single Brother subdirectory and checks the total
//! extracted size is well under 100 MB (proving we didn't accidentally
//! decompress the whole 1.48 GB archive).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sevenz_rust2::{ArchiveEntry, ArchiveWriter};

use prinstall::drivers::sdi::pack::{
    extract_driver_directory, list_entries_matching_prefix,
};

// -- Test helpers ------------------------------------------------------------

/// Fresh temp directory under the system tempdir, deleted if it
/// already exists from a previous run. Returns the absolute path.
fn fresh_temp_dir(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("prinstall-sdi-pack-test-{tag}"));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Total size of all regular files under `root`, recursive.
fn total_file_size(root: &Path) -> u64 {
    fn walk(p: &Path, acc: &mut u64) {
        if let Ok(md) = fs::metadata(p) {
            if md.is_file() {
                *acc += md.len();
                return;
            }
        }
        if let Ok(entries) = fs::read_dir(p) {
            for entry in entries.flatten() {
                walk(&entry.path(), acc);
            }
        }
    }
    let mut acc = 0u64;
    walk(root, &mut acc);
    acc
}

/// Number of regular files under `root`, recursive.
fn count_files(root: &Path) -> usize {
    fn walk(p: &Path, acc: &mut usize) {
        if let Ok(md) = fs::metadata(p) {
            if md.is_file() {
                *acc += 1;
                return;
            }
        }
        if let Ok(entries) = fs::read_dir(p) {
            for entry in entries.flatten() {
                walk(&entry.path(), acc);
            }
        }
    }
    let mut acc = 0usize;
    walk(root, &mut acc);
    acc
}

/// Build a synthetic 7z archive at `pack_path` containing the given
/// `(archive_name, contents)` entries. Uses non-solid compression
/// (one entry at a time via `push_archive_entry`) which is simpler
/// and still exercises the prefix filter.
fn build_synthetic_pack(pack_path: &Path, entries: &[(&str, &[u8])]) {
    if let Some(parent) = pack_path.parent() {
        fs::create_dir_all(parent).expect("create pack parent");
    }
    let mut writer = ArchiveWriter::create(pack_path).expect("create pack writer");
    for (name, contents) in entries {
        let entry = ArchiveEntry::new_file(name);
        writer
            .push_archive_entry(entry, Some(std::io::Cursor::new(*contents)))
            .expect("push entry");
    }
    writer.finish().expect("finish pack");
}

// -- Synthetic-pack tests ----------------------------------------------------

#[test]
fn extracts_from_synthetic_multi_file_7z() {
    let tmp = fresh_temp_dir("extract-synth");
    let pack = tmp.join("pack.7z");
    let dest = tmp.join("dest");

    let inf_body =
        b"[Version]\nSignature=\"$Windows NT$\"\nClass=Printer\nProvider=\"Brother Mock\"\n";
    let cat_body = b"cat file bytes";
    let dll_body = b"fake dll bytes";
    let canon_body = b"should NOT be extracted";

    build_synthetic_pack(
        &pack,
        &[
            ("brother/mock_model/amd64/brother.inf", inf_body),
            ("brother/mock_model/amd64/brother.cat", cat_body),
            ("brother/mock_model/amd64/brother.dll", dll_body),
            ("canon/other_model/x86/canon.inf", canon_body),
        ],
    );

    let inf_path = extract_driver_directory(
        &pack,
        "brother/mock_model/amd64/",
        "brother.inf",
        &dest,
    )
    .expect("extraction should succeed");

    // (a) All three brother files are extracted
    assert!(dest.join("brother/mock_model/amd64/brother.inf").is_file());
    assert!(dest.join("brother/mock_model/amd64/brother.cat").is_file());
    assert!(dest.join("brother/mock_model/amd64/brother.dll").is_file());

    // (b) The canon file is NOT extracted — prefix filter working
    assert!(!dest.join("canon/other_model/x86/canon.inf").exists());
    assert!(
        !dest.join("canon").exists(),
        "canon/ directory should not have been created"
    );

    // (c) Returned INF path points at dest/.../brother.inf and reads
    //     back the expected content
    assert_eq!(inf_path, dest.join("brother/mock_model/amd64/brother.inf"));
    let roundtrip = fs::read(&inf_path).expect("read extracted inf");
    assert_eq!(roundtrip, inf_body);
}

#[test]
fn rejects_missing_inf_filename() {
    let tmp = fresh_temp_dir("missing-inf");
    let pack = tmp.join("pack.7z");
    let dest = tmp.join("dest");

    build_synthetic_pack(
        &pack,
        &[
            ("brother/mock_model/amd64/brother.inf", b"[Version]"),
            ("brother/mock_model/amd64/brother.cat", b"cat"),
        ],
    );

    let err = extract_driver_directory(
        &pack,
        "brother/mock_model/amd64/",
        "nonexistent.inf",
        &dest,
    )
    .expect_err("should fail — INF not present under prefix");

    assert!(
        err.to_lowercase().contains("nonexistent.inf"),
        "error should mention the missing INF: {err}"
    );
}

#[test]
fn rejects_path_traversal() {
    let tmp = fresh_temp_dir("traversal");
    let pack = tmp.join("pack.7z");
    let dest = tmp.join("dest");

    // Synthesize a pack whose entry name contains a parent segment.
    // sevenz-rust2's writer doesn't validate names, so we can smuggle
    // the hostile path right through to the reader.
    build_synthetic_pack(
        &pack,
        &[
            ("evil/../escaped.txt", b"malicious"),
            ("brother/real/amd64/real.inf", b"[Version]"),
        ],
    );

    let err = extract_driver_directory(
        &pack,
        "brother/real/amd64/",
        "real.inf",
        &dest,
    )
    .expect_err("should reject path traversal");

    assert!(
        err.contains("..") || err.to_lowercase().contains("traversal"),
        "error should mention path traversal: {err}"
    );

    // And nothing under dest/ should have been written outside the
    // target directory.
    assert!(!dest.join("escaped.txt").exists());
}

#[test]
fn handles_nonexistent_pack() {
    let dest = fresh_temp_dir("nonexistent-dest");
    let missing = Path::new("/tmp/prinstall-sdi-pack-does-not-exist-12345.7z");
    let _ = fs::remove_file(missing);

    let err = extract_driver_directory(missing, "brother/foo/", "foo.inf", &dest)
        .expect_err("should fail on missing pack");
    assert!(
        err.to_lowercase().contains("not found") || err.to_lowercase().contains("no such"),
        "error should mention missing pack: {err}"
    );

    let err = list_entries_matching_prefix(missing, "brother/")
        .expect_err("list should fail on missing pack");
    assert!(
        err.to_lowercase().contains("not found") || err.to_lowercase().contains("no such"),
        "list error should mention missing pack: {err}"
    );
}

#[test]
fn list_entries_matches_prefix() {
    let tmp = fresh_temp_dir("list-prefix");
    let pack = tmp.join("pack.7z");

    build_synthetic_pack(
        &pack,
        &[
            ("brother/mock_model/amd64/brother.inf", b"[Version]"),
            ("brother/mock_model/amd64/brother.cat", b"cat"),
            ("brother/other/brother2.inf", b"[Version]"),
            ("canon/other_model/x86/canon.inf", b"[Version]"),
            ("canon/other_model/x86/canon.cat", b"cat"),
        ],
    );

    let brother = list_entries_matching_prefix(&pack, "brother/")
        .expect("list should succeed");
    let canon = list_entries_matching_prefix(&pack, "canon/")
        .expect("list should succeed");
    let all = list_entries_matching_prefix(&pack, "")
        .expect("list should succeed");

    assert_eq!(brother.len(), 3, "expected 3 brother entries, got {brother:?}");
    assert!(brother.iter().all(|(n, _)| n.to_lowercase().starts_with("brother/")));

    assert_eq!(canon.len(), 2, "expected 2 canon entries, got {canon:?}");
    assert!(canon.iter().all(|(n, _)| n.to_lowercase().starts_with("canon/")));

    assert_eq!(all.len(), 5, "empty prefix should match all entries");
}

#[test]
fn list_prefix_is_case_insensitive() {
    let tmp = fresh_temp_dir("list-case");
    let pack = tmp.join("pack.7z");

    build_synthetic_pack(
        &pack,
        &[
            ("Brother/Allx64/FORCED/-Gen/BHPCL5E.INF", b"[Version]"),
            ("Brother/Allx64/FORCED/-Gen/bhpcl5e.cat", b"cat"),
        ],
    );

    // Match with lowercase prefix even though pack uses mixed case.
    let hits = list_entries_matching_prefix(&pack, "brother/allx64/forced/-gen/")
        .expect("list should succeed");
    assert_eq!(hits.len(), 2);
}

// -- Real fixture test -------------------------------------------------------

const REAL_PACK: &str = "/tmp/sdio-check/packs/DP_Printer_26000.7z";

/// The main validation signal: given a real 1.48 GB SDIO Printer pack,
/// extract a single Brother vendor subdirectory and confirm the prefix
/// filter actually filters — the extracted bytes must be tiny compared
/// to the full pack.
#[test]
fn extracts_subdir_from_real_sdi_pack() {
    let pack = Path::new(REAL_PACK);
    if !pack.exists() {
        // Fixture not available on this machine — skip with a
        // human-readable reason written to stderr so it's obvious in
        // the test output. CI and machines without the local fixture
        // will simply noop this test rather than fail.
        let _ = writeln!(
            std::io::stderr(),
            "[sdi_pack::extracts_subdir_from_real_sdi_pack] skipping: fixture not present at {REAL_PACK}"
        );
        return;
    }

    let tmp = fresh_temp_dir("real-pack");

    // Step 1: discover what exists via list_entries_matching_prefix.
    // The real Printer pack contains `Brother/Allx64/FORCED/-Gen/`
    // which is a small 8-file subtree (~317 KB uncompressed) with a
    // BHPCL5E.INF at the root. That's our target.
    let prefix_raw = "Brother/Allx64/FORCED/-Gen/";
    let inf_filename = "BHPCL5E.INF";

    let listing = list_entries_matching_prefix(pack, prefix_raw)
        .expect("list should succeed on real pack");
    assert!(
        !listing.is_empty(),
        "expected at least one entry under {prefix_raw}"
    );
    let listing_total: u64 = listing.iter().map(|(_, sz)| *sz).sum();
    assert!(
        listing_total < 50 * 1024 * 1024,
        "listed prefix total should be << 50 MB, got {listing_total} bytes"
    );

    // Confirm the target INF exists in the listing (case-insensitive).
    assert!(
        listing
            .iter()
            .any(|(n, _)| n.to_lowercase().ends_with(&inf_filename.to_lowercase())),
        "expected {inf_filename} in listing: {listing:?}"
    );

    // Step 2: extract it. This is the load-bearing assertion — if the
    // filter closure doesn't filter, we'll write gigabytes to /tmp and
    // the total-size check below will blow up.
    let inf_path = extract_driver_directory(pack, prefix_raw, inf_filename, &tmp)
        .expect("real-pack extraction should succeed");

    // (a) Return Ok — already asserted above via .expect.

    // (b) At least one file written to the dest dir.
    let n_files = count_files(&tmp);
    assert!(
        n_files >= 1,
        "expected at least one extracted file, got {n_files}"
    );

    // (c) Total extracted size << 100 MB (proves the prefix filter
    //     worked and the whole 1.48 GB pack wasn't decompressed).
    let total = total_file_size(&tmp);
    const MAX_ALLOWED: u64 = 100 * 1024 * 1024; // 100 MB
    assert!(
        total < MAX_ALLOWED,
        "extracted size {total} bytes should be << {MAX_ALLOWED} bytes — prefix filter broken?"
    );

    // (d) The returned INF path exists on disk.
    assert!(
        inf_path.exists(),
        "returned INF path should exist: {}",
        inf_path.display()
    );
    assert!(
        inf_path.is_file(),
        "returned INF path should be a regular file: {}",
        inf_path.display()
    );
    assert!(
        inf_path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(&inf_filename.to_lowercase()),
        "returned INF path should end with {inf_filename}: {}",
        inf_path.display()
    );

    // Sanity: extracted count should match the listing count.
    assert_eq!(
        n_files,
        listing.len(),
        "extracted file count ({n_files}) should match listing count ({})",
        listing.len()
    );
}

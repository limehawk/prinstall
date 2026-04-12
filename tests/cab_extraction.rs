//! Integration tests for `drivers::cab::extract_cab_to_dir`.
//!
//! Uses the `cab` crate's writer API (`CabinetBuilder` → `CabinetWriter`) to
//! build small in-memory CAB archives at test time and then calls our
//! extraction helper on the resulting bytes. This avoids committing a binary
//! fixture file — the test is fully self-contained and regenerates the
//! archive on every run, so there's no drift risk between the fixture and
//! the crate version.

use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use cab::{CabinetBuilder, CompressionType};
use prinstall::drivers::cab::extract_cab_to_dir;

/// Build a small in-memory CAB archive containing one file called `name`
/// with the given `content`. Returns the raw CAB bytes.
fn build_single_file_cab(name: &str, content: &[u8]) -> Vec<u8> {
    let mut builder = CabinetBuilder::new();
    builder
        .add_folder(CompressionType::None)
        .add_file(name.to_string());
    let buffer = Cursor::new(Vec::new());
    let mut writer = builder.build(buffer).expect("build cabinet writer");
    {
        let mut file_writer = writer
            .next_file()
            .expect("open first file")
            .expect("first file present");
        file_writer.write_all(content).expect("write file data");
    }
    // Drain any remaining declared entries so finish() accepts the writer.
    while writer.next_file().expect("advance file").is_some() {
        // no-op
    }
    let cursor = writer.finish().expect("finish cabinet writer");
    cursor.into_inner()
}

/// Build a small in-memory CAB archive containing multiple files in a single
/// folder. Each entry is `(archived_name, content)`.
fn build_multi_file_cab(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = CabinetBuilder::new();
    {
        let folder = builder.add_folder(CompressionType::None);
        for (name, _) in files {
            folder.add_file(name.to_string());
        }
    }
    let buffer = Cursor::new(Vec::new());
    let mut writer = builder.build(buffer).expect("build cabinet writer");
    for (_, content) in files {
        let mut file_writer = writer
            .next_file()
            .expect("open file")
            .expect("file present");
        file_writer.write_all(content).expect("write file data");
    }
    // Drain any remaining declared entries so finish() doesn't complain.
    while writer.next_file().expect("advance file").is_some() {
        // no-op
    }
    let cursor = writer.finish().expect("finish cabinet writer");
    cursor.into_inner()
}

/// Per-test temp dir helper. Uses the test name as the subdir so parallel
/// test runs don't collide.
fn temp_dir(test_name: &str) -> PathBuf {
    let base = std::env::temp_dir().join("prinstall-cab-tests").join(test_name);
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create temp dir");
    base
}

#[test]
fn extracts_a_single_file_cab() {
    let dest = temp_dir("single_file");
    let content = b"Hello from a CAB file!";
    let bytes = build_single_file_cab("greeting.txt", content);

    let written = extract_cab_to_dir(&bytes, &dest).expect("extract single-file cab");
    assert_eq!(written.len(), 1, "should extract exactly one file");

    let extracted = fs::read(&written[0]).expect("read extracted file");
    assert_eq!(extracted, content, "extracted content should match original");
    assert_eq!(
        written[0].file_name().and_then(|n| n.to_str()),
        Some("greeting.txt")
    );
}

#[test]
fn extracts_multiple_files_with_distinct_content() {
    let dest = temp_dir("multi_file");
    let files: &[(&str, &[u8])] = &[
        ("readme.txt", b"# Fixture README\nInside a CAB."),
        ("data.bin", &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]),
        ("inf/driver.inf", b"[Version]\nSignature=\"$WINDOWS NT$\"\n"),
    ];
    let bytes = build_multi_file_cab(files);

    let written = extract_cab_to_dir(&bytes, &dest).expect("extract multi-file cab");
    assert_eq!(written.len(), files.len(), "all files should be extracted");

    for (name, expected) in files {
        // Rebuild the expected output path — accounting for the subdirectory
        // in `inf/driver.inf`. Our helper normalises backslashes to forward
        // slashes, so paths containing `/` round-trip cleanly.
        let outpath = {
            let mut p = dest.clone();
            for seg in name.split('/').filter(|s| !s.is_empty()) {
                p.push(seg);
            }
            p
        };
        let got = fs::read(&outpath)
            .unwrap_or_else(|e| panic!("read extracted {}: {}", outpath.display(), e));
        assert_eq!(&got, expected, "content for {name} should match");
    }
}

#[test]
fn rejects_empty_bytes() {
    let dest = temp_dir("empty");
    let err = extract_cab_to_dir(&[], &dest).expect_err("empty input should fail");
    assert!(
        err.contains("CAB") || err.contains("Invalid"),
        "error should mention CAB parsing: {err}"
    );
}

#[test]
fn rejects_random_garbage() {
    let dest = temp_dir("garbage");
    let garbage = b"this is definitely not a cabinet file, just text and bytes".to_vec();
    let err = extract_cab_to_dir(&garbage, &dest).expect_err("garbage should fail");
    // Error message surface varies by cab crate version — just assert we
    // returned Err, not panicked.
    assert!(!err.is_empty(), "error message should not be empty");
}

#[test]
fn creates_destination_directory_if_missing() {
    let base = temp_dir("creates_dest");
    let dest = base.join("nested").join("missing").join("dir");
    assert!(!dest.exists(), "pre-condition: destination should not exist");

    let bytes = build_single_file_cab("probe.txt", b"probe");
    let written = extract_cab_to_dir(&bytes, &dest).expect("extract into missing dir");

    assert_eq!(written.len(), 1);
    assert!(dest.exists(), "destination should be created");
    assert!(written[0].exists(), "extracted file should exist at returned path");
}

#[test]
fn preserves_nested_paths() {
    let dest = temp_dir("nested_paths");
    let files: &[(&str, &[u8])] = &[
        ("top.txt", b"top"),
        ("a/nested.txt", b"nested"),
        ("a/b/deep.txt", b"deep"),
    ];
    let bytes = build_multi_file_cab(files);

    extract_cab_to_dir(&bytes, &dest).expect("extract nested");

    assert!(dest.join("top.txt").is_file());
    assert!(dest.join("a").join("nested.txt").is_file());
    assert!(dest.join("a").join("b").join("deep.txt").is_file());
    assert_eq!(fs::read(dest.join("top.txt")).unwrap(), b"top");
    assert_eq!(fs::read(dest.join("a/nested.txt")).unwrap(), b"nested");
    assert_eq!(fs::read(dest.join("a/b/deep.txt")).unwrap(), b"deep");
}

#![cfg(feature = "sdi")]

//! Integration tests for the SDW index parser.
//!
//! The headline test is `parses_real_dp_printer_26000_bin`, which loads a
//! real, production-grade `.bin` from an SDIO installation on the dev
//! machine and walks the full HWID → driver chain. It's gated behind
//! the `real-sdi-fixtures` feature so CI doesn't break if the fixture
//! isn't present, but you should run it locally to validate the parser
//! against real data.

use prinstall::drivers::sdi::index::{parse_index, parse_index_file, SdiIndex};

/// Path to the real SDW fixture used for parser validation. Not committed
/// to the repo — it's copyrighted SDIO data. The test skips gracefully if
/// the file isn't present on the dev box.
const REAL_SDW_PATH: &str = "/tmp/sdio-check/indexes/DP_Printer_26000.bin";

/// THE test. Opens a real 1.13 MB `.bin` from an SDIO install, asserts
/// the header parses, decompresses the LZMA payload, walks every section
/// vector, validates cross-references, and proves a known HWID resolves
/// end-to-end through desc → manufacturer → inffile.
///
/// When the fixture is absent (e.g. on CI) we print a note and bail,
/// because demanding the fixture on every run would block upstream.
/// Locally you should always see this pass.
#[test]
fn parses_real_dp_printer_26000_bin() {
    let path = std::path::Path::new(REAL_SDW_PATH);
    if !path.exists() {
        eprintln!(
            "skipping parses_real_dp_printer_26000_bin: fixture {REAL_SDW_PATH} not present"
        );
        return;
    }

    let idx = parse_index_file(path).expect("real SDW file must parse");

    assert_eq!(idx.pack_name, "DP_Printer_26000");
    assert_eq!(
        idx.version, 0x0000_0205,
        "expected SDW version 0x205, got 0x{:x}",
        idx.version
    );
    assert!(
        idx.hwid_count() > 100_000,
        "DP_Printer_26000 should have hundreds of thousands of HWID entries, got {}",
        idx.hwid_count()
    );
    assert_eq!(idx.pack_filename(), "DP_Printer_26000.7z");

    // Known-good HWID that we verified exists in this file during format
    // discovery. The matcher is case-insensitive by spec, so we also try
    // a lowercase variant to cover that code path in one shot.
    let hits_upper = idx.find_matching(&["1284_CID_BROTHER_LASER_TYPE1".to_string()]);
    assert!(
        !hits_upper.is_empty(),
        "expected at least one hit for 1284_CID_BROTHER_LASER_TYPE1"
    );
    let hit = &hits_upper[0];
    assert_eq!(hit.driver_manufacturer, "Brother");
    assert!(
        hit.inf_filename.to_ascii_lowercase().ends_with(".inf"),
        "inf_filename should look like a .inf, got {:?}",
        hit.inf_filename
    );
    assert!(
        hit.inf_dir_prefix.ends_with('/'),
        "inf_dir_prefix must carry a trailing slash, got {:?}",
        hit.inf_dir_prefix
    );
    assert!(
        !hit.inf_dir_prefix.contains('\\'),
        "inf_dir_prefix must be forward-slash normalised, got {:?}",
        hit.inf_dir_prefix
    );
    assert!(
        hit.driver_display_name
            .to_ascii_lowercase()
            .contains("brother"),
        "driver_display_name should name the manufacturer, got {:?}",
        hit.driver_display_name
    );

    // Lowercase candidate — same HWID should still hit because matching
    // is explicitly case-insensitive.
    let hits_lower = idx.find_matching(&["1284_cid_brother_laser_type1".to_string()]);
    assert_eq!(
        hits_lower.len(),
        hits_upper.len(),
        "case-insensitive matching: lowercase variant must return same hits"
    );

    // Cold miss: nonsense HWID returns empty.
    let hits_none = idx.find_matching(&["ABSOLUTELY_NOT_A_REAL_HWID_ZZZ".to_string()]);
    assert!(hits_none.is_empty(), "bogus HWID must return no hits");
}

/// Magic check — anything that isn't "SDW" at the top should bounce
/// with an error that mentions the magic.
#[test]
fn rejects_wrong_magic() {
    // 32-byte buffer starting with "SDX".
    let mut buf = vec![0u8; 32];
    buf[0..3].copy_from_slice(b"SDX");
    // valid-looking version so we confirm the magic check fires first
    buf[3..7].copy_from_slice(&0x0205u32.to_le_bytes());

    let err = parse_index(&buf).unwrap_err();
    assert!(
        err.contains("magic") || err.contains("SDW"),
        "expected magic error, got: {err}"
    );
}

/// Unknown version number (0x999 is outside the accepted 0x200..=0x2FF
/// range) should produce a structured version error.
#[test]
fn rejects_wrong_version() {
    let mut buf = vec![0u8; 32];
    buf[0..3].copy_from_slice(b"SDW");
    buf[3..7].copy_from_slice(&0x0000_0999u32.to_le_bytes());

    let err = parse_index(&buf).unwrap_err();
    assert!(err.contains("version"), "expected version error, got: {err}");
}

/// A valid header followed by a too-short payload should produce a
/// structured LZMA-decode error, never a panic.
#[test]
fn rejects_truncated_body() {
    let mut buf = vec![0u8; 8];
    buf[0..3].copy_from_slice(b"SDW");
    buf[3..7].copy_from_slice(&0x0205u32.to_le_bytes());
    // Only four bytes of "payload" — not enough for even the LZMA Alone
    // 13-byte header.
    buf.extend_from_slice(&[0u8; 4]);

    let err = parse_index(&buf).unwrap_err();
    // We don't care about the specific error text — just that we got one
    // and it came from the LZMA path.
    assert!(
        err.to_ascii_lowercase().contains("lzma")
            || err.to_ascii_lowercase().contains("decompress"),
        "expected LZMA decode error, got: {err}"
    );
}

/// A valid header followed by 200 bytes of 0xFF should also produce a
/// structured error — 0xFF isn't a valid LZMA properties byte (max is
/// 224), and even if it were, random noise will fail the arithmetic
/// decoder almost immediately.
#[test]
fn rejects_garbage_payload() {
    let mut buf = vec![0u8; 8];
    buf[0..3].copy_from_slice(b"SDW");
    buf[3..7].copy_from_slice(&0x0205u32.to_le_bytes());
    buf.extend_from_slice(&[0xFFu8; 200]);

    let result = parse_index(&buf);
    assert!(
        result.is_err(),
        "0xFF garbage payload must produce a structured error"
    );
}

/// `parse_index_file` on a path that doesn't exist should return an
/// error referencing the path, not panic.
#[test]
fn parse_index_file_missing_path_errors_cleanly() {
    let err = parse_index_file(std::path::Path::new("/tmp/does-not-exist-prinstall-sdi.bin"))
        .unwrap_err();
    assert!(err.to_ascii_lowercase().contains("read"), "got: {err}");
}

/// Smoke check on the `SdiIndex` type import path so `cargo test
/// --test sdi_index` fails fast if someone accidentally removes the
/// public re-export down the line.
#[allow(dead_code)]
fn _compile_check(_idx: &SdiIndex) {}

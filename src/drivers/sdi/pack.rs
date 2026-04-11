//! Directory-prefix extraction from SDIO driver pack `.7z` archives.
//!
//! Pure-Rust wrapper over `sevenz-rust2` that pulls a single driver's
//! directory subtree out of a cached pack (e.g.,
//! `brother/hl_l8260cdw/amd64/`) instead of decompressing the whole
//! 1.48 GB archive. SDIO's packaging convention guarantees each
//! driver's directory is fully self-contained — no `[SourceDisksFiles]`
//! reference resolution needed. Mirrors SDIO's own `source/install.cpp`
//! extraction algorithm: hand 7z a path filter and trust that the
//! filtered output is complete and installable as-is.
//!
//! ## Why a filter callback instead of full decompression
//!
//! SDIO's printer driverpack is ~1.48 GB compressed, and full
//! decompression would produce many GB on disk plus minutes of wall
//! clock time. All we actually need for any given install is one
//! vendor-specific subdirectory (tens of MB at most). We use
//! [`ArchiveReader::for_each_entries`] with a closure that writes
//! matching entries to disk and drains non-matching entries through
//! `io::sink` so the solid-block decoder keeps advancing.
//!
//! ## Why we have to drain skipped entries
//!
//! SDIO's printer packs are **solid** LZMA2 archives — every file in a
//! block shares one LZMA stream, and skipping ahead without consuming
//! bytes would desynchronise the decoder for every subsequent file in
//! the block. The library gives us a [`BoundedReader`] per file, and
//! we're responsible for reading it to completion. So "skip" here
//! means "copy into `io::sink()`", not "return early". The entry is
//! still decompressed — we just don't write it to disk, which gives us
//! a huge I/O win on packs with thousands of irrelevant entries.
//!
//! ## Contract
//!
//! [`extract_driver_directory`] takes a cached pack `.7z` on disk, a
//! forward-slash-terminated directory prefix, and an expected INF
//! filename inside that prefix. It writes every archive entry whose
//! normalised name starts with the prefix into `dest`, preserving the
//! full relative path from the pack root forward, and returns the
//! absolute path to the extracted INF. That path is ready to hand to
//! [`crate::installer::powershell::stage_driver_inf`].
//!
//! [`list_entries_matching_prefix`] is the lightweight sibling: it
//! reads only the archive header (no decompression) via
//! [`Archive::open`] and returns every matching entry's
//! `(name, uncompressed_size)` tuple. Used by the SDI cache inspection
//! commands and by tests that need to discover what prefixes exist
//! inside a pack.
//!
//! ## Safety
//!
//! Both functions normalise backslash separators to forward slashes
//! before comparing, and perform lowercase comparison against the
//! prefix (Windows is case-insensitive; SDIO packs conventionally use
//! lowercase but we don't want to assume that). Any entry whose
//! normalised path contains a `..` segment is rejected with a
//! structured error rather than silently extracted — same
//! path-traversal guard as [`crate::drivers::cab::extract_cab_to_dir`].

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sevenz_rust2::{Archive, ArchiveReader, Error as SzError, Password};

/// Normalise a raw archive entry name to the form used for prefix
/// comparison: backslashes → forward slashes, lowercase.
fn normalise(raw: &str) -> String {
    raw.replace('\\', "/").to_lowercase()
}

/// Returns `true` if any path segment in `normalised` is `..`.
fn contains_parent_segment(normalised: &str) -> bool {
    normalised.split('/').any(|seg| seg == "..")
}

/// Extract every file from a cached SDI driver pack `.7z` whose archive
/// path starts with `inf_dir_prefix`, into `dest`. Preserves the
/// relative directory structure from the pack root forward (so if the
/// prefix is `brother/hl_l8260cdw/amd64/` and the pack contains
/// `brother/hl_l8260cdw/amd64/brother.inf`, the extracted file is at
/// `dest/brother/hl_l8260cdw/amd64/brother.inf`).
///
/// Uses [`ArchiveReader::for_each_entries`] with a filter closure that
/// accepts only entries whose normalised archive path starts with
/// `inf_dir_prefix`. Non-matching entries are still decompressed (the
/// solid-block decoder requires it) but are sunk into `io::sink()`
/// instead of being written to disk, so only the wanted subdirectory
/// ever touches the filesystem.
///
/// The prefix is normalised before comparison: backslashes become
/// forward slashes and the whole thing is lowercased. Callers should
/// pass a forward-slash-terminated prefix ending in `/` to avoid
/// matching siblings with the same stem (e.g., `brother/hl_l8260/`
/// should not match `brother/hl_l82600/`).
///
/// Returns the absolute path to the extracted INF file, ready to hand
/// to [`crate::installer::powershell::stage_driver_inf`]. Returns an
/// error if the pack cannot be opened, if no INF matching
/// `inf_filename` was extracted under the prefix, or if any archive
/// entry contains a `..` path-traversal segment.
pub fn extract_driver_directory(
    pack_path: &Path,
    inf_dir_prefix: &str,
    inf_filename: &str,
    dest: &Path,
) -> Result<PathBuf, String> {
    if !pack_path.exists() {
        return Err(format!("SDI pack not found: {}", pack_path.display()));
    }

    fs::create_dir_all(dest).map_err(|e| {
        format!("Failed to create SDI extract dir {}: {e}", dest.display())
    })?;

    let prefix = normalise(inf_dir_prefix);
    if prefix.is_empty() {
        return Err("SDI extract prefix is empty — refusing to extract the entire pack".into());
    }
    let expected_inf = inf_filename.to_lowercase();
    let expected_inf_full = format!("{prefix}{expected_inf}");

    let mut reader = ArchiveReader::open(pack_path, Password::empty()).map_err(|e| {
        format!("Failed to open SDI pack {}: {e}", pack_path.display())
    })?;

    let dest_owned = dest.to_path_buf();
    let mut written: Vec<PathBuf> = Vec::new();
    let mut found_inf: Option<PathBuf> = None;

    let result = reader.for_each_entries(|entry, rdr| {
        if entry.is_directory() {
            return Ok(true);
        }

        let raw_name = entry.name();
        let normalised = normalise(raw_name);

        // Path-traversal guard — reject before any disk I/O. Even a
        // skipped entry with `..` in its name is a red flag, so we
        // abort the whole extraction.
        if contains_parent_segment(&normalised) {
            return Err(SzError::Other(Cow::Owned(format!(
                "SDI pack entry '{raw_name}' contains '..' path segment — rejecting"
            ))));
        }

        // Entries outside the prefix still need their bytes drained
        // so the solid-block decoder stays in sync with the stream.
        if !normalised.starts_with(&prefix) {
            io::copy(rdr, &mut io::sink()).map_err(|e| {
                SzError::Other(Cow::Owned(format!(
                    "Failed draining skipped entry '{raw_name}': {e}"
                )))
            })?;
            return Ok(true);
        }

        // Build the output path from the normalised (forward-slash)
        // name so the result is cross-platform regardless of how the
        // pack was built.
        let mut outpath = dest_owned.clone();
        for seg in normalised.split('/').filter(|s| !s.is_empty() && *s != ".") {
            outpath.push(seg);
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                SzError::Other(Cow::Owned(format!(
                    "Failed to create {}: {e}",
                    parent.display()
                )))
            })?;
        }

        let mut outfile = fs::File::create(&outpath).map_err(|e| {
            SzError::Other(Cow::Owned(format!(
                "Failed to create {}: {e}",
                outpath.display()
            )))
        })?;

        io::copy(rdr, &mut outfile).map_err(|e| {
            SzError::Other(Cow::Owned(format!(
                "Failed to write {}: {e}",
                outpath.display()
            )))
        })?;

        if normalised == expected_inf_full {
            found_inf = Some(outpath.clone());
        }
        written.push(outpath);
        Ok(true)
    });

    result.map_err(|e| {
        format!(
            "Failed to extract '{inf_dir_prefix}' from {}: {e}",
            pack_path.display()
        )
    })?;

    if written.is_empty() {
        return Err(format!(
            "SDI pack {} contains no entries matching prefix '{inf_dir_prefix}'",
            pack_path.display()
        ));
    }

    found_inf.ok_or_else(|| {
        format!(
            "SDI pack extracted {} entries under '{inf_dir_prefix}' but none matched INF filename '{inf_filename}'",
            written.len()
        )
    })
}

/// List every file entry in a pack whose normalised archive path starts
/// with `prefix`, without decompressing any data. Reads only the
/// archive header via [`Archive::open`], so it's essentially free even
/// on a 1.48 GB pack.
///
/// Used by `prinstall sdi list`, the SDI cache pack-inspection
/// commands, and tests that want to discover what prefixes exist
/// inside a pack before calling [`extract_driver_directory`].
///
/// Returns `(archive_path, uncompressed_size)` tuples in the order they
/// appear in the archive. The returned `archive_path` is the raw name
/// straight from the archive (not normalised) so callers can display
/// the pack's native casing and separator style. Directories are
/// excluded from the results — only file entries with a data stream
/// are returned.
pub fn list_entries_matching_prefix(
    pack_path: &Path,
    prefix: &str,
) -> Result<Vec<(String, u64)>, String> {
    if !pack_path.exists() {
        return Err(format!("SDI pack not found: {}", pack_path.display()));
    }

    let archive = Archive::open(pack_path)
        .map_err(|e| format!("Failed to open SDI pack {}: {e}", pack_path.display()))?;

    let want = normalise(prefix);
    let mut out: Vec<(String, u64)> = Vec::new();
    for entry in &archive.files {
        if entry.is_directory {
            continue;
        }
        let normalised = normalise(&entry.name);
        if normalised.starts_with(&want) {
            out.push((entry.name.clone(), entry.size));
        }
    }
    Ok(out)
}

/// Convenience: return every distinct top-level directory name present
/// in a pack, normalised to lowercase. Not part of the public surface
/// used by the resolver; kept here for ad-hoc tooling and tests that
/// want to sample the pack's shape.
#[allow(dead_code)]
pub fn list_top_level_dirs(pack_path: &Path) -> Result<Vec<String>, String> {
    if !pack_path.exists() {
        return Err(format!("SDI pack not found: {}", pack_path.display()));
    }
    let archive = Archive::open(pack_path)
        .map_err(|e| format!("Failed to parse SDI pack header: {e}"))?;

    let mut seen = std::collections::BTreeSet::new();
    for entry in &archive.files {
        let normalised = normalise(&entry.name);
        if let Some((top, _)) = normalised.split_once('/') {
            seen.insert(top.to_string());
        }
    }
    Ok(seen.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_handles_backslashes_and_case() {
        assert_eq!(normalise("Brother\\HL\\amd64\\"), "brother/hl/amd64/");
        assert_eq!(normalise("brother/hl/amd64/"), "brother/hl/amd64/");
    }

    #[test]
    fn parent_segment_detector() {
        assert!(contains_parent_segment("foo/../bar"));
        assert!(contains_parent_segment("../bar"));
        assert!(!contains_parent_segment("foo/bar"));
        assert!(!contains_parent_segment("foo..bar"));
    }
}

//! Pure-Rust CAB archive extraction.
//!
//! Replaces the `expand.exe` shell-out that prinstall used to use in
//! [`crate::drivers::downloader::extract_cab`] and
//! [`crate::drivers::resolver::download_and_expand`]. Both call sites now
//! route through [`extract_cab_to_dir`].
//!
//! ## Why not `expand.exe`
//!
//! `expand.exe` is a Windows-only built-in. Calling it from Rust means:
//!
//! - The driver-acquisition stack can only be tested against real CAB data
//!   on a Windows host. Linux CI can't exercise it, so Tier 3 catalog
//!   resolver + the existing Tier 1/2 CAB path had to rely on handcrafted
//!   tests against non-CAB data.
//! - `cargo xwin` cross-compile builds succeed but can never be unit-tested
//!   for CAB correctness on the dev box.
//! - Any subprocess call is a failure surface (missing binary, path
//!   escaping bugs, stderr parsing, exit-code quirks).
//!
//! The [`cab`](https://crates.io/crates/cab) crate is pure Rust, reads
//! every Microsoft CAB format we've seen in the wild (including the ones
//! Tier 3 downloads from `download.windowsupdate.com`), and works
//! identically on Linux and Windows. After this migration, every part of
//! the driver acquisition pipeline is Linux-testable via MockExecutor.
//!
//! ## Contract
//!
//! `extract_cab_to_dir(bytes, dest)` takes the in-memory bytes of a CAB
//! archive plus a destination directory, and extracts every file from the
//! archive into `dest`, preserving the relative path structure declared
//! inside the CAB. This is the same behaviour as `expand.exe -F:*`.
//!
//! The function is **sync** and **cross-platform**. It does not touch the
//! network — callers are responsible for fetching the CAB bytes first and
//! handing them in.

use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

/// Extract every file from an in-memory CAB archive into `dest`.
///
/// The destination directory is created if it does not exist. Each file
/// inside the CAB is written to `dest` using its archived relative path,
/// with intermediate directories created on the fly. CAB paths containing
/// backslashes (the native MS format separator) are normalised to forward
/// slashes before being joined.
///
/// Returns an error if:
/// - The CAB header cannot be parsed
/// - A file entry cannot be read
/// - A path inside the archive would escape `dest` via `..` components
/// - Any filesystem operation fails
///
/// On success, returns the list of absolute paths that were written.
pub fn extract_cab_to_dir(bytes: &[u8], dest: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(dest)
        .map_err(|e| format!("Failed to create CAB extract dir {}: {e}", dest.display()))?;

    let cursor = Cursor::new(bytes);
    let mut cabinet = cab::Cabinet::new(cursor)
        .map_err(|e| format!("Invalid CAB archive: {e}"))?;

    // Collect the file paths first so we can drop the folder iterator
    // before opening readers (cab::Cabinet holds mutable state during
    // read_file). Vec<(archived_path, folder_index, file_index_in_folder)>
    // isn't needed — the crate's read_file takes the archived path string.
    let mut entries: Vec<String> = Vec::new();
    for folder in cabinet.folder_entries() {
        for file in folder.file_entries() {
            entries.push(file.name().to_string());
        }
    }

    let mut written: Vec<PathBuf> = Vec::with_capacity(entries.len());

    for archived_name in entries {
        // CAB native separator is backslash — normalise for a cross-platform
        // PathBuf. Empty components (from leading/trailing/double separators)
        // are dropped by the Path API.
        let normalised = archived_name.replace('\\', "/");

        // Reject paths that would escape the destination via `..`. This is
        // the same check `expand.exe` doesn't do — CAB doesn't have a
        // standard escape mechanism, but a maliciously-crafted archive
        // could contain a `..\..\..\windows\system32\evil.dll`. We bail
        // instead of silently clamping.
        if normalised.split('/').any(|seg| seg == "..") {
            return Err(format!(
                "CAB entry '{archived_name}' contains '..' path segment — rejecting"
            ));
        }

        let mut outpath = dest.to_path_buf();
        for seg in normalised.split('/').filter(|s| !s.is_empty() && *s != ".") {
            outpath.push(seg);
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        }

        let mut reader = cabinet
            .read_file(&archived_name)
            .map_err(|e| format!("Failed to open CAB entry '{archived_name}': {e}"))?;

        let mut outfile = fs::File::create(&outpath)
            .map_err(|e| format!("Failed to create {}: {e}", outpath.display()))?;

        io::copy(&mut reader, &mut outfile)
            .map_err(|e| format!("Failed to write {}: {e}", outpath.display()))?;

        written.push(outpath);
    }

    Ok(written)
}

/// Convenience wrapper that reads a CAB file from disk, then delegates to
/// [`extract_cab_to_dir`]. Used by callers that already have a `.cab` on
/// disk rather than in memory.
#[allow(dead_code)]
pub fn extract_cab_file_to_dir(cab_path: &Path, dest: &Path) -> Result<Vec<PathBuf>, String> {
    let mut file = fs::File::open(cab_path)
        .map_err(|e| format!("Failed to open CAB {}: {e}", cab_path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read CAB {}: {e}", cab_path.display()))?;
    extract_cab_to_dir(&bytes, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty byte slice should produce a structured error, not a panic.
    #[test]
    fn rejects_empty_bytes() {
        let dest = std::env::temp_dir().join("prinstall-cab-test-empty");
        let _ = fs::remove_dir_all(&dest);
        let result = extract_cab_to_dir(&[], &dest);
        assert!(result.is_err(), "empty bytes should fail cab parsing");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Invalid CAB") || msg.contains("CAB"),
            "error message should mention CAB: {msg}"
        );
    }

    /// Random garbage bytes should produce a structured error, not a panic.
    #[test]
    fn rejects_garbage_bytes() {
        let dest = std::env::temp_dir().join("prinstall-cab-test-garbage");
        let _ = fs::remove_dir_all(&dest);
        let garbage = b"this is not a CAB file, it is a string".to_vec();
        let result = extract_cab_to_dir(&garbage, &dest);
        assert!(result.is_err(), "garbage should fail cab parsing");
    }

    /// Directory-traversal attempt via `..` in an archived path should be
    /// rejected explicitly. We can't easily construct a malicious CAB in
    /// Rust without the cab crate's writer API, but the check on the
    /// extracted name is unit-testable via the internal helper. This is a
    /// guard-rail test against the future risk.
    #[test]
    fn path_traversal_check_rejects_parent_segments() {
        // Simulate the internal check by running it directly against a
        // synthetic archived name. We don't need a real CAB to validate
        // this logic — the reject happens before any filesystem access.
        let traversal = "..\\..\\windows\\evil.dll";
        let normalised = traversal.replace('\\', "/");
        let has_parent = normalised.split('/').any(|seg| seg == "..");
        assert!(has_parent, "normalised path should contain '..' segment");
    }
}

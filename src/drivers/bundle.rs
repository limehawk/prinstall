//! Local driver bundle resolver.
//!
//! Scans the bundle directory for INF files, parses them, and matches
//! HWIDs against the detected printer. No network, no downloads —
//! purely local. Positions as a tier between Local store and Manufacturer.
//!
//! Bundle location resolution (see [`crate::paths::bundle_dir_candidates`]):
//!   1. `PRINSTALL_BUNDLE_DIR` env var
//!   2. `drivers/` adjacent to the prinstall executable
//!   3. `<data_dir>/drivers/` fallback
//!
//! Intent: a tech can drop a folder like `hp-laserjet-m404/` containing
//! an extracted vendor driver pack next to the `prinstall.exe` they pushed
//! out via their RMM, and prinstall will prefer it over network sources.

use std::path::PathBuf;

use crate::drivers::inf;

/// A driver candidate found in the local bundle directory.
#[derive(Debug, Clone)]
pub struct BundleCandidate {
    /// Directory the INF lives in (the subfolder inside bundle_dir). This
    /// is what gets passed to the Authenticode verification gate; any `.cat`
    /// files next to the INF need to live here.
    pub pack_dir: PathBuf,
    /// Full path to the matched INF file.
    pub inf_path: PathBuf,
    /// Display name from the INF `[Models]` section (`%FriendlyName%` expanded).
    pub display_name: String,
    /// The specific HWID that matched — logged for audit trails.
    pub matched_hwid: String,
    /// Provider string from INF `[Version]` block.
    pub provider: Option<String>,
    /// DriverVer string from INF `[Version]` block (format `MM/DD/YYYY,X.Y.Z.W`).
    pub driver_ver: Option<String>,
}

/// Scan the bundle directory for INF files that match `device_id`.
///
/// `device_id` may be an IPP 1284 string (`MFG:...;CID:...`) or a USB
/// InstanceId (`USB\VID_xxxx&PID_yyyy\SERIAL`); both formats are handled
/// automatically via [`inf::synthesize_hwids`].
///
/// Iterates through every candidate bundle directory in
/// [`crate::paths::bundle_dir_candidates`]. The first directory that yields
/// at least one match wins — we don't merge results across locations because
/// the env-var override is meant as an authoritative escape hatch.
///
/// Returns an empty vec when:
///   * `device_id` produces no synthesizable HWIDs
///   * no candidate bundle dir exists
///   * no INF in any candidate dir matches
pub fn scan_candidates(device_id: &str, verbose: bool) -> Vec<BundleCandidate> {
    let hwids = inf::synthesize_hwids(device_id);
    if hwids.is_empty() {
        if verbose {
            eprintln!("[bundle] no candidate HWIDs for device_id: {device_id}");
        }
        return Vec::new();
    }
    if verbose {
        eprintln!("[bundle] HWID candidates: {}", hwids.join(", "));
    }

    let mut candidates = Vec::new();
    for bundle_dir in crate::paths::bundle_dir_candidates() {
        if !bundle_dir.exists() {
            if verbose {
                eprintln!("[bundle] skip (missing): {}", bundle_dir.display());
            }
            continue;
        }
        if verbose {
            eprintln!("[bundle] scanning {}", bundle_dir.display());
        }

        let infs = crate::drivers::downloader::find_inf_files(&bundle_dir);
        if infs.is_empty() && verbose {
            eprintln!("[bundle]   no INF files in {}", bundle_dir.display());
        }

        for inf_path in infs {
            let inf_data = match inf::parse_inf(&inf_path) {
                Ok(d) => d,
                Err(e) => {
                    if verbose {
                        eprintln!(
                            "[bundle]   parse error for {}: {e}",
                            inf_path.display()
                        );
                    }
                    continue;
                }
            };
            if let Some(entry) = inf::find_matching(&inf_data, &hwids) {
                let pack_dir = inf_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| bundle_dir.clone());
                if verbose {
                    eprintln!(
                        "[bundle]   ★ match: {} ({}) in {}",
                        entry.display_name,
                        entry.hwid,
                        inf_path.display()
                    );
                }
                candidates.push(BundleCandidate {
                    pack_dir,
                    inf_path: inf_path.clone(),
                    display_name: entry.display_name.clone(),
                    matched_hwid: entry.hwid.clone(),
                    provider: inf_data.provider.clone(),
                    driver_ver: inf_data.driver_ver.clone(),
                });
            }
        }

        // First bundle dir with matches wins — don't descend into fallbacks
        // once we've found something. Keeps behavior predictable when the
        // tech is using PRINSTALL_BUNDLE_DIR as an authoritative override.
        if !candidates.is_empty() {
            break;
        }
    }

    if verbose {
        eprintln!(
            "[bundle] found {} candidate(s) for {device_id}",
            candidates.len()
        );
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    // Share the env-var lock with `paths::tests` — both modules mutate
    // PRINSTALL_BUNDLE_DIR, so a per-module mutex would let them race
    // against each other even with the within-module serialization.
    use crate::paths::BUNDLE_ENV_LOCK as ENV_LOCK;

    fn fixture_inf() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests");
        p.push("fixtures");
        p.push("brother_type1.inf");
        p
    }

    /// Create a fresh temp bundle dir containing a subfolder with the
    /// Brother fixture copied in. Returns the bundle root plus a Drop
    /// guard that removes it on test teardown.
    fn setup_bundle() -> (PathBuf, TempDirGuard) {
        let unique = format!(
            "prinstall-bundle-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let pack = root.join("brother_type1");
        std::fs::create_dir_all(&pack).expect("create pack dir");
        std::fs::copy(fixture_inf(), pack.join("prnbrcl1.inf")).expect("copy fixture");
        (root.clone(), TempDirGuard(root))
    }

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scan_finds_matching_inf_by_ipp_cid() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (bundle_root, _cleanup) = setup_bundle();
        // SAFETY: locked.
        unsafe {
            std::env::set_var("PRINSTALL_BUNDLE_DIR", &bundle_root);
        }

        let device_id = "MFG:Brother;MDL:MFC-L2750DW series;\
                         CLS:PRINTER;CID:Brother Laser Type1";
        let candidates = scan_candidates(device_id, false);

        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }

        assert!(!candidates.is_empty(), "expected at least one match");
        let best = &candidates[0];
        assert_eq!(best.display_name, "Brother Laser Type1 Class Driver");
        assert_eq!(best.matched_hwid, "1284_CID_BROTHER_LASER_TYPE1");
        assert_eq!(best.provider.as_deref(), Some("Brother"));
        // The pack_dir should be the subfolder holding the INF, not the
        // bundle root itself — that's what the verification gate needs.
        assert!(best.pack_dir.ends_with("brother_type1"));
    }

    #[test]
    fn scan_returns_empty_for_no_match() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (bundle_root, _cleanup) = setup_bundle();
        // SAFETY: locked.
        unsafe {
            std::env::set_var("PRINSTALL_BUNDLE_DIR", &bundle_root);
        }

        // A device ID with no matching CID/HWID in the Brother fixture.
        let device_id = "MFG:HP;MDL:Imaginary 9000;CID:HP Laser Type99";
        let candidates = scan_candidates(device_id, false);

        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }

        assert!(candidates.is_empty(), "expected no match for HP against Brother fixture");
    }

    #[test]
    fn scan_returns_empty_when_bundle_dir_missing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Point to a path that definitely doesn't exist. The
        // programdata fallback is also very unlikely to contain a
        // matching INF in a CI environment, so this still asserts
        // the "no match" case end-to-end.
        let missing = std::env::temp_dir().join("prinstall-bundle-definitely-missing-abc123");
        let _ = std::fs::remove_dir_all(&missing);
        // SAFETY: locked.
        unsafe {
            std::env::set_var("PRINSTALL_BUNDLE_DIR", &missing);
        }

        let device_id = "MFG:Brother;CID:Brother Laser Type1";
        let candidates = scan_candidates(device_id, false);

        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }

        assert!(candidates.is_empty());
    }

    #[test]
    fn scan_returns_empty_for_garbage_device_id() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("PRINSTALL_BUNDLE_DIR");
        }
        // Empty / malformed device IDs produce no HWID candidates, so the
        // scan short-circuits before touching the filesystem.
        let candidates = scan_candidates("", false);
        assert!(candidates.is_empty());

        let candidates = scan_candidates("no colons no semicolons", false);
        assert!(candidates.is_empty());
    }
}

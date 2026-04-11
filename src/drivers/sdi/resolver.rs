//! SDI tier orchestrator — enumeration of candidates for a discovered
//! printer.
//!
//! This is the SDI side of the unified driver sources model. Given an
//! IPP device-id, [`enumerate_candidates`] walks every `.bin` index
//! file currently in the SDI cache, parses each via
//! [`super::index::parse_index_file`], searches for matches against
//! HWIDs synthesised from the device-id via
//! [`crate::drivers::inf::synthesize_hwids`], and produces one
//! [`SourceCandidate`] per match.
//!
//! ## Cached vs uncached distinction
//!
//! Every hit inside a parsed index maps to a specific `.7z` pack
//! filename (computed via [`SdiIndex::pack_filename`]). The resolver
//! checks whether that pack is already on disk via
//! [`super::cache::SdiCache::has_pack`]:
//!
//! - **Pack is cached** → emit `Source::SdiCached` with a `pack_path`
//!   install hint that points at the local file. Auto-pick treats this
//!   as effectively free and prefers it over Tier 3 catalog.
//! - **Pack is not cached** → emit `Source::SdiUncached` with a
//!   `pack_name` install hint that carries the size + sha256 from the
//!   index's surrounding metadata. Auto-pick gates this on
//!   `--sdi-fetch` or a prior `prinstall sdi prefetch` so scripted
//!   runs never silently trigger a multi-hundred-MB download.
//!
//! ## Scope boundary
//!
//! This module does NOT download packs, does NOT extract files, and
//! does NOT touch `pnputil`. Those steps belong to the install path
//! that gets wired into `commands/add.rs` in PR 3 via a separate
//! `install_from_candidate` entry point. Keeping enumeration
//! separate lets `prinstall drivers <ip>` show SDI matches cheaply,
//! without committing to any network traffic.
//!
//! ## Error handling
//!
//! Individual index parse failures are logged and skipped — one corrupt
//! `.bin` in the cache shouldn't blind the whole tier. The top-level
//! function always returns a `Vec<SourceCandidate>`; "no SDI matches"
//! is expressed as an empty vec, not an error. Hard failures (e.g.
//! cache dir permission errors) would already have surfaced from
//! [`SdiCache::load`] before we get here.

use crate::drivers::inf;
use crate::drivers::sdi::cache::SdiCache;
use crate::drivers::sdi::index::{self, SdiHit, SdiIndex};
use crate::drivers::sources::{InstallHint, Source, SourceCandidate};

/// Enumerate SDI candidates for a printer's IEEE 1284 device-id.
///
/// Walks every cached `.bin` index, searches each for HWID matches,
/// and returns the merged list of candidates. One `SourceCandidate`
/// per (index × HWID hit). Returns an empty vec if the cache is empty,
/// no indexes match, or the device-id synthesises no HWIDs.
///
/// This function is **pure enumeration** — no network traffic, no disk
/// extraction, no Windows-specific API calls. Safe to call from the
/// `prinstall drivers <ip>` dry-run display.
///
/// ## Parameters
///
/// - `device_id` — the IEEE 1284 device-id string returned by IPP
///   `printer-device-id` or SNMP (e.g., `"MFG:Brother;MDL:...;CID:Brother Laser Type1;..."`).
///   Passed through to [`inf::synthesize_hwids`] to build the HWID
///   candidate set.
/// - `cache` — loaded SDI cache. The function reads
///   `cache.list_cached_indexes()` and `cache.has_pack(pack_name)` but
///   never mutates the cache.
///
/// ## Return
///
/// A `Vec<SourceCandidate>` sorted in the order each index was scanned
/// (indexes are enumerated by the cache in filesystem order, which is
/// typically alphabetical). Within a single index, hits are returned in
/// the order `find_matching` produces them. Auto-pick and the drivers
/// command display are both responsible for their own final ordering —
/// this function just emits the raw hit set.
pub fn enumerate_candidates(device_id: &str, cache: &SdiCache) -> Vec<SourceCandidate> {
    let hwid_candidates = inf::synthesize_hwids(device_id);
    if hwid_candidates.is_empty() {
        return Vec::new();
    }

    let index_paths = cache.list_cached_indexes();
    if index_paths.is_empty() {
        return Vec::new();
    }

    let mut candidates: Vec<SourceCandidate> = Vec::new();

    for index_path in index_paths {
        let parsed = match index::parse_index_file(&index_path) {
            Ok(idx) => idx,
            Err(e) => {
                eprintln!(
                    "warning: SDI index {} is unreadable ({}); skipping",
                    index_path.display(),
                    e
                );
                continue;
            }
        };

        let hits = parsed.find_matching(&hwid_candidates);
        if hits.is_empty() {
            continue;
        }

        let pack_filename = parsed.pack_filename();
        let pack_is_cached = cache.has_pack(&pack_filename);

        for hit in &hits {
            if let Some(candidate) =
                candidate_for_hit(&parsed, &pack_filename, pack_is_cached, hit, cache)
            {
                candidates.push(candidate);
            }
        }
    }

    candidates
}

/// Build a `SourceCandidate` from a single `(index, hit)` pair.
///
/// Distinguishes `SdiCached` from `SdiUncached` based on whether the
/// pack file is currently on disk. For `SdiCached`, the install hint
/// carries the absolute cached pack path plus the INF prefix + filename
/// — everything the extractor needs, no mirror lookup required. For
/// `SdiUncached`, we emit the pack name and populate the cost from the
/// cache's pack metadata if we know it; otherwise we leave cost_bytes
/// as `None` because the size will only become known after the manifest
/// is fetched.
fn candidate_for_hit(
    index: &SdiIndex,
    pack_filename: &str,
    pack_is_cached: bool,
    hit: &SdiHit<'_>,
    cache: &SdiCache,
) -> Option<SourceCandidate> {
    let display_name = hit.driver_display_name.to_string();
    if display_name.trim().is_empty() {
        // Shouldn't happen in well-formed indexes, but an empty driver
        // name produces a useless candidate — skip rather than showing
        // a blank row to the user.
        return None;
    }

    let install_hint = if pack_is_cached {
        let pack_path = match cache.pack_path(pack_filename) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "warning: SDI pack {} rejected by cache path check ({}); treating as uncached",
                    pack_filename, e
                );
                // Fall through as uncached — better to offer the user a
                // "download this" option than silently drop the hit.
                return Some(build_uncached_candidate(
                    pack_filename,
                    hit,
                    display_name,
                    cache,
                    index,
                ));
            }
        };
        InstallHint::SdiCached {
            pack_path,
            inf_dir_prefix: hit.inf_dir_prefix.clone(),
            inf_filename: hit.inf_filename.to_string(),
        }
    } else {
        return Some(build_uncached_candidate(
            pack_filename,
            hit,
            display_name,
            cache,
            index,
        ));
    };

    let (size_bytes, source) = cached_cost_and_tag(cache, pack_filename);

    Some(SourceCandidate {
        source,
        driver_name: display_name,
        driver_version: hit.driver_ver.map(|s| s.to_string()),
        provider: Some(hit.driver_manufacturer.to_string()),
        confidence: 1000, // HWID match is deterministic
        cost_bytes: size_bytes,
        install_hint,
    })
}

/// Helper for the uncached branch — factored out so the
/// cache-path-rejection fallback can reuse it without duplicating the
/// candidate construction.
fn build_uncached_candidate(
    pack_filename: &str,
    hit: &SdiHit<'_>,
    display_name: String,
    cache: &SdiCache,
    _index: &SdiIndex,
) -> SourceCandidate {
    // If we happen to have the pack's size + sha256 already in metadata
    // (e.g., from a previous refresh that registered the pack but
    // whose file got deleted), reuse them. Otherwise we emit the
    // candidate with an empty sha256 and zero size — the fetcher will
    // replace those with real values from the manifest at install time.
    let meta = cache.metadata.packs.get(pack_filename);
    let size = meta.map(|m| m.size_bytes).unwrap_or(0);
    let sha256 = meta.map(|m| m.sha256.clone()).unwrap_or_default();

    SourceCandidate {
        source: Source::SdiUncached,
        driver_name: display_name,
        driver_version: hit.driver_ver.map(|s| s.to_string()),
        provider: Some(hit.driver_manufacturer.to_string()),
        confidence: 1000,
        cost_bytes: if size > 0 { Some(size) } else { None },
        install_hint: InstallHint::SdiUncached {
            pack_name: pack_filename.to_string(),
            pack_size_bytes: size,
            expected_sha256: sha256,
            inf_dir_prefix: hit.inf_dir_prefix.clone(),
            inf_filename: hit.inf_filename.to_string(),
        },
    }
}

/// Look up the cached pack's known byte size in the cache metadata.
/// Returns `(cost_bytes, source_tag)` — for cached packs the tag is
/// always `SdiCached`, and cost is `None` (cached reads are free).
///
/// Factored into its own function so a future refinement (e.g.,
/// surfacing the on-disk size even for cached packs so users can see
/// it in the drivers display) doesn't require plumbing through the
/// `candidate_for_hit` body.
fn cached_cost_and_tag(_cache: &SdiCache, _pack_filename: &str) -> (Option<u64>, Source) {
    (None, Source::SdiCached)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::sdi::cache::SdiCache;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    /// Build a fresh temp-rooted SdiCache for use in tests. Each test
    /// gets its own unique temp dir so parallel runs don't collide.
    fn fresh_cache(test_name: &str) -> (SdiCache, PathBuf) {
        let root = env::temp_dir()
            .join("prinstall-sdi-resolver-tests")
            .join(test_name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cache = SdiCache::load_from_root(root.clone()).unwrap();
        (cache, root)
    }

    /// Path to the real SDW index fixture. Tests that require it
    /// short-circuit gracefully if it's not present (e.g., on CI).
    fn real_index_fixture() -> PathBuf {
        PathBuf::from("/tmp/sdio-check/indexes/DP_Printer_26000.bin")
    }

    #[test]
    fn empty_cache_returns_no_candidates() {
        let (cache, _root) = fresh_cache("empty_cache");
        let device_id = "MFG:Brother;MDL:Foo;CID:Brother Laser Type1;";
        let candidates = enumerate_candidates(device_id, &cache);
        assert!(candidates.is_empty());
    }

    #[test]
    fn empty_device_id_returns_no_candidates() {
        let (cache, root) = fresh_cache("empty_device_id");
        // Even with a populated cache, an empty device_id synthesises
        // zero HWIDs and produces zero candidates.
        let fixture = real_index_fixture();
        if fixture.is_file() {
            let dest = root.join("indexes").join("DP_Printer_26000.bin");
            fs::copy(&fixture, &dest).unwrap();
        }
        let candidates = enumerate_candidates("", &cache);
        assert!(candidates.is_empty());
    }

    #[test]
    fn unmatched_device_id_returns_no_candidates() {
        let (cache, root) = fresh_cache("unmatched_device_id");
        let fixture = real_index_fixture();
        if !fixture.is_file() {
            // Skip — real fixture not available
            return;
        }
        let dest = root.join("indexes").join("DP_Printer_26000.bin");
        fs::copy(&fixture, &dest).unwrap();

        // A device-id that produces no HWIDs matching anything in the
        // real printer pack. We deliberately use a made-up CID so the
        // parser synthesises HWIDs nobody is going to have.
        let device_id = "MFG:NoSuchVendor;MDL:Nope;CID:NoSuchCid;";
        let candidates = enumerate_candidates(device_id, &cache);
        assert!(
            candidates.is_empty(),
            "expected zero candidates for a non-matching device_id, got {}",
            candidates.len()
        );
    }

    #[test]
    fn brother_device_id_against_real_index_returns_uncached_candidate() {
        let (cache, root) = fresh_cache("brother_match_uncached");
        let fixture = real_index_fixture();
        if !fixture.is_file() {
            return; // skip gracefully without the real fixture
        }
        let dest = root.join("indexes").join("DP_Printer_26000.bin");
        fs::copy(&fixture, &dest).unwrap();

        // Real device-id from the dockurr VM test earlier — we know
        // Brother Laser Type1 is a HWID that resolves through the full
        // chain in the real DP_Printer_26000.bin index.
        let device_id = "MFG:Brother;CMD:PJL,PCL,PCLXL,URF;MDL:MFC-L2750DW series;CLS:PRINTER;CID:Brother Laser Type1;";
        let candidates = enumerate_candidates(device_id, &cache);

        assert!(
            !candidates.is_empty(),
            "expected at least one SDI candidate for the Brother device-id"
        );

        // Every candidate should be SdiUncached because the pack isn't
        // on disk in this test.
        for c in &candidates {
            assert_eq!(
                c.source,
                Source::SdiUncached,
                "expected SdiUncached, got {:?}",
                c.source
            );
            match &c.install_hint {
                InstallHint::SdiUncached {
                    pack_name,
                    inf_dir_prefix,
                    inf_filename,
                    ..
                } => {
                    assert_eq!(pack_name, "DP_Printer_26000.7z");
                    assert!(
                        !inf_dir_prefix.is_empty(),
                        "inf_dir_prefix should be populated"
                    );
                    assert!(
                        !inf_filename.is_empty(),
                        "inf_filename should be populated"
                    );
                }
                other => panic!("expected SdiUncached install hint, got {other:?}"),
            }
            assert_eq!(c.confidence, 1000);
        }
    }

    #[test]
    fn brother_device_id_flips_to_cached_when_pack_registered() {
        let (mut cache, root) = fresh_cache("brother_match_cached");
        let fixture = real_index_fixture();
        if !fixture.is_file() {
            return;
        }
        let dest = root.join("indexes").join("DP_Printer_26000.bin");
        fs::copy(&fixture, &dest).unwrap();

        // Stub a fake pack file in the cache so has_pack returns true
        // without us needing to shuttle the real 1.48 GB 7z around.
        // register_pack SHA256's whatever file we hand it, so a small
        // dummy is fine for the cached-vs-uncached distinction.
        let dummy_pack = root.join("drivers").join("DP_Printer_26000.7z");
        fs::write(&dummy_pack, b"dummy pack content for test").unwrap();
        cache.register_pack("DP_Printer_26000.7z", &dummy_pack).unwrap();

        assert!(cache.has_pack("DP_Printer_26000.7z"));

        let device_id = "MFG:Brother;CMD:PJL,PCL,PCLXL,URF;MDL:MFC-L2750DW series;CLS:PRINTER;CID:Brother Laser Type1;";
        let candidates = enumerate_candidates(device_id, &cache);
        assert!(!candidates.is_empty());

        for c in &candidates {
            assert_eq!(
                c.source,
                Source::SdiCached,
                "expected SdiCached now that pack is registered"
            );
            match &c.install_hint {
                InstallHint::SdiCached {
                    pack_path,
                    inf_dir_prefix,
                    inf_filename,
                } => {
                    assert_eq!(pack_path, &dummy_pack);
                    assert!(!inf_dir_prefix.is_empty());
                    assert!(!inf_filename.is_empty());
                }
                other => panic!("expected SdiCached install hint, got {other:?}"),
            }
        }
    }
}

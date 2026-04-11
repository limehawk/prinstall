//! Catalog-based driver resolver.
//!
//! Orchestrates the deterministic Tier 3 driver resolution path described in
//! the README's "How prinstall picks a driver" section:
//!
//! 1. Pull the `CID:` field out of an IPP device ID string.
//! 2. Search the Microsoft Update Catalog for that CID via
//!    [`crate::drivers::catalog::search`] — returns candidates newest-first.
//! 3. For each candidate (up to [`RESOLVER_MAX_CANDIDATES`]):
//!    - Fetch the real CDN URL via [`crate::drivers::catalog::download_urls`]
//!    - Download the `.cab`
//!    - Expand via `expand.exe` (Windows only; the resolver is never called
//!      from a Linux runtime)
//!    - Parse each extracted INF via [`crate::drivers::inf::parse_inf`]
//!    - Synthesize PnP HWIDs from the full device ID and call
//!      [`crate::drivers::inf::find_matching`]
//!    - First match wins
//! 4. Return a [`ResolvedDriver`] with the INF path ready for
//!    [`crate::installer::powershell::stage_driver_inf`], or an error
//!    explaining why we gave up.
//!
//! The resolver is pure orchestration — it owns no HTTP or filesystem
//! primitives of its own, just a small CAB-URL download helper. All parsing
//! and HWID logic lives in `catalog.rs` and `inf.rs` where it can be tested
//! independently.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::drivers::{catalog, downloader, inf};

/// How many catalog rows to download and INF-scan before giving up. The
/// catalog frequently returns ~5 variants of the same package for different
/// Windows architectures / versions — scanning past 5 hits is almost always
/// wasted bandwidth.
const RESOLVER_MAX_CANDIDATES: usize = 5;

/// How long to wait on any single catalog CDN download. CAB files are small
/// (typically 3–20 MB), so a generous per-request timeout is fine.
const CAB_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// Outcome of a successful catalog-based driver resolution.
///
/// Everything the caller needs to stage the driver and run `Add-Printer`:
/// the path to the INF on disk, the human-readable driver name (taken from
/// the matched INF entry's display-name field — that's what `Add-Printer
/// -DriverName` expects), plus breadcrumbs for the audit trail.
#[derive(Debug, Clone)]
pub struct ResolvedDriver {
    /// Absolute path to the INF file that matched. Pass to
    /// [`crate::installer::powershell::stage_driver_inf`].
    pub inf_path: PathBuf,
    /// The display name from the matched `[Models]` entry, e.g.
    /// `"Brother Laser Type1 Class Driver"`. This becomes
    /// `Add-Printer -DriverName`.
    pub display_name: String,
    /// The catalog row title, e.g. `"Brother - Printer - 10.0.17119.1"`.
    /// Kept for the audit trail / history log.
    pub catalog_title: String,
    /// The catalog row's "last updated" date as shown in the catalog.
    pub catalog_date: String,
    /// The INF's `[Version]/DriverVer` field if we parsed it.
    pub driver_ver: Option<String>,
    /// The HWID that actually matched, e.g. `"1284_CID_BROTHER_LASER_TYPE1"`.
    /// Surfaced so the caller can log "here's why we picked this package".
    pub matched_hwid: String,
}

/// Find a vendor-accurate driver for a printer by walking the catalog-based
/// resolution tier.
///
/// `device_id` is the IEEE 1284 IPP device ID string returned by
/// [`crate::discovery::ipp::query_ipp_attributes`] — the `CID:` subfield is
/// required (we search the catalog by it) and at least one of `MDL:` / `CID:`
/// is required to synthesize HWID candidates.
///
/// Returns `Err` with a human-readable reason if the resolver can't find a
/// match; the caller should then fall back to the IPP Class Driver tier.
pub async fn resolve_driver_for_device(
    device_id: &str,
    verbose: bool,
) -> Result<ResolvedDriver, String> {
    // 1. Pull the raw CID from the device ID — this is our catalog query.
    let cid = extract_field(device_id, "CID").ok_or_else(|| {
        "Device ID has no CID: field — cannot resolve via catalog".to_string()
    })?;

    if verbose {
        eprintln!("[resolver] Searching catalog by CID: '{cid}'");
    }

    // 2. Catalog search. Results already sorted date-descending.
    let updates = catalog::search(&cid)
        .await
        .map_err(|e| format!("Catalog search failed: {e}"))?;
    if updates.is_empty() {
        return Err(format!("No catalog results for CID '{cid}'"));
    }
    if verbose {
        eprintln!(
            "[resolver] Catalog returned {} result(s), scanning top {}",
            updates.len(),
            RESOLVER_MAX_CANDIDATES.min(updates.len())
        );
    }

    // 3. Synthesize candidate PnP HWIDs from the full device ID.
    let candidates = inf::synthesize_hwids(device_id);
    if candidates.is_empty() {
        return Err(
            "Could not synthesize any HWID candidates from device ID".to_string(),
        );
    }
    if verbose {
        eprintln!("[resolver] HWID candidates: {}", candidates.join(", "));
    }

    // 4. Walk newest-first, download + scan, first match wins.
    let staging_root = crate::paths::staging_dir().join("catalog");
    std::fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Failed to create staging dir: {e}"))?;

    let mut last_err = String::from("no candidates scanned");

    for (idx, update) in updates.iter().take(RESOLVER_MAX_CANDIDATES).enumerate() {
        if verbose {
            eprintln!(
                "[resolver] #{}: {}  ({})",
                idx + 1,
                update.title,
                update.last_updated,
            );
        }

        // Fetch the CDN URL for this GUID.
        let urls = match catalog::download_urls(&update.guid).await {
            Ok(u) if !u.is_empty() => u,
            Ok(_) => {
                last_err = format!("no download URLs for '{}'", update.title);
                if verbose {
                    eprintln!("[resolver]   skip: {last_err}");
                }
                continue;
            }
            Err(e) => {
                last_err = format!("download URL fetch failed: {e}");
                if verbose {
                    eprintln!("[resolver]   skip: {last_err}");
                }
                continue;
            }
        };

        // Download + expand into a unique subdir per candidate.
        let extract_dir = staging_root.join(format!("{}-{idx}", sanitize(&update.guid)));
        if let Err(e) = download_and_expand(&urls[0], &extract_dir, verbose).await {
            last_err = format!("download/expand failed: {e}");
            if verbose {
                eprintln!("[resolver]   skip: {last_err}");
            }
            continue;
        }

        // Find every INF in the extract tree.
        let infs = downloader::find_inf_files(&extract_dir);
        if infs.is_empty() {
            last_err = format!("no INF files found in '{}'", update.title);
            if verbose {
                eprintln!("[resolver]   skip: {last_err}");
            }
            continue;
        }

        // Parse each INF and try to match HWIDs. First match wins.
        for inf_path in &infs {
            let inf_data = match inf::parse_inf(inf_path) {
                Ok(d) => d,
                Err(e) => {
                    if verbose {
                        eprintln!(
                            "[resolver]   {}: parse error: {e}",
                            inf_path.display()
                        );
                    }
                    continue;
                }
            };
            if let Some(entry) = inf::find_matching(&inf_data, &candidates) {
                if verbose {
                    eprintln!(
                        "[resolver] ★ MATCH: {} → {} ({})",
                        inf_path.display(),
                        entry.display_name,
                        entry.hwid,
                    );
                }
                return Ok(ResolvedDriver {
                    inf_path: inf_path.clone(),
                    display_name: entry.display_name.clone(),
                    catalog_title: update.title.clone(),
                    catalog_date: update.last_updated.clone(),
                    driver_ver: inf_data.driver_ver.clone(),
                    matched_hwid: entry.hwid.clone(),
                });
            }
        }

        last_err = format!(
            "none of {} INF(s) in '{}' matched our HWIDs",
            infs.len(),
            update.title
        );
        if verbose {
            eprintln!("[resolver]   skip: {last_err}");
        }
    }

    Err(format!(
        "Catalog scan exhausted without a match. Last reason: {last_err}"
    ))
}

/// Extract a single field value from an IEEE 1284 device-id string.
///
/// The format is `KEY:value;KEY:value;…`. Keys are compared case-insensitively.
/// Returns `None` if the key is absent or has an empty value.
pub(crate) fn extract_field(device_id: &str, key: &str) -> Option<String> {
    let key_upper = key.to_ascii_uppercase();
    for piece in device_id.split(';') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let Some((k, v)) = piece.split_once(':') else {
            continue;
        };
        if k.trim().to_ascii_uppercase() == key_upper {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Download a CAB URL and expand it into `dest` using `expand.exe`.
///
/// Windows-only at runtime: `expand.exe` doesn't exist on Linux, so this
/// function returns an error when called from a Linux build. All resolver
/// tests stay on Linux-safe surfaces (parsing, string helpers) to keep CI
/// green.
async fn download_and_expand(url: &str, dest: &Path, verbose: bool) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("create dir {}: {e}", dest.display()))?;

    if verbose {
        eprintln!("[resolver]   GET {url}");
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("prinstall/", env!("CARGO_PKG_VERSION")))
        .timeout(CAB_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client init: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} for {url}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("body read failed: {e}"))?;

    let cab_path = dest.join("__catalog.cab");
    std::fs::write(&cab_path, &bytes).map_err(|e| format!("write cab: {e}"))?;

    if verbose {
        eprintln!(
            "[resolver]   expand {} → {}",
            cab_path.display(),
            dest.display()
        );
    }

    let output = std::process::Command::new("expand")
        .args([
            cab_path.to_str().unwrap_or_default(),
            "-F:*",
            dest.to_str().unwrap_or_default(),
        ])
        .output()
        .map_err(|e| format!("expand.exe invoke failed: {e}"))?;

    // Clean up the downloaded cab either way.
    let _ = std::fs::remove_file(&cab_path);

    if !output.status.success() {
        return Err(format!(
            "expand.exe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Sanitize a string for use as a filesystem path component.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// The resolver's networked and filesystem-heavy paths aren't exercised here —
// they're verified end-to-end on a Windows VM. What we can and do test on
// Linux: the pure helpers (field extraction, sanitize) and the sanity of the
// module's surface.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_field_reads_cid_from_real_brother_device_id() {
        let dev = "MFG:Brother;CMD:PJL,PCL,PCLXL,URF;MDL:MFC-L2750DW series;\
                   CLS:PRINTER;CID:Brother Laser Type1;URF:W8,CP1";
        assert_eq!(extract_field(dev, "CID"), Some("Brother Laser Type1".to_string()));
    }

    #[test]
    fn extract_field_reads_mdl_with_internal_spaces() {
        let dev = "MFG:Brother;MDL:MFC-L2750DW series;CID:Brother Laser Type1;";
        assert_eq!(extract_field(dev, "MDL"), Some("MFC-L2750DW series".to_string()));
    }

    #[test]
    fn extract_field_is_case_insensitive_on_key() {
        let dev = "mfg:Brother;cid:Brother Laser Type1";
        assert_eq!(extract_field(dev, "CID"), Some("Brother Laser Type1".to_string()));
        assert_eq!(extract_field(dev, "cid"), Some("Brother Laser Type1".to_string()));
    }

    #[test]
    fn extract_field_returns_none_for_missing_key() {
        let dev = "MFG:Brother;MDL:MFC-L2750DW series";
        assert_eq!(extract_field(dev, "CID"), None);
    }

    #[test]
    fn extract_field_returns_none_for_empty_value() {
        let dev = "MFG:Brother;CID:;MDL:foo";
        assert_eq!(extract_field(dev, "CID"), None);
    }

    #[test]
    fn extract_field_handles_garbage_input() {
        assert_eq!(extract_field("", "CID"), None);
        assert_eq!(extract_field("not-a-device-id", "CID"), None);
        assert_eq!(extract_field(";;;;", "CID"), None);
    }

    #[test]
    fn sanitize_preserves_alnum_and_dash() {
        assert_eq!(sanitize("abc-123"), "abc-123");
        assert_eq!(sanitize("Brother Laser Type1"), "Brother_Laser_Type1");
        assert_eq!(
            sanitize("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        );
        assert_eq!(sanitize("a/b\\c:d*e"), "a_b_c_d_e");
    }
}

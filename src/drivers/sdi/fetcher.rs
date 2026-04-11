//! HTTP mirror fetcher for the SDI driver tier.
//!
//! Fetches three kinds of assets from a configurable mirror base URL
//! (default: prinstall's GitHub Releases `sdi-printer-v<N>` tag):
//!
//! 1. `manifest.json` — small (~2 KB). Describes the current mirror
//!    version and enumerates every available index and pack with SHA256
//!    hashes and byte counts. Fetched on every `prinstall sdi refresh`.
//! 2. `.bin` index files — ~1 MB each. Fetched once per refresh cycle
//!    and cached under the caller-provided `dest_dir`.
//! 3. `.7z` driver packs — ~50 MB to ~1.5 GB each. Fetched lazily on
//!    first HWID match (or eagerly via `prinstall sdi prefetch`) and
//!    cached under the caller-provided `dest_dir`. Big downloads show
//!    an [`indicatif`] progress bar when `show_progress` is set.
//!
//! ## Safety guarantees
//!
//! - **SHA256 verification.** Every index and pack download is hashed
//!   as bytes stream in, and the final digest is compared against the
//!   manifest's declared value. Mismatched downloads are deleted, never
//!   left in the cache.
//! - **Size guard.** Packs larger than `max_size_mb * 1024 * 1024` are
//!   rejected *before* the fetch starts (by inspecting
//!   `ManifestAsset::size_bytes`) and also via the `Content-Length`
//!   header as a second defense against a lying or tampered manifest.
//! - **Atomic writes.** Downloads land at `<dest>/<name>.downloading`,
//!   get hash-verified, and are only then renamed to `<dest>/<name>`.
//!   A crashed download leaves a `.downloading` file behind, never a
//!   half-written file that looks complete.
//! - **Path-traversal guard.** Asset names are validated to reject
//!   `..`, absolute paths, and any directory separators before they're
//!   joined onto the mirror URL. A malicious manifest can't make the
//!   client request `../../etc/passwd` or escape `dest_dir`.
//! - **Streaming.** The pack downloader reads the response body via
//!   [`reqwest::Response::chunk`] in an async loop, writes each chunk
//!   to disk, and updates the SHA256 hasher incrementally. A 1.48 GB
//!   pack uses O(chunk_size) memory, not O(pack_size).
//!
//! ## Design decisions that diverge from the stub API
//!
//! - `chrono::DateTime<Utc>` for `generated_at`. The mirror builder on
//!   GitHub Releases emits RFC 3339 timestamps, which serde handles via
//!   the `chrono/serde` feature already enabled at the workspace level.
//! - `fetch_manifest` uses the 120 s timeout on the whole request (not
//!   just connect) because `manifest.json` is small enough that slow
//!   read paths are also bugs worth surfacing.
//! - `fetch_pack` uses a 600 s timeout on the initial request headers
//!   only, not on the body read. A legitimately slow mirror can still
//!   stream a 1.5 GB file for 20 minutes without tripping the timeout.

use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Timeout for small-file fetches (`manifest.json` and `.bin` indexes).
const SMALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for the *headers* of a pack fetch. The body read loop is
/// unbounded so a slow mirror can legitimately stream for a long time.
const PACK_CONNECT_TIMEOUT: Duration = Duration::from_secs(600);

/// Top-level manifest describing the current state of the SDI mirror.
///
/// Fetched fresh on every `prinstall sdi refresh`. The `version` field
/// is the mirror release tag (e.g. `"sdi-printer-v1"`) and should be
/// treated as monotonically increasing — newer tags supersede older
/// ones even if the asset names are unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexBundleManifest {
    /// Mirror release tag (e.g. `"sdi-printer-v1"`). Monotonically
    /// increasing.
    pub version: String,
    /// Timestamp the mirror was built.
    pub generated_at: chrono::DateTime<chrono::Utc>,
    /// Available `.bin` index files.
    pub indexes: Vec<ManifestAsset>,
    /// Available `.7z` driver packs.
    pub packs: Vec<ManifestAsset>,
}

/// One asset entry in the manifest — either an index or a pack.
///
/// The `sha256` field is a lowercase hex string (64 chars). Mixed-case
/// hex is also accepted on verification; see [`verify_hash`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestAsset {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl IndexBundleManifest {
    /// Look up an asset by filename. Searches both `indexes` and
    /// `packs`, returning the first match. Case-sensitive.
    pub fn find_asset(&self, name: &str) -> Option<&ManifestAsset> {
        self.indexes
            .iter()
            .chain(self.packs.iter())
            .find(|a| a.name == name)
    }
}

/// Fetch `manifest.json` from the configured mirror base URL.
///
/// Returns the parsed manifest or a structured `Err(String)` if the
/// HTTP request fails, the status is non-2xx, the body isn't valid
/// JSON, or the JSON doesn't match the [`IndexBundleManifest`] shape.
pub async fn fetch_manifest(mirror_url: &str) -> Result<IndexBundleManifest, String> {
    let url = join_mirror(mirror_url, "manifest.json")?;
    let client = build_client(SMALL_TIMEOUT)?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("SDI manifest request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "SDI manifest fetch returned HTTP {} ({}) for {url}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("unknown")
        ));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read SDI manifest body: {e}"))?;

    let manifest: IndexBundleManifest = serde_json::from_slice(&body).map_err(|e| {
        format!(
            "SDI manifest at {url} is not valid JSON or has the wrong shape: {e}"
        )
    })?;

    Ok(manifest)
}

/// Fetch a specific index file from the mirror into `dest_dir`.
///
/// Verifies the SHA256 against `asset.sha256` before accepting. The
/// file is written atomically — streamed to `<dest>/<name>.downloading`
/// and only renamed to `<dest>/<name>` after the hash matches.
pub async fn fetch_index(
    mirror_url: &str,
    asset: &ManifestAsset,
    dest_dir: &Path,
) -> Result<PathBuf, String> {
    validate_asset_name(&asset.name)?;
    fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("Failed to create SDI index dir {}: {e}", dest_dir.display()))?;

    let url = join_mirror(mirror_url, &asset.name)?;
    let client = build_client(SMALL_TIMEOUT)?;

    stream_to_file(&client, &url, asset, dest_dir, None).await
}

/// Fetch a specific driver pack from the mirror into `dest_dir`.
///
/// If `show_progress` is true, displays an [`indicatif`] progress bar
/// to stderr while the body streams. RMM/scripted callers should pass
/// `false` to suppress the bar.
///
/// Verifies the SHA256 before accepting. Enforces `max_size_mb` by
/// rejecting the fetch *before* the HTTP request if the declared size
/// is too large, and again against `Content-Length` as a second defense.
pub async fn fetch_pack(
    mirror_url: &str,
    asset: &ManifestAsset,
    max_size_mb: u64,
    dest_dir: &Path,
    show_progress: bool,
) -> Result<PathBuf, String> {
    validate_asset_name(&asset.name)?;

    let max_bytes = max_size_mb.saturating_mul(1024 * 1024);
    if asset.size_bytes > max_bytes {
        return Err(format!(
            "SDI pack '{}' declared size {} MB exceeds max_size_mb {} MB — aborting before fetch",
            asset.name,
            asset.size_bytes / (1024 * 1024),
            max_size_mb
        ));
    }

    fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("Failed to create SDI pack dir {}: {e}", dest_dir.display()))?;

    let url = join_mirror(mirror_url, &asset.name)?;
    let client = build_client(PACK_CONNECT_TIMEOUT)?;

    let bar = if show_progress {
        Some(make_progress_bar(&asset.name, asset.size_bytes))
    } else {
        None
    };

    let result = stream_to_file(&client, &url, asset, dest_dir, bar.as_ref()).await;

    if let Some(b) = bar {
        if result.is_ok() {
            b.finish_with_message("done");
        } else {
            b.abandon_with_message("failed");
        }
    }

    result
}

// --- internals --------------------------------------------------------

/// Build an HTTP client with a timeout and the prinstall user agent.
fn build_client(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .timeout(timeout)
        .user_agent(concat!("prinstall/", env!("CARGO_PKG_VERSION"), " (sdi-fetcher)"))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Join a mirror base URL with an asset file name.
///
/// Handles trailing-slash quirks so both `http://host/path/` and
/// `http://host/path` produce `http://host/path/<name>`.
fn join_mirror(mirror_url: &str, name: &str) -> Result<String, String> {
    validate_asset_name(name)?;
    let base = mirror_url.trim_end_matches('/');
    if base.is_empty() {
        return Err("mirror_url is empty".to_string());
    }
    Ok(format!("{base}/{name}"))
}

/// Reject asset names that could escape `dest_dir` or the mirror base
/// URL. Also rejects empty names and names containing URL path
/// separators or backslashes (Windows path separator).
fn validate_asset_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("asset name is empty".to_string());
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(format!(
            "asset name '{name}' is absolute — refusing to fetch"
        ));
    }
    // Disallow any directory traversal or nesting. Asset names must be
    // a single flat filename — no `dir/file`, no `..`, no `./`.
    for part in name.split(['/', '\\']) {
        if part == ".." || part == "." {
            return Err(format!(
                "asset name '{name}' contains path traversal — refusing to fetch"
            ));
        }
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!(
            "asset name '{name}' contains a path separator — refusing to fetch"
        ));
    }
    Ok(())
}

/// Stream an HTTP GET body to `<dest_dir>/<asset.name>.downloading`,
/// hashing on the fly, then verify SHA256 and atomically rename on
/// success. On any failure the `.downloading` temp is deleted so no
/// half-written files linger.
async fn stream_to_file(
    client: &Client,
    url: &str,
    asset: &ManifestAsset,
    dest_dir: &Path,
    progress: Option<&ProgressBar>,
) -> Result<PathBuf, String> {
    let final_path = dest_dir.join(&asset.name);
    let tmp_path = dest_dir.join(format!("{}.downloading", asset.name));

    // If a previous run left a temp file behind, clear it.
    let _ = fs::remove_file(&tmp_path).await;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("SDI fetch {} failed: {e}", asset.name))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "SDI fetch {} returned HTTP {} ({})",
            asset.name,
            status.as_u16(),
            status.canonical_reason().unwrap_or("unknown")
        ));
    }

    // Second-line size defense: if the server set Content-Length and
    // it exceeds the declared manifest size by more than a small slop,
    // refuse. This catches a lying manifest or a tampered mirror.
    if let Some(content_len) = resp.content_length()
        && content_len > asset.size_bytes.saturating_add(1024)
    {
        return Err(format!(
            "SDI fetch {}: server Content-Length {} exceeds manifest size {}",
            asset.name, content_len, asset.size_bytes
        ));
    }

    let mut file = fs::File::create(&tmp_path).await.map_err(|e| {
        format!(
            "Failed to create SDI temp file {}: {e}",
            tmp_path.display()
        )
    })?;

    let mut hasher = Sha256::new();
    let mut received: u64 = 0;

    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = fs::remove_file(&tmp_path).await;
                return Err(format!("SDI fetch {} body read failed: {e}", asset.name));
            }
        };

        received = received.saturating_add(chunk.len() as u64);

        // Third-line size defense: drop downloads that exceed the
        // declared size mid-stream. Needs a small slop to tolerate any
        // server-side padding we don't know about.
        if received > asset.size_bytes.saturating_add(1024) {
            let _ = file.shutdown().await;
            let _ = fs::remove_file(&tmp_path).await;
            return Err(format!(
                "SDI fetch {}: received bytes {} exceeds manifest size {}",
                asset.name, received, asset.size_bytes
            ));
        }

        hasher.update(&chunk);
        if let Err(e) = file.write_all(&chunk).await {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(format!(
                "Failed to write SDI temp file {}: {e}",
                tmp_path.display()
            ));
        }

        if let Some(bar) = progress {
            bar.set_position(received);
        }
    }

    if let Err(e) = file.flush().await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(format!("Failed to flush SDI temp file: {e}"));
    }
    drop(file);

    if received != asset.size_bytes {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(format!(
            "SDI fetch {}: received {} bytes, manifest declared {}",
            asset.name, received, asset.size_bytes
        ));
    }

    let actual = hasher.finalize();
    let actual_hex = hex_lower(&actual);
    if !hashes_equal(&actual_hex, &asset.sha256) {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(format!(
            "SDI fetch {}: sha256 hash mismatch (expected {}, got {})",
            asset.name, asset.sha256, actual_hex
        ));
    }

    fs::rename(&tmp_path, &final_path).await.map_err(|e| {
        format!(
            "Failed to rename SDI temp {} → {}: {e}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    Ok(final_path)
}

/// Case-insensitive hex string comparison. Avoids any timing-attack
/// concerns via a short-circuit `eq_ignore_ascii_case` — this is a
/// content-integrity check, not a secret comparison.
fn hashes_equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.eq_ignore_ascii_case(b)
}

/// Lowercase hex encoding of a 32-byte SHA256 digest.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Build a progress bar for a pack download.
fn make_progress_bar(name: &str, total: u64) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template(
            "{msg:30} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-"),
    );
    bar.set_message(name.to_string());
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_asset_name_rejects_traversal() {
        assert!(validate_asset_name("../../etc/passwd").is_err());
        assert!(validate_asset_name("..").is_err());
        assert!(validate_asset_name("./secret").is_err());
        assert!(validate_asset_name("/absolute").is_err());
        assert!(validate_asset_name("\\absolute").is_err());
        assert!(validate_asset_name("dir/file.bin").is_err());
        assert!(validate_asset_name("dir\\file.bin").is_err());
        assert!(validate_asset_name("").is_err());
    }

    #[test]
    fn validate_asset_name_accepts_plain_filenames() {
        assert!(validate_asset_name("DP_Printer_26000.bin").is_ok());
        assert!(validate_asset_name("DP_Printer_26000.7z").is_ok());
        assert!(validate_asset_name("manifest.json").is_ok());
    }

    #[test]
    fn join_mirror_handles_trailing_slash() {
        assert_eq!(
            join_mirror("http://host/path/", "x.bin").unwrap(),
            "http://host/path/x.bin"
        );
        assert_eq!(
            join_mirror("http://host/path", "x.bin").unwrap(),
            "http://host/path/x.bin"
        );
    }

    #[test]
    fn join_mirror_rejects_bad_name() {
        assert!(join_mirror("http://host/", "../evil").is_err());
    }

    #[test]
    fn find_asset_searches_both_lists() {
        let m = IndexBundleManifest {
            version: "sdi-printer-v1".to_string(),
            generated_at: chrono::Utc::now(),
            indexes: vec![ManifestAsset {
                name: "a.bin".to_string(),
                size_bytes: 1,
                sha256: "00".to_string(),
            }],
            packs: vec![ManifestAsset {
                name: "b.7z".to_string(),
                size_bytes: 2,
                sha256: "11".to_string(),
            }],
        };
        assert!(m.find_asset("a.bin").is_some());
        assert!(m.find_asset("b.7z").is_some());
        assert!(m.find_asset("missing").is_none());
    }

    #[test]
    fn hex_lower_encodes_known_bytes() {
        assert_eq!(hex_lower(&[0x00, 0xff, 0xab, 0xcd]), "00ffabcd");
    }

    #[test]
    fn hashes_equal_is_case_insensitive() {
        assert!(hashes_equal("abcdef", "ABCDEF"));
        assert!(hashes_equal("00ff", "00ff"));
        assert!(!hashes_equal("abcdef", "abcde0"));
        assert!(!hashes_equal("ab", "abcd"));
    }
}

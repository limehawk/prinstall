//! HTTP client for the SDI mirror.
//!
//! **Status: stub. Implementation lands in PR 2 Agent D.**
//!
//! Fetches `manifest.json`, individual `.bin` index files, and `.7z`
//! driver packs from the configured mirror URL (default: prinstall
//! GitHub Releases `sdi-printer-v<N>` tag). Verifies SHA256 against the
//! manifest before accepting any pack. Shows an `indicatif` progress
//! bar for the big pack downloads. Uses `reqwest` (already in prinstall
//! deps) for the HTTP layer.
//!
//! Expected public surface (to be filled in):
//!
//! ```ignore
//! pub async fn fetch_manifest(mirror_url: &str) -> Result<IndexBundleManifest, String>;
//!
//! pub async fn fetch_index(
//!     mirror_url: &str,
//!     index_name: &str,
//!     dest_dir: &Path,
//! ) -> Result<PathBuf, String>;
//!
//! pub async fn fetch_pack(
//!     mirror_url: &str,
//!     pack_name: &str,
//!     expected_sha256: &str,
//!     max_size_mb: u64,
//!     dest_dir: &Path,
//!     progress: bool,
//! ) -> Result<PathBuf, String>;
//!
//! pub struct IndexBundleManifest {
//!     pub version: String,
//!     pub generated_at: chrono::DateTime<chrono::Utc>,
//!     pub indexes: Vec<ManifestAsset>,
//!     pub packs: Vec<ManifestAsset>,
//! }
//!
//! pub struct ManifestAsset {
//!     pub name: String,
//!     pub size_bytes: u64,
//!     pub sha256: String,
//! }
//! ```

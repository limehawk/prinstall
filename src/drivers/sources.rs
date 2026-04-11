//! Unified driver sources model.
//!
//! **Status: stub. Implementation lands in PR 2 integration phase.**
//!
//! Prinstall's driver acquisition pipeline is organized around a small
//! set of labeled sources. Every driver candidate for a given printer,
//! regardless of where it comes from, is represented as a
//! [`SourceCandidate`] tagged with a [`Source`]. The
//! [`collect_all_candidates`] function fans out to every source in
//! parallel with per-source timeouts and returns a merged list that
//! `prinstall drivers <ip>` displays uniformly and `prinstall add <ip>`
//! feeds into the matrix-derived auto-pick logic.
//!
//! This replaces the older rigid tier fall-through where each tier
//! only ran if the previous one failed. Under the sources model, every
//! source is queried every time and the admin sees the full picture.
//!
//! Expected public surface (to be filled in):
//!
//! ```ignore
//! pub enum Source {
//!     Local,        // Windows driver store (Get-PrinterDriver)
//!     Direct,       // drivers.toml entry with a working HTTPS URL
//!     Catalog,      // Microsoft Update Catalog (drivers/catalog.rs)
//!     SdiCached,    // SDI, pack already on disk
//!     SdiUncached,  // SDI, pack needs download (gated by --sdi-fetch)
//!     Universal,    // drivers.toml fallback (low confidence, no URL)
//!     Ipp,          // Microsoft IPP Class Driver (port 631 reachable)
//! }
//!
//! pub struct SourceCandidate {
//!     pub source: Source,
//!     pub driver_name: String,
//!     pub driver_version: Option<String>,
//!     pub provider: Option<String>,
//!     pub confidence: u16,      // 0-1000, reused matcher score
//!     pub cost_bytes: Option<u64>,
//!     pub install_hint: InstallHint,
//! }
//!
//! pub enum InstallHint {
//!     Local,
//!     Direct { url: String, extract_kind: String },
//!     Catalog { catalog_guid: String, cdn_url: String },
//!     SdiCached { pack_path: PathBuf, inf_dir_prefix: String, inf_filename: String },
//!     SdiUncached { pack_name: String, pack_url: String, pack_size_bytes: u64,
//!                   expected_sha256: String, inf_dir_prefix: String, inf_filename: String },
//!     Universal { driver_name: String },
//!     Ipp,
//! }
//!
//! pub async fn collect_all_candidates(
//!     ip: &str,
//!     device_id: Option<&str>,
//!     config: &AppConfig,
//! ) -> Vec<SourceCandidate>;
//! ```

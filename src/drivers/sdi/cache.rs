//! On-disk cache manager for the SDI tier.
//!
//! **Status: stub. Implementation lands in PR 2 Agent C.**
//!
//! Tracks the state of `C:\ProgramData\prinstall\sdi\`: which index
//! files are present, which driver packs are cached, their sizes and
//! SHA256 checksums, and per-pack `last_used` timestamps for LRU eviction.
//! Persists to `paths::sdi_metadata_path()` as JSON. Loaded on every
//! SDI tier invocation.
//!
//! Expected public surface (to be filled in):
//!
//! ```ignore
//! pub struct SdiCache {
//!     root: PathBuf,
//!     metadata: CacheMetadata,
//! }
//!
//! impl SdiCache {
//!     pub fn load() -> Result<Self, String>;
//!     pub fn has_pack(&self, pack_name: &str) -> bool;
//!     pub fn pack_path(&self, pack_name: &str) -> PathBuf;
//!     pub fn record_pack_used(&mut self, pack_name: &str) -> Result<(), String>;
//!     pub fn prune(&mut self, budget_mb: u64) -> Result<Vec<String>, String>;
//!     pub fn save_metadata(&self) -> Result<(), String>;
//!     pub fn list_cached_indexes(&self) -> Vec<PathBuf>;
//! }
//!
//! pub struct CacheMetadata {
//!     pub index_version: Option<String>,
//!     pub last_refresh: Option<chrono::DateTime<chrono::Utc>>,
//!     pub packs: HashMap<String, PackMeta>,
//! }
//!
//! pub struct PackMeta {
//!     pub size_bytes: u64,
//!     pub sha256: String,
//!     pub last_used: chrono::DateTime<chrono::Utc>,
//! }
//! ```

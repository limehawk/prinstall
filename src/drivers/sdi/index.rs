//! Pure-Rust parser for SDIO's SDW binary index format.
//!
//! **Status: stub. Implementation lands in PR 2 Agent A.**
//!
//! Clean-room port of the format from published `indexing.h` struct
//! definitions (facts, not source code). Decompresses LZMA-wrapped
//! payloads via `lzma-rs`. Produces a fast HWID → driver-inside-pack
//! lookup table that the SDI resolver uses to decide which pack
//! contains a match for a given printer's device-id.
//!
//! Real-world validation fixture: `/tmp/sdio-check/indexes/DP_Printer_26000.bin`
//! (1.13 MB, SDW magic + version 0x205 + LZMA payload).
//!
//! Expected public surface (to be filled in):
//!
//! ```ignore
//! pub struct SdiIndex { /* parsed SDW contents */ }
//!
//! pub fn parse_index(bytes: &[u8]) -> Result<SdiIndex, String>;
//! pub fn parse_index_file(path: &Path) -> Result<SdiIndex, String>;
//!
//! impl SdiIndex {
//!     pub fn pack_name(&self) -> &str;
//!     pub fn find_matching(&self, hwid_candidates: &[String]) -> Vec<SdiHit<'_>>;
//! }
//!
//! pub struct SdiHit<'a> {
//!     pub hwid: &'a str,
//!     pub inf_dir_prefix: String,
//!     pub inf_filename: &'a str,
//!     pub driver_display_name: &'a str,
//!     pub driver_ver: Option<&'a str>,
//! }
//! ```

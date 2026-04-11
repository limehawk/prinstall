//! SDI tier orchestrator.
//!
//! **Status: stub. Implementation lands in PR 2 integration phase
//! (after Agents A-D return with their modules).**
//!
//! Entry point for the SDI driver tier. Given an IPP device-id, this
//! module:
//!
//! 1. Loads the [`SdiCache`](super::cache::SdiCache) and checks which
//!    `.bin` indexes are already downloaded
//! 2. If the indexes are missing, ensures they're fetched from the
//!    mirror via [`super::fetcher::fetch_index`] (small, ~1 MB total,
//!    always fetched eagerly on first run)
//! 3. Parses each index via [`super::index::parse_index_file`] and
//!    searches for matches against `inf::synthesize_hwids(device_id)`
//! 4. Produces zero or more
//!    [`SourceCandidate`](crate::drivers::sources::SourceCandidate)s:
//!    one `SdiCached` candidate per hit when the pack is on disk, one
//!    `SdiUncached` candidate per hit when the pack still needs to be
//!    fetched
//! 5. For actual install (called after auto-pick selects a SDI
//!    candidate), fetches the pack if needed via
//!    [`super::fetcher::fetch_pack`], extracts the driver directory via
//!    [`super::pack::extract_driver_directory`], stages via
//!    `installer::powershell::stage_driver_inf`, retries `install_printer`
//!
//! Expected public surface (to be filled in):
//!
//! ```ignore
//! /// Enumerate SDI candidates for the given printer device-id. Never
//! /// downloads packs — returns SdiCached or SdiUncached based on
//! /// whether the matched pack is already on disk.
//! pub async fn enumerate_candidates(
//!     device_id: &str,
//!     config: &SdiConfig,
//! ) -> Result<Vec<SourceCandidate>, String>;
//!
//! /// Install a driver from a previously-enumerated SDI candidate.
//! /// Fetches the pack on demand if it's an SdiUncached hint.
//! pub async fn install_from_candidate(
//!     candidate: &SourceCandidate,
//!     config: &SdiConfig,
//!     verbose: bool,
//! ) -> Result<SdiResolvedDriver, String>;
//!
//! pub struct SdiResolvedDriver {
//!     pub inf_path: PathBuf,
//!     pub display_name: String,
//!     pub pack_name: String,
//!     pub matched_hwid: String,
//! }
//! ```

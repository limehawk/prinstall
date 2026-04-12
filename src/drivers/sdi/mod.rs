//! Snappy Driver Installer Origin (SDIO) integration.
//!
//! Prinstall's Tier 2.5 driver source. Reads SDIO's published `.bin`
//! indexes and driver pack `.7z` archives to find vendor drivers that
//! the Microsoft Update Catalog (Tier 3) doesn't reliably carry —
//! Brother, Canon, Epson, Ricoh, and others.
//!
//! ## Module layout
//!
//! - [`index`] — pure-Rust parser for SDIO's SDW binary index format.
//!   Clean-room port of the format from published struct definitions.
//!   No SDIO GPL code is compiled in.
//! - [`pack`] — wrapper over `sevenz-rust2` that extracts a single
//!   driver's directory subtree from a cached `.7z` pack via a
//!   filename-prefix filter.
//! - [`cache`] — on-disk state manager for the SDI cache directory
//!   (indexes, drivers, metadata.json) rooted at [`crate::paths::sdi_dir`].
//! - [`fetcher`] — HTTP client for the mirror. Fetches `manifest.json`,
//!   individual index files, and driver packs with SHA256 verification
//!   and progress bars.
//! - [`resolver`] — orchestrator. Given an IPP device-id, returns zero
//!   or more [`SourceCandidate`](crate::drivers::sources::SourceCandidate)s
//!   from the SDI tier by scanning cached indexes, fetching packs on
//!   demand (gated by config), extracting the matched driver's
//!   subdirectory, and handing off to `stage_driver_inf`.
//!
//! ## Design notes
//!
//! The SDI tier is one of several sources in the unified driver
//! selection pipeline. See [`crate::drivers::sources`] for the
//! [`SourceCandidate`](crate::drivers::sources::SourceCandidate) type
//! it produces and the [`collect_all_candidates`](crate::drivers::sources::collect_all_candidates)
//! fan-out that queries it alongside LOCAL / DIRECT / CATALOG / IPP.

pub mod cache;
pub mod fetcher;
pub mod index;
pub mod pack;
pub mod resolver;

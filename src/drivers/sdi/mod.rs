//! Snappy Driver Installer Origin (SDIO) integration.
//!
//! Reads SDIO `.bin` indexes and `.7z` packs to find vendor drivers the
//! Microsoft Update Catalog does not reliably carry.

pub mod cache;
pub mod fetcher;
pub mod index;
pub mod pack;
pub mod resolver;

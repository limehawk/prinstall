//! Unified driver sources model.
//!
//! Prinstall's driver acquisition pipeline is organized around a small
//! set of labeled **sources**, each of which can produce zero or more
//! [`SourceCandidate`]s for a given printer. Under the sources model,
//! `prinstall drivers <ip>` enumerates every candidate from every
//! source in parallel and shows them in a single numbered list, and
//! `prinstall add <ip>` feeds that same list into the matrix-derived
//! auto-pick logic described in the SDIO integration plan.
//!
//! This module defines the uniform `Source` / `SourceCandidate` /
//! `InstallHint` types that every source adapter produces. The
//! per-source adapters themselves — local store scan, drivers.toml
//! match, Microsoft Update Catalog resolver, SDI resolver, IPP probe —
//! live alongside their existing implementations and are wired together
//! by the `collect_all_candidates` fan-out that ships in PR 3.
//!
//! ## Why a uniform type
//!
//! Before this change, each tier of the driver acquisition pipeline had
//! its own concrete result type (`DriverMatch`, `ResolvedDriver`, etc.)
//! and lived inside its own tier's conditional branch in `commands/add.rs`.
//! Adding a new tier meant editing `add.rs` and `drivers.rs` in multiple
//! places. The tiers also couldn't show up side-by-side in `prinstall
//! drivers` output because they spoke different shapes.
//!
//! With `SourceCandidate`, every source speaks the same language:
//! "here's a driver I could install, here's its cost and confidence,
//! here's the opaque payload you hand back to me if you want to install
//! it." The consumer (`add` or `drivers`) doesn't need to know which
//! source produced a given candidate — it just reads the `Source` tag
//! for display and the `install_hint` for action.

use std::path::PathBuf;

/// Labeled origin of a driver candidate.
///
/// Every candidate is tagged with exactly one source. The tag drives
/// the label shown in `prinstall drivers <ip>` output and the priority
/// ordering in `auto_pick`.
///
/// See the SDIO integration plan for the full priority matrix and the
/// auto-pick logic. Briefly: `Local` wins over everything, then
/// `Direct` (vendor URL) → `Catalog` (Microsoft Update Catalog) →
/// `SdiCached` (SDI pack already on disk) → `SdiUncached` (SDI pack
/// needs fetching — gated on explicit `--sdi-fetch`) → `Ipp` fallback
/// → `Universal` last-resort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Pre-installed in the Windows driver store. Zero cost, zero
    /// latency, already known-good on this machine. Discovered via
    /// `Get-PrinterDriver`.
    Local,
    /// Vendor universal driver with a stable direct HTTPS URL in
    /// `drivers.toml`. Cheap download (10–50 MB), vendor-official,
    /// typically the freshest option.
    Direct,
    /// Microsoft Update Catalog entry discovered by the existing Tier 3
    /// catalog HTTP scraper (`drivers/catalog.rs` + `drivers/resolver.rs`).
    /// Mid-size CAB download (5–25 MB), Microsoft-validated.
    Catalog,
    /// SDI (Snappy Driver Installer Origin) driver in a pack that is
    /// **already cached** in `C:\ProgramData\prinstall\sdi\drivers\`.
    /// Effectively free — no network, just local extraction.
    SdiCached,
    /// SDI driver in a pack that is **not yet cached** and would
    /// require a multi-hundred-MB download on first use. Auto-pick
    /// treats this as a gated candidate — `prinstall add` only uses it
    /// when `--sdi-fetch` is set or when `prinstall sdi prefetch` has
    /// been run beforehand.
    SdiUncached,
    /// Low-confidence fallback entry from `drivers.toml` without a URL
    /// (e.g., a manufacturer-scoped entry matching just by prefix).
    /// Used only when no better source produced a candidate.
    Universal,
    /// Microsoft's built-in IPP Class Driver. Works for any printer
    /// that speaks IPP Everywhere on TCP port 631 but gives up
    /// vendor-specific features. Always the last-resort option before
    /// error.
    Ipp,
}

impl Source {
    /// Short uppercase label shown in `prinstall drivers <ip>` output.
    /// Used by the unified enumeration display in PR 3. Exposed here so
    /// every consumer prints the same strings.
    pub fn label(&self) -> &'static str {
        match self {
            Source::Local => "LOCAL",
            Source::Direct => "DIRECT",
            Source::Catalog => "CATALOG",
            Source::SdiCached => "SDI-CACHED",
            Source::SdiUncached => "SDI",
            Source::Universal => "UNIVERSAL",
            Source::Ipp => "IPP",
        }
    }

    /// Stable snake-case identifier for this source, suitable for
    /// logging, JSON output, and install-history action strings. Maps
    /// 1:1 to the enum variants.
    pub fn history_action(&self) -> &'static str {
        match self {
            Source::Local => "install_local",
            Source::Direct => "install_direct",
            Source::Catalog => "install_catalog",
            Source::SdiCached => "install_sdi_cached",
            Source::SdiUncached => "install_sdi",
            Source::Universal => "install_universal",
            Source::Ipp => "install_ipp",
        }
    }
}

/// A single driver candidate for a discovered printer.
///
/// Produced by a source adapter, consumed by `collect_all_candidates`
/// (in PR 3) and the `drivers` display. The `install_hint` is an opaque
/// payload the matching install adapter uses to actually stage and
/// install the driver — the consumer never inspects its contents.
#[derive(Debug, Clone)]
pub struct SourceCandidate {
    /// Which tier produced this candidate.
    pub source: Source,
    /// Human-readable driver name, as it will appear in the Windows
    /// driver store after install. Used for the display column and as
    /// the `-DriverName` argument to `Add-Printer`.
    pub driver_name: String,
    /// Driver version parsed from the INF's `[Version] DriverVer` field
    /// where available. None for sources that don't surface a version
    /// (LOCAL via `Get-PrinterDriver` on some Windows versions, IPP).
    pub driver_version: Option<String>,
    /// Driver provider (e.g., "Brother", "Hewlett-Packard") parsed from
    /// the INF's `[Version] Provider`. None when the source doesn't
    /// expose it.
    pub provider: Option<String>,
    /// Matcher confidence score on the 0–1000 scale. For LOCAL and
    /// UNIVERSAL sources this comes from `drivers/matcher.rs`. For
    /// CATALOG, SDI, and DIRECT sources — where a hit either matches
    /// an exact HWID or doesn't — confidence is a coarse constant
    /// (1000 for HWID-matched, 500 for prefix-matched, etc.).
    pub confidence: u16,
    /// Download cost in bytes. None for free sources (LOCAL, IPP,
    /// Universal). Used by the display to annotate "cached" vs
    /// "needs download" and by auto-pick to rank cost-sensitive tiers.
    pub cost_bytes: Option<u64>,
    /// Opaque payload used by the installer to actually install this
    /// candidate. The consumer never inspects this directly; it's
    /// passed straight back to the install adapter.
    pub install_hint: InstallHint,
}

/// Opaque per-source install payload.
///
/// The consumer of `SourceCandidate` passes this back to the install
/// adapter unchanged. Each variant carries exactly what that source's
/// install path needs to stage the driver.
///
/// Adding a new source means adding a variant here and a handler in
/// the install dispatch function (in PR 3). The `SourceCandidate`
/// surface doesn't need to know the details.
#[derive(Debug, Clone)]
pub enum InstallHint {
    /// Install from the local driver store via `Add-PrinterDriver`
    /// referencing the store name directly.
    Local {
        /// Exact name as it appears in `Get-PrinterDriver` output.
        driver_store_name: String,
    },
    /// Install via the Tier 1/2 downloader — fetch the URL, extract,
    /// stage the INF with `pnputil /add-driver`, then `Add-Printer`.
    Direct {
        /// Stable HTTPS download URL from `drivers.toml`.
        url: String,
        /// Archive format (`zip`, `cab`).
        format: String,
    },
    /// Install via the existing Tier 3 Microsoft Update Catalog
    /// resolver. The payload is the full `device_id` string because
    /// the existing resolver takes it and re-runs the whole catalog
    /// search internally — we don't store intermediate state.
    Catalog {
        /// IEEE 1284 device-id, passed through to
        /// `drivers::resolver::resolve_driver_for_device`.
        device_id: String,
    },
    /// Install from an SDI pack that is **already cached** on disk.
    /// The pack path + INF subdirectory prefix + INF filename together
    /// are enough to drive `sdi::pack::extract_driver_directory`
    /// without any network traffic.
    SdiCached {
        /// Absolute path to the cached `.7z` pack.
        pack_path: PathBuf,
        /// Directory prefix inside the pack (e.g.,
        /// `brother/hl_l8260cdw/amd64/`) ending in `/`.
        inf_dir_prefix: String,
        /// INF filename inside the prefix (e.g., `brother.inf`).
        inf_filename: String,
    },
    /// Install from an SDI pack that needs to be **downloaded first**
    /// before extraction. Carries everything the fetcher needs
    /// (pack name, mirror URL, size, expected SHA256) plus the
    /// extraction target.
    SdiUncached {
        /// Pack filename in the mirror (e.g., `DP_Printer_26000.7z`).
        pack_name: String,
        /// Declared byte size from the manifest.
        pack_size_bytes: u64,
        /// Expected SHA256 from the manifest (lowercase hex).
        expected_sha256: String,
        /// Directory prefix inside the pack.
        inf_dir_prefix: String,
        /// INF filename inside the prefix.
        inf_filename: String,
    },
    /// Install via `Add-PrinterDriver -Name` with a universal
    /// fallback driver name. No download, but no HWID verification
    /// either — lowest confidence.
    Universal {
        /// Universal driver name as it would appear in
        /// `Get-PrinterDriver` after the `Add-PrinterDriver` call.
        driver_name: String,
    },
    /// Install via `Add-Printer -DriverName "Microsoft IPP Class
    /// Driver"`. No payload needed — the driver name is fixed.
    Ipp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_labels_are_distinct() {
        let labels = [
            Source::Local.label(),
            Source::Direct.label(),
            Source::Catalog.label(),
            Source::SdiCached.label(),
            Source::SdiUncached.label(),
            Source::Universal.label(),
            Source::Ipp.label(),
        ];
        let mut sorted: Vec<_> = labels.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            labels.len(),
            "every Source variant should have a unique label"
        );
    }

    #[test]
    fn history_actions_use_install_prefix() {
        for src in [
            Source::Local,
            Source::Direct,
            Source::Catalog,
            Source::SdiCached,
            Source::SdiUncached,
            Source::Universal,
            Source::Ipp,
        ] {
            assert!(
                src.history_action().starts_with("install_"),
                "history action for {src:?} should be snake_case with install_ prefix"
            );
        }
    }

    #[test]
    fn install_hint_sdi_cached_round_trips_debug() {
        // Round-trip Debug/Clone — these derivations exist for snapshot
        // tests and should always succeed for every variant.
        let hint = InstallHint::SdiCached {
            pack_path: PathBuf::from("/tmp/fake.7z"),
            inf_dir_prefix: "brother/mock/amd64/".to_string(),
            inf_filename: "brother.inf".to_string(),
        };
        let dbg = format!("{hint:?}");
        assert!(dbg.contains("brother/mock/amd64/"));
        assert!(dbg.contains("brother.inf"));
        let _cloned = hint.clone();
    }

    #[test]
    fn source_candidate_cost_bytes_is_optional() {
        let local = SourceCandidate {
            source: Source::Local,
            driver_name: "Brother Laser Type1 Class Driver".to_string(),
            driver_version: None,
            provider: Some("Brother".to_string()),
            confidence: 850,
            cost_bytes: None,
            install_hint: InstallHint::Local {
                driver_store_name: "Brother Laser Type1 Class Driver".to_string(),
            },
        };
        assert!(local.cost_bytes.is_none());

        let sdi_uncached = SourceCandidate {
            source: Source::SdiUncached,
            driver_name: "Brother Universal Printer".to_string(),
            driver_version: Some("4.2".to_string()),
            provider: Some("Brother".to_string()),
            confidence: 1000,
            cost_bytes: Some(1_556_246_616),
            install_hint: InstallHint::SdiUncached {
                pack_name: "DP_Printer_26000.7z".to_string(),
                pack_size_bytes: 1_556_246_616,
                expected_sha256: "deadbeef".repeat(8),
                inf_dir_prefix: "brother/hl_l8260cdw/amd64/".to_string(),
                inf_filename: "brother.inf".to_string(),
            },
        };
        assert_eq!(sdi_uncached.cost_bytes, Some(1_556_246_616));
    }
}

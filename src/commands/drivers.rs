//! The `drivers` command — show all driver options for a printer.
//!
//! The command surfaces these data sources in one report:
//!
//! 1. **Matched drivers** — scored fuzzy matches from the local driver store
//!    and the curated `known_matches.toml` database.
//! 2. **Universal drivers** — manufacturer-level fallbacks from `drivers.toml`.
//! 3. **Microsoft Update Catalog** — scraped from
//!    `catalog.update.microsoft.com`, downloaded as `.cab` and matched by
//!    CID HWID. This is the authoritative Windows-side source.
//! 4. **SDI driver packs** (when compiled with `--features sdi`) — community
//!    driver-pack candidates indexed by IEEE 1284 device ID, with per-pack
//!    Authenticode verification status surfaced inline.

use crate::core::executor::PsExecutor;
use crate::models::{CatalogEntry, CatalogSearchResult, DriverResults};
use crate::{discovery, drivers as drivers_mod};

/// Maximum number of catalog rows to keep. The catalog can return hundreds
/// of rows for a broad manufacturer query — capping to the top 20 keeps the
/// CLI report readable and the JSON output useful for scripting.
const CATALOG_MAX_ROWS: usize = 20;

/// Arguments for `prinstall drivers <ip>`.
pub struct DriversArgs<'a> {
    pub ip: &'a str,
    pub model_override: Option<&'a str>,
    pub community: &'a str,
    pub verbose: bool,
}

/// Run the `drivers` command end-to-end.
///
/// Resolves the model (via `--model` or SNMP), runs the fuzzy matcher,
/// queries IPP for the device ID, and searches the Microsoft Update Catalog.
/// Gracefully degrades when any of those steps fail.
pub async fn run(executor: &dyn PsExecutor, args: DriversArgs<'_>) -> DriverResults {
    let verbose = args.verbose;

    // ── Step 1: resolve the model ────────────────────────────────────────────
    let model = if let Some(m) = args.model_override {
        m.to_string()
    } else {
        resolve_model_via_snmp(args.ip, args.community, verbose).await
    };

    // ── Step 2: local-store match (existing scoring pipeline) ────────────────
    // Pull driver names *with* their DriverDate in one PS shot, then feed the
    // names through the existing scorer and post-enrich matching rows with
    // their dates. Keeping the matcher signature intact avoids rippling
    // through `commands/add.rs`, which is a requirement for this task — the
    // enrichment happens after the match.
    let local_with_dates = drivers_mod::local_store::list_drivers_with_dates(verbose);
    let local_drivers: Vec<String> =
        local_with_dates.iter().map(|(n, _)| n.clone()).collect();
    let mut results = drivers_mod::matcher::match_drivers(&model, &local_drivers);
    let date_map: std::collections::HashMap<String, Option<String>> = local_with_dates
        .into_iter()
        .map(|(n, d)| (n, d.and_then(|s| crate::output::normalize_date(&s))))
        .collect();
    drivers_mod::matcher::enrich_with_dates(&mut results, &date_map);

    // ── Step 2b: manufacturer-URL HEAD probe for publication dates ───────────
    // HP/Xerox/Kyocera universal-driver URLs from drivers.toml almost
    // always return a `Last-Modified` header on HEAD. Populating
    // `driver_date` for those rows closes the last date-source gap so
    // the combined-score ranker no longer falls back to the midpoint
    // score for manufacturer-tier matches. Graceful failure — any error
    // leaves the date as None (existing behavior).
    drivers_mod::url_date::enrich_manufacturer_dates(&mut results, verbose).await;

    // ── Step 3: IPP device ID for pre-flight visibility ──────────────────────
    if let Ok(ipv4) = args.ip.parse::<std::net::Ipv4Addr>() {
        let attrs = discovery::ipp::query_ipp_attributes(ipv4, verbose).await;
        results.device_id = attrs.device_id;
    }

    // ── Step 4: Microsoft Update Catalog search (no admin needed) ────────────
    // Prefer the IPP device ID's CID field as the search query when we have
    // one — it narrows the result set to the exact driver family (e.g.
    // "Brother Laser Type1" returns 5 targeted packages instead of 25 generic
    // Brother rows from a model-name search). Falls back to the model string
    // when no CID is available. Same strategy the `add` command's catalog
    // resolver uses, so this preview matches what `add` would actually pick.
    let (query, query_source) = pick_catalog_query(&model, results.device_id.as_deref());
    if !query.is_empty() {
        results.catalog = Some(search_catalog(&query, query_source, verbose).await);
    }

    // ── Step 5: SDI candidates (sdi feature only) ────────────────────────────
    // Enumerate every cached SDI pack that claims a driver for this HWID, then
    // run Authenticode verification live on each extracted pack directory so
    // the display can show "verified" / "unsigned (N/M)" / etc. per candidate.
    // Uncached packs (SdiUncached) aren't verifiable without a fetch, so they
    // land as "not-extracted" — a cheap signal that a `--sdi-fetch` install
    // would incur a download before a verify gate could even run.
    #[cfg(not(feature = "sdi"))]
    let _ = executor;
    #[cfg(feature = "sdi")]
    {
        use crate::commands::sdi_verify::{PackVerifyOutcome, verify_pack_directory};
        use crate::drivers::sdi::cache::SdiCache;
        use crate::drivers::sdi::resolver::enumerate_candidates;
        use crate::drivers::sources::{InstallHint, Source};
        use crate::models::SdiDriverCandidate;

        if let Some(ref dev_id) = results.device_id
            && let Ok(mut cache) = SdiCache::load()
        {
            let _ = cache.auto_register_packs();
            let sdi_candidates = enumerate_candidates(dev_id, &cache);

            let mapped: Vec<SdiDriverCandidate> = sdi_candidates
                .into_iter()
                .map(|c| {
                    // pack_name = the .7z stem. For SdiCached we derive it
                    // from the pack_path file_stem (mirrors the same logic
                    // used by commands/add.rs::extract_sdi_driver). For
                    // SdiUncached we already have the pack_name field on
                    // the hint; trim the .7z suffix so both sources report
                    // the same stem format.
                    let pack_name = match &c.install_hint {
                        InstallHint::SdiCached { pack_path, .. } => pack_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        InstallHint::SdiUncached { pack_name, .. } => pack_name
                            .strip_suffix(".7z")
                            .unwrap_or(pack_name.as_str())
                            .to_string(),
                        _ => "unknown".to_string(),
                    };
                    let hwid_match = dev_id.clone();

                    let (verification, signer) = if c.source == Source::SdiCached {
                        let extract_dir =
                            crate::paths::sdi_dir().join("extracted").join(&pack_name);
                        if extract_dir.exists() {
                            match verify_pack_directory(executor, &extract_dir, verbose) {
                                PackVerifyOutcome::Verified { signers, .. } => (
                                    "verified".to_string(),
                                    signers.first().cloned(),
                                ),
                                PackVerifyOutcome::Unsigned { unsigned, total } => (
                                    format!("unsigned ({unsigned}/{total})"),
                                    None,
                                ),
                                PackVerifyOutcome::Invalid { first_reason, .. } => (
                                    format!("invalid: {first_reason}"),
                                    None,
                                ),
                                PackVerifyOutcome::NoCatalogs => {
                                    ("no-catalogs".to_string(), None)
                                }
                            }
                        } else {
                            ("not-extracted".to_string(), None)
                        }
                    } else {
                        // SdiUncached — pack hasn't been fetched, can't verify here.
                        ("not-extracted".to_string(), None)
                    };

                    // Parse the date out of the INF's DriverVer string
                    // (format `MM/DD/YYYY,version`). SDI indexes store that
                    // whole string as `driver_version`; the leading date is
                    // what we want for ranking.
                    let driver_date = c
                        .driver_version
                        .as_deref()
                        .and_then(crate::output::normalize_date);

                    SdiDriverCandidate {
                        driver_name: c.driver_name.clone(),
                        pack_name,
                        hwid_match,
                        verification,
                        signer,
                        driver_date,
                    }
                })
                .collect();

            results.sdi_candidates = mapped;
        }
    }

    results
}

/// Decide what string to feed the catalog search.
///
/// Returns `(query, source_label)`. `source_label` is a human-readable tag
/// that gets printed in verbose mode so the user knows why we picked this
/// particular query.
fn pick_catalog_query<'a>(model: &'a str, device_id: Option<&str>) -> (String, &'static str) {
    if let Some(dev) = device_id
        && let Some(cid) = drivers_mod::resolver::extract_field(dev, "CID")
    {
        return (cid, "CID");
    }
    (model.to_string(), "model")
}

/// Search the Microsoft Update Catalog for a printer. Always returns
/// a [`CatalogSearchResult`] so the output formatter can render either the
/// hits, the empty-result case, or the error reason uniformly.
async fn search_catalog(
    query: &str,
    query_source: &str,
    verbose: bool,
) -> CatalogSearchResult {
    if verbose {
        eprintln!(
            "[drivers] Searching Microsoft Update Catalog by {query_source}: '{query}'..."
        );
    }
    match drivers_mod::catalog::search(query).await {
        Ok(mut updates) => {
            let total = updates.len();
            updates.truncate(CATALOG_MAX_ROWS);
            if verbose {
                if total > CATALOG_MAX_ROWS {
                    eprintln!(
                        "[drivers] Catalog returned {total} rows, showing top {CATALOG_MAX_ROWS}"
                    );
                } else {
                    eprintln!("[drivers] Catalog returned {total} rows");
                }
            }
            CatalogSearchResult {
                query: query.to_string(),
                updates: updates.into_iter().map(CatalogEntry::from).collect(),
                error: None,
            }
        }
        Err(e) => {
            if verbose {
                eprintln!("[drivers] Catalog search failed: {e}");
            }
            CatalogSearchResult::failure(query, e)
        }
    }
}

/// Resolve a printer model via SNMP. Returns a placeholder on failure so
/// the matcher can still run against the empty string (producing no matches)
/// and the user sees a clean report rather than an exit code.
async fn resolve_model_via_snmp(ip: &str, community: &str, verbose: bool) -> String {
    let addr: std::net::Ipv4Addr = match ip.parse() {
        Ok(a) => a,
        Err(_) => return String::new(),
    };
    match discovery::snmp::identify_printer(addr, community, verbose).await {
        Some(p) => p.model.unwrap_or_default(),
        None => String::new(),
    }
}


use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::drivers::known_matches::KnownMatches;
use crate::drivers::manifest::Manifest;
use crate::models::*;

/// Minimum score for a driver to be considered a fuzzy match.
/// Scale is 0-1000. See `score_driver` for how scores are computed.
pub const MIN_FUZZY_SCORE: u32 = 250;

/// Fixed score for curated exact matches. Higher than any fuzzy match can reach.
const EXACT_SCORE: u32 = 1000;

/// Match a printer model string against all driver sources.
/// Returns a DriverResults with matched drivers (ranked) and universal drivers.
///
/// `local_store_drivers` is a list of driver names already staged on this machine
/// (from pnputil /enum-drivers).
pub fn match_drivers(model: &str, local_store_drivers: &[String]) -> DriverResults {
    let known = KnownMatches::load_embedded();
    let manifest = Manifest::load_embedded();

    let mut matched: Vec<DriverMatch> = Vec::new();
    let mut universal: Vec<DriverMatch> = Vec::new();

    // Tier 1: Exact match from known_matches.toml
    if let Some(km) = known.find(model) {
        matched.push(DriverMatch {
            name: km.driver.clone(),
            category: DriverCategory::Matched,
            confidence: MatchConfidence::Exact,
            source: match km.source.as_str() {
                "local_store" => DriverSource::LocalStore,
                _ => DriverSource::Manufacturer,
            },
            score: EXACT_SCORE,
            driver_date: None,
        });
    }

    // Tier 2: Score every local-store driver against the model.
    // Keep only those above the threshold.
    for driver_name in local_store_drivers {
        if matched.iter().any(|m| m.name == *driver_name) {
            continue;
        }
        let score = score_driver(model, driver_name);
        if score >= MIN_FUZZY_SCORE {
            matched.push(DriverMatch {
                name: driver_name.clone(),
                category: DriverCategory::Matched,
                confidence: MatchConfidence::Fuzzy,
                source: DriverSource::LocalStore,
                score,
                driver_date: None,
            });
        }
    }

    // Tier 2b: Score every known_matches entry we haven't already added.
    for km in &known.matches {
        if matched.iter().any(|m| m.name == km.driver) {
            continue;
        }
        let score = score_driver(model, &km.driver);
        if score >= MIN_FUZZY_SCORE {
            matched.push(DriverMatch {
                name: km.driver.clone(),
                category: DriverCategory::Matched,
                confidence: MatchConfidence::Fuzzy,
                source: match km.source.as_str() {
                    "local_store" => DriverSource::LocalStore,
                    _ => DriverSource::Manufacturer,
                },
                score,
                driver_date: None,
            });
        }
    }

    // Sort matched: Exact first, then by score descending.
    matched.sort_by(|a, b| match (&a.confidence, &b.confidence) {
        (MatchConfidence::Exact, MatchConfidence::Exact) => b.score.cmp(&a.score),
        (MatchConfidence::Exact, _) => std::cmp::Ordering::Less,
        (_, MatchConfidence::Exact) => std::cmp::Ordering::Greater,
        _ => b.score.cmp(&a.score),
    });

    // Universal drivers for this manufacturer (unscored — always shown as fallback)
    if let Some(mfr) = manifest.find_manufacturer(model) {
        for ud in &mfr.universal_drivers {
            universal.push(DriverMatch {
                name: ud.name.clone(),
                category: DriverCategory::Universal,
                confidence: MatchConfidence::Universal,
                source: DriverSource::Manufacturer,
                score: 0,
                driver_date: None,
            });
        }
    }

    DriverResults {
        printer_model: model.to_string(),
        matched,
        universal,
        device_id: None,
        catalog: None,
        bundle_candidates: Vec::new(),
        #[cfg(feature = "sdi")]
        sdi_candidates: Vec::new(),
    }
}

/// Apply driver dates from a `(name → date)` map onto an existing
/// `DriverResults`. Used by the `drivers` command to populate dates from
/// the local driver store without reworking `match_drivers`'s signature.
///
/// Each `DriverMatch` whose `name` appears in `dates` has its `driver_date`
/// field set to the provided value (overwrites any existing date). Missing
/// names are left alone.
pub fn enrich_with_dates(
    results: &mut DriverResults,
    dates: &std::collections::HashMap<String, Option<String>>,
) {
    for dm in results.matched.iter_mut().chain(results.universal.iter_mut()) {
        if let Some(date) = dates.get(&dm.name) {
            dm.driver_date = date.clone();
        }
    }
}

/// Score how well a driver name matches a printer model, on a 0-1000 scale.
///
/// Composition:
/// - **Model number prefix match (0 or 500):** If the model and driver share
///   an alphanumeric "model number" token and one is a prefix of the other
///   (e.g. `m428fdw` matches `m428f`), this is a strong signal of same-model
///   drivers. All-or-nothing, worth 500.
/// - **Token overlap (0-300):** Fraction of the shorter side's tokens that
///   appear in the longer side, scaled to 300.
/// - **Skim subsequence score (0-200):** Raw score from SkimMatcherV2, capped
///   at 200 to keep it from dominating. Catches suffix/insertion similarity
///   that plain token overlap misses.
pub fn score_driver(model: &str, driver: &str) -> u32 {
    let model_norm = normalize_model(model);
    let driver_norm = normalize_model(driver);
    let model_tokens: Vec<&str> = model_norm.split_whitespace().collect();
    let driver_tokens: Vec<&str> = driver_norm.split_whitespace().collect();

    if model_tokens.is_empty() || driver_tokens.is_empty() {
        return 0;
    }

    // Component 1: Model number prefix match
    let model_nums: Vec<&str> = model_tokens.iter().copied().filter(|t| is_model_number(t)).collect();
    let driver_nums: Vec<&str> = driver_tokens.iter().copied().filter(|t| is_model_number(t)).collect();
    let model_num_bonus: u32 = if model_nums.iter().any(|mn| driver_nums.iter().any(|dn| model_numbers_match(mn, dn))) {
        500
    } else {
        0
    };

    // Component 2: Token overlap
    let (shorter, longer) = if model_tokens.len() <= driver_tokens.len() {
        (&model_tokens, &driver_tokens)
    } else {
        (&driver_tokens, &model_tokens)
    };
    let hits = shorter.iter().filter(|t| longer.contains(t)).count();
    let overlap_ratio = hits as f64 / shorter.len() as f64;
    let overlap_score = (overlap_ratio * 300.0) as u32;

    // Component 3: Skim subsequence score, capped at 200
    let matcher = SkimMatcherV2::default();
    let skim_raw = matcher
        .fuzzy_match(&driver_norm, &model_norm)
        .or_else(|| matcher.fuzzy_match(&model_norm, &driver_norm))
        .unwrap_or(0)
        .max(0) as u32;
    let skim_score = skim_raw.min(200);

    model_num_bonus + overlap_score + skim_score
}

/// Normalize a model/driver string for fuzzy comparison.
/// Strips common noise words and normalizes whitespace.
fn normalize_model(s: &str) -> String {
    let noise = ["mfp", "series", "printer", "all-in-one", "multifunction"];
    let lower = s.to_lowercase();
    let words: Vec<&str> = lower
        .split_whitespace()
        .filter(|w| !noise.contains(w))
        .collect();
    words.join(" ")
}

/// A "model number" token contains both letters and digits (e.g. `m428fdw`, `l2750dw`, `cp5225`).
fn is_model_number(s: &str) -> bool {
    let has_letter = s.chars().any(|c| c.is_alphabetic());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    has_letter && has_digit
}

/// Two model numbers "match" if one is a prefix of the other.
/// Catches `m428fdw` (from SNMP) vs `m428f` (driver name for the family).
fn model_numbers_match(a: &str, b: &str) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

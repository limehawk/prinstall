use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::drivers::known_matches::KnownMatches;
use crate::drivers::manifest::Manifest;
use crate::models::*;

/// Minimum fuzzy match score to include a driver in results.
const MIN_FUZZY_SCORE: i64 = 80;

/// Match a printer model string against all driver sources.
/// Returns a DriverResults with matched drivers (ranked) and universal drivers.
///
/// `local_store_drivers` is a list of driver names already staged on this machine
/// (from pnputil /enum-drivers).
pub fn match_drivers(model: &str, local_store_drivers: &[String]) -> DriverResults {
    let known = KnownMatches::load_embedded();
    let manifest = Manifest::load_embedded();
    let matcher = SkimMatcherV2::default();

    let mut matched = Vec::new();
    let mut universal = Vec::new();

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
        });
    }

    // Tier 2: Fuzzy match against local store drivers
    let normalized_model = normalize_model(model);
    for driver_name in local_store_drivers {
        // Skip if this is already an exact match
        if matched.iter().any(|m| m.name == *driver_name) {
            continue;
        }

        let normalized_driver = normalize_model(driver_name);
        let skim_score = matcher.fuzzy_match(&normalized_driver, &normalized_model)
            .or_else(|| matcher.fuzzy_match(&normalized_model, &normalized_driver));
        let passes = match skim_score {
            Some(s) => s >= MIN_FUZZY_SCORE,
            None => token_overlap_score(&normalized_model, &normalized_driver) >= 0.6,
        };
        if passes {
            matched.push(DriverMatch {
                name: driver_name.clone(),
                category: DriverCategory::Matched,
                confidence: MatchConfidence::Fuzzy,
                source: DriverSource::LocalStore,
            });
        }
    }

    // Tier 2b: Fuzzy match against known_matches entries we didn't exact-match
    for km in &known.matches {
        if matched.iter().any(|m| m.name == km.driver) {
            continue;
        }
        let normalized_driver = normalize_model(&km.driver);
        let skim_score = matcher.fuzzy_match(&normalized_driver, &normalized_model)
            .or_else(|| matcher.fuzzy_match(&normalized_model, &normalized_driver));
        let passes = match skim_score {
            Some(s) => s >= MIN_FUZZY_SCORE,
            None => token_overlap_score(&normalized_model, &normalized_driver) >= 0.6,
        };
        if passes {
            matched.push(DriverMatch {
                name: km.driver.clone(),
                category: DriverCategory::Matched,
                confidence: MatchConfidence::Fuzzy,
                source: DriverSource::Manufacturer,
            });
        }
    }

    // Sort matched drivers: Exact first, then Fuzzy
    matched.sort_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap());

    // Universal drivers for this manufacturer
    if let Some(mfr) = manifest.find_manufacturer(model) {
        for ud in &mfr.universal_drivers {
            universal.push(DriverMatch {
                name: ud.name.clone(),
                category: DriverCategory::Universal,
                confidence: MatchConfidence::Universal,
                source: DriverSource::Manufacturer,
            });
        }
    }

    DriverResults {
        printer_model: model.to_string(),
        matched,
        universal,
    }
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

/// Token-overlap score: fraction of tokens in the shorter string that appear in the longer.
/// Fallback when skim's subsequence matcher returns None (e.g. divergent suffixes).
fn token_overlap_score(a: &str, b: &str) -> f64 {
    let a_tokens: Vec<&str> = a.split_whitespace().collect();
    let b_tokens: Vec<&str> = b.split_whitespace().collect();
    let (shorter, longer) = if a_tokens.len() <= b_tokens.len() {
        (&a_tokens, &b_tokens)
    } else {
        (&b_tokens, &a_tokens)
    };
    if shorter.is_empty() {
        return 0.0;
    }
    let hits = shorter.iter().filter(|t| longer.contains(t)).count();
    hits as f64 / shorter.len() as f64
}

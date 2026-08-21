use crate::drivers::known_matches::KnownMatches;
use crate::drivers::manifest::Manifest;
use crate::models::*;

/// Minimum score for a driver to be considered a fuzzy match.
/// Scale is 0-1000. See `score_driver` for how scores are computed.
pub const MIN_FUZZY_SCORE: u32 = 250;

/// Floor score applied when a local-store manufacturer Universal is promoted
/// despite raw fuzzy score falling under [`MIN_FUZZY_SCORE`]. Keeps ranking
/// below model-specific fuzzy hits while still selecting as a candidate.
const LOCAL_UNIVERSAL_FLOOR: u32 = MIN_FUZZY_SCORE;

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
    let mfr = manifest.find_manufacturer(model);

    let mut matched: Vec<DriverMatch> = Vec::new();
    let mut universal: Vec<DriverMatch> = Vec::new();
    let mut near_misses: Vec<DriverNearMiss> = Vec::new();

    // Tier 1: Exact match from known_matches.toml
    if let Some(km) = known.find(model) {
        matched.push(DriverMatch {
            name: km.driver.clone(),
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
    // Keep only those above the threshold. Manufacturer-aligned Universals
    // that land under the floor are promoted separately (see below) so a
    // staged UPD is never invisible to `drivers` / `add`.
    for driver_name in local_store_drivers {
        if matched.iter().any(|m| m.name == *driver_name) {
            continue;
        }
        let score = score_driver(model, driver_name);
        if score >= MIN_FUZZY_SCORE {
            matched.push(DriverMatch {
                name: driver_name.clone(),
                confidence: MatchConfidence::Fuzzy,
                source: DriverSource::LocalStore,
                score,
                driver_date: None,
            });
        } else if score > 0 {
            near_misses.push(DriverNearMiss {
                name: driver_name.clone(),
                score,
                reason: "below fuzzy threshold".into(),
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

    // Tier 2c: Promote local-store manufacturer Universals that the fuzzy
    // scorer rejected. Universals intentionally omit model numbers, so token
    // overlap alone often scores ~150–200 on multi-token models
    // ("KONICA MINOLTA bizhub C250i" vs "KONICA MINOLTA Universal PCL").
    if let Some(mfr) = mfr {
        for driver_name in local_store_drivers {
            if matched.iter().any(|m| m.name == *driver_name)
                || universal.iter().any(|u| u.name == *driver_name)
            {
                continue;
            }
            if !is_local_universal_for_manufacturer(mfr, driver_name) {
                continue;
            }
            let raw = score_driver(model, driver_name);
            let score = raw.max(LOCAL_UNIVERSAL_FLOOR);
            matched.push(DriverMatch {
                name: driver_name.clone(),
                confidence: MatchConfidence::Fuzzy,
                source: DriverSource::LocalStore,
                score,
                driver_date: None,
            });
            // Drop from near-misses if we just promoted it.
            near_misses.retain(|n| n.name != *driver_name);
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
    if let Some(mfr) = mfr {
        for ud in &mfr.universal_drivers {
            // Prefer the local-store row if we already promoted / matched it.
            if matched.iter().any(|m| m.name == ud.name)
                || universal.iter().any(|u| u.name == ud.name)
            {
                continue;
            }
            // If the local store has a close name variant (versioned alias),
            // surface that instead of the bare registry hint when present.
            if let Some(local) = local_store_drivers
                .iter()
                .find(|d| local_name_matches_universal(d, &ud.name))
            {
                if !matched.iter().any(|m| m.name == *local) {
                    universal.push(DriverMatch {
                        name: local.clone(),
                        confidence: MatchConfidence::Universal,
                        source: DriverSource::LocalStore,
                        score: 0,
                        driver_date: None,
                    });
                }
                continue;
            }
            universal.push(DriverMatch {
                name: ud.name.clone(),
                confidence: MatchConfidence::Universal,
                source: DriverSource::Manufacturer,
                score: 0,
                driver_date: None,
            });
        }
    }

    near_misses.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    // Cap diagnostics so empty-state output stays readable.
    near_misses.truncate(8);

    DriverResults {
        printer_model: model.to_string(),
        matched,
        universal,
        near_misses,
        device_id: None,
        catalog: None,
        bundle_candidates: Vec::new(),
        #[cfg(feature = "sdi")]
        sdi_candidates: Vec::new(),
    }
}

/// True when a local-store driver name is a manufacturer Universal for `mfr`.
///
/// Matches either an exact (or case-insensitive) registry universal name, or
/// a name that looks like a Universal/UPD and shares a manufacturer prefix.
fn is_local_universal_for_manufacturer(
    mfr: &crate::drivers::manifest::Manufacturer,
    driver_name: &str,
) -> bool {
    if mfr
        .universal_drivers
        .iter()
        .any(|ud| local_name_matches_universal(driver_name, &ud.name))
    {
        return true;
    }
    if !looks_like_universal_driver(driver_name) {
        return false;
    }
    let driver_upper = driver_name.to_uppercase();
    mfr.prefixes
        .iter()
        .any(|p| driver_upper.contains(&p.to_uppercase()))
}

fn looks_like_universal_driver(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("universal")
        || lower.contains("global print driver")
        || lower.split_whitespace().any(|t| t == "upd")
        || lower.contains(" upd")
}

fn local_name_matches_universal(local: &str, registry: &str) -> bool {
    if local.eq_ignore_ascii_case(registry) {
        return true;
    }
    // Versioned alias: "KONICA MINOLTA Universal PCL v3.9.13"
    let local_l = local.to_ascii_lowercase();
    let reg_l = registry.to_ascii_lowercase();
    local_l.starts_with(&reg_l)
        && local_l
            .get(reg_l.len()..)
            .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('v') || rest.starts_with('.'))
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
/// Composition: model-number prefix (0 or 500) + token overlap (0–300).
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

    model_num_bonus + overlap_score
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

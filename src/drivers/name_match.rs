//! Post-stage driver name matcher.
//!
//! `drivers.toml` carries a human-friendly hint like `"HP Universal Print
//! Driver PCL6"`, but the real string Windows registers is whatever the INF's
//! `[Models]` section declares (e.g. `"HP Universal Printing PCL 6"` — note
//! the different spelling). `Add-PrinterDriver -Name "..."` needs that real
//! string or it fails with HRESULT `0x80070705` ("Unknown printer driver").
//!
//! This module handles the post-stage reconciliation: given the list of
//! display names pulled out of freshly-staged INFs, pick the one that best
//! matches the manifest hint. Pragmatic token-overlap score — handles the
//! `PCL6` vs `PCL 6` variant by splitting digits away from letters during
//! normalization.
//!
//! The caller in `commands/add.rs::stage_driver_if_needed` stores the chosen
//! name on `StageOutcome::StagedVerified { actual_driver_name, .. }` (or
//! `StagedUnverified`) and downstream install calls prefer it over the hint.

/// Return the staged driver name whose tokens overlap most with `hint`.
///
/// Returns `None` when the staged list is empty. When no name shares any
/// tokens with the hint we still return *something* — the first entry —
/// because having a real INF-declared name beats dying on a bogus hint. The
/// caller is expected to fall back to the hint if this returns `None`.
pub fn pick_best_driver_name(staged_names: &[String], hint: &str) -> Option<String> {
    if staged_names.is_empty() {
        return None;
    }

    let hint_normalized = normalize_for_match(hint);
    let hint_tokens: Vec<&str> = hint_normalized.split_whitespace().collect();

    let mut best: Option<(String, usize)> = None;
    for name in staged_names {
        let name_normalized = normalize_for_match(name);
        let name_tokens: Vec<&str> = name_normalized.split_whitespace().collect();
        let overlap = hint_tokens
            .iter()
            .filter(|t| name_tokens.contains(t))
            .count();
        if best.as_ref().is_none_or(|(_, s)| overlap > *s) {
            best = Some((name.clone(), overlap));
        }
    }

    best.map(|(n, _)| n)
}

/// Normalize a driver name for token matching.
///
/// Lowercases, replaces punctuation with spaces, and critically — splits
/// digits attached to letters (so `"PCL6"` becomes `"pcl 6"` and lines up with
/// `"PCL 6"`). This is the difference between a 2-token and a 4-token
/// overlap on the HP Universal case.
pub fn normalize_for_match(s: &str) -> String {
    let mut out = String::new();
    let mut prev_alpha = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if !prev_alpha && !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push(c.to_ascii_lowercase());
            prev_alpha = true;
        } else if c.is_ascii_digit() {
            if prev_alpha {
                out.push(' ');
            }
            out.push(c);
            prev_alpha = false;
        } else {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            prev_alpha = false;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_best_match_by_token_overlap() {
        let names = vec![
            "HP Universal Printing PCL 6".to_string(),
            "HP Color LaserJet 2840".to_string(),
        ];
        let picked = pick_best_driver_name(&names, "HP Universal Print Driver PCL6");
        assert_eq!(picked.as_deref(), Some("HP Universal Printing PCL 6"));
    }

    #[test]
    fn picks_first_when_no_hint_matches_well() {
        let names = vec!["HP Color LaserJet 2840".to_string()];
        // No strong token overlap but we still pick something — caller falls
        // back to the hint via unwrap_or if it sees None, but here we always
        // return Some because the list is non-empty.
        let picked = pick_best_driver_name(&names, "HP Universal Print Driver PCL6");
        assert!(picked.is_some());
    }

    #[test]
    fn returns_none_for_empty_input() {
        assert_eq!(pick_best_driver_name(&[], "anything"), None);
    }

    #[test]
    fn normalize_splits_attached_digits() {
        assert_eq!(normalize_for_match("PCL6"), "pcl 6");
        assert_eq!(
            normalize_for_match("HP Universal Printing PCL 6"),
            "hp universal printing pcl 6"
        );
    }

    #[test]
    fn normalize_lowercases_and_strips_punctuation() {
        assert_eq!(normalize_for_match("HP_Universal-Print"), "hp universal print");
    }

    #[test]
    fn picks_best_when_hint_has_attached_digits() {
        // Real-world HP UPD case: drivers.toml says "PCL6", INF says "PCL 6".
        // After normalization both become "pcl 6" → overlap works.
        let names = vec![
            "HP Universal Printing PCL 6".to_string(),
            "HP Color LaserJet 2840".to_string(),
            "HP LaserJet Pro M404".to_string(),
        ];
        let picked = pick_best_driver_name(&names, "HP Universal Print Driver PCL6");
        assert_eq!(picked.as_deref(), Some("HP Universal Printing PCL 6"));
    }

    #[test]
    fn deduplication_is_callers_problem() {
        // pick_best_driver_name doesn't dedupe — that's on the caller. But
        // duplicates shouldn't break the scoring.
        let names = vec![
            "HP Universal Printing PCL 6".to_string(),
            "HP Universal Printing PCL 6".to_string(),
        ];
        let picked = pick_best_driver_name(&names, "HP Universal Print Driver PCL6");
        assert_eq!(picked.as_deref(), Some("HP Universal Printing PCL 6"));
    }
}

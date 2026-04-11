mod matcher_test {
    use prinstall::models::*;
    use prinstall::drivers::matcher;

    #[test]
    fn exact_match_ranks_first() {
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &[],  // no local store drivers
        );
        assert!(!results.matched.is_empty());
        assert_eq!(results.matched[0].confidence, MatchConfidence::Exact);
        assert!(results.matched[0].name.contains("M428f"));
    }

    #[test]
    fn universal_drivers_present_for_known_manufacturer() {
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &[],
        );
        assert!(!results.universal.is_empty());
        assert!(results.universal.iter().any(|d| d.name.contains("Universal")));
        assert!(results.universal.iter().all(|d| d.category == DriverCategory::Universal));
    }

    #[test]
    fn unknown_model_still_shows_universal_if_manufacturer_known() {
        let results = matcher::match_drivers(
            "HP SomeNewPrinter 9999xyz",
            &[],
        );
        // No exact match expected
        assert!(results.matched.iter().all(|d| d.confidence != MatchConfidence::Exact));
        // But universal drivers should still be there
        assert!(!results.universal.is_empty());
    }

    #[test]
    fn local_store_drivers_get_fuzzy_matched() {
        let local_drivers = vec![
            "HP LaserJet Pro MFP M428f PCL-6".to_string(),
            "HP Color LaserJet CP5225".to_string(),
            "Brother MFC-L8900CDW".to_string(),
        ];
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &local_drivers,
        );
        // The M428f driver should fuzzy-match and appear
        let has_local_match = results.matched.iter().any(|d| {
            d.name.contains("M428f") && d.source == DriverSource::LocalStore
        });
        assert!(has_local_match);
    }

    #[test]
    fn completely_unknown_manufacturer_returns_empty() {
        let results = matcher::match_drivers(
            "Acme Printer 3000",
            &[],
        );
        assert!(results.matched.is_empty());
        assert!(results.universal.is_empty());
    }

    #[test]
    fn matched_drivers_sorted_by_confidence() {
        let local_drivers = vec![
            "HP LaserJet Pro M400 Series PCL-6".to_string(),
        ];
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &local_drivers,
        );
        // If we have both exact and fuzzy, exact should come first
        if results.matched.len() >= 2 {
            assert!(results.matched[0].confidence <= results.matched[1].confidence);
        }
    }

    // ── New scoring tests ─────────────────────────────────────────────────

    #[test]
    fn exact_match_has_maximum_score() {
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &[],
        );
        let exact = results.matched.iter()
            .find(|m| m.confidence == MatchConfidence::Exact)
            .expect("should have an exact match");
        assert_eq!(exact.score, 1000);
    }

    #[test]
    fn specific_model_beats_family_driver() {
        // A driver for the exact model should rank higher than a driver
        // for just the family (M400 series).
        let local_drivers = vec![
            "HP LaserJet Pro M400 Series PCL6".to_string(),
            "HP LaserJet Pro MFP M428f PCL-6".to_string(),
        ];
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &local_drivers,
        );
        // Find both — the M428f one should rank above the M400 one.
        let m428f_idx = results.matched.iter().position(|m| m.name.contains("M428f"));
        let m400_idx = results.matched.iter().position(|m| m.name.contains("M400"));
        assert!(m428f_idx.is_some(), "M428f driver should be in matches");
        // M400 might or might not be in matches — depends on scoring.
        // If both are present, M428f must come first.
        if let (Some(a), Some(b)) = (m428f_idx, m400_idx) {
            assert!(a < b, "M428f must rank above M400 (got {} vs {})", a, b);
        }
    }

    #[test]
    fn wrong_family_drivers_excluded() {
        // Drivers for unrelated product lines should not appear as matches.
        let local_drivers = vec![
            "HP Color LaserJet CP5225".to_string(),
            "Brother MFC-L8900CDW".to_string(),
            "HP LaserJet Pro MFP M428f PCL-6".to_string(),
        ];
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &local_drivers,
        );
        // M428f must match
        assert!(results.matched.iter().any(|m| m.name.contains("M428f")));
        // CP5225 (different HP product line) must NOT be in matches
        assert!(
            !results.matched.iter().any(|m| m.name.contains("CP5225")),
            "CP5225 should not match M428fdw"
        );
        // Brother driver must NOT match an HP printer
        assert!(
            !results.matched.iter().any(|m| m.name.contains("Brother")),
            "Brother driver should not match HP printer"
        );
    }

    #[test]
    fn model_number_prefix_matching_works() {
        // The SNMP-reported model often has a suffix (fdw, cdw) that
        // the driver family name doesn't have. Prefix matching should
        // still find the driver.
        let score = matcher::score_driver(
            "HP LaserJet Pro MFP M428fdw",
            "HP LaserJet Pro MFP M428f PCL-6",
        );
        // Model number prefix match alone is worth 500. Plus token overlap.
        // Total should comfortably clear the 250 threshold.
        assert!(score >= 500, "prefix-matched driver should score high, got {}", score);
    }

    #[test]
    fn fuzzy_match_score_is_recorded() {
        let local_drivers = vec![
            "HP LaserJet Pro MFP M428f PCL-6".to_string(),
        ];
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &local_drivers,
        );
        let fuzzy = results.matched.iter()
            .find(|m| m.confidence == MatchConfidence::Fuzzy)
            .expect("should have a fuzzy match");
        assert!(fuzzy.score > 0);
        assert!(fuzzy.score < 1000);
    }

    #[test]
    fn fuzzy_matches_sorted_by_score_desc() {
        // Three drivers of varying quality, all should match but in order.
        let local_drivers = vec![
            "HP LaserJet Pro M400 Series PCL6".to_string(),         // family, ok
            "HP LaserJet Pro MFP M428f PCL-6".to_string(),          // specific, best
            "HP LaserJet Pro MFP M428f PostScript".to_string(),     // specific, best-ish
        ];
        let results = matcher::match_drivers(
            "HP LaserJet Pro MFP M428fdw",
            &local_drivers,
        );
        // All fuzzy matches must be in descending score order.
        let fuzzies: Vec<_> = results.matched.iter()
            .filter(|m| m.confidence == MatchConfidence::Fuzzy)
            .collect();
        for pair in fuzzies.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "fuzzy matches out of order: {} ({}) vs {} ({})",
                pair[0].name, pair[0].score, pair[1].name, pair[1].score
            );
        }
    }

    #[test]
    fn empty_model_returns_no_matches() {
        assert_eq!(matcher::score_driver("", "HP LaserJet Pro"), 0);
        assert_eq!(matcher::score_driver("HP LaserJet Pro", ""), 0);
    }
}

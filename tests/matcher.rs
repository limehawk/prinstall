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
}

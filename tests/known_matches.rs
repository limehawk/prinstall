mod known_matches_test {
    #[test]
    fn parses_embedded_known_matches() {
        let db = prinstall::drivers::known_matches::KnownMatches::load_embedded();
        assert!(db.matches.len() >= 3);
    }

    #[test]
    fn finds_exact_match() {
        let db = prinstall::drivers::known_matches::KnownMatches::load_embedded();
        let result = db.find("HP LaserJet Pro MFP M428fdw");
        assert!(result.is_some());
        assert_eq!(result.unwrap().driver, "HP LaserJet Pro MFP M428f PCL-6 (V4)");
    }

    #[test]
    fn returns_none_for_unknown_model() {
        let db = prinstall::drivers::known_matches::KnownMatches::load_embedded();
        let result = db.find("Acme Printer 3000");
        assert!(result.is_none());
    }
}

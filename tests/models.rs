// We'll test that our model types serialize/deserialize correctly
// for the --json output flag

mod models_test {
    #[test]
    fn printer_serializes_to_json() {
        // Will fail until models.rs exists
        let _printer = prinstall::models::Printer {
            ip: "192.168.1.50".to_string(),
            model: Some("HP LaserJet Pro MFP M428fdw".to_string()),
            serial: None,
            status: prinstall::models::PrinterStatus::Ready,
        };
        let json = serde_json::to_string(&_printer).unwrap();
        assert!(json.contains("192.168.1.50"));
        assert!(json.contains("HP LaserJet Pro MFP M428fdw"));
    }

    #[test]
    fn driver_match_preserves_category_and_confidence() {
        let dm = prinstall::models::DriverMatch {
            name: "HP LaserJet Pro MFP M428f PCL-6 (V4)".to_string(),
            category: prinstall::models::DriverCategory::Matched,
            confidence: prinstall::models::MatchConfidence::Exact,
            source: prinstall::models::DriverSource::LocalStore,
        };
        let json = serde_json::to_string(&dm).unwrap();
        assert!(json.contains("matched"));
        assert!(json.contains("exact"));
        assert!(json.contains("local_store"));
    }

    #[test]
    fn driver_results_has_both_sections() {
        let results = prinstall::models::DriverResults {
            printer_model: "HP LaserJet Pro MFP M428fdw".to_string(),
            matched: vec![],
            universal: vec![],
        };
        assert_eq!(results.matched.len(), 0);
        assert_eq!(results.universal.len(), 0);
    }

    #[test]
    fn install_result_serializes() {
        let result = prinstall::models::InstallResult {
            success: true,
            printer_name: "HP M428fdw".to_string(),
            driver_name: "HP LaserJet Pro MFP M428f PCL-6 (V4)".to_string(),
            port_name: "IP_192.168.1.50".to_string(),
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
    }
}

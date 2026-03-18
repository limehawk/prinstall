mod output_test {
    use prinstall::models::*;
    use prinstall::output;

    #[test]
    fn format_scan_results_includes_all_printers() {
        let printers = vec![
            Printer {
                ip: Some("192.168.1.50".parse().unwrap()),
                model: Some("HP LaserJet Pro MFP M428fdw".to_string()),
                serial: None,
                status: PrinterStatus::Ready,
                discovery_methods: vec![DiscoveryMethod::Snmp],
                ports: vec![],
                source: PrinterSource::Network,
                local_name: None,
            },
            Printer {
                ip: Some("192.168.1.51".parse().unwrap()),
                model: Some("Ricoh IM C3000".to_string()),
                serial: None,
                status: PrinterStatus::Offline,
                discovery_methods: vec![DiscoveryMethod::Snmp],
                ports: vec![],
                source: PrinterSource::Network,
                local_name: None,
            },
        ];
        let text = output::format_scan_results(&printers);
        assert!(text.contains("192.168.1.50"));
        assert!(text.contains("HP LaserJet Pro MFP M428fdw"));
        assert!(text.contains("192.168.1.51"));
        assert!(text.contains("Ricoh IM C3000"));
    }

    #[test]
    fn format_scan_results_json() {
        let printers = vec![
            Printer {
                ip: Some("192.168.1.50".parse().unwrap()),
                model: Some("HP LaserJet Pro".to_string()),
                serial: None,
                status: PrinterStatus::Ready,
                discovery_methods: vec![DiscoveryMethod::Snmp],
                ports: vec![],
                source: PrinterSource::Network,
                local_name: None,
            },
        ];
        let json = output::format_scan_results_json(&printers);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["ip"], "192.168.1.50");
    }

    #[test]
    fn format_driver_results_has_both_sections() {
        let results = DriverResults {
            printer_model: "HP LaserJet Pro MFP M428fdw".to_string(),
            matched: vec![
                DriverMatch {
                    name: "HP LaserJet Pro MFP M428f PCL-6 (V4)".to_string(),
                    category: DriverCategory::Matched,
                    confidence: MatchConfidence::Exact,
                    source: DriverSource::LocalStore,
                },
            ],
            universal: vec![
                DriverMatch {
                    name: "HP Universal Print Driver PCL6".to_string(),
                    category: DriverCategory::Universal,
                    confidence: MatchConfidence::Universal,
                    source: DriverSource::Manufacturer,
                },
            ],
        };
        let text = output::format_driver_results(&results);
        assert!(text.contains("Matched Drivers"));
        assert!(text.contains("Universal Drivers"));
        assert!(text.contains("M428f"));
        assert!(text.contains("Universal"));
    }

    #[test]
    fn format_no_results_message() {
        let text = output::format_snmp_failure_guidance("192.168.1.100");
        assert!(text.contains("SNMP"));
        assert!(text.contains("--community"));
        assert!(text.contains("--model"));
    }
}

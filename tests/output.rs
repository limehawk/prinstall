mod output_test {
    use prinstall::models::*;
    use prinstall::output;

    fn make_network_printer(ip: &str, model: Option<&str>, status: PrinterStatus) -> Printer {
        Printer {
            ip: ip.parse().ok(),
            model: model.map(|s| s.to_string()),
            serial: None,
            status,
            discovery_methods: vec![],
            ports: vec![],
            source: PrinterSource::Network,
            local_name: None,
        }
    }

    #[test]
    fn format_scan_results_includes_all_printers() {
        let printers = vec![
            make_network_printer(
                "192.168.1.50",
                Some("HP LaserJet Pro MFP M428fdw"),
                PrinterStatus::Ready,
            ),
            make_network_printer(
                "192.168.1.51",
                Some("Ricoh IM C3000"),
                PrinterStatus::Offline,
            ),
        ];
        let text = output::format_scan_results(&printers);
        assert!(text.contains("192.168.1.50"));
        assert!(text.contains("HP LaserJet Pro MFP M428fdw"));
        assert!(text.contains("192.168.1.51"));
        assert!(text.contains("Ricoh IM C3000"));
    }

    #[test]
    fn format_scan_results_json() {
        let printers = vec![make_network_printer(
            "192.168.1.50",
            Some("HP LaserJet Pro"),
            PrinterStatus::Ready,
        )];
        let json = output::format_scan_results_json(&printers);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        // ip is now serialized as Option<Ipv4Addr>
        assert!(!parsed[0]["ip"].is_null());
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
                    score: 1000,
                },
            ],
            universal: vec![
                DriverMatch {
                    name: "HP Universal Print Driver PCL6".to_string(),
                    category: DriverCategory::Universal,
                    confidence: MatchConfidence::Universal,
                    source: DriverSource::Manufacturer,
                    score: 0,
                },
            ],
            device_id: None,
            windows_update: None,
            catalog: None,
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

    #[test]
    fn format_scan_guidance_zero_results() {
        let text = output::format_scan_guidance("192.168.1.0/24", 0, 0);
        assert!(text.contains("No printers found"));
        assert!(text.contains("192.168.1.0/24"));
    }

    #[test]
    fn format_scan_guidance_hosts_but_no_models() {
        let text = output::format_scan_guidance("192.168.1.0/24", 3, 0);
        assert!(text.contains("3 device"));
        assert!(text.contains("model"));
    }
}

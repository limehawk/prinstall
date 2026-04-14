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
            port_name: None,
            driver_name: None,
            shared: None,
            is_default: None,
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
            #[cfg(feature = "sdi")]
            sdi_candidates: vec![],
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

    fn make_local_printer(
        name: &str,
        driver: &str,
        port: &str,
        source: PrinterSource,
        shared: bool,
        is_default: bool,
    ) -> Printer {
        Printer {
            ip: if port.starts_with("IP_") {
                port.trim_start_matches("IP_").parse().ok()
            } else {
                None
            },
            model: Some(driver.to_string()),
            serial: None,
            status: PrinterStatus::Ready,
            discovery_methods: vec![DiscoveryMethod::Local],
            ports: vec![],
            source,
            local_name: Some(name.to_string()),
            port_name: Some(port.to_string()),
            driver_name: Some(driver.to_string()),
            shared: Some(shared),
            is_default: Some(is_default),
        }
    }

    #[test]
    fn format_list_results_shows_name_driver_port_and_default_marker() {
        let printers = vec![
            make_local_printer(
                "Front Desk",
                "HP Universal Printing PCL 6",
                "IP_10.0.0.5",
                PrinterSource::Installed,
                true,
                true,
            ),
            make_local_printer(
                "Back Office",
                "Brother Laser Type1 Class Driver",
                "USB001",
                PrinterSource::Usb,
                false,
                false,
            ),
            make_local_printer(
                "Microsoft Print to PDF",
                "Microsoft Print To PDF",
                "PORTPROMPT:",
                PrinterSource::Installed,
                false,
                false,
            ),
        ];
        let text = output::format_list_results(&printers);
        // Queue names
        assert!(text.contains("Front Desk"));
        assert!(text.contains("Back Office"));
        assert!(text.contains("Microsoft Print to PDF"));
        // Drivers
        assert!(text.contains("HP Universal Printing PCL 6"));
        assert!(text.contains("Brother Laser Type1 Class Driver"));
        // Ports
        assert!(text.contains("IP_10.0.0.5"));
        assert!(text.contains("USB001"));
        assert!(text.contains("PORTPROMPT:"));
        // Shared column
        assert!(text.contains("Yes"));
        assert!(text.contains("No"));
        // Summary footer
        assert!(text.contains("3 printer(s)"));
        assert!(text.contains("1 USB"));
        assert!(text.contains("1 default"));
        // Default marker
        assert!(text.contains("* = Windows default printer"));
    }

    #[test]
    fn format_list_results_shows_ip_column_for_network_printers() {
        let printers = vec![
            make_local_printer(
                "Front Desk",
                "HP UPD",
                "IP_10.0.0.5",
                PrinterSource::Installed,
                false,
                false,
            ),
            make_local_printer(
                "Back Office",
                "Brother",
                "USB001",
                PrinterSource::Usb,
                false,
                false,
            ),
        ];
        let text = output::format_list_results(&printers);
        // Dedicated IP column header (separate from the Port column).
        let header_line = text.lines().find(|l| l.contains("Name") && l.contains("Driver")).expect("header row");
        assert!(header_line.contains("IP"), "expected IP column header, got: {header_line}");
        // Network printer's bare IP appears in its row independent of the Port cell.
        let front_desk_line = text.lines().find(|l| l.contains("Front Desk")).expect("Front Desk row");
        let ip_occurrences = front_desk_line.matches("10.0.0.5").count();
        assert!(
            ip_occurrences >= 2,
            "expected IP in both dedicated column AND Port column (IP_10.0.0.5), got {ip_occurrences} in:\n{front_desk_line}"
        );
    }

    #[test]
    fn format_list_results_empty_message() {
        let text = output::format_list_results(&[]);
        assert!(text.contains("No locally installed printers"));
    }

    #[test]
    #[cfg(feature = "sdi")]
    fn format_driver_results_renders_sdi_section_with_verification() {
        use prinstall::models::SdiDriverCandidate;

        let results = DriverResults {
            printer_model: "HP LaserJet 1320".into(),
            matched: vec![],
            universal: vec![],
            device_id: Some("USB\\VID_03F0&PID_1D17".into()),
            windows_update: None,
            catalog: None,
            sdi_candidates: vec![
                SdiDriverCandidate {
                    driver_name: "HP LaserJet 1320 Series".into(),
                    pack_name: "DP_Printer_26000".into(),
                    hwid_match: "USB\\VID_03F0&PID_1D17".into(),
                    verification: "verified".into(),
                    signer: Some("CN=HP Inc.".into()),
                },
                SdiDriverCandidate {
                    driver_name: "Random Generic Driver".into(),
                    pack_name: "DP_Sketchy_01".into(),
                    hwid_match: "USB\\VID_DEAD&PID_BEEF".into(),
                    verification: "unsigned (1/3)".into(),
                    signer: None,
                },
            ],
        };

        let out = output::format_driver_results(&results);
        // Section header shown
        assert!(out.contains("SDI Candidates"), "expected SDI Candidates header in:\n{out}");
        // Driver + pack names shown
        assert!(out.contains("HP LaserJet 1320 Series"));
        assert!(out.contains("DP_Printer_26000"));
        assert!(out.contains("Random Generic Driver"));
        assert!(out.contains("DP_Sketchy_01"));
        // Verification status per row
        assert!(out.contains("verified"));
        assert!(out.contains("unsigned"));
        // Signer shown for verified row
        assert!(out.contains("CN=HP Inc."));
    }

    #[test]
    #[cfg(feature = "sdi")]
    fn format_driver_results_omits_sdi_section_when_empty() {
        let results = DriverResults {
            printer_model: "HP LaserJet 1320".into(),
            matched: vec![],
            universal: vec![],
            device_id: None,
            windows_update: None,
            catalog: None,
            sdi_candidates: vec![],
        };
        let out = output::format_driver_results(&results);
        assert!(!out.contains("SDI Candidates"), "expected NO SDI section for empty candidates");
    }
}

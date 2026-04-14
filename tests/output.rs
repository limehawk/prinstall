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
    fn format_driver_results_header_uses_model_without_label_prefix() {
        let results = DriverResults {
            printer_model: "Brother MFC-L2750DW series".to_string(),
            matched: vec![],
            universal: vec![],
            device_id: None,
            windows_update: None,
            catalog: None,
            #[cfg(feature = "sdi")]
            sdi_candidates: vec![],
        };
        let text = output::format_driver_results(&results);
        // First non-empty line is the raw model — no "Printer:" label prefix.
        let first_line = text.lines().find(|l| !l.trim().is_empty()).expect("at least one line");
        assert_eq!(first_line.trim(), "Brother MFC-L2750DW series");
        assert!(!text.contains("Printer:"), "old 'Printer:' label should be gone");
    }

    #[test]
    fn format_driver_results_extracts_cid_from_device_id() {
        let results = DriverResults {
            printer_model: "Brother MFC-L2750DW series".to_string(),
            matched: vec![],
            universal: vec![],
            device_id: Some(
                "MFG:Brother;CMD:PJL,PCL;MDL:MFC-L2750DW series;CLS:PRINTER;CID:Brother Laser Type1;URF:W8,CP1".to_string()
            ),
            windows_update: None,
            catalog: None,
            #[cfg(feature = "sdi")]
            sdi_candidates: vec![],
        };
        let text = output::format_driver_results(&results);
        assert!(text.contains("CID: Brother Laser Type1"),
            "expected 'CID: Brother Laser Type1' in output:\n{text}");
        // Old "IPP Device ID:" label should not appear.
        assert!(!text.contains("IPP Device ID:"));
    }

    #[test]
    fn format_driver_results_renders_matched_and_universal_with_tree_icons() {
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
        // No section-header dividers.
        assert!(!text.contains("Matched Drivers"), "old section header should be gone");
        assert!(!text.contains("Universal Drivers"), "old section header should be gone");
        // Driver names still appear.
        assert!(text.contains("M428f"));
        assert!(text.contains("HP Universal Print Driver PCL6"));
        // Exact match uses the star icon.
        assert!(text.contains("\u{2605}"), "expected star (\u{2605}) for exact match");
        // Universal uses the open-circle icon.
        assert!(text.contains("\u{25CB}"), "expected open circle (\u{25CB}) for universal driver");
        // Evidence lines use the └ bullet.
        assert!(text.contains("\u{2514}"), "expected tree bullet \u{2514}");
        // Universal evidence mentions HWID.
        assert!(text.contains("no HWID match"));
    }

    #[test]
    fn format_driver_results_collapses_catalog_with_variant_count() {
        let entry = |title: &str, version: &str, date: &str| CatalogEntry {
            title: title.to_string(),
            products: "Windows 10, version 1803 and later".to_string(),
            classification: "Drivers".to_string(),
            last_updated: date.to_string(),
            version: version.to_string(),
            size: "3.5 MB".to_string(),
            size_bytes: 3_500_000,
            guid: "abc".to_string(),
        };
        let results = DriverResults {
            printer_model: "Brother MFC-L2750DW series".to_string(),
            matched: vec![],
            universal: vec![],
            device_id: None,
            windows_update: None,
            catalog: Some(CatalogSearchResult {
                query: "Brother MFC-L2750DW".to_string(),
                updates: vec![
                    entry("Brother Printer - 10.0.17119.1", "10.0.17119.1", "2009-04-21"),
                    entry("Brother Printer - 10.0.17119.0", "10.0.17119.0", "2008-05-01"),
                    entry("Brother Printer - 9.0.0.0", "9.0.0.0", "2007-01-01"),
                    entry("Brother Printer - 8.0.0.0", "8.0.0.0", "2006-01-01"),
                    entry("Brother Printer - 7.0.0.0", "7.0.0.0", "2005-01-01"),
                ],
                error: None,
            }),
            #[cfg(feature = "sdi")]
            sdi_candidates: vec![],
        };
        let text = output::format_driver_results(&results);
        // Only one catalog entry rendered (not 5).
        let occurrences = text.matches("Brother Printer").count();
        assert_eq!(occurrences, 1, "expected catalog collapsed to 1 row; got {occurrences}:\n{text}");
        // Variant count annotation present.
        assert!(text.contains("(Catalog \u{00B7} 5 variants)"),
            "expected '(Catalog \u{00B7} 5 variants)' annotation:\n{text}");
        // Best version used (10.0.17119.1 is newest).
        assert!(text.contains("10.0.17119.1"));
        // No products boilerplate.
        assert!(!text.contains("Windows 10, version 1803"));
        // No catalog footer.
        assert!(!text.contains("catalog.update.microsoft.com"));
    }

    #[test]
    fn format_driver_results_empty_shows_no_drivers_message() {
        let results = DriverResults {
            printer_model: "Unknown Printer".to_string(),
            matched: vec![],
            universal: vec![],
            device_id: None,
            windows_update: None,
            catalog: None,
            #[cfg(feature = "sdi")]
            sdi_candidates: vec![],
        };
        let text = output::format_driver_results(&results);
        assert!(text.contains("No drivers found for this printer."));
    }

    #[test]
    fn format_driver_results_renders_wu_probe_error_as_footer() {
        let results = DriverResults {
            printer_model: "Brother MFC-L2750DW series".to_string(),
            matched: vec![DriverMatch {
                name: "Brother Laser Type1 Class Driver".to_string(),
                category: DriverCategory::Matched,
                confidence: MatchConfidence::Exact,
                source: DriverSource::LocalStore,
                score: 1000,
            }],
            universal: vec![],
            device_id: None,
            windows_update: Some(WindowsUpdateProbe::failure("HRESULT 0x80070032")),
            catalog: None,
            #[cfg(feature = "sdi")]
            sdi_candidates: vec![],
        };
        let text = output::format_driver_results(&results);
        assert!(text.contains("Windows Update probe:"), "expected WU probe footer line:\n{text}");
        assert!(text.contains("0x80070032"));
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
    fn format_list_results_tree_layout_with_summary_and_icons() {
        let printers = vec![
            // Deliberately NOT in rank order — the formatter should reorder.
            make_local_printer(
                "Microsoft Print to PDF",
                "Microsoft Print To PDF",
                "PORTPROMPT:",
                PrinterSource::Installed,
                false,
                false,
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
                "Front Desk",
                "HP Universal Printing PCL 6",
                "IP_10.0.0.5",
                PrinterSource::Installed,
                true,
                true,
            ),
        ];
        let text = output::format_list_results(&printers);

        // Summary tokens at the top.
        assert!(text.contains("3 printer(s)"), "expected '3 printer(s)' summary:\n{text}");
        assert!(text.contains("1 network"), "expected '1 network' summary token:\n{text}");
        assert!(text.contains("1 USB"), "expected '1 USB' summary token:\n{text}");
        assert!(text.contains("1 default"), "expected '1 default' summary token:\n{text}");

        // The summary line precedes any printer block.
        let summary_pos = text.find("3 printer(s)").expect("summary present");
        let front_desk_pos = text.find("Front Desk").expect("Front Desk present");
        assert!(summary_pos < front_desk_pos, "summary should appear before printer blocks");

        // Default printer leads with the star icon.
        let star_line = text.lines().find(|l| l.contains("\u{2605}"))
            .expect("expected at least one star line");
        assert!(star_line.contains("Front Desk"),
            "expected default printer 'Front Desk' on the star line, got: {star_line}");

        // (no standalone dot here — the only network printer is the default,
        // so it gets the star icon instead of the filled dot.)

        // Drivers appear inline on evidence lines.
        assert!(text.contains("HP Universal Printing PCL 6"));
        assert!(text.contains("Brother Laser Type1 Class Driver"));

        // Bare IP appears on the Front Desk block (not just inside IP_10.0.0.5).
        let front_desk_block: String = text
            .lines()
            .skip_while(|l| !l.contains("Front Desk"))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(front_desk_block.contains("10.0.0.5"),
            "expected bare IP '10.0.0.5' in Front Desk block:\n{front_desk_block}");

        // Second evidence line has "Source · Status" format.
        assert!(text.contains("USB \u{00B7} Ready"),
            "expected 'USB · Ready' evidence line:\n{text}");
        assert!(text.contains("Network \u{00B7} Ready") || text.contains("Installed \u{00B7} Ready"),
            "expected 'Network · Ready' or 'Installed · Ready' line:\n{text}");

        // Tree bullet present (└).
        assert!(text.contains("\u{2514}"), "expected tree bullet \u{2514} in output");

        // Default annotation visible as text (belt-and-suspenders for NO_COLOR).
        assert!(text.contains("(default"),
            "expected '(default' annotation text near default queue:\n{text}");

        // Old table headers are gone.
        assert!(!text.contains("* = Windows default printer"),
            "old default marker footer should be gone");
    }

    #[test]
    fn format_list_results_network_printer_shows_bare_ip_with_dot_icon() {
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
        // No defaults here — network queue should get the filled-dot icon.
        assert!(text.contains("\u{25CF}"), "expected filled dot icon for network printer:\n{text}");
        // Front Desk row carries the bare IP on an evidence line.
        let front_desk_block: String = text
            .lines()
            .skip_while(|l| !l.contains("Front Desk"))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(front_desk_block.contains("10.0.0.5"),
            "expected bare IP '10.0.0.5' in Front Desk block:\n{front_desk_block}");
        // USB queue uses the open-circle icon.
        assert!(text.contains("\u{25CB}"), "expected open-circle icon for USB printer:\n{text}");
    }

    #[test]
    fn format_list_results_empty_message() {
        let text = output::format_list_results(&[]);
        assert!(text.contains("No locally installed printers"));
    }

    #[test]
    #[cfg(feature = "sdi")]
    fn format_driver_results_renders_verified_sdi_with_star_and_check() {
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
                    signer: Some("Microsoft WHCP".into()),
                },
            ],
        };
        let out = output::format_driver_results(&results);
        // No old "SDI Candidates" section header.
        assert!(!out.contains("SDI Candidates"),
            "expected no 'SDI Candidates' section header in new layout:\n{out}");
        // Verified SDI uses the star icon.
        assert!(out.contains("\u{2605}"), "expected star for verified SDI:\n{out}");
        // Driver name shown.
        assert!(out.contains("HP LaserJet 1320 Series"));
        // SDI evidence line with pack name.
        assert!(out.contains("SDI") && out.contains("DP_Printer_26000"),
            "expected 'SDI' and pack name in evidence:\n{out}");
        // Verified check mark and signer.
        assert!(out.contains("\u{2713}"), "expected check mark for verified:\n{out}");
        assert!(out.contains("verified"));
        assert!(out.contains("Microsoft WHCP"));
    }

    #[test]
    #[cfg(feature = "sdi")]
    fn format_driver_results_renders_unsigned_sdi_with_open_circle_and_x() {
        use prinstall::models::SdiDriverCandidate;

        let results = DriverResults {
            printer_model: "Generic Printer".into(),
            matched: vec![],
            universal: vec![],
            device_id: None,
            windows_update: None,
            catalog: None,
            sdi_candidates: vec![
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
        // Unsigned SDI uses open-circle icon.
        assert!(out.contains("\u{25CB}"), "expected open circle for unsigned SDI:\n{out}");
        // X mark for unsigned.
        assert!(out.contains("\u{2717}"), "expected X mark for unsigned:\n{out}");
        assert!(out.contains("unsigned"));
        assert!(out.contains("DP_Sketchy_01"));
    }

    #[test]
    #[cfg(feature = "sdi")]
    fn format_driver_results_verified_sdi_ordered_before_unsigned() {
        use prinstall::models::SdiDriverCandidate;

        let results = DriverResults {
            printer_model: "Generic".into(),
            matched: vec![],
            universal: vec![],
            device_id: None,
            windows_update: None,
            catalog: None,
            sdi_candidates: vec![
                // Unsigned first in the vec...
                SdiDriverCandidate {
                    driver_name: "Unsigned Driver".into(),
                    pack_name: "DP_Sketchy_01".into(),
                    hwid_match: "USB\\VID_DEAD".into(),
                    verification: "unsigned (1/3)".into(),
                    signer: None,
                },
                // ...but verified should render first in the output.
                SdiDriverCandidate {
                    driver_name: "Verified Driver".into(),
                    pack_name: "DP_Safe_01".into(),
                    hwid_match: "USB\\VID_BEEF".into(),
                    verification: "verified".into(),
                    signer: Some("CN=Trusted".into()),
                },
            ],
        };
        let out = output::format_driver_results(&results);
        let verified_pos = out.find("Verified Driver").expect("verified row present");
        let unsigned_pos = out.find("Unsigned Driver").expect("unsigned row present");
        assert!(verified_pos < unsigned_pos,
            "verified SDI should appear before unsigned in output:\n{out}");
    }

    #[test]
    #[cfg(feature = "sdi")]
    fn format_driver_results_omits_sdi_rows_when_empty() {
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
        assert!(!out.contains("SDI Candidates"), "expected no SDI section header");
        assert!(!out.contains("\u{2605}"), "no candidates → no star icons");
    }
}

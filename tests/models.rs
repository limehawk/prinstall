// We'll test that our model types serialize/deserialize correctly
// for the --json output flag

mod models_test {
    use prinstall::models::*;

    #[test]
    fn printer_serializes_to_json() {
        let _printer = Printer {
            ip: Some("192.168.1.50".parse().unwrap()),
            model: Some("HP LaserJet Pro MFP M428fdw".to_string()),
            serial: None,
            status: PrinterStatus::Ready,
            discovery_methods: vec![DiscoveryMethod::Snmp],
            ports: vec![],
            source: PrinterSource::Network,
            local_name: None,
        };
        let json = serde_json::to_string(&_printer).unwrap();
        assert!(json.contains("192.168.1.50"));
        assert!(json.contains("HP LaserJet Pro MFP M428fdw"));
    }

    #[test]
    fn driver_match_preserves_category_and_confidence() {
        let dm = DriverMatch {
            name: "HP LaserJet Pro MFP M428f PCL-6 (V4)".to_string(),
            category: DriverCategory::Matched,
            confidence: MatchConfidence::Exact,
            source: DriverSource::LocalStore,
            score: 1000,
        };
        let json = serde_json::to_string(&dm).unwrap();
        assert!(json.contains("matched"));
        assert!(json.contains("exact"));
        assert!(json.contains("local_store"));
    }

    #[test]
    fn driver_results_has_both_sections() {
        let results = DriverResults {
            printer_model: "HP LaserJet Pro MFP M428fdw".to_string(),
            matched: vec![],
            universal: vec![],
        };
        assert_eq!(results.matched.len(), 0);
        assert_eq!(results.universal.len(), 0);
    }

    #[test]
    fn install_result_serializes() {
        let result = PrinterOpResult::ok(InstallDetail {
            printer_name: "HP M428fdw".to_string(),
            driver_name: "HP LaserJet Pro MFP M428f PCL-6 (V4)".to_string(),
            port_name: "IP_192.168.1.50".to_string(),
            warning: None,
        });
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("HP M428fdw"));
    }

    #[test]
    fn printer_op_result_err_has_message() {
        let result = PrinterOpResult::err("boom");
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("boom"));
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("boom"));
    }

    #[test]
    fn printer_op_result_detail_roundtrips() {
        let detail = InstallDetail {
            printer_name: "Name".to_string(),
            driver_name: "Drv".to_string(),
            port_name: "Port".to_string(),
            warning: Some("ipp fallback".to_string()),
        };
        let result = PrinterOpResult::ok(detail);
        let back = result.detail_as::<InstallDetail>().unwrap();
        assert_eq!(back.printer_name, "Name");
        assert_eq!(back.warning.as_deref(), Some("ipp fallback"));
    }

    #[test]
    fn printer_with_ip_serializes() {
        let printer = Printer {
            ip: Some(std::net::Ipv4Addr::new(192, 168, 1, 50)),
            model: Some("HP LaserJet Pro".to_string()),
            serial: None,
            status: PrinterStatus::Ready,
            discovery_methods: vec![DiscoveryMethod::PortScan, DiscoveryMethod::Ipp],
            ports: vec![9100, 631],
            source: PrinterSource::Network,
            local_name: None,
        };
        let json = serde_json::to_string(&printer).unwrap();
        assert!(json.contains("192.168.1.50"));
        assert!(json.contains("port_scan"));
    }

    #[test]
    fn usb_printer_has_no_ip() {
        let printer = Printer {
            ip: None,
            model: Some("HP OfficeJet".to_string()),
            serial: None,
            status: PrinterStatus::Ready,
            discovery_methods: vec![DiscoveryMethod::Local],
            ports: vec![],
            source: PrinterSource::Usb,
            local_name: Some("HP OfficeJet Pro 9010".to_string()),
        };
        assert_eq!(printer.display_ip(), "USB");
        let json = serde_json::to_string(&printer).unwrap();
        assert!(json.contains("\"ip\":null"));
    }

    #[test]
    fn display_ip_returns_ip_string() {
        let printer = Printer {
            ip: Some(std::net::Ipv4Addr::new(10, 0, 0, 5)),
            model: None,
            serial: None,
            status: PrinterStatus::Unknown,
            discovery_methods: vec![DiscoveryMethod::PortScan],
            ports: vec![9100],
            source: PrinterSource::Network,
            local_name: None,
        };
        assert_eq!(printer.display_ip(), "10.0.0.5");
    }
}

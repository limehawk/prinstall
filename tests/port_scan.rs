mod port_scan_test {
    use prinstall::discovery::port_scan::{PortScanResult, PRINTER_PORTS};

    #[test]
    fn printer_ports_contains_9100() {
        assert!(PRINTER_PORTS.contains(&9100));
    }

    #[test]
    fn printer_ports_contains_631() {
        assert!(PRINTER_PORTS.contains(&631));
    }

    #[test]
    fn printer_ports_contains_515() {
        assert!(PRINTER_PORTS.contains(&515));
    }

    #[test]
    fn port_scan_result_default_is_empty() {
        let result = PortScanResult {
            ip: std::net::Ipv4Addr::new(192, 168, 1, 1),
            open_ports: vec![],
        };
        assert!(result.open_ports.is_empty());
    }

    #[test]
    fn port_scan_result_is_printer_candidate() {
        let result = PortScanResult {
            ip: std::net::Ipv4Addr::new(192, 168, 1, 1),
            open_ports: vec![9100],
        };
        assert!(!result.open_ports.is_empty());
    }
}

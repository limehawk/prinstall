mod local_enum_test {
    use prinstall::discovery::local::parse_get_printer_output;
    use prinstall::models::*;

    #[test]
    fn parses_usb_printer() {
        let output = "Name: HP OfficeJet Pro 9010\nDriverName: HP OfficeJet Pro 9010\nPortName: USB001\nShared: False\n---";
        let printers = parse_get_printer_output(output);
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].source, PrinterSource::Usb);
        assert!(printers[0].ip.is_none());
        assert_eq!(printers[0].local_name.as_deref(), Some("HP OfficeJet Pro 9010"));
    }

    #[test]
    fn parses_network_printer_with_ip_port() {
        let output = "Name: Front Desk\nDriverName: HP LaserJet\nPortName: IP_192.168.1.50\nShared: False\n---";
        let printers = parse_get_printer_output(output);
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].ip, Some(std::net::Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(printers[0].source, PrinterSource::Installed);
    }

    #[test]
    fn parses_multiple_printers() {
        let output = "Name: Printer1\nDriverName: Driver1\nPortName: USB001\nShared: False\n---\nName: Printer2\nDriverName: Driver2\nPortName: IP_10.0.0.5\nShared: False\n---";
        let printers = parse_get_printer_output(output);
        assert_eq!(printers.len(), 2);
    }

    #[test]
    fn empty_output_returns_empty() {
        let printers = parse_get_printer_output("");
        assert!(printers.is_empty());
    }

    #[test]
    fn extracts_ip_from_port_name_formats() {
        use prinstall::discovery::local::extract_ip_from_port_name;
        assert_eq!(extract_ip_from_port_name("IP_192.168.1.50"), Some(std::net::Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(extract_ip_from_port_name("TCPMON:192.168.1.50"), Some(std::net::Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(extract_ip_from_port_name("USB001"), None);
        assert_eq!(extract_ip_from_port_name("WSD-12345"), None);
    }
}

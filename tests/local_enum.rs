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

    #[test]
    fn parses_rich_output_with_shared_default_status() {
        let output = "\
Name: Front Desk
DriverName: HP Universal Printing PCL 6
PortName: IP_10.0.0.5
Shared: True
Default: True
Status: 3
---
Name: Microsoft Print to PDF
DriverName: Microsoft Print To PDF
PortName: PORTPROMPT:
Shared: False
Default: False
Status: 3
---";
        let printers = parse_get_printer_output(output);
        assert_eq!(printers.len(), 2);

        let front_desk = &printers[0];
        assert_eq!(front_desk.local_name.as_deref(), Some("Front Desk"));
        assert_eq!(front_desk.port_name.as_deref(), Some("IP_10.0.0.5"));
        assert_eq!(front_desk.driver_name.as_deref(), Some("HP Universal Printing PCL 6"));
        assert_eq!(front_desk.shared, Some(true));
        assert_eq!(front_desk.is_default, Some(true));

        let pdf = &printers[1];
        assert_eq!(pdf.port_name.as_deref(), Some("PORTPROMPT:"));
        assert_eq!(pdf.shared, Some(false));
        assert_eq!(pdf.is_default, Some(false));
    }

    #[test]
    fn status_mapping_tolerates_numeric_and_label_forms() {
        use prinstall::discovery::local::map_win32_printer_status;
        use prinstall::models::PrinterStatus;
        assert!(matches!(map_win32_printer_status("3"), PrinterStatus::Ready));
        assert!(matches!(map_win32_printer_status("Idle"), PrinterStatus::Ready));
        assert!(matches!(map_win32_printer_status("Normal"), PrinterStatus::Ready));
        assert!(matches!(map_win32_printer_status("7"), PrinterStatus::Offline));
        assert!(matches!(map_win32_printer_status("Offline"), PrinterStatus::Offline));
        assert!(matches!(map_win32_printer_status("6"), PrinterStatus::Error));
        assert!(matches!(map_win32_printer_status("Stopped"), PrinterStatus::Error));
    }

    #[test]
    fn legacy_three_field_format_still_parses() {
        // Earlier 0.3.x wrappers only emitted Name/Driver/Port — no Shared,
        // no Default, no Status. Make sure we still accept that shape.
        let output = "Name: Old Printer\nDriverName: Legacy\nPortName: USB001\n---";
        let printers = parse_get_printer_output(output);
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].shared, None);
        assert_eq!(printers[0].is_default, None);
        assert_eq!(printers[0].driver_name.as_deref(), Some("Legacy"));
    }
}

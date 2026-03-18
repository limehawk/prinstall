mod ipp_test {
    use prinstall::discovery::ipp;

    #[test]
    fn build_get_printer_attributes_request_is_valid() {
        let request = ipp::build_get_printer_attributes("192.168.1.10");
        assert_eq!(request[0], 2); // IPP major version
        assert_eq!(request[1], 0); // IPP minor version
        assert_eq!(request[2], 0x00); // Operation high byte
        assert_eq!(request[3], 0x0B); // Get-Printer-Attributes
        assert_eq!(request[4..8], [0, 0, 0, 1]); // Request ID = 1
        let as_str = String::from_utf8_lossy(&request);
        assert!(as_str.contains("ipp://192.168.1.10"));
    }

    #[test]
    fn parse_ipp_response_extracts_model() {
        let mut response = Vec::new();
        // Version 2.0, status successful-ok, request-id 1
        response.extend_from_slice(&[2, 0, 0x00, 0x00, 0, 0, 0, 1]);
        // Operation attributes tag (0x01)
        response.push(0x01);
        // charset attribute (required)
        response.push(0x47);
        let name = b"attributes-charset";
        response.extend_from_slice(&(name.len() as u16).to_be_bytes());
        response.extend_from_slice(name);
        let val = b"utf-8";
        response.extend_from_slice(&(val.len() as u16).to_be_bytes());
        response.extend_from_slice(val);
        // Printer attributes tag (0x04)
        response.push(0x04);
        // printer-make-and-model (textWithoutLanguage, tag 0x41)
        response.push(0x41);
        let name2 = b"printer-make-and-model";
        response.extend_from_slice(&(name2.len() as u16).to_be_bytes());
        response.extend_from_slice(name2);
        let val2 = b"HP LaserJet Pro MFP M428fdw";
        response.extend_from_slice(&(val2.len() as u16).to_be_bytes());
        response.extend_from_slice(val2);
        // End-of-attributes tag
        response.push(0x03);

        let model = ipp::parse_printer_make_and_model(&response);
        assert_eq!(model, Some("HP LaserJet Pro MFP M428fdw".to_string()));
    }

    #[test]
    fn parse_ipp_response_returns_none_for_empty() {
        assert_eq!(ipp::parse_printer_make_and_model(&[]), None);
    }

    #[test]
    fn parse_ipp_response_returns_none_for_no_model_attribute() {
        let mut response = Vec::new();
        response.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 1]);
        response.push(0x01);
        response.push(0x47);
        let name = b"attributes-charset";
        response.extend_from_slice(&(name.len() as u16).to_be_bytes());
        response.extend_from_slice(name);
        let val = b"utf-8";
        response.extend_from_slice(&(val.len() as u16).to_be_bytes());
        response.extend_from_slice(val);
        response.push(0x03);

        assert_eq!(ipp::parse_printer_make_and_model(&response), None);
    }
}

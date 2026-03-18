mod subnet_parse_test {
    #[test]
    fn parses_valid_cidr_24() {
        let hosts = prinstall::discovery::subnet::parse_cidr("192.168.1.0/24").unwrap();
        assert_eq!(hosts.len(), 254); // excludes .0 and .255
        assert_eq!(hosts[0].to_string(), "192.168.1.1");
        assert_eq!(hosts[253].to_string(), "192.168.1.254");
    }

    #[test]
    fn parses_valid_cidr_28() {
        let hosts = prinstall::discovery::subnet::parse_cidr("10.0.0.0/28").unwrap();
        assert_eq!(hosts.len(), 14); // excludes network and broadcast
    }

    #[test]
    fn rejects_invalid_cidr() {
        let result = prinstall::discovery::subnet::parse_cidr("not-a-cidr");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_prefix() {
        let result = prinstall::discovery::subnet::parse_cidr("192.168.1.0");
        assert!(result.is_err());
    }

    #[test]
    fn subnet_too_large_without_force() {
        let result = prinstall::discovery::subnet::validate_subnet_size(
            "192.168.0.0/16", false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn subnet_large_allowed_with_force() {
        let result = prinstall::discovery::subnet::validate_subnet_size(
            "192.168.0.0/16", true,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn slash_24_passes_size_check() {
        let result = prinstall::discovery::subnet::validate_subnet_size(
            "192.168.1.0/24", false,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn parse_netip_output_extracts_cidr() {
        use prinstall::discovery::subnet::parse_auto_detect_output;
        let output = "192.168.1.100/24";
        let result = parse_auto_detect_output(output);
        assert_eq!(result, Some("192.168.1.0/24".to_string()));
    }

    #[test]
    fn parse_netip_output_handles_multiple_lines() {
        use prinstall::discovery::subnet::parse_auto_detect_output;
        let output = "169.254.1.1/16\n192.168.1.100/24";
        let result = parse_auto_detect_output(output);
        assert_eq!(result, Some("192.168.1.0/24".to_string()));
    }

    #[test]
    fn parse_netip_output_empty() {
        use prinstall::discovery::subnet::parse_auto_detect_output;
        assert_eq!(parse_auto_detect_output(""), None);
    }
}

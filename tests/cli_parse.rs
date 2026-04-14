use clap::Parser;

mod cli_parse_test {
    use super::*;

    #[test]
    fn scan_no_args_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan"]);
        assert!(matches!(cli.command, Some(prinstall::cli::Commands::Scan { subnet: None, .. })));
    }

    #[test]
    fn scan_with_subnet_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan", "192.168.1.0/24"]);
        match cli.command {
            Some(prinstall::cli::Commands::Scan { subnet: Some(s), .. }) => {
                assert_eq!(s, "192.168.1.0/24");
            }
            _ => panic!("expected Scan with subnet"),
        }
    }

    #[test]
    fn id_requires_ip() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "id", "192.168.1.100"]);
        match cli.command {
            Some(prinstall::cli::Commands::Id { ip }) => {
                assert_eq!(ip, "192.168.1.100");
            }
            _ => panic!("expected Id"),
        }
    }

    #[test]
    fn drivers_requires_ip() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "drivers", "192.168.1.100"]);
        match cli.command {
            Some(prinstall::cli::Commands::Drivers { ip, model }) => {
                assert_eq!(ip, "192.168.1.100");
                assert!(model.is_none());
            }
            _ => panic!("expected Drivers"),
        }
    }

    #[test]
    fn add_with_all_flags() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "add", "192.168.1.100",
            "--driver", "HP Universal Print Driver PCL6",
            "--name", "Front Desk Printer",
            "--model", "HP LaserJet Pro MFP M428fdw",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Add { target, driver, name, model, usb, .. }) => {
                assert_eq!(target, "192.168.1.100");
                assert_eq!(driver.unwrap(), "HP Universal Print Driver PCL6");
                assert_eq!(name.unwrap(), "Front Desk Printer");
                assert_eq!(model.unwrap(), "HP LaserJet Pro MFP M428fdw");
                assert!(!usb);
            }
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn global_flags_parse() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "--json", "--verbose", "--community", "private",
            "scan",
        ]);
        assert!(cli.json);
        assert!(cli.verbose);
        assert_eq!(cli.community, "private");
    }

    #[test]
    fn no_subcommand_gives_none() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn scan_with_method_flag() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan", "--method", "snmp"]);
        match cli.command {
            Some(prinstall::cli::Commands::Scan { method, .. }) => {
                assert_eq!(method, Some("snmp".to_string()));
            }
            _ => panic!("expected Scan"),
        }
    }

    #[test]
    fn scan_with_timeout_flag() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan", "--timeout", "200"]);
        match cli.command {
            Some(prinstall::cli::Commands::Scan { timeout, .. }) => {
                assert_eq!(timeout, Some(200));
            }
            _ => panic!("expected Scan"),
        }
    }

    #[test]
    fn list_command_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "list"]);
        assert!(matches!(cli.command, Some(prinstall::cli::Commands::List)));
    }

    #[test]
    fn add_with_usb_flag() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "add", "192.168.1.100", "--usb"]);
        match cli.command {
            Some(prinstall::cli::Commands::Add { usb, .. }) => {
                assert!(usb);
            }
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn scan_accepts_network_only_flag() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan", "--network-only"]);
        match cli.command {
            Some(prinstall::cli::Commands::Scan { network_only, usb_only, .. }) => {
                assert!(network_only);
                assert!(!usb_only);
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn scan_accepts_usb_only_flag() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan", "--usb-only"]);
        match cli.command {
            Some(prinstall::cli::Commands::Scan { network_only, usb_only, .. }) => {
                assert!(!network_only);
                assert!(usb_only);
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn scan_rejects_both_only_flags() {
        let result = prinstall::cli::Cli::try_parse_from(["prinstall", "scan", "--network-only", "--usb-only"]);
        assert!(result.is_err(), "expected conflict error");
    }
}

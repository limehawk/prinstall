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

    #[test]
    fn add_accepts_no_verify_flag() {
        let cli = prinstall::cli::Cli::try_parse_from([
            "prinstall", "add", "192.168.1.100", "--no-verify",
        ])
        .unwrap();
        match cli.command {
            Some(prinstall::cli::Commands::Add { no_verify, .. }) => assert!(no_verify),
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn add_no_verify_defaults_to_false() {
        let cli = prinstall::cli::Cli::try_parse_from([
            "prinstall", "add", "192.168.1.100",
        ])
        .unwrap();
        match cli.command {
            Some(prinstall::cli::Commands::Add { no_verify, .. }) => assert!(!no_verify),
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn driver_add_parses_with_path() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "driver", "add", "C:\\test\\driver",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => match action {
                prinstall::cli::DriverAction::Add { target, driver, no_verify } => {
                    assert_eq!(target, "C:\\test\\driver");
                    assert!(driver.is_none());
                    assert!(!no_verify);
                }
                _ => panic!("expected DriverAction::Add"),
            },
            _ => panic!("expected Driver::Add"),
        }
    }

    #[test]
    fn driver_add_parses_with_no_verify() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "driver", "add", "C:\\test", "--no-verify",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => match action {
                prinstall::cli::DriverAction::Add { no_verify, .. } => {
                    assert!(no_verify);
                }
                _ => panic!("expected DriverAction::Add"),
            },
            _ => panic!("expected Driver::Add"),
        }
    }

    #[test]
    fn driver_add_parses_with_model_and_explicit_driver() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "driver", "add", "hp 1320",
            "--driver", "HP Universal Print Driver PCL6",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => match action {
                prinstall::cli::DriverAction::Add { target, driver, .. } => {
                    assert_eq!(target, "hp 1320");
                    assert_eq!(driver.as_deref(), Some("HP Universal Print Driver PCL6"));
                }
                _ => panic!("expected DriverAction::Add"),
            },
            _ => panic!("expected Driver::Add"),
        }
    }

    #[test]
    fn version_subcommand_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "version"]);
        assert!(matches!(cli.command, Some(prinstall::cli::Commands::Version)));
    }

    #[test]
    fn driver_remove_parses() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "driver", "remove", "HP Universal Print Driver PCL6",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => match action {
                prinstall::cli::DriverAction::Remove { target, force } => {
                    assert_eq!(target, "HP Universal Print Driver PCL6");
                    assert!(!force);
                }
                _ => panic!("expected DriverAction::Remove"),
            },
            _ => panic!("expected Driver"),
        }
    }

    #[test]
    fn driver_remove_with_force_parses() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "driver", "remove", "hp 1320", "--force",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => match action {
                prinstall::cli::DriverAction::Remove { target, force } => {
                    assert_eq!(target, "hp 1320");
                    assert!(force);
                }
                _ => panic!("expected DriverAction::Remove"),
            },
            _ => panic!("expected Driver"),
        }
    }

    #[test]
    fn driver_list_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "driver", "list"]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => {
                assert!(matches!(action, prinstall::cli::DriverAction::List));
            }
            _ => panic!("expected Driver"),
        }
    }

    #[test]
    fn driver_show_parses_with_ip() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "driver", "show", "192.168.1.100",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Driver { action }) => match action {
                prinstall::cli::DriverAction::Show { ip, model } => {
                    assert_eq!(ip, "192.168.1.100");
                    assert!(model.is_none());
                }
                _ => panic!("expected DriverAction::Show"),
            },
            _ => panic!("expected Driver"),
        }
    }
}

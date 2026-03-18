use clap::Parser;

mod cli_parse_test {
    use super::*;

    #[test]
    fn scan_no_args_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan"]);
        assert!(matches!(cli.command, Some(prinstall::cli::Commands::Scan { subnet: None })));
    }

    #[test]
    fn scan_with_subnet_parses() {
        let cli = prinstall::cli::Cli::parse_from(["prinstall", "scan", "192.168.1.0/24"]);
        match cli.command {
            Some(prinstall::cli::Commands::Scan { subnet: Some(s) }) => {
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
    fn install_with_all_flags() {
        let cli = prinstall::cli::Cli::parse_from([
            "prinstall", "install", "192.168.1.100",
            "--driver", "HP Universal Print Driver PCL6",
            "--name", "Front Desk Printer",
            "--model", "HP LaserJet Pro MFP M428fdw",
        ]);
        match cli.command {
            Some(prinstall::cli::Commands::Install { ip, driver, name, model }) => {
                assert_eq!(ip, "192.168.1.100");
                assert_eq!(driver.unwrap(), "HP Universal Print Driver PCL6");
                assert_eq!(name.unwrap(), "Front Desk Printer");
                assert_eq!(model.unwrap(), "HP LaserJet Pro MFP M428fdw");
            }
            _ => panic!("expected Install"),
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
}

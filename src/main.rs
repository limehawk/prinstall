use clap::Parser;
use std::io::IsTerminal;

use prinstall::core::executor::RealExecutor;
use prinstall::{cli, commands, discovery, output, privilege, tui};

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();

    // Configure ANSI color output before any formatter runs.
    // Respects --json, NO_COLOR, and whether stdout is a terminal.
    output::set_color_enabled(output::detect_color_mode(cli.json));

    match cli.command {
        Some(ref cmd) => run_cli(cmd, &cli).await,
        None => {
            if std::io::stdout().is_terminal() {
                run_tui(&cli).await;
            } else {
                let _ = cli::Cli::parse_from::<[&str; 2], &str>(["prinstall", "--help"]);
            }
        }
    }
}

async fn run_cli(cmd: &cli::Commands, cli: &cli::Cli) {
    // Privilege check for commands that mutate Windows state
    if matches!(cmd, cli::Commands::Add { .. } | cli::Commands::Remove { .. })
        && !privilege::is_elevated()
    {
        eprintln!("Error: Administrator privileges required for this command.");
        eprintln!("Run this command from an elevated terminal or RMM shell.");
        std::process::exit(1);
    }

    match cmd {
        cli::Commands::Scan { subnet, method, timeout } => {
            cmd_scan(subnet.clone(), method.as_deref(), *timeout, cli).await
        }
        cli::Commands::Id { ip } => cmd_id(ip, cli).await,
        cli::Commands::Drivers { ip, model } => cmd_drivers(ip, model.as_deref(), cli).await,
        cli::Commands::Add { target, driver, name, model, usb } => {
            cmd_add(target, driver.as_deref(), name.as_deref(), model.as_deref(), *usb, cli).await;
        }
        cli::Commands::Remove { target, keep_driver, keep_port } => {
            cmd_remove(target, *keep_driver, *keep_port, cli).await;
        }
        cli::Commands::List => cmd_list(cli).await,
    }
}

async fn cmd_remove(
    target: &str,
    keep_driver: bool,
    keep_port: bool,
    cli: &cli::Cli,
) {
    let executor = RealExecutor::new(cli.verbose);
    let result = commands::remove::run(
        &executor,
        commands::remove::RemoveArgs {
            target,
            keep_driver,
            keep_port,
            verbose: cli.verbose,
        },
    )
    .await;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
    } else {
        println!("{}", output::format_remove_result(&result));
    }

    if !result.success {
        std::process::exit(1);
    }
}

async fn cmd_add(
    target: &str,
    driver_override: Option<&str>,
    name_override: Option<&str>,
    model_override: Option<&str>,
    usb: bool,
    cli: &cli::Cli,
) {
    let result = commands::add::run(commands::add::AddArgs {
        target,
        driver_override,
        name_override,
        model_override,
        usb,
        community: &cli.community,
        verbose: cli.verbose,
    })
    .await;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
    } else {
        println!("{}", output::format_install_result(&result));
    }

    if !result.success {
        std::process::exit(1);
    }
}

async fn cmd_scan(
    subnet: Option<String>,
    method: Option<&str>,
    timeout_ms: Option<u64>,
    cli: &cli::Cli,
) {
    let cidr = match subnet {
        Some(s) => s,
        None => {
            eprintln!("Auto-detecting local subnet is not yet implemented.");
            eprintln!("Please provide a subnet: prinstall scan 192.168.1.0/24");
            std::process::exit(1);
        }
    };

    if let Err(e) = discovery::subnet::validate_subnet_size(&cidr, cli.force) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }

    let hosts = match discovery::subnet::parse_cidr(&cidr) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    if cli.verbose {
        eprintln!("[scan] Scanning {} hosts on {cidr}...", hosts.len());
    }

    let scan_method = match method {
        Some("snmp") => discovery::ScanMethod::Snmp,
        Some("port") => discovery::ScanMethod::Port,
        _ => discovery::ScanMethod::All,
    };

    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(100));

    let printers = discovery::scan_subnet(hosts, &cli.community, &scan_method, timeout, cli.verbose).await;

    if cli.json {
        println!("{}", output::format_scan_results_json(&printers));
    } else if printers.is_empty() {
        println!("{}", output::format_scan_guidance(&cidr, 0, 0));
    } else {
        println!("{}", output::format_scan_results(&printers));
    }
}

async fn cmd_list(cli: &cli::Cli) {
    let printers = discovery::local::list_local_printers(cli.verbose);
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&printers).unwrap_or_else(|_| "[]".to_string()));
    } else if printers.is_empty() {
        println!("No locally installed printers found.");
    } else {
        println!("{}", output::format_scan_results(&printers));
    }
}

async fn cmd_id(ip: &str, cli: &cli::Cli) {
    if cli.verbose {
        eprintln!("[id] Querying {ip} via SNMP (community: {})...", cli.community);
    }

    let addr: std::net::Ipv4Addr = match ip.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: invalid IP address '{ip}': {e}");
            std::process::exit(1);
        }
    };

    match discovery::snmp::identify_printer(addr, &cli.community, cli.verbose).await {
        Some(printer) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&printer).unwrap());
            } else {
                println!("{}", output::format_printer_id(&printer));
            }
        }
        None => {
            println!("{}", output::format_snmp_failure_guidance(ip));
            std::process::exit(1);
        }
    }
}

async fn cmd_drivers(ip: &str, model_override: Option<&str>, cli: &cli::Cli) {
    if cli.verbose {
        eprintln!("[drivers] Finding drivers for printer at {ip}");
    }

    let executor = RealExecutor::new(cli.verbose);
    let results = commands::drivers::run(
        &executor,
        commands::drivers::DriversArgs {
            ip,
            model_override,
            community: &cli.community,
            verbose: cli.verbose,
        },
    )
    .await;

    if cli.json {
        println!("{}", output::format_driver_results_json(&results));
    } else {
        println!("{}", output::format_driver_results(&results));
    }
}

async fn run_tui(cli: &cli::Cli) {
    let mut terminal = ratatui::init();
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture
    ).ok();

    let mut app = tui::App::new(cli.community.clone(), cli.subnet.clone());
    let result = app.run(&mut terminal).await;

    ratatui::restore();
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture
    ).ok();

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

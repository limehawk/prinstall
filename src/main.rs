use clap::Parser;
use std::io::IsTerminal;

use prinstall::{cli, discovery, drivers, installer, output, privilege, tui};

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();

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
    // Privilege check for install command
    if matches!(cmd, cli::Commands::Install { .. }) && !privilege::is_elevated() {
        eprintln!("Error: Administrator privileges required for installation.");
        eprintln!("Run this command from an elevated terminal or RMM shell.");
        std::process::exit(1);
    }

    match cmd {
        cli::Commands::Scan { subnet } => cmd_scan(subnet.clone(), cli).await,
        cli::Commands::Id { ip } => cmd_id(ip, cli).await,
        cli::Commands::Drivers { ip, model } => cmd_drivers(ip, model.as_deref(), cli).await,
        cli::Commands::Install { ip, driver, name, model } => {
            cmd_install(ip, driver.as_deref(), name.as_deref(), model.as_deref(), cli).await;
        }
    }
}

async fn cmd_scan(subnet: Option<String>, cli: &cli::Cli) {
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

    let printers = discovery::scan_subnet(
        hosts,
        &cli.community,
        &discovery::ScanMethod::All,
        std::time::Duration::from_millis(100),
        cli.verbose,
    ).await;

    if cli.json {
        println!("{}", output::format_scan_results_json(&printers));
    } else {
        if printers.is_empty() {
            println!("{}", output::format_snmp_failure_guidance(&cidr));
        } else {
            println!("{}", output::format_scan_results(&printers));
        }
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
    // Get model via --model override or SNMP
    let model = if let Some(m) = model_override {
        m.to_string()
    } else {
        resolve_model(ip, cli).await
    };

    if cli.verbose {
        eprintln!("[drivers] Finding drivers for: {model}");
    }

    let local_drivers = drivers::local_store::list_drivers(cli.verbose);
    let results = drivers::matcher::match_drivers(&model, &local_drivers);

    if cli.json {
        println!("{}", output::format_driver_results_json(&results));
    } else {
        println!("{}", output::format_driver_results(&results));
    }
}

async fn cmd_install(
    ip: &str,
    driver_override: Option<&str>,
    name_override: Option<&str>,
    model_override: Option<&str>,
    cli: &cli::Cli,
) {
    // Pre-install reachability check
    if cli.verbose {
        eprintln!("[install] Checking reachability of {ip}...");
    }

    let addr: std::net::Ipv4Addr = match ip.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: invalid IP address '{ip}': {e}");
            std::process::exit(1);
        }
    };

    // Resolve model
    let model = if let Some(m) = model_override {
        m.to_string()
    } else {
        match discovery::snmp::identify_printer(addr, &cli.community, cli.verbose).await {
            Some(p) => p.model.unwrap_or_else(|| {
                eprintln!("{}", output::format_snmp_failure_guidance(ip));
                std::process::exit(1);
            }),
            None => {
                eprintln!("{}", output::format_snmp_failure_guidance(ip));
                std::process::exit(1);
            }
        }
    };

    // Resolve driver
    let driver_name = if let Some(d) = driver_override {
        d.to_string()
    } else {
        let local_drivers = drivers::local_store::list_drivers(cli.verbose);
        let results = drivers::matcher::match_drivers(&model, &local_drivers);

        // Auto-pick: first matched driver, or first universal
        if let Some(best) = results.matched.first().or(results.universal.first()) {
            if cli.verbose {
                eprintln!("[install] Auto-selected driver: {}", best.name);
            }
            best.name.clone()
        } else {
            eprintln!("No drivers found for '{}'. Try --driver to specify one manually.", model);
            std::process::exit(1);
        }
    };

    let printer_name = name_override.unwrap_or(&model).to_string();

    if cli.verbose {
        eprintln!("[install] Installing: printer='{}', driver='{}', ip={}", printer_name, driver_name, ip);
    }

    // Check if driver is already staged locally; if not, try to download it
    let local_drivers = drivers::local_store::list_drivers(cli.verbose);
    let driver_staged = local_drivers.iter().any(|d| d == &driver_name);

    if !driver_staged {
        if cli.verbose {
            eprintln!("[install] Driver not in local store, checking manufacturer downloads...");
        }
        // Look up the driver in the manifest for download info
        let manifest = drivers::manifest::Manifest::load_embedded();
        if let Some(mfr) = manifest.find_manufacturer(&model)
            && let Some(ud) = mfr.universal_drivers.iter().find(|u| u.name == driver_name)
        {
            match drivers::downloader::download_and_stage(ud, cli.verbose).await {
                Ok(extract_dir) => {
                    // Stage INF files via pnputil
                    let infs = drivers::downloader::find_inf_files(&extract_dir);
                    for inf in &infs {
                        if cli.verbose {
                            eprintln!("[install] Staging driver: {}", inf.display());
                        }
                        let stage_result = installer::powershell::stage_driver_inf(
                            inf.to_str().unwrap_or_default(),
                            cli.verbose,
                        );
                        if !stage_result.success {
                            eprintln!("Warning: failed to stage {}: {}", inf.display(), stage_result.stderr);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: driver download failed: {e}");
                    eprintln!("Proceeding anyway — driver may already be available via Windows Update.");
                }
            }
        }
    }

    let result = installer::install_printer(ip, &driver_name, &printer_name, &model, cli.verbose);

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!("{}", output::format_install_result(&result));
    }

    if !result.success {
        std::process::exit(1);
    }
}

/// Resolve printer model: use --model if provided, otherwise query SNMP.
async fn resolve_model(ip: &str, cli: &cli::Cli) -> String {
    // Check for global --model override (via install subcommand, not available here directly)
    // For drivers/id commands, we always use SNMP
    let addr: std::net::Ipv4Addr = match ip.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: invalid IP address '{ip}': {e}");
            std::process::exit(1);
        }
    };

    match discovery::snmp::identify_printer(addr, &cli.community, cli.verbose).await {
        Some(p) => p.model.unwrap_or_else(|| {
            eprintln!("{}", output::format_snmp_failure_guidance(ip));
            std::process::exit(1);
        }),
        None => {
            eprintln!("{}", output::format_snmp_failure_guidance(ip));
            std::process::exit(1);
        }
    }
}

async fn run_tui(cli: &cli::Cli) {
    let mut terminal = ratatui::init();
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture
    ).ok();

    let mut app = tui::App::new(cli.community.clone());
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

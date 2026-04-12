use clap::Parser;
use std::io::IsTerminal;

use prinstall::core::executor::RealExecutor;
use prinstall::{cli, commands, discovery, output, privilege};

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
        cli::Commands::Add { target, driver, name, model, usb, no_catalog, .. } => {
            #[cfg(feature = "sdi")]
            let (no_sdi, sdi_fetch) = match cmd {
                cli::Commands::Add { no_sdi, sdi_fetch, .. } => (*no_sdi, *sdi_fetch),
                _ => unreachable!(),
            };
            #[cfg(not(feature = "sdi"))]
            let (no_sdi, sdi_fetch) = (true, false);
            cmd_add(target, driver.as_deref(), name.as_deref(), model.as_deref(), *usb, no_sdi, *no_catalog, sdi_fetch, cli).await;
        }
        cli::Commands::Remove { target, keep_driver, keep_port } => {
            cmd_remove(target, *keep_driver, *keep_port, cli).await;
        }
        cli::Commands::List => cmd_list(cli).await,
        #[cfg(feature = "sdi")]
        cli::Commands::Sdi(action) => cmd_sdi(action, cli).await,
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
    no_sdi: bool,
    no_catalog: bool,
    sdi_fetch: bool,
    cli: &cli::Cli,
) {
    let result = commands::add::run(commands::add::AddArgs {
        target,
        driver_override,
        name_override,
        model_override,
        usb,
        force: cli.force,
        no_sdi,
        no_catalog,
        sdi_fetch,
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

#[cfg(feature = "sdi")]
async fn cmd_sdi(action: &cli::SdiAction, cli: &cli::Cli) {
    match action {
        cli::SdiAction::Status => commands::sdi::status(cli.verbose),
        cli::SdiAction::Refresh => commands::sdi::refresh(cli.verbose).await,
        cli::SdiAction::List => commands::sdi::list(cli.verbose),
        cli::SdiAction::Prefetch => commands::sdi::prefetch(cli.verbose).await,
        cli::SdiAction::Clean => commands::sdi::clean(cli.verbose),
    }
}

async fn cmd_scan(
    subnet: Option<String>,
    method: Option<&str>,
    timeout_ms: Option<u64>,
    cli: &cli::Cli,
) {
    let scan_method = match method {
        Some("snmp") => discovery::ScanMethod::Snmp,
        Some("port") => discovery::ScanMethod::Port,
        Some("mdns") => discovery::ScanMethod::Mdns,
        _ => discovery::ScanMethod::All,
    };

    // mDNS is multicast — it doesn't need a subnet argument at all.
    // `--method mdns` skips host enumeration entirely.
    let mdns_only = matches!(scan_method, discovery::ScanMethod::Mdns);

    let (cidr, hosts) = if mdns_only {
        if cli.verbose {
            eprintln!("[scan] mDNS-only scan (no subnet target needed)");
        }
        (String::from("(mdns)"), Vec::new())
    } else {
        let raw_cidr = match subnet {
            Some(s) => s,
            None => {
                if cli.verbose {
                    eprintln!("[scan] No subnet arg — auto-detecting from local NIC...");
                }
                match discovery::subnet::auto_detect_subnet(cli.verbose) {
                    Some(detected) => {
                        if cli.verbose {
                            eprintln!("[scan] Auto-detected subnet: {detected}");
                        } else {
                            eprintln!("Auto-detected subnet: {detected}");
                        }
                        detected
                    }
                    None => {
                        eprintln!("Could not auto-detect the local subnet.");
                        eprintln!("Please provide a subnet: prinstall scan 192.168.1.0/24");
                        std::process::exit(1);
                    }
                }
            }
        };

        // Normalize so `10.10.20.1/24` becomes `10.10.20.0/24` — the
        // host bits are masked off by the prefix length.
        let cidr = match discovery::subnet::normalize_cidr(&raw_cidr) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {e}");
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

        (cidr, hosts)
    };

    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(500));

    let printers = discovery::scan_subnet(
        hosts,
        &cli.community,
        &scan_method,
        timeout,
        cli.verbose,
    )
    .await;

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
        println!(
            "{}",
            serde_json::to_string_pretty(&printers).unwrap_or_else(|_| "[]".to_string())
        );
    } else {
        println!("{}", output::format_list_results(&printers));
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

/// "Under construction" block screen — temporary stand-in for the real
/// interactive TUI while the TUI layer is being reworked. Takes over the
/// terminal like the real TUI would, shows a yellow construction-tape
/// banner with a pointer at the working CLI subcommands, and exits on
/// any keypress. The `src/tui/` module is intentionally left in place
/// and unused until the rework lands.
async fn run_tui(_cli: &cli::Cli) {
    use crossterm::event::{self, Event, KeyEventKind};

    let mut terminal = ratatui::init();

    let result = (|| -> std::io::Result<()> {
        terminal.draw(draw_under_construction)?;
        loop {
            match event::read()? {
                Event::Key(ke) if ke.kind == KeyEventKind::Press => return Ok(()),
                Event::Resize(_, _) => {
                    terminal.draw(draw_under_construction)?;
                }
                _ => continue,
            }
        }
    })();

    ratatui::restore();

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn draw_under_construction(frame: &mut ratatui::Frame) {
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout, Margin},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, BorderType, Borders, Paragraph},
    };

    let area = frame.area();

    let yellow_bold = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let yellow = Style::default().fg(Color::Yellow);
    let cyan_bold = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // Outer panel — double yellow border with a centered " prinstall " title
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(yellow_bold)
        .title(Line::from(" prinstall ").style(yellow_bold))
        .title_alignment(Alignment::Center);
    frame.render_widget(outer, area);

    let inner = area.inner(Margin {
        horizontal: 3,
        vertical: 2,
    });

    // Vertical split: top tape / headline / description / commands / flex /
    // bottom tape / footer. Length constraints are tuned for an 80×24 shell
    // with ratatui truncating gracefully on anything smaller.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top tape
            Constraint::Length(1), // spacer
            Constraint::Length(3), // headline
            Constraint::Length(1), // spacer
            Constraint::Length(2), // description
            Constraint::Length(1), // spacer
            Constraint::Length(6), // command list
            Constraint::Min(0),    // flex
            Constraint::Length(1), // bottom tape
            Constraint::Length(1), // spacer
            Constraint::Length(1), // footer
        ])
        .split(inner);

    // Construction tape: alternating ▀▄ glyphs stretched across the inner
    // width. Yellow fg makes it read as hazard tape against the terminal bg.
    let tape_len = inner.width as usize;
    let tape: String = "▀▄".chars().cycle().take(tape_len).collect();
    frame.render_widget(
        Paragraph::new(tape.clone()).style(yellow),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(tape).style(yellow), chunks[8]);

    // Headline: highlighter-style black-on-yellow UNDER CONSTRUCTION bar
    let headline_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let headline_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "   UNDER CONSTRUCTION   ",
            headline_style,
        )),
        Line::from(""),
    ];
    frame.render_widget(
        Paragraph::new(headline_lines).alignment(Alignment::Center),
        chunks[2],
    );

    // Description
    let description = Paragraph::new(vec![
        Line::from(Span::styled(
            "The interactive TUI is being reworked and is temporarily offline.",
            yellow_bold,
        )),
        Line::from("Use the CLI subcommands in the meantime:"),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(description, chunks[4]);

    // Command list — fixed-width command column, plain description column
    let cmds: &[(&str, &str)] = &[
        ("prinstall scan",         "Scan a subnet for printers"),
        ("prinstall id <IP>",      "Identify a printer via SNMP"),
        ("prinstall drivers <IP>", "Show matching drivers for a printer"),
        ("prinstall add <IP>",     "Install a network printer"),
        ("prinstall remove <IP>",  "Remove + clean up ports and drivers"),
        ("prinstall list",         "List locally installed printers"),
    ];
    let cmd_lines: Vec<Line> = cmds
        .iter()
        .map(|(cmd, desc)| {
            Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("{:<22}  ", cmd), cyan_bold),
                Span::raw(*desc),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(cmd_lines), chunks[6]);

    // Footer: press any key to exit
    let footer = Paragraph::new(Line::from(vec![
        Span::raw("Press "),
        Span::styled("any key", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to exit"),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(footer, chunks[10]);
}

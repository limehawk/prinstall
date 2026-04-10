use std::io::IsTerminal;
use std::sync::OnceLock;

use crossterm::style::Stylize;

#[allow(unused_imports)]
use crate::models::*;

// ── Color control ────────────────────────────────────────────────────────────

/// Set once at startup by `main()` after inspecting `--json`, `NO_COLOR`,
/// and whether stdout is a real terminal. Formatters read this to decide
/// whether to emit ANSI escape codes.
static COLOR_ENABLED: OnceLock<bool> = OnceLock::new();

/// Auto-detect whether the process should emit colored output.
///
/// Rules (in priority order):
/// 1. `--json` mode: never colorize — JSON consumers would choke on escape codes
/// 2. `NO_COLOR` env var set: never colorize (standard per no-color.org)
/// 3. stdout is not a terminal (pipe, file redirect, RMM capture): never colorize
/// 4. Otherwise: colorize
pub fn detect_color_mode(json: bool) -> bool {
    if json {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

/// Install the color mode for the remainder of the process. Idempotent —
/// subsequent calls are ignored. Call from `main()` once after parsing CLI.
pub fn set_color_enabled(enabled: bool) {
    let _ = COLOR_ENABLED.set(enabled);
}

/// Whether ANSI colors should be emitted. Defaults to `false` if
/// `set_color_enabled` was never called (e.g. during `cargo test`).
fn color_enabled() -> bool {
    *COLOR_ENABLED.get().unwrap_or(&false)
}

// ── Color helpers ────────────────────────────────────────────────────────────
//
// Each helper applies a semantic style (success, warning, error, header, dim,
// accent, badge-by-confidence) and falls back to plain text when color is
// disabled. Semantic names (not color names) so we can retune the palette
// later without touching every callsite.

fn ok(s: &str) -> String {
    if color_enabled() { s.green().bold().to_string() } else { s.to_string() }
}

fn err_text(s: &str) -> String {
    if color_enabled() { s.red().bold().to_string() } else { s.to_string() }
}

fn warn(s: &str) -> String {
    if color_enabled() { s.yellow().bold().to_string() } else { s.to_string() }
}

fn header(s: &str) -> String {
    if color_enabled() { s.cyan().bold().to_string() } else { s.to_string() }
}

fn dim(s: &str) -> String {
    if color_enabled() { s.dark_grey().to_string() } else { s.to_string() }
}

fn label(s: &str) -> String {
    if color_enabled() { s.cyan().to_string() } else { s.to_string() }
}

fn badge_exact(s: &str) -> String {
    if color_enabled() { s.green().bold().to_string() } else { s.to_string() }
}

fn badge_fuzzy(s: &str) -> String {
    if color_enabled() { s.yellow().to_string() } else { s.to_string() }
}

fn status_color(s: &str, status: &PrinterStatus) -> String {
    if !color_enabled() {
        return s.to_string();
    }
    match status {
        PrinterStatus::Ready => s.green().to_string(),
        PrinterStatus::Error => s.red().to_string(),
        PrinterStatus::Offline => s.dark_grey().to_string(),
        PrinterStatus::Unknown => s.to_string(),
    }
}

/// Format scan results as a readable table.
pub fn format_scan_results(printers: &[Printer]) -> String {
    if printers.is_empty() {
        return "No printers found.".to_string();
    }

    let ip_width = printers
        .iter()
        .map(|p| p.display_ip().len())
        .max()
        .unwrap_or(15)
        .max(15);
    let model_width = printers
        .iter()
        .map(|p| p.model.as_deref().unwrap_or("Unknown").len())
        .max()
        .unwrap_or(20)
        .max(20);
    let source_width = "Source".len().max(9);

    let mut out = String::new();
    out.push_str(&format!(
        "\n{:<ip_w$}  {:<model_w$}  {:<src_w$}  {}\n",
        "IP", "Model", "Source", "Status",
        ip_w = ip_width, model_w = model_width, src_w = source_width
    ));
    out.push_str(&format!(
        "{:-<ip_w$}  {:-<model_w$}  {:-<src_w$}  {:-<10}\n",
        "", "", "", "",
        ip_w = ip_width, model_w = model_width, src_w = source_width
    ));

    for p in printers {
        let source_str = match p.source {
            PrinterSource::Network => "Network",
            PrinterSource::Usb => "USB",
            PrinterSource::Installed => "Installed",
        };
        let status_str = p.status.to_string();
        out.push_str(&format!(
            "{:<ip_w$}  {:<model_w$}  {:<src_w$}  {}\n",
            p.display_ip(),
            p.model.as_deref().unwrap_or("Unknown"),
            source_str,
            status_color(&status_str, &p.status),
            ip_w = ip_width,
            model_w = model_width,
            src_w = source_width,
        ));
    }

    out
}

/// Format scan results as JSON.
pub fn format_scan_results_json(printers: &[Printer]) -> String {
    serde_json::to_string_pretty(printers).unwrap_or_else(|_| "[]".to_string())
}

/// Format driver matching results with two sections.
pub fn format_driver_results(results: &DriverResults) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n{} {}\n", label("Printer:"), results.printer_model));

    if results.matched.is_empty() && results.universal.is_empty() {
        out.push_str("\nNo drivers found for this printer.\n");
        return out;
    }

    let mut num = 1;

    if !results.matched.is_empty() {
        out.push_str(&format!(
            "\n{}\n",
            header("── Matched Drivers ──────────────────────────────────────────")
        ));
        for dm in &results.matched {
            let badge = match dm.confidence {
                MatchConfidence::Exact => badge_exact("★ exact"),
                MatchConfidence::Fuzzy => badge_fuzzy("● fuzzy"),
                MatchConfidence::Universal => dim("○"),
            };
            let source_text = match dm.source {
                DriverSource::LocalStore => "[Local Store]",
                DriverSource::Manufacturer => "[Manufacturer]",
            };
            // Score is 0-1000; display as 0-100% for humans.
            let pct = (dm.score / 10).min(100);
            out.push_str(&format!(
                "  #{:<2} {:<45} {:<10} {:>4}%  {}\n",
                num,
                dm.name,
                badge,
                pct,
                dim(source_text)
            ));
            num += 1;
        }
    }

    if !results.universal.is_empty() {
        out.push_str(&format!(
            "\n{}\n",
            header("── Universal Drivers ────────────────────────────────────────")
        ));
        for dm in &results.universal {
            let source_text = match dm.source {
                DriverSource::LocalStore => "[Local Store]",
                DriverSource::Manufacturer => "[Manufacturer]",
            };
            out.push_str(&format!(
                "  #{:<2} {:<45} {:<10} {}\n",
                num,
                dm.name,
                "",
                dim(source_text)
            ));
            num += 1;
        }
    }

    out
}

/// Format driver results as JSON.
pub fn format_driver_results_json(results: &DriverResults) -> String {
    serde_json::to_string_pretty(results).unwrap_or_else(|_| "{}".to_string())
}

/// Format the SNMP failure guidance message.
pub fn format_snmp_failure_guidance(ip: &str) -> String {
    format!(
        "\nCould not identify printer at {ip} via SNMP.\n\n\
         Common causes:\n  \
         • SNMP is disabled on the printer\n  \
         • Non-default community string — try --community <string>\n  \
         • Firewall blocking UDP port 161\n  \
         • Printer is offline or unreachable\n\n\
         Workarounds:\n  \
         • Try a different community string: prinstall id {ip} --community private\n  \
         • Bypass SNMP with manual model: prinstall drivers {ip} --model \"Model Name\"\n  \
         • Check printer web UI for SNMP settings\n"
    )
}

/// Context-aware guidance when scan finds no or few results.
pub fn format_scan_guidance(subnet: &str, candidates: usize, _identified: usize) -> String {
    if candidates == 0 {
        format!(
            "\nNo printers found on {subnet}.\n\n\
             Possible causes:\n  \
             • Wrong subnet — verify with: ipconfig /all\n  \
             • Printers on a different VLAN\n  \
             • Firewall blocking scan ports (9100, 631, 515)\n\n\
             Try:\n  \
             • Different subnet: prinstall scan <subnet>\n  \
             • SNMP-only mode: prinstall scan {subnet} --method snmp\n"
        )
    } else {
        format!(
            "\nFound {candidates} device(s) with printer ports open, \
             but could not identify model for any.\n\n\
             Try:\n  \
             • Specify model manually: prinstall drivers <IP> --model \"Model Name\"\n  \
             • Enable SNMP on the printer via its web UI\n  \
             • Use --verbose for diagnostic details\n"
        )
    }
}

/// Format a single printer identification.
pub fn format_printer_id(printer: &Printer) -> String {
    let mut out = String::new();
    out.push_str(&format!("\nPrinter at {}\n", printer.display_ip()));
    out.push_str(&format!("  Model:  {}\n", printer.model.as_deref().unwrap_or("Unknown")));
    out.push_str(&format!("  Serial: {}\n", printer.serial.as_deref().unwrap_or("N/A")));
    out.push_str(&format!("  Status: {}\n", printer.status));
    if !printer.ports.is_empty() {
        let ports_str: Vec<String> = printer.ports.iter().map(|p| p.to_string()).collect();
        out.push_str(&format!("  Ports:  {}\n", ports_str.join(", ")));
    }
    if !printer.discovery_methods.is_empty() {
        let methods: Vec<&str> = printer.discovery_methods.iter().map(|m| match m {
            crate::models::DiscoveryMethod::PortScan => "Port Scan",
            crate::models::DiscoveryMethod::Ipp => "IPP",
            crate::models::DiscoveryMethod::Snmp => "SNMP",
            crate::models::DiscoveryMethod::Local => "Local",
        }).collect();
        out.push_str(&format!("  Found:  {}\n", methods.join(" + ")));
    }
    if let Some(ref name) = printer.local_name {
        out.push_str(&format!("  Name:   {}\n", name));
    }
    out
}

/// Format the result of an install/add operation for human-readable output.
pub fn format_install_result(result: &PrinterOpResult) -> String {
    if !result.success {
        return format!(
            "\n{}\n  {} {}\n",
            err_text("Printer installation failed."),
            label("Error:"),
            result.error.as_deref().unwrap_or("Unknown error")
        );
    }
    let Some(detail) = result.detail_as::<InstallDetail>() else {
        return format!("\n{}\n", ok("Printer installed successfully."));
    };
    let mut out = format!(
        "\n{}\n  {} {}\n  {} {}\n",
        ok("Printer installed successfully!"),
        label("Name:  "),
        detail.printer_name,
        label("Driver:"),
        detail.driver_name,
    );
    if !detail.port_name.is_empty() {
        out.push_str(&format!("  {} {}\n", label("Port:  "), detail.port_name));
    }
    if let Some(ref warning) = detail.warning {
        out.push_str(&format!("\n  {} {warning}\n", warn("WARNING:")));
    }
    out
}

/// Format the result of a remove operation for human-readable output.
pub fn format_remove_result(result: &PrinterOpResult) -> String {
    if !result.success {
        return format!(
            "\n{}\n  {} {}\n",
            err_text("Printer removal failed."),
            label("Error:"),
            result.error.as_deref().unwrap_or("Unknown error")
        );
    }
    let Some(detail) = result.detail_as::<RemoveDetail>() else {
        return format!("\n{}\n", ok("Printer removed."));
    };
    if detail.already_absent {
        return format!(
            "\n{} '{}' — nothing to remove.\n",
            dim("No printer found matching"),
            detail.printer_name
        );
    }
    let mut out = format!(
        "\n{} {}\n",
        ok("Removed printer:"),
        detail.printer_name
    );
    if detail.port_removed {
        out.push_str(&format!(
            "  {}\n",
            dim("· Port also removed (no other printers were using it)")
        ));
    }
    if detail.driver_removed {
        out.push_str(&format!(
            "  {}\n",
            dim("· Driver also removed from driver store")
        ));
    }
    out
}

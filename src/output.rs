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
///
/// On Windows, additionally kicks the console into VT processing mode so
/// the ANSI escape codes crossterm's `Stylize` trait emits actually render
/// as colors instead of printing as literal `\x1b[32m` garbage. Older
/// Windows PowerShell 5.1 sessions in the classic conhost window don't
/// always inherit VT mode automatically.
pub fn set_color_enabled(enabled: bool) {
    let _ = COLOR_ENABLED.set(enabled);
    if enabled {
        // `execute!(stdout, ResetColor)` triggers crossterm's internal
        // Windows VT enablement as a side effect. On Linux/macOS it's a
        // harmless ANSI reset. We ignore errors — worst case colors don't
        // render, which the caller can't do anything useful about anyway.
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::style::ResetColor
        );
    }
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

/// Format `prinstall list` results.
///
/// Dedicated formatter because `list` carries richer metadata than
/// scan — queue name, driver, port, shared flag, default flag — and
/// those all deserve their own columns. A `*` marker prefixes the
/// default printer so operators can see at a glance which queue
/// Windows will use when an app just says "print".
pub fn format_list_results(printers: &[Printer]) -> String {
    if printers.is_empty() {
        return "No locally installed printers found.".to_string();
    }

    // ── Column widths ─────────────────────────────────────────────────────
    let name_width = printers
        .iter()
        .map(|p| p.local_name.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(20)
        .max(4);
    let driver_width = printers
        .iter()
        .map(|p| {
            p.driver_name
                .as_deref()
                .or(p.model.as_deref())
                .unwrap_or("-")
                .len()
        })
        .max()
        .unwrap_or(20)
        .max(6);
    let port_width = printers
        .iter()
        .map(|p| p.port_name.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(12)
        .max(4);
    let source_width = "Source".len().max(9);
    let shared_width = "Shared".len();

    let mut out = String::new();

    // ── Header ────────────────────────────────────────────────────────────
    out.push('\n');
    out.push_str(&format!(
        "  {:<name_w$}  {:<driver_w$}  {:<port_w$}  {:<src_w$}  {:<shared_w$}  {}\n",
        header("Name"),
        header("Driver"),
        header("Port"),
        header("Source"),
        header("Shared"),
        header("Status"),
        name_w = name_width,
        driver_w = driver_width,
        port_w = port_width,
        src_w = source_width,
        shared_w = shared_width,
    ));
    out.push_str(&format!(
        "  {:-<name_w$}  {:-<driver_w$}  {:-<port_w$}  {:-<src_w$}  {:-<shared_w$}  {:-<8}\n",
        "", "", "", "", "", "",
        name_w = name_width,
        driver_w = driver_width,
        port_w = port_width,
        src_w = source_width,
        shared_w = shared_width,
    ));

    // ── Rows ──────────────────────────────────────────────────────────────
    let default_count = printers.iter().filter(|p| p.is_default == Some(true)).count();

    for p in printers {
        let name = p.local_name.as_deref().unwrap_or("-");
        let driver = p
            .driver_name
            .as_deref()
            .or(p.model.as_deref())
            .unwrap_or("-");
        let port = p.port_name.as_deref().unwrap_or("-");
        let source_str = match p.source {
            PrinterSource::Network => "Network",
            PrinterSource::Usb => "USB",
            PrinterSource::Installed => "Installed",
        };
        let shared_str = match p.shared {
            Some(true) => "Yes",
            Some(false) => "No",
            None => "-",
        };
        let status_str = p.status.to_string();
        let marker = if p.is_default == Some(true) { "*" } else { " " };

        out.push_str(&format!(
            "{} {:<name_w$}  {:<driver_w$}  {:<port_w$}  {:<src_w$}  {:<shared_w$}  {}\n",
            marker,
            name,
            driver,
            port,
            source_str,
            shared_str,
            status_color(&status_str, &p.status),
            name_w = name_width,
            driver_w = driver_width,
            port_w = port_width,
            src_w = source_width,
            shared_w = shared_width,
        ));
    }

    // ── Footer ────────────────────────────────────────────────────────────
    let total = printers.len();
    let usb_count = printers
        .iter()
        .filter(|p| matches!(p.source, PrinterSource::Usb))
        .count();
    let net_count = printers
        .iter()
        .filter(|p| p.ip.is_some())
        .count();
    let virtual_count = total - usb_count - net_count;

    let mut summary_parts = vec![format!("{total} printer(s)")];
    if net_count > 0 {
        summary_parts.push(format!("{net_count} network"));
    }
    if usb_count > 0 {
        summary_parts.push(format!("{usb_count} USB"));
    }
    if virtual_count > 0 {
        summary_parts.push(format!("{virtual_count} virtual/installed"));
    }
    if default_count > 0 {
        summary_parts.push(format!("{default_count} default"));
    }

    out.push('\n');
    out.push_str(&dim(&format!("  {}", summary_parts.join("  ·  "))));
    out.push('\n');
    if default_count > 0 {
        out.push_str(&dim("  * = Windows default printer"));
        out.push('\n');
    }

    out
}

/// Format driver matching results with all sections:
///   1. Printer info (model, IPP device ID)
///   2. Windows Update probe result (if available)
///   3. Matched drivers (ranked by fuzzy score)
///   4. Universal drivers (manufacturer fallback)
pub fn format_driver_results(results: &DriverResults) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n{} {}\n", label("Printer:"), results.printer_model));
    if let Some(ref device_id) = results.device_id {
        out.push_str(&format!("{} {}\n", label("IPP Device ID:"), dim(device_id)));
    }

    // ── Windows Update probe ──────────────────────────────────────────────────
    if let Some(ref probe) = results.windows_update {
        out.push_str(&format!(
            "\n{}\n",
            header("── Windows Update ───────────────────────────────────────────")
        ));
        if let Some(ref err) = probe.probe_error {
            out.push_str(&format!("  {} {}\n", warn("probe skipped:"), dim(err)));
        } else if probe.from_in_box_fallback {
            out.push_str(&format!(
                "  {} {}  {}\n",
                dim("○"),
                probe.driver_name,
                dim("[Windows in-box fallback]"),
            ));
            out.push_str(&format!(
                "    {}\n",
                dim("Windows Update had no vendor-specific driver for this model.")
            ));
        } else {
            out.push_str(&format!(
                "  {} {}  {}\n",
                badge_exact("★ Windows Update"),
                probe.driver_name,
                dim("[staged in local driver store]"),
            ));
            out.push_str(&format!(
                "    {}\n",
                dim("Ready for install — run: prinstall add <ip>")
            ));
        }
    }

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

    // ── Microsoft Update Catalog ──────────────────────────────────────────────
    if let Some(ref catalog) = results.catalog {
        out.push_str(&format!(
            "\n{}\n",
            header("── Microsoft Update Catalog ─────────────────────────────────")
        ));
        if let Some(ref err) = catalog.error {
            out.push_str(&format!("  {} {}\n", warn("search failed:"), dim(err)));
        } else if catalog.updates.is_empty() {
            out.push_str(&format!(
                "  {}\n",
                dim("No catalog matches — try a broader model or manufacturer name.")
            ));
        } else {
            out.push_str(&format!(
                "  {} {}\n\n",
                dim("query:"),
                dim(&catalog.query),
            ));
            for entry in &catalog.updates {
                out.push_str(&format!(
                    "  #{:<2} {}\n",
                    num,
                    entry.title,
                ));
                out.push_str(&format!(
                    "      {} {}  {} {}\n",
                    label("size:"),
                    entry.size,
                    label("updated:"),
                    entry.last_updated,
                ));
                out.push_str(&format!(
                    "      {} {}\n",
                    dim("products:"),
                    dim(&entry.products),
                ));
                num += 1;
            }
            out.push_str(&format!(
                "\n  {}\n",
                dim("Source: catalog.update.microsoft.com"),
            ));
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
            crate::models::DiscoveryMethod::Mdns => "mDNS",
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
    if let Some(ref note) = detail.warning {
        // "Installed via SDI" and "Installed via Microsoft Update Catalog"
        // are informational breadcrumbs — the install succeeded with a
        // real vendor driver. Only the IPP Class Driver fallback deserves
        // an actual WARNING label (it's a degraded experience).
        let prefix = if note.contains("IPP Class Driver") {
            warn("WARNING:")
        } else {
            dim("SOURCE:")
        };
        out.push_str(&format!("\n  {prefix} {note}\n"));
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

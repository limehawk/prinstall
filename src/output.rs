use crate::models::*;

/// Format scan results as a readable table.
pub fn format_scan_results(printers: &[Printer]) -> String {
    if printers.is_empty() {
        return "No printers found.".to_string();
    }

    let ip_width = printers.iter().map(|p| p.display_ip().len()).max().unwrap_or(15).max(15);
    let model_width = printers
        .iter()
        .map(|p| p.model.as_deref().unwrap_or("Unknown").len())
        .max()
        .unwrap_or(20)
        .max(20);

    let mut out = String::new();
    out.push_str(&format!(
        "\n{:<ip_w$}  {:<model_w$}  {}\n",
        "IP", "Model", "Status",
        ip_w = ip_width, model_w = model_width
    ));
    out.push_str(&format!(
        "{:-<ip_w$}  {:-<model_w$}  {:-<10}\n",
        "", "", "",
        ip_w = ip_width, model_w = model_width
    ));

    for p in printers {
        out.push_str(&format!(
            "{:<ip_w$}  {:<model_w$}  {}\n",
            p.display_ip(),
            p.model.as_deref().unwrap_or("Unknown"),
            p.status,
            ip_w = ip_width,
            model_w = model_width,
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
    out.push_str(&format!("\nPrinter: {}\n", results.printer_model));

    if results.matched.is_empty() && results.universal.is_empty() {
        out.push_str("\nNo drivers found for this printer.\n");
        return out;
    }

    let mut num = 1;

    if !results.matched.is_empty() {
        out.push_str("\n── Matched Drivers ──────────────────────────────────────────\n");
        for dm in &results.matched {
            let badge = match dm.confidence {
                MatchConfidence::Exact => "★ exact",
                MatchConfidence::Fuzzy => "● fuzzy",
                MatchConfidence::Universal => "○",
            };
            let source = match dm.source {
                DriverSource::LocalStore => "[Local Store]",
                DriverSource::Manufacturer => "[Manufacturer]",
            };
            out.push_str(&format!("  #{:<2} {:<45} {:<10} {}\n", num, dm.name, badge, source));
            num += 1;
        }
    }

    if !results.universal.is_empty() {
        out.push_str("\n── Universal Drivers ────────────────────────────────────────\n");
        for dm in &results.universal {
            let source = match dm.source {
                DriverSource::LocalStore => "[Local Store]",
                DriverSource::Manufacturer => "[Manufacturer]",
            };
            out.push_str(&format!("  #{:<2} {:<45} {:<10} {}\n", num, dm.name, "", source));
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

/// Format a single printer identification.
pub fn format_printer_id(printer: &Printer) -> String {
    let mut out = String::new();
    out.push_str(&format!("\nPrinter at {}\n", printer.display_ip()));
    out.push_str(&format!("  Model:  {}\n", printer.model.as_deref().unwrap_or("Unknown")));
    out.push_str(&format!("  Serial: {}\n", printer.serial.as_deref().unwrap_or("N/A")));
    out.push_str(&format!("  Status: {}\n", printer.status));
    out
}

/// Format install result.
pub fn format_install_result(result: &InstallResult) -> String {
    if result.success {
        format!(
            "\nPrinter installed successfully!\n  \
             Name:   {}\n  \
             Driver: {}\n  \
             Port:   {}\n",
            result.printer_name, result.driver_name, result.port_name
        )
    } else {
        format!(
            "\nPrinter installation failed.\n  \
             Error: {}\n",
            result.error.as_deref().unwrap_or("Unknown error")
        )
    }
}

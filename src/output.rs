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

pub fn ok(s: &str) -> String {
    if color_enabled() { s.green().bold().to_string() } else { s.to_string() }
}

pub fn err_text(s: &str) -> String {
    if color_enabled() { s.red().bold().to_string() } else { s.to_string() }
}

pub fn warn(s: &str) -> String {
    if color_enabled() { s.yellow().bold().to_string() } else { s.to_string() }
}

pub fn header(s: &str) -> String {
    if color_enabled() { s.cyan().bold().to_string() } else { s.to_string() }
}

pub fn dim(s: &str) -> String {
    if color_enabled() { s.dark_grey().to_string() } else { s.to_string() }
}

pub fn label(s: &str) -> String {
    if color_enabled() { s.cyan().to_string() } else { s.to_string() }
}

/// Accent color (orange) for highlighted values — printer names, driver
/// names, matched HWIDs, anything the user's eye should land on first.
pub fn accent(s: &str) -> String {
    if color_enabled() {
        s.with(crossterm::style::Color::Rgb { r: 255, g: 107, b: 53 })
            .bold()
            .to_string()
    } else {
        s.to_string()
    }
}

/// Verbose-mode prefix tag. Maps a module name to its semantic color
/// so every `eprintln!("{} ...", vpfx("sdi"))` line gets the right
/// color without the callsite knowing the palette.
pub fn vpfx(module: &str) -> String {
    let tag = format!("[{module}]");
    if !color_enabled() {
        return tag;
    }
    match module {
        "scan" => tag.cyan().to_string(),
        "add" => tag.blue().bold().to_string(),
        "sdi" => tag.with(crossterm::style::Color::Rgb { r: 255, g: 107, b: 53 }).bold().to_string(),
        "resolver" => tag.yellow().to_string(),
        "remove" => tag.magenta().to_string(),
        "PS" | "PS stdout" => tag.dark_grey().to_string(),
        "PS stderr" => tag.red().to_string(),
        "skip" => tag.dark_grey().to_string(),
        "download" => tag.cyan().to_string(),
        _ => tag.dark_grey().to_string(),
    }
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
    let ip_width = printers
        .iter()
        .map(|p| p.ip.map(|ip| ip.to_string().len()).unwrap_or(1))
        .max()
        .unwrap_or(2)
        .max(2);
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
        "  {:<name_w$}  {:<ip_w$}  {:<driver_w$}  {:<port_w$}  {:<src_w$}  {:<shared_w$}  {}\n",
        header("Name"),
        header("IP"),
        header("Driver"),
        header("Port"),
        header("Source"),
        header("Shared"),
        header("Status"),
        name_w = name_width,
        ip_w = ip_width,
        driver_w = driver_width,
        port_w = port_width,
        src_w = source_width,
        shared_w = shared_width,
    ));
    out.push_str(&format!(
        "  {:-<name_w$}  {:-<ip_w$}  {:-<driver_w$}  {:-<port_w$}  {:-<src_w$}  {:-<shared_w$}  {:-<8}\n",
        "", "", "", "", "", "", "",
        name_w = name_width,
        ip_w = ip_width,
        driver_w = driver_width,
        port_w = port_width,
        src_w = source_width,
        shared_w = shared_width,
    ));

    // ── Rows ──────────────────────────────────────────────────────────────
    let default_count = printers.iter().filter(|p| p.is_default == Some(true)).count();

    for p in printers {
        let name = p.local_name.as_deref().unwrap_or("-");
        let ip_str = p.ip.map(|ip| ip.to_string()).unwrap_or_else(|| "-".to_string());
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
            "{} {:<name_w$}  {:<ip_w$}  {:<driver_w$}  {:<port_w$}  {:<src_w$}  {:<shared_w$}  {}\n",
            marker,
            name,
            ip_str,
            driver,
            port,
            source_str,
            shared_str,
            status_color(&status_str, &p.status),
            name_w = name_width,
            ip_w = ip_width,
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

/// Icon tier for a ranked driver candidate. Maps to a semantic color so
/// the user's eye lands on the verified / top-ranked option first.
///   * `Best`     → `★` (green bold)  — verified SDI, Exact match, real WU hit
///   * `Ranked`   → `●` (yellow bold) — Fuzzy match, best Catalog hit
///   * `Fallback` → `○` (dim)         — Universal, unverified SDI, in-box WU
#[derive(Debug, Clone, Copy)]
enum TreeIcon {
    Best,
    Ranked,
    Fallback,
}

impl TreeIcon {
    fn render(self) -> String {
        match self {
            Self::Best => ok("\u{2605}"),        // ★
            Self::Ranked => warn("\u{25CF}"),    // ●
            Self::Fallback => dim("\u{25CB}"),   // ○
        }
    }
}

/// One ranked driver candidate in the tree layout. The `evidence` lines
/// are already colored — [`render_tree`] just prepends the └ bullet.
struct TreeCandidate {
    icon: TreeIcon,
    name: String,
    evidence: Vec<String>,
}

/// Extract the `CID:` (Compatible ID) field from a 1284 device ID string.
/// Returns `None` if no CID segment is present.
fn extract_cid(device_id: &str) -> Option<&str> {
    for part in device_id.split(';') {
        let part = part.trim();
        if let Some(v) = part
            .strip_prefix("CID:")
            .or_else(|| part.strip_prefix("COMPATIBLEID:"))
        {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Split a dotted-numeric version string (e.g. "10.0.17119.1") into a
/// `Vec<u32>` for lexicographic-by-numeric sorting. Non-numeric segments
/// sort as 0 so we never panic on weird catalog data.
fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|s| s.parse::<u32>().unwrap_or(0)).collect()
}

/// Pick the "best" catalog entry — highest parsed version, tiebreak on
/// first occurrence (stable).
fn pick_best_catalog_entry(entries: &[CatalogEntry]) -> Option<&CatalogEntry> {
    entries.iter().max_by(|a, b| {
        version_key(&a.version)
            .cmp(&version_key(&b.version))
    })
}

/// Build the ranked candidate list from a `DriverResults`.
///
/// Order (highest priority first):
///   1. Verified SDI candidates                      → `★`
///   2. Matched drivers with `Exact` confidence      → `★`
///   3. Matched drivers with `Fuzzy` confidence      → `●`
///   4. Windows Update probe success (non-in-box)    → `●`  (promoted here)
///   5. Best Catalog hit (collapsed to 1 row)        → `●`
///   6. Universal drivers                            → `○`
///   7. In-box WU fallback                           → `○`
///   8. Unverified / invalid SDI candidates          → `○`
fn build_tree(results: &DriverResults) -> Vec<TreeCandidate> {
    let mut out: Vec<TreeCandidate> = Vec::new();

    // 1. Verified SDI — lead with the trust story.
    #[cfg(feature = "sdi")]
    for c in results.sdi_candidates.iter().filter(|c| c.verification == "verified") {
        let mut evidence = vec![format!("SDI \u{00B7} pack {}", dim(&c.pack_name))];
        let signer = c.signer.as_deref().unwrap_or("unknown signer");
        evidence.push(format!("{} verified \u{00B7} {}", ok("\u{2713}"), dim(signer)));
        out.push(TreeCandidate {
            icon: TreeIcon::Best,
            name: c.driver_name.clone(),
            evidence,
        });
    }

    // 2. + 3. Matched drivers (Exact first, then Fuzzy).
    for dm in &results.matched {
        let icon = match dm.confidence {
            MatchConfidence::Exact => TreeIcon::Best,
            MatchConfidence::Fuzzy => TreeIcon::Ranked,
            MatchConfidence::Universal => TreeIcon::Fallback,
        };
        let conf = match dm.confidence {
            MatchConfidence::Exact => badge_exact("exact"),
            MatchConfidence::Fuzzy => badge_fuzzy("fuzzy"),
            MatchConfidence::Universal => dim("universal"),
        };
        let src = match dm.source {
            DriverSource::LocalStore => "Local Store",
            DriverSource::Manufacturer => "Manufacturer",
        };
        let pct = (dm.score / 10).min(100);
        let evidence = vec![format!("{} \u{00B7} {} \u{00B7} {}", dim(src), conf, dim(&format!("{pct}%")))];
        out.push(TreeCandidate { icon, name: dm.name.clone(), evidence });
    }
    // Stable sort: Exact before Fuzzy. Ranking within a confidence tier
    // is already delivered by the caller via the `score` ordering.
    out.sort_by_key(|c| match c.icon {
        TreeIcon::Best => 0,
        TreeIcon::Ranked => 1,
        TreeIcon::Fallback => 2,
    });

    // 4. WU probe success — promote to a real candidate row.
    if let Some(ref probe) = results.windows_update
        && probe.is_success()
        && !probe.from_in_box_fallback
    {
        out.push(TreeCandidate {
            icon: TreeIcon::Ranked,
            name: probe.driver_name.clone(),
            evidence: vec![
                format!("Windows Update \u{00B7} {}", dim("staged in driver store")),
            ],
        });
    }

    // 5. Catalog collapsed to best entry.
    if let Some(ref catalog) = results.catalog
        && catalog.error.is_none()
        && !catalog.updates.is_empty()
        && let Some(best) = pick_best_catalog_entry(&catalog.updates)
    {
        let n = catalog.updates.len();
        let name = if n > 1 {
            format!("{} {}", best.title, dim(&format!("(Catalog \u{00B7} {n} variants)")))
        } else {
            format!("{} {}", best.title, dim("(Catalog)"))
        };
        let version_trim = best.version.trim();
        let version_usable = !version_trim.is_empty()
            && !version_trim.eq_ignore_ascii_case("n/a");
        let evidence = vec![if version_usable {
            format!(
                "latest: {} \u{00B7} {} \u{00B7} {}",
                version_trim, best.size, best.last_updated
            )
        } else {
            format!("{} \u{00B7} {}", best.size, best.last_updated)
        }];
        out.push(TreeCandidate {
            icon: TreeIcon::Ranked,
            name,
            evidence,
        });
    }

    // 6. Universal drivers.
    for dm in &results.universal {
        let src = match dm.source {
            DriverSource::LocalStore => "Local Store",
            DriverSource::Manufacturer => "Manufacturer",
        };
        out.push(TreeCandidate {
            icon: TreeIcon::Fallback,
            name: dm.name.clone(),
            evidence: vec![format!("{} \u{00B7} no HWID match", dim(src))],
        });
    }

    // 7. In-box WU fallback — after real drivers, before sketchy SDI.
    if let Some(ref probe) = results.windows_update
        && probe.is_success()
        && probe.from_in_box_fallback
    {
        out.push(TreeCandidate {
            icon: TreeIcon::Fallback,
            name: probe.driver_name.clone(),
            evidence: vec![format!(
                "Windows Update \u{00B7} {}",
                dim("in-box fallback (no vendor driver)")
            )],
        });
    }

    // 8. Unverified / invalid SDI candidates — last so they don't lead.
    #[cfg(feature = "sdi")]
    for c in results.sdi_candidates.iter().filter(|c| c.verification != "verified") {
        let mut evidence = vec![format!("SDI \u{00B7} pack {}", dim(&c.pack_name))];
        let v = &c.verification;
        let verdict_line = if v.starts_with("unsigned") || v.starts_with("invalid") {
            format!("{} {}", err_text("\u{2717}"), err_text(v))
        } else {
            // "no-catalogs", "not-extracted", future states
            format!("{} {}", dim("\u{2717}"), dim(v))
        };
        evidence.push(verdict_line);
        out.push(TreeCandidate {
            icon: TreeIcon::Fallback,
            name: c.driver_name.clone(),
            evidence,
        });
    }

    out
}

/// Render a `Vec<TreeCandidate>` into the final text block. Each candidate
/// gets one header row (icon + name) followed by `└`-prefixed evidence
/// lines. Candidates are separated by a blank line for breathing room
/// on narrow terminals.
fn render_tree(candidates: &[TreeCandidate]) -> String {
    let mut out = String::new();
    for (i, c) in candidates.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{} {}\n", c.icon.render(), c.name));
        for line in &c.evidence {
            out.push_str(&format!("  {} {}\n", dim("\u{2514}"), line));
        }
    }
    out
}

/// Shorten a PowerShell stderr dump into something the user can actually
/// read at the bottom of the drivers report. If the message carries an
/// `HRESULT 0xXXXXXXXX` fragment, preserve that; otherwise trim to the
/// first 60 chars.
fn shorten_probe_error(raw: &str) -> String {
    // Search for an HRESULT token first — it's the single most useful
    // signal in most Add-Printer failures (WU rejection, driver missing).
    for tok in raw.split_whitespace() {
        let t = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != 'x' && c != 'X');
        if (t.starts_with("0x") || t.starts_with("0X"))
            && t.len() >= 6
            && t.chars().skip(2).all(|c| c.is_ascii_hexdigit())
        {
            return format!("HRESULT {t}");
        }
    }
    let trimmed = raw.trim();
    if trimmed.len() > 60 {
        format!("{}…", &trimmed[..60])
    } else {
        trimmed.to_string()
    }
}

/// Format driver matching results as a narrow-terminal tree layout.
///
/// The output is a two-line header (printer model + CID) followed by a
/// ranked candidate list where each driver is one icon-prefixed row with
/// `└`-bulleted evidence lines. Verified SDI packs lead with `★`, fuzzy
/// matches and catalog hits use `●`, fallback universals use `○`.
/// Target width: ~60 columns (fits any RMM SSH shell).
pub fn format_driver_results(results: &DriverResults) -> String {
    let mut out = String::new();

    // ── Header block ──────────────────────────────────────────────────────────
    out.push('\n');
    out.push_str(&accent(&results.printer_model));
    out.push('\n');
    if let Some(ref device_id) = results.device_id {
        if let Some(cid) = extract_cid(device_id) {
            out.push_str(&format!("{} {}\n", dim("CID:"), dim(cid)));
        } else {
            // No CID — surface a trimmed IPP device-id fragment so the
            // operator sees *something* identifying.
            let trimmed = device_id.trim();
            if !trimmed.is_empty() {
                let snippet: String = trimmed.chars().take(40).collect();
                let label = if trimmed.chars().count() > 40 {
                    format!("{snippet}…")
                } else {
                    snippet
                };
                out.push_str(&format!("{} {}\n", dim("IPP:"), dim(&label)));
            }
        }
    }
    out.push('\n');

    // ── Candidate list ────────────────────────────────────────────────────────
    let candidates = build_tree(results);

    // Empty case — preserve the legacy message verbatim.
    let has_traditional = !results.matched.is_empty() || !results.universal.is_empty();
    let has_catalog = results
        .catalog
        .as_ref()
        .map(|c| c.error.is_none() && !c.updates.is_empty())
        .unwrap_or(false);
    #[cfg(feature = "sdi")]
    let has_sdi = !results.sdi_candidates.is_empty();
    #[cfg(not(feature = "sdi"))]
    let has_sdi = false;

    if candidates.is_empty() && !has_traditional && !has_catalog && !has_sdi {
        out.push_str("No drivers found for this printer.\n");
        return out;
    }

    out.push_str(&render_tree(&candidates));

    // ── WU probe footer ───────────────────────────────────────────────────────
    // Only render the footer when the probe *didn't* already land as a
    // candidate row in the main list above (i.e. it failed). Successful
    // probes are already promoted into the tree.
    if let Some(ref probe) = results.windows_update
        && let Some(ref err) = probe.probe_error
    {
        let msg = shorten_probe_error(err);
        out.push('\n');
        out.push_str(&format!(
            "  {} {}\n",
            dim("Windows Update probe: skipped"),
            dim(&format!("({msg})")),
        ));
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

/// Render a full ScanResult as plain text with Network + USB sections.
/// Orphan USB devices (no queue, has_error = true) get a `hint:` line
/// with the exact `prinstall add --usb` command to install them.
pub fn format_scan_result_plain(result: &ScanResult) -> String {
    let mut out = String::new();

    out.push_str("Network printers\n");
    out.push_str("----------------\n");
    if result.network.is_empty() {
        out.push_str("  (none discovered)\n");
    } else {
        for p in &result.network {
            out.push_str(&format!(
                "  {:<15}  {}\n",
                p.display_ip(),
                p.model.as_deref().unwrap_or("(unknown model)")
            ));
        }
    }
    out.push('\n');

    out.push_str("USB-attached printers\n");
    out.push_str("---------------------\n");
    if result.usb.is_empty() {
        out.push_str("  (none detected)\n");
        return out;
    }
    for dev in &result.usb {
        out.push_str(&format_usb_device_line(dev));
    }
    out
}

fn format_usb_device_line(dev: &UsbDevice) -> String {
    let name = dev.friendly_name.as_deref().unwrap_or("(unknown device)");
    let state = match (&dev.queue_name, dev.has_error) {
        (Some(q), _) => format!("queue: {q}"),
        (None, true) => "NO QUEUE (driver missing)".to_string(),
        (None, false) => "NO QUEUE".to_string(),
    };
    let mut line = format!("  {name}  [{state}]\n");
    if dev.queue_name.is_none() {
        line.push_str(&format!(
            "    hint: run 'prinstall add --usb \"{name}\"' to install\n"
        ));
    }
    line
}

/// Render a ScanResult as pretty JSON.
pub fn format_scan_result_json(result: &ScanResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod scan_result_print_tests {
    use super::*;
    use crate::models::{ScanResult, UsbDevice};

    fn sample_result() -> ScanResult {
        ScanResult {
            network: vec![],
            usb: vec![
                UsbDevice {
                    hardware_id: "USB\\VID_03F0&PID_1D17\\ABC".into(),
                    friendly_name: Some("HP LaserJet 1320".into()),
                    queue_name: None,
                    has_error: true,
                },
                UsbDevice {
                    hardware_id: "USB\\VID_04B8&PID_0005\\DEF".into(),
                    friendly_name: Some("Brother MFC".into()),
                    queue_name: Some("Brother MFC".into()),
                    has_error: false,
                },
            ],
        }
    }

    #[test]
    fn plain_output_has_usb_section_header() {
        let out = format_scan_result_plain(&sample_result());
        assert!(out.contains("USB-attached printers"));
    }

    #[test]
    fn plain_output_shows_orphan_install_hint() {
        let out = format_scan_result_plain(&sample_result());
        assert!(out.contains("hint:"));
        assert!(out.contains("prinstall add"));
        assert!(out.contains("--usb"));
        assert!(out.contains("HP LaserJet 1320"));
    }

    #[test]
    fn plain_output_omits_hint_when_queue_exists() {
        let result = ScanResult {
            network: vec![],
            usb: vec![UsbDevice {
                hardware_id: "USB\\VID_04B8&PID_0005\\DEF".into(),
                friendly_name: Some("Brother MFC".into()),
                queue_name: Some("Brother MFC".into()),
                has_error: false,
            }],
        };
        let out = format_scan_result_plain(&result);
        assert!(!out.contains("hint:"));
    }
}


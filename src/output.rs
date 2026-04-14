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

/// Format `prinstall list` results as a narrow-terminal tree layout.
///
/// Matches the style established by [`format_driver_results`]: a summary
/// line at the top, then one icon-prefixed block per printer with two
/// `└`-bulleted evidence lines. Default printer leads with `★`, network
/// queues use `●`, USB / installed queues use `○`. Target width ~60 cols.
pub fn format_list_results(printers: &[Printer]) -> String {
    if printers.is_empty() {
        return "No locally installed printers found.".to_string();
    }

    // ── Summary line ──────────────────────────────────────────────────────
    let total = printers.len();
    let net_count = printers.iter().filter(|p| p.ip.is_some()).count();
    let usb_count = printers
        .iter()
        .filter(|p| matches!(p.source, PrinterSource::Usb))
        .count();
    let virtual_count = total.saturating_sub(usb_count).saturating_sub(net_count);
    let default_count = printers
        .iter()
        .filter(|p| p.is_default == Some(true))
        .count();

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

    let mut out = String::new();
    out.push('\n');
    out.push_str(&dim(&summary_parts.join(" \u{00B7} ")));
    out.push_str("\n\n");

    // ── Rank: default → network → USB → virtual/installed ────────────────
    // Within each tier, network rows sort by IP. Everything else keeps
    // insertion order so the PowerShell `Get-Printer` sort is preserved.
    let mut ranked: Vec<(usize, &Printer)> = printers.iter().enumerate().collect();
    ranked.sort_by_key(|(idx, p)| {
        let tier = if p.is_default == Some(true) {
            0
        } else if p.ip.is_some() {
            1
        } else if matches!(p.source, PrinterSource::Usb) {
            2
        } else {
            3
        };
        // Secondary sort key for network tier: IP bytes for natural ordering.
        let ip_key = p.ip.map(|ip| ip.octets()).unwrap_or([255, 255, 255, 255]);
        (tier, ip_key, *idx)
    });

    // ── Build tree candidates ─────────────────────────────────────────────
    let candidates: Vec<TreeCandidate> = ranked
        .into_iter()
        .map(|(_, p)| {
            let name = p
                .local_name
                .clone()
                .unwrap_or_else(|| "(unnamed)".to_string());

            // Annotation: "(default)", "(shared)", or "(default · shared)".
            let is_default = p.is_default == Some(true);
            let is_shared = p.shared == Some(true);
            let annotated_name = match (is_default, is_shared) {
                (true, true) => format!("{name} {}", dim("(default \u{00B7} shared)")),
                (true, false) => format!("{name} {}", dim("(default)")),
                (false, true) => format!("{name} {}", dim("(shared)")),
                (false, false) => name,
            };

            // Icon: star if default, filled dot if network (non-default),
            // open circle otherwise (USB / installed / virtual).
            let icon = if is_default {
                TreeIcon::Best
            } else if p.ip.is_some() {
                TreeIcon::Ranked
            } else {
                TreeIcon::Fallback
            };

            // First evidence line: "<ip or port> · <driver>".
            let driver = p
                .driver_name
                .as_deref()
                .or(p.model.as_deref())
                .unwrap_or("-");
            let locator = if let Some(ip) = p.ip {
                ip.to_string()
            } else {
                p.port_name
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
            };
            let evidence_1 = format!("{} \u{00B7} {}", locator, driver);

            // Second evidence line: "<source> · <status>".
            let source_str = match p.source {
                PrinterSource::Network => "Network",
                PrinterSource::Usb => "USB",
                PrinterSource::Installed => "Installed",
            };
            let status_str = p.status.to_string();
            let evidence_2 = format!(
                "{} \u{00B7} {}",
                dim(source_str),
                status_color(&status_str, &p.status),
            );

            TreeCandidate::bare(icon, annotated_name, vec![evidence_1, evidence_2])
        })
        .collect();

    out.push_str(&render_tree(&candidates));
    out
}

/// Normalize a driver-date string into `YYYY-MM-DD`.
///
/// Accepts the common shapes we see across driver sources:
///   * ISO: `"2024-03-15"` or `"2024-03-15T00:00:00"` — passed through
///   * US slashed: `"3/15/2024"` or `"03/15/2024"`
///   * INF `DriverVer`: `"03/15/2024,1.0.0.0"` — takes the leading date
///   * PS JSON DateTime fallback: `"/Date(1710460800000)/"` (ms since epoch)
///
/// Returns `None` for anything unparseable. Month/day/year validation is
/// strict — out-of-range components return `None` rather than a silently
/// rolled-over date.
pub fn normalize_date(raw: &str) -> Option<String> {
    use chrono::{DateTime, NaiveDate};

    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    // INF DriverVer: "MM/DD/YYYY,x.y.z.w" — take the date half.
    let head = s.split(',').next().unwrap_or(s).trim();

    // ISO datetime: "2024-03-15T00:00:00" — take the date portion.
    let date_only = head.split('T').next().unwrap_or(head).trim();

    // ISO date: "2024-03-15"
    if let Ok(d) = NaiveDate::parse_from_str(date_only, "%Y-%m-%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }

    // US slashed: "M/D/YYYY" or "MM/DD/YYYY"
    for fmt in &["%m/%d/%Y", "%-m/%-d/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(date_only, fmt) {
            return Some(d.format("%Y-%m-%d").to_string());
        }
    }

    // PS JSON DateTime: "/Date(1710460800000)/"
    if let Some(inner) = date_only
        .strip_prefix("/Date(")
        .and_then(|s| s.strip_suffix(")/"))
    {
        // Strip trailing timezone offset if present (e.g. "1234567890000-0500").
        let ms_str = inner.split(['+', '-']).next().unwrap_or(inner);
        if let Ok(ms) = ms_str.parse::<i64>()
            && let Some(dt) = DateTime::from_timestamp_millis(ms)
        {
            return Some(dt.date_naive().format("%Y-%m-%d").to_string());
        }
    }

    // Full ISO timestamp with fractional seconds / offsets.
    if let Ok(dt) = DateTime::parse_from_rfc3339(head) {
        return Some(dt.date_naive().format("%Y-%m-%d").to_string());
    }

    None
}

/// Trust tier for a driver candidate. Drives the verification-score half
/// of the combined ranking in [`build_tree`].
///
///   * `Verified`            — explicit signature check passed (Task 17 SDI
///     gate, or Task 25 catalog / manufacturer verified). 1.0.
///   * `TrustedUnverified`   — comes from a trusted source we just didn't
///     gate (catalog, manufacturer, local driver store). 0.3.
///   * `UnverifiedCommunity` — unsigned SDI, unknown origin. 0.1.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Verification {
    Verified,
    TrustedUnverified,
    UnverifiedCommunity,
}

impl Verification {
    fn score(self) -> f64 {
        match self {
            Self::Verified => 1.0,
            Self::TrustedUnverified => 0.3,
            Self::UnverifiedCommunity => 0.1,
        }
    }
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
///
/// The `parsed_date` / `verification` fields only matter in the driver-tree
/// path — [`build_tree`] uses them to compute a combined recency-plus-trust
/// sort score. List / scan / printer-id consumers leave them defaulted and
/// skip the sort step, preserving insertion order.
struct TreeCandidate {
    icon: TreeIcon,
    name: String,
    evidence: Vec<String>,
    /// Publication date parsed from the source's raw date string via
    /// [`normalize_date`] then [`chrono::NaiveDate::parse_from_str`]. Only
    /// populated for driver rows; other callers (list/scan/id) leave it None.
    parsed_date: Option<chrono::NaiveDate>,
    /// Trust tier for this candidate. Only meaningful on driver rows; other
    /// callers leave it at the default `TrustedUnverified`.
    verification: Verification,
}

impl TreeCandidate {
    /// Minimal constructor used by the list/scan/id paths — no date, no
    /// verification, just the icon+name+evidence trio they already produce.
    fn bare(icon: TreeIcon, name: String, evidence: Vec<String>) -> Self {
        Self {
            icon,
            name,
            evidence,
            parsed_date: None,
            verification: Verification::TrustedUnverified,
        }
    }
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
/// Ranking (Task 26): a combined score of
///
/// ```text
/// score = date_score * 0.6  +  verification_score * 0.4
/// ```
///
/// * `date_score` — normalized recency across the full candidate set.
///   Oldest → 0.0, newest → 1.0; linear interpolation by days. Candidates
///   with no known date receive a midpoint `0.5` so they're not shoved
///   to the bottom on the absence-of-data alone.
/// * `verification_score` — 1.0 for verified signatures, 0.3 for
///   trusted-but-unverified sources (catalog, manufacturer, local store),
///   0.1 for unsigned / unknown (community SDI without a signature).
///
/// Icon (★ / ● / ○) still reflects verification independently of the sort
/// order — a freshly-dated but unsigned SDI candidate can outrank an older
/// verified one while still carrying the open-circle marker so the user
/// can see the trust tier at a glance.
fn build_tree(results: &DriverResults) -> Vec<TreeCandidate> {
    let mut out: Vec<TreeCandidate> = Vec::new();

    // 1. Verified SDI — lead with the trust story.
    #[cfg(feature = "sdi")]
    for c in results.sdi_candidates.iter().filter(|c| c.verification == "verified") {
        let mut evidence = vec![format_sdi_evidence_line(&c.pack_name, c.driver_date.as_deref())];
        let signer = c.signer.as_deref().unwrap_or("unknown signer");
        evidence.push(format!("{} verified \u{00B7} {}", ok("\u{2713}"), dim(signer)));
        out.push(TreeCandidate {
            icon: TreeIcon::Best,
            name: c.driver_name.clone(),
            evidence,
            parsed_date: parse_normalized(c.driver_date.as_deref()),
            verification: Verification::Verified,
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
        let date_suffix = format_date_suffix(dm.driver_date.as_deref());
        let evidence = vec![format!(
            "{} \u{00B7} {} \u{00B7} {}{}",
            dim(src),
            conf,
            dim(&format!("{pct}%")),
            date_suffix,
        )];
        out.push(TreeCandidate {
            icon,
            name: dm.name.clone(),
            evidence,
            parsed_date: parse_normalized(dm.driver_date.as_deref()),
            verification: Verification::TrustedUnverified,
        });
    }

    // 4. Catalog collapsed to best entry.
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
        let normalized_date = normalize_date(&best.last_updated);
        let shown_date = normalized_date.clone().unwrap_or_else(|| best.last_updated.clone());
        let version_trim = best.version.trim();
        let version_usable = !version_trim.is_empty()
            && !version_trim.eq_ignore_ascii_case("n/a");
        let evidence = vec![if version_usable {
            format!(
                "latest: {} \u{00B7} {} \u{00B7} date: {}",
                version_trim, best.size, shown_date,
            )
        } else {
            format!("{} \u{00B7} date: {}", best.size, shown_date)
        }];
        out.push(TreeCandidate {
            icon: TreeIcon::Ranked,
            name,
            evidence,
            parsed_date: normalized_date.as_deref().and_then(parse_iso_date),
            verification: Verification::TrustedUnverified,
        });
    }

    // 5. Universal drivers.
    for dm in &results.universal {
        let src = match dm.source {
            DriverSource::LocalStore => "Local Store",
            DriverSource::Manufacturer => "Manufacturer",
        };
        let date_suffix = format_date_suffix(dm.driver_date.as_deref());
        out.push(TreeCandidate {
            icon: TreeIcon::Fallback,
            name: dm.name.clone(),
            evidence: vec![format!(
                "{} \u{00B7} no HWID match{}",
                dim(src),
                date_suffix,
            )],
            parsed_date: parse_normalized(dm.driver_date.as_deref()),
            verification: Verification::TrustedUnverified,
        });
    }

    // 6. Unverified / invalid SDI candidates — sketchy trust tier.
    #[cfg(feature = "sdi")]
    for c in results.sdi_candidates.iter().filter(|c| c.verification != "verified") {
        let mut evidence = vec![format_sdi_evidence_line(&c.pack_name, c.driver_date.as_deref())];
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
            parsed_date: parse_normalized(c.driver_date.as_deref()),
            verification: Verification::UnverifiedCommunity,
        });
    }

    // ── Rank by combined (date, verification) score ─────────────────────────
    sort_by_combined_score(&mut out);

    out
}

/// Build the evidence line `SDI · pack {name} · date: {YYYY-MM-DD|unknown}`.
/// Shared between the verified and unverified SDI branches so the two render
/// with identical structure.
fn format_sdi_evidence_line(pack_name: &str, raw_date: Option<&str>) -> String {
    let shown = raw_date
        .and_then(normalize_date)
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "SDI \u{00B7} pack {} \u{00B7} date: {}",
        dim(pack_name),
        dim(&shown),
    )
}

/// Build the trailing ` · date: {YYYY-MM-DD|unknown}` suffix appended to an
/// existing evidence line. Returns "" (not " · date: unknown") when the caller
/// wants to suppress the suffix entirely — currently nobody does, but the
/// empty-branch keeps the API honest.
fn format_date_suffix(raw_date: Option<&str>) -> String {
    let shown = raw_date
        .and_then(normalize_date)
        .unwrap_or_else(|| "unknown".to_string());
    format!(" \u{00B7} date: {}", dim(&shown))
}

/// Parse a date string through [`normalize_date`] then into a `NaiveDate`
/// for range math. Returns `None` on any failure — the combined-score pass
/// treats that as "unknown date, use the midpoint".
fn parse_normalized(raw: Option<&str>) -> Option<chrono::NaiveDate> {
    raw.and_then(normalize_date).and_then(|s| parse_iso_date(&s))
}

/// Parse an already-normalized `YYYY-MM-DD` string into `chrono::NaiveDate`.
fn parse_iso_date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// Stable sort by combined `(date_score * 0.6 + verification_score * 0.4)`.
/// Higher score ranks earlier. Equal-score rows preserve their insertion
/// order (stable sort — tie-broken by original index).
fn sort_by_combined_score(candidates: &mut Vec<TreeCandidate>) {
    // Determine min/max date across candidates that have one. An empty range
    // (everyone known, same day) collapses to date_score 1.0.
    let dates: Vec<chrono::NaiveDate> =
        candidates.iter().filter_map(|c| c.parsed_date).collect();
    let min = dates.iter().copied().min();
    let max = dates.iter().copied().max();

    // Attach a score to every candidate, move the whole lot through a sort
    // key, then drop the score to yield the sorted `Vec<TreeCandidate>`.
    // `Vec::drain(..)` + `.collect()` avoids cloning while letting us sort
    // on the attached float.
    let mut scored: Vec<(f64, TreeCandidate)> = candidates
        .drain(..)
        .map(|c| {
            let date_score = match (c.parsed_date, min, max) {
                (Some(d), Some(lo), Some(hi)) => {
                    let span = (hi - lo).num_days();
                    if span <= 0 {
                        1.0
                    } else {
                        (d - lo).num_days() as f64 / span as f64
                    }
                }
                _ => 0.5,
            };
            let combined = date_score * 0.6 + c.verification.score() * 0.4;
            (combined, c)
        })
        .collect();

    // `sort_by` is stable in Rust, so equal scores preserve insertion order
    // without a manual tie-breaker. Descending on score means highest first.
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    candidates.extend(scored.into_iter().map(|(_, c)| c));
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
    let name = printer
        .model
        .as_deref()
        .or(printer.local_name.as_deref())
        .map(|s| s.to_string());

    let mut evidence: Vec<String> = Vec::new();

    // Line 1: ip · methods
    let ip_str = printer.display_ip();
    let methods: Vec<&str> = printer
        .discovery_methods
        .iter()
        .map(method_label)
        .collect();
    let line1 = if methods.is_empty() {
        ip_str.clone()
    } else {
        format!("{} \u{00B7} {}", ip_str, methods.join(" \u{00B7} "))
    };
    if !line1.trim().is_empty() && ip_str != "Unknown" {
        evidence.push(line1);
    }

    if let Some(ref s) = printer.serial {
        evidence.push(format!("serial: {}", s));
    }

    if !printer.ports.is_empty() {
        let ports: Vec<String> = printer.ports.iter().map(|p| p.to_string()).collect();
        evidence.push(format!("ports: {}", ports.join(", ")));
    }

    let candidate = if let Some(n) = name {
        TreeCandidate::bare(TreeIcon::Ranked, n, evidence)
    } else {
        TreeCandidate::bare(TreeIcon::Fallback, dim("(unknown printer)"), evidence)
    };

    render_tree(&[candidate])
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

/// Render a full ScanResult as plain text using the tree layout shared
/// with `drivers` and `list`. Network and USB appear as two minimal
/// labeled blocks separated by a blank line. Orphan USB devices (no
/// queue) get a `hint:` child row with the exact `prinstall add --usb`
/// command to install them.
pub fn format_scan_result_plain(result: &ScanResult) -> String {
    let mut out = String::new();

    if result.network.is_empty() && result.usb.is_empty() {
        out.push_str("(no printers discovered)\n\n");
        out.push_str(&dim("If this is unexpected, try:"));
        out.push('\n');
        out.push_str(&dim("  · prinstall scan --community <string>     (non-default SNMP community)"));
        out.push('\n');
        out.push_str(&dim("  · prinstall scan --method port            (TCP-only, skip SNMP)"));
        out.push('\n');
        out.push_str(&dim("  · prinstall scan --method mdns            (mDNS multicast browse)"));
        out.push('\n');
        out.push_str(&dim("  · prinstall scan --timeout 1500           (slower networks)"));
        out.push('\n');
        out.push_str(&dim("  · Check printer power, SNMP enabled in printer web UI"));
        out.push('\n');
        return out;
    }

    // ── Network block ────────────────────────────────────────────────────
    if !result.network.is_empty() {
        let candidates: Vec<TreeCandidate> = result
            .network
            .iter()
            .map(network_tree_candidate)
            .collect();
        out.push_str(&header("Network"));
        out.push('\n');
        out.push_str(&render_tree(&candidates));
    }

    // ── USB block ────────────────────────────────────────────────────────
    if !result.usb.is_empty() {
        if !result.network.is_empty() {
            out.push('\n');
        }
        let candidates: Vec<TreeCandidate> =
            result.usb.iter().map(usb_tree_candidate).collect();
        out.push_str(&header("USB"));
        out.push('\n');
        out.push_str(&render_tree(&candidates));
    }

    out
}

/// Human-readable tag for each discovery method, joined with ` · ` in
/// the evidence line.
fn method_label(m: &DiscoveryMethod) -> &'static str {
    match m {
        DiscoveryMethod::PortScan => "Port",
        DiscoveryMethod::Ipp => "IPP",
        DiscoveryMethod::Snmp => "SNMP",
        DiscoveryMethod::Local => "Local",
        DiscoveryMethod::Mdns => "mDNS",
    }
}

/// Build a network printer row: `● <ip>  <model>` with a single
/// evidence line listing discovery methods and probed ports.
fn network_tree_candidate(p: &Printer) -> TreeCandidate {
    let ip_str = p.display_ip();
    let name = if let Some(ref model) = p.model {
        format!("{}  {}", ip_str, model)
    } else {
        format!("{}  {}", ip_str, dim("(unknown model)"))
    };

    let methods: Vec<&'static str> = p.discovery_methods.iter().map(method_label).collect();
    let mut evidence_line = String::new();
    if !methods.is_empty() {
        evidence_line.push_str(&dim(&methods.join(" \u{00B7} ")));
    }
    if !p.ports.is_empty() {
        if !evidence_line.is_empty() {
            evidence_line.push_str("  ");
        }
        let port_word = if p.ports.len() == 1 { "port" } else { "ports" };
        let port_list = p
            .ports
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        evidence_line.push_str(&dim(&format!("{port_word} {port_list}")));
    }

    let evidence = if evidence_line.is_empty() {
        Vec::new()
    } else {
        vec![evidence_line]
    };

    TreeCandidate::bare(TreeIcon::Ranked, name, evidence)
}

/// Build a USB device row. Working queues get `●` with a dim
/// `(queue: X)` suffix and no child row. Orphans get `○` with an
/// inline `NO QUEUE` marker (colored if `has_error`) plus a `hint:`
/// child row carrying the exact install command.
fn usb_tree_candidate(dev: &UsbDevice) -> TreeCandidate {
    let friendly = dev.friendly_name.as_deref().unwrap_or("(unknown device)");
    match &dev.queue_name {
        Some(q) => TreeCandidate::bare(
            TreeIcon::Ranked,
            format!("{friendly} {}", dim(&format!("(queue: {q})"))),
            Vec::new(),
        ),
        None => {
            let marker = if dev.has_error { warn("NO QUEUE") } else { dim("NO QUEUE") };
            TreeCandidate::bare(
                TreeIcon::Fallback,
                format!("{friendly}  {marker}"),
                vec![dim(&format!("hint: prinstall add --usb \"{friendly}\""))],
            )
        }
    }
}

/// Render a ScanResult as pretty JSON.
pub fn format_scan_result_json(result: &ScanResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod scan_result_print_tests {
    use super::*;
    use crate::models::{DiscoveryMethod, Printer, PrinterSource, PrinterStatus, ScanResult, UsbDevice};

    fn network_printer(ip: &str, model: Option<&str>) -> Printer {
        Printer {
            ip: ip.parse().ok(),
            model: model.map(|s| s.to_string()),
            serial: None,
            status: PrinterStatus::Ready,
            discovery_methods: vec![DiscoveryMethod::Snmp, DiscoveryMethod::Ipp],
            ports: vec![9100, 631],
            source: PrinterSource::Network,
            local_name: None,
            port_name: None,
            driver_name: None,
            shared: None,
            is_default: None,
        }
    }

    fn sample_result() -> ScanResult {
        ScanResult {
            network: vec![network_printer(
                "192.168.1.50",
                Some("Brother MFC-L2750DW series"),
            )],
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
    fn plain_output_renders_both_section_labels() {
        let out = format_scan_result_plain(&sample_result());
        assert!(out.contains("Network"), "missing Network label:\n{out}");
        assert!(out.contains("USB"), "missing USB label:\n{out}");
        // The old section-header divider style must be gone.
        assert!(!out.contains("----"), "dashed divider should be gone:\n{out}");
        assert!(!out.contains("Network printers"));
        assert!(!out.contains("USB-attached printers"));
    }

    #[test]
    fn plain_output_renders_network_icon_and_row() {
        let out = format_scan_result_plain(&sample_result());
        // ● icon precedes the IP + model on the same line.
        assert!(
            out.contains("\u{25CF} 192.168.1.50"),
            "expected ● with IP, got:\n{out}"
        );
        assert!(out.contains("Brother MFC-L2750DW series"));
    }

    #[test]
    fn plain_output_shows_discovery_methods_child() {
        let out = format_scan_result_plain(&sample_result());
        // Methods joined with ` · `
        assert!(
            out.contains("SNMP \u{00B7} IPP"),
            "expected joined methods, got:\n{out}"
        );
        // Port list appears with the word `ports`.
        assert!(out.contains("ports 9100, 631"), "expected port list:\n{out}");
    }

    #[test]
    fn plain_output_usb_orphan_has_no_queue_marker_and_hint() {
        let out = format_scan_result_plain(&sample_result());
        // ○ icon precedes the friendly name for orphan.
        assert!(
            out.contains("\u{25CB} HP LaserJet 1320"),
            "expected ○ orphan row, got:\n{out}"
        );
        // `NO QUEUE` appears inline on the same row.
        assert!(out.contains("NO QUEUE"));
        // Install hint appears as a child.
        assert!(out.contains("hint: prinstall add --usb \"HP LaserJet 1320\""));
    }

    #[test]
    fn plain_output_usb_working_queue_has_queue_suffix() {
        let out = format_scan_result_plain(&sample_result());
        // ● icon for working queue + `(queue: X)` annotation.
        assert!(
            out.contains("\u{25CF} Brother MFC"),
            "expected ● for working queue, got:\n{out}"
        );
        assert!(out.contains("(queue: Brother MFC)"));
    }

    #[test]
    fn plain_output_working_queue_has_no_hint_child() {
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
        assert!(!out.contains("hint:"), "working queue should have no hint:\n{out}");
    }

    #[test]
    fn plain_output_both_empty_shows_friendly_message() {
        let result = ScanResult {
            network: vec![],
            usb: vec![],
        };
        let out = format_scan_result_plain(&result);
        assert!(out.contains("(no printers discovered)"));
        assert!(!out.contains("Network"));
        assert!(!out.contains("USB"));
    }

    #[test]
    fn plain_output_both_empty_shows_troubleshooting_guidance() {
        let result = ScanResult {
            network: vec![],
            usb: vec![],
        };
        let out = format_scan_result_plain(&result);
        assert!(out.contains("If this is unexpected"));
        assert!(out.contains("--community"));
        assert!(out.contains("--method port"));
        assert!(out.contains("--method mdns"));
        assert!(out.contains("--timeout"));
        assert!(out.contains("SNMP enabled"));
    }

    #[test]
    fn plain_output_usb_only_omits_network_label() {
        let result = ScanResult {
            network: vec![],
            usb: vec![UsbDevice {
                hardware_id: "USB\\VID_03F0&PID_1D17\\ABC".into(),
                friendly_name: Some("HP LaserJet 1320".into()),
                queue_name: None,
                has_error: true,
            }],
        };
        let out = format_scan_result_plain(&result);
        assert!(out.contains("USB"));
        // Network label should be absent on a USB-only result.
        assert!(
            !out.lines().any(|l| l.trim() == "Network"),
            "Network label leaked on USB-only result:\n{out}"
        );
    }

    #[test]
    fn plain_output_network_only_omits_usb_label() {
        let result = ScanResult {
            network: vec![network_printer("10.10.20.16", Some("HP LaserJet Pro"))],
            usb: vec![],
        };
        let out = format_scan_result_plain(&result);
        assert!(out.lines().any(|l| l.trim() == "Network"));
        assert!(
            !out.lines().any(|l| l.trim() == "USB"),
            "USB label leaked on network-only result:\n{out}"
        );
    }
}


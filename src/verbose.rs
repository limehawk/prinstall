//! Structured verbose output for `prinstall add`.
//!
//! Instead of streaming `eprintln!("[module] text")` lines as each step
//! happens, the add flow builds an [`InstallReport`] incrementally and
//! renders it as a single structured block when the install completes.
//!
//! As of v0.4.11 the report is rendered as a stack of rounded-corner
//! cards (╭─╮│╰╯) with semantic emoji/icon status markers. Each phase
//! (Discovery, Driver Resolution, Install) gets its own card, and the
//! final summary card uses emoji + color to communicate the outcome:
//!
//!   ✔️  success (with optional 🛡️ Authenticode-verified child line)
//!   ⚠️  Installed via IPP Class Driver (generic fallback)
//!   ❌  Install failed — all tiers exhausted
//!
//! Fixed 62-column inner width keeps it readable in narrow RMM shells.
//! Set `PRINSTALL_NO_EMOJI=1` to switch to ASCII fallbacks for consoles
//! that can't render the Unicode glyphs cleanly.

use std::fmt::Write;
use std::time::Duration;

use crate::output;

// ── Card geometry ────────────────────────────────────────────────────────────

/// Inner card width in visible columns (content between the two │ border
/// characters). Total visible line width is `CARD_WIDTH + 2` = 64 cols.
const CARD_WIDTH: usize = 62;

// ── Phase data types ─────────────────────────────────────────────────────────

/// What discovery found about the printer.
#[derive(Default)]
pub struct DiscoveryPhase {
    pub snmp_model: Option<String>,
    pub ipp_model: Option<String>,
    pub ipp_cid: Option<String>,
    pub device_id: Option<String>,
    pub ip: String,
}

/// A single driver resolution tier's outcome.
pub struct TierResult {
    pub name: String,
    pub status: TierStatus,
    pub detail: String,
    /// Optional WHCP / signer info. Rendered as a `└─ 🛡️ verified · <signer>`
    /// child line under Verified tiers.
    pub signer: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum TierStatus {
    /// tried and failed or no match
    Failed,
    /// not attempted (skipped by flag or prior hit)
    Skipped,
    /// this tier won
    Matched,
    /// this tier won AND Authenticode verification passed
    Verified,
    /// explicitly disabled via flag
    Disabled,
}

/// The driver resolution cascade.
#[derive(Default)]
pub struct ResolutionPhase {
    pub tiers: Vec<TierResult>,
    pub winner_idx: Option<usize>,
}

impl ResolutionPhase {
    pub fn add_tier(&mut self, name: &str, status: TierStatus, detail: &str) {
        if matches!(status, TierStatus::Matched | TierStatus::Verified)
            && self.winner_idx.is_none()
        {
            self.winner_idx = Some(self.tiers.len());
        }
        self.tiers.push(TierResult {
            name: name.to_string(),
            status,
            detail: detail.to_string(),
            signer: None,
        });
    }

    /// Attach a signer annotation to the most recently added tier.
    /// Renders as a `└─ 🛡️ verified · <signer>` child line.
    pub fn set_last_signer(&mut self, signer: &str) {
        if let Some(last) = self.tiers.last_mut() {
            last.signer = Some(signer.to_string());
        }
    }
}

/// Per-step result for the install phase.
pub struct StepResult {
    pub label: String,
    pub value: String,
    pub ok: bool,
}

/// The three-step install (port + driver + queue).
#[derive(Default)]
pub struct InstallPhase {
    pub steps: Vec<StepResult>,
}

impl InstallPhase {
    pub fn add_step(&mut self, label: &str, value: &str, ok: bool) {
        self.steps.push(StepResult {
            label: label.to_string(),
            value: value.to_string(),
            ok,
        });
    }
}

/// The full phased report — built up during `run_network`, rendered once.
pub struct InstallReport {
    pub discovery: DiscoveryPhase,
    pub resolution: ResolutionPhase,
    pub install: InstallPhase,
    pub elapsed: Duration,
    pub source_annotation: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    /// What command produced this report. Used in the header card.
    /// Defaults to `"prinstall add <ip>"` when not explicitly set.
    pub command: Option<String>,
}

impl InstallReport {
    pub fn new(ip: &str) -> Self {
        Self {
            discovery: DiscoveryPhase {
                ip: ip.to_string(),
                ..Default::default()
            },
            resolution: ResolutionPhase::default(),
            install: InstallPhase::default(),
            elapsed: Duration::ZERO,
            source_annotation: None,
            success: false,
            error: None,
            command: None,
        }
    }

    /// Render the full structured report to stderr.
    pub fn render(&self) {
        let mut buf = String::with_capacity(2048);
        self.render_to_string(&mut buf);
        eprint!("{buf}");
    }

    /// Render the full structured report to a caller-provided buffer.
    /// Used by tests and by [`Self::render`] which then writes to stderr.
    pub fn render_to_string(&self, buf: &mut String) {
        buf.push('\n');

        self.render_header(buf);
        self.render_discovery(buf);
        self.render_resolution(buf);
        self.render_install(buf);
        self.render_summary(buf);
    }

    // ── Header card ─────────────────────────────────────────────────────

    fn render_header(&self, buf: &mut String) {
        let cmd = self.command.clone().unwrap_or_else(|| {
            if self.discovery.ip.is_empty() {
                "prinstall add".to_string()
            } else {
                format!("prinstall add {}", self.discovery.ip)
            }
        });

        let version = env!("CARGO_PKG_VERSION");
        let model = self
            .discovery
            .snmp_model
            .as_deref()
            .or(self.discovery.ipp_model.as_deref());

        render_card(buf, None, |body| {
            let line1 = format!(" {}  {}", emoji("🚀", ">>"), cmd);
            body.push(card_line(&line1));
            if let Some(model) = model {
                let line2 = format!("     v{}  {}  {}", version, emoji("·", "-"), output::accent(model));
                body.push(card_line(&line2));
            } else {
                let line2 = format!("     v{}", version);
                body.push(card_line(&output::dim(&line2)));
            }
        });
        buf.push('\n');
    }

    // ── Discovery ───────────────────────────────────────────────────────

    fn render_discovery(&self, buf: &mut String) {
        let d = &self.discovery;
        // Collect rows first so we can skip the card entirely when empty.
        let mut rows: Vec<(&str, String, bool)> = Vec::new();

        if let Some(ref model) = d.snmp_model {
            rows.push(("SNMP     ", output::accent(model), false));
        }
        if let Some(ref model) = d.ipp_model {
            let text = if d.ipp_cid.is_some() {
                format!("{} {}", output::accent(model), output::dim("(CID confirmed)"))
            } else {
                output::accent(model)
            };
            rows.push(("IPP      ", text, false));
        }
        if let Some(ref dev_id) = d.device_id {
            let short = abbreviate_device_id(dev_id);
            rows.push(("Device ID", output::dim(&short), false));
        }

        if rows.is_empty() {
            return;
        }

        render_card(buf, Some("Discovery"), |body| {
            body.push(card_line(""));
            let last = rows.len() - 1;
            for (i, (label, value, _)) in rows.iter().enumerate() {
                let branch = if i == last { "└─" } else { "├─" };
                let content = format!("  {} {} {}", branch, label, value);
                body.push(card_line(&content));
            }
            body.push(card_line(""));
        });
        buf.push('\n');
    }

    // ── Driver Resolution ───────────────────────────────────────────────

    fn render_resolution(&self, buf: &mut String) {
        if self.resolution.tiers.is_empty() {
            return;
        }

        // Max tier name width (plain text), clamped so it doesn't overflow.
        let name_width = self
            .resolution
            .tiers
            .iter()
            .map(|t| display_width(&t.name))
            .max()
            .unwrap_or(16)
            .max(16);

        render_card(buf, Some("Driver Resolution"), |body| {
            body.push(card_line(""));
            for tier in &self.resolution.tiers {
                let (icon_plain, icon_styled) = tier_icon(tier.status);

                // Pad tier name to name_width using visible-width math.
                let name_pad = name_width.saturating_sub(display_width(&tier.name));
                let name_styled = match tier.status {
                    TierStatus::Matched | TierStatus::Verified => output::accent(&tier.name),
                    _ => output::dim(&tier.name),
                };

                let detail_styled = match tier.status {
                    TierStatus::Matched => output::accent(&tier.detail),
                    TierStatus::Verified => output::ok(&tier.detail),
                    TierStatus::Failed | TierStatus::Skipped | TierStatus::Disabled => {
                        output::dim(&tier.detail)
                    }
                };

                // Icon (2 visual cols) + 2 spaces + name + pad + 2 spaces + detail
                let content = format!(
                    "  {}  {}{}  {}",
                    icon_styled,
                    name_styled,
                    " ".repeat(name_pad),
                    detail_styled,
                );
                body.push(card_line(&content));
                let _ = icon_plain; // reserved for width debugging

                // Verified tier child line with signer info.
                if matches!(tier.status, TierStatus::Verified)
                    && let Some(signer) = tier.signer.as_deref()
                {
                    let child = format!(
                        "        └─ {}  {} {} {}",
                        emoji("🛡️", "+"),
                        output::ok("verified"),
                        output::dim("·"),
                        output::dim(signer),
                    );
                    body.push(card_line(&child));
                }
            }
            body.push(card_line(""));
        });
        buf.push('\n');
    }

    // ── Install ─────────────────────────────────────────────────────────

    fn render_install(&self, buf: &mut String) {
        if self.install.steps.is_empty() {
            return;
        }

        let label_width = self
            .install
            .steps
            .iter()
            .map(|s| display_width(&s.label))
            .max()
            .unwrap_or(6)
            .max(6);

        render_card(buf, Some("Install"), |body| {
            body.push(card_line(""));
            let last = self.install.steps.len() - 1;
            for (i, step) in self.install.steps.iter().enumerate() {
                let branch = if i == last { "└─" } else { "├─" };
                let (_, icon_styled) = if step.ok {
                    tier_icon(TierStatus::Matched)
                } else {
                    tier_icon(TierStatus::Failed)
                };
                let label_pad = label_width.saturating_sub(display_width(&step.label));
                let content = format!(
                    "  {} {}  {}{}  {}",
                    branch,
                    icon_styled,
                    output::label(&step.label),
                    " ".repeat(label_pad),
                    step.value,
                );
                body.push(card_line(&content));
            }
            body.push(card_line(""));
        });
        buf.push('\n');
    }

    // ── Summary ─────────────────────────────────────────────────────────

    fn render_summary(&self, buf: &mut String) {
        let elapsed = format_duration(self.elapsed);
        let is_ipp_fallback = self
            .source_annotation
            .as_deref()
            .map(|s| s.to_ascii_lowercase().contains("ipp class driver"))
            .unwrap_or(false);

        render_card(buf, None, |body| {
            body.push(card_line(""));

            if self.success && is_ipp_fallback {
                // Warning: generic IPP fallback
                let opener = format!(
                    "  {}  {}",
                    emoji("⚠️", "?"),
                    output::warn("Installed via IPP Class Driver (generic fallback)"),
                );
                body.push(card_line(&opener));
                body.push(card_line(""));

                let name = self
                    .install
                    .steps
                    .iter()
                    .find(|s| s.label.eq_ignore_ascii_case("queue"))
                    .map(|s| s.value.as_str())
                    .or(self.discovery.snmp_model.as_deref())
                    .or(self.discovery.ipp_model.as_deref())
                    .unwrap_or("(unknown)");
                let driver = self
                    .install
                    .steps
                    .iter()
                    .find(|s| s.label.eq_ignore_ascii_case("driver"))
                    .map(|s| s.value.as_str())
                    .unwrap_or("Microsoft IPP Class Driver");
                let port = self
                    .install
                    .steps
                    .iter()
                    .find(|s| s.label.eq_ignore_ascii_case("port"))
                    .map(|s| s.value.as_str())
                    .unwrap_or("(unknown)");

                body.push(card_line(&format!("     Name:   {}", name)));
                body.push(card_line(&format!("     Driver: {}", driver)));
                body.push(card_line(&format!("     Port:   {}", port)));
                body.push(card_line(""));
                body.push(card_line(&output::dim(
                    "     Basic printing works. Vendor-specific features",
                )));
                body.push(card_line(&output::dim(
                    "     (duplex, trays, finishing) may not be available.",
                )));
                body.push(card_line(""));
                body.push(card_line(&output::dim(&format!(
                    "     Completed in {}",
                    elapsed
                ))));
            } else if self.success {
                // Happy path
                let opener = format!(
                    "  {}  {}",
                    emoji("✔️", "*"),
                    output::ok("Printer installed successfully"),
                );
                body.push(card_line(&opener));
                body.push(card_line(""));

                let name = self
                    .install
                    .steps
                    .iter()
                    .find(|s| s.label.eq_ignore_ascii_case("queue"))
                    .map(|s| s.value.as_str())
                    .or(self.discovery.snmp_model.as_deref())
                    .or(self.discovery.ipp_model.as_deref())
                    .unwrap_or("(unknown)");
                let driver = self
                    .install
                    .steps
                    .iter()
                    .find(|s| s.label.eq_ignore_ascii_case("driver"))
                    .map(|s| s.value.as_str())
                    .unwrap_or("(unknown)");
                let port = self
                    .install
                    .steps
                    .iter()
                    .find(|s| s.label.eq_ignore_ascii_case("port"))
                    .map(|s| s.value.as_str())
                    .unwrap_or("(unknown)");

                body.push(card_line(&format!("     Name:   {}", name)));
                body.push(card_line(&format!("     Driver: {}", driver)));
                body.push(card_line(&format!("     Port:   {}", port)));
                if let Some(src) = self.source_annotation.as_deref() {
                    body.push(card_line(&format!("     Source: {}", src)));
                }
                body.push(card_line(""));
                body.push(card_line(&output::dim(&format!(
                    "     Completed in {}",
                    elapsed
                ))));
            } else {
                // Failure path
                let err_msg = self
                    .error
                    .as_deref()
                    .unwrap_or("all tiers exhausted");
                let opener = format!(
                    "  {}  {}",
                    emoji("❌", "!"),
                    output::err_text(&format!("Install failed — {}", err_msg)),
                );
                body.push(card_line(&opener));
                body.push(card_line(""));

                let model = self
                    .discovery
                    .snmp_model
                    .as_deref()
                    .or(self.discovery.ipp_model.as_deref())
                    .unwrap_or("this printer");
                body.push(card_line("     No driver could be resolved for"));
                body.push(card_line(&format!(
                    "     {} at {}.",
                    model, self.discovery.ip
                )));
                body.push(card_line(""));
                body.push(card_line("     Try:"));
                body.push(card_line(&format!(
                    "       {} prinstall drivers {}",
                    emoji("·", "-"),
                    self.discovery.ip,
                )));
                body.push(card_line(&output::dim(
                    "         (inspect resolution tiers)",
                )));
                body.push(card_line(&format!(
                    "       {} prinstall add {} --driver \"<name>\"",
                    emoji("·", "-"),
                    self.discovery.ip,
                )));
                body.push(card_line(&output::dim(
                    "         (override with a specific driver)",
                )));
                body.push(card_line(&format!(
                    "       {} Verify the printer's IPP or LPD port is open",
                    emoji("·", "-"),
                )));
                body.push(card_line(""));
                body.push(card_line(&output::dim(&format!(
                    "     Failed after {}",
                    elapsed
                ))));
            }
            body.push(card_line(""));
        });
    }
}

// ── Card-rendering primitives ────────────────────────────────────────────────

/// Render a card with an optional title. The closure appends already-
/// formatted body lines (use `card_line` to wrap each one).
fn render_card(buf: &mut String, title: Option<&str>, content: impl FnOnce(&mut Vec<String>)) {
    // Top border
    match title {
        Some(t) => {
            // ╭──  Title  ─── ... ─╮
            let prefix = format!("╭──  {}  ", t);
            let prefix_width = display_width(&prefix);
            // Total dashes needed between corners = CARD_WIDTH
            // We've used prefix_width - 1 (subtracting the leading ╭) so far
            let used = prefix_width.saturating_sub(1); // minus the ╭
            let remaining = CARD_WIDTH.saturating_sub(used);
            let _ = writeln!(
                buf,
                "{}{}{}",
                output::header(&prefix),
                output::header(&"─".repeat(remaining)),
                output::header("╮"),
            );
        }
        None => {
            let _ = writeln!(
                buf,
                "{}{}{}",
                output::header("╭"),
                output::header(&"─".repeat(CARD_WIDTH)),
                output::header("╮"),
            );
        }
    }

    // Body
    let mut body: Vec<String> = Vec::new();
    content(&mut body);
    for line in body {
        buf.push_str(&line);
        buf.push('\n');
    }

    // Bottom border
    let _ = writeln!(
        buf,
        "{}{}{}",
        output::header("╰"),
        output::header(&"─".repeat(CARD_WIDTH)),
        output::header("╯"),
    );
}

/// Wrap a content string in `│ <content>  │` padded to CARD_WIDTH visible
/// columns. Handles ANSI color codes and double-wide emoji by measuring
/// visible width (not byte length).
fn card_line(content: &str) -> String {
    let vis = display_width(content);
    let pad = CARD_WIDTH.saturating_sub(vis);
    format!(
        "{}{}{}{}",
        output::header("│"),
        content,
        " ".repeat(pad),
        output::header("│"),
    )
}

// ── Width + icon + emoji helpers ─────────────────────────────────────────────

/// Compute the visible terminal width of a string, ignoring ANSI escape
/// sequences and treating emoji / East Asian Wide code points as 2 columns.
/// The Unicode variation selector U+FE0F is skipped so that `✔️` counts as 2,
/// matching how Windows conhost and most Linux terminals actually render it.
pub(crate) fn display_width(s: &str) -> usize {
    let mut width = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip an ANSI CSI escape: ESC [ ... <final byte 0x40-0x7E>
            if chars.peek() == Some(&'[') {
                chars.next();
                for cc in chars.by_ref() {
                    if (0x40..=0x7E).contains(&(cc as u32)) {
                        break;
                    }
                }
            } else {
                // ESC ( X and similar — consume 1 char
                chars.next();
            }
            continue;
        }
        // Variation selector — no width
        if c == '\u{FE0F}' || c == '\u{FE0E}' {
            continue;
        }
        // Zero-width joiner — no width
        if c == '\u{200D}' {
            continue;
        }
        // Combining marks (most common range)
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        // Emoji / symbol ranges commonly rendered as 2 columns.
        // Not exhaustive, but covers the set we use (🚀 ✔️ ❌ ⚠️ 🛡️ ℹ️).
        let code = c as u32;
        let wide = matches!(code,
            0x1F300..=0x1FAFF   // misc symbols, pictographs, emoji, etc.
            | 0x2600..=0x27BF   // misc symbols + dingbats (✔ ✗ ⚠ etc when paired w/ VS16)
            | 0x2B00..=0x2BFF   // arrows + misc symbols
            | 0x1100..=0x115F   // hangul jamo (wide)
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
        );
        // Special case: ✔ ✗ ⚠ ❌ ℹ when followed by VS16 render as emoji (2 cols).
        // The ❌ U+274C already falls in the 0x2600-0x27BF range, as does ✔ U+2714.
        // Without VS16 they might render narrow, but in practice conhost + most
        // modern terminals render them wide when we emit them here.
        width += if wide { 2 } else { 1 };
    }
    width
}

/// Decide whether to emit emoji or ASCII fallback. Honors the
/// `PRINSTALL_NO_EMOJI` env var for consoles that can't render Unicode
/// presentation forms cleanly. Also honors a thread-local override that
/// the test suite uses to avoid racing on process-global env state.
fn emoji_enabled() -> bool {
    #[cfg(test)]
    {
        if let Some(v) = tests::test_emoji_override() {
            return v;
        }
    }
    std::env::var_os("PRINSTALL_NO_EMOJI").is_none()
}

/// Pick emoji or ASCII fallback based on [`emoji_enabled`].
///
/// Takes the desired glyph and its ASCII fallback, both as `&'static str`.
/// Returns one or the other based on env-var detection.
fn emoji(glyph: &'static str, fallback: &'static str) -> &'static str {
    if emoji_enabled() { glyph } else { fallback }
}

/// Return (plain-icon, styled-icon) for a tier status. The plain icon is
/// reserved for future width debugging; styled is what goes in the card.
fn tier_icon(status: TierStatus) -> (&'static str, String) {
    match status {
        TierStatus::Verified => ("✔️", output::ok(emoji("✔️", "*"))),
        TierStatus::Matched => ("✔️", output::ok(emoji("✔️", "*"))),
        TierStatus::Failed => ("❌", output::err_text(emoji("❌", "!"))),
        TierStatus::Skipped => ("❌", output::dim(emoji("❌", "!"))),
        TierStatus::Disabled => ("—", output::dim("—")),
    }
}

// ── Misc helpers ─────────────────────────────────────────────────────────────

/// Format a duration as a human-friendly string.
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let mins = secs as u64 / 60;
        let remaining = secs - (mins as f64 * 60.0);
        format!("{}m {:.0}s", mins, remaining)
    }
}

/// Pull MFG + MDL from a 1284 device ID for compact display.
fn abbreviate_device_id(dev_id: &str) -> String {
    let mut mfg = None;
    let mut mdl = None;
    for part in dev_id.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("MFG:").or_else(|| part.strip_prefix("MANUFACTURER:")) {
            mfg = Some(v.trim());
        }
        if let Some(v) = part.strip_prefix("MDL:").or_else(|| part.strip_prefix("MODEL:")) {
            mdl = Some(v.trim());
        }
    }
    match (mfg, mdl) {
        (Some(m), Some(d)) => format!("{m} {d}"),
        (Some(m), None) => m.to_string(),
        _ => dev_id.to_string(),
    }
}

/// Calculate how many extra bytes ANSI codes add vs. the visible length.
/// Used for column alignment with format padding. Preserved for existing
/// callsites — new code should prefer [`display_width`].
#[allow(dead_code)]
fn ansi_overhead(styled: &str, visible_len: usize) -> usize {
    styled.len().saturating_sub(visible_len)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static EMOJI_OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
    }

    pub(super) fn test_emoji_override() -> Option<bool> {
        EMOJI_OVERRIDE.with(|c| c.get())
    }

    /// RAII guard that force-disables emoji in the current thread for the
    /// duration of its lifetime. Restores the prior setting on drop.
    struct EmojiGuard(Option<bool>);
    impl EmojiGuard {
        fn disable() -> Self {
            let prev = EMOJI_OVERRIDE.with(|c| c.replace(Some(false)));
            Self(prev)
        }
    }
    impl Drop for EmojiGuard {
        fn drop(&mut self) {
            let prev = self.0;
            EMOJI_OVERRIDE.with(|c| c.set(prev));
        }
    }

    fn sample_report() -> InstallReport {
        let mut report = InstallReport::new("10.10.20.16");
        report.discovery.snmp_model = Some("HP PageWide Pro 477dw MFP".into());
        report.discovery.ipp_model = Some("HP PageWide Pro 477dw MFP".into());
        report.discovery.ipp_cid = Some("HP PageWide Pro".into());
        report.discovery.device_id = Some("MFG:HP;MDL:PageWide Pro 477dw;CID:HP PageWide Pro".into());

        report.resolution.add_tier("Local store", TierStatus::Failed, "no match");
        report.resolution.add_tier(
            "Manufacturer",
            TierStatus::Verified,
            "HP Universal Printing PCL 6",
        );
        report.resolution.set_last_signer("Microsoft WHCP");

        report.install.add_step("Port", "IP_10.10.20.16", true);
        report.install.add_step("Driver", "HP Universal Printing PCL 6", true);
        report.install.add_step("Queue", "HP PageWide Pro 477dw MFP", true);

        report.elapsed = Duration::from_secs_f64(10.5);
        report.source_annotation = Some("Manufacturer · verified".into());
        report.success = true;
        report
    }

    #[test]
    fn format_duration_short() {
        assert_eq!(format_duration(Duration::from_secs_f64(3.24)), "3.2s");
        assert_eq!(format_duration(Duration::from_secs_f64(0.5)), "0.5s");
    }

    #[test]
    fn format_duration_long() {
        assert_eq!(format_duration(Duration::from_secs(125)), "2m 5s");
    }

    #[test]
    fn abbreviate_device_id_parses_mfg_mdl() {
        let did = "MFG:Brother;MDL:HL-L8260CDW;CID:Brother Laser Type1;";
        assert_eq!(abbreviate_device_id(did), "Brother HL-L8260CDW");
    }

    #[test]
    fn abbreviate_device_id_fallback() {
        assert_eq!(abbreviate_device_id("garbage"), "garbage");
    }

    #[test]
    fn ansi_overhead_zero_when_plain() {
        assert_eq!(ansi_overhead("hello", 5), 0);
    }

    #[test]
    fn display_width_plain_ascii() {
        assert_eq!(display_width("hello"), 5);
    }

    #[test]
    fn display_width_strips_ansi() {
        // "\x1b[32mhello\x1b[0m" — should be 5 visible cols
        assert_eq!(display_width("\x1b[32mhello\x1b[0m"), 5);
    }

    #[test]
    fn display_width_counts_emoji_as_two() {
        assert_eq!(display_width("🚀"), 2);
        assert_eq!(display_width("❌"), 2);
        // ✔️ is U+2714 + U+FE0F — should count as 2
        assert_eq!(display_width("✔️"), 2);
    }

    #[test]
    fn renders_rounded_card_top_border() {
        let report = sample_report();
        let mut buf = String::new();
        report.render_to_string(&mut buf);
        assert!(buf.contains("╭"), "expected rounded top-left corner");
        assert!(buf.contains("╮"), "expected rounded top-right corner");
        assert!(buf.contains("╰"), "expected rounded bottom-left corner");
        assert!(buf.contains("╯"), "expected rounded bottom-right corner");
        // Angular card corners (┌┐┘) must not appear. Note that └ is used
        // inline for tree branches (`└─ Device ID ...`), so we don't ban it
        // here — but ┌ ┐ ┘ are strictly card-corner-only.
        assert!(!buf.contains("┌"), "angular card corners must not appear (┌)");
        assert!(!buf.contains("┐"), "angular card corners must not appear (┐)");
        assert!(!buf.contains("┘"), "angular card corners must not appear (┘)");
    }

    #[test]
    fn renders_section_titles() {
        let report = sample_report();
        let mut buf = String::new();
        report.render_to_string(&mut buf);
        assert!(buf.contains("Discovery"));
        assert!(buf.contains("Driver Resolution"));
        assert!(buf.contains("Install"));
    }

    #[test]
    fn renders_tier_icons_by_status() {
        let mut report = InstallReport::new("10.0.0.2");
        report.resolution.add_tier("Local", TierStatus::Failed, "no match");
        report.resolution.add_tier("Man", TierStatus::Skipped, "skipped");
        report.resolution.add_tier("Cat", TierStatus::Disabled, "--no-catalog");
        report.resolution.add_tier("SDI", TierStatus::Matched, "got it");
        report.resolution.add_tier("IPP", TierStatus::Verified, "verified");
        report.success = true;
        report.source_annotation = Some("SDI".into());

        let mut buf = String::new();
        report.render_to_string(&mut buf);
        // ❌ icon for Failed + Skipped (at least one)
        assert!(buf.contains("❌"), "expected ❌ for failed tier");
        // ✔️ icon for Matched/Verified
        assert!(buf.contains("✔"), "expected ✔ for matched/verified tier");
        // em dash for Disabled
        assert!(buf.contains("—"), "expected — for disabled tier");
    }

    #[test]
    fn verified_tier_shows_shield_child_line() {
        let report = sample_report();
        let mut buf = String::new();
        report.render_to_string(&mut buf);
        assert!(
            buf.contains("🛡") || buf.contains("+"),
            "expected 🛡️ or + child line on verified tier"
        );
        assert!(buf.contains("verified"));
        assert!(buf.contains("Microsoft WHCP"));
    }

    #[test]
    fn success_summary_uses_green_check() {
        let report = sample_report();
        let mut buf = String::new();
        report.render_to_string(&mut buf);
        assert!(buf.contains("✔"), "expected ✔ in summary");
        assert!(buf.contains("installed successfully"));
        assert!(buf.contains("Completed in"));
    }

    #[test]
    fn fallback_summary_uses_warning() {
        let mut report = InstallReport::new("10.0.0.3");
        report.discovery.snmp_model = Some("Canon imageCLASS MF743Cdw".into());
        report.install.add_step("Port", "IP_10.0.0.3", true);
        report.install.add_step("Driver", "Microsoft IPP Class Driver", true);
        report.install.add_step("Queue", "Canon imageCLASS MF743Cdw (IPP)", true);
        report.source_annotation = Some("IPP Class Driver (generic fallback)".into());
        report.success = true;
        report.elapsed = Duration::from_secs_f64(32.2);

        let mut buf = String::new();
        report.render_to_string(&mut buf);
        assert!(buf.contains("⚠") || buf.contains("?  "), "expected ⚠️ in fallback summary");
        assert!(buf.contains("generic fallback"));
    }

    #[test]
    fn failure_summary_uses_red_x() {
        let mut report = InstallReport::new("10.10.20.99");
        report.discovery.snmp_model = Some("Canon imageCLASS MF743Cdw".into());
        report.resolution.add_tier("Local store", TierStatus::Failed, "no match");
        report.resolution.add_tier("Manufacturer", TierStatus::Failed, "no URL for Canon");
        report.resolution.add_tier("Catalog", TierStatus::Failed, "no CID for query");
        report.resolution.add_tier("SDI Origin", TierStatus::Failed, "no HWID match in indexes");
        report.resolution.add_tier("IPP Class Driver", TierStatus::Failed, "port 631 not reachable");
        report.success = false;
        report.error = Some("all tiers exhausted".into());
        report.elapsed = Duration::from_secs_f64(6.2);

        let mut buf = String::new();
        report.render_to_string(&mut buf);
        assert!(buf.contains("❌"), "expected ❌ in failure summary");
        assert!(buf.contains("tiers exhausted"));
        assert!(buf.contains("Failed after"));
        assert!(buf.contains("Try:"), "expected troubleshooting guidance");
    }

    #[test]
    fn emoji_fallback_when_env_set() {
        // Uses a thread-local override instead of actual env var mutation —
        // process-global env mutation races with parallel tests. The runtime
        // `PRINSTALL_NO_EMOJI` env var still works in production.
        let _guard = EmojiGuard::disable();
        let report = sample_report();
        let mut buf = String::new();
        report.render_to_string(&mut buf);

        // Header rocket should fall back to >>, success check to *.
        assert!(!buf.contains("🚀"), "🚀 should be replaced when emoji disabled");
        assert!(!buf.contains("✔️"), "✔️ should be replaced when emoji disabled");
        assert!(buf.contains(">>"), "expected >> fallback for rocket");
        assert!(buf.contains('*'), "expected * fallback for check");
    }

    #[test]
    fn card_line_pads_to_width() {
        let line = card_line("hello");
        // Should start with │ and end with │
        assert!(line.starts_with("│") || line.contains("│"));
        assert!(line.ends_with("│"));
        // display_width of the whole line (minus the borders) should be CARD_WIDTH
        let vis = display_width(&line);
        assert_eq!(vis, CARD_WIDTH + 2, "line visible width = CARD_WIDTH + 2 borders");
    }

    #[test]
    fn report_renders_without_panic() {
        sample_report().render();
    }

    #[test]
    fn report_renders_failure() {
        let mut report = InstallReport::new("10.0.0.1");
        report.elapsed = Duration::from_secs(5);
        report.success = false;
        report.error = Some("no driver available".into());
        report.render();
    }

    #[test]
    fn tier_winner_tracking() {
        let mut res = ResolutionPhase::default();
        res.add_tier("Local", TierStatus::Failed, "no match");
        res.add_tier("SDI", TierStatus::Matched, "found it");
        assert_eq!(res.winner_idx, Some(1));
    }

    #[test]
    fn verified_tier_is_recognized_as_winner() {
        let mut res = ResolutionPhase::default();
        res.add_tier("Local", TierStatus::Failed, "no match");
        res.add_tier("SDI Origin", TierStatus::Verified, "HP UPD [verified]");
        assert_eq!(res.winner_idx, Some(1));
    }
}

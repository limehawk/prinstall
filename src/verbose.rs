//! Structured verbose output for `prinstall add`.
//!
//! Instead of streaming `eprintln!("[module] text")` lines as each step
//! happens, the add flow builds an [`InstallReport`] incrementally and
//! renders it as a single structured block when the install completes.
//! This gives admins a scannable, color-coded summary with clear sections
//! instead of a raw implementation log.
//!
//! Design: Discovery → Driver Resolution → Install → Summary.
//! Box-drawing section headers, tier cascade as a status list, per-step
//! check marks, timing, and a one-line summary with the winning source.

use std::fmt::Write;
use std::time::Duration;

use crate::output;

// ── Section rendering constants ─────────────────────────────────────────────

const RULE_WIDTH: usize = 46;

/// Thin horizontal rule for section headers.
fn section_header(title: &str) -> String {
    // ━━ Title ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let prefix = format!("━━ {title} ");
    let fill = RULE_WIDTH.saturating_sub(prefix.chars().count());
    let line = format!("{prefix}{}", "━".repeat(fill));
    output::header(&line)
}

/// Full-width closing rule.
fn closing_rule() -> String {
    output::header(&"━".repeat(RULE_WIDTH))
}

// ── Phase data types ────────────────────────────────────────────────────────

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
}

#[derive(Clone, Copy, PartialEq)]
pub enum TierStatus {
    /// ✗ — tried and failed or no match
    Failed,
    /// ✗ — not attempted (skipped by flag or prior hit)
    Skipped,
    /// ★ — this tier won
    Matched,
    /// - — explicitly disabled via flag
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
        if status == TierStatus::Matched && self.winner_idx.is_none() {
            self.winner_idx = Some(self.tiers.len());
        }
        self.tiers.push(TierResult {
            name: name.to_string(),
            status,
            detail: detail.to_string(),
        });
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
        }
    }

    /// Render the full structured report to stderr.
    pub fn render(&self) {
        let mut buf = String::with_capacity(1024);
        buf.push('\n');

        self.render_discovery(&mut buf);
        self.render_resolution(&mut buf);
        self.render_install(&mut buf);
        self.render_summary(&mut buf);

        eprint!("{buf}");
    }

    // ── Discovery ───────────────────────────────────────────────────────

    fn render_discovery(&self, buf: &mut String) {
        let _ = writeln!(buf, "{}", section_header("Discovery"));
        let d = &self.discovery;

        if let Some(ref model) = d.snmp_model {
            let _ = writeln!(buf, "  {}  {} {}", output::dim("SNMP"), output::dim("→"), output::accent(model));
        } else {
            let _ = writeln!(buf, "  {}  {} {}", output::dim("SNMP"), output::dim("→"), output::dim("(no response)"));
        }

        if let Some(ref model) = d.ipp_model {
            let cid_note = if d.ipp_cid.is_some() {
                format!(" {}", output::dim("(CID confirmed)"))
            } else {
                String::new()
            };
            let _ = writeln!(buf, "  {}   {} {}{}", output::dim("IPP"), output::dim("→"), output::accent(model), cid_note);
        }

        if let Some(ref dev_id) = d.device_id {
            // Show just MFG + MDL from the device ID for readability
            let short = abbreviate_device_id(dev_id);
            let _ = writeln!(buf, "  {}    {}", output::dim("DID"), output::dim(&short));
        }

        buf.push('\n');
    }

    // ── Driver Resolution ───────────────────────────────────────────────

    fn render_resolution(&self, buf: &mut String) {
        let _ = writeln!(buf, "{}", section_header("Driver Resolution"));

        let name_width = self.resolution.tiers.iter()
            .map(|t| t.name.len())
            .max()
            .unwrap_or(12)
            .max(12);

        for (i, tier) in self.resolution.tiers.iter().enumerate() {
            let is_winner = self.resolution.winner_idx == Some(i);
            let prefix = if is_winner { "▸" } else { " " };

            let (icon, detail_str) = match tier.status {
                TierStatus::Failed => (
                    output::dim("✗"),
                    output::dim(&tier.detail),
                ),
                TierStatus::Skipped => (
                    output::dim("✗"),
                    output::dim(&tier.detail),
                ),
                TierStatus::Matched => (
                    output::accent("★"),
                    output::accent(&tier.detail),
                ),
                TierStatus::Disabled => (
                    output::dim("-"),
                    output::dim(&tier.detail),
                ),
            };

            let name_str = if is_winner {
                output::accent(&tier.name)
            } else {
                output::dim(&tier.name)
            };

            let _ = writeln!(
                buf,
                "  {prefix} {name:<width$} {icon} {detail}",
                name = name_str,
                width = name_width + ansi_overhead(&name_str, tier.name.len()),
                icon = icon,
                detail = detail_str,
            );

            // If this is the winner and has staging info, show it indented
            if is_winner && tier.detail.contains('[') {
                // The detail already shows the driver + source tag
            }
        }

        buf.push('\n');
    }

    // ── Install ─────────────────────────────────────────────────────────

    fn render_install(&self, buf: &mut String) {
        if self.install.steps.is_empty() {
            return;
        }
        let _ = writeln!(buf, "{}", section_header("Install"));

        let label_width = self.install.steps.iter()
            .map(|s| s.label.len())
            .max()
            .unwrap_or(6);
        let value_width = self.install.steps.iter()
            .map(|s| s.value.len())
            .max()
            .unwrap_or(20);

        for step in &self.install.steps {
            let check = if step.ok {
                output::ok("✓")
            } else {
                output::err_text("✗")
            };
            let _ = writeln!(
                buf,
                "  {:<lw$} {:<vw$}  {check}",
                output::label(&step.label),
                step.value,
                lw = label_width + ansi_overhead(&output::label(&step.label), step.label.len()),
                vw = value_width,
            );
        }

        buf.push('\n');
    }

    // ── Summary ─────────────────────────────────────────────────────────

    fn render_summary(&self, buf: &mut String) {
        let _ = writeln!(buf, "{}", closing_rule());

        let elapsed = format_duration(self.elapsed);

        if self.success {
            let source = self.source_annotation.as_deref().unwrap_or("direct");
            let _ = writeln!(
                buf,
                "  {} Installed in {} via {}",
                output::ok("✓"),
                output::accent(&elapsed),
                output::accent(source),
            );
        } else {
            let err_msg = self.error.as_deref().unwrap_or("unknown error");
            let _ = writeln!(
                buf,
                "  {} Failed after {} — {}",
                output::err_text("✗"),
                output::dim(&elapsed),
                output::err_text(err_msg),
            );
        }

        let _ = writeln!(buf, "{}", closing_rule());
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

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
/// Used for column alignment with format padding.
fn ansi_overhead(styled: &str, visible_len: usize) -> usize {
    styled.len().saturating_sub(visible_len)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
    fn section_header_contains_title() {
        // Color disabled in tests, so we get plain text
        let h = section_header("Discovery");
        assert!(h.contains("Discovery"));
        assert!(h.contains("━━"));
    }

    #[test]
    fn report_renders_without_panic() {
        let mut report = InstallReport::new("192.168.1.50");
        report.discovery.snmp_model = Some("Brother MFC-L2750DW series".into());
        report.discovery.ipp_model = Some("Brother Laser Type1".into());
        report.discovery.ipp_cid = Some("Brother Laser Type1".into());
        report.discovery.device_id = Some("MFG:Brother;MDL:MFC-L2750DW;CID:Brother Laser Type1".into());

        report.resolution.add_tier("Local store", TierStatus::Failed, "no match");
        report.resolution.add_tier("Manufacturer", TierStatus::Failed, "no URL for Brother");
        report.resolution.add_tier("Catalog", TierStatus::Skipped, "not attempted (SDI hit first)");
        report.resolution.add_tier("SDI Origin", TierStatus::Matched, "Brother Laser Type1 Class Driver [cached]");

        report.install.add_step("Port", "IP_192.168.1.50", true);
        report.install.add_step("Driver", "Brother Laser Type1 Class Driver", true);
        report.install.add_step("Queue", "Brother MFC-L2750DW series", true);

        report.elapsed = Duration::from_secs_f64(3.2);
        report.source_annotation = Some("SDI [cached]".into());
        report.success = true;

        // Just verify it doesn't panic — output goes to stderr
        report.render();
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
}

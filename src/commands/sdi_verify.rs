//! `prinstall sdi verify` — Authenticode signature verification for SDI driver packs.
//!
//! Walks the SDI extraction cache (`sdi/extracted/<pack>/`), finds every `.cat`
//! catalog file, and verifies its Authenticode signature via PowerShell's
//! `Get-AuthenticodeSignature`. Reports per-manufacturer totals and a grand
//! total so the admin knows whether the drivers are untampered vendor binaries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::executor::PsExecutor;
use crate::installer::powershell::escape_ps_string;
use crate::output;

// ── Data types ──────────────────────────────────────────────────────────────

/// Signature status from Get-AuthenticodeSignature.
#[derive(Debug, Clone, PartialEq)]
pub enum SigStatus {
    Valid,
    NotSigned,
    HashMismatch,
    NotTrusted,
    UnknownError,
    Other(String),
}

impl SigStatus {
    fn from_ps(s: &str) -> Self {
        // PS returns a numeric enum that ConvertTo-Json renders as an integer.
        // Status 0 = Valid, 1 = UnknownError, 2 = NotSigned, 3 = HashMismatch,
        // 4 = NotTrusted, 5 = NotSupportedFileFormat. Also handle string forms.
        match s.trim() {
            "0" | "Valid" => Self::Valid,
            "1" | "UnknownError" => Self::UnknownError,
            "2" | "NotSigned" => Self::NotSigned,
            "3" | "HashMismatch" => Self::HashMismatch,
            "4" | "NotTrusted" => Self::NotTrusted,
            other => Self::Other(other.to_string()),
        }
    }

    fn is_valid(&self) -> bool {
        *self == Self::Valid
    }

    fn is_unsigned(&self) -> bool {
        *self == Self::NotSigned
    }
}

/// Raw JSON shape from Get-AuthenticodeSignature | ConvertTo-Json.
#[derive(Debug, serde::Deserialize)]
struct RawSigResult {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Status")]
    status: serde_json::Value,
    #[serde(rename = "StatusMessage", default)]
    status_message: String,
    #[serde(rename = "SignerCertificate")]
    signer_certificate: Option<RawCert>,
}

#[derive(Debug, serde::Deserialize)]
struct RawCert {
    #[serde(rename = "Subject", default)]
    subject: String,
    #[serde(rename = "Issuer", default)]
    issuer: String,
}

/// Processed result for one .cat file.
pub struct CatResult {
    pub path: PathBuf,
    pub status: SigStatus,
    pub signer: Option<String>,
    pub issuer_ca: Option<String>,
}

/// Per-manufacturer stats.
pub struct ManufacturerStats {
    pub name: String,
    pub total: usize,
    pub valid: usize,
    pub unsigned: usize,
    pub invalid: usize,
    pub signers: Vec<String>,
}

/// JSON-serializable report.
#[derive(serde::Serialize)]
pub struct VerifyReport {
    pub pack_name: String,
    pub total_cats: usize,
    pub total_valid: usize,
    pub total_unsigned: usize,
    pub total_invalid: usize,
    pub manufacturers: Vec<ManufacturerReport>,
}

#[derive(serde::Serialize)]
pub struct ManufacturerReport {
    pub name: String,
    pub total: usize,
    pub valid: usize,
    pub unsigned: usize,
    pub invalid: usize,
    pub signers: Vec<String>,
}

/// Summary of verification status for an entire driver pack.
///
/// Reduces a list of per-cat `CatResult`s to a single outcome. The SDI
/// install gate and the `drivers` command both use `is_safe_to_install`
/// to decide whether to offer / install a pack.
#[derive(Debug, Clone)]
pub enum PackVerifyOutcome {
    /// Every `.cat` in the pack has a valid Authenticode signature.
    Verified {
        valid: usize,
        signers: Vec<String>,
    },
    /// At least one `.cat` is missing a signature. Install must skip.
    Unsigned {
        unsigned: usize,
        total: usize,
    },
    /// At least one `.cat` has a bad signature (hash mismatch, not trusted).
    Invalid {
        invalid: usize,
        total: usize,
        first_reason: String,
    },
    /// Pack has no `.cat` files at all — treat as untrustworthy.
    NoCatalogs,
}

impl PackVerifyOutcome {
    /// Reduce per-cat results to a pack-level outcome.
    /// Priority: Invalid > Unsigned > Verified > NoCatalogs.
    pub fn from_cat_results(results: &[CatResult]) -> Self {
        if results.is_empty() {
            return Self::NoCatalogs;
        }
        let total = results.len();

        let invalid_count = results
            .iter()
            .filter(|r| {
                matches!(
                    r.status,
                    SigStatus::HashMismatch
                        | SigStatus::NotTrusted
                        | SigStatus::UnknownError
                        | SigStatus::Other(_)
                )
            })
            .count();
        if invalid_count > 0 {
            let first_reason = results
                .iter()
                .find(|r| {
                    !matches!(r.status, SigStatus::Valid | SigStatus::NotSigned)
                })
                .map(|r| format!("{:?}", r.status))
                .unwrap_or_else(|| "unknown".to_string());
            return Self::Invalid {
                invalid: invalid_count,
                total,
                first_reason,
            };
        }

        let unsigned_count = results.iter().filter(|r| r.status.is_unsigned()).count();
        if unsigned_count > 0 {
            return Self::Unsigned {
                unsigned: unsigned_count,
                total,
            };
        }

        let valid = results.iter().filter(|r| r.status.is_valid()).count();
        let mut signers: Vec<String> = results
            .iter()
            .filter_map(|r| r.signer.clone())
            .collect();
        signers.sort();
        signers.dedup();
        Self::Verified { valid, signers }
    }

    /// True when the pack is safe to install (verified signature).
    pub fn is_safe_to_install(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Walk a pack directory for `.cat` files and verify each.
/// Wraps `verify_cats` + `from_cat_results` for the common install-gate path.
pub fn verify_pack_directory(
    executor: &dyn PsExecutor,
    pack_dir: &Path,
    verbose: bool,
) -> PackVerifyOutcome {
    let cats = find_cat_files(pack_dir);
    if cats.is_empty() {
        return PackVerifyOutcome::NoCatalogs;
    }
    let results = verify_cats(executor, &cats, verbose);
    PackVerifyOutcome::from_cat_results(&results)
}

/// Run the `prinstall sdi verify` command.
pub fn run(executor: &dyn PsExecutor, json: bool, verbose: bool) {
    let packs = match walk_extraction_cache() {
        Ok(p) if p.is_empty() => {
            if json {
                println!("[]");
            } else {
                eprintln!();
                eprintln!("  {}  No extracted drivers found.", output::dim("○"));
                eprintln!();
                eprintln!("  Run {} to install a printer first, or", output::accent("prinstall add <ip>"));
                eprintln!("  run {} to pre-cache the full pack.", output::accent("prinstall sdi prefetch"));
                eprintln!();
            }
            return;
        }
        Ok(p) => p,
        Err(e) => {
            if json {
                println!("[]");
            } else {
                eprintln!("\n{}", output::err_text(&e));
            }
            return;
        }
    };

    let mut all_reports = Vec::new();

    for (pack_dir, cats) in &packs {
        let pack_name = pack_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        if !json {
            print_header(pack_name, cats.len());
        }

        let results = verify_cats(executor, cats, verbose);
        let stats = aggregate_by_manufacturer(pack_dir, &results);

        if json {
            let report = build_report(pack_name, &stats);
            all_reports.push(report);
        } else {
            print_dashboard(&stats);
        }
    }

    if json {
        if all_reports.len() == 1 {
            println!("{}", serde_json::to_string_pretty(&all_reports[0]).unwrap_or_default());
        } else {
            println!("{}", serde_json::to_string_pretty(&all_reports).unwrap_or_default());
        }
    }
}

// ── Filesystem walk ─────────────────────────────────────────────────────────

/// Find all pack directories and their .cat files in the extraction cache.
fn walk_extraction_cache() -> Result<Vec<(PathBuf, Vec<PathBuf>)>, String> {
    let extracted_dir = crate::paths::sdi_dir().join("extracted");
    if !extracted_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut packs = Vec::new();
    let entries = std::fs::read_dir(&extracted_dir)
        .map_err(|e| format!("Failed to read extraction cache: {e}"))?;

    for entry in entries.flatten() {
        let pack_dir = entry.path();
        if !pack_dir.is_dir() {
            continue;
        }
        let cats = find_cat_files(&pack_dir);
        if !cats.is_empty() {
            packs.push((pack_dir, cats));
        }
    }

    Ok(packs)
}

/// Recursively find all .cat files under a directory.
fn find_cat_files(dir: &Path) -> Vec<PathBuf> {
    let mut cats = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                cats.extend(find_cat_files(&path));
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cat")) {
                cats.push(path);
            }
        }
    }
    cats
}

// ── PowerShell verification ─────────────────────────────────────────────────

const BATCH_SIZE: usize = 200;

/// Verify a batch of .cat files via Get-AuthenticodeSignature.
fn verify_cats(executor: &dyn PsExecutor, cats: &[PathBuf], verbose: bool) -> Vec<CatResult> {
    let mut results = Vec::with_capacity(cats.len());

    for chunk in cats.chunks(BATCH_SIZE) {
        let cmd = build_ps_command(chunk);
        if verbose {
            eprintln!(
                "{} Verifying batch of {} .cat files",
                output::vpfx("sdi"),
                chunk.len()
            );
        }
        let ps_result = executor.run(&cmd);
        if !ps_result.success || ps_result.stdout.trim().is_empty() {
            // PS failed — mark all in this batch as unknown
            for path in chunk {
                results.push(CatResult {
                    path: path.clone(),
                    status: SigStatus::UnknownError,
                    signer: None,
                    issuer_ca: None,
                });
            }
            continue;
        }
        results.extend(parse_raw_results(&ps_result.stdout, chunk));
    }

    results
}

/// Build the PowerShell command for a batch of .cat paths.
fn build_ps_command(paths: &[PathBuf]) -> String {
    let path_list: Vec<String> = paths
        .iter()
        .map(|p| format!("'{}'", escape_ps_string(&p.to_string_lossy())))
        .collect();
    format!(
        "ConvertTo-Json -InputObject @(Get-AuthenticodeSignature -LiteralPath @({})) -Depth 4 -Compress",
        path_list.join(",")
    )
}

/// Parse the JSON output from Get-AuthenticodeSignature.
fn parse_raw_results(stdout: &str, fallback_paths: &[PathBuf]) -> Vec<CatResult> {
    let raw: Vec<RawSigResult> = match serde_json::from_str(stdout) {
        Ok(v) => v,
        Err(_) => {
            // Fallback: mark everything as unknown
            return fallback_paths
                .iter()
                .map(|p| CatResult {
                    path: p.clone(),
                    status: SigStatus::UnknownError,
                    signer: None,
                    issuer_ca: None,
                })
                .collect();
        }
    };

    raw.into_iter()
        .map(|r| {
            let status_str = match &r.status {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => "UnknownError".to_string(),
            };
            let status = SigStatus::from_ps(&status_str);
            let signer = r
                .signer_certificate
                .as_ref()
                .and_then(|c| extract_cn(&c.subject));
            let issuer_ca = r
                .signer_certificate
                .as_ref()
                .and_then(|c| extract_cn(&c.issuer));
            CatResult {
                path: PathBuf::from(r.path),
                status,
                signer,
                issuer_ca,
            }
        })
        .collect()
}

/// Extract the CN= component from an X.509 Subject/Issuer string.
/// X.509 fields are separated by `, ` followed by a key like `O=`, `C=`, etc.
/// A raw `,` inside a value (e.g. "Brother Industries, Ltd.") is NOT a field separator.
fn extract_cn(subject: &str) -> Option<String> {
    // Find CN= at the start or after ", "
    let cn_start = if subject.starts_with("CN=") {
        Some(3)
    } else {
        subject.find(", CN=").map(|i| i + 5)
    };

    let start = cn_start?;
    let rest = &subject[start..];

    // Find the next field separator: ", " followed by a known key
    let end = rest
        .find(", O=")
        .or_else(|| rest.find(", OU="))
        .or_else(|| rest.find(", C="))
        .or_else(|| rest.find(", L="))
        .or_else(|| rest.find(", S="))
        .or_else(|| rest.find(", ST="))
        .or_else(|| rest.find(", E="))
        .unwrap_or(rest.len());

    let cn = rest[..end].trim();
    if cn.is_empty() {
        None
    } else {
        Some(cn.to_string())
    }
}

// ── Aggregation ─────────────────────────────────────────────────────────────

/// Group results by the first directory component under the pack dir.
fn aggregate_by_manufacturer(pack_dir: &Path, results: &[CatResult]) -> Vec<ManufacturerStats> {
    let mut groups: HashMap<String, Vec<&CatResult>> = HashMap::new();

    for result in results {
        let manufacturer = result
            .path
            .strip_prefix(pack_dir)
            .ok()
            .and_then(|rel| rel.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("unknown")
            .to_string();
        groups.entry(manufacturer).or_default().push(result);
    }

    let mut stats: Vec<ManufacturerStats> = groups
        .into_iter()
        .map(|(name, cats)| {
            let total = cats.len();
            let valid = cats.iter().filter(|c| c.status.is_valid()).count();
            let unsigned = cats.iter().filter(|c| c.status.is_unsigned()).count();
            let invalid = total - valid - unsigned;
            let mut signers: Vec<String> = cats
                .iter()
                .filter_map(|c| c.signer.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            signers.sort();
            ManufacturerStats {
                name,
                total,
                valid,
                unsigned,
                invalid,
                signers,
            }
        })
        .collect();

    stats.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    stats
}

// ── Output ──────────────────────────────────────────────────────────────────

const DASH_WIDTH: usize = 56;

fn print_header(pack_name: &str, cat_count: usize) {
    eprintln!();
    eprintln!("{}", output::header(&"━".repeat(DASH_WIDTH)));
    eprintln!("  {}  {}", output::accent("SDI Driver Verification"), output::dim("Authenticode .cat check"));
    eprintln!("{}", output::header(&"━".repeat(DASH_WIDTH)));
    eprintln!();
    eprintln!("  {}  {}", output::label("Pack:"), output::accent(pack_name));
    eprintln!("  {}  {} .cat files found", output::label("Scan:"), output::accent(&cat_count.to_string()));
    eprintln!();
}

fn print_dashboard(stats: &[ManufacturerStats]) {
    // ── Per-manufacturer section ────────────────────────────────────
    eprintln!("{}", output::header(&format!("━━ Manufacturers {}", "━".repeat(DASH_WIDTH - 18))));
    eprintln!();

    let name_w = stats.iter().map(|s| s.name.len()).max().unwrap_or(8).max(8);
    let max_total = stats.iter().map(|s| s.total).max().unwrap_or(1).max(1);
    let bar_max = 20; // max width of the progress bar

    for s in stats {
        // Status icon
        let icon = if s.invalid > 0 {
            output::err_text("✗")
        } else if s.unsigned > 0 {
            output::warn("○")
        } else {
            output::ok("✓")
        };

        // Progress bar
        let bar_filled = (s.valid * bar_max) / max_total;
        let bar_empty = bar_max - bar_filled;
        let bar = format!(
            "{}{}",
            output::ok(&"█".repeat(bar_filled)),
            output::dim(&"░".repeat(bar_empty)),
        );

        // Count
        let count_str = format!("{}/{}", s.valid, s.total);

        eprintln!(
            "  {icon}  {name:<w$}  {bar}  {count}",
            name = output::accent(&s.name),
            bar = bar,
            count = if s.valid == s.total {
                output::ok(&count_str)
            } else {
                output::warn(&count_str)
            },
            w = name_w + ansi_len_overhead(&output::accent(&s.name), s.name.len()),
        );

        // Signer line (indented under the bar)
        if !s.signers.is_empty() {
            eprintln!(
                "     {:<w$}  {}",
                "",
                output::dim(&format!("└─ {}", s.signers.join(", "))),
                w = name_w,
            );
        }

        // Invalid/unsigned detail
        if s.invalid > 0 {
            eprintln!(
                "     {:<w$}  {}",
                "",
                output::err_text(&format!("└─ {} hash mismatch — driver files may be tampered", s.invalid)),
                w = name_w,
            );
        }
        if s.unsigned > 0 {
            eprintln!(
                "     {:<w$}  {}",
                "",
                output::warn(&format!("└─ {} unsigned — no vendor certificate", s.unsigned)),
                w = name_w,
            );
        }
    }

    // ── Grand totals ────────────────────────────────────────────────
    let total: usize = stats.iter().map(|s| s.total).sum();
    let valid: usize = stats.iter().map(|s| s.valid).sum();
    let unsigned: usize = stats.iter().map(|s| s.unsigned).sum();
    let invalid: usize = stats.iter().map(|s| s.invalid).sum();
    let mfr_count = stats.len();

    // Unique signers across all manufacturers
    let mut all_signers: Vec<String> = stats
        .iter()
        .flat_map(|s| s.signers.iter().cloned())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    all_signers.sort();

    eprintln!();
    eprintln!("{}", output::header(&format!("━━ Summary {}", "━".repeat(DASH_WIDTH - 12))));
    eprintln!();

    // Big verdict line
    if invalid == 0 && unsigned == 0 {
        eprintln!(
            "  {}  All {} drivers across {} vendors verified",
            output::ok("✓ PASS"),
            output::accent(&total.to_string()),
            output::accent(&mfr_count.to_string()),
        );
    } else if invalid > 0 {
        eprintln!(
            "  {}  {} of {} drivers have hash mismatches",
            output::err_text("✗ FAIL"),
            output::err_text(&invalid.to_string()),
            total,
        );
    } else {
        eprintln!(
            "  {}  {} valid, {} unsigned (no vendor cert)",
            output::warn("○ PARTIAL"),
            output::ok(&valid.to_string()),
            output::warn(&unsigned.to_string()),
        );
    }

    eprintln!();

    // Stats block
    eprintln!("  {}   {}", output::label("Drivers:"), format!("{total} total, {valid} signed, {unsigned} unsigned, {invalid} tampered"));
    eprintln!("  {}   {}", output::label("Vendors:"), output::dim(&mfr_count.to_string()));
    if !all_signers.is_empty() {
        eprintln!("  {}   {}", output::label("Signers:"), output::dim(&all_signers.join(", ")));
    }

    eprintln!();
    eprintln!("{}", output::header(&"━".repeat(DASH_WIDTH)));

    // Actionable guidance
    if invalid > 0 {
        eprintln!();
        eprintln!(
            "  {} Re-download with: {}",
            output::err_text("!"),
            output::accent("prinstall sdi clean && prinstall sdi prefetch"),
        );
        eprintln!();
    } else if total > 0 && invalid == 0 {
        eprintln!();
        eprintln!(
            "  {} Every driver is signed by its vendor and chains to Microsoft's root CA.",
            output::dim("ℹ"),
        );
        eprintln!();
    }
}

/// Calculate ANSI escape overhead for column alignment.
fn ansi_len_overhead(styled: &str, visible_len: usize) -> usize {
    styled.len().saturating_sub(visible_len)
}

fn build_report(pack_name: &str, stats: &[ManufacturerStats]) -> VerifyReport {
    VerifyReport {
        pack_name: pack_name.to_string(),
        total_cats: stats.iter().map(|s| s.total).sum(),
        total_valid: stats.iter().map(|s| s.valid).sum(),
        total_unsigned: stats.iter().map(|s| s.unsigned).sum(),
        total_invalid: stats.iter().map(|s| s.invalid).sum(),
        manufacturers: stats
            .iter()
            .map(|s| ManufacturerReport {
                name: s.name.clone(),
                total: s.total,
                valid: s.valid,
                unsigned: s.unsigned,
                invalid: s.invalid,
                signers: s.signers.clone(),
            })
            .collect(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sig_status_from_ps_numeric() {
        assert_eq!(SigStatus::from_ps("0"), SigStatus::Valid);
        assert_eq!(SigStatus::from_ps("2"), SigStatus::NotSigned);
        assert_eq!(SigStatus::from_ps("3"), SigStatus::HashMismatch);
        assert_eq!(SigStatus::from_ps("4"), SigStatus::NotTrusted);
    }

    #[test]
    fn sig_status_from_ps_string() {
        assert_eq!(SigStatus::from_ps("Valid"), SigStatus::Valid);
        assert_eq!(SigStatus::from_ps("NotSigned"), SigStatus::NotSigned);
    }

    #[test]
    fn extract_cn_parses_subject() {
        assert_eq!(
            extract_cn("CN=Brother Industries, Ltd., O=Brother Industries, Ltd., C=JP"),
            Some("Brother Industries, Ltd.".to_string())
        );
    }

    #[test]
    fn extract_cn_returns_none_for_empty() {
        assert_eq!(extract_cn(""), None);
        assert_eq!(extract_cn("O=Foo, C=US"), None);
    }

    #[test]
    fn build_ps_command_wraps_array() {
        let paths = vec![PathBuf::from(r"C:\test\driver.cat")];
        let cmd = build_ps_command(&paths);
        assert!(cmd.contains("@(Get-AuthenticodeSignature"));
        assert!(cmd.contains("-LiteralPath"));
        assert!(cmd.contains("driver.cat"));
    }

    #[test]
    fn build_ps_command_escapes_quotes() {
        let paths = vec![PathBuf::from(r"C:\it's\a test.cat")];
        let cmd = build_ps_command(&paths);
        assert!(cmd.contains("it''s"));
    }

    #[test]
    fn parse_raw_results_handles_valid() {
        let json = r#"[{
            "Path": "C:\\test\\driver.cat",
            "Status": 0,
            "StatusMessage": "Signature verified.",
            "SignerCertificate": {
                "Subject": "CN=Brother Industries, Ltd., O=Brother, C=JP",
                "Issuer": "CN=Microsoft Code Signing PCA 2011, O=Microsoft, C=US"
            }
        }]"#;
        let results = parse_raw_results(json, &[PathBuf::from(r"C:\test\driver.cat")]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, SigStatus::Valid);
        assert_eq!(results[0].signer.as_deref(), Some("Brother Industries, Ltd."));
    }

    #[test]
    fn parse_raw_results_handles_unsigned() {
        let json = r#"[{
            "Path": "C:\\test\\unsigned.cat",
            "Status": 2,
            "StatusMessage": "The file is not signed.",
            "SignerCertificate": null
        }]"#;
        let results = parse_raw_results(json, &[PathBuf::from(r"C:\test\unsigned.cat")]);
        assert_eq!(results[0].status, SigStatus::NotSigned);
        assert!(results[0].signer.is_none());
    }

    #[test]
    fn parse_raw_results_handles_bad_json() {
        let results = parse_raw_results("not json", &[PathBuf::from("a.cat"), PathBuf::from("b.cat")]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, SigStatus::UnknownError);
    }

    #[test]
    fn aggregate_groups_by_first_dir() {
        let pack_dir = PathBuf::from("/sdi/extracted/DP_Printer_26000");
        let results = vec![
            CatResult {
                path: PathBuf::from("/sdi/extracted/DP_Printer_26000/Brother/hl/driver.cat"),
                status: SigStatus::Valid,
                signer: Some("Brother".into()),
                issuer_ca: None,
            },
            CatResult {
                path: PathBuf::from("/sdi/extracted/DP_Printer_26000/Brother/mfc/driver.cat"),
                status: SigStatus::Valid,
                signer: Some("Brother".into()),
                issuer_ca: None,
            },
            CatResult {
                path: PathBuf::from("/sdi/extracted/DP_Printer_26000/Canon/ir/driver.cat"),
                status: SigStatus::NotSigned,
                signer: None,
                issuer_ca: None,
            },
        ];
        let stats = aggregate_by_manufacturer(&pack_dir, &results);
        assert_eq!(stats.len(), 2);
        let brother = stats.iter().find(|s| s.name == "Brother").unwrap();
        assert_eq!(brother.total, 2);
        assert_eq!(brother.valid, 2);
        let canon = stats.iter().find(|s| s.name == "Canon").unwrap();
        assert_eq!(canon.total, 1);
        assert_eq!(canon.unsigned, 1);
    }
}

#[cfg(test)]
mod pack_verify_tests {
    use super::*;

    #[test]
    fn outcome_valid_when_all_cats_valid() {
        let results = vec![
            CatResult {
                path: PathBuf::from("a.cat"),
                status: SigStatus::Valid,
                signer: Some("CN=Vendor".into()),
                issuer_ca: Some("CN=MS Root".into()),
            },
            CatResult {
                path: PathBuf::from("b.cat"),
                status: SigStatus::Valid,
                signer: Some("CN=Vendor".into()),
                issuer_ca: Some("CN=MS Root".into()),
            },
        ];
        let outcome = PackVerifyOutcome::from_cat_results(&results);
        match outcome {
            PackVerifyOutcome::Verified { valid, signers } => {
                assert_eq!(valid, 2);
                assert_eq!(signers, vec!["CN=Vendor".to_string()]);
            }
            other => panic!("expected Verified, got {:?}", other),
        }
    }

    #[test]
    fn outcome_unsigned_when_any_cat_unsigned() {
        let results = vec![
            CatResult {
                path: PathBuf::from("a.cat"),
                status: SigStatus::Valid,
                signer: Some("CN=Vendor".into()),
                issuer_ca: None,
            },
            CatResult {
                path: PathBuf::from("b.cat"),
                status: SigStatus::NotSigned,
                signer: None,
                issuer_ca: None,
            },
        ];
        let outcome = PackVerifyOutcome::from_cat_results(&results);
        match outcome {
            PackVerifyOutcome::Unsigned { unsigned, total } => {
                assert_eq!(unsigned, 1);
                assert_eq!(total, 2);
            }
            other => panic!("expected Unsigned, got {:?}", other),
        }
    }

    #[test]
    fn outcome_invalid_when_hash_mismatch() {
        let results = vec![CatResult {
            path: PathBuf::from("a.cat"),
            status: SigStatus::HashMismatch,
            signer: None,
            issuer_ca: None,
        }];
        let outcome = PackVerifyOutcome::from_cat_results(&results);
        match outcome {
            PackVerifyOutcome::Invalid {
                invalid,
                total,
                first_reason,
            } => {
                assert_eq!(invalid, 1);
                assert_eq!(total, 1);
                assert!(first_reason.contains("HashMismatch"));
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn outcome_no_catalogs_for_empty_input() {
        let results: Vec<CatResult> = vec![];
        let outcome = PackVerifyOutcome::from_cat_results(&results);
        assert!(matches!(outcome, PackVerifyOutcome::NoCatalogs));
    }

    #[test]
    fn is_safe_to_install_only_true_for_verified() {
        let v = PackVerifyOutcome::Verified {
            valid: 1,
            signers: vec![],
        };
        let u = PackVerifyOutcome::Unsigned {
            unsigned: 1,
            total: 1,
        };
        let i = PackVerifyOutcome::Invalid {
            invalid: 1,
            total: 1,
            first_reason: "x".into(),
        };
        let n = PackVerifyOutcome::NoCatalogs;
        assert!(v.is_safe_to_install());
        assert!(!u.is_safe_to_install());
        assert!(!i.is_safe_to_install());
        assert!(!n.is_safe_to_install());
    }
}

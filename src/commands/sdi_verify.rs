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

// ── Entry point ─────────────────────────────────────────────────────────────

/// Run the `prinstall sdi verify` command.
pub fn run(executor: &dyn PsExecutor, json: bool, verbose: bool) {
    let packs = match walk_extraction_cache() {
        Ok(p) if p.is_empty() => {
            if json {
                println!("[]");
            } else {
                println!(
                    "\n{}",
                    output::dim("No extracted drivers found. Run 'prinstall add <ip>' or 'prinstall sdi prefetch' first.")
                );
            }
            return;
        }
        Ok(p) => p,
        Err(e) => {
            if json {
                println!("[]");
            } else {
                println!("\n{}", output::err_text(&e));
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
            println!(
                "\nVerifying {} .cat files in {}/...\n",
                output::accent(&cats.len().to_string()),
                output::dim(&format!("sdi/extracted/{pack_name}"))
            );
        }

        let results = verify_cats(executor, cats, verbose);
        let stats = aggregate_by_manufacturer(pack_dir, &results);

        if json {
            let report = build_report(pack_name, &stats);
            all_reports.push(report);
        } else {
            print_table(pack_name, &stats);
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

fn print_table(_pack_name: &str, stats: &[ManufacturerStats]) {
    let name_w = stats.iter().map(|s| s.name.len()).max().unwrap_or(10).max(10);

    for s in stats {
        let valid_str = if s.valid == s.total {
            output::ok(&format!("{} valid", s.valid))
        } else {
            format!("{} valid", s.valid)
        };
        let invalid_str = if s.invalid > 0 {
            output::err_text(&format!("{} invalid", s.invalid))
        } else {
            output::dim(&format!("{} invalid", s.invalid))
        };
        let unsigned_str = if s.unsigned > 0 {
            output::warn(&format!("{} unsigned", s.unsigned))
        } else {
            output::dim(&format!("{} unsigned", s.unsigned))
        };
        let signers_str = if s.signers.is_empty() {
            output::dim("(no signer)")
        } else {
            output::dim(&format!("Signers: {}", s.signers.join(", ")))
        };

        println!(
            "  {:<w$}  {:>3} drivers   {}   {}   {}   {}",
            s.name,
            s.total,
            valid_str,
            invalid_str,
            unsigned_str,
            signers_str,
            w = name_w,
        );
    }

    // Totals
    let total: usize = stats.iter().map(|s| s.total).sum();
    let valid: usize = stats.iter().map(|s| s.valid).sum();
    let unsigned: usize = stats.iter().map(|s| s.unsigned).sum();
    let invalid: usize = stats.iter().map(|s| s.invalid).sum();

    println!("\n{}", output::header(&"━".repeat(46)));
    println!(
        "  {} total   {} valid   {} invalid   {} unsigned",
        output::accent(&total.to_string()),
        output::ok(&valid.to_string()),
        if invalid > 0 { output::err_text(&invalid.to_string()) } else { output::dim(&invalid.to_string()) },
        if unsigned > 0 { output::warn(&unsigned.to_string()) } else { output::dim(&unsigned.to_string()) },
    );
    println!("{}", output::header(&"━".repeat(46)));

    if invalid > 0 {
        println!(
            "\n  {} Hash mismatches detected. Run {} to re-download.",
            output::err_text("!"),
            output::accent("prinstall sdi clean && prinstall sdi prefetch"),
        );
    }
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

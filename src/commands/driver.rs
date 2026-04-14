//! `prinstall driver add <path>` — stage a driver into the Windows driver
//! store without creating a printer queue.
//!
//! Wraps `pnputil /add-driver <path>\*.inf /install /subdirs` (for folders)
//! or the single-INF equivalent (for explicit .inf paths). When verification
//! is enabled (the default), runs `Get-AuthenticodeSignature` on any `.cat`
//! files in the path first — the same gate `prinstall add` uses. Pass
//! `--no-verify` to stage without the signature check; the success line
//! carries an `[UNVERIFIED]` marker so the audit trail is explicit about it.
//!
//! The verification path is gated behind `--features sdi` because it reuses
//! the `commands::sdi_verify` helpers. In the lean (no-SDI) build, the
//! `--no-verify` flag is implicit and the command stages the driver
//! immediately.
//!
//! This command is deliberately minimal — no printer queue, no port, no
//! `Set-Printer` wiring. Pre-staging drivers like this is what RMM runbooks
//! want before any user plugs in a USB device or a first `Add-Printer` call
//! fires, and it's what SDI-origin packs get for free via the install path.

use std::path::Path;

use crate::core::executor::RealExecutor;

/// Arguments for `prinstall driver add`.
pub struct DriverAddArgs<'a> {
    pub path: &'a str,
    pub no_verify: bool,
    pub verbose: bool,
    pub json: bool,
}

/// Entry point for `prinstall driver add <path>`.
///
/// Returns the exit code (0 on success, 1 on failure).
pub async fn add(args: DriverAddArgs<'_>) -> i32 {
    let path = Path::new(args.path);
    if !path.exists() {
        emit_error(args.json, &format!("path not found: {}", args.path));
        return 1;
    }

    let executor = RealExecutor::new(args.verbose);

    // ── Verification gate (SDI feature only) ────────────────────────────────
    // The verification helpers live in `commands::sdi_verify`, which is
    // compiled only under `--features sdi`. In lean builds we skip straight
    // to staging, matching the `--no-verify` path.
    #[cfg(feature = "sdi")]
    if !args.no_verify {
        let verify_dir = if path.is_file() {
            path.parent().unwrap_or(path).to_path_buf()
        } else {
            path.to_path_buf()
        };

        let outcome = crate::commands::sdi_verify::verify_pack_directory(
            &executor,
            &verify_dir,
            args.verbose,
        );

        use crate::commands::sdi_verify::PackVerifyOutcome;
        match &outcome {
            PackVerifyOutcome::Verified { valid, signers } => {
                if !args.json {
                    let signer = signers
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or("unknown");
                    println!(
                        "✓ Verified: {valid} .cat file(s), signed by {signer}"
                    );
                }
            }
            PackVerifyOutcome::Unsigned { unsigned, total } => {
                let msg = format!(
                    "verification failed: {unsigned}/{total} cats unsigned — use --no-verify to override"
                );
                emit_error(args.json, &msg);
                if !args.json {
                    eprintln!("Use --no-verify to stage this driver anyway.");
                }
                return 1;
            }
            PackVerifyOutcome::Invalid { first_reason, .. } => {
                let msg = format!("verification failed: {first_reason}");
                emit_error(args.json, &msg);
                if !args.json {
                    eprintln!("Use --no-verify to stage this driver anyway.");
                }
                return 1;
            }
            PackVerifyOutcome::NoCatalogs => {
                let msg = format!(
                    "no .cat catalog files in {} — use --no-verify to override",
                    args.path
                );
                emit_error(args.json, &msg);
                if !args.json {
                    eprintln!(
                        "Use --no-verify if this vendor ships drivers without signed catalogs."
                    );
                }
                return 1;
            }
        }
    }

    // ── Stage via pnputil ───────────────────────────────────────────────────
    let (success, stdout, error) = if path.is_file() {
        // Single INF path — sync wrapper, no subdirs.
        let res =
            crate::installer::powershell::stage_driver_inf(args.path, args.verbose);
        let err = if res.success { String::new() } else { res.error_summary() };
        (res.success, res.stdout, err)
    } else {
        // Directory — async wrapper with /install /subdirs.
        let pnp = crate::installer::powershell::pnputil_add_driver(
            &executor,
            args.path,
            args.verbose,
        )
        .await;
        (pnp.success, pnp.stdout, pnp.error.unwrap_or_default())
    };

    if args.json {
        emit_result_json(args.path, success, &stdout, &error);
    } else if success {
        let unverified = if args.no_verify { " [UNVERIFIED]" } else { "" };
        println!("✓ Driver staged successfully{unverified}");
        if args.verbose && !stdout.is_empty() {
            println!();
            println!("{stdout}");
        }
    } else {
        eprintln!("Error: pnputil failed to stage driver");
        if !error.is_empty() {
            eprintln!("{error}");
        }
    }

    if success {
        0
    } else {
        1
    }
}

/// Emit a single-line JSON error object, or a plain-text stderr message.
fn emit_error(json: bool, message: &str) {
    if json {
        println!(
            r#"{{"success":false,"error":"{}"}}"#,
            escape_json(message)
        );
    } else {
        eprintln!("Error: {message}");
    }
}

/// Emit the final pnputil result as JSON.
fn emit_result_json(path: &str, success: bool, stdout: &str, error: &str) {
    println!(
        r#"{{"success":{success},"path":"{}","stdout":"{}","error":"{}"}}"#,
        escape_json(path),
        escape_json(stdout),
        escape_json(error),
    );
}

/// Minimal JSON string escape — backslash, quote, control chars.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_add_args_shape() {
        let args = DriverAddArgs {
            path: "C:\\test",
            no_verify: false,
            verbose: false,
            json: false,
        };
        assert_eq!(args.path, "C:\\test");
        assert!(!args.no_verify);
    }

    #[tokio::test]
    async fn nonexistent_path_returns_1() {
        let args = DriverAddArgs {
            path: "/nonexistent/path/that/does/not/exist",
            no_verify: true, // skip verify to avoid PS calls on Linux
            verbose: false,
            json: true,
        };
        let exit = add(args).await;
        assert_eq!(exit, 1);
    }

    #[test]
    fn escape_json_handles_windows_paths() {
        let s = escape_json(r#"C:\Drivers\HP"LaserJet""#);
        assert_eq!(s, r#"C:\\Drivers\\HP\"LaserJet\""#);
    }

    #[test]
    fn escape_json_handles_newlines() {
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
    }
}

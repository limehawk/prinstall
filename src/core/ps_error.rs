//! Parse PowerShell stderr into a clean, user-friendly error message.
//!
//! PowerShell cmdlets emit a very verbose error format to stderr:
//!
//! ```text
//! Add-Printer : An error occurred while performing the specified operation.  See the error details for more information.
//! At line:1 char:1
//! + Add-Printer -ConnectionName 'http://10.10.20.16:631/ipp/print' -Error ...
//! + ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
//!     + CategoryInfo          : InvalidOperation: (MSFT_Printer:ROOT/StandardCimv2/MSFT_Printer) [Add-Printer], CimException
//!     + FullyQualifiedErrorId : HRESULT 0x80070032,Add-Printer
//! ```
//!
//! Dumping this raw into CLI error output is unreadable. This module extracts
//! the useful parts — cmdlet name, primary message, HRESULT code + description —
//! and formats them as a single line:
//!
//! ```text
//! Add-Printer: An error occurred while performing the specified operation. [HRESULT 0x80070032: The request is not supported]
//! ```

/// Parsed PowerShell error with the noise stripped and semantics extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanedPsError {
    /// Cmdlet name parsed from the error line (e.g. `"Add-Printer"`).
    /// `None` if the stderr didn't follow the `<Cmdlet> : <message>` shape.
    pub cmdlet: Option<String>,
    /// Human-readable message, boilerplate trimmed.
    pub message: String,
    /// HRESULT code if one was present in the error, e.g. `0x80070032`.
    pub hresult: Option<u32>,
    /// Human-readable description for the HRESULT, if we recognize it.
    pub hresult_description: Option<&'static str>,
}

impl CleanedPsError {
    /// Parse a raw PowerShell stderr string into a cleaned error.
    /// Always succeeds — if the input isn't recognizable PS-style output,
    /// the whole input becomes the message with no metadata.
    pub fn parse(stderr: &str) -> Self {
        let mut cmdlet: Option<String> = None;
        let mut message = String::new();

        // Walk lines looking for the primary `<Cmdlet-Name> : <message>` line.
        // Skip PowerShell decorator lines (`At line:`, `+ ...`, CategoryInfo,
        // FullyQualifiedErrorId).
        for line in stderr.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("At line:") || trimmed.starts_with("At char:") {
                continue;
            }
            if trimmed.starts_with("+ ") || trimmed.starts_with("+~") {
                continue;
            }
            if trimmed.starts_with("+ CategoryInfo")
                || trimmed.starts_with("CategoryInfo")
                || trimmed.starts_with("+ FullyQualifiedErrorId")
                || trimmed.starts_with("FullyQualifiedErrorId")
            {
                continue;
            }

            // Look for `Cmdlet-Name : message` pattern.
            if let Some((left, right)) = trimmed.split_once(" : ")
                && is_cmdlet_name(left)
            {
                cmdlet = Some(left.to_string());
                message = right.trim().to_string();
                // Trim generic "See the error details" boilerplate.
                if let Some(idx) = message.find("  See the error details") {
                    message.truncate(idx);
                }
                if let Some(idx) = message.find(" See the error details") {
                    message.truncate(idx);
                }
                break;
            }
        }

        // Parse HRESULT from anywhere in the full stderr. Format is either
        // `HRESULT 0xNNNNNNNN` or `HRESULT 0xNNNNNNNN,Cmdlet-Name`.
        let hresult = extract_hresult(stderr);

        // Fallback: if we couldn't identify a clean message line, use the
        // first non-empty, non-decorator line as-is.
        if message.is_empty() {
            for line in stderr.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.starts_with("At line:")
                    || trimmed.starts_with("+ ")
                    || trimmed.starts_with("+~")
                    || trimmed.contains("CategoryInfo")
                    || trimmed.contains("FullyQualifiedErrorId")
                {
                    continue;
                }
                message = trimmed.to_string();
                break;
            }
        }
        // Ultimate fallback: the first non-empty line, even if it's a decorator.
        if message.is_empty() {
            message = stderr
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .to_string();
        }

        Self {
            cmdlet,
            message,
            hresult,
            hresult_description: hresult.and_then(lookup_hresult),
        }
    }

    /// Format as a single-line user-facing error string.
    pub fn display(&self) -> String {
        let mut out = String::new();
        if let Some(ref cmdlet) = self.cmdlet {
            out.push_str(cmdlet);
            out.push_str(": ");
        }
        out.push_str(self.message.trim());
        if let Some(hr) = self.hresult {
            out.push_str(&format!(" [HRESULT 0x{hr:08x}"));
            if let Some(desc) = self.hresult_description {
                out.push_str(": ");
                out.push_str(desc);
            }
            out.push(']');
        }
        out
    }
}

/// Convenience: parse raw PS stderr and return the one-line display string.
pub fn clean(stderr: &str) -> String {
    CleanedPsError::parse(stderr).display()
}

/// Heuristic — does this look like a PowerShell cmdlet name?
/// PowerShell cmdlets follow `Verb-Noun` with initial caps.
fn is_cmdlet_name(s: &str) -> bool {
    if s.is_empty() || s.contains(' ') {
        return false;
    }
    if !s.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return false;
    }
    // Must contain a dash AND no dash at the end
    s.contains('-') && !s.ends_with('-')
}

/// Extract an HRESULT code from arbitrary stderr text.
/// Looks for `HRESULT 0x` followed by up to 8 hex digits.
fn extract_hresult(stderr: &str) -> Option<u32> {
    let prefix = "HRESULT 0x";
    let start = stderr.find(prefix)? + prefix.len();
    let hex: String = stderr[start..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    if hex.is_empty() {
        return None;
    }
    u32::from_str_radix(&hex, 16).ok()
}

/// Look up a human-readable description for HRESULTs we've actually
/// encountered during prinstall development. Intentionally NOT a
/// comprehensive Windows error list — only the ones that meaningfully
/// improve the user experience for printer operations.
fn lookup_hresult(code: u32) -> Option<&'static str> {
    match code {
        0x80070002 => Some("The system cannot find the file specified"),
        0x80070005 => Some("Access denied"),
        0x80070032 => Some("The request is not supported"),
        0x80070057 => Some("The parameter is incorrect"),
        0x80070070 => Some("There is not enough space on the disk"),
        0x800700AA => Some("The requested resource is in use"),
        0x80070705 => Some("Unknown printer driver"),
        0x8007070A => Some("The specified printer already exists"),
        0x80070BB9 => Some("The specified driver is in use by another printer"),
        0x80070BBB => Some("Unknown printer driver"),
        _ => None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_printer_not_supported() {
        let stderr = "Add-Printer : An error occurred while performing the specified operation.  See the error details for more information.\n\
At line:1 char:1\n\
+ Add-Printer -ConnectionName 'http://10.10.20.16:631/ipp/print' -Error ...\n\
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~\n\
    + CategoryInfo          : InvalidOperation: (MSFT_Printer:ROOT/StandardCimv2/MSFT_Printer) [Add-Printer], CimException\n\
    + FullyQualifiedErrorId : HRESULT 0x80070032,Add-Printer";

        let cleaned = CleanedPsError::parse(stderr);
        assert_eq!(cleaned.cmdlet.as_deref(), Some("Add-Printer"));
        assert_eq!(
            cleaned.message,
            "An error occurred while performing the specified operation."
        );
        assert_eq!(cleaned.hresult, Some(0x80070032));
        assert_eq!(
            cleaned.hresult_description,
            Some("The request is not supported")
        );

        let display = cleaned.display();
        assert_eq!(
            display,
            "Add-Printer: An error occurred while performing the specified operation. \
             [HRESULT 0x80070032: The request is not supported]"
        );
    }

    #[test]
    fn parses_add_printer_driver_not_in_store() {
        let stderr = "Add-PrinterDriver : The specified driver does not exist in the driver store.\n\
At line:1 char:1\n\
+ Add-PrinterDriver -Name 'Brother Universal Printer'\n\
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~\n\
    + CategoryInfo          : NotSpecified: (MSFT_PrinterDriver:ROOT/StandardCimv2/MSFT_PrinterDriver) [Add-PrinterDriver], CimException\n\
    + FullyQualifiedErrorId : HRESULT 0x80070705,Add-PrinterDriver";

        let cleaned = CleanedPsError::parse(stderr);
        assert_eq!(cleaned.cmdlet.as_deref(), Some("Add-PrinterDriver"));
        assert_eq!(
            cleaned.message,
            "The specified driver does not exist in the driver store."
        );
        assert_eq!(cleaned.hresult, Some(0x80070705));
    }

    #[test]
    fn parses_remove_printer_driver_in_use() {
        let stderr = "Remove-PrinterDriver : The specified driver is in use by one or more printers.\n\
+ FullyQualifiedErrorId : HRESULT 0x80070bb9,Remove-PrinterDriver";

        let cleaned = CleanedPsError::parse(stderr);
        assert_eq!(cleaned.cmdlet.as_deref(), Some("Remove-PrinterDriver"));
        assert!(cleaned.message.contains("in use"));
        assert_eq!(cleaned.hresult, Some(0x80070BB9));
        assert_eq!(
            cleaned.hresult_description,
            Some("The specified driver is in use by another printer")
        );
    }

    #[test]
    fn parses_add_printer_already_exists() {
        let stderr = "Add-Printer : The specified printer already exists.\n\
+ FullyQualifiedErrorId : HRESULT 0x8007070a,Add-Printer";

        let cleaned = CleanedPsError::parse(stderr);
        assert_eq!(cleaned.cmdlet.as_deref(), Some("Add-Printer"));
        assert_eq!(cleaned.message, "The specified printer already exists.");
        assert_eq!(cleaned.hresult, Some(0x8007070A));
    }

    #[test]
    fn parses_remove_printer_port_in_use() {
        let stderr = "Remove-PrinterPort : The specified port is in use by one or more printers.\n\
+ FullyQualifiedErrorId : HRESULT 0x800700aa,Remove-PrinterPort";

        let cleaned = CleanedPsError::parse(stderr);
        assert_eq!(cleaned.cmdlet.as_deref(), Some("Remove-PrinterPort"));
        assert_eq!(cleaned.hresult, Some(0x800700AA));
        assert_eq!(
            cleaned.hresult_description,
            Some("The requested resource is in use")
        );
    }

    #[test]
    fn handles_plain_non_ps_error() {
        // A mock executor might return plain strings like "Access denied"
        // without any PS decoration.
        let cleaned = CleanedPsError::parse("Access denied");
        assert_eq!(cleaned.cmdlet, None);
        assert_eq!(cleaned.message, "Access denied");
        assert_eq!(cleaned.hresult, None);
        assert_eq!(cleaned.display(), "Access denied");
    }

    #[test]
    fn handles_empty_input() {
        let cleaned = CleanedPsError::parse("");
        assert_eq!(cleaned.cmdlet, None);
        assert_eq!(cleaned.message, "");
        assert_eq!(cleaned.display(), "");
    }

    #[test]
    fn handles_only_decorator_lines() {
        // Pathological case — stderr is nothing but decoration.
        let stderr = "At line:1 char:1\n+ some command\n+ ~~~~~~~";
        let cleaned = CleanedPsError::parse(stderr);
        // Should fall back to the first non-empty line
        assert!(!cleaned.message.is_empty());
    }

    #[test]
    fn is_cmdlet_name_matches_pattern() {
        assert!(is_cmdlet_name("Add-Printer"));
        assert!(is_cmdlet_name("Get-PrinterDriver"));
        assert!(is_cmdlet_name("Remove-PrinterPort"));
        assert!(is_cmdlet_name("ConvertTo-Json"));

        assert!(!is_cmdlet_name(""));
        assert!(!is_cmdlet_name("add-printer")); // lowercase
        assert!(!is_cmdlet_name("AddPrinter")); // no dash
        assert!(!is_cmdlet_name("Add Printer")); // space
        assert!(!is_cmdlet_name("Add-")); // trailing dash
    }

    #[test]
    fn extract_hresult_finds_code() {
        assert_eq!(
            extract_hresult("... HRESULT 0x80070032,Add-Printer"),
            Some(0x80070032)
        );
        assert_eq!(extract_hresult("HRESULT 0x00000000"), Some(0));
        assert_eq!(extract_hresult("No hresult here"), None);
        assert_eq!(extract_hresult("HRESULT 0x"), None); // no hex digits
    }

    #[test]
    fn clean_is_display_convenience() {
        let stderr = "Add-Printer : Boom.\n+ FullyQualifiedErrorId : HRESULT 0x80070005,Add-Printer";
        assert_eq!(clean(stderr), "Add-Printer: Boom. [HRESULT 0x80070005: Access denied]");
    }
}

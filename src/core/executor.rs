//! PowerShell execution abstraction.
//!
//! `PsExecutor` is the trait every printer operation depends on. In production
//! it shells out to `powershell.exe` via `RealExecutor`. In tests — including
//! tests running on Linux dev machines — it returns stubbed responses via
//! `MockExecutor`. This is the foundation that makes command logic unit-testable
//! without a Windows host.

use serde::de::DeserializeOwned;

use crate::installer::powershell::{self, PsResult};

/// Something that can execute a PowerShell command and return its result.
///
/// Kept deliberately minimal — one method, `run`, so the trait stays
/// dyn-compatible. Use the free function `run_json` when you need typed
/// deserialization of `ConvertTo-Json` output.
pub trait PsExecutor: Send + Sync {
    /// Execute a raw PowerShell command.
    fn run(&self, command: &str) -> PsResult;
}

/// Execute a PowerShell command that ends with `| ConvertTo-Json`,
/// then deserialize stdout into `T`.
///
/// Free function (not a trait method) so `PsExecutor` can be used as
/// `&dyn PsExecutor`. Empty stdout is treated as JSON `null` so
/// `Option<T>` and empty-collection types deserialize cleanly.
pub fn run_json<T: DeserializeOwned>(
    executor: &dyn PsExecutor,
    command: &str,
) -> Result<T, String> {
    let result = executor.run(command);
    if !result.success {
        return Err(if result.stderr.is_empty() {
            "PowerShell command failed".to_string()
        } else {
            result.stderr
        });
    }
    let stdout = result.stdout.trim();
    let json = if stdout.is_empty() { "null" } else { stdout };
    serde_json::from_str(json)
        .map_err(|e| format!("JSON parse failed: {e}\nOutput was: {stdout}"))
}

// ── Real executor ────────────────────────────────────────────────────────────

/// Production executor — shells out to `powershell.exe` through the existing
/// `powershell::run_ps` helper.
pub struct RealExecutor {
    pub verbose: bool,
}

impl RealExecutor {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}

impl Default for RealExecutor {
    fn default() -> Self {
        Self { verbose: false }
    }
}

impl PsExecutor for RealExecutor {
    fn run(&self, command: &str) -> PsResult {
        powershell::run_ps(command, self.verbose)
    }
}

// ── Mock executor ────────────────────────────────────────────────────────────

/// Test-only executor that returns stubbed responses without spawning PowerShell.
/// Usable on any platform. Commands are matched against registered stubs in
/// registration order — first match wins. If nothing matches, the `default`
/// response is returned.
pub struct MockExecutor {
    stubs: Vec<(MatchKind, PsResult)>,
    default: PsResult,
}

enum MatchKind {
    Exact(String),
    Prefix(String),
    Contains(String),
}

impl MockExecutor {
    pub fn new() -> Self {
        Self {
            stubs: Vec::new(),
            default: PsResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            },
        }
    }

    /// Stub a response for commands matching `cmd` exactly.
    pub fn stub_exact(mut self, cmd: impl Into<String>, result: PsResult) -> Self {
        self.stubs.push((MatchKind::Exact(cmd.into()), result));
        self
    }

    /// Stub a response for any command starting with `prefix`.
    pub fn stub_prefix(mut self, prefix: impl Into<String>, result: PsResult) -> Self {
        self.stubs.push((MatchKind::Prefix(prefix.into()), result));
        self
    }

    /// Stub a response for any command containing `substr`.
    pub fn stub_contains(mut self, substr: impl Into<String>, result: PsResult) -> Self {
        self.stubs.push((MatchKind::Contains(substr.into()), result));
        self
    }

    /// Stub a serialized JSON response for any command containing `substr`.
    /// Convenience wrapper around `stub_contains` for typed payloads.
    pub fn stub_json<T: serde::Serialize>(
        mut self,
        substr: impl Into<String>,
        payload: &T,
    ) -> Self {
        let json = serde_json::to_string(payload).expect("mock stub payload must serialize");
        self.stubs.push((
            MatchKind::Contains(substr.into()),
            PsResult {
                success: true,
                stdout: json,
                stderr: String::new(),
            },
        ));
        self
    }

    /// Stub a failure (non-zero exit) for any command containing `substr`.
    pub fn stub_failure(
        mut self,
        substr: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Self {
        self.stubs.push((
            MatchKind::Contains(substr.into()),
            PsResult {
                success: false,
                stdout: String::new(),
                stderr: stderr.into(),
            },
        ));
        self
    }

    /// Override the default (no-match) response.
    pub fn with_default(mut self, result: PsResult) -> Self {
        self.default = result;
        self
    }
}

impl Default for MockExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl PsExecutor for MockExecutor {
    fn run(&self, command: &str) -> PsResult {
        for (kind, result) in &self.stubs {
            let matches = match kind {
                MatchKind::Exact(s) => command == s,
                MatchKind::Prefix(s) => command.starts_with(s.as_str()),
                MatchKind::Contains(s) => command.contains(s.as_str()),
            };
            if matches {
                return result.clone();
            }
        }
        self.default.clone()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Payload {
        name: String,
        count: u32,
    }

    #[test]
    fn mock_returns_stub_for_exact_match() {
        let mock = MockExecutor::new().stub_exact(
            "Get-Printer",
            PsResult {
                success: true,
                stdout: "HP LaserJet".to_string(),
                stderr: String::new(),
            },
        );
        let r = mock.run("Get-Printer");
        assert!(r.success);
        assert_eq!(r.stdout, "HP LaserJet");
    }

    #[test]
    fn mock_prefix_matches_partial_command() {
        let mock = MockExecutor::new().stub_prefix(
            "Add-Printer",
            PsResult {
                success: true,
                stdout: "added".to_string(),
                stderr: String::new(),
            },
        );
        assert!(mock.run("Add-Printer -Name 'X'").success);
        assert!(mock.run("Add-Printer -Name 'Y' -DriverName 'Z'").success);
    }

    #[test]
    fn mock_contains_matches_anywhere() {
        let mock = MockExecutor::new().stub_contains(
            "Remove-Printer",
            PsResult {
                success: true,
                stdout: "removed".to_string(),
                stderr: String::new(),
            },
        );
        assert!(mock.run("$p = 'foo'; Remove-Printer -Name $p").success);
    }

    #[test]
    fn mock_default_response_when_no_match() {
        let mock = MockExecutor::new();
        let r = mock.run("some unknown command");
        assert!(r.success);
        assert!(r.stdout.is_empty());
    }

    #[test]
    fn mock_stub_failure_returns_error() {
        let mock = MockExecutor::new().stub_failure("Bad-Command", "command not recognized");
        let r = mock.run("Bad-Command -Arg foo");
        assert!(!r.success);
        assert_eq!(r.stderr, "command not recognized");
    }

    #[test]
    fn run_json_deserializes_typed_payload() {
        let expected = Payload {
            name: "foo".to_string(),
            count: 42,
        };
        let mock = MockExecutor::new().stub_json("Get-Test", &expected);
        let result: Payload = run_json(&mock, "Get-Test -Detailed").unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn run_json_propagates_command_failure() {
        let mock = MockExecutor::new().stub_failure("Get-Test", "access denied");
        let result: Result<Payload, String> = run_json(&mock, "Get-Test");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("access denied"));
    }

    #[test]
    fn run_json_parses_empty_stdout_as_null() {
        let mock = MockExecutor::new();
        // Default is success+empty stdout → deserializes to Option::None
        let result: Result<Option<Payload>, String> = run_json(&mock, "Get-Nothing");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn run_json_returns_error_on_invalid_json() {
        let mock = MockExecutor::new().stub_contains(
            "Get-Bad",
            PsResult {
                success: true,
                stdout: "not valid json".to_string(),
                stderr: String::new(),
            },
        );
        let result: Result<Payload, String> = run_json(&mock, "Get-Bad");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("JSON parse failed"));
    }

    #[test]
    fn first_matching_stub_wins() {
        let mock = MockExecutor::new()
            .stub_contains(
                "Get-Printer",
                PsResult {
                    success: true,
                    stdout: "first".to_string(),
                    stderr: String::new(),
                },
            )
            .stub_contains(
                "Get-Printer",
                PsResult {
                    success: true,
                    stdout: "second".to_string(),
                    stderr: String::new(),
                },
            );
        assert_eq!(mock.run("Get-Printer -Name X").stdout, "first");
    }
}

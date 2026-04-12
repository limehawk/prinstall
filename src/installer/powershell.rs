use std::process::Command;

use crate::core::ps_error;

/// Result of a PowerShell command execution.
#[derive(Debug, Clone)]
pub struct PsResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl PsResult {
    /// Extract a human-readable error message from this result.
    ///
    /// Tries stderr first (via `ps_error::clean()`), then falls back to the
    /// first non-empty line of stdout (some tools like `pnputil` write errors
    /// there), then a generic fallback so callers never get an empty string.
    pub fn error_summary(&self) -> String {
        let cleaned = ps_error::clean(&self.stderr);
        if !cleaned.is_empty() {
            return cleaned;
        }
        // pnputil and other non-PS tools may report errors on stdout.
        let first_stdout_line = self
            .stdout
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        if !first_stdout_line.is_empty() {
            return first_stdout_line.to_string();
        }
        "unknown error (no output captured)".to_string()
    }
}

/// Escape a string for safe use inside PowerShell single quotes.
/// Single quotes in PS are escaped by doubling them: ' → ''
pub fn escape_ps_string(s: &str) -> String {
    s.replace('\'', "''")
}

/// Run a PowerShell command and return the result.
pub fn run_ps(command: &str, verbose: bool) -> PsResult {
    if verbose {
        eprintln!("{} {command}", crate::output::vpfx("PS"));
    }

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if verbose {
                if !stdout.is_empty() {
                    eprintln!("{} {stdout}", crate::output::vpfx("PS stdout"));
                }
                if !stderr.is_empty() {
                    // Route through ps_error::clean() so the verbose log
                    // shows the same single-line human-readable message
                    // the final PrinterOpResult gets, instead of dumping
                    // the raw PowerShell decoration (CategoryInfo, At line,
                    // FullyQualifiedErrorId, etc).
                    eprintln!("{} {}", crate::output::vpfx("PS stderr"), ps_error::clean(&stderr));
                }
            }
            PsResult {
                success: o.status.success(),
                stdout,
                stderr,
            }
        }
        Err(e) => PsResult {
            success: false,
            stdout: String::new(),
            stderr: format!("Failed to run PowerShell: {e}"),
        },
    }
}

/// Check if a printer port already exists.
pub fn port_exists(port_name: &str, verbose: bool) -> bool {
    let cmd = format!(
        "Get-PrinterPort -Name '{}' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name",
        escape_ps_string(port_name)
    );
    let result = run_ps(&cmd, verbose);
    result.success && !result.stdout.is_empty()
}

/// Create a TCP/IP printer port.
pub fn create_port(ip: &str, verbose: bool) -> PsResult {
    let port_name = format!("IP_{ip}");
    if port_exists(&port_name, verbose) {
        if verbose {
            eprintln!("{} Port {port_name} already exists", crate::output::vpfx("skip"));
        }
        return PsResult {
            success: true,
            stdout: port_name,
            stderr: String::new(),
        };
    }
    let safe_ip = escape_ps_string(ip);
    let cmd = format!(
        "Add-PrinterPort -Name 'IP_{safe_ip}' -PrinterHostAddress '{safe_ip}'; 'IP_{safe_ip}'"
    );
    run_ps(&cmd, verbose)
}

/// Check if a printer driver is already installed.
pub fn driver_installed(driver_name: &str, verbose: bool) -> bool {
    let cmd = format!(
        "Get-PrinterDriver -Name '{}' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name",
        escape_ps_string(driver_name)
    );
    let result = run_ps(&cmd, verbose);
    result.success && !result.stdout.is_empty()
}

/// Install a printer driver by name (must already be staged in driver store).
pub fn install_driver(driver_name: &str, verbose: bool) -> PsResult {
    if driver_installed(driver_name, verbose) {
        if verbose {
            eprintln!("{} Driver '{}' already installed", crate::output::vpfx("skip"), crate::output::accent(driver_name));
        }
        return PsResult {
            success: true,
            stdout: driver_name.to_string(),
            stderr: String::new(),
        };
    }
    let cmd = format!("Add-PrinterDriver -Name '{}'", escape_ps_string(driver_name));
    run_ps(&cmd, verbose)
}

/// Stage a driver INF file via pnputil.
pub fn stage_driver_inf(inf_path: &str, verbose: bool) -> PsResult {
    let cmd = format!(
        "pnputil /add-driver '{}' /install",
        escape_ps_string(inf_path)
    );
    run_ps(&cmd, verbose)
}

/// Find the printer queue name assigned to a given port (e.g. `IP_192.168.1.50`).
/// Returns `Some(queue_name)` if a printer is bound to that port, `None` otherwise.
pub fn find_printer_on_port(port_name: &str, verbose: bool) -> Option<String> {
    let cmd = format!(
        "Get-Printer | Where-Object {{ $_.PortName -eq '{}' }} | Select-Object -ExpandProperty Name -First 1",
        escape_ps_string(port_name)
    );
    let result = run_ps(&cmd, verbose);
    if !result.success {
        return None;
    }
    let name = result.stdout.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Check if a printer queue with the given name already exists.
pub fn printer_exists(name: &str, verbose: bool) -> bool {
    let cmd = format!(
        "Get-Printer -Name '{}' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name",
        escape_ps_string(name)
    );
    let result = run_ps(&cmd, verbose);
    result.success && !result.stdout.is_empty()
}

/// Add a printer queue. Idempotent — returns success if a queue with the
/// same name already exists.
pub fn add_printer(name: &str, driver_name: &str, port_name: &str, verbose: bool) -> PsResult {
    if printer_exists(name, verbose) {
        if verbose {
            eprintln!("{} Printer '{}' already exists", crate::output::vpfx("skip"), crate::output::accent(name));
        }
        return PsResult {
            success: true,
            stdout: name.to_string(),
            stderr: String::new(),
        };
    }
    let cmd = format!(
        "Add-Printer -Name '{}' -DriverName '{}' -PortName '{}'",
        escape_ps_string(name),
        escape_ps_string(driver_name),
        escape_ps_string(port_name),
    );
    run_ps(&cmd, verbose)
}

/// Update an existing printer's driver via Set-Printer.
pub fn set_printer_driver(printer_name: &str, driver_name: &str, verbose: bool) -> PsResult {
    let cmd = format!(
        "Set-Printer -Name '{}' -DriverName '{}'",
        escape_ps_string(printer_name),
        escape_ps_string(driver_name),
    );
    run_ps(&cmd, verbose)
}

/// List drivers from the local driver store via pnputil.
/// Returns a list of driver names for print-class drivers.
pub fn list_local_drivers(verbose: bool) -> Vec<String> {
    let cmd = "Get-PrinterDriver | Select-Object -ExpandProperty Name";
    let result = run_ps(cmd, verbose);
    if !result.success {
        return Vec::new();
    }
    result
        .stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

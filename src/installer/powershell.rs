use std::process::Command;

/// Result of a PowerShell command execution.
pub struct PsResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Escape a string for safe use inside PowerShell single quotes.
/// Single quotes in PS are escaped by doubling them: ' → ''
pub fn escape_ps_string(s: &str) -> String {
    s.replace('\'', "''")
}

/// Run a PowerShell command and return the result.
pub fn run_ps(command: &str, verbose: bool) -> PsResult {
    if verbose {
        eprintln!("[PS] {command}");
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
                    eprintln!("[PS stdout] {stdout}");
                }
                if !stderr.is_empty() {
                    eprintln!("[PS stderr] {stderr}");
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
            eprintln!("[skip] Port {port_name} already exists");
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
            eprintln!("[skip] Driver '{driver_name}' already installed");
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
    let cmd = format!("pnputil /add-driver '{inf_path}' /install");
    run_ps(&cmd, verbose)
}

/// Add a printer queue.
pub fn add_printer(name: &str, driver_name: &str, port_name: &str, verbose: bool) -> PsResult {
    let cmd = format!(
        "Add-Printer -Name '{}' -DriverName '{}' -PortName '{}'",
        escape_ps_string(name),
        escape_ps_string(driver_name),
        escape_ps_string(port_name),
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

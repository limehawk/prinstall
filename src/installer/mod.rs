pub mod powershell;

use crate::models::InstallResult;

/// Install a printer: create port, install driver, add printer queue.
pub fn install_printer(
    ip: &str,
    driver_name: &str,
    printer_name: &str,
    model: &str,
    verbose: bool,
) -> InstallResult {
    let port_name = format!("IP_{ip}");

    // Step 1: Create TCP/IP port
    let port_result = powershell::create_port(ip, verbose);
    if !port_result.success {
        return InstallResult {
            success: false,
            printer_name: printer_name.to_string(),
            driver_name: driver_name.to_string(),
            port_name: port_name.clone(),
            error: Some(format!("Failed to create port: {}", port_result.stderr)),
        };
    }

    // Step 2: Install driver
    let driver_result = powershell::install_driver(driver_name, verbose);
    if !driver_result.success {
        return InstallResult {
            success: false,
            printer_name: printer_name.to_string(),
            driver_name: driver_name.to_string(),
            port_name: port_name.clone(),
            error: Some(format!("Failed to install driver: {}", driver_result.stderr)),
        };
    }

    // Step 3: Add printer queue
    let printer_result = powershell::add_printer(printer_name, driver_name, &port_name, verbose);
    if !printer_result.success {
        return InstallResult {
            success: false,
            printer_name: printer_name.to_string(),
            driver_name: driver_name.to_string(),
            port_name,
            error: Some(format!("Failed to add printer: {}", printer_result.stderr)),
        };
    }

    let result = InstallResult {
        success: true,
        printer_name: printer_name.to_string(),
        driver_name: driver_name.to_string(),
        port_name,
        error: None,
    };

    // Record to history
    crate::history::record_install(model, driver_name, "install");

    result
}

pub mod powershell;

use crate::models::{InstallDetail, PrinterOpResult};

/// Install a printer: create port, install driver, add printer queue.
pub fn install_printer(
    ip: &str,
    driver_name: &str,
    printer_name: &str,
    model: &str,
    verbose: bool,
) -> PrinterOpResult {
    let port_name = format!("IP_{ip}");

    // Step 1: Create TCP/IP port
    let port_result = powershell::create_port(ip, verbose);
    if !port_result.success {
        return PrinterOpResult::err(format!("Failed to create port: {}", port_result.stderr));
    }

    // Step 2: Install driver
    let driver_result = powershell::install_driver(driver_name, verbose);
    if !driver_result.success {
        return PrinterOpResult::err(format!(
            "Failed to install driver: {}",
            driver_result.stderr
        ));
    }

    // Step 3: Add printer queue
    let printer_result = powershell::add_printer(printer_name, driver_name, &port_name, verbose);
    if !printer_result.success {
        return PrinterOpResult::err(format!(
            "Failed to add printer: {}",
            printer_result.stderr
        ));
    }

    // Record to history
    crate::history::record_install(model, driver_name, "install");

    PrinterOpResult::ok(InstallDetail {
        printer_name: printer_name.to_string(),
        driver_name: driver_name.to_string(),
        port_name,
        warning: None,
    })
}

/// Update a USB/local printer's driver (no port/queue creation).
/// Steps: install driver → set printer driver.
pub fn update_printer_driver(
    printer_name: &str,
    driver_name: &str,
    model: &str,
    verbose: bool,
) -> PrinterOpResult {
    // Step 1: Install driver (same as network flow)
    let driver_result = powershell::install_driver(driver_name, verbose);
    if !driver_result.success {
        return PrinterOpResult::err(format!(
            "Failed to install driver: {}",
            driver_result.stderr
        ));
    }

    // Step 2: Update the printer to use the new driver
    let update_result = powershell::set_printer_driver(printer_name, driver_name, verbose);
    if !update_result.success {
        return PrinterOpResult::err(format!(
            "Failed to update driver: {}",
            update_result.stderr
        ));
    }

    crate::history::record_install(model, driver_name, "usb_update");

    PrinterOpResult::ok(InstallDetail {
        printer_name: printer_name.to_string(),
        driver_name: driver_name.to_string(),
        port_name: String::new(),
        warning: None,
    })
}

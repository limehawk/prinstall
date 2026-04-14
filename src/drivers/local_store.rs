use crate::installer::powershell;

/// Get list of printer driver names from the local driver store.
/// Uses Get-PrinterDriver via PowerShell.
pub fn list_drivers(verbose: bool) -> Vec<String> {
    powershell::list_local_drivers(verbose)
}

/// Get printer drivers from the local driver store alongside their
/// `DriverDate` property. Each tuple is `(driver_name, optional_date)`
/// where the date is already normalized to `YYYY-MM-DD` by the PS
/// `ToString('yyyy-MM-dd')` call in the pipeline. Returns an empty Vec
/// when the PS call fails or the box has no queues — callers should
/// treat missing dates as "unknown", not as an error signal.
pub fn list_drivers_with_dates(verbose: bool) -> Vec<(String, Option<String>)> {
    powershell::list_local_drivers_with_dates(verbose)
}

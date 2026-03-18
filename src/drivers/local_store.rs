use crate::installer::powershell;

/// Get list of printer driver names from the local driver store.
/// Uses Get-PrinterDriver via PowerShell.
pub fn list_drivers(verbose: bool) -> Vec<String> {
    powershell::list_local_drivers(verbose)
}

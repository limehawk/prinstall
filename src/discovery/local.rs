use crate::models::Printer;

/// List printers already installed on the local Windows machine via Get-Printer.
/// Returns an empty Vec on non-Windows or if PowerShell is unavailable.
pub fn list_local_printers(_verbose: bool) -> Vec<Printer> {
    // Stub: full implementation queries Get-Printer via PowerShell.
    Vec::new()
}

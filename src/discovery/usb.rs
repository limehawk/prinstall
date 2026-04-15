//! USB printer discovery via `Get-PnpDevice`.
//!
//! Returns every USB-attached printing device Windows knows about — both
//! working queues and yellow-bang orphans that PnP could not auto-install
//! a driver for. The caller cross-references queue state and can use the
//! result to drive the `add --usb` flow for legacy printers.

use crate::core::executor::PsExecutor;
use crate::models::UsbDevice;
use serde::Deserialize;

#[derive(Deserialize)]
struct PnpRow {
    #[serde(rename = "FriendlyName")]
    friendly_name: Option<String>,
    #[serde(rename = "InstanceId")]
    instance_id: String,
    #[serde(rename = "Status")]
    status: Option<String>,
}

#[derive(Deserialize)]
struct QueueRow {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "PortName")]
    #[allow(dead_code)]
    port_name: Option<String>,
}

// Filter matches three USB-printing enumeration cases Windows can report:
//   1. `USBPRINT\*` — working USB print-class devices. This is how
//      driver-bound printers show up in `Get-PnpDevice`. Missing this
//      prefix was the original bug — scan --usb-only returned empty
//      even with a working printer attached.
//   2. `USB\*` + `Class -eq 'Printer'` / `'USBPrint'` — raw USB devices
//      that PnP has classified as printers but didn't shift onto the
//      USBPRINT bus (rare, some vendor-specific bindings).
//   3. `USB\*` + `Status -eq 'Error'` + vendor keyword in FriendlyName —
//      yellow-bang orphans where PnP couldn't load a driver. These are
//      what `add --usb` targets for stage-and-install.
const PNP_CMD: &str = "ConvertTo-Json -InputObject @(\
    Get-PnpDevice -PresentOnly | \
    Where-Object { \
        ($_.InstanceId -like 'USBPRINT\\*') -or \
        ($_.InstanceId -like 'USB\\*' -and \
            ($_.Class -eq 'Printer' -or $_.Class -eq 'USBPrint' -or \
             ($_.Status -eq 'Error' -and $_.FriendlyName -match 'print|LaserJet|DeskJet|OfficeJet|Brother|Canon|Epson|Kyocera|Lexmark|Xerox|Ricoh|HP'))) \
    } | \
    Select-Object FriendlyName, InstanceId, Status)";

const QUEUE_CMD: &str = "ConvertTo-Json -InputObject @(\
    Get-Printer | Where-Object { $_.PortName -like 'USB*' } | \
    Select-Object Name, PortName)";

/// Enumerate every USB-attached printing device Windows knows about,
/// marking each with its matching print queue name when one exists.
///
/// Devices with `has_error: true` and `queue_name: None` are the
/// yellow-bang orphans — what `add --usb` targets.
pub async fn enumerate(exec: &dyn PsExecutor, verbose: bool) -> Vec<UsbDevice> {
    let pnp_rows: Vec<PnpRow> =
        crate::core::executor::run_json(exec, PNP_CMD).unwrap_or_default();
    let queue_rows: Vec<QueueRow> =
        crate::core::executor::run_json(exec, QUEUE_CMD).unwrap_or_default();

    if verbose {
        eprintln!(
            "[usb] PnP returned {} device(s), queue cross-ref {} queue(s)",
            pnp_rows.len(),
            queue_rows.len()
        );
    }

    pnp_rows
        .into_iter()
        .map(|r| {
            let has_error = r
                .status
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case("Error"))
                .unwrap_or(false);
            let queue_name = r.friendly_name.as_ref().and_then(|fname| {
                queue_rows
                    .iter()
                    .find(|q| q.name.eq_ignore_ascii_case(fname))
                    .map(|q| q.name.clone())
            });
            UsbDevice {
                hardware_id: r.instance_id,
                friendly_name: r.friendly_name,
                queue_name,
                has_error,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use crate::installer::powershell::PsResult;

    fn ok(stdout: &str) -> PsResult {
        PsResult {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn parses_pnp_json_into_devices() {
        let pnp_json = r#"[
            {"FriendlyName":"HP LaserJet 1320","InstanceId":"USB\\VID_03F0&PID_1D17\\ABC","Status":"OK","Class":"Printer"},
            {"FriendlyName":"Unknown device","InstanceId":"USB\\VID_03F0&PID_1D17\\XYZ","Status":"Error","Class":"Unknown"}
        ]"#;
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok(pnp_json))
            .stub_contains("Get-Printer", ok("[]"));
        let devices = enumerate(&mock, false).await;
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].friendly_name.as_deref(), Some("HP LaserJet 1320"));
        assert!(!devices[0].has_error);
        assert!(devices[1].has_error);
        assert!(devices[0].queue_name.is_none());
    }

    #[tokio::test]
    async fn cross_references_queues_by_friendly_name() {
        let pnp_json = r#"[{"FriendlyName":"HP LaserJet 1320","InstanceId":"USB\\VID_03F0&PID_1D17\\ABC","Status":"OK","Class":"USBPrint"}]"#;
        let queues_json = r#"[{"Name":"HP LaserJet 1320","PortName":"USB001"}]"#;
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok(pnp_json))
            .stub_contains("Get-Printer", ok(queues_json));
        let devices = enumerate(&mock, false).await;
        assert_eq!(devices[0].queue_name.as_deref(), Some("HP LaserJet 1320"));
    }

    #[tokio::test]
    async fn empty_input_returns_empty_vec() {
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok("[]"))
            .stub_contains("Get-Printer", ok("[]"));
        let devices = enumerate(&mock, false).await;
        assert!(devices.is_empty());
    }

    #[tokio::test]
    async fn malformed_json_returns_empty_vec() {
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok("not json"))
            .stub_contains("Get-Printer", ok("[]"));
        let devices = enumerate(&mock, false).await;
        assert!(devices.is_empty());
    }

    /// Regression: working USB printers have `USBPRINT\...` InstanceIds
    /// (the USB Print Class bus), not `USB\...`. The filter must accept
    /// that prefix or scan --usb-only misses every driver-bound printer.
    #[tokio::test]
    async fn accepts_usbprint_prefix_for_working_printers() {
        let pnp_json = r#"[
            {"FriendlyName":"Brother MFC-L2750DW","InstanceId":"USBPRINT\\BROTHERMFC-L2750DW_SERIES7A4C\\7&312E9F27&0&USB001","Status":"OK","Class":"USBPrint"}
        ]"#;
        let queues_json = r#"[{"Name":"Brother MFC-L2750DW","PortName":"USB001"}]"#;
        let mock = MockExecutor::new()
            .stub_contains("Get-PnpDevice", ok(pnp_json))
            .stub_contains("Get-Printer", ok(queues_json));
        let devices = enumerate(&mock, false).await;
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].friendly_name.as_deref(),
            Some("Brother MFC-L2750DW")
        );
        assert_eq!(
            devices[0].queue_name.as_deref(),
            Some("Brother MFC-L2750DW"),
            "queue cross-ref should still work with USBPRINT\\ prefix"
        );
        assert!(!devices[0].has_error);
    }
}

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

const PNP_CMD: &str = "Get-PnpDevice -PresentOnly | \
    Where-Object { $_.InstanceId -like 'USB\\*' -and \
        ($_.Class -eq 'Printer' -or $_.Class -eq 'USBPrint' -or \
         ($_.Status -eq 'Error' -and $_.FriendlyName -match 'print|LaserJet|DeskJet|OfficeJet|Brother|Canon|Epson|Kyocera|Lexmark|Xerox|Ricoh|HP')) } | \
    Select-Object FriendlyName, InstanceId, Status | \
    ConvertTo-Json -InputObject @($_)";

const QUEUE_CMD: &str = "Get-Printer | Where-Object { $_.PortName -like 'USB*' } | \
    Select-Object Name, PortName | \
    ConvertTo-Json -InputObject @($_)";

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
}

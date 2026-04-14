use prinstall::core::executor::MockExecutor;
use prinstall::discovery;
use prinstall::installer::powershell::PsResult;
use prinstall::models::{ScanResult, UsbDevice};
use prinstall::output::format_scan_result_plain;

fn ok(stdout: &str) -> PsResult {
    PsResult {
        success: true,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

#[tokio::test]
async fn full_scan_result_returns_both_sections_from_mock() {
    let mock = MockExecutor::new()
        .stub_contains(
            "Get-PnpDevice",
            ok(r#"[{"FriendlyName":"HP LaserJet 1320","InstanceId":"USB\\VID_03F0&PID_1D17\\ABC","Status":"Error"}]"#),
        )
        .stub_contains("Get-Printer", ok("[]"));

    let result = discovery::full_scan_result(
        vec![],
        "public",
        &discovery::ScanMethod::All,
        std::time::Duration::from_millis(10),
        &mock,
        false,
    )
    .await;

    // No hosts = no network printers (mDNS pass runs but with a tiny 10ms
    // timeout it's overwhelmingly likely to return empty — we only assert
    // the USB side precisely).
    assert_eq!(result.usb.len(), 1);
    assert_eq!(
        result.usb[0].friendly_name.as_deref(),
        Some("HP LaserJet 1320")
    );
    assert!(result.usb[0].has_error);
    assert!(result.usb[0].queue_name.is_none());
}

#[tokio::test]
async fn orphan_device_triggers_install_hint_in_plain_output() {
    let result = ScanResult {
        network: vec![],
        usb: vec![UsbDevice {
            hardware_id: "USB\\VID_03F0&PID_1D17\\ABC".into(),
            friendly_name: Some("HP LaserJet 1320".into()),
            queue_name: None,
            has_error: true,
        }],
    };
    let out = format_scan_result_plain(&result);
    assert!(out.contains("HP LaserJet 1320"));
    assert!(out.contains("hint:"));
    assert!(out.contains("prinstall add --usb \"HP LaserJet 1320\""));
}

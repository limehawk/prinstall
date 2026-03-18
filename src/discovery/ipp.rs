use std::net::Ipv4Addr;
use std::time::Duration;

const IPP_PATHS: &[&str] = &["/ipp/print", "/ipp", "/"];
const IPP_TIMEOUT: Duration = Duration::from_millis(500);

/// Build a binary IPP Get-Printer-Attributes request.
pub fn build_get_printer_attributes(ip: &str) -> Vec<u8> {
    let printer_uri = format!("ipp://{ip}:631/ipp/print");
    let mut buf = Vec::new();

    // IPP version 2.0
    buf.extend_from_slice(&[2, 0]);
    // Operation: Get-Printer-Attributes (0x000B)
    buf.extend_from_slice(&[0x00, 0x0B]);
    // Request ID: 1
    buf.extend_from_slice(&[0, 0, 0, 1]);
    // Operation attributes group (tag 0x01)
    buf.push(0x01);

    write_ipp_attribute(&mut buf, 0x47, "attributes-charset", "utf-8");
    write_ipp_attribute(&mut buf, 0x48, "attributes-natural-language", "en");
    write_ipp_attribute(&mut buf, 0x45, "printer-uri", &printer_uri);
    write_ipp_attribute(
        &mut buf,
        0x44,
        "requested-attributes",
        "printer-make-and-model",
    );

    // End of attributes (tag 0x03)
    buf.push(0x03);
    buf
}

fn write_ipp_attribute(buf: &mut Vec<u8>, value_tag: u8, name: &str, value: &str) {
    buf.push(value_tag);
    buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
    buf.extend_from_slice(value.as_bytes());
}

/// Parse an IPP response to extract the printer-make-and-model attribute.
pub fn parse_printer_make_and_model(data: &[u8]) -> Option<String> {
    if data.len() < 9 {
        return None;
    }
    let mut pos = 8; // Skip header: version(2) + status(2) + request-id(4)
    let target = b"printer-make-and-model";

    while pos < data.len() {
        let tag = data[pos];
        pos += 1;

        if tag <= 0x0F {
            if tag == 0x03 {
                break;
            } // End of attributes
            continue;
        }

        if pos + 2 > data.len() {
            break;
        }
        let name_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if pos + name_len > data.len() {
            break;
        }
        let name = &data[pos..pos + name_len];
        pos += name_len;
        if pos + 2 > data.len() {
            break;
        }
        let value_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if pos + value_len > data.len() {
            break;
        }
        let value = &data[pos..pos + value_len];
        pos += value_len;

        if name == target {
            let s = String::from_utf8_lossy(value).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Query a printer via IPP to get its make-and-model string.
pub async fn identify_printer_ipp(ip: Ipv4Addr, verbose: bool) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(IPP_TIMEOUT)
        .danger_accept_invalid_certs(true)
        .build()
        .ok()?;

    let request_body = build_get_printer_attributes(&ip.to_string());

    for path in IPP_PATHS {
        let url = format!("http://{ip}:631{path}");
        if verbose {
            eprintln!("[scan] {ip}: trying IPP at {url}");
        }

        match client
            .post(&url)
            .header("Content-Type", "application/ipp")
            .body(request_body.clone())
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(body) = resp.bytes().await
                    && let Some(model) = parse_printer_make_and_model(&body)
                {
                    if verbose {
                        eprintln!("[scan] {ip}: IPP → \"{model}\"");
                    }
                    return Some(model);
                }
            }
            Err(e) => {
                if verbose {
                    eprintln!("[scan] {ip}: IPP {path} failed: {e}");
                }
            }
        }
    }

    if verbose {
        eprintln!("[scan] {ip}: IPP → no model found");
    }
    None
}

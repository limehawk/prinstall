//! INF file parser and HWID synthesizer.
//!
//! Used by the catalog-based driver resolver: parse a downloaded INF, then
//! check whether candidate hardware IDs derived from an IPP device-id string
//! actually appear in the INF's `[Models]` section. If they don't, the package
//! is rejected — no gambling.

use std::path::Path;

/// A single model entry parsed from an INF's [Models] section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwidEntry {
    pub display_name: String,
    pub install_section: String,
    pub hwid: String,
}

/// Parsed view of an INF file, focused on what we need for driver matching.
#[derive(Debug, Clone)]
pub struct InfData {
    /// Value of the [Version] section's `Provider` key, unquoted.
    pub provider: Option<String>,
    /// Value of the [Version] section's `DriverVer` key (format: "MM/DD/YYYY,X.Y.Z.W").
    pub driver_ver: Option<String>,
    /// All HWID entries from all relevant [Models] sections (amd64 first,
    /// bare fallback second). Multiple entries per display name are preserved.
    pub hwids: Vec<HwidEntry>,
}

/// Parse an INF file from disk. Handles UTF-16 LE/BE BOMs and UTF-8.
pub fn parse_inf(path: &Path) -> Result<InfData, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Failed to read INF file {}: {e}", path.display()))?;
    let text = decode_inf_bytes(&bytes)
        .map_err(|e| format!("Failed to decode INF file {}: {e}", path.display()))?;
    parse_inf_str(&text)
}

/// Decode INF bytes to a UTF-8 string. Detects UTF-16 LE/BE via BOM, falls
/// back to UTF-8.
fn decode_inf_bytes(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        // UTF-16 LE
        let payload = &bytes[2..];
        if !payload.len().is_multiple_of(2) {
            return Err("UTF-16 LE payload has odd byte length".to_string());
        }
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        decode_utf16_units(&units)
    } else if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // UTF-16 BE
        let payload = &bytes[2..];
        if !payload.len().is_multiple_of(2) {
            return Err("UTF-16 BE payload has odd byte length".to_string());
        }
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        decode_utf16_units(&units)
    } else {
        // UTF-8 (with optional BOM)
        let payload = if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
            &bytes[3..]
        } else {
            bytes
        };
        std::str::from_utf8(payload)
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {e}"))
    }
}

fn decode_utf16_units(units: &[u16]) -> Result<String, String> {
    let mut out = String::with_capacity(units.len());
    for r in std::char::decode_utf16(units.iter().copied()) {
        match r {
            Ok(c) => out.push(c),
            Err(e) => return Err(format!("Invalid UTF-16 sequence: {e}")),
        }
    }
    Ok(out)
}

/// Parse INF text that's already been decoded to UTF-8.
pub fn parse_inf_str(s: &str) -> Result<InfData, String> {
    // Strip a leading UTF-8/UTF-16-decoded BOM if it survived decoding.
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);

    // First pass: collect raw section content keyed by section name.
    // Section headers may legally repeat — merge their bodies in order.
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<String> = None;

    for raw_line in s.lines() {
        // Strip CR and trailing whitespace from CRLF endings.
        let line = raw_line.trim_end_matches('\r');
        let stripped = strip_comment(line);
        let trimmed = stripped.trim();

        if trimmed.is_empty() {
            continue;
        }

        if let Some(name) = parse_section_header(trimmed) {
            current = Some(name);
            continue;
        }

        if let Some(ref name) = current {
            // Append to existing section if present, else create new.
            if let Some((_, body)) = sections.iter_mut().find(|(n, _)| n == name) {
                body.push(stripped.to_string());
            } else {
                sections.push((name.clone(), vec![stripped.to_string()]));
            }
        }
    }

    // Extract Version fields.
    let (provider, driver_ver) = parse_version_section(&sections);

    // Find manufacturer section → list of (section_base, target) pairs.
    let manufacturer_targets = parse_manufacturer_section(&sections);

    // For each manufacturer target, look for matching model section.
    // Priority: NTamd64, then bare base name. Other targets ignored.
    let mut hwids: Vec<HwidEntry> = Vec::new();
    let mut seen_sections: Vec<String> = Vec::new();

    for (base, targets) in &manufacturer_targets {
        // Pass 1: NTamd64
        if targets.iter().any(|t| t.eq_ignore_ascii_case("NTamd64")) {
            let section_name = format!("{base}.NTamd64");
            if let Some((_, body)) = find_section(&sections, &section_name)
                && !seen_sections.contains(&section_name)
            {
                seen_sections.push(section_name.clone());
                hwids.extend(parse_model_lines(body));
            }
        }

        // Pass 2: bare fallback. Only if no NTamd64 hits and the base section
        // exists as a standalone.
        let bare = base.clone();
        if let Some((_, body)) = find_section(&sections, &bare)
            && !seen_sections.contains(&bare)
        {
            seen_sections.push(bare);
            hwids.extend(parse_model_lines(body));
        }
    }

    Ok(InfData {
        provider,
        driver_ver,
        hwids,
    })
}

fn find_section<'a>(
    sections: &'a [(String, Vec<String>)],
    name: &str,
) -> Option<&'a (String, Vec<String>)> {
    sections.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
}

/// Strip a `;` comment from a line, but only if the `;` is outside a
/// double-quoted string.
fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ';' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

/// `[name]` → `Some("name")`.
fn parse_section_header(trimmed: &str) -> Option<String> {
    if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        let inner = inner.trim();
        if inner.is_empty() {
            None
        } else {
            Some(inner.to_string())
        }
    } else {
        None
    }
}

/// Pull `Provider` and `DriverVer` from the [Version] section, if present.
fn parse_version_section(
    sections: &[(String, Vec<String>)],
) -> (Option<String>, Option<String>) {
    let Some((_, body)) = find_section(sections, "Version") else {
        return (None, None);
    };

    let mut provider = None;
    let mut driver_ver = None;

    for line in body {
        if let Some((key, value)) = split_kv(line) {
            let key_unq = unquote(key.trim());
            let val_unq = unquote(value.trim());
            match key_unq.to_ascii_lowercase().as_str() {
                "provider" => provider = Some(val_unq),
                "driverver" => driver_ver = Some(val_unq),
                _ => {}
            }
        }
    }

    (provider, driver_ver)
}

/// Split a line on the first `=`, returning `(key, value)`.
fn split_kv(line: &str) -> Option<(&str, &str)> {
    line.split_once('=')
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Parse the [Manufacturer] section into a list of (section_base, [targets]).
///
/// Each line is `%Key%=SectionBase[,Target1,Target2,...]`. The base is the
/// raw token to the right of `=`, and the targets are arch suffixes used to
/// build the actual model section name (`SectionBase.Target`).
fn parse_manufacturer_section(
    sections: &[(String, Vec<String>)],
) -> Vec<(String, Vec<String>)> {
    let Some((_, body)) = find_section(sections, "Manufacturer") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for line in body {
        let Some((_lhs, rhs)) = split_kv(line) else {
            continue;
        };
        let parts: Vec<&str> = rhs.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let base = parts[0].to_string();
        let targets: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
        out.push((base, targets));
    }
    out
}

/// Parse model section lines of the form:
///   "Display Name" = InstallSection,HWID1[,HWID2,...]
fn parse_model_lines(body: &[String]) -> Vec<HwidEntry> {
    let mut out = Vec::new();

    for line in body {
        let Some((lhs, rhs)) = split_kv(line) else {
            continue;
        };
        let display = unquote(lhs.trim());
        if display.is_empty() {
            continue;
        }

        // Right-hand side: InstallSection,HWID1[,HWID2,...]
        let parts: Vec<&str> = rhs.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            continue;
        }
        let install = parts[0].to_string();
        for hwid in &parts[1..] {
            out.push(HwidEntry {
                display_name: display.clone(),
                install_section: install.clone(),
                hwid: (*hwid).to_string(),
            });
        }
    }

    out
}

/// Synthesize candidate PnP hardware IDs from a printer-identification string.
///
/// Two input formats are recognized:
///
/// * **IEEE 1284 IPP device ID** —
///   `MFG:Brother;CMD:PJL,PCL;MDL:MFC-L2750DW series;CID:Brother Laser Type1;`
///   Emits, in priority order:
///     1. `1284_CID_<NORMALIZED_CID>` — canonical Microsoft CID-derived form
///     2. `<NORMALIZED_CID>` alone — secondary form some INFs use
///     3. `<NORMALIZED_MFG><NORMALIZED_MDL>` — long-shot model-based form
///
/// * **USB PnP InstanceId** — `USB\VID_03F0&PID_1D17\ABC123` (from
///   `Get-PnpDevice`). The per-device serial suffix after the second `\`
///   is stripped so the output matches the `USB\VID_xxxx&PID_yyyy` entries
///   that commonly appear in INF `[Models]` sections. Emits:
///     1. `USB\VID_xxxx&PID_yyyy` — full VID/PID (most specific)
///     2. `USB\VID_xxxx` — VID-only fallback
///
/// Detection is by prefix: inputs starting with `USB\` (case-insensitive)
/// are treated as USB InstanceIds; everything else falls through to the
/// IPP 1284 parser.
pub fn synthesize_hwids(device_id: &str) -> Vec<String> {
    // USB InstanceId fast path — must check before IPP parsing because
    // a USB\VID_... string never contains `;` or `:`.
    let trimmed = device_id.trim();
    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("USB\\") {
        return synthesize_usb_hwids(trimmed);
    }

    let mut mfg: Option<String> = None;
    let mut mdl: Option<String> = None;
    let mut cid: Option<String> = None;

    for piece in device_id.split(';') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let Some((key, value)) = piece.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "MFG" => mfg = Some(value),
            "MDL" => mdl = Some(value),
            "CID" => cid = Some(value),
            _ => {}
        }
    }

    let mut out = Vec::new();

    if let Some(c) = cid.as_deref() {
        let norm = normalize(c);
        if !norm.is_empty() {
            out.push(format!("1284_CID_{norm}"));
            out.push(norm);
        }
    }

    if let Some(m) = mdl.as_deref() {
        let mfg_norm = mfg.as_deref().map(normalize).unwrap_or_default();
        let mdl_norm = normalize(m);
        if !mdl_norm.is_empty() {
            let combined = if mfg_norm.is_empty() {
                mdl_norm
            } else {
                format!("{mfg_norm}{mdl_norm}")
            };
            if !combined.is_empty() && !out.contains(&combined) {
                out.push(combined);
            }
        }
    }

    out
}

/// Emit candidate HWIDs for a USB PnP InstanceId.
///
/// Trims a per-device serial suffix (everything after the second `\`) so the
/// output matches how vendor INFs list their device IDs. Returns an empty
/// list for inputs that don't contain a `VID_` segment.
fn synthesize_usb_hwids(instance_id: &str) -> Vec<String> {
    // Strip the trailing `\<serial>` segment if present. A bare
    // `USB\VID_xxxx&PID_yyyy` input returns unchanged from this step.
    let without_serial = match instance_id.match_indices('\\').nth(1) {
        Some((idx, _)) => &instance_id[..idx],
        None => instance_id,
    };

    let parts: Vec<&str> = without_serial.splitn(2, '\\').collect();
    if parts.len() != 2 {
        return Vec::new();
    }
    let prefix = parts[0]; // "USB" (any case)
    let body = parts[1]; // "VID_xxxx&PID_yyyy" (or similar)

    // Canonicalize the `USB\` prefix to upper case; vendors are consistent
    // about this but PnP occasionally reports mixed case.
    let prefix_upper = prefix.to_ascii_uppercase();

    let mut out = Vec::new();
    let full = format!("{prefix_upper}\\{body}");
    out.push(full);

    // Extract the first `VID_xxxx` segment for the VID-only fallback. Split
    // on `&` to pull out the VID alone — PID-less INF entries are common.
    let vid_only = body
        .split('&')
        .find(|seg| {
            let up = seg.to_ascii_uppercase();
            up.starts_with("VID_")
        })
        .map(|vid| format!("{prefix_upper}\\{vid}"));
    if let Some(v) = vid_only
        && !out.contains(&v)
    {
        out.push(v);
    }

    out
}

/// Normalize a string for HWID comparison: uppercase, replace non-alphanumeric
/// with `_`, collapse runs of `_`, trim leading/trailing `_`.
fn normalize(s: &str) -> String {
    let upper: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();

    // Collapse consecutive underscores.
    let mut collapsed = String::with_capacity(upper.len());
    let mut prev_us = false;
    for c in upper.chars() {
        if c == '_' {
            if !prev_us {
                collapsed.push('_');
            }
            prev_us = true;
        } else {
            collapsed.push(c);
            prev_us = false;
        }
    }

    collapsed.trim_matches('_').to_string()
}

/// Find the first HwidEntry whose `hwid` matches any candidate (case-insensitive).
pub fn find_matching<'a>(inf: &'a InfData, candidates: &[String]) -> Option<&'a HwidEntry> {
    for cand in candidates {
        for entry in &inf.hwids {
            if entry.hwid.eq_ignore_ascii_case(cand) {
                return Some(entry);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BROTHER_DEVICE_ID: &str =
        "MFG:Brother;CMD:PJL,PCL,PCLXL,URF;MDL:MFC-L2750DW series;CLS:PRINTER;CID:Brother Laser Type1;URF:W8,CP1";

    #[test]
    fn synthesize_hwids_emits_cid_form() {
        let hwids = synthesize_hwids(BROTHER_DEVICE_ID);
        assert!(!hwids.is_empty());
        assert_eq!(hwids[0], "1284_CID_BROTHER_LASER_TYPE1");
        assert!(hwids.contains(&"BROTHER_LASER_TYPE1".to_string()));
    }

    #[test]
    fn synthesize_hwids_handles_missing_cid() {
        let id = "MFG:Brother;MDL:HL-L2390DW series;CLS:PRINTER";
        let hwids = synthesize_hwids(id);
        // No CID, but MDL+MFG should still produce a candidate.
        assert!(!hwids.is_empty());
        assert!(hwids.iter().any(|h| h.contains("HL_L2390DW")));
        assert!(hwids.iter().any(|h| h.starts_with("BROTHER")));
    }

    #[test]
    fn synthesize_hwids_returns_empty_on_garbage() {
        assert!(synthesize_hwids("").is_empty());
        assert!(synthesize_hwids("garbage with no colons or semicolons").is_empty());
    }

    #[test]
    fn synthesize_hwids_usb_instance_id() {
        let hwids = synthesize_hwids("USB\\VID_03F0&PID_1D17\\ABC");
        assert_eq!(
            hwids,
            vec![
                "USB\\VID_03F0&PID_1D17".to_string(),
                "USB\\VID_03F0".to_string(),
            ]
        );
    }

    #[test]
    fn synthesize_hwids_usb_no_serial() {
        // Idempotent: feeding in a USB id that lacks the trailing serial
        // produces the same full + VID-only candidates.
        let hwids = synthesize_hwids("USB\\VID_03F0&PID_1D17");
        assert_eq!(
            hwids,
            vec![
                "USB\\VID_03F0&PID_1D17".to_string(),
                "USB\\VID_03F0".to_string(),
            ]
        );
    }

    #[test]
    fn synthesize_normalize_uppercases_and_underscores() {
        // Drive normalize through the public API.
        let hwids = synthesize_hwids("CID:brother laser-type1");
        assert!(hwids.contains(&"1284_CID_BROTHER_LASER_TYPE1".to_string()));
        assert!(hwids.contains(&"BROTHER_LASER_TYPE1".to_string()));

        // Multiple non-alnum collapse to a single underscore, leading/trailing trimmed.
        let hwids2 = synthesize_hwids("CID:  ---foo!!bar---  ");
        assert!(hwids2.contains(&"1284_CID_FOO_BAR".to_string()));
    }

    #[test]
    fn parse_inf_str_reads_version_fields() {
        let inf = r#"
[Version]
Signature="$Windows NT$"
Provider="Brother"
Class=Printer
DriverVer = 04/22/2009,10.0.17119.1
"#;
        let data = parse_inf_str(inf).expect("parse should succeed");
        assert_eq!(data.provider.as_deref(), Some("Brother"));
        assert!(data.driver_ver.is_some());
        assert!(data.driver_ver.as_ref().unwrap().contains("04/22/2009"));
    }

    #[test]
    fn parse_inf_str_reads_manufacturer_and_models() {
        let inf = r#"
[Version]
Provider="Acme"
DriverVer=01/01/2020,1.0.0.0

[Manufacturer]
%Acme%=ACME_PRN,NTamd64

[ACME_PRN.NTamd64]
"Acme Laser Pro" = AcmeInst,1284_CID_ACME_LASER_PRO
"#;
        let data = parse_inf_str(inf).expect("parse should succeed");
        assert_eq!(data.hwids.len(), 1);
        assert_eq!(data.hwids[0].display_name, "Acme Laser Pro");
        assert_eq!(data.hwids[0].install_section, "AcmeInst");
        assert_eq!(data.hwids[0].hwid, "1284_CID_ACME_LASER_PRO");
    }

    #[test]
    fn parse_inf_str_handles_multiple_hwids_same_display_name() {
        let inf = r#"
[Version]
Provider="Acme"

[Manufacturer]
%Acme%=ACME_PRN,NTamd64

[ACME_PRN.NTamd64]
"Acme Laser" = AcmeInst,{12345678-1234-1234-1234-123456789012}
"Acme Laser" = AcmeInst,1284_CID_ACME_LASER
"#;
        let data = parse_inf_str(inf).expect("parse should succeed");
        assert_eq!(data.hwids.len(), 2);
        assert!(data.hwids.iter().all(|h| h.display_name == "Acme Laser"));
        assert!(data.hwids.iter().all(|h| h.install_section == "AcmeInst"));
        assert!(data.hwids.iter().any(|h| h.hwid == "1284_CID_ACME_LASER"));
        assert!(data
            .hwids
            .iter()
            .any(|h| h.hwid == "{12345678-1234-1234-1234-123456789012}"));
    }

    #[test]
    fn parse_inf_str_handles_comma_separated_hwids_one_line() {
        let inf = r#"
[Version]
Provider="Acme"

[Manufacturer]
%Acme%=ACME_PRN,NTamd64

[ACME_PRN.NTamd64]
"Acme Multi" = AcmeInst,HWID1,HWID2,HWID3
"#;
        let data = parse_inf_str(inf).expect("parse should succeed");
        assert_eq!(data.hwids.len(), 3);
        assert_eq!(data.hwids[0].hwid, "HWID1");
        assert_eq!(data.hwids[1].hwid, "HWID2");
        assert_eq!(data.hwids[2].hwid, "HWID3");
        assert!(data.hwids.iter().all(|h| h.display_name == "Acme Multi"));
        assert!(data.hwids.iter().all(|h| h.install_section == "AcmeInst"));
    }

    #[test]
    fn parse_inf_str_strips_comments() {
        let inf = r#"
; this is a header comment
[Version]
Provider="Acme" ; trailing comment
DriverVer=01/01/2020,1.0.0.0

[Manufacturer]
%Acme%=ACME_PRN,NTamd64 ; another trailing comment

[ACME_PRN.NTamd64]
; full-line comment inside section
"Acme One" = AcmeInst,HWID_ONE ; inline comment
"#;
        let data = parse_inf_str(inf).expect("parse should succeed");
        assert_eq!(data.provider.as_deref(), Some("Acme"));
        assert_eq!(data.hwids.len(), 1);
        assert_eq!(data.hwids[0].hwid, "HWID_ONE");
    }

    #[test]
    fn parse_inf_str_ignores_unrelated_sections() {
        let inf = r#"
[Version]
Provider="Acme"

[Manufacturer]
%Acme%=ACME_PRN,NTamd64

[ACME_PRN.NTamd64]
"Acme One" = AcmeInst,HWID_ONE

[Strings]
Acme="Acme Corporation"
RandomKey="Some random text with [brackets] and = signs"

[SourceDisksNames]
1 = "Acme Disk",,,
"#;
        let data = parse_inf_str(inf).expect("parse should succeed");
        assert_eq!(data.hwids.len(), 1);
        assert_eq!(data.provider.as_deref(), Some("Acme"));
    }

    fn fixture_path() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests");
        p.push("fixtures");
        p.push("brother_type1.inf");
        p
    }

    #[test]
    fn parse_inf_fixture_brother_type1() {
        let path = fixture_path();
        let data = parse_inf(&path).expect("fixture should parse");
        assert_eq!(data.provider.as_deref(), Some("Brother"));
        assert!(
            data.driver_ver
                .as_ref()
                .map(|v| v.contains("04/22/2009"))
                .unwrap_or(false),
            "driver_ver was {:?}",
            data.driver_ver
        );
        assert!(!data.hwids.is_empty(), "expected non-empty hwids");
        assert!(
            data.hwids
                .iter()
                .any(|h| h.hwid == "1284_CID_BROTHER_LASER_TYPE1"),
            "expected 1284_CID_BROTHER_LASER_TYPE1 in fixture hwids"
        );
    }

    #[test]
    fn find_matching_returns_cid_entry_from_brother_fixture() {
        let path = fixture_path();
        let data = parse_inf(&path).expect("fixture should parse");

        let candidates = synthesize_hwids(BROTHER_DEVICE_ID);
        assert!(!candidates.is_empty());

        let hit = find_matching(&data, &candidates).expect("expected a match");
        assert_eq!(hit.hwid, "1284_CID_BROTHER_LASER_TYPE1");
        assert_eq!(hit.install_section, "BRIBMF01");
        assert_eq!(hit.display_name, "Brother Laser Type1 Class Driver");
    }
}

use std::net::Ipv4Addr;

/// Decompose a CIDR string into its network address, prefix length, and
/// host-bit mask. Shared by every public helper in this module.
fn decompose_cidr(cidr: &str) -> Result<(u32, u32, u32), String> {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("invalid CIDR notation: {cidr} (expected format: x.x.x.x/N)"));
    }

    let base_ip: Ipv4Addr = parts[0]
        .parse()
        .map_err(|e| format!("invalid IP address '{}': {e}", parts[0]))?;

    let prefix_len: u32 = parts[1]
        .parse()
        .map_err(|e| format!("invalid prefix length '{}': {e}", parts[1]))?;

    if prefix_len > 32 {
        return Err(format!("prefix length {prefix_len} is out of range (0-32)"));
    }

    let ip_u32 = u32::from(base_ip);
    let mask = if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
    let network = ip_u32 & mask;

    Ok((network, prefix_len, mask))
}

/// Normalize a CIDR string so the IP is the true network address.
/// `10.10.20.1/24` → `10.10.20.0/24`, `10.10.20.0/24` → unchanged.
pub fn normalize_cidr(cidr: &str) -> Result<String, String> {
    let (network, prefix_len, _) = decompose_cidr(cidr)?;
    Ok(format!("{}/{prefix_len}", Ipv4Addr::from(network)))
}

/// Parse a CIDR notation string into a list of host IPs.
/// Excludes network and broadcast addresses.
/// The IP in the CIDR string is masked to the network address first, so
/// `10.10.20.1/24` and `10.10.20.0/24` produce the identical host list.
pub fn parse_cidr(cidr: &str) -> Result<Vec<Ipv4Addr>, String> {
    let (network, prefix_len, mask) = decompose_cidr(cidr)?;
    let broadcast = network | !mask;

    let mut hosts = Vec::new();
    // For /31 and /32, return all addresses (point-to-point or single host)
    if prefix_len >= 31 {
        for addr in network..=broadcast {
            hosts.push(Ipv4Addr::from(addr));
        }
    } else {
        // Exclude network and broadcast addresses
        for addr in (network + 1)..broadcast {
            hosts.push(Ipv4Addr::from(addr));
        }
    }

    Ok(hosts)
}

/// Parse the output of the subnet auto-detection PowerShell command.
/// Input format: "192.168.1.100/24" (one per line).
/// Returns the CIDR with network address (e.g., "192.168.1.0/24").
pub fn parse_auto_detect_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('/').collect();
        if parts.len() != 2 {
            continue;
        }
        let ip: Ipv4Addr = match parts[0].parse() {
            Ok(ip) => ip,
            Err(_) => continue,
        };
        let octets = ip.octets();
        if octets[0] == 169 && octets[1] == 254 { continue; }
        if octets[0] == 127 { continue; }
        let prefix_len: u32 = match parts[1].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let ip_u32 = u32::from(ip);
        let mask = if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        let network = Ipv4Addr::from(ip_u32 & mask);
        return Some(format!("{network}/{prefix_len}"));
    }
    None
}

/// Auto-detect the local subnet via PowerShell.
/// Returns a CIDR string like "192.168.1.0/24".
pub fn auto_detect_subnet(verbose: bool) -> Option<String> {
    let cmd = "Get-NetIPAddress -AddressFamily IPv4 | \
        Where-Object { $_.InterfaceAlias -notlike '*Loopback*' -and \
        $_.InterfaceAlias -notlike '*tun*' -and \
        $_.InterfaceAlias -notlike '*tap*' } | \
        Sort-Object -Property { \
            (Get-NetRoute -InterfaceIndex $_.InterfaceIndex -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue) -ne $null \
        } -Descending | \
        Select-Object -First 1 | \
        ForEach-Object { \"$($_.IPAddress)/$($_.PrefixLength)\" }";

    let result = crate::installer::powershell::run_ps(cmd, verbose);
    if !result.success || result.stdout.is_empty() {
        if verbose {
            eprintln!("[subnet] Auto-detect failed: {}", result.stderr);
        }
        return None;
    }
    parse_auto_detect_output(&result.stdout)
}

/// Validate that a subnet isn't too large. /24 or smaller is fine.
/// Larger requires --force.
pub fn validate_subnet_size(cidr: &str, force: bool) -> Result<(), String> {
    let (_, prefix_len, _) = decompose_cidr(cidr)?;

    if prefix_len < 24 && !force {
        return Err(format!(
            "subnet /{prefix_len} is larger than /24 ({} hosts). \
             Use --force to scan anyway.",
            2u32.pow(32 - prefix_len) - 2
        ));
    }

    Ok(())
}

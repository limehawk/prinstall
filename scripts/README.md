# prinstall deploy scripts

Scripts for getting `prinstall` onto a Windows box. For RMM-flavored
per-command wrappers (SuperOps runtime variables, exit-code mapping, etc.)
see the [`rmm-scripts`](https://github.com/limehawk/rmm-scripts) repo —
those live there because their version tracks the `prinstall` app version
they target.

## `prinstall_setup.ps1`

Standalone installer for techs doing a manual install. Downloads the
latest `prinstall` release from GitHub, drops `prinstall.exe` into
`C:\ProgramData\prinstall\`, adds that directory to Machine PATH, and
creates a Windows Firewall rule for mDNS discovery (UDP 5353).

### Install (latest)

```powershell
iwr -useb https://raw.githubusercontent.com/limehawk/prinstall/main/scripts/prinstall_setup.ps1 | iex
```

### Install (specific version)

```powershell
# Download first, then run with -Version
iwr -useb https://raw.githubusercontent.com/limehawk/prinstall/main/scripts/prinstall_setup.ps1 -o prinstall_setup.ps1
.\prinstall_setup.ps1 -Version v0.4.15
```

### Uninstall

```powershell
.\prinstall_setup.ps1 -Uninstall
```

Removes the install dir, the firewall rule, and the PATH entry. No network
access needed.

### Requirements

- Windows 10 / Server 2016 or newer
- PowerShell 5.1+
- Administrator privileges (installs into ProgramData + modifies Machine PATH)
- Network access to `api.github.com` + `github.com` (install only)

## RMM deployment

For SuperOps/NinjaOne/ConnectWise runbooks, use the per-command wrappers
in the [`rmm-scripts`](https://github.com/limehawk/rmm-scripts) repo:

| Wrapper | Purpose |
|---|---|
| `prinstall_setup.ps1` | RMM install/uninstall with `$InstallOrUninstall` runtime var |
| `prinstall_scan.ps1` | Subnet scan |
| `prinstall_id.ps1` | Identify a printer by IP |
| `prinstall_drivers.ps1` | Show matched drivers for a printer |
| `prinstall_add.ps1` | Install a printer (network or USB) |
| `prinstall_remove.ps1` | Remove a printer + optional cleanup |
| `prinstall_list.ps1` | List installed printer queues |
| `prinstall_driver_add.ps1` | Stage a driver (by path or model) |
| `prinstall_driver_remove.ps1` | Remove a driver from the store |
| `prinstall_driver_list.ps1` | List drivers in the store |
| `prinstall_trust_codesign.ps1` | Push the self-signed cert to fleet TrustedPublisher |

Wrapper versions match the `prinstall` app version they target — a
`v0.4.15` wrapper targets the `prinstall` 0.4.15 CLI surface.

<div align="center">

```
┌─┐┬─┐┬┌┐┌┌─┐┌┬┐┌─┐┬  ┬
├─┘├┬┘││││└─┐ │ ├─┤│  │
┴  ┴└─┴┘└┘└─┘ ┴ ┴ ┴┴─┘┴─┘
```

### Discover. Match. Install.

**A Rust TUI and CLI for nuking printer setup pain on Windows.**

[![Release](https://img.shields.io/github/v/release/limehawk/prinstall?style=flat-square&color=orange&label=release)](https://github.com/limehawk/prinstall/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](#license)
[![Platform](https://img.shields.io/badge/platform-windows-0078D4?style=flat-square&logo=windows)](https://github.com/limehawk/prinstall/releases)
[![Built with Rust](https://img.shields.io/badge/built_with-rust-CE422B?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Build](https://img.shields.io/github/actions/workflow/status/limehawk/prinstall/release.yml?style=flat-square&label=build)](https://github.com/limehawk/prinstall/actions)

</div>

---

## Why

MSP technicians burn hours on printer installs. Find the IP, hunt the driver, wrestle `Add-Printer`, paste the wrong universal PCL6 again, repeat on the next site visit.

`prinstall` collapses the whole workflow into a single 5 MB binary. Scan a subnet. Pick a printer. Pick a driver. Done. Works over SSH or RMM remote shells with a clean CLI, or drops you into an interactive TUI when launched from a real terminal.

## Features

```
 ▸ Multi-method discovery   TCP port probe  ·  IPP  ·  SNMP  ·  Get-Printer
 ▸ Curated driver matching  17 manufacturers, fuzzy + hand-curated ranking
 ▸ Lazy-style TUI           Two-panel, vim keybindings, ratatui widgets
 ▸ Scriptable CLI           --json on every command for RMM automation
 ▸ Single binary            Embedded data, UAC manifest, static CRT
 ▸ Idempotent installs      Existing ports and drivers are reused, not duped
```

### Discovery, the multi-method way

Most printers have SNMP disabled out of the box, so SNMP-only scanners miss the majority of a real network. `prinstall` runs a layered pipeline:

| Phase | Method | Purpose |
|-------|--------|---------|
| 1 | TCP port probe (9100 / 631 / 515) | Find anything listening — fast parallel sweep |
| 2 | IPP (port 631) | Pull model from `printer-make-and-model` attribute |
| 3 | SNMP v2c | Enrich with serial, status, full model string |
| 4 | `Get-Printer` | Include locally installed USB / network queues |

Results are merged and deduplicated automatically.

### Driver matching

```
  ★ exact    Curated match from the known-matches database
  ● fuzzy    Name similarity above threshold
  ○ low      Weak partial match — verify before installing
```

Matched drivers are ranked by confidence. The manufacturer's universal drivers are always shown alongside them, so you've got a known-good fallback when an exact match isn't there.

## Install

Grab the latest Windows binary from [Releases](https://github.com/limehawk/prinstall/releases) and drop `prinstall.exe` anywhere on `PATH`.

Or build from source:

```bash
cargo install --git https://github.com/limehawk/prinstall
```

Cross-compiling from Linux works too — CI builds the Windows binary on tag push.

## Quick Start

```
prinstall                              Launch interactive TUI
prinstall scan                         Scan local subnet (auto-detected)
prinstall scan 192.168.1.0/24          Scan specific subnet
prinstall id 192.168.1.100             Identify a printer via SNMP
prinstall drivers 192.168.1.100        Show matched + universal drivers
prinstall install 192.168.1.100        Full install: port + driver + queue
prinstall list                         List printers Windows already knows
```

Global flags: `--json`, `--verbose`, `--community <str>`, `--force`.

## Usage

### Scan a subnet

```console
$ prinstall scan 192.168.1.0/24

  IP              MODEL                                STATUS
  192.168.1.12    HP LaserJet Pro MFP M428fdw          Ready
  192.168.1.47    Brother HL-L2370DW                   Ready
  192.168.1.88    RICOH MP C3004                       Ready
  192.168.1.104   Canon imageRUNNER ADVANCE C5535      Warming up

  Scanned 254 hosts  ·  4 printers found  ·  1.8s
```

Choose a method with `--method all|snmp|port`, tune with `--timeout <ms>`, override the community with `--community <str>`.

### Find drivers

```console
$ prinstall drivers 192.168.1.12

  Printer: HP LaserJet Pro MFP M428fdw
  Serial:  PHBDK01234

  MATCHED DRIVERS
    ★ HP LaserJet Pro MFP M428 PCL-6           exact, curated
    ● HP LaserJet Pro MFP M400 Series PCL6     fuzzy, 87%

  UNIVERSAL DRIVERS (HP)
    · HP Universal Print Driver PCL6
    · HP Universal Print Driver PS
```

### Install

```console
$ prinstall install 192.168.1.12

  [1/3] Add-PrinterPort    IP_192.168.1.12          OK
  [2/3] Add-PrinterDriver  HP LaserJet Pro MFP...   OK
  [3/3] Add-Printer        HP LaserJet Pro MFP...   OK

  Installed: HP LaserJet Pro MFP M428fdw
  History:   C:\ProgramData\prinstall\history.toml
```

### Automation (JSON mode)

```bash
prinstall scan 192.168.1.0/24 --json \
  | jq '.[] | select(.model | test("HP"; "i")) | .ip'
```

Every subcommand speaks `--json`. Pipe it through `jq`, feed it to PowerShell, chain it from your RMM runner — whatever fits.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Interface                            │
│     cli.rs (clap)   │   tui/ (ratatui)   │   output.rs      │
└─────────┬────────────────────┬────────────────────┬─────────┘
          │                    │                    │
┌─────────▼────────┐  ┌────────▼─────────┐  ┌──────▼──────────┐
│    Discovery     │  │      Drivers     │  │    Installer    │
│                  │  │                  │  │                 │
│   port_scan.rs   │  │    matcher.rs    │  │  powershell.rs  │
│       ipp.rs     │  │    manifest.rs   │  │                 │
│      snmp.rs     │  │ known_matches.rs │  │    Add-Port     │
│     local.rs     │  │  downloader.rs   │  │    Add-Driver   │
│    subnet.rs     │  │  local_store.rs  │  │    Add-Printer  │
└──────────────────┘  └──────────────────┘  └─────────────────┘
```

Four layers. One binary. One job.

**Design notes:**

- Data files (`data/drivers.toml`, `data/known_matches.toml`) are embedded at compile time via `include_str!()`. No sidecar files to lose.
- PowerShell strings are escaped at the boundary (`escape_ps_string`) to prevent injection.
- Install history logs to `C:\ProgramData\prinstall\history.toml` for audit and rollback context.
- UAC manifest embedded via `embed-manifest` so Windows prompts for elevation on launch.
- Static CRT linking produces a zero-dependency Windows binary.

## Requirements

- **Windows 10/11** (Server 2016+) for installation functions
- **Administrator privileges** — `Add-Printer` requires elevation. UAC triggers automatically.
- **Network** — UDP/161 (SNMP), TCP/9100 · 631 · 515 (port probe), TCP/631 (IPP)

SNMP is no longer required. The port probe pipeline handles printers that don't speak SNMP at all.

## Development

```bash
cargo test                       # Run the test suite
cargo clippy -- -W clippy::all   # Lint
cargo build --release            # Local dev build (Linux / macOS ok)
```

Windows release binaries are built automatically by GitHub Actions on tag push — see [`.github/workflows/release.yml`](.github/workflows/release.yml).

```
src/
├── main.rs              Entry point, CLI dispatch
├── lib.rs               Module declarations
├── cli.rs               clap subcommands with rich help
├── models.rs            Printer, DriverMatch, DriverResults, ...
├── output.rs            Plain-text and JSON formatters
├── privilege.rs         Windows admin detection
├── history.rs           Install history logging
├── discovery/           port_scan · ipp · snmp · local · subnet
├── drivers/             matcher · manifest · known_matches · downloader
├── installer/           PowerShell-driven install orchestration
└── tui/                 Two-panel ratatui UI
```

## Roadmap

- [ ] User-editable subnet input inside the TUI (auto-detect already works)
- [ ] Printer defaults — duplex, color/mono, paper size, set-default
- [ ] mDNS / WS-Discovery fallback for fully-silent printers
- [ ] Batch install mode (multiple IPs in one shot)
- [ ] Shared match database across fleet
- [ ] SignPath.io code signing for SmartScreen trust

## License

`prinstall` is released under the MIT License.

---

<div align="center">

Built in Rust.  ·  Born in an RMM shell.  ·  Designed for techs who just want the printer to work.

</div>

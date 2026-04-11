```
                                                                                   
▄▄▄▄▄▄▄   ▄▄▄▄▄▄▄   ▄▄▄▄▄ ▄▄▄    ▄▄▄  ▄▄▄▄▄▄▄ ▄▄▄▄▄▄▄▄▄   ▄▄▄▄   ▄▄▄      ▄▄▄      
███▀▀███▄ ███▀▀███▄  ███  ████▄  ███ █████▀▀▀ ▀▀▀███▀▀▀ ▄██▀▀██▄ ███      ███      
███▄▄███▀ ███▄▄███▀  ███  ███▀██▄███  ▀████▄     ███    ███  ███ ███      ███      
███▀▀▀▀   ███▀▀██▄   ███  ███  ▀████    ▀████    ███    ███▀▀███ ███      ███      
███       ███  ▀███ ▄███▄ ███    ███ ███████▀    ███    ███  ███ ████████ ████████ 
                                                                                   
                                                                                   
```

### Discover. Match. Add. Remove.

**Adding printers on Windows sucks. `prinstall` fixes it.**

[![Release](https://img.shields.io/github/v/release/limehawk/prinstall?style=flat-square&color=orange&label=release)](https://github.com/limehawk/prinstall/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](#license)
[![Platform](https://img.shields.io/badge/platform-windows-0078D4?style=flat-square&logo=windows)](https://github.com/limehawk/prinstall/releases)
[![Built with Rust](https://img.shields.io/badge/built_with-rust-CE422B?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Build](https://img.shields.io/github/actions/workflow/status/limehawk/prinstall/release.yml?style=flat-square&label=build)](https://github.com/limehawk/prinstall/actions)

One command. The right vendor driver, pulled from the Microsoft Update Catalog. Installed.

---

## Why

MSP technicians burn hours on printer installs. Find the IP, hunt the driver, wrestle `Add-Printer`, paste the wrong universal PCL6 again, curse, repeat on the next site visit. Then the printer breaks a week later and you get to do it all over again.

`prinstall` collapses the whole workflow into a single 12 MB binary. Scan a subnet, add a printer, remove it cleanly, audit what's installed. Works over SSH or RMM remote shells with a clean CLI (`--json` on every command), or drops into a dense lazy-style TUI when launched from a real terminal.

## Features

- **Multi-method discovery** — TCP port probe, IPP, SNMP, mDNS/Bonjour, and `Get-Printer` in one parallel pipeline
- **Deterministic driver resolution** — scrapes the Update Catalog, parses the INF, matches the synthesized HWID
- **Network + USB** — one binary, both install paths, idempotent
- **Clean remove** — queue, driver, and port teardown with spooler-lag retries
- **Readable errors** — PowerShell stderr parsed and HRESULT-decoded before you see it
- **Scriptable CLI, vim TUI** — `--json` for RMM automation, two-panel ratatui when you're on a real terminal

## Install

**Windows (PowerShell one-liner):**

```powershell
iwr https://github.com/limehawk/prinstall/releases/latest/download/prinstall.exe -OutFile prinstall.exe
```

**From source:**

```bash
cargo install --git https://github.com/limehawk/prinstall
```

## Quick start

```powershell
prinstall                             # launch the interactive TUI
prinstall scan                        # scan your subnet for printers
prinstall id 192.168.1.50             # identify one via SNMP + IPP
prinstall add 192.168.1.50            # install it
prinstall remove 192.168.1.50         # rip it out cleanly
```

Every command takes `--json` for scripting and `--verbose` for the full audit trail.

## The four-tier driver resolver

`prinstall add` walks the pipeline in priority order and only escalates when the previous tier comes up empty:

```
  TIER 1   Local driver store         Reuse what's already installed
  TIER 2   Manufacturer download      Pull from the embedded URL manifest
  TIER 3   Update Catalog + HWID      Search by IPP CID, parse INF, match hardware ID
  TIER 4   IPP Class Driver           The always-works safety net (Windows 8+)
```

Tier 3 is the clever bit — it downloads a candidate driver package, parses the INF, and confirms a `1284_CID_*` hardware-ID match **before** installing. No gambling on model names.

Full writeup: [Driver Resolution on the wiki](https://github.com/limehawk/prinstall/wiki/Driver-Resolution).

## Docs

- **Website** — [prinstall.limehawk.io](https://prinstall.limehawk.io)
- **Wiki** — [github.com/limehawk/prinstall/wiki](https://github.com/limehawk/prinstall/wiki)
- **Getting started** — [wiki/Getting-Started](https://github.com/limehawk/prinstall/wiki/Getting-Started)
- **Command reference** — [wiki/CLI-Reference](https://github.com/limehawk/prinstall/wiki/CLI-Reference)
- **Architecture** — [wiki/Architecture](https://github.com/limehawk/prinstall/wiki/Architecture)
- **Roadmap** — [wiki/Roadmap](https://github.com/limehawk/prinstall/wiki/Roadmap)

Data, history, and driver staging live under `C:\ProgramData\prinstall\`.

## License

MIT. Built by [limehawk](https://limehawk.io).

---

<div align="center">

*Built in Rust  ·  Born in an RMM shell  ·  Designed for techs who just want the printer to work*

</div>

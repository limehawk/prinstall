```
                                                                                   
▄▄▄▄▄▄▄   ▄▄▄▄▄▄▄   ▄▄▄▄▄ ▄▄▄    ▄▄▄  ▄▄▄▄▄▄▄ ▄▄▄▄▄▄▄▄▄   ▄▄▄▄   ▄▄▄      ▄▄▄      
███▀▀███▄ ███▀▀███▄  ███  ████▄  ███ █████▀▀▀ ▀▀▀███▀▀▀ ▄██▀▀██▄ ███      ███      
███▄▄███▀ ███▄▄███▀  ███  ███▀██▄███  ▀████▄     ███    ███  ███ ███      ███      
███▀▀▀▀   ███▀▀██▄   ███  ███  ▀████    ▀████    ███    ███▀▀███ ███      ███      
███       ███  ▀███ ▄███▄ ███    ███ ███████▀    ███    ███  ███ ████████ ████████ 
                                                                                   
                                                                                   
```
<p>
  <img src="assets/prinstall-icon.png" width="128" alt="prinstall" />
</p>

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

`prinstall` collapses the whole workflow into a single 8 MB binary. Scan a subnet, add a printer, remove it cleanly, audit what's installed. Works over SSH, RMM remote shells, or any local terminal with a clean CLI (`--json` on every command).

## Features

- **Multi-method discovery** — TCP port probe, IPP, SNMP, mDNS/Bonjour, and `Get-Printer` in one parallel pipeline
- **Deterministic driver resolution** — scrapes the Microsoft Update Catalog, downloads the CAB, parses the INF, and matches the synthesized HWID before installing
- **Structured output** — every `prinstall add` shows a phased report (Discovery → Driver Resolution → Install → Summary) with tier status and timing. `--verbose` adds raw PS commands for debugging
- **Network + USB** — one binary, both install paths, idempotent
- **Clean remove** — queue, driver, and port teardown with spooler-lag retries
- **Readable errors** — PowerShell stderr parsed and HRESULT-decoded before you see it
- **Scriptable CLI** — `--json` on every command for RMM automation, semantic coloring that respects `NO_COLOR`
- **SDI driver packs** *(opt-in)* — build with `--features sdi` to add Snappy Driver Installer Origin integration for Brother, Canon, Epson, Ricoh, and other vendors the Update Catalog doesn't carry

## Install

Each release ships two binaries:

| Binary | Size | SDI | Use case |
|---|---|---|---|
| `prinstall.exe` | ~8 MB | No | Default — Tiers 1–3 + IPP fallback |
| `prinstall-sdi.exe` | ~9 MB | Yes | Extended — adds Tier 4 SDI driver packs |

**Windows (PowerShell one-liner):**

```powershell
# Standard build (no SDI)
iwr https://github.com/limehawk/prinstall/releases/latest/download/prinstall.exe -OutFile prinstall.exe

# SDI-enabled build
iwr https://github.com/limehawk/prinstall/releases/latest/download/prinstall-sdi.exe -OutFile prinstall.exe
```

**From source:**

```bash
cargo install --git https://github.com/limehawk/prinstall                    # default
cargo install --git https://github.com/limehawk/prinstall --features sdi     # with SDI
```

## Quick start

```powershell
prinstall                             # launch the interactive TUI
prinstall scan                        # scan your subnet for printers
prinstall id 192.168.1.50             # identify one via SNMP + IPP
prinstall add 192.168.1.50            # install it
prinstall remove 192.168.1.50         # rip it out cleanly
```

Every command takes `--json` for scripting and `--verbose` for the raw PS audit trail.

## The driver pipeline

`prinstall add` walks the pipeline in priority order and only escalates when the previous tier comes up empty:

```
  TIER 1   Local driver store         Reuse what's already installed
  TIER 2   Manufacturer download      HP, Xerox, Kyocera — stable direct URLs
  TIER 3   Update Catalog + HWID      Search by IPP CID, download CAB, parse INF, match HWID
  TIER 4   SDI Origin (opt-in)        Community driver packs — Brother, Canon, Epson, Ricoh
  TIER 5   IPP Class Driver           The always-works safety net (Windows 8+)
```

Tier 3 is the default workhorse — it scrapes the Microsoft Update Catalog, downloads a candidate CAB, parses the INF, and confirms a `1284_CID_*` hardware-ID match **before** installing. No gambling on model names.

Tier 4 (SDI) is compiled in only with `cargo build --features sdi`. It provides vendor-specific drivers for brands the Update Catalog doesn't reliably carry, using Snappy Driver Installer Origin's community-maintained driver packs.

### SDI Origin integration (opt-in)

The default `prinstall.exe` release binary does **not** include SDI support. The SDI tier uses third-party community-maintained driver packs from [Snappy Driver Installer Origin](https://www.glenn.delahoy.com/snappy-driver-installer-origin/) whose supply chain we haven't independently verified yet. It's behind a compile-time feature flag so you can make a conscious choice.

**To build with SDI enabled:**

```bash
cargo build --release --features sdi
```

**What SDI adds when enabled:**

- `prinstall sdi` subcommand — `status`, `refresh`, `list`, `prefetch`, `clean`
- `--sdi-fetch` flag on `prinstall add` — allows auto-pick to trigger a first-run pack download (~1.5 GB)
- `--no-sdi` flag on `prinstall add` — skip the SDI tier for a single run
- Tier 4 in the driver pipeline — searches cached SDI index files by hardware ID, extracts the matching driver from the pack, stages the INF, and installs

**How it works:**

1. Run `prinstall sdi refresh` to download the SDI index files (~1 MB) from the configured mirror
2. Run `prinstall sdi prefetch` to download the printer driver pack (~1.5 GB one-time download)
3. Now `prinstall add <ip>` will search the SDI index when Tiers 1–3 come up empty. If the pack is cached, extraction + install takes seconds.

The SDI pack is cached at `C:\ProgramData\prinstall\sdi\` and only needs to be downloaded once. Subsequent installs from the same pack are instant.

**Why opt-in?** The SDI driver packs are built by a third-party community (SamLab). While the drivers inside are real vendor binaries with valid INF files, the pack build process isn't independently auditable. We plan to add Authenticode signature verification for `.cat` files before promoting SDI to a default feature. Until then, it's available for techs who want the coverage and understand the tradeoff.

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

*Built in Rust  ·  8 MB binary  ·  Designed for techs who just want the printer to work*

</div>

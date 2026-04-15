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

`prinstall` collapses the whole workflow into a single ~9 MB binary. Scan a subnet, add a printer, remove it cleanly, audit what's installed. Works over SSH, RMM remote shells, or any local terminal with a clean CLI (`--json` on every command).

## Features

- **Multi-method discovery** — TCP port probe, IPP, SNMP, mDNS/Bonjour, and `Get-Printer` in one parallel pipeline
- **Deterministic driver resolution** — scrapes the Microsoft Update Catalog, downloads the CAB, parses the INF, and matches the synthesized HWID before installing
- **Structured output** — every `prinstall add` shows a phased report (Discovery → Driver Resolution → Install → Summary) with tier status and timing. `--verbose` adds raw PS commands for debugging
- **Network + USB** — one binary, both install paths, idempotent
- **Clean remove** — queue, driver, and port teardown with spooler-lag retries
- **Readable errors** — PowerShell stderr parsed and HRESULT-decoded before you see it
- **Scriptable CLI** — `--json` on every command for RMM automation, semantic coloring that respects `NO_COLOR`
- **SDI driver packs** — Snappy Driver Installer Origin integration for Brother, Canon, Epson, Ricoh, and other vendors the Update Catalog doesn't carry. Every pack's `.cat` Authenticode signature is verified before install

## Install

Each release ships two binaries:

| Binary | Size | SDI | Use case |
|---|---|---|---|
| `prinstall.exe` | ~9 MB | Yes | Default — Tiers 1–4 + IPP fallback, .cat signature verification on SDI |
| `prinstall-nosdi.exe` | ~8 MB | No | Lean — Tiers 1–3 + IPP fallback, zero SDI code |

**Windows (PowerShell one-liner):**

```powershell
# Default (includes SDI with signature verification)
iwr https://github.com/limehawk/prinstall/releases/latest/download/prinstall.exe -OutFile prinstall.exe

# Lean (no SDI — Tiers 1–3 only)
iwr https://github.com/limehawk/prinstall/releases/latest/download/prinstall-nosdi.exe -OutFile prinstall.exe
```

**From source:**

```bash
cargo install --git https://github.com/limehawk/prinstall                               # default (includes SDI)
cargo install --git https://github.com/limehawk/prinstall --no-default-features         # lean (no SDI)
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
  TIER 4   SDI Origin (verified)      Community driver packs — Brother, Canon, Epson, Ricoh
  TIER 5   IPP Class Driver           The always-works safety net (Windows 8+)
```

Tier 3 is the default workhorse — it scrapes the Microsoft Update Catalog, downloads a candidate CAB, parses the INF, and confirms a `1284_CID_*` hardware-ID match **before** installing. No gambling on model names.

Tier 4 (SDI) runs by default. Every SDI driverpack has its `.cat` Authenticode signature verified against Microsoft's certificate chain before install — unsigned or tampered packs are skipped and the pipeline falls through to Tier 5. Use `--no-default-features` at build time to drop SDI entirely (see `prinstall-nosdi.exe`).

### SDI Origin integration

Tier 4 of the driver pipeline uses [Snappy Driver Installer Origin](https://www.glenn.delahoy.com/snappy-driver-installer-origin/) driver packs for vendors the Update Catalog doesn't reliably carry — Brother, Canon, Epson, Ricoh, and others.

**Why we include it by default:**

SDIO packs contain real vendor binaries with valid Microsoft-chained Authenticode signatures. Prinstall verifies every `.cat` catalog file in a pack before trusting it — if any signature is missing, mismatched, or not chain-trusted, the pack is skipped and the pipeline falls through to Tier 5 (IPP Class Driver). This means unsigned or tampered packs can't install, whether an attacker slipped them into a mirror or the pack author forgot to sign them.

**What SDI adds:**

- `prinstall sdi` subcommand — `status`, `refresh`, `list`, `prefetch`, `clean`, `verify`
- `--sdi-fetch` flag on `prinstall add` — allows auto-pick to trigger a first-run pack download (~1.5 GB)
- `--no-sdi` flag on `prinstall add` — skip the SDI tier for a single run
- `prinstall sdi verify` — manually inspect every cached pack's signature chain

**How it works:**

1. Run `prinstall sdi refresh` to download the SDI index files (~1 MB) from the configured mirror
2. Run `prinstall sdi prefetch` to cache the printer driver pack (~1.5 GB one-time download)
3. `prinstall add <ip>` searches the SDI index when Tiers 1–3 come up empty, verifies the pack's `.cat` signatures, and installs only if they pass

The SDI pack is cached at `C:\ProgramData\prinstall\sdi\` and only needs to be downloaded once.

**Opting out:**

If you want zero SDI code in your binary — some regulated environments prefer a reviewed-and-pinned binary with no third-party pack support at all — use the lean `prinstall-nosdi.exe` release, or build with `cargo build --release --no-default-features`. Everything above Tier 4 still works.

**The supply chain note:**

[SDIO](https://www.glenn.delahoy.com/snappy-driver-installer-origin/) is maintained by Glenn Delahoy. Printer packs are built by a separate group ([SamLab](https://samlab.ws/), a Russian-language driver pack community active since 2013) and distributed through Glenn's torrents alongside his own packs. The drivers inside are real vendor binaries, but the pack build process itself isn't independently auditable — which is exactly why we verify each pack's `.cat` signature against Microsoft's certificate chain before install. If the content is untampered vendor code, it verifies; if it isn't, prinstall refuses to install it.

## Docs

- **Website** — [prinstall.limehawk.io](https://prinstall.limehawk.io)
- **Wiki** — [github.com/limehawk/prinstall/wiki](https://github.com/limehawk/prinstall/wiki)
- **Getting started** — [wiki/Getting-Started](https://github.com/limehawk/prinstall/wiki/Getting-Started)
- **Command reference** — [wiki/CLI-Reference](https://github.com/limehawk/prinstall/wiki/CLI-Reference)
- **Architecture** — [wiki/Architecture](https://github.com/limehawk/prinstall/wiki/Architecture)
- **Roadmap** — [wiki/Roadmap](https://github.com/limehawk/prinstall/wiki/Roadmap)

Data, history, and driver staging live under `C:\ProgramData\prinstall\`.

## Contributing

**Your printer didn't match? That's a contribution waiting to happen.**

Two tracks, wildly different bars to entry:

- **Driver data (no Rust required).** [`data/drivers.toml`](data/drivers.toml) and [`data/known_matches.toml`](data/known_matches.toml) are the embedded driver knowledge. If you just installed a printer and prinstall picked the wrong driver — open a [driver issue](../../issues/new?template=new_driver.yml) or submit a 3-line PR against those TOMLs. Full walkthrough: [`docs/contributing-drivers.md`](docs/contributing-drivers.md).
- **Code (Rust).** See [`CONTRIBUTING.md`](CONTRIBUTING.md) for setup, testing, and style. The `PsExecutor` trait + `MockExecutor` pattern means the whole test suite runs on Linux without a Windows VM.

Bugs, feature requests, and driver-match debugging all have [issue templates](../../issues/new/choose) — the raw `--verbose` output you paste is what makes fixes land fast.

## License

MIT. See [LICENSE](LICENSE). Built by [limehawk](https://limehawk.io).

---

<div align="center">

*Built in Rust  ·  ~9 MB binary  ·  Designed for techs who just want the printer to work*

</div>

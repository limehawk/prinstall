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
prinstall list                        # show what's installed
```

Every command takes `--json` for scripting and `--verbose` for the raw PS audit trail.
Each subcommand has its own detailed `--help`, e.g. `prinstall add --help`.

## First-run setup

After downloading the exe, run `setup install` once. It copies the binary to
`C:\ProgramData\prinstall\`, adds that directory to the Machine PATH, and opens
UDP 5353 on Windows Firewall so `scan --method mdns` works:

```powershell
.\prinstall.exe setup install         # admin required, idempotent
prinstall setup uninstall             # reverses all three
```

Skip this if you're invoking the exe directly from an RMM payload or one-shot
script.

## Commands

Run any command with `--help` to see every flag and a usage example. Global flags
(`--json`, `--verbose`, `--community`, `--force`, `--subnet`) work on every
subcommand.

### Global flags

| Flag | What it does |
|---|---|
| `--json` | Emit machine-readable JSON instead of human output. Works on every command — use it from RMM/PowerShell pipelines |
| `--verbose` / `-v` | Stream the raw PowerShell commands and the structured Discovery → Resolution → Install → Summary report |
| `--community <str>` | SNMP community string (default: `public`) |
| `--force` | Allow scans larger than /24, skip the scanner fail-fast guard on `add`, cascade dependent queues on `driver remove` |
| `--subnet <cidr>` | Override the auto-detected subnet for TUI launch |

### `prinstall` (no args) — Interactive TUI

Launches the ratatui two-panel TUI: printer list on the left, detail pane on the
right. Vim-style keybindings, scan progress, in-flight driver match results.

| Key | Action |
|---|---|
| `j` / `k` | Move up/down in lists |
| `h` / `l` | Move focus between panels |
| `Tab` | Cycle panel focus |
| `g` / `G` | Jump to top/bottom |
| `Enter` | Select / install driver |
| `Esc` | Back / close overlay |
| `s` | Rescan |
| `?` | Toggle help overlay |
| `q` | Quit |

### `prinstall scan [SUBNET]` — Network discovery

Probes every IP on the subnet via TCP port checks, IPP, and SNMP in parallel,
PLUS an mDNS multicast browse. Discovered printers show IP, model, and status.
Auto-detects the subnet from the local NIC when no argument is passed. Subnets
larger than `/24` require `--force`.

```powershell
prinstall scan                        # auto-detect subnet, all methods
prinstall scan 192.168.1.0/24         # scan a specific subnet
prinstall scan --method snmp          # SNMP only
prinstall scan --method port          # TCP 9100 only
prinstall scan --method mdns          # mDNS multicast browse (no subnet needed)
prinstall scan --timeout 200          # 200 ms per-host timeout
prinstall scan --network-only         # skip USB enumeration
prinstall scan --usb-only             # skip network scan
prinstall scan 10.0.0.0/24 --community private
```

| Flag | Default | What it does |
|---|---|---|
| `--method <all\|snmp\|port\|mdns>` | `all` | Which discovery method(s) to run |
| `--timeout <ms>` | `500` | Per-host timeout |
| `--network-only` | off | Skip USB enumeration |
| `--usb-only` | off | Skip network scan, USB only |

**Troubleshooting empty results:** SNMP often disabled by default — enable in
the printer web UI, or try `--community private`. UDP 161 blocked by firewall.
mDNS multicast blocked by the NIC or router.

### `prinstall id <IP>` — Identify a single printer

Sends SNMP GET requests to one IP on UDP 161. Returns device description,
serial number, and status. Times out after 2 seconds.

```powershell
prinstall id 192.168.1.100
prinstall id 10.0.0.50 --community private
prinstall id 192.168.1.100 --json
```

### `prinstall add <TARGET>` — Install a printer

For network printers: identifies via SNMP, picks the best-matched driver, stages
it if needed, then runs `Add-PrinterPort` → `Add-PrinterDriver` → `Add-Printer`.
If the primary install fails and port 631 is open, falls back to Microsoft's IPP
Class Driver with a clearly-marked warning.

For USB printers: pass `--usb` and the existing queue name as the target. The
command verifies the queue exists, finds the best driver, stages it, and swaps
it in via `Set-Printer`.

```powershell
# Network
prinstall add 192.168.1.100
prinstall add 192.168.1.100 --driver "HP Universal Print Driver PCL6"
prinstall add 192.168.1.100 --name "Front Desk Printer"
prinstall add 192.168.1.100 --model "HP LaserJet Pro M404dn"

# USB (target is the queue name, not an IP)
prinstall add "Brother MFC-L2750DW" --usb
prinstall add "HP OfficeJet Pro" --usb --driver "HP Universal PCL6"
```

| Flag | What it does |
|---|---|
| `--driver <name>` | Use this exact driver name instead of auto-matching |
| `--name <str>` | Display name for the printer queue (network mode only) |
| `--model <str>` | Manually specify the model (bypass SNMP / override USB queue name) |
| `--usb` | USB mode: target is a queue name; skip port creation; swap via `Set-Printer` |
| `--no-sdi` | Skip the SDI driver tier for this run (default build only) |
| `--no-catalog` | Skip the Microsoft Update Catalog tier for this run |
| `--sdi-fetch` | Allow auto-pick to trigger a first-run SDI pack download (~1.5 GB) |
| `--no-verify` | Skip Authenticode `.cat` signature verification (use only for vendor packs that legitimately ship without a catalog) |

Requires admin. Ports/drivers already on the box are reused, not duplicated.

### `prinstall remove <TARGET>` — Remove a printer

Removes the printer queue, the driver (if no other queue uses it), and the
TCP/IP port (if orphaned). `target` is either the IP (resolved via the
`IP_<ip>` port name convention) or the queue name directly.

```powershell
prinstall remove 192.168.1.100              # full cleanup
prinstall remove "HP LaserJet Pro"          # by queue name
prinstall remove 192.168.1.100 --keep-driver
prinstall remove 192.168.1.100 --keep-port --keep-driver
```

| Flag | What it does |
|---|---|
| `--keep-driver` | Leave the driver in the store even if no other queue uses it |
| `--keep-port` | Leave the TCP/IP port even if no other queue uses it |

Driver/port cleanup failures are non-fatal warnings. If the printer doesn't
exist, the command succeeds (idempotent). Requires admin.

### `prinstall list` — List installed printers

Enumerates every printer queue Windows knows about via `Get-Printer`. Shows
queue name, driver, port, IP (for network queues), and source (USB / network /
installed). No admin required.

```powershell
prinstall list
prinstall list --json                 # JSON for scripting
prinstall list --verbose              # raw Get-Printer output
```

### `prinstall driver` — Manage drivers in the driver store

Stage, list, remove, or inspect drivers independent of any printer queue.
Useful for pre-loading drivers before a PnP event fires.

```powershell
prinstall driver add C:\Drivers\HP_LaserJet_1320          # folder of INFs
prinstall driver add C:\Drivers\brother.inf               # single INF
prinstall driver add "HP LaserJet 1320"                   # model string — auto-stages curated match
prinstall driver add "HP Universal Print Driver PS"       # exact universal name — auto-picks
prinstall driver add "HP LaserJet" --driver "HP Universal Print Driver PCL6"

prinstall driver remove "HP Universal Print Driver PCL6"  # exact name
prinstall driver remove "hp 1320"                         # fuzzy
prinstall driver remove "Brother MFC" --force             # cascade: remove queues first

prinstall driver list                                     # all staged drivers (no admin)
prinstall driver list --json

prinstall driver show 192.168.1.100                       # matched drivers for an IP
prinstall driver show 192.168.1.100 --model "HP LaserJet Pro MFP M428fdw"
```

**Sub-actions:**

| Sub-action | What it does | Admin |
|---|---|---|
| `add <PATH\|model>` | Stage from an INF, folder of INFs, or auto-resolved model string. `--driver` picks among multiple candidates. `--no-verify` skips Authenticode | Yes |
| `remove <name\|fuzzy>` | Remove from the driver store. Refuses if bound to a queue; `--force` cascades through dependent queues. System drivers (IPP Class etc.) are protected | Yes |
| `list` | Pretty-print every driver from `Get-PrinterDriver` with manufacturer, version, and date | No |
| `show <IP>` | Show ranked matched drivers for a printer (★ exact / ● fuzzy / ○ low). `--model` bypasses SNMP | No |

The top-level `prinstall drivers <IP>` is a deprecated alias for `driver show <IP>`.

### `prinstall sdi` — SDI driverpack cache (default build only)

Manages the [Snappy Driver Installer Origin](https://www.glenn.delahoy.com/snappy-driver-installer-origin/)
cache used by Tier 4 of the driver pipeline. Not present in `prinstall-nosdi.exe`.

```powershell
prinstall sdi status                  # cache contents, total size, mirror URL
prinstall sdi refresh                 # pull latest index files (~1 MB)
prinstall sdi list                    # list cached indexes + packs
prinstall sdi prefetch                # download all driver packs (~1.5 GB, one time)
prinstall sdi clean                   # LRU-evict past the size budget
prinstall sdi verify                  # Authenticode-verify every cached pack's .cat
```

Cache lives at `C:\ProgramData\prinstall\sdi\`. Run `refresh` then `prefetch`
on a freshly-deployed box to enable Tier 4 for everyone.

### `prinstall setup` — Self-bootstrap

```powershell
prinstall setup install               # → C:\ProgramData\prinstall\, +PATH, +firewall rule
prinstall setup install --dir C:\Tools\prinstall
prinstall setup uninstall             # reverses all three
prinstall setup uninstall --dir C:\Tools\prinstall
```

Run from a copy of the exe **outside** the install dir so the running-exe file
lock doesn't block uninstall. Requires admin.

### `prinstall version`

Alias for `prinstall --version`. Matches the muscle memory from `git`, `cargo`,
`npm`.

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

### Where to go

- 🐛 [**Issues**](../../issues/new/choose) — bugs, driver-match problems, feature requests. Structured templates capture the `--verbose` output that makes fixes land fast.
- 💬 [**Discussions**](../../discussions) — Q&A, show-and-tell, early-stage ideas. Good for "how do I do X" or "I used prinstall to deploy 40 printers across 8 clients in a morning" stories.

## License

MIT. See [LICENSE](LICENSE). Built by [limehawk](https://limehawk.io).

---

<div align="center">

*Built in Rust  ·  ~9 MB binary  ·  Designed for techs who just want the printer to work*

</div>

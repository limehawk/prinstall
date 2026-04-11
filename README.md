```
                                                                                   
▄▄▄▄▄▄▄   ▄▄▄▄▄▄▄   ▄▄▄▄▄ ▄▄▄    ▄▄▄  ▄▄▄▄▄▄▄ ▄▄▄▄▄▄▄▄▄   ▄▄▄▄   ▄▄▄      ▄▄▄      
███▀▀███▄ ███▀▀███▄  ███  ████▄  ███ █████▀▀▀ ▀▀▀███▀▀▀ ▄██▀▀██▄ ███      ███      
███▄▄███▀ ███▄▄███▀  ███  ███▀██▄███  ▀████▄     ███    ███  ███ ███      ███      
███▀▀▀▀   ███▀▀██▄   ███  ███  ▀████    ▀████    ███    ███▀▀███ ███      ███      
███       ███  ▀███ ▄███▄ ███    ███ ███████▀    ███    ███  ███ ████████ ████████ 
                                                                                   
                                                                                   
```

### Discover. Match. Add. Remove.

**A Rust CLI and TUI for nuking printer setup pain on Windows.**

[![Release](https://img.shields.io/github/v/release/limehawk/prinstall?style=flat-square&color=orange&label=release)](https://github.com/limehawk/prinstall/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](#license)
[![Platform](https://img.shields.io/badge/platform-windows-0078D4?style=flat-square&logo=windows)](https://github.com/limehawk/prinstall/releases)
[![Built with Rust](https://img.shields.io/badge/built_with-rust-CE422B?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Build](https://img.shields.io/github/actions/workflow/status/limehawk/prinstall/release.yml?style=flat-square&label=build)](https://github.com/limehawk/prinstall/actions)

---

## Why

MSP technicians burn hours on printer installs. Find the IP, hunt the driver, wrestle `Add-Printer`, paste the wrong universal PCL6 again, repeat on the next site visit. Then the printer breaks a week later and you have to rip it out and redo everything.

`prinstall` collapses the whole workflow into a single 9 MB binary. Scan a subnet, add a printer, remove it cleanly, audit what's installed. Works over SSH or RMM remote shells with a clean CLI (and a `--json` flag on every command), or drops you into an interactive TUI when launched from a real terminal.

## Features

```
 ▸ Multi-method discovery    TCP port probe  ·  IPP  ·  SNMP  ·  Get-Printer
 ▸ Deterministic driver      Microsoft Update Catalog scraped in pure Rust,
   resolution                 driver package downloaded, INF parsed, HWID
                              matched — no gambling, no model-name guessing
 ▸ Four-tier matching        Local store → manufacturer → catalog → IPP fallback
                              with a visible audit breadcrumb on whichever
                              tier actually landed the driver
 ▸ Network + USB printers    Single binary handles both install paths
 ▸ Clean remove              Queue → driver → port cleanup with spooler-lag
                              retry loop and -RemoveFromDriverStore to take the
                              underlying oem<N>.inf package with it
 ▸ Clean error output        PowerShell stderr is parsed + HRESULT-decoded so
                              you don't drown in CategoryInfo/FullyQualifiedErrorId
 ▸ Lazy-style TUI            Two-panel, vim keybindings, ratatui widgets
 ▸ Scriptable CLI            --json on every command for RMM automation
 ▸ Terminal colors           Semantic coloring via crossterm, honors NO_COLOR
                              and auto-disables when stdout isn't a TTY
 ▸ Single 12 MB binary       Embedded data, UAC manifest, static CRT
 ▸ Idempotent                Existing ports, drivers, and queues are reused
```

### Discovery, the multi-method way

Most printers have SNMP disabled out of the box, so SNMP-only scanners miss the majority of a real network. `prinstall` runs a layered pipeline:

| Phase | Method | Purpose |
|-------|--------|---------|
| 1 | TCP port probe (9100 / 631 / 515) | Find anything listening — fast parallel sweep |
| 2 | IPP (port 631) | Pull model + IEEE 1284 device ID from `printer-make-and-model` and `printer-device-id` |
| 3 | SNMP v2c | Enrich with serial, status, full model string |
| 4 | `Get-Printer` | Include locally installed USB / network queues |

Results are merged and deduplicated automatically.

### Driver matching

Matching runs four tiers against the identified model string:

```
  ★ exact    Curated match from the known-matches database (score 1000)
  ● fuzzy    Scored match combining model-number prefix, token overlap,
             and subsequence similarity (threshold 250/1000)
  ○ universal Manufacturer's generic driver, always shown as a fallback
```

Scoring is deterministic and ranks by a numeric 0-1000 score, not just a coarse "low/medium/high" tier. Wrong-family drivers (e.g. HP Color LaserJet matching a Brother mono printer) are filtered below the threshold.

### Deterministic driver resolution via the Microsoft Update Catalog

When the primary install fails because the matched vendor driver isn't in the local store and no manufacturer URL is available, `prinstall add` doesn't give up — it runs the **catalog resolver**: a pure-Rust port of the MSCatalogLTS PowerShell module that scrapes `catalog.update.microsoft.com` directly, searches by the IPP **`CID:`** field (not the fuzzy model name), downloads candidate driver packages from Microsoft's CDN, expands them with `expand.exe`, parses the INF, and confirms a **deterministic hardware-ID match** before staging.

The HWID match is the punch line. Your printer advertises `CID:Brother Laser Type1` over IPP. The resolver synthesizes the canonical PnP hardware ID — `1284_CID_BROTHER_LASER_TYPE1` — and looks for it verbatim in the downloaded INF's `[Models]` section. If Windows native PnP would pick this driver, we pick this driver. If the INF doesn't list the HWID, we reject the package and move on to the next candidate. No guessing, no "probably-the-right-one" heuristics.

If the catalog resolver also fails (rare — usually when a cheap printer omits the `CID:` field), `prinstall add` still has a last-resort **Microsoft IPP Class Driver fallback** for any IPP Everywhere printer (Windows 8+). That path always attaches a visible `WARNING:` line to the result for audit trails.

See [How prinstall picks a driver](#how-prinstall-picks-a-driver) below for the full four-tier pipeline.

## Install

Grab the latest Windows binary from [Releases](https://github.com/limehawk/prinstall/releases) and drop `prinstall.exe` anywhere on `PATH`.

Or build from source:

```bash
cargo install --git https://github.com/limehawk/prinstall
```

Cross-compiling from Linux works via `messense/cargo-xwin`:

```bash
docker run --rm -v "$PWD":/io -w /io messense/cargo-xwin:latest \
  bash -c 'ln -sf /usr/bin/llvm-mt /usr/local/bin/mt.exe && \
           cargo xwin build --release --target x86_64-pc-windows-msvc'
```

CI builds the Windows binary on tag push via GitHub Actions `windows-latest` runner.

## Quick Start

```
prinstall                              Launch interactive TUI
prinstall scan                         Scan local subnet (auto-detected)
prinstall scan 192.168.1.0/24          Scan specific subnet
prinstall id 192.168.1.100             Identify a printer via SNMP
prinstall drivers 192.168.1.100        Show matched + universal drivers
prinstall add 192.168.1.100            Install a network printer
prinstall add "HP OfficeJet" --usb     Swap driver on an existing USB printer
prinstall remove 192.168.1.100         Remove printer + orphaned driver + port
prinstall list                         List printers Windows already knows
```

Global flags: `--json`, `--verbose`, `--community <str>`, `--force`, `--subnet <cidr>`.

## Usage

### Scan a subnet

```console
$ prinstall scan 192.168.1.0/24

  IP              MODEL                                STATUS
  192.168.1.12    HP LaserJet Pro MFP M428fdw          Ready
  192.168.1.47    Brother MFC-L2750DW series           Ready
  192.168.1.88    RICOH MP C3004                       Ready
  192.168.1.104   Canon imageRUNNER ADVANCE C5535      Warming up

  Scanned 254 hosts  ·  4 printers found  ·  1.8s
```

Choose a method with `--method all|snmp|port`, tune with `--timeout <ms>`, override the community with `--community <str>`.

### Find drivers

```console
$ prinstall drivers 192.168.1.47

  Printer: Brother MFC-L2750DW series
  IPP Device ID: MFG:Brother;CMD:PJL,PCL,PCLXL,URF;MDL:MFC-L2750DW series;CLS:PRINTER;...

  ── Matched Drivers ──────────────────────────────────────────
    #1  Brother MFC-L2750DW PCL-6                      ● fuzzy    78%  [Local Store]

  ── Universal Drivers ────────────────────────────────────────
    #2  Brother Universal Printer                      [Manufacturer]
```

The IEEE 1284 device ID row shows the string Windows Update matches drivers against — useful when manually looking up a driver at `catalog.update.microsoft.com`.

### Add a network printer

When the vendor driver is already in the local store, install is instant:

```console
$ prinstall add 192.168.1.12 --verbose

  [add] SNMP → HP LaserJet Pro MFP M428fdw
  [add] Auto-selected driver: HP LaserJet Pro MFP M428f PCL-6
  [PS] Add-PrinterPort -Name 'IP_192.168.1.12' -PrinterHostAddress '192.168.1.12'
  [PS] Add-PrinterDriver -Name 'HP LaserJet Pro MFP M428f PCL-6'
  [PS] Add-Printer -Name 'HP LaserJet Pro MFP M428fdw' -DriverName '...' -PortName 'IP_192.168.1.12'

  Printer installed successfully!
    Name:   HP LaserJet Pro MFP M428fdw
    Driver: HP LaserJet Pro MFP M428f PCL-6
    Port:   IP_192.168.1.12
```

When the vendor driver isn't staged locally and the manufacturer doesn't publish a direct download URL, the catalog resolver kicks in and fetches the exact driver from Microsoft's CDN via deterministic HWID match:

```console
$ prinstall add 10.10.20.16 --verbose

  [add] IPP device ID: MFG:Brother;MDL:MFC-L2750DW series;CID:Brother Laser Type1;...
  [add] Auto-selected driver: Brother Universal Printer
  [add] Download failed: No download URL available for 'Brother Universal Printer'.
  [PS] Add-PrinterDriver -Name 'Brother Universal Printer'
  [PS stderr] HRESULT 0x80070705: The specified driver does not exist in the driver store.
  [add] Primary install failed. Trying catalog resolver with device ID...
  [resolver] Searching catalog by CID: 'Brother Laser Type1'
  [resolver] Catalog returned 5 result(s), scanning top 5
  [resolver] HWID candidates: 1284_CID_BROTHER_LASER_TYPE1, BROTHER_LASER_TYPE1, ...
  [resolver] #1: Brother - Printer - 10.0.17119.1  (4/21/2009)
  [resolver]   GET https://catalog.s.download.windowsupdate.com/.../prnbrcl1.cab
  [resolver]   expand → C:\Users\tech\AppData\Roaming\prinstall\staging\catalog\...
  [resolver] ★ MATCH: prnbrcl1.inf → Brother Laser Type1 Class Driver (1284_CID_BROTHER_LASER_TYPE1)
  [add] Catalog resolver matched 'Brother Laser Type1 Class Driver' — staging INF and retrying install.
  [PS] pnputil /add-driver '...\prnbrcl1.inf' /install
  [PS] Add-Printer -Name 'Brother MFC-L2750DW series' -DriverName 'Brother Laser Type1 Class Driver' ...

  Printer installed successfully!
    Name:   Brother MFC-L2750DW series
    Driver: Brother Laser Type1 Class Driver
    Port:   IP_10.10.20.16

    WARNING: Installed via Microsoft Update Catalog: 'Brother Laser Type1 Class Driver'
             from 'Brother - Printer - 10.0.17119.1' (DriverVer 04/22/2009,10.0.17119.1).
             Matched HWID: 1284_CID_BROTHER_LASER_TYPE1.
```

The `WARNING:` line on a catalog-resolver install isn't actually a warning — it's a breadcrumb for the audit trail, naming the exact catalog package, driver version, and matched HWID so you can trace every install back to its source.

### Add a USB printer

For a USB printer that Windows already auto-created a queue for via PnP, pass `--usb` with the queue name as the target. `prinstall` verifies the queue exists, finds the best vendor driver, stages it if needed, and swaps it in via `Set-Printer`:

```console
$ prinstall add "Brother MFC-L2750DW" --usb --verbose

  [add] USB mode — target queue: 'Brother MFC-L2750DW'
  [add] Auto-selected driver: Brother MFC-L2750DW PCL-6
  [add] Swapping driver on 'Brother MFC-L2750DW' → 'Brother MFC-L2750DW PCL-6'

  Printer installed successfully!
    Name:   Brother MFC-L2750DW
    Driver: Brother MFC-L2750DW PCL-6
```

### Remove a printer

```console
$ prinstall remove 10.10.20.16 --verbose

  [remove] Looking up printer by port 'IP_10.10.20.16'
  [remove] Resolved target '10.10.20.16' → 'Brother MFC-L2750DW series'
  [remove] Printer uses driver 'Brother Laser Type1 Class Driver' on port 'IP_10.10.20.16'
  [PS] Remove-Printer -Name 'Brother MFC-L2750DW series' -Confirm:$false
  [remove] Waiting 500ms for spooler to release references...
  [PS] Remove-PrinterDriver -Name 'Brother Laser Type1 Class Driver' -RemoveFromDriverStore -Confirm:$false
  [remove] Removed driver 'Brother Laser Type1 Class Driver' (including driver store package)
  [PS] Remove-PrinterPort -Name 'IP_10.10.20.16' -Confirm:$false
  [remove] Removed port 'IP_10.10.20.16'

  Removed printer: Brother MFC-L2750DW series
    · Port also removed (no other printers were using it)
    · Driver also removed from driver store
```

Remove is three-step with orphan detection and a **spooler-lag retry loop**. After `Remove-Printer` returns, the Windows spooler keeps internal reference counts on the driver and port for 1-3 seconds — long enough that a single-shot cleanup fails with a misleading "in use" error even though `Get-Printer` reports zero references. `prinstall` waits 500ms for the spooler to settle, then retries each removal with an escalating backoff schedule ([0, 1s, 2s, 2.5s]) so cleanup succeeds on slow and fast systems alike.

Driver cleanup uses `Remove-PrinterDriver -RemoveFromDriverStore`, which also kills the underlying `oem<N>.inf` package in the Windows driver store. That's important when a class driver INF registers multiple siblings (e.g. `prnbrcl1.inf` ships 6+ Brother drivers — Laser Type1, Laser Leg, Color Leg, Color Type3, IJ Leg) in a single `pnputil /add-driver` call. Without the store flag, removing the named driver leaves its siblings orphaned; with it, the whole package goes.

System drivers (`Microsoft IPP Class Driver`, `Universal Print Class Driver`, `Microsoft Print To PDF`, etc.) are skipped because they're not removable. Non-TCP/IP ports (`USB001`, `LPT1`, `COM1`, `PORTPROMPT:`, `WSD-*`) are whitelisted out — `prinstall` only touches ports it created.

Flags: `--keep-driver` and `--keep-port` disable the respective cleanup step.

### Automation (JSON mode)

```bash
prinstall scan 192.168.1.0/24 --json \
  | jq '.[] | select(.model | test("HP"; "i")) | .ip'
```

Every subcommand speaks `--json`. Pipe it through `jq`, feed it to PowerShell, chain it from your RMM runner. JSON output never includes terminal color codes.

## How prinstall picks a driver

When you run `prinstall add <ip>`, the resolver walks four tiers in priority order. Each tier is cheaper or more reliable than the next, so the pipeline only escalates when the previous tier comes up empty.

```
  ┌──────────────────────────────────────────────────────────────┐
  │  prinstall add 192.168.1.47                                  │
  └─────────────────────────────┬────────────────────────────────┘
                                ▼
  ┌──────────────────────────────────────────────────────────────┐
  │  Tier 1   Local driver store                  (no network)   │
  │           Get-PrinterDriver  →  fuzzy score ≥ 250            │
  └────────────┬──────────────────────────────────┬──────────────┘
               │ hit                              │ miss
               ▼                                  ▼
        install + done           ┌──────────────────────────────────┐
                                 │  Tier 2   Manufacturer download  │
                                 │           drivers.toml URL       │
                                 │           pnputil /add-driver    │
                                 └──────┬──────────────────────┬────┘
                                        │ hit                  │ miss / empty URL
                                        ▼                      ▼
                                 install + done   ┌──────────────────────────┐
                                                  │  Tier 3   MS Update      │
                                                  │           Catalog + INF  │
                                                  │           HWID match     │
                                                  │           by IPP CID     │
                                                  └────┬─────────────────┬───┘
                                                       │ hit             │ miss / no CID
                                                       ▼                 ▼
                                                install + done   ┌─────────────────┐
                                                                 │  Tier 4   IPP   │
                                                                 │   Class Driver  │
                                                                 │   (port 631)    │
                                                                 └────────┬────────┘
                                                                          ▼
                                                                  install + WARNING
```

**Why this order:** local store first because it's instant and has zero side effects — if the driver's already on the box, we use it. Manufacturer download next because it's the cleanest result when a vendor publishes a stable URL. Microsoft Update Catalog third because it's authoritative but requires a download and an INF parse. IPP Class Driver last as the always-works safety net so a tech is never left stranded.

### Tier 1 — Local driver store

- **Source:** `Get-PrinterDriver` enumerated from the local Windows driver store.
- **Matcher:** numeric scoring 0-1000 from `src/drivers/matcher.rs` — model-number prefix (up to 500 pts), token overlap (up to 300 pts), skim subsequence (up to 200 pts).
- **Threshold:** fuzzy score ≥ 250, or an exact hit on the curated `data/known_matches.toml` table (score 1000).
- **Success:** an already-installed driver gets reused — install runs with zero network calls.
- **When it pays off:** the tech installed this driver once before. The next install on a different queue is instant.

### Tier 2 — Manufacturer driver download

- **Source:** embedded `data/drivers.toml` (17 manufacturers, but only HP currently has working direct download URLs — Brother / Canon / Epson / Xerox have entries with empty URL fields pending real links).
- **Matcher:** same 0-1000 scoring as Tier 1, against the universal drivers listed in the manifest.
- **Success:** URL reachable → `.zip` or `.cab` downloaded into `paths::staging_dir()` → INFs extracted → `pnputil /add-driver` → `Add-Printer` installs.
- **Skip condition:** entry has an empty URL field — falls through silently to Tier 3.
- **Limitation:** depends entirely on manufacturers publishing stable direct download URLs, which most of them actively avoid.

### Tier 3 — Microsoft Update Catalog + INF HWID match

The deterministic path. This tier scrapes `catalog.update.microsoft.com` directly, downloads the candidate driver package, parses the INF, and confirms an exact hardware-ID match before installing.

- **Source:** `https://catalog.update.microsoft.com` scraped by `src/drivers/catalog.rs` — a Rust-native port of the MSCatalogLTS PowerShell module. No PS module runtime dependency.
- **Discovery input:** the IEEE 1284 IPP device ID surfaced by `src/discovery/ipp.rs`. Looks like:
  ```
  MFG:Brother;CMD:PJL,PCL,PCLXL,URF;MDL:MFC-L2750DW series;CLS:PRINTER;CID:Brother Laser Type1;
  ```
- **Query:** the `CID:` field, **verbatim**. Not the model name. CIDs are manufacturer-defined compatible IDs that group printers by driver family — `Brother Laser Type1`, `Canon PCL`, etc. Searching the catalog by CID narrows ~25 generic hits down to ~5 targeted package variants.
- **Match verification:** after download + CAB extraction, `src/drivers/inf.rs` parses the INF `[Models]` section and looks for the synthesized PnP hardware ID derived from the IPP CID:
  ```
  CID:"Brother Laser Type1"  →  1284_CID_BROTHER_LASER_TYPE1
  ```
  This is the exact HWID Windows would synthesize during native PnP enumeration, and it appears verbatim in the Brother Laser Type1 Class Driver INF. A match confirms the package supports this printer.
- **Tie-break:** when multiple INFs match, pick the package with the newest `DriverVer` from the `[Version]` section, falling back to the catalog's "Last Updated" date.
- **Success:** INF match found → `pnputil /add-driver` stages the INF → `Add-Printer` installs.
- **Skip condition:** the printer's IPP response doesn't include a `CID:` field (cheap network printers sometimes omit it) — falls through to Tier 4.

**This tier is deterministic, not a gamble.** The HWID match is exact. If the INF declares `1284_CID_BROTHER_LASER_TYPE1` and the printer advertises `CID:Brother Laser Type1`, it's the same driver Windows PnP would install — no guessing among lookalike catalog entries, no "this one's probably right" heuristics.

### Tier 4 — Microsoft IPP Class Driver fallback

- **Source:** Windows built-in `Microsoft IPP Class Driver` — the generic class driver that ships with Windows 8+ and handles basic IPP printing for any printer that speaks IPP Everywhere.
- **Trigger:** primary install failed (no driver resolved in Tiers 1-3, or the install pipeline errored out) **and** port 631 is reachable on the printer (verified by a 1.5s TCP probe).
- **Implementation:** `Add-Printer -Name "<model> (IPP)" -DriverName "Microsoft IPP Class Driver" -PortName IP_<ip>`.
- **Caveat:** basic printing works, but vendor-specific features (duplex modes, tray selection, finishing options) may not be available. This is the MSP safety net, not the target outcome.
- **Audit:** a `WARNING:` line is always attached to the result so post-install audits can identify generic-fallback installs and re-do them once a real driver becomes available.

### About that IPP device ID

Tier 3 hinges on the IEEE 1284 device ID, and we get it for free from the discovery pipeline — no extra user configuration. When `prinstall` probes a printer it opens an IPP `Get-Printer-Attributes` request on port 631 and reads the `printer-device-id` attribute. The full string is shown in `prinstall drivers <ip>` output and stashed on the `Printer` model so the resolver can pull the `CID:` field straight out without re-querying the printer.

If a printer doesn't speak IPP at all, Tier 3 is skipped and the pipeline lands on the IPP Class Driver fallback or, if 631 is also closed, returns a clean "no driver available" error.

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                           Interface                                │
│   cli.rs (clap)   │   tui/ (ratatui)   │   output.rs  (colorized)  │
└───────┬──────────────────┬───────────────────────┬─────────────────┘
        │                  │                       │
        ▼                  ▼                       ▼
┌───────────────┐   ┌──────────────┐       ┌───────────────────────┐
│   commands/   │◄──┤     core/    │       │       installer/      │
│               │   │              │       │                       │
│    add.rs     │   │  executor.rs │       │     powershell.rs     │
│   remove.rs   │   │  ps_error.rs │       │                       │
│   drivers.rs  │   └──────────────┘       │  create_port, install │
└───────┬───────┘                          │  _driver, add_printer │
        │                                  │  printer_exists, etc. │
        ▼                                  └───────────────────────┘
┌───────────────┐   ┌──────────────┐       ┌───────────────────────┐
│   Discovery   │   │    Drivers   │       │   Data + Persistence  │
│               │   │              │       │                       │
│   port_scan   │   │    matcher   │       │   paths.rs  config.rs │
│      ipp      │   │   manifest   │       │     history.rs        │
│      snmp     │   │known_matches │       │                       │
│     local     │   │  downloader  │       │  %APPDATA%\prinstall\ │
│    subnet     │   │ local_store  │       │                       │
└───────────────┘   └──────────────┘       └───────────────────────┘
```

Layered, testable, single binary.

**Design notes:**

- **`PsExecutor` trait** — every PowerShell call goes through a `&dyn PsExecutor`. `RealExecutor` shells out to `powershell.exe`; `MockExecutor` stubs responses for Linux unit tests. Lets us test the command logic on any platform without a Windows host.
- **`PrinterOpResult`** — uniform result type across all commands with a `detail: serde_json::Value` payload. `InstallDetail` and `RemoveDetail` are typed payloads serialized into the detail field. Works cleanly with `--json`.
- **`core::ps_error::clean`** — parses PowerShell stderr into single-line messages with HRESULT decoding. Drops the `CategoryInfo`, `FullyQualifiedErrorId`, line/column decorators that make raw PS errors unreadable.
- **`%APPDATA%\prinstall\`** — single data directory for history, config, driver staging, future logs. On first run, auto-migrates history from the legacy `C:\ProgramData\prinstall\` location.
- **Embedded data** — `data/drivers.toml` and `data/known_matches.toml` are compiled into the binary via `include_str!()`. No sidecar files to lose.
- **Escaped PS strings** — all user-controlled strings go through `escape_ps_string()` before entering `format!()` command templates. No injection vectors.
- **UAC manifest** — embedded via `embed-manifest` at build time so Windows prompts for elevation on launch.
- **Static CRT** — produces a zero-dependency Windows binary.

## Requirements

- **Windows 10/11** (Server 2016+) for installation functions
- **Administrator privileges** — `Add-Printer`, `Remove-Printer`, `pnputil /add-driver` all require elevation. UAC prompts automatically.
- **Network** — UDP/161 (SNMP), TCP/9100 · 631 · 515 (port probe), TCP/631 (IPP)

SNMP is no longer required. The port probe + IPP pipeline handles printers that don't speak SNMP at all.

## Development

```bash
cargo test                       # 147 tests, all on Linux via MockExecutor + INF fixture
cargo clippy -- -W clippy::all   # Lint
cargo build --release            # Local dev build (Linux / macOS ok)
```

Cross-compile a Windows binary from Linux:

```bash
docker run --rm -v "$PWD":/io -w /io messense/cargo-xwin:latest \
  bash -c 'ln -sf /usr/bin/llvm-mt /usr/local/bin/mt.exe && \
           cargo xwin build --release --target x86_64-pc-windows-msvc'
```

Windows release binaries are built automatically by GitHub Actions on tag push — see [`.github/workflows/release.yml`](.github/workflows/release.yml).

```
src/
├── main.rs                  Entry point, CLI dispatch
├── lib.rs                   Module declarations
├── cli.rs                   clap subcommands with rich help
├── models.rs                Printer, DriverMatch, PrinterOpResult, payloads
├── output.rs                Plain-text + JSON formatters, semantic coloring
├── paths.rs                 Canonical paths under %APPDATA%\prinstall\
├── config.rs                Persistent AppConfig (TOML)
├── history.rs               Install history log
├── privilege.rs             Windows admin detection
├── commands/
│   ├── add.rs               Network + USB install, catalog resolver, IPP fallback
│   ├── remove.rs            Three-step cleanup with spooler-lag retries
│   └── drivers.rs           Driver matching + catalog search + WU probe
├── core/
│   ├── executor.rs          PsExecutor trait, RealExecutor, MockExecutor
│   └── ps_error.rs          PowerShell stderr → clean single-line errors
├── discovery/               port_scan · ipp · snmp · local · subnet
├── drivers/
│   ├── matcher.rs           Numeric 0-1000 scoring (model-num + overlap + subseq)
│   ├── manifest.rs          Embedded data/drivers.toml (17 manufacturers)
│   ├── known_matches.rs     Embedded data/known_matches.toml
│   ├── downloader.rs        HTTP + ZIP/CAB extraction
│   ├── local_store.rs       Get-PrinterDriver enumeration
│   ├── catalog.rs           Microsoft Update Catalog scraper (Rust port of MSCatalogLTS)
│   ├── inf.rs               INF parser + IEEE 1284 HWID synthesizer
│   └── resolver.rs          Catalog → download → INF match orchestrator
├── installer/               powershell wrappers, multi-step orchestration
└── tui/                     Two-panel ratatui UI
data/
├── drivers.toml             Manufacturer registry — prefixes + universal driver URLs
└── known_matches.toml       Curated exact model → driver name mappings
tests/
├── fixtures/
│   └── brother_type1.inf    Real Brother Print Class Driver INF used by the
│                            HWID-match end-to-end test
└── *.rs                     Integration tests (140+ tests total, all run on
                             Linux via MockExecutor — no Windows required for CI)
```

## Roadmap

Shipped in `0.3.0`:

- [x] **Microsoft Update Catalog resolver** — pure-Rust scraper of
      `catalog.update.microsoft.com`, CID-based search, INF `[Models]` parser,
      deterministic `1284_CID_*` HWID match, newest-first candidate walk
- [x] **IEEE 1284 INF parser + HWID synthesizer** (`src/drivers/inf.rs`) with
      UTF-16 LE/BE BOM handling and a real Brother INF fixture test
- [x] `add` / `remove` commands with idempotent install + orphan cleanup
- [x] **Spooler-lag retry loop** in remove — settle sleep + backoff schedule,
      `-RemoveFromDriverStore` flag to take the underlying oem<N>.inf package
      with the registered driver
- [x] USB printer support via `--usb` flag
- [x] Microsoft IPP Class Driver fallback with visible audit breadcrumbs
- [x] `PsExecutor` trait for Linux-testable command logic
- [x] `core::ps_error::clean` — PowerShell stderr → single-line errors with
      HRESULT decoding, no more `CategoryInfo` / `FullyQualifiedErrorId` noise
- [x] `%APPDATA%\prinstall\` unified data directory + legacy migration
- [x] Terminal color output (crossterm, respects NO_COLOR)
- [x] IPP device ID surfacing in `drivers` output

Planned:

- [ ] Catalog resolver for USB printer installs (`add --usb`)
- [ ] Catalog result caching with TTL to cut network chatter on bulk installs
- [ ] `prinstall drivers <ip> --install <N>` to pick a specific catalog row
- [ ] Printer defaults — duplex, color/mono, paper size, set-default
- [ ] mDNS / WS-Discovery fallback for fully-silent printers
- [ ] Batch install mode (multiple IPs in one shot)
- [ ] `prinstall health <ip>` — toner/drum/tray status via SNMP Printer MIB
- [ ] User-editable subnet input inside the TUI (auto-detect already works)
- [ ] SignPath.io code signing for SmartScreen trust

## License

`prinstall` is released under the MIT License.

---

<div align="center">

Built in Rust.  ·  Born in an RMM shell.  ·  Designed for techs who just want the printer to work.

</div>

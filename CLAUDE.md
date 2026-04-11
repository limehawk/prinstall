# CLAUDE.md

## What This Is

Prinstall — a Rust CLI and TUI for Windows that discovers network printers,
matches them to drivers, and installs or removes them. Built for MSP technicians
running it locally or through RMM remote shells (SuperOps). Active development
happens on `feat/scaffold-printer-manager`; `main` still points at the v0.2.1
release.

## Architecture

**Dual interface, auto-detected:**
- **TUI mode** (real terminal): ratatui + crossterm, two-panel single-view
  layout (printer list + detail pane), lazy-style with vim keybindings
- **CLI mode** (pipe/RMM): clap subcommands with verbose plain text output,
  `--json` on every command for scripting

**Layers:**
1. **Interface** — `cli.rs` (clap), `tui/` (ratatui), `output.rs` (formatters
   + semantic coloring via crossterm)
2. **Commands** — `commands/add.rs`, `commands/remove.rs`, `commands/drivers.rs`
   — each an async fn that takes `&dyn PsExecutor` so the logic is unit-testable
   on Linux without a Windows host
3. **Core abstractions** — `core/executor.rs` (`PsExecutor` trait, `RealExecutor`,
   `MockExecutor`, free function `run_json<T>`), `core/ps_error.rs`
   (`clean(stderr)` parses PowerShell verbose errors into single-line messages
   with HRESULT decoding)
4. **Discovery** — `discovery/snmp.rs` (csnmp, 4 OIDs), `discovery/ipp.rs`
   (printer-make-and-model + printer-device-id), `discovery/port_scan.rs`
   (9100/631/515 parallel probe), `discovery/local.rs` (Get-Printer via PS),
   `discovery/subnet.rs` (CIDR + auto-detect from NIC)
5. **Drivers** — `drivers/matcher.rs` (numeric scoring 0-1000 with three
   components: model-number prefix, token overlap, skim subsequence),
   `drivers/manifest.rs` + `drivers/known_matches.rs` (embedded TOML),
   `drivers/downloader.rs` (HTTP + ZIP/CAB), `drivers/local_store.rs`
6. **Installer** — `installer/powershell.rs` (thin wrappers around
   `Add-PrinterPort` / `Add-PrinterDriver` / `Add-Printer` / `Remove-Printer` /
   etc. with `escape_ps_string` for injection safety), `installer/mod.rs`
   (three-step `install_printer` orchestration)
7. **Data + persistence** — `paths.rs` (canonical paths under
   `C:\ProgramData\prinstall\` with forward-migration from the 0.2.2–0.3.0 %APPDATA% layout),
   `config.rs` (TOML `AppConfig`), `history.rs` (install log)

**Key design decisions:**

- **`PsExecutor` trait** for all PowerShell calls. Real impl shells out, mock
  impl stubs responses. Every command is unit-testable on Linux via `cargo test`.
  Free function `run_json<T>()` for typed `ConvertTo-Json` deserialization (trait
  stays dyn-compatible).
- **`PrinterOpResult`** is the uniform result type with `detail: serde_json::Value`
  payload. `InstallDetail` and `RemoveDetail` are typed per-command payloads
  serialized into the `detail` field.
- **IPP Class Driver fallback**: when the primary install fails and port 631
  is open, `add` falls back to `Add-Printer -DriverName "Microsoft IPP Class Driver"`.
  Always surfaces a visible `WARNING:` line in the result so MSP audit trails
  can identify generic-fallback installs.
- **`C:\ProgramData\prinstall\`** — single machine-wide data directory for
  history, config, driver staging, future logs. ProgramData (not APPDATA) so
  SYSTEM-run RMM runbooks and interactive admin sessions share one audit
  trail instead of splitting across per-user silos. On first run under 0.3.1+,
  auto-migrates forward from the 0.2.2–0.3.0 `%APPDATA%\prinstall\`
  location if present. See `src/paths.rs` for the rationale.
- **Embedded data** — `data/drivers.toml` (17 manufacturers) and
  `data/known_matches.toml` (curated exact matches) compiled in via
  `include_str!()`. Note: most manufacturer entries in drivers.toml have empty
  URL fields — HP is currently the only one with a stable direct download URL.
  Brother/Canon/Epson/etc. fall through to IPP Class Driver fallback.
- **Terminal colors** via crossterm's `Stylize` trait, semantic helpers in
  `output.rs`. Auto-detects via `NO_COLOR` env var, `--json` flag, and
  stdout-is-terminal. VT mode enablement kicked via
  `execute!(stdout, ResetColor)` on Windows.
- PowerShell stderr is parsed through `core::ps_error::clean()` before surfacing
  — drops `At line:`, `CategoryInfo`, `FullyQualifiedErrorId` decorators, decodes
  HRESULT codes to human-readable text.
- UAC manifest embedded via `embed-manifest` build crate.
- Static CRT linking for zero-dependency Windows binary.

## CLI Commands

```
prinstall                                  Launch interactive TUI
prinstall scan [SUBNET]                    Multi-method subnet scan
prinstall id <IP>                          Identify a printer via SNMP
prinstall drivers <IP>                     Show matched + universal drivers + WU probe
prinstall add <IP>                         Install a network printer
prinstall add <QUEUE-NAME> --usb           Swap driver on an existing USB printer queue
prinstall remove <IP|QUEUE-NAME>           Remove printer + orphaned driver + port
prinstall list                             List locally installed printers
```

Global flags: `--json`, `--verbose`, `--community <str>`, `--force`,
`--subnet <cidr>`. Per-command flags: `--driver`, `--name`, `--model`, `--usb`
on `add`; `--keep-driver`, `--keep-port` on `remove`.

## Project Structure

```
src/
├── main.rs                  Entry point, CLI dispatch, thin cmd_* wrappers
├── lib.rs                   Module declarations
├── cli.rs                   clap Commands enum with rich help
├── models.rs                Printer, DriverMatch, PrinterOpResult, typed payloads
├── output.rs                Plain-text + JSON formatters, semantic coloring
├── paths.rs                 Canonical C:\ProgramData\prinstall\ paths + legacy APPDATA migration
├── config.rs                Persistent AppConfig (TOML)
├── history.rs               Install history log
├── privilege.rs             Windows admin detection
├── commands/
│   ├── add.rs               Network + USB install paths, IPP Class Driver fallback
│   ├── remove.rs            Three-step cleanup with orphan detection + system-port whitelist
│   └── drivers.rs           Driver matching + Windows Update probe (currently blocked on dockurr VMs)
├── core/
│   ├── executor.rs          PsExecutor trait, RealExecutor, MockExecutor, run_json<T>
│   └── ps_error.rs          PowerShell stderr → clean single-line errors + HRESULT lookup
├── discovery/
│   ├── snmp.rs              csnmp async queries
│   ├── ipp.rs               Binary IPP Get-Printer-Attributes (make/model + device-id)
│   ├── port_scan.rs         9100/631/515 parallel probe
│   ├── local.rs             Get-Printer via PS
│   ├── subnet.rs            CIDR + auto-detect from NIC
│   └── mod.rs               scan_subnet / full_discovery orchestration
├── drivers/
│   ├── matcher.rs           Numeric scoring 0-1000 (model-num + overlap + subseq)
│   ├── manifest.rs          Embedded drivers.toml (17 manufacturers)
│   ├── known_matches.rs     Embedded known_matches.toml
│   ├── downloader.rs        HTTP + ZIP/CAB extraction, staging under paths::staging_dir()
│   ├── local_store.rs       Get-PrinterDriver enumeration
│   └── mod.rs
├── installer/
│   ├── powershell.rs        Cmdlet wrappers, escape_ps_string, printer_exists helper
│   └── mod.rs               Three-step install orchestration
└── tui/
    ├── mod.rs               App state, event loop, Message enum
    ├── layout.rs            Three breakpoints: Wide/Stacked/Narrow
    ├── keys.rs, theme.rs
    └── views/               scan, drivers, install, help
data/
├── drivers.toml             Manufacturer registry — HP has real URLs, others empty
└── known_matches.toml       Curated exact matches (3 HP entries currently)
assets/
├── prinstall-icon.png       Rasterized source for the app icon (2048×2048)
├── prinstall-icon.pdf       Vector source (kept for future re-exports)
├── prinstall.ico            Compiled 7-resolution ICO (16/32/48/64/96/128/256)
└── prinstall.rc             Windows resource file — embedded via build.rs
tests/
├── cli_parse.rs             11 tests
├── matcher.rs               13 tests
├── models.rs                9 tests
├── output.rs                6 tests
├── manifest.rs              5 tests
├── known_matches.rs         3 tests
├── local_enum.rs            5 tests
├── port_scan.rs             5 tests
├── ipp.rs                   4 tests
└── subnet_parse.rs          10 tests
# Plus ~40 inline lib tests in src/commands/*.rs, src/core/*.rs, src/drivers/matcher.rs.
# Total: 100+ tests, all run on Linux via MockExecutor (no Windows required for CI).
```

## Development

```bash
# Tests run on Linux — MockExecutor stubs all PowerShell calls
cargo test
cargo clippy -- -W clippy::all
cargo build --release        # Linux native build (ratatui works, PS calls fail at runtime)
```

### Cross-compile a Windows binary from Linux

```bash
docker run --rm -v "$PWD":/io -w /io messense/cargo-xwin:latest \
  bash -c 'ln -sf /usr/bin/llvm-mt /usr/local/bin/mt.exe && \
           cargo xwin build --release --target x86_64-pc-windows-msvc'
```

Binary lands at `target/x86_64-pc-windows-msvc/release/prinstall.exe`.

Release builds happen via GitHub Actions `windows-latest` runner on tag push
(`.github/workflows/release.yml`). The docker workflow above is for dev loop only.

### Changing the app icon

The Windows app icon is embedded via a Windows `ICON` resource at build
time. `build.rs` calls `embed_resource::compile("assets/prinstall.rc", ...)`
on Windows targets only — Linux dev builds skip it so no ImageMagick or
resource compiler is needed for `cargo check` / `cargo test`.

To replace the icon:

1. Drop a new square transparent PNG in at `assets/prinstall-icon.png`
   (2048×2048 recommended — the auto-resize directive below downsamples
   to every standard icon size).
2. Regenerate the multi-resolution ICO:
   ```bash
   magick assets/prinstall-icon.png \
     -define icon:auto-resize=256,128,96,64,48,32,16 \
     assets/prinstall.ico
   ```
3. Rebuild — `build.rs` picks up the new `.ico` on the next Windows build.

All icon-related files:

- `assets/prinstall-icon.png` — rasterized source (2048×2048 transparent PNG)
- `assets/prinstall-icon.pdf` — vector source, kept for future re-exports
- `assets/prinstall.ico` — compiled 7-resolution ICO (16/32/48/64/96/128/256)
- `assets/prinstall.rc` — Windows resource file (`1 ICON "prinstall.ico"`)
- `build.rs` — `embed_resource::compile("assets/prinstall.rc", ...)` inside
  the `target_os == "windows"` branch, alongside the UAC manifest embed
- `Cargo.toml` — `embed-resource = "3"` in `[build-dependencies]`

## Testing infrastructure

- `MockExecutor` in `core/executor.rs` provides stateless first-match-wins
  command stubbing via `stub_exact`, `stub_prefix`, `stub_contains`, `stub_json`,
  `stub_failure`. Used by every command's inline tests.
- `run_json<T>` is a free function (not a trait method) so `PsExecutor` stays
  dyn-compatible. Callers that need typed JSON output use
  `core::executor::run_json(executor, cmd)`.
- Existing tests against PowerShell-adjacent code (install, remove, drivers) all
  run on Linux because the executor trait abstracts away the actual PS call.
  Real PS tests happen only via manual testing on a Windows VM.

## Dev loop (against a real Windows VM)

1. Edit code in Linux
2. `cargo test` — verify logic on Linux with MockExecutor
3. `docker run ... messense/cargo-xwin ...` — cross-compile the Windows binary
4. `cp target/x86_64-pc-windows-msvc/release/prinstall.exe ~/Windows/prinstall-dev.exe`
   — the `~/Windows/` directory is bind-mounted into a `dockurr/windows` VM as
   `\\host.lan\Data\`, so the binary appears there automatically
5. In the Windows VM PowerShell:
   `Copy-Item \\host.lan\Data\prinstall-dev.exe .\prinstall.exe -Force` (SMB
   caches exe files aggressively — copy to local path then run)
6. `.\prinstall.exe --version` / `.\prinstall.exe add <ip> --verbose` / etc.

Version-bump `Cargo.toml` on every dev build so you can distinguish builds in
the VM (currently `0.2.12-dev`).

## Spec & Plan

Design spec and implementation plan are in the rmm-scripts repo (gitignored there):
- `~/dev/rmm-scripts/docs/superpowers/specs/2026-03-18-prinstall-design.md`
- `~/dev/rmm-scripts/docs/superpowers/plans/2026-03-18-prinstall.md`

## Known gotchas

- **PowerShell `ConvertTo-Json` unwraps single-element pipelines** — use
  `ConvertTo-Json -InputObject @(...)` (NOT piped) for list queries. See
  `commands/drivers.rs::probe_windows_update` for the pattern.
- **`Add-Printer -ConnectionName "http://..."` returns HRESULT 0x80070032
  "Not supported"** on dockurr's Windows 11 image (and possibly others). The
  cmdlet doesn't trigger Windows Update driver lookup — it only wraps
  `InstallPrinterDriverFromPackage` which requires a pre-existing driver. This
  is why the WU probe feature is currently non-functional and we fall back to
  explicit `-DriverName "Microsoft IPP Class Driver"`.
- **`Microsoft IPP Class Driver` and other Windows system drivers are NOT
  removable** — the remove command skips them via `is_system_driver` whitelist.
- **TCP/IP printer port removal has a ~500ms spooler lag** — the
  `try_remove_port_if_orphaned` helper retries once after a delay.
- **SMB exe loader cache** — running an exe from `\\host.lan\Data\` caches the
  binary in Windows' SMB client, so overwriting the file doesn't evict the
  running exe. Always `Copy-Item ... -Force` to a local path before running.

## Current backlog

- [ ] Real manufacturer driver URLs in `drivers.toml` — HP works, others have empty URLs
- [ ] SDI driverpack integration — authoritative offline vendor driver database (~1GB)
- [ ] MSCatalogLTS PowerShell module integration — programmatic WU catalog query
      by printer model, returns downloadable .cab driver packages. Needs
      investigation against a real printer on a real VM.
- [ ] Windows Update install path that actually works — pending diagnostic probe
      that tests rundll32 / prnmngr.vbs / WMI / MSCatalogLTS against a real
      Brother printer
- [ ] Printer defaults (duplex, color/mono, paper size, set-default) via
      `Set-PrintConfiguration`
- [ ] `prinstall health <ip>` — toner/drum/tray status via SNMP Printer MIB
- [ ] mDNS / WS-Discovery fallback for fully-silent printers
- [ ] Batch install mode (multiple IPs in one shot)
- [ ] User-editable subnet input inside the TUI (auto-detect already works)
- [ ] SignPath.io code signing for SmartScreen trust

# CLAUDE.md

## What This Is

Prinstall — a Rust CLI and TUI for Windows that discovers network printers,
matches them to drivers, and installs or removes them. Built for MSP technicians
running it locally or through RMM remote shells (SuperOps). Active development
happens on the `dev` branch; `main` tracks the latest release and accumulates
website/docs iterations between cuts. See "Branching & release workflow" below.

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
  `include_str!()`. HP, Xerox, and Kyocera have stable direct download URLs.
  Other vendors fall through to the Catalog resolver or IPP Class Driver fallback.
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
prinstall scan [SUBNET]                    Multi-method subnet scan + USB enum
prinstall scan --network-only              Skip USB enumeration
prinstall scan --usb-only                  Skip network scan
prinstall id <IP>                          Identify a printer via SNMP
prinstall drivers <IP>                     Show matched drivers (also: `driver`)
prinstall add <IP>                         Install a network printer
prinstall add <QUEUE-NAME> --usb           Swap driver on an existing USB printer queue
prinstall remove <IP|QUEUE-NAME>           Remove printer + orphaned driver + port
prinstall list                             List locally installed printers
prinstall sdi status|refresh|list|prefetch|clean|verify   (--features sdi only)
```

Global flags: `--json`, `--verbose`, `--community <str>`, `--force`,
`--subnet <cidr>`. Per-command flags: `--driver`, `--name`, `--model`, `--usb`,
`--no-catalog` on `add`; `--no-sdi`, `--sdi-fetch` on `add` (sdi feature only);
`--keep-driver`, `--keep-port` on `remove`.

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
├── verbose.rs               Structured install report (Discovery → Resolution → Install → Summary)
├── commands/
│   ├── add.rs               Network + USB install paths, tier cascade, IPP Class Driver fallback
│   ├── remove.rs            Three-step cleanup with orphan detection + system-port whitelist
│   ├── drivers.rs           Driver matching + Windows Update probe
│   ├── sdi.rs               SDI subcommand (status/refresh/list/prefetch/clean) [sdi feature]
│   └── sdi_verify.rs        Authenticode .cat signature verification [sdi feature]
├── core/
│   ├── executor.rs          PsExecutor trait, RealExecutor, MockExecutor, run_json<T>
│   └── ps_error.rs          PowerShell stderr → clean single-line errors + HRESULT lookup
├── discovery/
│   ├── snmp.rs              csnmp async queries
│   ├── ipp.rs               Binary IPP Get-Printer-Attributes (make/model + device-id)
│   ├── port_scan.rs         9100/631/515 parallel probe
│   ├── local.rs             Get-Printer via PS
│   ├── subnet.rs            CIDR + auto-detect from NIC
│   ├── usb.rs               Get-PnpDevice enumeration + queue cross-ref
│   └── mod.rs               scan_subnet / full_discovery orchestration
├── drivers/
│   ├── matcher.rs           Numeric scoring 0-1000 (model-num + overlap + subseq)
│   ├── manifest.rs          Embedded drivers.toml (17 manufacturers)
│   ├── known_matches.rs     Embedded known_matches.toml
│   ├── downloader.rs        HTTP + ZIP/CAB extraction, staging under paths::staging_dir()
│   ├── local_store.rs       Get-PrinterDriver enumeration
│   ├── cab.rs               Pure-Rust CAB extraction (replaces expand.exe)
│   ├── sources.rs           Unified Source enum + SourceCandidate + InstallHint types
│   ├── sdi/                 SDI Origin integration [sdi feature]
│   │   ├── index.rs         Clean-room SDW binary index parser
│   │   ├── pack.rs          7z directory-prefix extraction via sevenz-rust2
│   │   ├── cache.rs         On-disk cache manager with LRU prune
│   │   ├── fetcher.rs       HTTP mirror fetcher with SHA256 + progress bars
│   │   └── resolver.rs      SDI candidate enumeration from cached indexes
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
├── drivers.toml             Manufacturer registry — HP, Xerox, Kyocera have URLs
└── known_matches.toml       Curated exact matches (HP + Xerox + Kyocera)
assets/
├── prinstall-icon.svg       Vector source — full orange-tile design
├── prinstall-icon-glyph.svg Vector source — transparent glyph for small sizes
├── prinstall-icon.png       Rasterized 2048×2048 PNG (rendered from the tile)
├── prinstall.ico            Compiled 7-resolution ICO (mixed tile + glyph)
├── prinstall.rc             Windows resource file — embedded via build.rs
└── icon-previews/           Reference renders at every standard size
    ├── tile/{16,32,48,64,96,128,256}.png
    └── glyph/{16,32,48,64,96,128,256}.png
tests/
├── cli_parse.rs             15 tests
├── matcher.rs               13 tests
├── models.rs                9 tests
├── output.rs                9 tests
├── manifest.rs              5 tests
├── known_matches.rs         3 tests
├── local_enum.rs            8 tests
├── port_scan.rs             5 tests
├── ipp.rs                   4 tests
├── subnet_parse.rs          13 tests
├── cab_extraction.rs        6 tests
├── usb_discovery.rs         2 tests
├── sdi_index.rs             6 tests  [sdi feature]
├── sdi_pack.rs              7 tests  [sdi feature]
├── sdi_cache.rs             17 tests [sdi feature]
└── sdi_fetcher.rs           10 tests [sdi feature]
# Plus 118 inline lib tests (150 with --features sdi) in src/commands/*.rs,
# src/core/*.rs, src/drivers/*.rs, src/discovery/*.rs, etc.
# Total: 210 tests without SDI (118 lib + 92 integration),
#        282 with --features sdi (150 lib + 92 non-SDI integration + 40 SDI integration).
# All run on Linux via MockExecutor (no Windows required for CI).
```

## Development

```bash
# Tests run on Linux — MockExecutor stubs all PowerShell calls
cargo test                          # default build (no SDI)
cargo test --features sdi           # with SDI modules
cargo clippy -- -W clippy::all
cargo build --release               # default binary (~8 MB)
cargo build --release --features sdi  # SDI-enabled binary (~9 MB)
```

### Cross-compile a Windows binary from Linux

```bash
docker run --rm -v "$PWD":/io -w /io messense/cargo-xwin:latest \
  bash -c 'ln -sf /usr/bin/llvm-mt /usr/local/bin/mt.exe && \
           cargo xwin build --release --target x86_64-pc-windows-msvc'
```

Binary lands at `target/x86_64-pc-windows-msvc/release/prinstall.exe`.

Release builds happen via GitHub Actions `windows-latest` runner on tag push
(`.github/workflows/release.yml`). CI builds both `prinstall.exe` (default) and
`prinstall-sdi.exe` (with SDI). The docker workflow above is for dev loop only.

### Changing the app icon

Two SVG sources: `assets/prinstall-icon.svg` (full tile, 48px+) and
`assets/prinstall-icon-glyph.svg` (transparent glyph, 16/32px). The ICO
uses the glyph at small sizes because the tile's printer detail is
invisible below 32px. Render with `rsvg-convert`, compose ICO with
`magick`. `build.rs` embeds via `embed_resource` on Windows targets only.
See `assets/icon-previews/` for reference renders at all standard sizes.

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
the VM (currently `0.4.0`).

## Branching & release workflow

The only two long-lived branches are `dev` and `main`. Everything else (feature
branches, agent worktrees, experimental spikes) is transient and should be
deleted after it merges or gets abandoned.

**The flow:**

1. All work happens on `dev`. Commit there directly or merge short-lived
   feature branches into it. Never commit directly to `main`.
2. When `dev` is ready to ship (a release, a docs iteration, anything), open
   a PR from `dev` → `main`:
   ```bash
   gh pr create --base main --head dev --title "<title>" --body "<body>"
   ```
3. Merge the PR with `gh pr merge <num> --merge` (regular merge commit, not
   squash — preserves the dev history). Then locally:
   ```bash
   git checkout main && git pull --ff-only
   git checkout dev
   ```
4. Keep working on `dev`. Rinse and repeat.

**What NOT to do:**

- Do not commit directly to `main` from the terminal, the GitHub web UI, or
  a mobile Claude Code session. Every stray commit on `main` that skips the
  `dev` PR flow creates a merge conflict later and breaks history audit.
- Do not push `claude/*`, `feat/*`, or any other transient branch straight
  into `main`. Route it through `dev` first.
- Do not squash-merge PRs. The non-squash merge commit is the marker we rely
  on to reason about release history.

**Releases specifically:**

- A release is just a `dev` → `main` PR that also includes a `Cargo.toml`
  version bump and a git tag pushed after the merge lands on `main`.
- CI on `windows-latest` builds the release binary from the tag — see
  `.github/workflows/release.yml`.
- Website-only iterations (the kind that touch `docs/` and nothing else) use
  the same PR flow but don't need a version bump or a tag — they just
  fast-forward `main`.

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

**Shipped (v0.4.0):**
- [x] SDI driverpack integration (behind `--features sdi` for supply chain review)
- [x] Pure-Rust CAB extraction (replaced `expand.exe`)
- [x] Xerox + Kyocera direct download URLs in `drivers.toml`
- [x] Structured verbose output (rice report)
- [x] `prinstall sdi verify` — Authenticode .cat signature verification
- [x] Duplicate printer detection (`--force` to reinstall)

**Shipped (v0.4.1):**
- [x] USB printer discovery via Get-PnpDevice (scan shows USB-attached devices + yellow-bang orphans)
- [x] USB stage-and-install flow via pnputil (for legacy printers like HP LaserJet 1320)
- [x] `prinstall list` shows IP column for network-attached queues
- [x] `driver` accepted as alias for `drivers` command
- [x] Scan flags: `--network-only`, `--usb-only`

**Open:**
- [ ] Authenticode verification at install time — only offer SDI drivers whose
      .cat passes signature check, then promote SDI to default (no feature flag)
- [ ] Lexmark Universal Print Driver URL — needs .exe extraction support
      (InstallShield wrapper, not zip/cab)
- [ ] Printer defaults (duplex, color/mono, paper size, set-default) via
      `Set-PrintConfiguration`
- [ ] `prinstall health <ip>` — toner/drum/tray status via SNMP Printer MIB
- [ ] Batch install mode (multiple IPs in one shot)
- [ ] SignPath.io code signing for SmartScreen trust
- [ ] Interactive TUI rework (lazygit-style panels)
- [ ] Feature-gate tests/sdi_*.rs properly — they fail to compile under `cargo test`
      (without `--features sdi`). Either add `#![cfg(feature = "sdi")]` at the top of
      each file, or configure CI to always pass `--features sdi`.

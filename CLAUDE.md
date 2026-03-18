# CLAUDE.md

## What This Is

Prinstall — a Rust TUI/CLI tool for Windows that discovers network printers via SNMP, matches them to drivers, and installs them. Built for MSP technicians running it locally or through RMM remote shells (SuperOps).

## Architecture

**Dual interface, auto-detected:**
- **TUI mode** (real terminal): ratatui + crossterm, 4-page navigation (Scan → Identify → Drivers → Install)
- **CLI mode** (pipe/RMM): clap subcommands with verbose plain text output, `--json` for scripting

**Four layers:**
1. **Interface** — `cli.rs` (clap), `tui/` (ratatui), `output.rs` (formatters)
2. **Discovery** — `discovery/snmp.rs` (csnmp async), `discovery/subnet.rs` (CIDR parsing, parallel scan)
3. **Drivers** — `drivers/matcher.rs` (fuzzy matching), `drivers/manifest.rs` + `drivers/known_matches.rs` (embedded TOML data), `drivers/downloader.rs` (HTTP + ZIP/CAB), `drivers/local_store.rs` (pnputil query)
4. **Installer** — `installer/powershell.rs` (Add-PrinterPort → Add-PrinterDriver → Add-Printer)

**Key design decisions:**
- Data files (`data/drivers.toml`, `data/known_matches.toml`) embedded via `include_str!()` — single binary, no sidecar files
- Driver results always show two sections: Matched (ranked by confidence) + Universal (always visible for manufacturer)
- PowerShell executor escapes strings for injection safety (`escape_ps_string`)
- Install history logged to `C:\ProgramData\prinstall\history.toml`
- UAC manifest embedded via `embed-manifest` build crate
- Static CRT linking for zero-dependency Windows binary (`.cargo/config.toml`)

## CLI Commands

```
prinstall scan [SUBNET]              # Scan subnet for printers via SNMP
prinstall id <IP>                    # Identify a single printer
prinstall drivers <IP>               # Show matched + universal drivers
prinstall install <IP>               # Full install (port + driver + queue)
prinstall                            # Launch TUI (if real terminal)
```

Global flags: `--json`, `--verbose`, `--community <str>`, `--model <str>`, `--force`

## Project Structure

```
src/
├── main.rs              # Entry point, CLI dispatch, all command handlers
├── lib.rs               # Module declarations
├── cli.rs               # clap subcommands with rich help
├── models.rs            # Printer, DriverMatch, DriverResults, InstallResult, History
├── output.rs            # Plain-text and JSON formatters
├── privilege.rs         # Windows admin detection
├── history.rs           # Install history (C:\ProgramData\prinstall\)
├── discovery/
│   ├── snmp.rs          # csnmp async queries
│   ├── subnet.rs        # CIDR parsing, size validation
│   └── mod.rs           # scan_subnet() orchestration
├── drivers/
│   ├── manifest.rs      # Embedded drivers.toml parsing
│   ├── known_matches.rs # Embedded known_matches.toml parsing
│   ├── matcher.rs       # Fuzzy matching + ranking
│   ├── downloader.rs    # HTTP download, ZIP/CAB extraction
│   ├── local_store.rs   # PowerShell driver enumeration
│   └── mod.rs
├── installer/
│   ├── powershell.rs    # PS cmdlet wrapper with string escaping
│   └── mod.rs           # Three-step install orchestration
└── tui/
    ├── mod.rs           # App state, event loop, page navigation
    ├── theme.rs         # Color constants
    └── views/           # scan, identify, drivers, install views
data/
├── drivers.toml         # Manufacturer → universal driver URLs (8 manufacturers)
└── known_matches.toml   # Curated model → driver name mappings
tests/
├── cli_parse.rs         # 7 tests
├── models.rs            # 4 tests
├── manifest.rs          # 5 tests
├── known_matches.rs     # 3 tests
├── matcher.rs           # 6 tests
├── output.rs            # 4 tests
└── subnet_parse.rs      # 7 tests
```

## Development

```bash
cargo test                # 36 tests
cargo clippy -- -W clippy::all
cargo build --release     # Linux dev build
```

Windows release builds happen via GitHub Actions (tag push triggers `.github/workflows/release.yml`).

## Spec & Plan

Design spec and implementation plan are in the rmm-scripts repo (gitignored there):
- `~/dev/rmm-scripts/docs/superpowers/specs/2026-03-18-prinstall-design.md`
- `~/dev/rmm-scripts/docs/superpowers/plans/2026-03-18-prinstall.md`

## Future Work (not yet implemented)

- Printer defaults (duplex, color/mono, paper size, default printer)
- mDNS / WS-Discovery for printers with SNMP disabled
- Shared match database across fleet
- Batch install mode
- TUI subnet input prompt (currently hardcoded to 192.168.1.0/24)
- SignPath.io code signing for SmartScreen trust

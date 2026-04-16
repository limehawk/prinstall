# Contributing to prinstall

Thanks for your interest. This is an MSP-first tool — the most valuable
contributions come from people who actually install printers for a living.
There are two ways to help, and they have **very** different bars to entry.

## Two contribution tracks

### 1. Driver data (no Rust required)

If you've installed a printer with `prinstall` and it picked the wrong driver,
or couldn't match at all, **that's a contribution waiting to happen**. The
embedded driver knowledge lives in two TOML files:

- [`data/drivers.toml`](data/drivers.toml) — manufacturer + universal-driver URLs
- [`data/known_matches.toml`](data/known_matches.toml) — curated exact model → driver mappings

**Both are plain TOML. No Rust, no build system, just pattern-match and edit.**

The full walkthrough with schema and examples:
**[docs/contributing-drivers.md](docs/contributing-drivers.md)**

Fastest path: open a [new-driver issue](../../issues/new?template=new_driver.yml)
— we'll turn it into a PR.

### 2. Code (Rust)

The installer, discovery, TUI, tests, etc. Standard open-source flow —
fork, branch, PR. Details below.

---

## Code contributions

### Project layout

- `src/cli.rs` — clap subcommands
- `src/commands/` — one file per top-level subcommand (`add`, `remove`, `drivers`, `driver`, `setup`, …)
- `src/discovery/` — network + USB discovery (SNMP, IPP, port scan, mDNS, Get-Printer, Get-PnpDevice)
- `src/drivers/` — matching, manifests, download, bundle, SDI
- `src/installer/` — PowerShell wrappers + install orchestration
- `src/core/executor.rs` — `PsExecutor` trait (real + mock). **Every PowerShell call goes through this.** That's what lets the test suite run on Linux without a Windows VM.
- `src/tui/` — ratatui code (currently under construction)
- `tests/` — integration tests
- `data/` — embedded TOML (drivers, known matches)

Full architecture lives in [`CLAUDE.md`](CLAUDE.md), which is also the
developer's map of the codebase.

### Development setup

```bash
# Linux dev loop — MockExecutor stubs all PowerShell calls
cargo test                            # default build (includes SDI)
cargo test --no-default-features      # lean build (no SDI)
cargo clippy -- -W clippy::all
cargo build --release                 # default binary with SDI
cargo build --release --no-default-features  # lean binary
```

### Cross-compile a Windows binary from Linux

```bash
docker run --rm -v "$PWD":/io -w /io messense/cargo-xwin:latest \
  bash -c 'ln -sf /usr/bin/llvm-mt /usr/local/bin/mt.exe && \
           cargo xwin build --release --target x86_64-pc-windows-msvc'
```

Binary lands at `target/x86_64-pc-windows-msvc/release/prinstall.exe`.

### Testing on a real Windows box

1. Edit on Linux → `cargo test` (MockExecutor makes the tests green on Linux)
2. Cross-compile → copy the exe to a Windows VM
3. Run the command you're changing with `--verbose` to see the raw PS round-trips
4. If it shells out to PowerShell, the test suite should have a MockExecutor-based
   unit test pinning the command shape

### What we look for in a code PR

- [ ] Tests pass: `cargo test` and `cargo test --no-default-features`
- [ ] `cargo clippy -- -W clippy::all` is clean for the files you touched
- [ ] New PowerShell calls go through `PsExecutor` + have a unit test using `MockExecutor`
- [ ] No secrets, credentials, or customer data in commit history
- [ ] Commit messages follow the existing style — `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `ci:`, scoped where useful (e.g., `fix(usb): ...`)

### Branching

- All development lands on `dev`
- Releases are a `dev` → `main` PR with a version bump and a git tag — CI builds the release binaries on tag push
- Don't push directly to `main`

### Code style

- Match existing style. No global reformatting PRs.
- Error messages are user-facing — write them for a tech reading them in an RMM shell at 11pm
- Follow the "why" rule on comments: explain **why** a non-obvious decision was made, not what the code does. Most of the existing comments (`[add] ...`, `[sdi] ...`) are operational traces for `--verbose`; keep that pattern

### Running the full local CI

```bash
cargo fmt --check
cargo clippy -- -W clippy::all
cargo test
cargo test --no-default-features
```

---

## Reporting bugs

Use the issue templates:
- [Bug report](../../issues/new?template=bug_report.yml) — something doesn't work
- [Driver match issue](../../issues/new?template=new_driver.yml) — prinstall picked the wrong driver or can't find one
- [Feature request](../../issues/new?template=feature_request.yml) — missing functionality

The bug template asks for `prinstall --verbose` output and the exact PS error.
The more raw output you paste, the faster the fix lands.

## Questions and community

- [Discussions](../../discussions) — Q&A, show-and-tell (field stories, fleet-wide
  deploys, weird printer setups), and early-stage ideas. Good for anything that
  isn't a concrete bug report or feature ask.
- [Discussions → Q&A](../../discussions/categories/q-a) — accepted-answer format
  for how-tos. Search here before opening an issue.

---

## License

By contributing you agree that your contributions are licensed under the
[MIT License](LICENSE) — the same license covering the rest of the repo.

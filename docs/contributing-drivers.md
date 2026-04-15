# Contributing driver data

The fastest way to help `prinstall` work on more printers: add to the two
TOML files that encode its driver knowledge. **You don't need to know Rust.**
If you can pattern-match and edit a text file, you can contribute.

## The two files

### `data/known_matches.toml`

Curated exact model → driver mappings. When SNMP or IPP returns this model
string, prinstall uses that driver. No ambiguity, no fuzzy scoring.

```toml
[[matches]]
model = "HP LaserJet Pro MFP M428fdw"
driver = "HP LaserJet Pro MFP M428f PCL-6 (V4)"
source = "manufacturer"
```

- `model` — exact string prinstall sees from `prinstall id <ip>` or `prinstall scan`
- `driver` — exact name as it appears in Windows `Get-PrinterDriver`
- `source` — `"manufacturer"` (vendor INF in the store) or `"local_store"` (any staged driver)

### `data/drivers.toml`

Manufacturer-level universal drivers with direct download URLs. When
`known_matches.toml` has no entry for a model, prinstall falls back to the
manufacturer's universal driver.

```toml
[[manufacturers]]
name = "HP"
prefixes = ["HP", "Hewlett-Packard", "Hewlett Packard"]

[[manufacturers.universal_drivers]]
name = "HP Universal Print Driver PCL6"
url = "https://ftp.hp.com/pub/softlib/software13/printers/UPD/upd-pcl6-x64-7.9.0.26347.zip"
format = "zip"
```

- `name` — manufacturer display name
- `prefixes` — strings that mark a model as this manufacturer (`"Hewlett-Packard LaserJet 1320"` starts with `"Hewlett-Packard"` → HP)
- `universal_drivers[].name` — what Windows will register the driver as after install
- `universal_drivers[].url` — direct download link. Must 200 OK on a HEAD request.
- `universal_drivers[].format` — `"zip"` or `"cab"` (today). `.exe` installers aren't supported yet.

## How to add a known match

Scenario: you just installed an **HP LaserJet Enterprise M607** and had to pick
the driver manually because prinstall guessed wrong.

### Step 1 — capture the exact strings

```powershell
prinstall id 192.168.1.50 --json
```

Copy the `model` field verbatim. Then:

```powershell
Get-PrinterDriver | Where-Object Name -like 'HP*M607*' | Select Name
```

Copy the `Name` verbatim.

### Step 2 — add the entry

Open `data/known_matches.toml` and append:

```toml
[[matches]]
model = "HP LaserJet Enterprise M607"
driver = "HP LaserJet Enterprise M607-M608-M609 PCL 6"
source = "manufacturer"
```

Keep the file sorted by manufacturer + model to make future edits easier.
If you're not sure where something goes, append it at the end — we'll
sort during review.

### Step 3 — verify it locally (optional)

```bash
cargo test --no-default-features   # runs all tests including TOML parse checks
```

The tests should pass. If they don't, the TOML is malformed — double-check
the bracket-bracket (`[[matches]]`) and make sure all three fields are present.

### Step 4 — submit

- Open a PR against the `dev` branch
- Title: `data(matches): add HP LaserJet Enterprise M607`
- Body: one line describing when/how you confirmed the match

Or, if you don't want to open a PR, file a **"new driver"** issue with the
data and we'll turn it into a PR.

## How to add a manufacturer universal driver

Scenario: you want to wire up **Ricoh PCL6 Universal Driver** so every
Ricoh model in prinstall can fall back to it.

### Step 1 — find a stable direct download URL

The URL must:
- Not require form submission, cookies, or JavaScript
- Not be region-gated (or, if it is, document which region it serves)
- 200 OK on a HEAD request
- Point at a `.zip` or `.cab` archive containing one or more `.inf` files

HP, Xerox, and Kyocera all publish stable direct URLs. Brother/Canon/Epson/Ricoh
historically require form flows; if you find a stable URL, verify it with
`curl -IL <url>` before submitting.

### Step 2 — add the entry

```toml
[[manufacturers]]
name = "Ricoh"
prefixes = ["Ricoh", "RICOH", "Savin", "Lanier"]  # Ricoh rebrands

[[manufacturers.universal_drivers]]
name = "Ricoh PCL6 Universal Driver"
url = "https://support.ricoh.com/.../rpdl4-pcl6-x64-<version>.zip"
format = "zip"
```

### Step 3 — verify

```bash
cargo test --no-default-features
```

### Step 4 — submit

- PR title: `data(drivers): add Ricoh PCL6 Universal Driver`
- Body: the URL you verified and a test model the driver should work on

## Vendor-specific notes

### HP

- URLs carry the version in the filename (`upd-pcl6-x64-7.9.0.26347.zip`)
- New UPD releases change the URL — we bump these periodically
- PCL and PS are independent version streams

### Xerox

- Global Print Driver covers VersaLink, AltaLink, Phaser, and WorkCentre
- Version embedded in the path — bumps required occasionally

### Kyocera

- KX Driver evergreen URL via AEM dispatcher alias (stays stable across versions)

### Brother / Canon / Epson / Lexmark / Ricoh

- Currently no direct URL — these fall through to the Microsoft Update Catalog or SDI tiers
- If you find a reliably stable direct URL, contributions welcome

## Triaging a bad match

Sometimes prinstall picks a "close but wrong" driver — the model has 5 variants
and it scored the wrong sibling highest. That's not a data gap in
`drivers.toml`; it's a signal to add a curated entry to `known_matches.toml`
that pins the exact right driver for this model.

Run `prinstall drivers <ip> --verbose` and look at the scoring output.
If the "right" driver is somewhere in the ranked list but not the top, adding
a `known_matches.toml` entry promotes it to an exact match (score 1000).

## What we don't want

- **Speculative entries** — don't add matches you haven't verified on a real printer
- **Driver URLs behind login walls** — if `curl -IL` fails without cookies, it won't work in CI either
- **Huge sweep PRs** — "I added 50 models from an AI-generated list" is noise. 1–10 verified models per PR is ideal.
- **Manufacturer rename spam** — if you're adding `"HP Inc."` as a prefix alias, one-off entries are fine; if you're reshuffling every vendor's prefixes, open an issue first.

## Questions?

Open a [discussion](../../discussions) or a [new-driver issue](../../issues/new?template=new_driver.yml).
Driver-match questions from real MSP techs are the best feedback we get —
they're how we figure out what prinstall gets wrong in the field.

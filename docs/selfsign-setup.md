# Self-Signed Code Signing Setup

This is the $0 interim path for getting prinstall.exe past Smart App Control
on MSP-managed endpoints without buying a commercial code signing cert.

**Scope.** After the one-time setup below, every release binary is signed
with a self-signed cert. Public GitHub downloaders still hit SmartScreen/SAC
blocks because the cert isn't chained to a publicly-trusted CA — but managed
endpoints where we've pushed the companion `prinstall-codesign.cer` into the
local trust store (via the rmm-scripts rollout) pass SAC cleanly.

**Trade-offs.**
- $0 recurring cost. No SignPath clout gate. No $200/yr DigiCert bill.
- Trust is fleet-scoped — only endpoints that have imported our `.cer` trust
  the signature. Off-fleet public downloads still hit SAC.
- Timestamped, so signatures remain valid past cert expiration.
- When we eventually ship with a publicly-trusted CA, this scaffolding gets
  deleted and releases move to that cert with zero workflow disruption.

---

## One-time actions (Watson)

### 1. Generate the signing cert (local Windows box, elevated PowerShell)

Run ONCE. The `.cer` (public) gets committed to rmm-scripts. The `.pfx`
(private) gets uploaded as a GitHub Actions secret and then deleted from
disk — 1Password holds the password, GitHub holds the bytes.

```powershell
$cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject "CN=Prinstall Code Signing, O=Limehawk, C=US" `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -NotAfter (Get-Date).AddYears(10) `
    -CertStoreLocation "Cert:\CurrentUser\My"

# Export public cert (.cer) — safe to commit to rmm-scripts
Export-Certificate -Cert $cert -FilePath "$env:USERPROFILE\Desktop\prinstall-codesign.cer"

# Export PFX (private key) — keep secret
$pwd = Read-Host -AsSecureString "Cert password (save in 1Password)"
Export-PfxCertificate -Cert $cert -FilePath "$env:USERPROFILE\Desktop\prinstall-codesign.pfx" -Password $pwd

# Base64 encode PFX for GitHub secret
$pfxBytes = [IO.File]::ReadAllBytes("$env:USERPROFILE\Desktop\prinstall-codesign.pfx")
$base64 = [Convert]::ToBase64String($pfxBytes)
Set-Clipboard $base64
Write-Host "PFX base64 copied to clipboard — paste into GitHub secret CODESIGN_PFX_BASE64"
```

### 2. Upload GitHub Actions secrets

From the prinstall repo root:

```bash
# Paste the base64 blob when prompted (already on clipboard from step 1)
gh secret set CODESIGN_PFX_BASE64

# Paste the password (the one from the Read-Host prompt, saved in 1Password)
gh secret set CODESIGN_PFX_PASSWORD
```

Or via the GitHub web UI: **Settings → Secrets and variables → Actions →
New repository secret**. Names must be exact: `CODESIGN_PFX_BASE64` and
`CODESIGN_PFX_PASSWORD`.

### 3. Commit the public cert to rmm-scripts

The `.cer` is the public half — safe to commit. The `.pfx` is the private
half — **never commit**, never push anywhere but the GitHub secret.

```bash
cp ~/Desktop/prinstall-codesign.cer ~/dev/rmm-scripts/scripts/prinstall-codesign.cer
cd ~/dev/rmm-scripts
git add scripts/prinstall-codesign.cer
git commit -m "chore(prinstall): add self-signed code signing public cert for fleet trust rollout"
git push
```

### 4. Scrub the local .pfx

Once the secret is uploaded and verified, wipe the .pfx off the Desktop. It
exists only in GitHub secrets and 1Password from here on.

```powershell
Remove-Item "$env:USERPROFILE\Desktop\prinstall-codesign.pfx" -Force
```

### 5. Deploy the trust rollout script via SuperOps

`scripts/prinstall_trust_codesign.ps1` in the rmm-scripts repo imports the
`.cer` into `Cert:\LocalMachine\Root` and `Cert:\LocalMachine\TrustedPublisher`
on each managed endpoint. Schedule it to run once at agent deployment time
(it's idempotent — safe to re-run).

---

## What CI does on every tag push

`release.yml` on `windows-latest`:

1. Builds `prinstall.exe` (default) and `prinstall-nosdi.exe` (lean).
2. If `CODESIGN_PFX_BASE64` and `CODESIGN_PFX_PASSWORD` are set, decodes
   the .pfx, runs `signtool.exe sign` on both binaries with a DigiCert
   public timestamp, then wipes the .pfx from the runner.
3. If the secrets are missing, signing steps no-op — releases still ship
   (unsigned) during the setup window.

Timestamp server: `http://timestamp.digicert.com`. SHA-256 file digest and
timestamp digest (required for Win10+).

---

## Cert renewal (once every 10 years)

The cert is valid for 10 years. When it expires:

1. Re-run the cert generation script from step 1.
2. Update the `CODESIGN_PFX_BASE64` and `CODESIGN_PFX_PASSWORD` secrets.
3. Commit the new `prinstall-codesign.cer` to rmm-scripts/scripts.
4. Re-run `prinstall_trust_codesign.ps1` across the fleet to trust the
   new cert.

**Timestamps save us on already-shipped binaries** — because signtool embeds
a DigiCert timestamp token, binaries signed with the old cert remain valid
after the cert expires. We only need fleet re-trust for *new* releases signed
with the new cert.

---

## Eventual migration off self-signed

When budget allows a commercial cert (or SignPath.io OSS approval comes
through), the migration is a two-secret swap:

1. Replace `CODESIGN_PFX_BASE64` + `CODESIGN_PFX_PASSWORD` with the new
   cert's bytes/password.
2. `release.yml` needs no changes — signtool invocation is identical.
3. The rmm-scripts trust push becomes a no-op for new releases (public CA
   chain works without local trust). Keep the old `.cer` in the trust
   stores so existing signed binaries stay trusted.

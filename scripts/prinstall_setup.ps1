<#
██╗     ██╗███╗   ███╗███████╗██╗  ██╗ █████╗ ██╗    ██╗██╗  ██╗
██║     ██║████╗ ████║██╔════╝██║  ██║██╔══██╗██║    ██║██║ ██╔╝
██║     ██║██╔████╔██║█████╗  ███████║███████║██║ █╗ ██║█████╔╝
██║     ██║██║╚██╔╝██║██╔══╝  ██╔══██║██╔══██║██║███╗██║██╔═██╗
███████╗██║██║ ╚═╝ ██║███████╗██║  ██║██║  ██║╚███╔███╔╝██║  ██╗
╚══════╝╚═╝╚═╝     ╚═╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝ ╚══╝╚══╝ ╚═╝  ╚═╝
================================================================================
 SCRIPT   : Prinstall Public Installer                                    v1.0.0
 AUTHOR   : Limehawk.io
 DATE     : April 2026
 USAGE    : iwr -useb https://raw.githubusercontent.com/limehawk/prinstall/main/scripts/prinstall_setup.ps1 | iex
================================================================================
 FILE     : prinstall_setup.ps1
 DESCRIPTION : Downloads the latest prinstall release and installs it machine-wide
--------------------------------------------------------------------------------
 README
--------------------------------------------------------------------------------
 PURPOSE

   Installs prinstall to C:\ProgramData\prinstall\prinstall.exe and adds
   the install directory to Machine PATH so "prinstall" is available from
   any shell. Intended for MSP techs doing a manual install on a
   workstation. RMM deployments use rmm-scripts\prinstall_setup.ps1 — both
   land in the same C:\ProgramData\prinstall\ directory so the audit
   trail stays unified.

 DATA SOURCES & PRIORITY

   - GitHub Releases API (api.github.com) — latest release tag
   - Release assets (github.com):
       1. Bare prinstall.exe asset (preferred, single-step install)
       2. prinstall-windows-amd64.zip (fallback, extracted on the fly
          for releases that ship only the zip)

 REQUIRED INPUTS

   Public installer — no hardcoded inputs to edit before running.
   Optional parameter:
     - -Version vX.Y.Z : Install a specific release tag instead of latest

 SETTINGS

   - $installDir        : C:\ProgramData\prinstall
   - $installedExe      : C:\ProgramData\prinstall\prinstall.exe
   - $repoOwner         : limehawk
   - $repoName          : prinstall
   - $firewallRuleName  : Prinstall (mDNS discovery)

 BEHAVIOR

   1. Verify the shell is elevated — ProgramData writes and Machine PATH
      updates both require admin.
   2. Resolve the target release tag (latest via GitHub API, or the tag
      provided via -Version).
   3. Try to download the bare prinstall.exe release asset. On failure,
      fall back to the zip asset and extract prinstall.exe from it.
   4. Move the binary into C:\ProgramData\prinstall\ via a staging
      temp dir so a failed download cannot clobber a working install.
   5. Create a Windows Firewall rule allowing inbound UDP 5353 so the
      mDNS discovery backend can receive multicast responses. Idempotent:
      skipped if an identically named rule already exists.
   6. Add C:\ProgramData\prinstall to Machine PATH if missing, and
      update $env:Path in the current session so prinstall is usable
      immediately without a shell restart.

 PREREQUISITES

   - Windows 10 / Server 2016 or newer
   - PowerShell 5.1 or newer
   - Administrator privileges (elevated PowerShell)
   - Network access to api.github.com and github.com

 SECURITY NOTES

   - No secrets in logs. Nothing sensitive passes through this script.
   - The public installer pattern executes remote script content via
     iex. Users who want to audit first should download
     prinstall_setup.ps1 locally and run it from disk instead of piping
     it through iex.
   - TLS 1.2 is forced before any network call. PowerShell 5.1 defaults
     to TLS 1.0 on some boxes, which GitHub rejects.
   - The mDNS firewall rule is inbound UDP 5353 only. It does not open
     any TCP ports or expose prinstall itself to the network.

 ENDPOINTS

   - https://api.github.com/repos/limehawk/prinstall/releases/latest
   - https://github.com/limehawk/prinstall/releases/download/<tag>/prinstall.exe
   - https://github.com/limehawk/prinstall/releases/download/<tag>/prinstall-windows-amd64.zip

 EXIT CODES

   0 = Success
   1 = Failure (not elevated, download failed, write failed, etc.)

 EXAMPLE RUN

   [INFO] INPUT VALIDATION
   ==============================================================
   Elevation       : Admin
   Target version  : latest
   Install dir     : C:\ProgramData\prinstall
   Inputs validated successfully

   [RUN] RESOLVE RELEASE
   ==============================================================
   Tag             : v0.3.5

   [RUN] DOWNLOAD
   ==============================================================
   [RUN] Trying bare exe asset
   [OK]  Downloaded 12.1 MB
   Source          : prinstall.exe

   [RUN] INSTALL
   ==============================================================
   [OK]  Installed C:\ProgramData\prinstall\prinstall.exe

   [RUN] FIREWALL RULE
   ==============================================================
   [OK]  Created 'Prinstall (mDNS discovery)' allowing UDP 5353

   [RUN] PATH
   ==============================================================
   [OK]  Added C:\ProgramData\prinstall to Machine PATH
   Session PATH updated

   [OK] FINAL STATUS
   ==============================================================
   Status          : Success
   Version         : v0.3.5
   Location        : C:\ProgramData\prinstall\prinstall.exe
   Run             : prinstall --help

   [OK] SCRIPT COMPLETED
   ==============================================================

--------------------------------------------------------------------------------
 CHANGELOG
--------------------------------------------------------------------------------
 2026-04-11 v1.0.0 Initial public installer. Installs to
                   C:\ProgramData\prinstall, adds to Machine PATH, and
                   creates the mDNS firewall rule on UDP 5353. Prefers
                   the bare prinstall.exe release asset, falls back to
                   extracting prinstall-windows-amd64.zip for older
                   releases that ship only the zip.
================================================================================
#>

# [CmdletBinding] + param is intentional here and not a framework violation:
# this is a public, user-invoked installer fetched via iwr | iex, not a
# SuperOps runbook. The hardcoded-inputs rule applies to RMM scripts where
# SuperOps injects runtime variables; it doesn't apply to a bootstrap that
# needs a real user-facing -Version knob.
[CmdletBinding()]
param(
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$ProgressPreference    = 'SilentlyContinue'
Set-StrictMode -Version Latest

# PowerShell 5.1 defaults to TLS 1.0 on some boxes and GitHub rejects
# anything below TLS 1.2. Force TLS 1.2 before any network call.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # Ancient .NET without the Tls12 enum — let the first download fail loudly.
}

# ============================================================================
# SETTINGS
# ============================================================================
$repoOwner        = 'limehawk'
$repoName         = 'prinstall'
$installDir       = Join-Path $env:ProgramData 'prinstall'
$installedExe     = Join-Path $installDir 'prinstall.exe'
$firewallRuleName = 'Prinstall (mDNS discovery)'

function Write-Section {
    param(
        [ValidateSet('info','run','ok','warn','error')][string]$Type,
        [string]$Name
    )
    $label = @{ info = 'INFO'; run = 'RUN'; ok = 'OK'; warn = 'WARN'; error = 'ERROR' }[$Type]
    Write-Host ''
    Write-Host "[$label] $Name"
    Write-Host '=============================================================='
}

# ============================================================================
# INPUT VALIDATION
# ============================================================================
Write-Section info 'INPUT VALIDATION'

$errorOccurred = $false
$errorText     = ''

$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin = ([Security.Principal.WindowsPrincipal]$currentIdentity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    $errorOccurred = $true
    if ($errorText.Length -gt 0) { $errorText += "`n" }
    $errorText += "- Administrator privileges required. Right-click PowerShell -> Run as Administrator, then re-run the installer."
}

if ($PSBoundParameters.ContainsKey('Version') -and -not [string]::IsNullOrWhiteSpace($Version)) {
    $targetTag         = if ($Version -match '^v') { $Version } else { "v$Version" }
    $targetDescription = $targetTag
} else {
    $targetTag         = $null
    $targetDescription = 'latest'
}

if ($errorOccurred) {
    Write-Section error 'ERROR OCCURRED'
    Write-Host $errorText
    exit 1
}

Write-Host "Elevation       : Admin"
Write-Host "Target version  : $targetDescription"
Write-Host "Install dir     : $installDir"
Write-Host "Inputs validated successfully"

# Staging dir for downloads. Wrapped in try/finally at the bottom so any
# failure below still cleans up the staging artifacts.
$stagingRoot = Join-Path $env:TEMP ("prinstall-setup-{0}" -f ([guid]::NewGuid().Guid))
$null = New-Item -ItemType Directory -Path $stagingRoot -Force

try {
    # ========================================================================
    # RESOLVE RELEASE
    # ========================================================================
    Write-Section run 'RESOLVE RELEASE'

    if ($null -eq $targetTag) {
        $apiUrl = "https://api.github.com/repos/$repoOwner/$repoName/releases/latest"
        try {
            $release = Invoke-RestMethod -Uri $apiUrl -UseBasicParsing
        } catch {
            Write-Section error 'ERROR OCCURRED'
            Write-Host "Failed to reach the GitHub API to resolve the latest release."
            Write-Host "Error : $($_.Exception.Message)"
            exit 1
        }
        $targetTag = $release.tag_name
    }

    Write-Host "Tag             : $targetTag"

    # ========================================================================
    # DOWNLOAD
    # ========================================================================
    Write-Section run 'DOWNLOAD'

    $bareExeUrl = "https://github.com/$repoOwner/$repoName/releases/download/$targetTag/prinstall.exe"
    $zipUrl     = "https://github.com/$repoOwner/$repoName/releases/download/$targetTag/prinstall-windows-amd64.zip"

    $downloadedExe = $null
    $assetSource   = $null

    # Tier 1: bare exe asset (present on 0.3.5+ releases).
    Write-Host "[RUN] Trying bare exe asset"
    $tempExe = Join-Path $stagingRoot 'prinstall.exe'
    try {
        Invoke-WebRequest -Uri $bareExeUrl -OutFile $tempExe -UseBasicParsing
        $downloadedExe = $tempExe
        $assetSource   = 'prinstall.exe'
    } catch {
        Write-Host "[INFO] Bare exe asset not found, falling back to zip"
    }

    # Tier 2: zip asset (every 0.3.x release has this).
    if ($null -eq $downloadedExe) {
        Write-Host "[RUN] Downloading prinstall-windows-amd64.zip"
        $tempZip = Join-Path $stagingRoot 'prinstall-windows-amd64.zip'
        try {
            Invoke-WebRequest -Uri $zipUrl -OutFile $tempZip -UseBasicParsing
        } catch {
            Write-Section error 'ERROR OCCURRED'
            Write-Host "Failed to download release assets for $targetTag."
            Write-Host "Tried : $bareExeUrl"
            Write-Host "Tried : $zipUrl"
            Write-Host "Error : $($_.Exception.Message)"
            exit 1
        }

        Write-Host "[RUN] Extracting prinstall.exe from zip"
        $extractDir = Join-Path $stagingRoot 'extracted'
        Expand-Archive -Path $tempZip -DestinationPath $extractDir -Force
        $extractedExe = Join-Path $extractDir 'prinstall.exe'
        if (-not (Test-Path $extractedExe)) {
            Write-Section error 'ERROR OCCURRED'
            Write-Host "prinstall.exe not found inside prinstall-windows-amd64.zip for $targetTag."
            exit 1
        }
        $downloadedExe = $extractedExe
        $assetSource   = 'prinstall-windows-amd64.zip'
    }

    $sizeMb = [math]::Round((Get-Item $downloadedExe).Length / 1MB, 1)
    Write-Host "[OK]  Downloaded $sizeMb MB"
    Write-Host "Source          : $assetSource"

    # ========================================================================
    # INSTALL
    # ========================================================================
    Write-Section run 'INSTALL'

    if (-not (Test-Path $installDir)) {
        $null = New-Item -ItemType Directory -Path $installDir -Force
        Write-Host "[OK]  Created $installDir"
    }

    try {
        Move-Item -Path $downloadedExe -Destination $installedExe -Force
    } catch {
        Write-Section error 'ERROR OCCURRED'
        Write-Host "Could not write $installedExe. Close any running prinstall and re-run."
        Write-Host "Error : $($_.Exception.Message)"
        exit 1
    }
    Write-Host "[OK]  Installed $installedExe"

    # ========================================================================
    # FIREWALL RULE
    # ========================================================================
    Write-Section run 'FIREWALL RULE'

    $existingRule = Get-NetFirewallRule -DisplayName $firewallRuleName -ErrorAction SilentlyContinue
    if ($existingRule) {
        Write-Host "[OK]  '$firewallRuleName' already exists"
    } else {
        try {
            New-NetFirewallRule `
                -DisplayName $firewallRuleName `
                -Description 'Allow prinstall.exe to receive mDNS multicast responses on UDP 5353 for network printer discovery.' `
                -Direction Inbound `
                -Protocol UDP `
                -LocalPort 5353 `
                -Action Allow `
                -Profile Any `
                -Enabled True | Out-Null
            Write-Host "[OK]  Created '$firewallRuleName' allowing UDP 5353"
        } catch {
            Write-Host "[WARN] Could not create firewall rule: $($_.Exception.Message)"
            Write-Host "       mDNS discovery may be blocked until the rule is added manually."
        }
    }

    # ========================================================================
    # PATH
    # ========================================================================
    Write-Section run 'PATH'

    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $entries     = @()
    if ($machinePath) { $entries = $machinePath -split ';' | Where-Object { $_ -ne '' } }
    if ($entries -notcontains $installDir) {
        $newPath = (($entries + $installDir) -join ';')
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'Machine')
        Write-Host "[OK]  Added $installDir to Machine PATH"
    } else {
        Write-Host "[OK]  Already on Machine PATH"
    }

    # Update $env:Path in the current session so prinstall is usable right
    # away without waiting for a fresh shell to pick up the Machine change.
    if (($env:Path -split ';') -notcontains $installDir) {
        $env:Path = "$env:Path;$installDir"
    }
    Write-Host "Session PATH updated"

    # ========================================================================
    # FINAL STATUS
    # ========================================================================
    Write-Section ok 'FINAL STATUS'
    Write-Host "Status          : Success"
    Write-Host "Version         : $targetTag"
    Write-Host "Location        : $installedExe"
    Write-Host "Run             : prinstall --help"

    Write-Section ok 'SCRIPT COMPLETED'
    exit 0
} finally {
    if (Test-Path $stagingRoot) {
        Remove-Item $stagingRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

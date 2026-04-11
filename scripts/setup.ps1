<#
.SYNOPSIS
    Installs prinstall to a canonical location and adds it to PATH.

.DESCRIPTION
    Downloads the latest prinstall.exe from the GitHub Releases page,
    installs it to a standard directory, and adds that directory to PATH
    so you can run `prinstall` from any shell.

    Default behavior is a per-user install under
    $env:LOCALAPPDATA\Programs\prinstall — no admin required for the
    install itself. (Running prinstall still needs admin because it
    installs printer drivers.)

    Pass -Machine to install under $env:ProgramFiles\prinstall with
    Machine-scope PATH. That requires an elevated PowerShell session.

.PARAMETER Machine
    Install machine-wide to "$env:ProgramFiles\prinstall" and update
    Machine PATH. Requires admin.

.PARAMETER Version
    Specific release tag to install (e.g. "v0.3.4"). Defaults to the
    latest release.

.EXAMPLE
    # Per-user install, no admin needed:
    iwr -useb https://raw.githubusercontent.com/limehawk/prinstall/main/scripts/setup.ps1 | iex

.EXAMPLE
    # Machine-wide install from an elevated shell:
    $s = iwr -useb https://raw.githubusercontent.com/limehawk/prinstall/main/scripts/setup.ps1
    & ([scriptblock]::Create($s.Content)) -Machine

.LINK
    https://github.com/limehawk/prinstall
#>
[CmdletBinding()]
param(
    [switch]$Machine,
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# GitHub rejects TLS 1.0/1.1. PowerShell 5.1 defaults to TLS 1.0 on some
# boxes, so force TLS 1.2+ before any network call.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # Older .NET without Tls12 enum — nothing we can do, let the download fail loud.
}

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message" -ForegroundColor Cyan
}
function Write-Ok {
    param([string]$Message)
    Write-Host "    $Message" -ForegroundColor Green
}

# Resolve install location and PATH scope
if ($Machine) {
    $installDir = Join-Path $env:ProgramFiles 'prinstall'
    $pathScope  = 'Machine'

    $isAdmin = ([Security.Principal.WindowsPrincipal] `
        [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $isAdmin) {
        throw "Machine-wide install requires an elevated PowerShell. Right-click PowerShell -> Run as Administrator, then re-run this command."
    }
} else {
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\prinstall'
    $pathScope  = 'User'
}

# Resolve release tag
Write-Step "Resolving release"
if ($Version) {
    $tag = if ($Version -match '^v') { $Version } else { "v$Version" }
} else {
    $apiUrl  = 'https://api.github.com/repos/limehawk/prinstall/releases/latest'
    try {
        $release = Invoke-RestMethod -Uri $apiUrl -UseBasicParsing
    } catch {
        throw "Could not reach the GitHub API to resolve the latest release: $($_.Exception.Message)"
    }
    $tag = $release.tag_name
}
Write-Ok $tag

$downloadUrl = "https://github.com/limehawk/prinstall/releases/download/$tag/prinstall.exe"

# Create install dir
Write-Step "Preparing install directory"
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}
Write-Ok $installDir

# Download to a temp file, then move into place so a failed download
# doesn't clobber a working binary.
Write-Step "Downloading prinstall.exe"
$tempFile = Join-Path $env:TEMP ("prinstall-{0}.exe" -f ([guid]::NewGuid().Guid))
try {
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile -UseBasicParsing
} catch {
    throw "Download failed: $($_.Exception.Message)"
}
$sizeMb = [math]::Round((Get-Item $tempFile).Length / 1MB, 1)
Write-Ok "$sizeMb MB"

# Install
$finalPath = Join-Path $installDir 'prinstall.exe'
Write-Step "Installing to $installDir"
try {
    Move-Item -Path $tempFile -Destination $finalPath -Force
} catch {
    Remove-Item $tempFile -ErrorAction SilentlyContinue
    throw "Could not write $finalPath. Close any running prinstall.exe and try again. Underlying error: $($_.Exception.Message)"
}
Write-Ok "Installed"

# Add to PATH if missing
Write-Step "Updating $pathScope PATH"
$currentPath = [Environment]::GetEnvironmentVariable('Path', $pathScope)
$entries     = @()
if ($currentPath) { $entries = $currentPath -split ';' | Where-Object { $_ -ne '' } }
if ($entries -notcontains $installDir) {
    $newPath = (($entries + $installDir) -join ';')
    [Environment]::SetEnvironmentVariable('Path', $newPath, $pathScope)
    # Also update the current session so the user can run prinstall right now.
    $env:Path = "$env:Path;$installDir"
    $pathChanged = $true
    Write-Ok "Added $installDir"
} else {
    $pathChanged = $false
    Write-Ok "Already on PATH"
}

Write-Host ""
Write-Host "prinstall $tag installed." -ForegroundColor Green
Write-Host "  Location: $finalPath"
Write-Host ""
if ($pathChanged) {
    Write-Host "Open a new PowerShell session to pick up the PATH change, then run:" -ForegroundColor Yellow
} else {
    Write-Host "Run:"
}
Write-Host "  prinstall --help"

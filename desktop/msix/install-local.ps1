<#
.SYNOPSIS
  Installs the Store version of Beam on this PC, before it's in the Store.

.DESCRIPTION
  Copies the package layout that build-msix.ps1 made into a permanent
  folder (%LOCALAPPDATA%\Programs\Beam), then registers it from there. Once
  it's in that folder, rebuilding or cleaning the build folder can't break
  the install, and the next run of this script updates it in place.

  Registering an unsigned layout needs Developer Mode (Settings → System →
  For developers). People who aren't developers get Beam from the Store.

  Beam's identity, trusted devices and settings live in AppData, outside the
  package, so they carry over from the .msi version and survive updates.

.EXAMPLE
  ./build-msix.ps1 -TargetDir D:/beam-target
  ./install-local.ps1 -TargetDir D:/beam-target
#>
param(
    # Where cargo puts release builds (the layout is in <TargetDir>/msix/layout).
    [string]$TargetDir = ""
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent (Split-Path -Parent $here)
if (-not $TargetDir) {
    $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo "target" }
}
$layout = Join-Path $TargetDir "msix/layout"
if (-not (Test-Path (Join-Path $layout "AppxManifest.xml"))) {
    throw "No package layout at $layout. Run build-msix.ps1 first."
}

$devMode = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock" -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
if ($devMode -ne 1) {
    throw "Developer Mode is off. Turn it on in Settings → System → For developers, or install Beam from the Store."
}

$manifest = [xml](Get-Content (Join-Path $layout "AppxManifest.xml") -Raw)
$name = $manifest.Package.Identity.Name
$installDir = Join-Path $env:LOCALAPPDATA "Programs\Beam"

# Beam must not be running while its files are replaced.
Get-Process Beam, beam-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1

# A registration from another folder (for example the build folder) would
# keep pointing there: remove it first. Settings and trust are in AppData,
# so nothing of the user's is lost.
$existing = Get-AppxPackage $name
if ($existing -and $existing.InstallLocation -ne $installDir) {
    Remove-AppxPackage $existing.PackageFullName
}

New-Item -ItemType Directory -Force $installDir | Out-Null
robocopy $layout $installDir /MIR /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Copying the package to $installDir failed (robocopy $LASTEXITCODE)." }

Add-AppxPackage -Register (Join-Path $installDir "AppxManifest.xml") -ForceApplicationShutdown
$package = Get-AppxPackage $name
Write-Output "Installed $($package.Name) $($package.Version) from $installDir"

# Start it, so it's in the tray and takes over its startup task.
Start-Process "shell:AppsFolder\$($package.PackageFamilyName)!Beam"

# Get-Process above reports "not found" as an error record; the install itself succeeded.
exit 0

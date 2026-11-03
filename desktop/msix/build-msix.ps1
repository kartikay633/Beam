<#
.SYNOPSIS
  Packages the release build of Beam as an MSIX for the Microsoft Store.

.DESCRIPTION
  Run after `npm run tauri build` (which produces the release exe). The
  Store signs the package itself, so this doesn't sign it. To install it
  locally for testing, sign it with a test certificate first (see
  desktop/msix/README.md).

.EXAMPLE
  ./build-msix.ps1 -IdentityName "12345Kartikay.Beam" -Publisher "CN=ABCD-1234" -PublisherDisplayName "Kartikay"
#>
param(
    # Partner Center → Product identity → Package/Identity/Name.
    [string]$IdentityName = "Beam.Dev",
    # Partner Center → Product identity → Package/Identity/Publisher.
    [string]$Publisher = "CN=Beam Dev",
    [string]$PublisherDisplayName = "Beam",
    # Defaults to tauri.conf.json's version plus ".0" (MSIX needs four parts).
    [string]$Version = "",
    # Where cargo puts release builds.
    [string]$TargetDir = "",
    # Also install the unpacked layout on this PC (needs Developer Mode).
    [switch]$Register
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$desktop = Split-Path -Parent $here
$repo = Split-Path -Parent $desktop

if (-not $Version) {
    $conf = Get-Content (Join-Path $desktop "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    $Version = "$($conf.version).0"
}
if (-not $TargetDir) {
    $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo "target" }
}
$exe = Join-Path $TargetDir "release/beam-desktop.exe"
if (-not (Test-Path $exe)) { throw "No release build at $exe. Run 'npm run tauri build' first." }
$wakeTask = Join-Path $TargetDir "release/beam_wake_task.dll"
if (-not (Test-Path $wakeTask)) { throw "No wake-up task at $wakeTask. Run 'cargo build --release -p beam-wake-task' first." }

$sdk = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin/*/x64/makeappx.exe" |
    Sort-Object { [version]$_.Directory.Parent.Name } | Select-Object -Last 1
if (-not $sdk) { throw "makeappx.exe not found: install the Windows 10/11 SDK." }

$out = Join-Path $TargetDir "msix"
$layout = Join-Path $out "layout"
Remove-Item -Recurse -Force $layout -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $layout "Assets") | Out-Null

Copy-Item $exe (Join-Path $layout "Beam.exe")
Copy-Item $wakeTask (Join-Path $layout "beam_wake_task.dll")
# Every size and form of the icons (msix/Assets). The "unplated" ones let
# the taskbar and Start show the icon on its own; without them Windows puts
# it on a plate in the accent colour, which shows at the rounded corners.
Copy-Item (Join-Path $here "Assets/*") (Join-Path $layout "Assets")

$manifest = (Get-Content (Join-Path $here "AppxManifest.xml") -Raw).
    Replace("{{IDENTITY_NAME}}", $IdentityName).
    Replace("{{PUBLISHER}}", $Publisher).
    Replace("{{PUBLISHER_DISPLAY_NAME}}", $PublisherDisplayName).
    Replace("{{VERSION}}", $Version)
Set-Content -Path (Join-Path $layout "AppxManifest.xml") -Value $manifest -Encoding utf8

# resources.pri indexes those variants; without it Windows only sees the
# exact file names in the manifest.
$makepri = Join-Path $sdk.DirectoryName "makepri.exe"
$priConfig = Join-Path $out "priconfig.xml"
& $makepri createconfig /cf $priConfig /dq en-US /pv 10.0.0 /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makepri createconfig failed ($LASTEXITCODE)" }
# One resources.pri for one package: no per-scale split (that's for bundles).
$config = [xml](Get-Content $priConfig -Raw)
$config.SelectNodes("//packaging") | ForEach-Object { $_.ParentNode.RemoveChild($_) | Out-Null }
$config.Save($priConfig)
Remove-Item (Join-Path $layout "resources*.pri") -ErrorAction SilentlyContinue
& $makepri new /pr $layout /cf $priConfig /mn (Join-Path $layout "AppxManifest.xml") /of (Join-Path $layout "resources.pri") /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makepri new failed ($LASTEXITCODE)" }

# -Register installs the layout folder directly (Developer Mode, no signing),
# for testing on this PC.
if ($Register) {
    Add-AppxPackage -Register (Join-Path $layout "AppxManifest.xml") -ForceApplicationShutdown
    Write-Output "Registered $layout"
}

$package = Join-Path $out "Beam_${Version}_x64.msix"
& $sdk.FullName pack /o /d $layout /p $package
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)" }
Write-Output "Packaged $package"

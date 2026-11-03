param(
    [string]$AdbPath = 'adb',
    [string]$Serial,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
if (-not $Serial) {
    $connected = @(& $AdbPath devices | ForEach-Object {
        if ($_ -match '^(\S+)\s+device$') { $Matches[1] }
    })
    if ($connected.Count -ne 1) {
        throw 'Connect one authorized USB-debugging device, or pass -Serial explicitly. Keep USB tethering off.'
    }
    $Serial = $connected[0]
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
function Save-Redacted([string]$Name, [string[]]$Lines) {
    $Lines | ForEach-Object {
        $_ -replace '(?i)((?:passphrase|password|psk)\s*[:=]\s*)(?:"[^"]*"|\S+)', '$1[redacted]'
    } | Set-Content -LiteralPath (Join-Path $OutputDirectory $Name) -Encoding utf8
}

# Read-only snapshots: no clearing logs, changing radios, pairing or app control.
$directLogs = & $AdbPath -s $Serial logcat -d -v threadtime `
    'BeamDirect:V' 'BeamBle:V' 'WifiP2pService:V' 'WifiP2pServiceImpl:V' `
    'WifiP2pNative:V' 'SupplicantP2pIfaceHal:V' 'SupplicantP2pIfaceHalHidlImpl:V' 'wpa_supplicant:V' '*:S' 2>&1
if ($LASTEXITCODE -ne 0) { throw 'Could not read Android logcat. Authorize USB debugging on the phone.' }
Save-Redacted 'android-direct.log' $directLogs

$p2p = & $AdbPath -s $Serial shell dumpsys wifip2p 2>&1
if ($LASTEXITCODE -eq 0) { Save-Redacted 'android-p2p-state.log' $p2p }

$windowsLog = Join-Path $env:LOCALAPPDATA 'Beam/offline.log'
if (Test-Path -LiteralPath $windowsLog) {
    Save-Redacted 'windows-offline.log' (Get-Content -LiteralPath $windowsLog -Tail 250)
}

$interface = & netsh wlan show interfaces
Save-Redacted 'windows-wifi-state.log' @($interface | Select-String -Pattern '^\s+(State|Band|Channel)\s+:' | ForEach-Object { $_.Line })
Write-Output "Saved redacted Direct diagnostics to $OutputDirectory"

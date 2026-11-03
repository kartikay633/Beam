# Beam Windows 0.1.2 candidate — preserve the existing Wi-Fi

Prepared locally on 2026-11-02. Compatible with Android Beam 0.1.1;
the Android APK, core protocol and stored identities are unchanged.

## Report and evidence

The user reported 1–2 minute setup and Windows switching from the existing
college Wi-Fi to `AndroidShare…`. Windows WLAN events confirmed this.
One attempt had a hotspot reply after 8 seconds, followed by 18 more seconds
before Windows joined it. Transfers were using hotspot fallback, not proven
Wi-Fi Direct. The user also confirmed the phone's Location switch was off.

Android's [Wi-Fi Direct documentation](https://developer.android.com/develop/connectivity/wifi/wifi-direct)
requires Location mode for discovery. This explains a prerequisite missing
from the tested Direct route; it does not prove turning it on will resolve
all driver or pairing issues.

## Changes

- Windows offers reachable LAN and laptop-hosted Wi-Fi Direct only.
- Removed Android-hotspot joining and delayed Wi-Fi disconnection/restoration
  from production Windows code. Unsolicited hotspot replies are rejected.
- If Direct cannot connect, the transfer fails instead of switching SSIDs.
  Hardware must support concurrent infrastructure Wi-Fi and Wi-Fi Direct;
  the app cannot promise uninterrupted internet on every driver.
- Replaced three fresh Bluetooth characteristic-discovery requests with one,
  selecting the required characteristics by UUID. Missing services still fail.
  This follows Microsoft's [GATT discovery API](https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.genericattributeprofile.gattdeviceservice.getcharacteristicsasync).
- Added elapsed timings for Bluetooth device opening, Direct offer creation,
  service/characteristic discovery, peer info and link negotiation.

## Verification and use

Policy and UUID-selection regressions were observed failing before the fix
and passing afterward. Final checks: 149 Rust tests passed (10 hardware/
network tests ignored by default), workspace Clippy passed with warnings
denied, formatting/whitespace checks passed, and the UI production build
and Windows MSI packaging passed. An independent focused review found no
critical or important issue. No UI behavior or Android source changed in
this update. See local verification logs under
`D:/beam-repair-backup/2026-11-02` and the implementation plan in
`docs/superpowers/plans/2026-11-02-preserve-wifi.md`.

Quit the existing Windows tray app and update using the 0.1.2 MSI. Keep the
current Android 0.1.1 APK. Turn on Android Location, Wi-Fi and Bluetooth;
grant Nearby devices permission. Preserve the PC's current Wi-Fi and test
both transfer directions. Confirm the PC's SSID and internet continue
working and time from Send to the acceptance prompt. Repeat immediately.

The ten-second target and physical Wi-Fi Direct interoperability are still
unverified. No public release, installation, signing-key replacement or
forced network change was performed while building this candidate.

Local installer and SHA-256 checksums: `D:/beam-releases/0.1.2`.

## Physical follow-up

After enabling Location, the user reported Android asking to temporarily
disconnect its Wi-Fi to connect to KARTIKAY. This matches Android's
frequency-conflict consent flow; it is not the removed Windows hotspot
join. The PC was observed still connected to its existing infrastructure
Wi-Fi. Stage logs show approximately 3.9 s to send the Bluetooth request,
then 34 s waiting for Direct setup before the phone declined the link.
A later Direct request also timed out during pairing. Successful physical
Direct transfer and simultaneous preservation of both devices' Wi-Fi are
therefore still unverified. Do not assume all drivers support the required
concurrent channels or bypass the system disconnection prompt.

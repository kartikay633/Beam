# Beam Windows 0.1.3 — negotiated Direct candidate

Prepared locally on 2026-11-02. Compatible with Android Beam 0.1.1;
no Android APK, stored identity, trust or wire-protocol changes are required.

## What is established

The phone is not associated with an infrastructure Wi-Fi network. Its
temporary-disconnection prompt therefore does not prove a conflict between
two active Wi-Fi channels. The user accepted the prompt promptly on 0.1.2,
but Direct setup still failed. PC logs show the Bluetooth request sent in
2–4 seconds, then approximately 34 seconds waiting for Direct. A late
Direct request was accepted after the transfer had already failed and
Windows then waited another 25 seconds for pairing.

The PC was observed associated with its college Wi-Fi on channel 52 and
later channel 44. This does not establish the phone's Direct channel.
Driver concurrency counts reported unavailable/zero and are inconclusive.
No ADB phone is attached, so phone supplicant diagnostics are unavailable.

## Changes

- Advertise normally and negotiate the Direct group before creating it,
  rather than pre-forming a fixed-channel autonomous group. Prefer Windows
  as owner with intent 14, the documented default; Android's existing
  intent 0 remains the client. This permits channel negotiation, but does
  not force a channel or add unsupported concurrent-radio capability.
- Close unsolicited/late requests when no transfer owns a Direct lease,
  even if the earlier acceptance timestamp has not expired.
- Cancel pending pairing/connection operations when the last transfer
  releases its lease. Bind leases and retained devices to their publisher
  generation so an old request cannot attach to a replacement publisher.
- Pin each request to its transfer-session generation as well. A quick
  retry on the same publisher cannot adopt an old pairing result, even if
  that result becomes ready at the same time as cancellation.
- Retain LAN/Direct-only behavior: no AndroidShare joining, SSID switching,
  forgotten pairings, hidden Android APIs or bypassed consent prompts.

Microsoft documents the publisher modes in its [Wi-Fi Direct sample](https://github.com/microsoft/Windows-universal-samples/blob/main/Samples/WiFiDirect/cs/Scenario1_Advertiser.xaml.cs)
and owner preference in [GroupOwnerIntent](https://learn.microsoft.com/en-us/uwp/api/windows.devices.wifidirect.wifidirectconnectionparameters.groupownerintent).

## Proof and outstanding test

Admission and pending-cancellation regressions failed before implementation
and passed after it. The negotiated publisher test was also observed failing
under the autonomous configuration, then passing with negotiation. The
explicit PC hardware test started discovery without a peer, verified the
publisher started without an autonomous group, and verified the connected
Wi-Fi profiles stayed the same; the PC remained on its existing Wi-Fi.
It does not prove a physical phone transfer or concurrent operation with
two infrastructure associations.

Independent scoped review found a fast-retry cancellation race. A regression
reproduced it and passed after session-generation checks were added at all
three request checkpoints. Desktop tests: 16 passed, 9 hardware checks
ignored by default. Phone transfer and ten-second setup are still unverified.

Final verification: 152 workspace Rust tests passed, 11 hardware/network
tests ignored by default; workspace Clippy passed with warnings denied;
formatting and whitespace checks passed; production UI build and final
Windows MSI packaging passed. The explicit discovery-only PC hardware
check also passed separately. The installer was not installed automatically.

Quit the Windows tray app and install 0.1.3. Keep Android 0.1.1. Turn on
Wi-Fi, Bluetooth and Android Location. Test a small file in each direction,
approving the OS connection prompt promptly. If the phone is currently
unassociated, accepting that prompt cannot remove an existing phone Wi-Fi
association; do not assume the same behavior when it is associated.
Confirm the PC's existing SSID/internet and time from Send to acceptance.

This remains an experimental connection candidate until the user's phone
transfer succeeds. No ten-second or universal concurrent-Wi-Fi guarantee.
No installation, public release, commit or push was performed during build.
Local package: `D:/beam-releases/0.1.3`; logs:
`D:/beam-repair-backup/2026-11-02/direct-*`.

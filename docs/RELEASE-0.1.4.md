# Beam Windows 0.1.4 / Android 0.1.2 — connection lifetime repair

Prepared 2026-11-02 on `repair-direct-sharing`. Android versionCode 3;
shared Rust core and wire protocol remain 0.1.1 / protocol v1.

## Physical evidence

With Android Location enabled, the phone discovered the PC and negotiated
5220 MHz as a Direct client. Failed attempts reached WPS but received M2D
and group-formation failure while Windows waited for pairing. This is not
evidence that the hardware cannot establish Direct. The phone was not
associated with an infrastructure Wi-Fi network during these tests.

An instrumented Windows 0.1.3 attempt completed pairing and established
Wi-Fi Direct. A 3,736,428-byte video arrived on the PC. SHA-256 computed
independently on the phone and PC matched. The PC remained on its existing
5 GHz Wi-Fi. Later attempts failed, so that one success was not sufficient
to claim reliability. USB was used for diagnostics and Android installation;
the verified video transfer used the app's Wi-Fi Direct path.

## Changes

- Retain negotiated group formation and LAN/Direct-only routing; no
  automatic AndroidShare joining or SSID switching.
- Reap receive leases after the native Direct clients disconnect and no
  receive remains active. Retire unused publishers, explicitly closing old
  device handles before fresh group negotiation. The previous 90-second
  setup grace could leave an inactive WPS group available to the next call.
- Serialize inbound setup across BLE addresses. Rotating addresses cannot
  create overlapping negotiations. A completed setup has a two-second
  quiet period rather than a 30-second same-address retry delay.
- Keep the Bluetooth session alive from initial service discovery through
  response notification cleanup. Log actual discovery status instead of
  claiming an unreachable phone is not running Beam.
- Android cleanup is pinned to its connection generation. A delayed old
  timer closes its old channel without cancelling/removing a newer group.
- Keep publisher/session generation checks and last-lease cancellation for
  late Windows pairing results. Add Windows ceremony logging for diagnosis.

## Verification

- Rust workspace: 152 tests passed; 11 hardware/network tests ignored by
  default. Workspace Clippy passed with warnings denied.
- Android: 47 JVM tests passed; debug/release APKs, instrumented-test
  compilation and debug/release lint passed for all three ABIs. The new
  APK retains the existing shared test signing key and updates in place.
- Calls and Android delayed-cleanup regressions were observed failing
  before their fixes and passing afterward.
- UI production build and Windows MSI packaging passed. No UI behavior
  changed in this update. Independent focused review found no critical
  or important issue in the connection-lifetime changes.
- Instrumented Android tests were compiled, not run. Production signing
  remains outstanding. Ten-second setup and final repeat/bidirectional
  acceptance must be measured on the updated builds.

## Artifacts and test state

Installable candidates and SHA-256 checksums: `D:/beam-releases/0.1.4`.
Android 0.1.2 was installed over 0.1.1 with `adb install -r`; identity/trust
were preserved. The PC test process runs the 0.1.4 diagnostic executable
from the existing allowed development path. The MSI still needs normal
installation for the permanent Windows update.

Local diagnostics: `D:/beam-repair-backup/2026-11-02`. Collection script:
`scripts/collect-direct-diagnostics.ps1`; it reads relevant logcat/P2P state
and masks password fields. No radios, network profiles or pairing records
are changed by the collector. The app was restarted during diagnosis.

No public release, commit/push, key replacement or blanket firewall change
was performed. Keep Wi-Fi, Bluetooth and Android Location on for offline
tests. OS consent prompts may still be required.

# Beam Windows 0.1.6 / Android 0.1.4

Local repair builds prepared and installed on 2026-11-03. Android
versionCode 5; Rust core/FFI remain 0.1.1 and protocol v1.

## Changes

- Windows offers reachable LAN or negotiated Wi-Fi Direct. It never joins
  AndroidShare or disconnects/restores the PC's infrastructure Wi-Fi.
- Prefer the Wi-Fi Direct Invitation pairing procedure, with push-button
  configuration and ConfirmOnly consent handling. This lets the accepting
  side select the method. It is a preference, not a forced protocol or a
  proven explanation for every earlier driver failure.
- Retain established Android Direct groups for 60 seconds after their last
  transfer lease. Serialize setup, validate before reuse, close on stop,
  and prevent old timers from removing a newer or active connection.
- Reuse connected Windows groups when the sending lease has ended.
  Detect when LAN probing actually uses the managed P2P interface, and
  retain Direct ownership on both devices. Native transfer activity also
  protects an address rediscovered through mDNS from idle teardown.
- Keep previous call serialization, generation-bound cancellation and
  Bluetooth-session lifetime fixes.
- The Windows legacy credential experiment is disabled: this POCO's P2P
  client received association denial status 12 before Windows delivered
  any application connection request. Optional encrypted wire fields stay
  backward compatible; current Windows offers no group passphrase.

## Physical verification

Test hardware: Windows laptop with Intel AX201 and Android 13 POCO X4 Pro
5G. The phone's ordinary Wi-Fi client was disconnected from any router;
Location, Wi-Fi and Bluetooth were enabled. USB tethering remained off.
USB was used for installation, logs and hash verification, not file data.

- Phone → PC: three 65,536-byte proof files arrived with SHA-256 matching
  their deterministic source bytes. Fresh connections completed in about
  11.3 and 18.4 seconds. A consecutive Direct handshake took 4.75 seconds
  and did not pair again. USB disconnected before its final JUnit report;
  file arrival/integrity is established independently.
- PC → phone: two consecutive 1,048,576-byte files completed, confirmed by
  the user. Both saved copies matched the PC source SHA-256. The second
  used the established network without a new Bluetooth/Direct handshake
  and was reported faster. The fresh reverse setup took about 25.5 seconds,
  including operating-system pairing/approval time.
- The PC remained associated with college Wi-Fi in sampled observations. Its AP
  and channel roamed between samples, so channel was not a controlled test
  variable. The phone's infrastructure client remained disconnected.
- After installation, the user confirmed another normal phone → PC send
  completed; logs show its Bluetooth handshake selecting an existing LAN
  route in 3.15 seconds. By then the user had switched the PC to the phone's
  personal hotspot because college Wi-Fi had closed for the night.
- Final cold-Direct automation on the installed version could not establish
  its Bluetooth-only precondition on that already-shared network. This
  must not be reported as a fresh installed-build Direct pass. The harness
  now skips when the target is already found over LAN. Earlier ambiguous
  automation results did not establish a release-only runtime defect;
  temporary startup instrumentation was removed after the normal installed
  app's Bluetooth/LAN transfer succeeded.

These results do not guarantee a ten-second first connection. Preserving
an already-associated Android Wi-Fi connection still depends on that
phone's radio/driver concurrency and was not verified in this test.
Repeat a cold Direct check on the installed build when the PC again has
independent Wi-Fi and the phone's personal hotspot can be turned off
without removing the PC's internet connection.

## Software verification

- Rust workspace: 154 passed; 11 hardware/network checks ignored by
  default. Final desktop pairing build: 17 passed, 9 ignored. Workspace
  Clippy passed with warnings denied.
- Android: 51 JVM tests passed; debug and release builds, test APK
  compilation, and debug/release lint passed. Final packaging also passed.
- Retention, managed-route ownership, and Windows idle reuse regressions
  were observed failing before their fixes. Focused independent review
  found and resolved two integration defects; final review found no
  remaining critical or important finding in this scope.
- Production UI build and MSI packaging passed. MSI ProductVersion was
  read back as 0.1.6. Standard elevated upgrade completed with exit code 0;
  Program Files executable and Send To shortcut point to the new version.
- Android updated in place with the existing shared test signer; stored
  identity and trusted devices were preserved. MIUI rejected streamed
  updates; standard `adb install --no-streaming -r -t` succeeded without
  disabling verification or changing security settings.

## Local artifacts and release boundary

Packages and SHA-256 checksums are in `D:/beam-releases/0.1.6`:

- `Beam-0.1.6-Windows-x64.msi`
- `Beam-0.1.4-Android-test.apk` — installable shared test signer
- `Beam-0.1.4-Android-release-unsigned.apk` — production signing required
- `SHA256SUMS.txt`

These are local builds; no public release, commit or push was performed.
Windows code signing and an externally managed Android production signing
key remain prerequisites for a public production release. Do not publish
the shared debug signer as a production key.

Keep Wi-Fi and Bluetooth on, and enable Android Location for new Direct
discovery. Internet, a shared router and USB tethering are unnecessary.
Operating-system approval may still be required on a fresh connection.

Diagnostics and build logs: `D:/beam-repair-backup/2026-11-02` (the ongoing
investigation directory). Collector: `scripts/collect-direct-diagnostics.ps1`.

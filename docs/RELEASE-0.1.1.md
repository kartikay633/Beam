# Beam 0.1.1 repair candidate

Prepared locally on 2026-10-31 on `repair-direct-sharing`. Android
versionCode is 2. No commit, push, public release, or installation was
performed. Existing identity/trust data and the Android test signing key
were preserved. Pre-existing local changes were backed up before repair.

## What changed

- Restored the desktop Share, Devices and Activity screens, pairing retry,
  visibility controls, settings drafts, file drops and listener cleanup.
- Repaired Windows Wi-Fi Direct API thread usage and pairing result
  handling; bounded negotiation, retained hotspot fallback and prevented
  overlapping calls. Refreshes recover after Bluetooth changes.
- Removed Android's preliminary P2P group/Parcel workaround. Direct peer
  discovery uses public PBC connection APIs and does not wait for an
  unreliable group-owner flag. Actual TCP probes choose a usable LAN.
- Fixed link cleanup, stale replies and cancellation during setup. Rust
  send tasks stop when their FFI caller is cancelled. Unsent Android
  descriptors are closed, and fallback submits files only once.
- Added pairing over a newly negotiated private address while preserving
  the QR's pinned identity and one-time token.

## Local verification

| Check | Result |
|---|---|
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `cargo test --workspace --locked` | 147 passed, 10 hardware/network tests ignored by default |
| Explicit real mDNS discovery/transfer tests | 2 passed on this PC |
| Explicit Windows Bluetooth publisher test | Passed on this PC; does not prove phone interoperability |
| Android `testDebugUnitTest` | 46 passed |
| Android debug/release APK and instrumented-test compilation | Passed for arm64-v8a, armeabi-v7a, x86_64 |
| Android `lintDebug lintRelease` | Passed, 0 errors; 5 warnings per variant |
| Desktop Playwright workflows | 5 passed |
| Desktop typecheck and production UI build | Passed |
| Windows MSI packaging | Passed |
| Strict premium UI audit | 0 findings |

Android warnings concern API-gated permission attributes, an available
Gradle plugin update, backup compatibility metadata and a redundant SDK
condition. Instrumented tests were compiled, not run. Browser workflows
use a mocked Tauri bridge; Rust and FFI tests exercise actual TCP
transfers, encryption, pairing, resume and cancellation separately.

Logs are saved locally in `D:/beam-repair-backup/2026-10-31`. Regression
tests for negotiation, descriptor reuse, link ownership, overlapping calls
and cancellation were observed failing before their corresponding fixes.
An independent read-only review identified additional lifecycle defects;
those findings were addressed before the final checks.

## Artifacts and signing

Local deliverables are in `D:/beam-releases/0.1.1` with SHA-256 checksums:

- `Beam_0.1.1_x64_en-US.msi`: Windows test installer, unsigned.
- `Beam-0.1.1-test.apk`: installable Android debug APK using the existing
  shared test key. This key is not suitable for production distribution.
- `Beam-0.1.1-release-unsigned.apk`: release APK awaiting production signing.

Production Android builds take signing credentials from
`BEAM_ANDROID_KEYSTORE`, `BEAM_ANDROID_STORE_PASSWORD`,
`BEAM_ANDROID_KEY_ALIAS`, and `BEAM_ANDROID_KEY_PASSWORD`. Keep the key
outside the repository. Windows distribution also needs the intended
code-signing/Store identity. Signing keys were not created or replaced.

## Required physical-device release checks

The user requested continuation without the phone. The repaired build has
not been installed on the POCO, and physical Wi-Fi Direct/fallback
transfers are unverified. Do not label it production-ready until these
checks pass with the intended signed builds:

1. Keep USB tethering off. Turn Wi-Fi and Bluetooth on; Beam uses BLE for
   offline discovery/control and Wi-Fi for file data. No shared router or
   internet connection is required. Approve OS connection prompts.
2. Make both unpaired devices visible to everyone. Confirm each appears
   in the other app. After ten minutes, confirm trusted-only behavior and
   repeat discovery after toggling Bluetooth.
3. With the devices on different networks, send in both directions over
   laptop-hosted Wi-Fi Direct. Verify received file hashes, filenames,
   multi-file batches, a large file and Android `Download/Beam` output.
4. Force a Direct failure; verify the Android local-only hotspot fallback
   transfers in both directions and releases/restores the network after
   completion. Record the actual route in logs.
5. Cancel during discovery, Direct setup, hotspot setup and transfer.
   Retry repeatedly; confirm no abandoned group, hotspot or stale offer.
6. Interrupt/recover a transfer, pair by QR with stale LAN addresses,
   restart both apps, and verify trust/auto-accept remains intact.
7. Install as a fresh Windows user and as an upgrade, verify firewall and
   background reception, then run Android on-device instrumentation.

This candidate includes completed software repairs and build artifacts;
hardware acceptance and production signing remain outstanding.

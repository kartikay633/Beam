# Beam Windows 0.1.7 / Android 0.1.5

Offline audit and repair pass, 2026-11-03. Android versionCode 6. Core/FFI
version remains 0.1.1, protocol v1; no wire or trust policy changes.

## Repairs

- A fully checkpointed resume can request a repair stream if the receiver's
  final disk hash fails. Previously there were no workers to process the
  retry, leaving both devices connected but the transfer stuck indefinitely.
- Android delayed Direct Share sends only its original selection. Selecting
  other files while waiting cannot send them to the earlier recipient;
  removing a selection invalidates it for that pending request.
- File-open failure merges the failed selection with newly staged files.
  Cancellation during provider access closes unsubmitted descriptors and
  removes temporary copies, including cancellation at the IO return boundary.
- A share-sheet ranking failure cannot restage files already submitted to the
  sender or misreport the successful handoff as a file-read failure.
- Failed Android startup closes the created native node even before it was
  published, stops discovery/radio adapters, clears stale running state and
  permits a retry. Cancellation remains cancellation.
- Windows download folder changes now explicitly apply after restart, like
  device-name changes. Saving settings preserves the running listener and its
  advertised port, active routes and cancellation handles. Activity and
  folder-opening actions use the actual live receiver folder.
- Windows settings commit only after OS integration changes succeed. A
  failed startup/Send To update or settings write restores attempted changes;
  restoration failures are reported rather than hidden.
- Desktop commands report failures inline and prevent duplicate submissions.
  Receive prompts recover from load failures and stop accepting expired
  requests. Visibility supports arrow keys, Home/End, Escape and focus return.
- The initial desktop snapshot is fetched after event subscription, closing
  a startup race that could leave stale state on screen indefinitely.

## Verification

- Focused regressions were observed failing before fixes for selection scope,
  file restoration, cancellation cleanup, failed startup, corrupt resume,
  settings rollback, duplicate send, silent action failures, offer loading,
  keyboard navigation and subscription ordering.
- Rust workspace: 155 passed / 11 hardware or network tests ignored, followed
  by 19 desktop tests passed / 9 ignored after adding two settings regressions
  (157 distinct passing Rust tests in the final source). Workspace Clippy with
  warnings denied passed. Formatting and whitespace checks passed.
- Desktop: 14 browser workflows passed; TypeScript and production UI build
  passed. Light/narrow/error and dark/settings/menu screenshots inspected;
  reduced-motion behavior checked. Strict UI contract audit found no violations.
- Windows production MSI built with the Tauri CLI; MSI and bundled executable
  versions read back as 0.1.7.
- Android: 58 JVM tests passed; debug and release APKs, instrumented-test APK
  compilation, and debug/release lint passed. Native libraries rebuilt for
  arm64-v8a, armeabi-v7a and x86_64. Both app manifests report 0.1.5 / code 6;
  the installable test APK retains the existing shared test signer.

Logs: `D:/beam-repair-backup/2026-11-03-polish`.

Local packages and checksums: `D:/beam-releases/0.1.7`:

- `Beam-0.1.7-Windows-x64.msi`
- `Beam-0.1.5-Android-test.apk` (installable upgrade with shared test signer)
- `Beam-0.1.5-Android-release-unsigned.apk` (requires production signing)
- `SHA256SUMS.txt`

## Installation and limits

These are local builds, not a public release. No app was installed or restarted
on the user's devices during this pass. No phone, ADB, network reconfiguration,
screen control, commit or push was used.

Install the new MSI and test APK as upgrades when ready; do not uninstall the
existing apps, which would remove device identity/trust. The Android test APK
uses the existing shared test signer. The release APK requires an externally
managed production signing key; Windows production code signing is also pending.

No new physical Wi-Fi Direct or Android UI test was possible with the phone
disconnected. Previous hardware measurements and their topology limitations
are historical evidence in [0.1.6](RELEASE-0.1.6.md), not acceptance of these new
packages. First-connection latency and phone Wi-Fi coexistence still need a
device check. This audit does not establish that no other bugs exist.

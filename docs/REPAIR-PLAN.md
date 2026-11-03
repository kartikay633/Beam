# Direct sharing repair plan

Goal: reliable Android ↔ Windows transfers on an existing LAN or a private Wi-Fi link, with actionable failures and buildable release artifacts.

Keep the shared Rust/UniFFI engine, Noise encryption, acceptance/trust rules, ten-minute public visibility, and existing visual identity. Bluetooth LE remains the offline discovery/control channel; file bytes use Wi-Fi. Preserve the pre-existing local changes (backed up outside the repository) and work on `repair-direct-sharing`.

## Work and proof

- [x] Inspect connected POCO and Windows logs for the missing-laptop/failed-connection report and record baseline failures.
- [x] Restore incomplete desktop screens/imports, preserve pairing/trust settings, complete existing navigation/radar styles, and verify production build and browser interaction.
- [x] Repair offline negotiation: probe reachability, retain hotspot fallback, bound setup, handle cancellation/cleanup, use the required Windows UI context, and add policy regressions.
- [x] Repair Android descriptor ownership across fallback and link setup cancellation; cover cleanup and FFI task lifetime with regressions.
- [x] Build Android debug/release APKs and the Windows installer without replacing signing keys or touching installed identity/trust data.
- [ ] Install on the physical phone and verify both transfer directions, received content hashes, repeated sends and actual Direct/fallback routes. Deferred because the user requested continuation without the phone.
- [x] Run formatting, Clippy, Rust tests, Android JVM tests, instrumented-test compilation, lint, browser tests and packaging. Review independently and document deployment requirements and unverified hardware behavior.
- [ ] Run Android instrumented tests on-device and production signing/installation acceptance checks.

## Review focus

Blocked campus LAN; different networks sharing the same address prefix; unknown peers while public visibility has expired; cancellation during radio setup; repeated connections with stale P2P groups; file descriptors consumed on a failed first attempt; files copied to Android Downloads after transfer completion.

## Evidence ledger

- Baseline: desktop typecheck fails with missing `Devices`, `Transfers`, `FileIcon`, and `UploadIcon` exports/imports.
- Real device: POCO X4 Pro 5G is connected through ADB with required runtime permissions already granted.
- Existing Windows logs repeatedly report pairing result 19 and unsuccessful Wi-Fi Direct connection creation. Android logs show discovery waiting for `isGroupOwner` while the matching laptop reports false.
- Microsoft Wi-Fi Direct sample explicitly requires `WiFiDirectDevice.FromIdAsync` to be called on the UI thread; current app calls it on a Tokio worker.
- Decision: fix within the existing architecture; no product redesign or unrelated features. No commits, publishing, or signing-key replacement as part of repair.
- Desktop build restored; five Playwright workflows pass, including navigation, pairing failure/retry, preserving settings drafts across snapshots, Escape, and narrow layouts. Rendered desktop/narrow screenshots were inspected.
- New negotiation regression tests were observed failing before implementation, then passing. The descriptor-reuse regression was reproduced as two submissions of the same descriptor; resolution now happens before transfer ownership.
- Independent review found abandoned Android call acquisitions, overlapping Windows negotiations after 30 seconds, hidden-phone send rejection, wrong-hello cleanup, and a late drag-listener leak. All were addressed. New LinkLease and call-tracker tests were observed failing before their fixes.
- QR pairing now supports the negotiated private address while retaining the original QR's pinned key and one-time token. A real loopback FFI test proves both successful pairing and wrong-key rejection.
- Version is 0.1.1 (Android versionCode 2). Release signing is configurable using external environment variables; no production signing key was generated or substituted.
- Phone was disconnected by the user. New physical Wi-Fi Direct/fallback transfers and Android on-device instrumentation remain unverified; USB tethering is not used by any verification.
- Final cancellation regressions reproduced a detached FFI sender and Android pre-start cleanup failure. Send futures now abort their runtime task, native registrations clean up on drop, and Android registers setup before execution with completion cleanup.
- Final software verification: 147 Rust tests, 46 Android JVM tests, 5 browser workflows passed; Clippy, builds and Android lint passed. Ten Rust tests remain ignored by default; two real mDNS checks and the Windows Bluetooth publisher check were also run explicitly. See [release checks](RELEASE-0.1.1.md).

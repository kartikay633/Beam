# Offline repair and polish pass

User authorized bug fixes and polish with the phone disconnected. Preserve the
existing architecture, Wi-Fi preservation policy, trust rules and visual design.
No radio/network changes, deployment, commit, or installation is part of this pass.

## Scope and proof

1. Reproduce and fix Android staging races: delayed Direct Share owns only its
   selected files, failure merges staged files, cancellation releases descriptors.
2. Unwind failed/cancelled Android startup, including discovery/radio resources.
3. Reproduce a fully acknowledged resume with corrupt partial data; keep a sender
   worker available for the receiver's hash-repair requests.
4. Do not restart the desktop receiver when saving download settings. Apply folder
   changes after restart, explain this in Settings, and keep live folder actions
   pointed at the actual running receiver's folder.
5. Use one desktop async-action owner for failures and duplicate prevention;
   repair receive-offer loading/retry/expiry and keyboard visibility controls.
6. Run focused red/green regressions, full Rust/JVM/browser checks, lint and builds.
   Prepare versioned local installers/APKs and record hardware/signing limits.

Evidence is recorded in D:/beam-repair-backup/2026-11-03-polish. Existing dirty
changes are preserved. Hardware Wi-Fi Direct and Android UI/device behavior remain
untested in this pass; previous physical results are historical evidence only.

Completed: selection/lifetime and resume regressions, desktop settings and error
handling, browser keyboard/theme checks, builds and lint. Added settings rollback
and advisory shortcut-ranking failure regressions during final review. See
`docs/RELEASE-0.1.7.md` for the exact verification results and artifact boundary.

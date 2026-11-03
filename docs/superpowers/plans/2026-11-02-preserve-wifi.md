# Preserve Wi-Fi and investigate setup latency

User requirements: transfers must not replace the PC's existing Wi-Fi
connection; reduce the observed one-to-two-minute setup toward ten seconds.
This continues the authorized repair on `repair-direct-sharing`.

## Design

Use reachable LAN or laptop-hosted Wi-Fi Direct only. Remove automatic
Android-hotspot joining from Windows, including unsolicited responses and
restore/disconnect tasks. Fail clearly if neither permitted route works.
This enforces the user's network requirement even when Direct fails; it
does not claim the currently failing Direct path has been repaired.

Keeping automatic hotspot fallback conflicts with the request. Adding an
opt-in network-switch setting is unnecessary for the requested behavior.
Do not promise ten-second physical setup without device measurements.

Evidence on 2026-11-02: Windows logs show hotspot responses after 8–10 s;
WLAN events show the college Wi-Fi replaced by AndroidShare. One attempt
spent a further 18 s before joining that hotspot. GATT currently discovers
three characteristics through separate uncached operations.

## Implementation and verification

1. Write Windows regressions for permitted offers and rejecting hotspot
   responses, then observe failure on the old policy.
2. Remove Windows hotspot network switching, its leases and delayed
   disconnect/restoration. Guard response selection in both directions.
3. Reduce redundant Bluetooth characteristic-discovery requests and add
   per-stage elapsed logging for physical timing diagnosis. Keep identity
   checks and operation cancellation. Verify against the WinRT API.
4. Run Rust formatting, focused tests, workspace Clippy/tests, UI build and
   package a new Windows 0.1.2 candidate. Retain Android 0.1.1 compatibility;
   do not change installed apps or the user's networks while building.
5. Record that LAN/Direct software paths pass; physical Direct connectivity
   and ten-second performance still require a test on the user's phone.

No commit, publishing, forced network changes or signing-key changes.

## Evidence ledger

- Two network-preservation tests failed against the previous behavior,
  then passed after removing hotspot offers/join/restore paths.
- UUID-selection regression failed against the stub, then passed when
  selecting all required characteristics from one discovery result.
- User confirmed Android Location is OFF. Official Android documentation
  requires Location mode for Direct discovery; user was instructed to
  enable it for the next physical test.
- Desktop tests: 13 passed, 8 hardware tests ignored. Workspace Clippy
  passed with warnings denied. Independent scoped review found no critical
  or important issue; no hardware changes were performed by the reviewer.
- Ruling: version only the Windows app as 0.1.2, retaining Android/core
  0.1.1; the existing Android negotiator honors LAN/Direct-only requests.
- Final workspace tests: 149 passed, 10 ignored by default. Windows MSI
  0.1.2 and production UI build passed; formatting/whitespace checks passed.
  Physical Direct success, concurrent Wi-Fi and ten-second setup still
  require testing with Android Location enabled.

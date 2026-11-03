# Preserve established Direct links between transfers

Windows 0.1.5 / Android 0.1.3 credential-mode experiment failed on the POCO:
the phone found the exact Windows SSID, attempted association twice, and
received ASSOC-REJECT status 12 both times. Windows received no application
ConnectionRequested event. The error is an unspecified association denial;
it does not prove a password mismatch, unsupported radio concurrency, or
that adding a peer MAC would solve it. Do not ship that experimental host
mode as a verified connection fix.

Earlier negotiated/PBC versions completed a real 3,736,428-byte transfer
with matching phone/PC SHA-256, and another first transfer connected in
13.4 seconds. A second transfer failed. Its preceding successful log shows
the Android client deliberately removing the group when the send completed.
Avoid unnecessary fresh provisioning for consecutive transfers.

Implementation (Windows 0.1.6 / Android 0.1.4 versionCode 5):

1. Restore Windows negotiated/PBC advertising; keep optional encrypted
   credential wire fields backward compatible but do not offer them.
   Actual warm-link build still failed its first GroupOwnerNegotiation
   setup. A controlled change to PreferredPairingProcedure::Invitation
   lets Windows choose PBC rather than relying on the connecting device.
   Microsoft documents these procedure semantics; do not infer an exact
   driver-level cause solely from WPS M2D's password-id field.
2. Reference-count a successfully established Android group. Keep it warm
   for 60 seconds after the last release, validate before reuse, serialize
   setup, close on stop, and guard idle timers with lease generations.
3. Reuse a connected Windows idle publisher even when its sending lease
   has ended. Retire disconnected/unusable publishers as before.
4. A TCP probe can reach a retained P2P address. Recognize that managed
   route, acquire its Android lease, and report Direct so Windows retains
   its lease too. Native transfer activity also prevents idle teardown
   for addresses rediscovered through mDNS.
5. Add opt-in physical two-transfer instrumentation, requiring an explicit
   beamOfflinePeer argument. No test changes visibility/trust or approves
   an incoming offer. USB is installation/diagnostics only; tethering off.

Regressions: RetainedLink tests observed 3 failures before retention and
passed after. Managed-route classification regression observed failing
with ordinary LAN treatment. Windows connected-idle reuse regression
observed failing with users-only admission. Finish all relevant builds,
tests, scoped review, physical repeat/reverse tests, and truthful artifact
notes. Do not claim ten-second or deployment acceptance before measured.

Phone test APK initially rejected by Android; installing this explicitly
test-only package with adb install -r -t succeeded. Regular updates keep
the shared signing key and stored identity/trust. Do not uninstall Beam.

Subsequent streamed main-APK installation was repeatedly rejected by MIUI
even after consent. Standard adb install --no-streaming -r -t succeeded
without changing security settings. Android 0.1.4 is installed in place.

Physical Invitation-mode evidence so far:

- A first 65,536-byte transfer completed in 11,285 ms. The original test
  then timed out looking only for Bluetooth devices after mDNS discovered
  the new Direct network. Test now permits either discovery on attempt 2.
- After a Windows restart, another first transfer completed in 18,417 ms.
  The next BLE handshake selected the existing Direct group in 4,754 ms
  without new WPS. Both test files arrived on Windows with the expected
  SHA-256 4b640d85ab3ba30fd02c9fc9db4a8928f416322ad27022ea58a65aaee68a4df2.
  USB dropped before that second attempt's final instrumentation report;
  do not label that run a complete JUnit pass. It proves arrival/integrity.
- PC remained associated with L-VIT throughout sampled observations. Its
  AP/channel roamed between observations; channels are not a controlled
  variable, so a single setting comparison is not proof of the driver cause.
- Reverse direction: user confirmed two consecutive PC → phone transfers.
  Both 1 MiB saved copies matched the PC source SHA-256. Fresh setup25.455s
  including OS approval; second used the established route and was faster.
- Installed Windows0.1.6 upgrade succeeded(code0), Android0.1.4 updated in
  place. A normal installed-app phone → PC transfer also succeeded with
  BLE/LAN setup3.148s. The user had switched PC Wi-Fi to their phone hotspot
  because college Wi-Fi stops at1:30am. Do not disconnect this internet link.
- Later automated cold-Direct runs cannot assert a Bluetooth-only initial
  route on that already-shared network. Harness now explicitly skips such
  a LAN precondition. Temporary startup/panic instrumentation used while
  checking that ambiguity was removed; it established no runtime defect.
  Final installed cold Direct check remains for independent Wi-Fi conditions.

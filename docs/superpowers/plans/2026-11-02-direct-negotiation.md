# Direct negotiation and stale-request repair

User authorized investigating compatible channels/negotiation to preserve
existing Wi-Fi and reduce setup delays. Correction: the phone's Wi-Fi is
on but it is NOT connected to an infrastructure network. Its popup alone
does not establish a conflict between two existing STA channels. PC was
observed on L-VIT, channel 52 earlier and channel 44 later. Driver-reported
concurrent-channel count is unavailable/zero, not proof of no support.

## Evidence and hypothesis

- Latest actual setup: Bluetooth request sent after 3.9 s; phone declined
  Direct after a further 34 s. A Direct request arrived three seconds
  after the declined reply and Windows unnecessarily accepted it for a
  further 25 s. Admission currently checks a 90 s timestamp but not whether
  any negotiated transfer still owns a Direct lease.
- Windows pre-forms an autonomous group before the phone's capabilities
  participate, and forces owner intent 15. Test the standard negotiated
  group path with Windows owner preference 14 and Android client preference
  0; routing can remain unchanged. This is a candidate hypothesis, not a
  verified fix for this phone. Microsoft supports both publisher modes and
  documents 14 as its default owner preference.
- No ADB device is available. User was asked to test baseline 0.1.2 by
  accepting the prompt promptly while the phone has no Wi-Fi association.

## Work

1. Reproduce stale admission in pure unit tests. Require an active lease
   as well as an unexpired acceptance window. Re-check before accepting,
   after pairing, and before retaining the device; prevent a completed
   call from leaving late/pending pairing work behind.
2. Configure standard negotiated discovery rather than an autonomous GO.
   Keep Android as client and laptop as preferred owner; do not change
   SSIDs, force frequencies, use private Android APIs, or forget pairings.
3. Verify the negotiated publisher's properties and lifetime on this PC
   without connecting a peer or changing infrastructure Wi-Fi; compare
   active Wi-Fi before/after. Do not run a test that disconnects a network.
4. Run focused regression tests, workspace Clippy/tests, UI build and MSI
   packaging as Windows 0.1.3. Android 0.1.1 remains compatible.
5. Review the scoped change independently. Document physical Direct and
   setup time as unverified until the user's test proves them. If it still
   fails, obtain phone-side diagnostics rather than claiming a hardware
   limit or piling on channel/role changes without evidence.

No commit/push/publishing, automatic install, or forced network changes.

## Ledger

- User's baseline retest: accepted the phone's prompt promptly; 0.1.2 still
  failed with the no-fallback error. Do not blame approval delay or assume
  the unassociated phone has a STA-channel conflict.
- Stale-admission and pending-pairing tests were observed failing first,
  then passing. Desktop suite: 15 passed, 9 hardware checks ignored.
- Explicit negotiated-publisher check failed with autonomous mode, then
  passed with negotiated mode; publisher started and connected Wi-Fi
  profiles were unchanged. PC stayed on L-VIT channel 44. No peer connected
  during this check, no app was installed and no SSID was switched.
- Admission is bound to publisher UUID and a cancellation receiver for the
  current group of active leases. New lease windows cannot clear an old
  receiver's cancellation. Pairing and FromId operations keep cancellation
  guards and the request closes on every exit.
- Review identified a same-publisher retry race when both cancellation and
  an old operation are ready. Added a separately captured transfer-session
  UUID, validated at entry, after pairing and under the final retention
  lock; cancellation also has select priority. Regression observed RED
  then GREEN. Final desktop suite: 16 passed, 9 ignored.
- Final workspace verification: 152 passed, 11 ignored by default;
  workspace Clippy/formatting passed; final Windows 0.1.3 MSI and UI
  production build passed. Explicit PC discovery-only check passed.

# Credential-based Wi-Fi Direct investigation

Current versions: Windows 0.1.4 diagnostic process, Android 0.1.2 installed.
One Direct video transfer was verified by matching SHA-256 on phone and PC;
first updated transfer also completed in about 13.4 s, but repeat still
failed. Phone logs establish GO negotiation succeeded at 5220 MHz followed
by WPS M2D / formation failure. Cleanup fixes alone did not resolve this.

Test the public Windows LegacySettings credential mode and Android public
WifiP2pConfig.Builder networkName/passphrase path. This is still a Windows
virtual Direct group and Android P2P interface, not a WiFiAdapter/STA SSID
switch. Windows already has that virtual adapter beside its infrastructure
connection. No hidden Android fields, root, disabled firewall, unpairing,
or ordinary Wi-Fi joining API is permitted.

1. Extend DirectPeer with optional network_name/passphrase fields, serde
   defaults preserving older CBOR. Keep credentials inside encrypted GATT
   request, and never log them. Add encrypted round-trip and old-message
   compatibility regressions before completing forwarding.
2. Enable Windows autonomous GO+LegacySettings and await generated
   credentials before offering Direct. For credential-mode requests skip
   custom WPS pairing, as Microsoft's advertiser sample does for legacy
   clients. Still create FromIdAsync on UI and keep admission/cancellation.
3. Android uses public P2P config with the offered group name/passphrase,
   skips peer discovery, and keeps the existing generation cleanup guard.
   Older requests retain the peer/PBC path. No ordinary network switching.
4. Build and test all layers; test actual consecutive transfers and the
   reverse direction before claiming reliability. Previous candidate
   artifacts and the installed debug process are not production-ready.

USB is connected intermittently. It was used for redacted diagnostics,
hash calculation and an in-place Android update, never for the verified
video's data transport. An owned adb logcat helper PID 19724 may still run;
stop it and redact raw captures before completing the task. Current PC
diagnostic PID 26532 runs D:/beam-target/debug/beam-desktop.exe (0.1.4).
Restore the normal SendTo shortcut from the saved backup when leaving
diagnostic mode or launch the installed stable app after upgrade.

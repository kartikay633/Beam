# Beam privacy policy

Last updated: 3 November 2026

Beam sends files directly between your own devices. It has no servers and
no accounts, and it collects nothing.

## What Beam does with your data

- **Files you send** go straight from one device to the other, over your
  local network or a private Wi-Fi link between the two devices, and are
  encrypted end to end (Noise protocol). They never pass through anyone
  else's server.
- **Device keys** are created on your device and stay there, protected by
  Windows (DPAPI) or Android Keystore. They are never backed up or uploaded.
- **Trusted devices**, your settings, and the name you give your device are
  stored only on your device.
- **Nearby discovery**: while visible, Beam announces your device name (or,
  in "trusted only" mode, an anonymous identifier that changes every 15
  minutes) on your local network and over Bluetooth LE, so your other
  devices can find it. You can turn visibility off at any time.

## What Beam doesn't do

- No analytics, telemetry, crash reporting or advertising.
- No internet connection is needed or used.
- Nothing is shared with third parties.

## Permissions

- **Bluetooth / Nearby devices**: to find your other devices and agree on a
  connection when you share no Wi-Fi network.
- **Location (Windows 11; Android 12 and older)**: Windows and older
  versions of Android only let apps scan for Wi-Fi networks and Bluetooth
  devices with location permission. Beam never reads your location.
- **Turning on Bluetooth, Wi-Fi and Location (Android)**: when you open
  Beam, it asks Android to switch these on if they're off, because some
  phones only find Wi-Fi devices with Location on. Beam still never reads
  your location. When you leave Beam, it reminds you to switch off the ones
  it turned on.
- **Waking up after Quit (Windows, Microsoft Store version)**: after you
  quit Beam, Windows watches for a nearby phone running Beam over
  Bluetooth and starts Beam again in the tray. This happens on your PC;
  nothing is sent anywhere.
- **Camera (Android)**: only to scan a pairing QR code. Nothing is recorded.
- **Notifications (Android)**: to tell you about incoming files and progress.

## Contact

Questions: open an issue at https://github.com/kartikay633/Beam/issues.

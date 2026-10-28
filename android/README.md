# Beam for Android

Kotlin + Jetpack Compose, minSdk 29 (Android 10), target SDK 37. The
protocol, encryption and transfer engine are `beam-core` in Rust, used
through UniFFI bindings from [`core-ffi/`](../core-ffi). See
[docs/SPEC.md](../docs/SPEC.md) section 8 and
[docs/DECISIONS.md](../docs/DECISIONS.md) for how it fits together.

## What you need

- JDK 21 (`JAVA_HOME`).
- The Android SDK with platform `android-37.0`, build-tools `37.0.0` and
  NDK `30.0.16248370`, found through `ANDROID_HOME` or `local.properties`
  (`sdk.dir=D\:/android/sdk`; use forward slashes on Windows).
- Rust with the Android targets, and `cargo-ndk`:

  ```bash
  rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
  cargo install cargo-ndk --version 4.1.2 --locked
  ```

## Building

```bash
./gradlew assembleDebug            # the APK, for arm64-v8a, armeabi-v7a and x86_64
./gradlew testDebugUnitTest        # JVM tests (no device needed)
./gradlew connectedDebugAndroidTest  # instrumented tests, on a device or emulator
```

Every build first runs two Gradle tasks:

- `cargoNdkBuild`: `cargo ndk … build -p beam-core-ffi --release` into
  `app/build/rust/jniLibs`;
- `uniffiBindings`: generates the Kotlin bindings (`dev.beam.core`) from the
  built library into `app/build/generated/uniffi`.

For a faster build while developing, build only the ABI of your phone:

```bash
./gradlew assembleDebug -Pbeam.abis=arm64-v8a
```

## Layout

| Package | What's in it |
|---|---|
| `data` | `BeamRepository` (owns the Rust node, turns its events into state), settings, the Keystore-wrapped device key, reading shared files, saving to Downloads, NsdManager discovery |
| `service` | The foreground service, its notifications, and the Wi-Fi and wake locks |
| `model` | Plain Kotlin transfer state (speed, ETA), unit-tested on the JVM |
| `ui` | Compose screens: Home, Transfers, Devices (trusted + QR pairing), Settings |

## Trying it

Install the APK on a phone on the same Wi-Fi as a laptop running the
desktop app (or `beam-cli receive`). Then:

- **Phone → laptop:** share a photo from the gallery to **Beam**, then tap
  the laptop under **Nearby**. Accept on the laptop after comparing the
  6-digit codes.
- **Laptop → phone:** drop a file on the phone in the desktop app. The phone
  asks (a dialog, or a notification when Beam is in the background) and
  saves to `Download/Beam`.
- **Pairing:** desktop **Pair a phone** → phone **Devices → Pair with a
  laptop**, scan. Without a camera, use **Copy pairing link** on the laptop
  and paste it on the phone.
- **Discovery:** Settings → **Finding devices** switches between Android's
  NsdManager (default) and Beam's own mDNS.

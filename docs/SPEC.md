# Beam — AirDrop for Android ↔ Windows

> "Beam" is a placeholder name. Rename it everywhere if you like.
> Put this file in the repo at `docs/SPEC.md` so the AI can read it in every session.

## 1. What we are building

Beam lets you send files (and clipboard text) between an **Android phone** and a **Windows laptop**:

- **Fast:** close to full Wi-Fi speed (target ≥ 25 MB/s on 5 GHz)
- **Works with no shared Wi-Fi network and no internet:** devices find each other over Bluetooth Low Energy (BLE), then open a direct Wi-Fi link
- **Secure:** end-to-end encrypted, with device verification
- **Reliable:** transfers resume after a disconnect, and every byte is verified

### The user experience

1. On the phone, the user picks a photo, taps **Share → Beam**, and sees nearby laptops appear within about 2 seconds.
2. They tap the laptop. On the laptop, a popup appears: **"Kartikay's Pixel wants to send 3 files (2.3 GB). Accept / Decline."**
3. They tap Accept. The file transfers with a progress bar, speed and ETA, and is saved to `Downloads\Beam`.
4. It works the same in reverse, by dragging files onto the Beam window or right-clicking a file and choosing **Send to → Beam**.

### Visibility modes (like AirDrop)

- **Off:** not discoverable
- **Trusted devices only:** only paired devices can see you and send to you
- **Everyone for 10 minutes:** anyone nearby can send you a request (you still have to accept)

---

## 2. Architecture

```
┌───────────────────────── Android app (Kotlin) ─────────────────────────┐
│ Jetpack Compose UI · Share-sheet target · Foreground service           │
│ Radio layer: BLE scan/advertise/GATT · Wi-Fi Direct · LocalOnlyHotspot │
│ NsdManager (mDNS)                                                      │
│                ▲ calls via UniFFI-generated Kotlin bindings            │
└────────────────┼───────────────────────────────────────────────────────┘
                 │
        ┌────────┴──────────┐
        │  beam-core (Rust) │  ← ONE implementation, used on every platform
        │  protocol, crypto,│
        │  transfer engine, │
        │  resume, mDNS     │
        └────────┬──────────┘
                 │ linked directly (Rust)
┌────────────────┼──────────── Desktop app (Tauri v2) ───────────────────┐
│ React + TypeScript UI · tray icon · accept popup · drag-and-drop       │
│ Radio layer (Rust `windows` crate → WinRT):                            │
│ BLE advertise/scan/GATT server · Wi-Fi Direct · join Wi-Fi network     │
└─────────────────────────────────────────────────────────────────────────┘
```

**The key design decision:** the protocol, encryption and transfer engine live in **one Rust library (`beam-core`)**. The Android app uses it through **UniFFI** bindings, and the desktop app links it directly. It's written once, tested once, and behaves identically on both sides. Only the radio code (BLE and Wi-Fi) is platform-specific.

There's also a **CLI (`beam-cli`)** built on the same core. It's used for testing, benchmarks and development without a phone.

### Tech stack

| Part | Choice |
|---|---|
| Core library | Rust (stable), `tokio` async runtime |
| Encryption | Noise Protocol `Noise_XX_25519_ChaChaPoly_BLAKE2s` via the `snow` crate |
| Serialization (control messages) | CBOR via `ciborium` + `serde` |
| Hashing | BLAKE3 (`blake3` crate) |
| mDNS | `mdns-sd` crate |
| Android bindings | UniFFI, built with `cargo-ndk` (arm64-v8a, armeabi-v7a, x86_64) |
| Android app | Kotlin, Jetpack Compose, minSdk 29 (Android 10), target latest SDK |
| Desktop app | Tauri v2, React + TypeScript + Vite UI |
| Windows radio APIs | `windows` crate (WinRT): `Windows.Devices.Bluetooth.*`, `Windows.Devices.WiFiDirect.*`, `Windows.Devices.WiFi` |
| CI | GitHub Actions |

> **Fallback if WinRT from Rust becomes too painful:** write a small C# (.NET 8) helper process for the Windows radio APIs only. Tauri talks to it over a local pipe. Keep this as plan B; don't start with it.

### Repository layout

```
beam/
  core/          Rust crate: protocol, crypto, transfer engine, resume, mDNS
  core-ffi/      UniFFI wrapper exposing core to Kotlin
  cli/           beam-cli: send/receive/discover from a terminal
  desktop/       Tauri app (src-tauri/ = Rust, ui/ = React)
  android/       Android Studio project (Kotlin + Compose)
  bench/         benchmark scripts and results (CSV + charts)
  docs/          SPEC.md, PROTOCOL.md, DECISIONS.md, LEARNINGS.md
  .github/workflows/  CI
```

---

## 3. How a transfer works, end to end

```
Phone (sender)                                  Laptop (receiver)
──────────────                                  ─────────────────
                                     ◄── BLE advertisement (always on when visible)
BLE scan finds "Beam" service UUID
Show laptop in device list
User taps laptop
GATT connect ──────────────────────────────────►  GATT server
Write CONNECT_REQUEST (capabilities,
  ephemeral X25519 public key) ────────────────►
                                     ◄──────── Notify CONNECT_RESPONSE (encrypted):
                                               chosen link + parameters
Bring up link (same LAN / Wi-Fi Direct / hotspot)
TCP connect ───────────────────────────────────►
Noise XX handshake ◄──────────────────────────► (mutual authentication)
HELLO ◄───────────────────────────────────────► HELLO
OFFER (file list, sizes) ──────────────────────►
                                               Popup: Accept / Decline
                                     ◄──────── ACCEPT (+ already-received ranges, for resume)
Open N parallel data connections (Noise each)
DATA frames on all streams ════════════════════►  write to .part files at offsets
CHUNK_HASH per 4 MiB chunk ────────────────────►  verify, persist progress
FILE_DONE (full BLAKE3) ───────────────────────►  verify, rename .part → final name
                                     ◄──────── TRANSFER_COMPLETE
Tear down link, restore previous Wi-Fi if changed
```

---

## 4. Discovery

### 4.1 Same-network discovery (mDNS)

- Service type: `_beam._tcp.local.`
- TXT records: `v` (protocol version), `id` (device id, hex of first 8 bytes of SHA-256 of static public key), `name`, `type` (`phone` | `laptop`), `vis` (visibility mode)
- Android uses `NsdManager`. Desktop and CLI use the `mdns-sd` crate in core.

### 4.2 No-network discovery (BLE)

**Advertisement** (legacy advertising, 31-byte limit):
- Flags (3 bytes)
- Service Data with a 128-bit Beam service UUID (generate one and hard-code it) and a 10-byte payload:

| Byte | Field |
|---|---|
| 0 | protocol version |
| 1 | device type (0 = phone, 1 = laptop) |
| 2–7 | device id hash (6 bytes) |
| 8 | flags: bit0 accepting, bit1 has LAN address, bit2 Wi-Fi Direct capable, bit3 hotspot capable |
| 9 | reserved |

- Scan response: shortened device name.
- **Stretch goal (privacy):** for trusted-only mode, replace the static id hash with a rotating `HMAC(shared_secret, 15-minute time window)` so strangers can't track a device by its BLE broadcasts.

**GATT service** (hosted by the receiver):

| Characteristic | Properties | Content |
|---|---|---|
| `INFO` | read | CBOR: device name, type, static public key fingerprint, capabilities, and a fresh **ephemeral X25519 public key** (new one per GATT connection) |
| `CONNECT_REQUEST` | write | Sender's ephemeral X25519 public key (plaintext), then an encrypted CBOR payload: capabilities, preferred links, and hotspot SSID/password if the sender is hosting one |
| `CONNECT_RESPONSE` | notify | Encrypted CBOR payload: the chosen link and its parameters (LAN IP:port, Wi-Fi Direct info, or hotspot SSID/password) |

Encryption for both payloads: X25519(ephemeral keys) → HKDF-SHA256 → ChaCha20-Poly1305, with a separate key per direction.

Request an MTU of 517 after connecting. If a payload is larger than the MTU, split it into numbered fragments.

Why encrypt these? They can contain a Wi-Fi password. This BLE step is **not** the authentication step; real mutual authentication happens in the Noise handshake over TCP.

---

## 5. Link negotiation (choosing how to connect)

The receiver picks the first option that works, in this order:

1. **Same LAN:** both devices are already on the same Wi-Fi. Use the receiver's LAN IP and port. Try a TCP connect with a 1.5 s timeout.
2. **Wi-Fi Direct (P2P):** Android `WifiP2pManager` ↔ Windows `WiFiDirectDevice` / `WiFiDirectConnectionListener`. The laptop keeps its normal internet connection if the adapter supports it. Interop between Android and Windows here is quirky, so expect debugging.
3. **Android local-only hotspot:** the phone calls `WifiManager.startLocalOnlyHotspot()`, which gives a random SSID and password. These are sent to the laptop inside the encrypted GATT payload (`CONNECT_REQUEST` if the phone is sending, `CONNECT_RESPONSE` if the phone is receiving). The laptop joins via `Windows.Devices.WiFi.WiFiAdapter.ConnectAsync`, then **reconnects to its previous network** after the transfer. This is the most reliable option, so **build it first** and add Wi-Fi Direct after.

Store which link was used, and how long setup took, in transfer stats.

---

## 6. Security

### 6.1 Identity

- On first launch, each device generates a **static X25519 key pair** (the Noise static key). It's stored in Android Keystore-encrypted storage on Android and in Windows Credential Manager / DPAPI on desktop.
- **Device fingerprint:** first 8 bytes of SHA-256(static public key), shown as 4 groups of 4 hex characters.

### 6.2 Session encryption

- Every TCP connection starts with a **Noise XX** handshake. Both sides learn and verify each other's static public key.
- Data connections are bound to the control session: the first message on a data connection carries a `session_token` derived from the control connection's handshake hash.
- **Don't invent our own crypto.** Use `snow`, and document in `DECISIONS.md` why Noise was chosen over TLS with self-signed certs (no certificate infrastructure, mutual auth built in, small and auditable).

### 6.3 Trust and pairing

- **QR pairing:** the laptop shows a QR code containing `{device_id, static_public_key, name, LAN addresses}`. When the phone scans it, the devices become **trusted**, and the phone sends its own key back over the authenticated session.
- **Unpaired transfers** (visibility "Everyone"): the receiver popup shows a **6-digit verification code** derived from the Noise handshake hash. The sender shows the same code, and the users can compare them (like Bluetooth numeric comparison).
- Trusted devices can be set to **auto-accept**. Unknown devices **never** auto-accept.

### 6.4 Input hardening (checklist)

- **Path traversal:** sanitize every file name. Strip `..`, `/`, `\`, drive letters, control characters and Windows reserved names (`CON`, `NUL`, …). Never trust sender paths.
- If a file name already exists, save as `name (1).ext`.
- Check free disk space before accepting.
- Limit offer size (max files per offer, max name length).
- Decline everything when visibility is Off, even if a connection reaches the port.
- Rate-limit incoming requests per device (no popup spam).
- Fuzz the frame decoder (`cargo-fuzz`). It must never panic on bad input.

---

## 7. Wire protocol

### 7.1 Framing

After the Noise handshake, every Noise transport message (max 65,535 bytes) contains exactly one frame:

```
frame = type:u8 | body
```

Control message bodies are CBOR. `DATA` uses a fixed binary header so it can be parsed without CBOR overhead.

### 7.2 Messages

| Type | Name | Direction | Body |
|---|---|---|---|
| 0x01 | `HELLO` | both | `{version, device_id, name, type, capabilities[]}` |
| 0x02 | `OFFER` | sender → receiver | `{transfer_id: uuid, files: [{file_id, name, size, mime, modified_unix}], total_size}` |
| 0x03 | `ACCEPT` | receiver → sender | `{transfer_id, have: [{file_id, chunk_bitmap}]}` (for resume) |
| 0x04 | `REJECT` | receiver → sender | `{transfer_id, reason}` |
| 0x05 | `STREAM_OPEN` | sender → receiver (data connection) | `{transfer_id, session_token, stream_index}` |
| 0x06 | `DATA` | sender → receiver | `file_id:u32 \| offset:u64 \| len:u32 \| bytes` |
| 0x07 | `CHUNK_HASH` | sender → receiver | `{file_id, chunk_index, blake3}` |
| 0x08 | `CHUNK_ACK` | receiver → sender | `{file_id, chunk_indices[]}` (batched about every 250 ms) |
| 0x09 | `FILE_DONE` | sender → receiver | `{file_id, blake3_full}` |
| 0x0A | `TRANSFER_COMPLETE` | receiver → sender | `{transfer_id, stats}` |
| 0x0B | `CANCEL` | both | `{transfer_id, reason}` |
| 0x0C | `PING` / 0x0D `PONG` | both | `{nonce}`, keepalive every 5 s, timeout 15 s |
| 0x10 | `CLIPBOARD` | both | `{mime: "text/plain", text}` |

`PROTOCOL.md` must describe every message with an example. Version negotiation: if the major version in `HELLO` differs, close with a clear error.

### 7.3 Transfer engine

- **Chunk** = 4 MiB, the unit of hashing, resume and scheduling. A chunk is sent as several `DATA` frames of up to about 64 KiB each.
- **Parallel streams:** 1 control connection plus **N data connections** (default 4, configurable, benchmark 1/2/4/8). Chunks go into a shared queue, and each stream takes the next chunk when it's free (work stealing).
- **Receiver writes** with positional writes (`pwrite`-style, `FileChannel.write(buf, pos)` on Android) into `name.beam-part`. Allocate the full size upfront.
- **Progress file:** `name.beam-part.json` holds the transfer id, file fingerprint (name + size + modified time) and the bitmap of verified chunks. Save it at least every 2 seconds.
- **Verification:** each chunk is checked against `CHUNK_HASH`. A mismatch means re-requesting that chunk. At the end, the full-file BLAKE3 is checked against `FILE_DONE`, then the file is renamed to its final name.
- **Backpressure:** the sender has a bounded in-flight window per stream, and the receiver's disk writes go through a bounded channel. Memory must stay flat (under about 100 MB) even for a 20 GB file.
- **Resume:** if the connection drops, the sender retries the link for up to 60 s. On reconnect, `ACCEPT.have` lists verified chunks and the sender skips them. Partial transfers are kept for 24 hours.
- **Small files:** batch many small files so 1,000 × 10 KB files don't take 1,000 round trips (pipeline offers and data without waiting per file).
- **Stats:** bytes, duration, average and peak MB/s, link type, stream count, retries. Shown in the UI and written to the benchmark CSV.

---

## 8. Android app

- **UI (Compose):** home screen (visibility toggle, nearby devices, recent transfers), send screen, transfer progress, trusted devices, settings.
- **Share target:** intent filters for `ACTION_SEND` and `ACTION_SEND_MULTIPLE`, `*/*`. Read files through `ContentResolver` URIs, never raw paths.
- **Foreground service** with type `connectedDevice` during discovery and transfers, plus an ongoing notification with progress and Cancel.
- **Saving files:** `MediaStore.Downloads` under `Download/Beam` (scoped storage).
- **During transfers:** a `WifiManager.WifiLock` (`WIFI_MODE_FULL_LOW_LATENCY`) and a partial wake lock, released immediately after.
- **Permissions:** `BLUETOOTH_SCAN`, `BLUETOOTH_ADVERTISE`, `BLUETOOTH_CONNECT`, `NEARBY_WIFI_DEVICES` (Android 13+), `ACCESS_FINE_LOCATION` (only where older APIs require it), `POST_NOTIFICATIONS`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`. Ask with clear explanations, and handle the "denied" case gracefully.
- **Quick Settings tile:** toggles visibility.
- **QR scanner** for pairing: CameraX + ML Kit barcode scanning.

## 9. Desktop app (Windows first)

- **Tray icon:** visibility mode, open window, quit. Starts with Windows (optional setting).
- **Main window:** nearby devices, drag-and-drop area, transfer list with progress, trusted devices, settings (download folder, auto-accept, stream count).
- **Incoming request popup:** small always-on-top window with Accept / Decline, the 6-digit verification code and a 60 s timeout.
- **Send To integration:** install a shortcut in the Windows `SendTo` folder. The single-instance plugin forwards the file paths to the running app.
- **Pairing:** show a QR code.
- **Linux/macOS:** the same app runs in LAN-only mode (mDNS discovery). BLE and Wi-Fi Direct are Windows-only in v1.

## 10. Extra features (after the core works)

1. **Clipboard sync:** copy on one device, then press "Send clipboard" (or enable auto-sync for trusted devices).
2. **Auto-lock the laptop when the phone walks away:** BLE RSSI below a threshold for 10 s triggers `LockWorkStation()`. (Unlocking needs a Windows Credential Provider in C++; that's a stretch goal.)
3. **Rotating BLE identifiers** for privacy (see 4.2).

---

## 11. Testing

- **Unit tests (core):** frame encode/decode, CBOR messages, chunk scheduler, bitmap persistence, file name sanitizer (with a nasty-input table).
- **Property tests (`proptest`):** decode(encode(x)) == x for every message.
- **Fuzzing (`cargo-fuzz`):** the frame decoder and CBOR parsing must never panic.
- **Integration tests:** two in-process peers over loopback:
  - transfer 1 GB and 1,000 small files, then compare hashes
  - **chaos test:** kill the connection at random points 50 times; the transfer must resume and finish with an identical hash
  - corrupt a byte in a chunk; it must be detected and re-sent
  - path-traversal file names are sanitized
- **Android:** unit tests for the view-models, and an instrumented test for the share-intent flow.
- **CI (GitHub Actions):** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, the Android build + unit tests and the Tauri build on `windows-latest`.

## 12. Benchmarks (these become your resume numbers)

`bench/` contains:
- A script that generates test sets: 1 × 1 GB random file, 10 × 100 MB, and 1,000 × 10 KB.
- A runner that does 5 runs per scenario and records the median MB/s, setup time and link type to `results.csv`.
- **Scenarios:** same LAN, Wi-Fi Direct, hotspot × stream count 1/2/4/8.
- **Comparisons** (measured by hand with a stopwatch, same files, same devices): LocalSend, Quick Share and Bluetooth file transfer.
- **Android battery and CPU** during a 5 GB transfer (`adb shell dumpsys batterystats`).
- A chart script that produces PNGs for the README.

Always record device models, Wi-Fi band (2.4/5 GHz) and distance. Honest numbers only.

## 13. Definition of done (v1)

- [ ] Phone → laptop and laptop → phone work with **no shared Wi-Fi**
- [ ] ≥ 25 MB/s on 5 GHz for a 1 GB file (or a documented explanation of the bottleneck)
- [ ] Resume works after airplane-mode toggling mid-transfer
- [ ] Noise encryption, QR pairing and the verification code all work
- [ ] All tests pass in CI, and the fuzzer runs 10 minutes without crashes
- [ ] README with a GIF demo, architecture diagram, benchmark chart and install instructions
- [ ] Signed APK on GitHub Releases (Play Store optional), plus a Windows installer (`.msi` from Tauri)

## 14. Documents the AI must keep updated

- `docs/PROTOCOL.md`: the wire protocol, precise enough that someone could write a compatible client
- `docs/DECISIONS.md`: every important choice, with alternatives and why (append-only)
- `docs/LEARNINGS.md`: bugs that took more than an hour, root cause and fix. **This is your interview story bank.**

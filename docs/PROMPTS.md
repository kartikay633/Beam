# Beam — prompts to give your AI, phase by phase

## How to use this file

1. Create an empty GitHub repo called `beam` and put `SPEC.md` in it at `docs/SPEC.md`.
2. Work **one phase at a time**. Don't paste the whole file at once. The AI does much better work on one focused step.
3. Start every new AI session with the **Session starter** prompt below, then paste the phase prompt.
4. After each phase:
   - Run the tests yourself and try the feature with your own hands
   - Commit (`git commit -m "phase N: ..."`)
   - Use the **Learn it** prompt at the bottom until you can explain every part without looking

> **Important:** interviewers will ask you *why* things work. If the AI writes it and you can't explain it, it hurts you instead of helping. The Learn-it step isn't optional.

---

## Session starter (paste at the start of every session)

```
You are a senior engineer helping me build "Beam", an AirDrop-like file-sharing app
for Android ↔ Windows. The full design is in docs/SPEC.md. Read it fully before
doing anything. Also read docs/DECISIONS.md and docs/LEARNINGS.md if they exist.

Rules:
- Follow SPEC.md. If you think the spec is wrong or something is impossible,
  stop and tell me why before changing direction.
- Work only on the phase I give you. Don't build future phases early.
- Write tests for everything you build, and run them.
- Keep code simple and well-named. Comment only the non-obvious parts.
- After finishing, append important choices to docs/DECISIONS.md (what, why,
  alternatives considered) and any hard bugs to docs/LEARNINGS.md (symptom,
  root cause, fix).
- End with a short summary: what you built, how to run it, how to test it,
  and what I should try by hand.
```

---

## Phase 0 — Repository setup (day 1)

```
Phase 0: set up the repository skeleton exactly as in SPEC.md section 2
"Repository layout".

- Create a Cargo workspace with crates: core, core-ffi, cli.
- core: empty lib with a placeholder test. cli: binary that prints its version.
- Leave desktop/ and android/ as folders with a README saying "coming in a later phase".
- Add docs/PROTOCOL.md, docs/DECISIONS.md, docs/LEARNINGS.md with headings only.
- Add .gitignore for Rust, Node, Android and Tauri.
- Add a GitHub Actions workflow running cargo fmt --check, cargo clippy -D warnings,
  and cargo test on ubuntu-latest and windows-latest.
- Write a top-level README with the one-line pitch and a "status: in development" note.

Done when: `cargo test` passes locally and the workflow file is valid.
```

---

## Phase 1 — Basic transfer over TCP, no encryption yet (week 1)

```
Phase 1: implement the core transfer protocol from SPEC.md section 7, WITHOUT
encryption for now (plain TCP). Encryption comes in phase 2, so structure the code
so a Noise layer can wrap the connection later without rewriting the protocol.

In core/:
- Frame encode/decode (section 7.1) and all messages in section 7.2 using CBOR
  (ciborium + serde). DATA uses the fixed binary header.
- A Receiver that listens on a TCP port, receives HELLO + OFFER, and calls a
  callback to ask "accept or reject?".
- A Sender that connects, sends HELLO + OFFER, then streams files in 4 MiB chunks
  using DATA frames of ~64 KiB, then CHUNK_HASH (BLAKE3) and FILE_DONE.
- Receiver writes to name.beam-part, verifies hashes, renames on success.
- The file-name sanitizer from SPEC.md section 6.4, with a test table of nasty inputs
  ("../../etc/passwd", "CON.txt", "a\\b", names with control characters, very long names).
- Use only ONE connection for now (parallel streams come in phase 3).

In cli/:
- `beam-cli receive --port 9000 --dir ./inbox` (auto-accept for now)
- `beam-cli send --to 127.0.0.1:9000 file1 file2 ...`
- Show progress and final MB/s.

Tests:
- Property test: decode(encode(msg)) == msg for every message type.
- Integration test: send a 200 MB random file and 500 small files over loopback,
  compare BLAKE3 hashes of source and destination.

Also write docs/PROTOCOL.md for everything implemented so far.

Done when: I can run receive in one terminal, send in another, and get identical files.
```

---

## Phase 2 — Encryption and device identity (week 2)

```
Phase 2: add security per SPEC.md section 6.

- Generate and persist a static X25519 key pair per device (for the CLI, store it in
  a config dir file for now; platform keystores come later). Compute the fingerprint.
- Wrap every TCP connection in a Noise XX handshake using the `snow` crate
  (Noise_XX_25519_ChaChaPoly_BLAKE2s). All frames travel as Noise transport messages.
- Derive the 6-digit verification code from the handshake hash. The CLI prints it on
  both sides.
- Trusted devices: a simple JSON store of {device_id, name, public_key, auto_accept}.
  CLI commands: `beam-cli trust list`, `beam-cli trust remove <id>`.
- Receiver: unknown devices must be asked (CLI prompt y/n). Trusted + auto_accept skips
  the prompt.
- Add a test that a man-in-the-middle with a different key is detected (the static key
  doesn't match the trusted key → connection refused).

Write in DECISIONS.md why Noise was chosen over TLS with self-signed certificates.

Done when: transfers still work, the traffic is encrypted (show me how to confirm with
Wireshark that no plaintext file content is visible), and both sides print the same
verification code.
```

---

## Phase 3 — Speed and reliability: parallel streams and resume (week 3)

```
Phase 3: implement the full transfer engine from SPEC.md section 7.3.

- Parallel data connections (default 4, `--streams N` flag), each with its own Noise
  handshake and STREAM_OPEN bound to the control session via session_token.
- A shared chunk queue with work stealing across streams.
- Positional writes on the receiver, file pre-allocation, and a bounded write channel.
- Progress file (.beam-part.json) with the verified-chunk bitmap, saved at least every 2 s.
- CHUNK_ACK batching, PING/PONG keepalive (5 s interval, 15 s timeout).
- Resume: when the connection drops, the sender retries for up to 60 s; on reconnect
  the receiver's ACCEPT lists already-verified chunks and only missing chunks are sent.
- Pipelining for many small files (don't wait for a round trip per file).
- Memory must stay under ~100 MB even for a 10 GB file.

Tests:
- CHAOS TEST: transfer a 1 GB file and kill the connection at random points 50 times.
  The transfer must finish with an identical hash every time.
- Corrupt one byte of one chunk in transit (test hook) → detected and re-sent.
- 1,000 × 10 KB files complete in reasonable time (print it).

Also create bench/: a script to generate test files and a runner that does 5 runs per
scenario (streams = 1, 2, 4, 8) over loopback and LAN, writing results.csv, plus a
script that charts results to PNG.

Done when: the chaos test passes and the bench runner produces a CSV and a chart.
```

---

## Phase 4 — Finding devices on the same Wi-Fi (week 4, first half)

```
Phase 4: add LAN discovery per SPEC.md section 4.1 using the `mdns-sd` crate in core.

- Receivers advertise _beam._tcp.local. with the TXT records from the spec.
- `beam-cli discover` lists nearby devices (name, type, fingerprint, address) live.
- `beam-cli send --to <device name or id>` resolves via mDNS.
- Respect visibility modes (off / trusted-only / everyone-for-10-minutes):
  trusted-only must not appear to unknown devices, and "everyone" expires after 10 minutes.

Done when: two laptops (or a laptop and a VM) on the same Wi-Fi find each other and
transfer by device name.
```

---

## Phase 5 — Desktop app (week 4, second half, and week 5)

```
Phase 5: build the Windows desktop app per SPEC.md section 9, using Tauri v2 with a
React + TypeScript UI. The Tauri Rust side uses beam-core directly. No BLE or
Wi-Fi Direct yet, LAN only.

- Tray icon with the visibility mode switch, Open and Quit.
- Main window: nearby devices, a drag-and-drop zone, a transfer list (progress bar,
  speed, ETA, cancel), trusted devices, settings (download folder, auto-accept,
  stream count).
- Incoming request popup: always-on-top, shows sender name, file list, total size and
  the 6-digit verification code, with Accept / Decline and a 60 s timeout.
- Store keys using Windows DPAPI / Credential Manager instead of a plain file.
- Single-instance app, plus a "Send to → Beam" shortcut in the Windows SendTo folder
  that forwards file paths to the running app.
- QR code screen for pairing (contents per SPEC.md section 6.3).
- Clean, modern UI. Keep it simple; prioritize clarity over decoration.

Done when: I can send files laptop ↔ laptop through the app, and the CLI and the app
can talk to each other.
```

---

## Phase 6 — Android app on the same Wi-Fi (weeks 5–6) ★ first real milestone

```
Phase 6: build the Android app per SPEC.md section 8, LAN mode only for now.

- Set up core-ffi with UniFFI: expose an async API to start/stop the receiver,
  discover, send, accept/reject, cancel, plus a callback interface for events
  (device found/lost, incoming offer, progress, completed, error).
- Build the Rust library for arm64-v8a, armeabi-v7a, x86_64 with cargo-ndk, wired
  into Gradle so `./gradlew assembleDebug` builds everything.
- Kotlin + Jetpack Compose UI: home (visibility toggle, nearby devices, recent
  transfers), send flow, progress screen, trusted devices, settings.
- Share target for ACTION_SEND and ACTION_SEND_MULTIPLE (*/*), reading via
  ContentResolver URIs. Since core expects file paths, design an approach for
  content URIs (e.g. pass file descriptors through FFI, or stream through a callback)
  and explain the choice in DECISIONS.md.
- Discovery via NsdManager (or core's mDNS if it works reliably on Android; test both
  and document the choice).
- Foreground service (type connectedDevice) with a progress notification and Cancel.
- Save received files to Download/Beam via MediaStore.
- WifiLock + partial wake lock during transfers only.
- QR scanner (CameraX + ML Kit) for pairing with the desktop app.
- Runtime permission requests with clear explanations; handle denial gracefully.

Done when: on the same Wi-Fi, I can share a photo from the Android gallery to my laptop,
and send a file from the laptop to the phone, both through the apps.
```

> 🎉 After Phase 6, you have a working, useful product. Everything after this is what makes it stand out.

---

## Phase 7 — Bluetooth discovery with no network (week 7)

```
Phase 7: implement BLE discovery and the GATT handshake per SPEC.md section 4.2.

Android (Kotlin, in the app's radio layer):
- BluetoothLeAdvertiser with the service data layout from the spec, plus a scan response.
- BluetoothLeScanner filtered by the Beam service UUID, merged into the nearby list.
- GATT server with INFO, CONNECT_REQUEST and CONNECT_RESPONSE characteristics, and a
  GATT client for the sending side. Negotiate MTU 517 and fragment large payloads.

Windows (Rust `windows` crate, WinRT) in the desktop app:
- BluetoothLEAdvertisementPublisher / BluetoothLEAdvertisementWatcher.
- GattServiceProvider for the GATT server, and the GATT client side.
- If WinRT from Rust gets blocked on something, stop and tell me; plan B in SPEC.md
  is a small C# helper process.

Core:
- The ephemeral X25519 + HKDF + ChaCha20-Poly1305 encryption of the GATT payloads,
  with tests, so both platforms call the same code.

For now, after the GATT exchange, still connect over LAN. The goal of this phase is:
devices DISCOVER each other over BLE even when they're on different Wi-Fi networks or
none at all.

Done when: with Wi-Fi disconnected from any network on both devices, each one appears
in the other's nearby list within ~3 seconds.
```

---

## Phase 8 — Direct Wi-Fi link with no network (weeks 8–9) ★ the "wow" phase

```
Phase 8: implement link negotiation per SPEC.md section 5.

Order of work:
1. Android local-only hotspot path FIRST (most reliable):
   - Phone: WifiManager.startLocalOnlyHotspot(), send SSID/password in the encrypted
     GATT payload.
   - Laptop: join via Windows.Devices.WiFi.WiFiAdapter (scan, find the SSID,
     ConnectAsync with the password), get the phone's IP (the hotspot gateway),
     connect over TCP, run the normal Noise + transfer.
   - After the transfer: stop the hotspot and reconnect the laptop to its previous
     Wi-Fi network automatically.
2. Then Wi-Fi Direct P2P (Android WifiP2pManager ↔ Windows WiFiDirect APIs), so the
   laptop can keep its internet connection. Expect interop problems: log everything,
   and record every issue in LEARNINGS.md.
3. Negotiation: try same-LAN (1.5 s TCP timeout) → Wi-Fi Direct → hotspot, and
   record the link type and setup time in transfer stats.

Done when: BOTH devices have no shared network (airplane mode with only Bluetooth and
Wi-Fi radios on, not joined to any network), and I can send a 1 GB file
phone → laptop and laptop → phone at good speed. Report the measured MB/s and setup time.
```

---

## Phase 9 — Extras (week 10, first half)

```
Phase 9: implement SPEC.md section 10 items 1 and 2.

1. Clipboard: "Send clipboard" button on both apps, using the CLIPBOARD message.
   Optional auto-sync for trusted devices (desktop watches the clipboard; Android
   sends on tap because of background clipboard restrictions; explain this in
   DECISIONS.md).
2. Auto-lock: the desktop tracks the trusted phone's BLE RSSI. If it stays below a
   configurable threshold for 10 s, call LockWorkStation(). Add a settings toggle
   and a calibration screen ("hold your phone next to the laptop... now walk away").

Done when: both work reliably on my devices, with sensible defaults.
```

---

## Phase 10 — Benchmarks, polish and launch (week 10, second half)

```
Phase 10: get Beam ready to show and share.

- Run the full benchmark suite from SPEC.md section 12 on real devices (I'll give you
  my device models). Produce charts. Help me write a fair comparison against LocalSend
  and Quick Share (I'll time those by hand; give me a step-by-step method).
- Run cargo-fuzz on the frame decoder for at least 10 minutes and fix any crash.
- Go through the security checklist in SPEC.md 6.4 item by item and confirm each one
  with a test reference.
- README: GIF demo at the top, one-paragraph pitch, features, architecture diagram
  (mermaid), how it works (discovery → link → Noise → transfer), benchmark chart,
  security model, install steps, roadmap.
- Release builds: signed APK on GitHub Releases, Windows .msi from Tauri.
- Check every box in SPEC.md section 13 and report anything still unchecked.
```

---

## Learn it (use after EVERY phase)

```
Now act as a strict interviewer from Google or Atlassian. I built this phase of Beam.
1. Explain how this part works in simple terms, as if I'm learning it for the first
   time, using the actual files and function names from our code.
2. Then ask me 5 interview questions about it, one at a time: design choices, edge
   cases, "what if" scaling questions, and "why not X instead?". Wait for my answer
   to each, tell me what I got right or wrong, and give the ideal answer.
```

## When something breaks

```
<paste the exact error / logs / what you saw>

This is happening in phase N. Don't guess. First list the 3 most likely causes based on
the code and logs, explain how to confirm each one, then fix the real root cause.
Add the bug to docs/LEARNINGS.md once it's fixed.
```

## Resume and interview prep (after you finish)

```
Read docs/SPEC.md, docs/DECISIONS.md, docs/LEARNINGS.md and bench/results.csv.
1. Write 3 resume bullet points for Beam using my REAL benchmark numbers only.
2. Write a 60-second spoken pitch for "tell me about a project you're proud of".
3. List the 15 most likely interview questions about this project, with model answers
   based on what we actually built and the bugs in LEARNINGS.md.
```

# Beam

**AirDrop for Android ↔ Windows:** fast, end-to-end encrypted file and clipboard
sharing that works even with no shared Wi-Fi network and no internet.

> **Status: in development.** Nothing is usable yet. Follow progress in
> [docs/SPEC.md](docs/SPEC.md).

## Repository layout

| Folder | What's in it |
|---|---|
| `core/` | `beam-core` (Rust): protocol, crypto, transfer engine, resume, mDNS |
| `core-ffi/` | UniFFI wrapper exposing `beam-core` to Kotlin |
| `cli/` | `beam-cli`: send / receive / discover from a terminal |
| `desktop/` | Tauri v2 desktop app (later phase) |
| `android/` | Android app, Kotlin + Jetpack Compose (later phase) |
| `bench/` | Benchmark scripts and results (later phase) |
| `docs/` | Spec, protocol, design decisions, learnings |

## Building

Requires stable Rust (installed via [rustup](https://rustup.rs)). On Windows,
also install the Visual Studio C++ Build Tools.

```bash
cargo test --workspace
```

## Trying it (phase 2: encrypted, CLI only)

Every connection is end-to-end encrypted with Noise XX. On first run each
device creates a key pair in its config folder (`%APPDATA%\Beam` on Windows).

In one terminal, start a receiver:

```bash
cargo run --release -p beam-cli -- receive --port 9000 --dir ./inbox
```

In another terminal, send files to it:

```bash
cargo run --release -p beam-cli -- send --to 127.0.0.1:9000 photo.jpg notes.txt
```

Both terminals show the same 6-digit **verification code**. The receiver asks:
`[y]es`, `[n]o`, or `[t]rust`. Trust remembers the sender's key and accepts
its future transfers automatically.

To try two separate devices on one machine, give each its own identity with
`--config-dir`:

```bash
cargo run --release -p beam-cli -- --config-dir ./device-b --name Device-B send --to 127.0.0.1:9000 photo.jpg
```

Other commands:

| Command | What it does |
|---|---|
| `beam-cli identity` | Show this device's id (fingerprint) and public key |
| `beam-cli trust list` | List trusted devices |
| `beam-cli trust remove <id>` | Stop trusting a device (takes effect immediately, even for a running receiver) |

To send between two machines, use the receiver's LAN IP address. The wire
format is described in [docs/PROTOCOL.md](docs/PROTOCOL.md).

### Checking the encryption with Wireshark

1. Install [Wireshark](https://www.wireshark.org/download.html). On Windows,
   keep **Npcap** selected in the installer and tick **"Support loopback
   traffic"**.
2. Start capturing on **"Adapter for loopback traffic capture"** (or your
   Wi-Fi adapter if the devices are on different machines). Use the capture
   filter `tcp port 9000`.
3. Create a text file with an easy-to-spot line, such as
   `BEAM-SECRET-MARKER` repeated many times, and send it as above.
4. Stop the capture and right-click any packet → **Follow → TCP Stream**.
   - In phase 2 you'll see only random-looking bytes: no file name, no device
     names, no marker text.
   - Search with **Edit → Find Packet → String → "BEAM-SECRET-MARKER"**
     (search in "Packet bytes"). It finds nothing.
5. For contrast, check out the `phase 1` commit and repeat. The file name,
   device names and the marker are all readable in the TCP stream.

The test `nothing_readable_crosses_the_wire` in `core/tests/transfer.rs`
checks the same thing automatically, using a recording proxy.

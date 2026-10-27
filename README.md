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

## Trying it (phase 1: plain TCP, no encryption yet)

In one terminal, start a receiver. It accepts every transfer automatically:

```bash
cargo run --release -p beam-cli -- receive --port 9000 --dir ./inbox
```

In another terminal, send files to it:

```bash
cargo run --release -p beam-cli -- send --to 127.0.0.1:9000 photo.jpg notes.txt
```

To send between two machines, use the receiver's LAN IP address. The wire
format is described in [docs/PROTOCOL.md](docs/PROTOCOL.md).

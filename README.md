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

Requires stable Rust (installed via [rustup](https://rustup.rs)).

```bash
cargo test --workspace
cargo run -p beam-cli
```

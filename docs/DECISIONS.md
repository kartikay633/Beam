# Decisions

Append-only log of important choices: what was decided, why, and which
alternatives were considered. Newest entries go at the bottom.

## 2026-10-27 — Cargo workspace with `beam-core`, `beam-core-ffi`, `beam-cli`

**What:** One Cargo workspace at the repo root with the crates `core/`,
`core-ffi/` and `cli/`. The packages are named `beam-core`, `beam-core-ffi` and
`beam-cli`; the folders keep the short names from SPEC.md.

**Why:** A workspace shares one `Cargo.lock`, one `target/` directory and one
set of dependency versions, so the CLI, the desktop app and the Android
bindings always build against the same core. The `beam-` prefix is needed
because a package named `core` would clash with Rust's built-in `core` crate.

**Alternatives:** Separate repositories per component (harder to keep the
protocol in sync); a single crate with feature flags (mixes FFI and CLI
dependencies into the core library).

## 2026-10-27 — Pin the stable toolchain and LF line endings

**What:** `rust-toolchain.toml` selects `stable` with `rustfmt` and `clippy`;
`.gitattributes` stores text files with LF endings.

**Why:** CI runs on both Ubuntu and Windows. Pinning the channel and the line
endings means `cargo fmt --check` gives the same answer on every machine.

**Alternatives:** Pinning an exact Rust version (more reproducible, but needs
manual bumps; revisit if a stable release ever breaks the build).

## 2026-10-27 — Frames may span several records (deviation from SPEC.md 7.1)

**What:** SPEC.md 7.1 says every Noise transport message (max 65,535 bytes)
holds exactly one frame. Instead, the stream is split into *records* (one
record = one future Noise message), and each frame is `frame_len:u32 | frame`
spread over as many records as it needs. A record never mixes two frames, and
DATA frames are sized to fit in exactly one record, so the hot path matches the
spec's intent.

**Why:** A single 64 KiB frame is too small for real control messages. An
OFFER for 1,000 phone photos (`IMG_20261027_123456.jpg`, size, MIME, mtime)
is about 85 KB of CBOR, and SPEC.md 7.3 explicitly wants 1,000-file transfers.
CLIPBOARD text can also be larger than 64 KB. Frames are capped at 4 MiB so a
hostile peer still can't make us allocate much.

**Alternatives:**
- Split a big OFFER into several OFFER frames. This changes the meaning of
  OFFER, needs "more to come" bookkeeping, and doesn't help CLIPBOARD.
- Compact CBOR (integer keys or arrays). This only moves the limit, and long
  file names still break it.
- TLS-style packing of several frames into one record. It saves a few bytes
  for tiny frames, at the cost of more complex parsing.

## 2026-10-27 — Encryption plugs in through a `RecordCipher` trait

**What:** `Connection` reads and writes records through
`RecordCipher::seal`/`open`. Phase 1 uses `PlainCipher` (identity). Phase 2
adds a Noise implementation without touching framing, messages, the sender or
the receiver. Record plaintext is already limited to 65,519 bytes (65,535 minus
the 16-byte tag), so frame sizes won't change when encryption arrives.

**Alternatives:** Wrapping the TCP stream in an `AsyncRead`/`AsyncWrite` adapter
that encrypts. That works, but it hides record boundaries, which Noise needs.

## 2026-10-27 — CBOR maps with text keys for control messages

**What:** Control bodies are serde's default struct encoding: a CBOR map keyed
by field name. Decoders ignore unknown keys.

**Why:** Self-describing, so it's easy to debug with any CBOR tool, and new
optional fields don't break older peers (minor versions). The extra bytes only
affect control messages; file data uses the binary DATA header.

**Alternatives:** Integer keys or arrays (smaller, but brittle and unreadable);
protobuf (needs a schema compiler and another dependency).

## 2026-10-27 — File-name sanitizer keeps only the last path component

**What:** `../../etc/passwd` becomes `passwd`, and `C:\Users\x\a.txt` becomes
`a.txt`. Separators are not turned into `_` to keep the "path" as part of the
name.

**Why:** Beam sends files, not folders, so a sender path never has meaning on
the receiver. Keeping only the last component is the simplest rule that can't
produce traversal, and it gives the name the user expects. The full rule list
and its test table are in `core/src/sanitize.rs`. Windows name collisions are
checked case-insensitively on every platform.

## 2026-10-27 — Phase 1 simplifications, to be replaced later

- **Partial files are deleted on failure.** Phase 3 keeps them (with a
  progress file) for resume.
- **DATA must arrive in order per file**, and the receiver writes
  sequentially. Phase 3 adds positional writes for parallel streams.
- **The device id is random per run.** Phase 2 derives it from the static key.
- **No disk-space check before accepting.** This SPEC.md 6.4 item is still
  open (it needs a platform-specific API).
- **The sender writes without reading** until all files are sent. If the
  receiver cancels mid-transfer, the sender sees a connection error rather
  than the CANCEL reason. Phase 3's control and data connection split fixes
  this.

## 2026-10-27 — Units, DATA size and test build settings

- **Speeds are reported in MB/s with 1 MB = 1,000,000 bytes.** This is the
  usual convention for network benchmarks, and it keeps README numbers
  comparable to other tools.
- **The DATA payload is 63 KiB (64,512 bytes),** so `frame_len + type + header
  + payload` fits in one 65,519-byte record.
- **`[profile.dev.package."*"] opt-level = 3`:** dependencies such as blake3
  are optimized even in debug and test builds, so the 200 MB integration test
  finishes in about a second instead of minutes. Our own crates still compile
  quickly.

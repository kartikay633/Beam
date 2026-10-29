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

## 2026-10-27 — Noise XX instead of TLS with self-signed certificates

**What:** Every connection starts with a `Noise_XX_25519_ChaChaPoly_BLAKE2s`
handshake through the `snow` crate. Each record after it is one Noise
transport message.

**Why:**
- **No certificate infrastructure.** Beam devices identify each other by a raw
  public key (pinned through QR pairing or the verification code). TLS is
  built around certificates and CA chains. With self-signed certificates we
  would have to turn off chain validation and write custom pinning callbacks,
  which is exactly where TLS setups usually go wrong.
- **Mutual authentication is built in.** In XX, both sides send their static
  key, encrypted, and prove they own it. With TLS, client certificates are an
  optional extra that many libraries handle awkwardly.
- **Small and auditable.** One fixed cipher suite, no negotiation, and no
  version or extension zoo, so there's nothing to downgrade. `snow` is a
  focused implementation of a published spec. We don't invent any crypto
  ourselves.
- **A handshake hash for free.** Both sides end with the same handshake hash.
  The 6-digit verification code and (in phase 3) the data-stream
  `session_token` are derived from it.
- **Easy to write a compatible client:** Noise libraries exist for Kotlin,
  C, Go and others.

**Alternatives:**
- TLS 1.3 with self-signed certificates and pinning (rustls): mature and fast,
  but certificate generation, storage and custom verifiers add complexity for
  no gain here.
- Noise IK: one fewer round trip, but the initiator must already know the
  responder's static key, which isn't true for unknown devices. XX works for
  both first contact and trusted peers.
- Our own construction (X25519 + HKDF + ChaCha20-Poly1305): no. That's the
  "don't invent crypto" rule.

**Details:**
- **Prologue:** `beam/1` is mixed into the handshake, so a Beam handshake
  can't be confused with another Noise protocol.
- **Handshake timeout:** 10 s, so a peer that stalls can't tie up the
  receiver.

## 2026-10-27 — HELLO's device id must match the key from the handshake

**What:** After the handshake, each side checks two things:
1. `HELLO.device_id` is derived from the static key the handshake proved.
2. If that id is trusted, the proven key equals the stored key exactly.

Either failure closes the connection.

**Why:** The handshake proves who holds a key; HELLO is just a claim. Binding
the two means an attacker can't show a trusted device's id (or a familiar
name) with its own key. The best it can do is appear as an unknown device,
which is never auto-accepted and shows a verification code that won't match.

**Alternatives:** Trusting by name, or by id alone. Names are free text, and
the 8-byte id is a short fingerprint, so trust always compares the full
32-byte key.

## 2026-10-27 — Verification code from the handshake hash

**What:**
`code = u64_le(BLAKE3-derive_key("beam 2026-10-27 verification code v1", handshake_hash)[0..8]) mod 10^6`

**Why:** Both ends of one Noise session share the handshake hash. A
man-in-the-middle runs two sessions with different hashes, so the codes
differ with probability 1 − 10⁻⁶. BLAKE3's `derive_key` mode adds domain
separation, so the code can't collide with other values derived from the same
hash (such as the phase 3 `session_token`). Taking 64 bits mod 10⁶ makes the
bias negligible.

## 2026-10-27 — Identity and trust storage for the CLI

**What:**
- `identity.json` holds the private and public key in hex. It is created
  with mode 0600 on Unix and lives in the user's config folder
  (`%APPDATA%\Beam` on Windows, `--config-dir` to override).
- `trusted_devices.json` lists `{device_id, name, public_key, auto_accept}`.
  It is written to a temp file and renamed into place, so a crash never leaves
  it half-written. On load, each stored id is checked against its key.

**Why:** Phase 2 asks for a config-dir file for the CLI. The desktop app
moves the key to DPAPI / Credential Manager in phase 5, and Android to the
Keystore in phase 6. The core only needs `Identity` and `TrustStore`, not the
storage backend.

## 2026-10-27 — Trust store re-reads the file on every lookup

**What:** `TrustStore::get`, `list`, `upsert` and `remove` re-read
`trusted_devices.json` each time. If the file can't be read or parsed, lookups
fail (fail closed) instead of falling back to a cached copy.

**Why:** Found by hand-testing. A running `beam-cli receive` kept its own copy
of the trust list, so `beam-cli trust remove` in another terminal didn't
revoke trust until a restart: the removed device was still auto-accepted.
Revocation has to take effect immediately. The file is tiny, so re-reading
it per connection costs nothing. Re-reading before each change also stops
one process from overwriting another's edits with a stale list. The
regression test is `removal_by_another_process_takes_effect_immediately`.

**Alternatives:** Watching the file for changes (more code, and platform
differences); caching with an mtime check (timestamps can be coarse, so a
quick remove-then-add could be missed).

## 2026-10-27 — Phase 1 follow-up: device ids

The phase 1 note "the device id is random per run" no longer applies. It is
now derived from the persisted static key, as described above.

## 2026-10-27 — Phase 3 transfer engine: control + data connections

**What:**
- One control connection carries HELLO, OFFER, ACCEPT, CHUNK_ACK,
  CHUNK_RETRY, FILE_DONE, PING and TRANSFER_COMPLETE.
- N data connections (default 4, up to 16) carry only DATA and CHUNK_HASH.
- Each data connection runs its own Noise handshake and must prove the *same
  static key* as the control connection. Its STREAM_OPEN carries a session
  token derived from the control handshake hash.
- A chunk (4 MiB) always travels whole on one data connection. Each stream
  takes the next chunk from a shared queue when it's free (work stealing).

**Why:**
- A single TCP connection rarely fills a fast link: one congestion window, and
  one core doing encryption. Parallel connections spread both.
- Keeping each chunk on one stream means the receiver can verify a chunk with
  no cross-stream reassembly.
- Data connections authenticate with their own handshake, so the session
  token only has to *route* them, not prove anything. That matters because
  the token is derived from the handshake hash, which an eavesdropper can
  compute too.

**Alternatives:**
- Multiplexing streams over one connection (QUIC-style): it doesn't get
  around a single TCP connection's limits.
- Striping each chunk across all streams: needs reordering and only verifies
  once every stream has delivered.

## 2026-10-27 — Snow's stateless transport so connections can be split

**What:** After the handshake, `into_stateless_transport_mode()` produces a
state shared by a sealer and an opener, each with its own nonce counter. A
`Connection` splits into a `FrameReader` and a `FrameWriter` used by
different tasks.

**Why:** The control connection must read acks while the session writes
FILE_DONE and PINGs. Reading a frame isn't cancel-safe, so the reader lives
in its own task. Snow's regular `TransportState` needs `&mut` for both
directions, so it can't be shared that way.

## 2026-10-27 — New message CHUNK_RETRY (0x0E) (spec gap)

**What:** The receiver sends `CHUNK_RETRY {file_id, chunk_index}` on the
control connection when a chunk fails its CHUNK_HASH, or when a finished file
fails its whole-file check. The sender puts the chunk back at the front of
the queue. Five requests for the same chunk end the transfer.

**Why:** SPEC.md 7.3 says a mismatch "means re-requesting that chunk", but the
message table in 7.2 has no message for it.

**Alternatives:**
- Drop the data connection and let resume handle it: it works, but costs a
  reconnect and throws away other in-flight chunks.
- An optional field on CHUNK_ACK: it mixes positive and negative acks in one
  message.

## 2026-10-27 — One progress file per transfer, not one per file (deviation from SPEC.md 7.3)

**What:** SPEC.md describes a `name.beam-part.json` next to each part file.
Instead there is one manifest per transfer,
`<download dir>/.beam-partial/<transfer_id>.json`, holding every file's
fingerprint, reserved name, chunk bitmap and finished hash. It's written at
ACCEPT time and then at most every 2 s.

**Why:**
- A 1,000-file transfer would otherwise create and rewrite 1,000 extra JSON
  files.
- Writing the manifest at ACCEPT means "the user accepted this transfer"
  survives a drop that happens before any chunk lands, so a reconnect never
  asks again.
- One atomic rename updates everything at once.
- Part files stay next to their final names, as in the spec.

## 2026-10-27 — Verify before writing; bounded queues keep memory flat

**What:**
- A data stream buffers one chunk (at most 4 MiB), hashes it as it arrives,
  and hands it to the disk writer only if CHUNK_HASH matches.
- The writer is one blocking thread behind a 4-chunk channel and uses
  positional writes into preallocated part files.
- The sender reads one chunk per stream and has a 16 MiB unacknowledged
  window per stream.

**Why:** Nothing unverified ever touches the disk, and a bitmap bit always
means "verified and written". Memory is bounded by streams × 4 MiB plus the
4-chunk queue on each side, whatever the file size. Peak memory is measured
by the benchmark runner (`bench/raw_results.csv`).

## 2026-10-27 — Durability: sync partial files at checkpoints, not every file

**What:**
- Before each progress-file save, the writer fsyncs the part files that are
  still partially written.
- A file whose bytes are all written is closed without an fsync. It is then
  checked against FILE_DONE's whole-file hash and renamed.
- If that final check fails (for example data lost in a power cut before it
  reached the disk), the receiver re-requests the file's chunks with
  CHUNK_RETRY instead of failing.

**Why:** An fsync per small file made 1,000 × 10 KB files take seconds (see
LEARNINGS.md). Resume needs the *claims* in the progress file to be true, and
syncing before each save guarantees that for big files, where re-sending is
expensive. Small files are cheap to re-send, and the end-to-end hash check
catches anything lost.

**Alternatives:**
- Fsync every file before renaming: safest, but far too slow on Windows.
- No syncing at all: a power cut could make a big file's progress file claim
  chunks that are gone, and the final check would then re-request the whole
  file.

## 2026-10-27 — Whole-file hashes: no extra read for one-chunk files

**What:**
- For a file of one chunk, the chunk hash *is* the whole-file hash.
  - The sender takes it from the stream that sent the chunk.
  - The receiver takes it from the verified chunk.
- Multi-chunk files are hashed once more from disk: in the background on the
  sender, and after the last chunk on the receiver (so the receiver also
  verifies what's actually on disk). Both use 4 MiB blocks and all cores
  (`update_rayon`) rather than memory-mapping.

**Why:**
- An extra open and read per small file doubled the per-file cost.
- Memory-mapping a 1 GB file counted 950 MB toward the process's resident
  memory (see LEARNINGS.md).

## 2026-10-27 — CHUNK_ACK batching thresholds

**What:** The receiver flushes CHUNK_ACKs when 4 MiB of chunks or 64 chunks
are pending, and otherwise every 250 ms.

**Why:** With only a timer, one stream could have at most its 16 MiB window
in flight per 250 ms, which caps it at 64 MB/s. Acking every 4 MiB keeps the
window open, and the 64-chunk limit stops thousands of tiny files from
producing one ack each.

## 2026-10-27 — The receiver is a concurrent server; reconnects take over

**What:**
- `Receiver::run(handler)` serves any number of connections; the front-end
  implements `ReceiveHandler` (`decide`, `progress`, `finished`,
  `connection_refused`), which also suits the coming FFI callback interface.
- If a resumed offer arrives while the old session of the same transfer is
  still running (it hasn't noticed the drop), the new session cancels the old
  one and waits until it has saved its progress.

**Why:**
- Data connections arrive on the same port while a transfer runs, so one
  transfer at a time can't work.
- Without takeover, a quick reconnect would race the old session over the same
  part files.

## 2026-10-27 — Reconnect policy

**What:** The sender retries only after the receiver has accepted once, and
only for network-level failures (I/O error, closed connection, timeout).

- **Backoff:** 100 ms, 200 ms, 400 ms, 800 ms, then 1.6 s.
- **Time limit:** up to 60 s after the last drop.
- **Same receiver:** the receiver's key is pinned from the first session, and
  a reconnect that reaches a different key is refused.

**Why:**
- Retrying a rejection, a protocol error or an impersonation would be wrong.
- Retrying before acceptance would show the user a second prompt.
- Pinning the key stops a man-in-the-middle from slipping in during a
  reconnect.

## 2026-10-27 — Benchmarks in Python with a venv on D:

**What:**
- `bench/generate.py`, `bench/run.py` and `bench/chart.py`, with
  matplotlib and psutil installed in a virtual environment.
- On this machine, the venv, test data and scratch files all live on D:. The
  scripts take `--data-dir` and `--work-dir`.
- `run.py` drives the real `beam-cli` (release build) with `--json` output.
  It records every run in `raw_results.csv` and medians in `results.csv`,
  including peak memory of both processes.

**Why:** It measures the shipped binary end to end, and Python is the quickest
way to make charts. Loopback numbers measure the software (crypto, hashing,
disk), not the network; LAN numbers need a second device.

## 2026-10-27 — Trusted-only visibility over mDNS: an anonymous, rotating announcement

**What:** In trusted-only mode the receiver still announces `_beam._tcp`, but
the TXT record has no name and no id. It has only
`rid = BLAKE3-keyed(derive_key(public key), 15-minute window)[0..8]`, and the
instance and host names are built from `rid`. Devices that trust it compute
the same `rid` from the public key in their trust store and show it by the
name they gave it. Everyone else gets an anonymous entry that changes every
15 minutes.

**Why:** mDNS is multicast: every announcement reaches every device on the
network, so "must not appear to unknown devices" can't be done by choosing who
receives it. It can only be done by making the announcement meaningless to
them. This is the stretch goal of SPEC.md 4.2 (rotating identifiers), applied
to mDNS. On top of that, the receiver declines offers from untrusted devices
in this mode, so knowing the port isn't enough.

**Trade-off:** The tag is keyed by the *public* key. Any device that has ever
learned it can recognize the announcement, including one that connected once
without being trusted, since the handshake reveals the key. A tag per trusted
peer, keyed by an X25519 shared secret, would close that gap. But it needs one
tag per peer (TXT size limits) and a pairing step that doesn't exist yet
(phase 5/6), so it stays a possible upgrade.

**Alternatives:**
- Not announcing at all in trusted-only mode: trusted devices couldn't find it
  either, without a separate unicast mechanism.
- Announcing normally and only refusing offers: that doesn't meet "must not
  appear".

## 2026-10-27 — mDNS results are hints; the sender pins the expected device id

**What:**
- `send --to <name or id>` resolves the device over mDNS, then sets
  `SendOptions::expect_device`. If the handshake proves a different device
  id, the sender stops before sending the OFFER (`UnexpectedPeer`).
- A name matching several devices is an error that lists their ids.

**Why:** Anyone on the network can announce "Kartikay's Laptop". Without this
check, a spoofed announcement would at best get the user an unexpected
verification code, and at worst a mistaken "yes" from someone hurrying.

## 2026-10-27 — Announced addresses: loopback last; no host name broadcast

**What:** The advertiser announces every interface address (mdns-sd's
`enable_addr_auto`). Browsers order them: routable IPv4, routable IPv6,
link-local IPv4, link-local IPv6, then loopback. The sender takes the first
that accepts a TCP connection within 1.5 s (SPEC.md 5). Instance and host
labels are `beam-<id>` or `beam-<rid>`.

**Why:**
- Found while testing: 127.0.0.1 sorted first. From another machine it would
  reach the *sender's own* computer (possibly its own Beam receiver), and the
  only thing stopping that was the id check.
- The computer's host name is often personal ("Kartikays-Laptop") and would
  defeat trusted-only anonymity, so it's never used.

## 2026-10-27 — Visibility is enforced by the receiver, not just by mDNS

**What:** `ReceiverConfig.visibility` is checked for every OFFER, before
anyone is asked.

- **Off** declines everything, including resumes.
- **Trusted-only** declines untrusted senders without asking, but lets an
  already-accepted transfer resume (for example when "everyone" expires in
  the middle of a long transfer).
- **Everyone** falls back to trusted-only after 10 minutes, as on AirDrop.

`Visibility` is a shared handle that can change at runtime (a tray menu in
phase 5). The advertiser follows its changes, its expiry and the rotation
windows.

**Why:** SPEC.md 6.4 says "Decline everything when visibility is Off, even if a
connection reaches the port". Discovery only hides a device; it can't stop a
sender that already knows the address.

## 2026-10-27 — Desktop app layout and build

**What:**
- `desktop/` is the npm project (Vite root `desktop/ui`, output
  `desktop/dist`), and `desktop/src-tauri` is a workspace member.
- The workspace's `default-members` leave the desktop crate out, so
  `cargo build` / `cargo test` at the root (and the Linux CI job) don't need
  WebView or GTK libraries.
- A separate Windows CI job builds the UI, runs clippy and the tests on the
  desktop crate, and builds the MSI.

**Why:** Tauri's CLI looks for `src-tauri` next to `package.json`, and
SPEC.md asks for `desktop/src-tauri` and `desktop/ui`. Keeping the crate in
the workspace shares `Cargo.lock` and the build cache with `beam-core`.

## 2026-10-27 — The UI renders snapshots pushed by the backend

**What:** The Rust side owns all state:
- nearby devices, transfers, staged files, pending offers;
- settings and trust (read from their stores).

On any change it emits one `snapshot` event with everything. React just
renders the latest snapshot; user actions are commands
(`send_staged`, `cancel_transfer`, `respond_offer`, …). Progress-only updates
are throttled to one snapshot every 150 ms.

**Why:** There is one source of truth, so there's no state to keep in sync
between Rust and JavaScript, and a reload or a new window (the offer popup)
can't show something stale. Snapshots are a few kilobytes, so sending
everything is cheaper than designing fine-grained events. The same shape will
suit the Android UI's view model.

**Alternatives:** Fine-grained events (device-found, progress, …) merged
into React state: more code on both sides, and easy to drift.

## 2026-10-27 — Incoming offers: a separate always-on-top window

**What:** `ReceiveHandler::decide` opens a `WebviewWindow` labelled
`offer-<id>` (always on top, focused, not resizable) that loads
`#/offer/<id>`, and waits on a oneshot channel for the answer, for up to
60 s. A timeout or a closed popup declines with "no answer". Ticking "trust
this device" adds the sender to the trust store with auto-accept.

**Why:** SPEC.md 9 asks for a small always-on-top popup. A separate window
appears over whatever the user is doing even when the main window is hidden
in the tray. The 60 s limit matches the sender's patience and stops a
forgotten prompt from holding a session open.

## 2026-10-27 — Device key encrypted with DPAPI

**What:** On Windows the key pair is stored as `identity.dpapi`:
`CryptProtectData` over the 64 bytes, with app-specific entropy
(`"beam device identity v1"`) and `CRYPTPROTECT_UI_FORBIDDEN`. beam-core
gained `Identity::export_secret` / `import_secret` so any platform keystore
can hold the bytes.

**Why:** DPAPI ties the secret to the Windows user account and machine, with
no password to manage. The entropy means another app running as the same
user can't simply ask DPAPI to decrypt our file without knowing it.

**Alternatives:** Credential Manager (`CredWrite`): it's encrypted with DPAPI
underneath anyway and limited to small blobs, and it adds a second API for no
gain. A plain file (the CLI's approach): readable by any process running as
the user.

## 2026-10-27 — Explorer "Send to → Beam" via a shortcut and single-instance

**What:**
- On start-up (and when the setting is turned on) the app writes
  `%APPDATA%\Microsoft\Windows\SendTo\Beam.lnk` with `IShellLinkW` /
  `IPersistFile`, on its own thread with its own COM apartment.
- `tauri-plugin-single-instance` forwards the command line of any second
  launch to the running app, which stages those files and shows its window.

**Why:** It's the standard way to appear in Send To, and needs no installer
or registry entries. Rewriting the shortcut at start-up keeps it pointing at
the current executable after an update or a move.

## 2026-10-27 — Cancelling: CANCEL first, then close

**What:**
- **Sender:** `SendOptions::cancel` (a watch channel) can stop a send at any
  point: while waiting for ACCEPT, mid-transfer, or between reconnects. The
  sender sends CANCEL and returns `CancelledByUser`.
- **Receiver:** a `ReceiverHandle` cancels a running incoming transfer, which
  sends CANCEL **before** closing its data connections.
- **Sender side of a broken data connection:** the sender first waits briefly
  (500 ms) for a CANCEL on the control connection before treating it as a
  network failure.

**Why:** Found by a test (see LEARNINGS.md). If the data connections close
first, the sender sees a network drop and reconnects, so the cancelled
transfer starts again as a fresh offer.

## 2026-10-27 — `ReceiveHandler::accepted` callback

**What:** Called once an offer is accepted by any route: the user, a
trusted device with auto-accept, or a resume.

**Why:** Auto-accepted and resumed transfers never reach `decide`, but a UI
still has to show them with their file names and sizes.

## 2026-10-27 — Demo mode for UI work

**What:** When the page isn't running inside Tauri (no
`__TAURI_INTERNALS__`), `ui/src/api.ts` answers commands from built-in demo
data.

**Why:** Every screen, including the offer popup, can be designed and checked
in an ordinary browser without building the Rust side. It's how the screens
were reviewed before the first native run.

## 2026-10-28 — Pairing: a one-time token in the QR code

**What:** The `beam://pair` code carries a random 16-byte token, valid once
for 5 minutes. The phone checks the handshake proved the code's key, then
sends PAIR_REQUEST with the token; the computer trusts the phone's key only
for a valid token (PROTOCOL.md, "Pairing exchange").

**Why:** SPEC.md 6.3 says the phone "sends its own key back over the
authenticated session", but an authenticated session only proves who the
phone is, not that it is the phone in front of the screen. Without a secret
from the screen, anyone on the network who had seen an mDNS announcement
could make the laptop trust them. One use and a short lifetime limit what a
photographed code is worth.

## 2026-10-28 — Kotlin API: one `BeamNode`, events through one listener

**What:** core-ffi exposes a `BeamNode` object (UniFFI 0.32). Long operations
are Kotlin `suspend` functions (`send`, `pair`); everything else (devices
found and lost, offers, progress, results, pairing) arrives through a single
`BeamListener` callback interface. Offers are answered later with
`respondOffer`, and wait at most 60 seconds. The node owns its tokio runtime.

**Why:** A suspend function per transfer would leave incoming transfers,
which nobody "calls", without a place to report from; one listener covers
both directions the same way. Keeping the runtime inside the node means no
Kotlin call ever blocks on the network except `start` (binding a socket).

## 2026-10-28 — Content URIs: file descriptors through the FFI

**What:** The app opens each shared `content://` URI with
`ContentResolver.openFileDescriptor`, detaches the descriptor and passes it
to Rust as `FileSource.Fd`; core takes ownership and reads chunks with
positional reads (`pread`). If the descriptor isn't seekable (a pipe from a
cloud provider) or its size is unknown, the app copies the file to its cache
first and passes that path instead. Core's sender now accepts any open file
(`SendSource::open_file`) besides a path.

**Why:**
- Core sends with parallel streams, each reading chunks at arbitrary
  offsets, and hashes the file before offering it. A stream through a
  callback would serialize all of that, and add a JNI call per read.
- Descriptors keep "read through `ContentResolver`, never raw paths"
  (SPEC.md 8) without copying a 5 GB video before sending it.
- Ownership moves to Rust at the call, so nothing has to agree on when to
  close it; if one file in a batch fails to open, every descriptor already
  handed over is still closed.

## 2026-10-28 — Receive into app storage, then move to Download/Beam

**What:** Incoming files are written to app-private storage
(`filesDir/incoming`). When a transfer completes and every hash checks out,
the app copies each file into `MediaStore.Downloads` under `Download/Beam`
(with `IS_PENDING` while copying) and deletes the private copy.

**Why:** The receiver needs positional writes from several streams, a
`.beam-partial` manifest beside the file, and renames to resume after a
dropped connection, and MediaStore URIs offer none of these. `filesDir`,
not the cache, so Android doesn't delete half a transfer that is waiting to
resume. The cost is one local copy per received file, after the transfer.

## 2026-10-28 — Android discovery: NsdManager by default, core's mDNS as an option

**What:** Both work, and Settings → "Finding devices" switches between them:
- **Android (NsdManager)**, the default (SPEC.md 4.1). Core still decides
  what to announce: with `Discovery.PLATFORM` the node tells the app what to
  register (`advertisementChanged`, including every 15-minute change of the
  trusted-only rotating id and the end of "everyone"), and the app passes
  what NsdManager resolves back with `serviceFound`/`serviceLost`, where core
  parses the TXT record, hides our own announcement and recognizes trusted
  devices.
- **Beam's own mDNS** (`mdns-sd`, as on the desktop), holding a Wi-Fi
  `MulticastLock` while it runs.

**Why:** PROMPTS.md asks to test both and document the choice, but there is
no phone in this phase's setup, so neither could be tried on a device yet.
NsdManager is the default because it is the system's own service: it shares
one multicast socket with other apps, survives Wi-Fi changes, and needs no
multicast lock, which costs battery. Core's mDNS is kept because some
NsdManager versions are known to be unreliable (resolve failures before
Android 14). The TXT format and rotating id stay in Rust either way, so the
two can't drift apart. **To do on a device:** compare how fast each finds
the laptop, and whether trusted-only devices are recognized, then revisit
the default.

## 2026-10-28 — Foreground service while Beam is on; locks only while busy

**What:** `BeamService` (type `connectedDevice`) runs whenever the node runs,
so the phone can receive with the app closed. Its notification shows who
can see the phone, or the transfer's progress with Cancel. A
`WIFI_MODE_FULL_LOW_LATENCY` Wi-Fi lock and a partial wake lock are taken
when a transfer starts moving and released as soon as none is, including
while a received transfer waits for its sender to reconnect. "Stop" in the
notification stops receiving.

**Why:** SPEC.md 8. Receiving can't wait for the user to open the app, and
Android only lets a foreground service keep the network alive in the
background. Holding the locks only during transfers keeps idle battery use
to the service itself.

## 2026-10-28 — Device key on Android: wrapped by an Android Keystore key

**What:** The 64-byte identity secret is generated by core, encrypted with
an AES-256-GCM key kept in Android Keystore, and stored in app-private
`identity.bin`. Auto Backup is off, and the data extraction rules exclude
everything from cloud backup and device-to-device transfer (Android 12+
ignores `allowBackup` for the latter). If the Keystore key is gone, the app
starts over as a new device.

**Why:** Keystore can't hold an X25519 key that Rust can use, so it guards
the key that encrypts it (SPEC.md 6: "Android Keystore-encrypted storage").
With backup off, the identity never leaves the phone; a restored copy on
another phone could otherwise impersonate it to every trusted laptop.

## 2026-10-29 — Phase 7 scope: discovery and a working offline link

**What:** Phase 7 now covers BLE discovery (the original Phase 7) *and* the
Android local-only hotspot link (step 1 of the original Phase 8), so that
Phase 7 ends with a real transfer between devices that share no network.
Wi-Fi Direct and the full link negotiation stay in Phase 8. Alongside: the
Microsoft Store package, a cleaner UI on both apps, and a new logo.

**Why:** The user's priority after Phase 6: what sets Beam apart is working
with no shared Wi-Fi network, router or internet, the way AirDrop does.
Discovery alone shows a device you still can't send to.

## 2026-10-29 — BLE: no identity in trusted-only mode, even over GATT

**What:** In trusted-only mode the advertisement carries the rotating id
(not a device id prefix), the scan response has no name, and `INFO` leaves
out the name and fingerprint. SPEC.md 4.2 lists the rotating id as a
stretch goal and always puts name and fingerprint in `INFO`.

**Why:** Otherwise anyone could follow a phone around by its BLE broadcasts,
or simply connect and read `INFO` to learn who it is, defeating trusted-only
mode. The rotating id already existed for mDNS, so BLE reuses it (its first
6 bytes). A sender that trusts the device already knows its key, and Noise
proves it afterwards.

## 2026-10-29 — Desktop distribution through the Microsoft Store

**What:** The main way to install the desktop app will be the Microsoft
Store (an MSIX package, signed by the Store). The unsigned `.msi` on GitHub
stays as a secondary download.

**Why:** Unsigned installers make Windows SmartScreen say the app "may be
harmful", and testers won't get past that. Signing is the only fix. Store
registration is free for individual developers, and the Store signs the
package, hosts it and updates it. Azure code signing for individuals is
limited to the USA and Canada.

## 2026-10-29 — Logo and visual style: neutral, one ink accent

**What:**
- **Logo:** two points joined by a beam, white on a rounded ink (`#14171C`)
  square. Sources are in `assets/brand/` (`logo.svg`, `mark.svg`, and
  `render.py`, which draws the 1024 px PNG that `npx tauri icon` turns into
  every desktop icon).
- **Android:** the launcher icon is an adaptive vector icon with a
  monochrome layer for themed icons; the notification icon is the same mark.
- **UI style:** both apps share one palette: neutral greys, ink as the only
  accent (it inverts to near-white in dark mode), green, amber and red only
  for status, hairline borders instead of shadows, and no gradients.
- **Android colour:** the app sets every Material 3 colour role and doesn't
  use dynamic colour.

**Why:** Testers found the Phase 5–6 look (blue-to-purple gradients, tinted
panels) generic and "AI-made". A product asking for trust, and headed for the
Microsoft Store, should look calm and deliberate. Material 3's defaults are
purple and dynamic colour follows the wallpaper, so leaving any role unset
brings the purple back.

## 2026-10-29 — Windows doesn't host the Beam GATT service; phones call laptops

**What:** Only Android hosts the GATT service (`INFO`, `CONNECT_REQUEST`,
`CONNECT_RESPONSE`). The Windows app advertises with a plain
`BluetoothLEAdvertisementPublisher`, scans, and is always the GATT
*client*:
- **Laptop → phone:** the laptop connects to the phone and asks for a link
  (SPEC.md 4.2 as written).
- **Phone → laptop:** the phone *calls* the laptop: an advertisement
  naming it. The laptop connects back, joins the hotspot the phone offers,
  and sends a UDP hello with its receiving port (PROTOCOL.md, "Calls").

**Why:** Tested on this machine's Intel AX201: advertising through
`GattServiceProvider` ended in status `Aborted` (error `Success`) every
time, with or without service data and in every connectable/discoverable
combination, while a plain publisher on the same adapter reported
`Started` and the watcher heard dozens of advertisements a second.
Reports of the same failure exist with no fix (see LEARNINGS.md). Rather
than depend on a role some laptops can't play, every laptop plays the one
that works, the central role. A C# helper (SPEC.md's plan B) would call
the same failing API.

## 2026-10-29 — Direct Share: trusted devices in the Android share sheet

**What:** Every trusted device is a dynamic sharing shortcut in the
`dev.beam.android.category.SEND_TO_DEVICE` category, so Android's share
sheet lists "ThinkPad" directly. Sharing to it opens Beam, waits up to 20 s
for that device to be nearby (network or Bluetooth), and sends without
asking anything else. Recently used devices rank higher.

**Why:** The bar is "faster than sending it to yourself on Telegram". With
auto-accept on the laptop, phone → laptop becomes share → tap the laptop:
two taps and no app to open, with nothing to download on the other side.

## 2026-10-29 — Store package: firewall rules and no write virtualization

**What:** The MSIX (desktop/msix) declares inbound TCP and UDP firewall
rules for every network profile, and turns off file-system and registry
write virtualization (`unvirtualizedResources`).

**Why:**
- **Firewall:** when the laptop joins a phone's hotspot, Windows classifies
  it as a public network, and the exception Windows asks for on first run
  usually covers private networks only, so phones couldn't connect in. A
  packaged app can declare its rules at install time. The `.msi` does the
  same with a WiX fragment (`src-tauri/wix/firewall.wxs`): a deferred
  custom action runs `netsh` to add a rule for Beam's executable on every
  profile, and removes it on uninstall. The MSI already installs
  per-machine, elevated, so this adds no prompt.
- **Virtualization:** the "Send to → Beam" shortcut and the start-with-
  Windows entry must be visible to Explorer and Windows. Under MSIX they'd
  be written into the package's private copy of AppData and HKCU instead.

## 2026-10-29 — A LAN device that doesn't answer is tried over Bluetooth

**What:** When a device found on the LAN doesn't accept a connection on
any of its addresses, the sender looks for the same device among its
Bluetooth sightings (`BeamNode::bluetooth_route` on Android,
`AppState::bluetooth_route` on Windows). If it finds one, it continues
with the Bluetooth handshake instead of failing.

**Why:** Being found on the LAN doesn't prove the device can be reached
there. Campus and office networks often let mDNS through but block
traffic between devices, and addresses go stale when a laptop changes
networks. Bluetooth gives a second route. The handshake's own LAN check
then picks the network if it does work in one direction, or the hotspot
if not. Untrusted devices are keyed by the id prefix they advertise, so
the lookup tries both keys.

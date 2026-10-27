# Beam wire protocol

Status: **version 1.0, phase 4.** This covers encrypted transfers over one
control connection plus parallel data connections, with resume, and
discovery on the local network with mDNS. BLE discovery comes in a later
phase.

All integers on the wire are **big-endian**. "MB" means 1,000,000 bytes;
"MiB" means 1,048,576 bytes.

## Overview

A transfer uses one **control connection** and N **data connections**
(default 4, at most 16), all opened by the sender to the receiver's port:

```
Sender                                      Receiver
──────                                      ────────
control connection:
  Noise XX handshake (3 messages) ◄───────► both learn each other's static key
  ── everything below is encrypted ──
  HELLO ──────────────────────────────────►
        ◄──────────────────────────────────  HELLO
  OFFER ──────────────────────────────────►
                                             (asks the user, unless trusted
                                              with auto-accept, or a resume)
        ◄──────────────────────────────────  ACCEPT (+ chunks already held)
data connections 0..N-1, each:
  Noise XX handshake (same static keys)
  STREAM_OPEN (session token) ────────────►
  for each chunk it takes from the queue:
    DATA, DATA, … CHUNK_HASH ─────────────►  verify, then write to <name>.beam-part
back on the control connection:
        ◄──────────────────────────────────  CHUNK_ACK  (batched)
        ◄──────────────────────────────────  CHUNK_RETRY (chunk failed its hash)
  FILE_DONE (when all its chunks are acked) ► check whole file, rename to <name>
        ◄──────────────────────────────────  TRANSFER_COMPLETE
  PING / PONG both ways every 5 s
```

The sender never waits between files, so many small files are pipelined.

## Device identity

Each device has a long-lived **static X25519 key pair**: its Noise static key.

- **Device id:** the first 8 bytes of SHA-256(static public key).
- **Fingerprint:** the device id written as 4 groups of 4 hex characters,
  for example `346e-029e-3df3-3625`.
- **Trusted devices:** a trusted device is remembered by its full 32-byte
  public key, not by its name or id alone.

## Transport and encryption

Every TCP connection, control or data, is a sequence of **records**:

```
record = len:u16 | payload[len]      (len ≤ 65,535)
```

### Handshake

Each connection starts with its own Noise handshake:

| Parameter | Value |
|---|---|
| Pattern | `Noise_XX_25519_ChaChaPoly_BLAKE2s` |
| Prologue | the 6 ASCII bytes `beam/1` |
| Initiator | the side that opened the TCP connection (always the sender) |
| Payloads | empty in all three messages |

Each of the three XX messages (`→ e`, `← e, ee, s, es`, `→ s, se`) is sent as
one record. A side that doesn't finish the handshake within **10 seconds** is
disconnected.

### Encrypted records

After the handshake, every record payload is **one Noise transport message**
(ChaCha20-Poly1305, 16-byte tag), so record plaintext is at most
**65,519 bytes**. Each direction keeps its own nonce counter, starting at 0
and increasing by one per record. A record that fails authentication ends the
connection.

### What a new connection is

The first frame after the handshake decides it:

- **HELLO**: a control connection. The receiver must see it within
  **10 seconds**.
- **STREAM_OPEN**: a data connection.
- Anything else: the receiver closes the connection.

### Peer verification (control connections)

After the HELLO exchange, each side checks the other before going on:

1. **Versions:** the major versions must match.
2. **Id matches key:** `HELLO.device_id` must equal the id derived from the
   static key the handshake proved. A peer can't claim another device's id.
3. **Trusted key matches:** if that id is in the local trust store, the proven
   key must equal the stored key byte for byte.

If any check fails, the connection is closed without further messages.

### Binding data connections to the session

Both sides derive a **session token** from the control connection's
handshake hash:

```
session_token = BLAKE3-derive_key(context = "beam 2026-10-27 session token v1",
                                  material = control handshake hash)   (32 bytes)
```

A data connection is accepted only if all three hold:

- Its STREAM_OPEN carries a `session_token` of a running session.
- Its `transfer_id` is that session's transfer.
- Its own handshake proved the **same sender static key** as the control
  connection.

The sender likewise checks that each data connection reaches the same
receiver key. The token routes the connection to its session, but it is not
a secret (handshake hashes can be computed from the visible handshake). The
key check is what authenticates.

### Verification code

Both sides show a 6-digit code so users can spot a man-in-the-middle:

```
key  = BLAKE3-derive_key(context = "beam 2026-10-27 verification code v1",
                         material = control handshake hash)
code = little_endian_u64(key[0..8]) mod 1,000,000, written as 6 digits with leading zeros
```

A man-in-the-middle has to run two separate handshakes, so the two screens
show different codes. Unknown devices are never accepted automatically. A
trusted device with `auto_accept` skips the prompt.

## Framing

Each frame is carried in one or more records:

```
frame_message = frame_len:u32 | frame[frame_len]
frame         = type:u8 | body
```

- `frame_message` is split into records of at most 65,519 bytes of
  plaintext. The first record of a frame contains at least the 4-byte
  `frame_len`.
- A record never contains bytes from two different frames.
- `frame_len` must be at most **4 MiB**; otherwise the connection is closed
  before anything is allocated.
- Control bodies are CBOR (RFC 8949). DATA has a fixed binary header.
- Every DATA frame fits in one record; only very large OFFER or CLIPBOARD
  frames span several.

### CBOR conventions

- Every control body is a CBOR **map with text-string keys**, one key per
  field listed below.
- Decoders **ignore unknown keys**, so a minor version can add fields.
- The body must be exactly one CBOR item; trailing bytes are an error.
- Byte strings: `uuid` = 16 bytes, `hash` = 32 bytes (BLAKE3),
  `device_id` = 8 bytes, `token` = 32 bytes.
- Unsigned integers are `uint`; `modified_unix` may be negative (`int`).

Wire example: `PING {nonce: 1}` is the frame `0C A1 65 6E 6F 6E 63 65 01`
(type 0x0C, map(1), text(5) "nonce", 1). With its frame length the record
plaintext is 13 bytes:

```
00 00 00 09      frame_len = 9
0C A1 65 6E 6F 6E 63 65 01
```

Encrypted, that becomes a 29-byte Noise message (13 + 16-byte tag), so the
record is `00 1D` followed by 29 bytes of ciphertext.

## Chunks

Files are split into **chunks of 4 MiB** (the last chunk may be shorter; an
empty file has none). Chunk `i` covers bytes
`[i × 4 MiB, min((i+1) × 4 MiB, size))`. The chunk is the unit of hashing,
scheduling, acknowledgement and resume.

- **One chunk, one stream:** a chunk is sent entirely on one data connection
  as consecutive DATA frames, in order, followed by its CHUNK_HASH.
- **Work stealing:** each data connection takes the next unsent chunk when
  it's free. Chunks of different files interleave freely across connections.
- **Verify before writing:** the receiver buffers a chunk, checks it against
  CHUNK_HASH, and only then writes it to `<final name>.beam-part` (at its
  offset; the part file is preallocated to the full size).

## Flow control and acknowledgements

- The sender keeps at most **16 MiB of unacknowledged data per data
  connection**. A CHUNK_ACK frees the space of the chunks it lists.
- The receiver acknowledges a chunk once it is verified *and written*. It
  sends CHUNK_ACKs when 4 MiB or 64 chunks are pending, and at least every
  **250 ms**.
- The receiver writes through a short queue (4 chunks). When the disk is
  slower than the network, data connections stop being read and TCP slows the
  sender down. Memory stays flat.

## Keepalive

On the control connection, each side sends **PING every 5 s** and answers
every PING with a PONG echoing its nonce. A control connection that receives
nothing for **15 s** is treated as dead. Data connections carry no
keepalive; their session ends with its control connection.

## Resume

### Receiver side

When it accepts a transfer, the receiver writes a **progress file**:

```
<download dir>/.beam-partial/<transfer_id>.json
```

It holds the sender's public key, and for each file: the offered fingerprint
(name, size, modified time), the reserved final name, a bitmap of verified
chunks, and the whole-file hash once the file is finished.

- The progress file is saved at least **every 2 s**.
- Before each save, the files with partly written data are synced to disk, so
  the bitmap never claims chunks that aren't stored.
- It's deleted when the transfer completes.
- Unfinished transfers are kept for **24 hours**, then their part files and
  progress file are removed.

### Sender side

If the connection drops after the receiver accepted (network error, closed
connection, keepalive timeout), the sender:

1. Reconnects to the same address, waiting 100 ms, 200 ms, 400 ms, 800 ms,
   then 1.6 s between attempts, for up to **60 s** after the drop.
2. Refuses to continue if the receiver now has a different static key.
3. Sends the **same OFFER** (same `transfer_id`) again.

### Matching a resumed offer

The receiver resumes, without asking the user again, when a progress file
exists for that `transfer_id`, the sender key matches, and every offered
file's fingerprint matches (same file ids, names, sizes and modified times, in
the same order).

- Its ACCEPT then lists the verified chunks in `have`, and the sender sends
  only the missing ones.
- If an older session of the same transfer is still running on the receiver
  (it hasn't noticed the drop yet), the new session tells it to stop and waits
  until it has saved its progress.
- A progress file that doesn't match is discarded, and the offer is treated as
  new.

## Messages

Examples use CBOR diagnostic notation (RFC 8949 section 8). `h'…'` is a byte
string.

### 0x01 HELLO (both directions, control)

The first frame on a control connection. The sender sends it first; the
receiver replies with its own HELLO **before** checking, so both sides can
report a version mismatch.

| Field | Type | Meaning |
|---|---|---|
| `version` | map `{major: uint, minor: uint}` | Protocol version. Currently 1.0. |
| `device_id` | bytes(8) | First 8 bytes of SHA-256 of the sender's static key. Must match the key proven in the handshake. |
| `name` | text | Human-readable device name. |
| `type` | text | `"phone"` or `"laptop"`. |
| `capabilities` | array of text | Optional features. Empty in version 1.0. |

```
{"version": {"major": 1, "minor": 0},
 "device_id": h'0123456789abcdef',
 "name": "Kartikay's Pixel", "type": "phone", "capabilities": []}
```

### 0x02 OFFER (sender → receiver, control)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | Random v4 UUID chosen by the sender; reused when resuming. |
| `files` | array of file entries | See below. |
| `total_size` | uint | Sum of all `size` fields. |

File entry:

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint (u32) | Unique within this transfer. |
| `name` | text | File name as the sender sees it. **Untrusted.** |
| `size` | uint | Bytes. |
| `mime` | text | Best-effort MIME type, for display only. |
| `modified_unix` | int | Last-modified time, seconds since 1970, 0 if unknown. |

```
{"transfer_id": h'5f0c…(16 bytes)',
 "files": [{"file_id": 0, "name": "IMG_0042.jpg", "size": 2311042,
            "mime": "image/jpeg", "modified_unix": 1790000000}],
 "total_size": 2311042}
```

The receiver answers REJECT (and closes) if the offer:
- has no files, or more than **10,000**;
- repeats a `file_id`;
- has a `name` longer than **1,024 bytes**;
- has a `total_size` that doesn't equal the sum of the sizes;
- uses a `transfer_id` that is currently running for a different sender.

Names are then sanitized before they are shown or used:
- Only the last path component is kept.
- `<>:"/\|?*` and control characters become `_`.
- Bidirectional-override characters are removed.
- Leading/trailing whitespace and trailing dots are trimmed.
- Windows device names (`CON`, `NUL`, `COM1`, …) get a `_` prefix.
- The name is truncated to 200 bytes, keeping a short extension.
- An empty result becomes `file`.

If the name is already taken, the file is saved as `name (1).ext`,
`name (2).ext`, and so on. See `core/src/sanitize.rs`.

### 0x03 ACCEPT (receiver → sender, control)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | From the OFFER. |
| `have` | array of `{file_id: uint, chunk_bitmap: bytes}` | Chunks the receiver already holds (resume). Files with nothing are left out. |

In `chunk_bitmap`, chunk `i` is bit `i % 8` (least significant first) of byte
`i / 8`. A set bit means the chunk is verified and stored. The bitmap has
exactly `ceil(chunks / 8)` bytes, and bits past the last chunk are zero.

```
{"transfer_id": h'5f0c…', "have": [{"file_id": 0, "chunk_bitmap": h'ff0f'}]}
```

### 0x04 REJECT (receiver → sender, control)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | From the OFFER. |
| `reason` | text | Human-readable reason. |

```
{"transfer_id": h'5f0c…', "reason": "declined by user"}
```

After REJECT the receiver closes the connection.

### 0x05 STREAM_OPEN (sender → receiver, first frame of a data connection)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | The transfer this data connection belongs to. |
| `session_token` | token | Derived from the control handshake hash (see above). |
| `stream_index` | uint (u16) | 0-based index of this data connection (informational). |

```
{"transfer_id": h'5f0c…', "session_token": h'9a41…(32 bytes)', "stream_index": 2}
```

A data connection that doesn't match a running session is closed. The
receiver never writes on data connections.

### 0x06 DATA (sender → receiver, data connection)

Binary, not CBOR:

```
type=0x06 | file_id:u32 | offset:u64 | len:u32 | bytes[len]
```

- `len` must equal the number of bytes that follow, and must not be 0.
- The sender uses up to **64,512 bytes** (63 KiB) per DATA frame, so a DATA
  frame always fits in one record.
- The first DATA frame of a chunk starts at the chunk's first byte. Each
  following frame continues exactly where the previous one ended, and no
  frame crosses the chunk's end.

Example: 3 bytes `abc` at offset 1024 of file 7:

```
06 00000007 0000000000000400 00000003 616263
```

### 0x07 CHUNK_HASH (sender → receiver, data connection)

Sent right after the last DATA frame of each chunk, on the same connection.

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `chunk_index` | uint | 0-based. |
| `blake3` | hash | BLAKE3 of the chunk's bytes. |

```
{"file_id": 0, "chunk_index": 0, "blake3": h'af1349b9…(32 bytes)'}
```

If the chunk matches, the receiver writes it and later acknowledges it. If it
doesn't, the receiver discards it and sends CHUNK_RETRY.

### 0x08 CHUNK_ACK (receiver → sender, control)

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `chunk_indices` | array of uint | Chunks verified and written since the last ack. |

```
{"file_id": 0, "chunk_indices": [12, 13, 14]}
```

### 0x0E CHUNK_RETRY (receiver → sender, control)

Not in SPEC.md's original table; added because SPEC.md 7.3 says a failed
chunk is "re-requested" (see DECISIONS.md).

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `chunk_index` | uint | The chunk to send again. |

```
{"file_id": 0, "chunk_index": 7}
```

Sent when:
- A chunk fails its CHUNK_HASH check.
- A finished file fails its whole-file check (see FILE_DONE). In that case
  the receiver asks for every chunk of the file, even chunks it had already
  acknowledged. The sender then treats them as unacknowledged again.

A chunk asked for **5 times** ends the transfer with an error.

### 0x09 FILE_DONE (sender → receiver, control)

Sent once all of a file's chunks have been acknowledged. An empty file has no
chunks, so it gets only FILE_DONE.

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `blake3_full` | hash | BLAKE3 of the whole file. |

```
{"file_id": 0, "blake3_full": h'af1349b9…(32 bytes)'}
```

The receiver checks that every chunk is verified, then compares the whole-file
hash:
- **One-chunk files:** the verified chunk hash *is* the whole-file hash, so
  it's compared directly.
- **Other files:** the part file is read back from disk and hashed.

On a match, the receiver renames `<name>.beam-part` to `<name>` (choosing
`<name> (n)` if that name appeared in the meantime). On a mismatch, it sends
CHUNK_RETRY for every chunk of the file. FILE_DONE for a file that is already
finished (from an earlier session) is answered by nothing if the hash matches.

### 0x0A TRANSFER_COMPLETE (receiver → sender, control)

Sent when every file in the OFFER is verified and saved.

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | |
| `stats` | map | `{bytes: uint, files: uint, duration_ms: uint}`, measured by the receiver for this session. |

```
{"transfer_id": h'5f0c…', "stats": {"bytes": 2311042, "files": 1, "duration_ms": 412}}
```

Both sides then close all connections.

### 0x0B CANCEL (both directions, control)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | |
| `reason` | text | Human-readable reason. |

```
{"transfer_id": h'5f0c…', "reason": "protocol violation: DATA for unknown file_id 9"}
```

A CANCEL (sent or received) ends the transfer for good. The receiver deletes
the part files of unfinished files and the progress file. Files already
finished are kept.

### 0x0C PING / 0x0D PONG (both directions, control)

| Field | Type | Meaning |
|---|---|---|
| `nonce` | uint (u64) | PONG echoes the PING's nonce. |

```
{"nonce": 1}
```

### 0x10 CLIPBOARD (both directions)

Defined now, used from phase 9.

| Field | Type | Meaning |
|---|---|---|
| `mime` | text | `"text/plain"` |
| `text` | text | Clipboard contents. |

```
{"mime": "text/plain", "text": "hello from my phone"}
```

## Errors and closing

- Protocol violations end the transfer: unknown frame types, malformed CBOR,
  a bad DATA length, an oversized frame, a frame on the wrong kind of
  connection or at the wrong time, or DATA that doesn't continue its chunk.
  The receiver sends CANCEL with a reason (when it can) and closes.
- A dropped connection (network error, clean close in the middle of a
  transfer, keepalive timeout) is **not** an error by itself. The receiver
  keeps its progress, and the sender reconnects and resumes as described
  above.

## Versioning

`version.major` changes when old and new peers can't understand each other.
`version.minor` changes for compatible additions, such as new optional map
keys, which old decoders ignore. Unknown frame types are an error in
version 1.0.

## Discovery (mDNS)

Receivers announce themselves on the local network with multicast DNS /
DNS-SD (RFC 6762, RFC 6763) as service type **`_beam._tcp.local.`**. The SRV
record carries the receiver's TCP port. The A/AAAA records carry all of its
addresses, including loopback and link-local ones, so senders should prefer
routable IPv4, then IPv6, then link-local, and try loopback last.

What is announced depends on the receiver's **visibility**:

| Visibility | Announced | TXT record |
|---|---|---|
| everyone | yes | `v`, `vis=everyone`, `id`, `name`, `type` |
| trusted only | yes, anonymously | `v`, `vis=trusted`, `rid` |
| off | nothing | — |

TXT keys:

| Key | Value |
|---|---|
| `v` | Protocol major version, `1`. Announcements with another value are ignored. |
| `vis` | `everyone` or `trusted`. |
| `id` | Device id: 16 lowercase hex digits (no dashes). |
| `name` | Device name, at most 63 bytes of UTF-8. |
| `type` | `phone` or `laptop`. |
| `rid` | Rotating identifier: 16 lowercase hex digits (see below). |

Example (everyone):

```
instance  beam-80b4eac682edd3d3._beam._tcp.local.
host      beam-80b4eac682edd3d3.local.
TXT       v=1 vis=everyone id=80b4eac682edd3d3 name=Laptop-A type=laptop
```

The instance and host labels are `beam-` followed by the `id` (or the `rid`),
so the computer's own host name is never broadcast.

### Rotating identifier (trusted-only mode)

```
window = floor(unix_time / 900)                     (15-minute windows)
k      = BLAKE3-derive_key(context = "beam 2026-10-27 rotating id v1",
                           material = static public key (32 bytes))
rid    = BLAKE3-keyed_hash(key = k, input = window as u64 little-endian)[0..8]
```

A device that trusts the announcer knows its public key, so it can compute
`rid` for the current window and the windows on either side (tolerating
±15 minutes of clock difference) and recognize it. It then shows it under the
name from its own trust store. Anyone else sees an anonymous service whose
name and `rid` change every 15 minutes, so they can't identify or track the
device. The announcer re-registers at every window boundary.

### Discovery is a hint, not proof

Anyone on the network can announce any name or id. A sender that picks a
device found over mDNS must check, after the Noise handshake, that the
receiver's proven device id equals the `id` it chose (or, for a trusted-only
device, the trusted key). If it doesn't, the sender closes the connection
without sending an OFFER.

### Visibility on the receiver

Visibility is also enforced when an OFFER arrives, whatever the mDNS
announcement says:

- **Off:** every offer is declined: REJECT with reason
  `not accepting transfers right now`.
- **Trusted only:** offers from devices not in the trust store are declined
  without asking the user (`only accepting transfers from trusted devices`).
  A resumed transfer that was already accepted may continue.
- **Everyone:** offers go to the user as usual. This mode falls back to
  trusted-only **10 minutes** after it was chosen.

## Discovery (BLE advertisement and GATT)

To be written in phase 7.

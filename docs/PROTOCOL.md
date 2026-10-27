# Beam wire protocol

Status: **version 1.0, phase 1.** This covers a single plain-TCP connection.
Encryption (Noise XX), parallel data connections, CHUNK_ACK, keepalive timing
and resume are specified in SPEC.md and will be documented here as they are
built. Message bodies for those are already defined below, so the encoding
won't change.

All integers on the wire are **big-endian**. "MB" means 1,000,000 bytes.

## Overview

A transfer runs over one TCP connection, which the sender opens:

```
Sender                                   Receiver
──────                                   ────────
HELLO ──────────────────────────────────►
      ◄────────────────────────────────── HELLO
OFFER ──────────────────────────────────►
                                          (asks the user)
      ◄────────────────────────────────── ACCEPT   (or REJECT, then close)
for each file:
  for each 4 MiB chunk:
    DATA, DATA, … ──────────────────────►  write to <name>.beam-part
    CHUNK_HASH ─────────────────────────►  verify chunk
  FILE_DONE ────────────────────────────►  verify file, rename to <name>
      ◄────────────────────────────────── TRANSFER_COMPLETE
close
```

The sender does not wait between files, so many small files are pipelined.

## Transport and encryption

The TCP byte stream is a sequence of **records**:

```
record = len:u16 | payload[len]
```

A record is the unit that will become one Noise transport message in phase 2,
so `len` is at most 65,535. Phase 1 sends the payload in plaintext. To keep
sizes identical once encryption is added, record plaintext is limited to
**65,519 bytes** (65,535 minus a 16-byte AEAD tag).

## Framing

Each frame is carried in one or more records:

```
frame_message = frame_len:u32 | frame[frame_len]
frame         = type:u8 | body
```

- `frame_message` is split into records of at most 65,519 bytes. The first
  record of a frame must contain at least the 4-byte `frame_len`.
- A record never contains bytes from two different frames.
- `frame_len` must be at most **4 MiB** (4,194,304). A receiver closes the
  connection if it's larger, before allocating anything.
- Control bodies are CBOR (RFC 8949). DATA has a fixed binary header.
- Small frames (all control frames in practice, and every DATA frame) fit in
  one record. Only very large OFFER or CLIPBOARD frames span several.

### CBOR conventions

- Every control body is a CBOR **map with text-string keys**, one key per field
  listed below.
- Decoders **ignore unknown keys**, so a minor version can add fields.
- The body must be exactly one CBOR item; trailing bytes are an error.
- `uuid` = 16-byte byte string. `hash` = 32-byte byte string (BLAKE3).
  `device_id` = 8-byte byte string. Other `bytes` fields are byte strings.
- Unsigned integers are `uint`; `modified_unix` may be negative (`int`).

Wire example: `PING {nonce: 1}` is the frame
`0C A1 65 6E 6F 6E 63 65 01` (type 0x0C, map(1), text(5) "nonce", 1).
On the wire, with the frame length and record length:

```
00 0D            record len = 13
00 00 00 09      frame_len  = 9
0C A1 65 6E 6F 6E 63 65 01
```

## Messages

Examples use CBOR diagnostic notation (RFC 8949 section 8). `h'…'` is a byte
string.

### 0x01 HELLO (both directions)

The first frame each side sends. The sender sends it first; the receiver
replies with its own HELLO **before** checking versions, so both sides can
report a mismatch.

| Field | Type | Meaning |
|---|---|---|
| `version` | map `{major: uint, minor: uint}` | Protocol version. Currently 1.0. |
| `device_id` | bytes(8) | Device id (phase 1: random per run; phase 2: first 8 bytes of SHA-256 of the static key). |
| `name` | text | Human-readable device name. |
| `type` | text | `"phone"` or `"laptop"`. |
| `capabilities` | array of text | Optional features. Empty in phase 1. |

```
{"version": {"major": 1, "minor": 0},
 "device_id": h'0123456789abcdef',
 "name": "Kartikay's Pixel", "type": "phone", "capabilities": []}
```

If the major versions differ, both sides close the connection and show an
error such as "peer speaks protocol 2.0, but this device speaks 1.0".

### 0x02 OFFER (sender → receiver)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | Random v4 UUID chosen by the sender. |
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
- has a `total_size` that doesn't equal the sum of the sizes.

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

### 0x03 ACCEPT (receiver → sender)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | From the OFFER. |
| `have` | array of `{file_id: uint, chunk_bitmap: bytes}` | Chunks already verified, for resume. Empty in phase 1. |

In `chunk_bitmap`, chunk `i` is bit `i % 8` (least significant first) of byte
`i / 8`. A set bit means the receiver already has that chunk.

```
{"transfer_id": h'5f0c…', "have": []}
```

### 0x04 REJECT (receiver → sender)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | From the OFFER. |
| `reason` | text | Human-readable reason. |

```
{"transfer_id": h'5f0c…', "reason": "declined by user"}
```

After REJECT the receiver closes the connection.

### 0x05 STREAM_OPEN (sender → receiver, on a data connection)

Defined now, used from phase 3.

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | The transfer this data connection belongs to. |
| `session_token` | bytes(32) | Derived from the control connection's Noise handshake hash. |
| `stream_index` | uint (u16) | 0-based index of this data connection. |

```
{"transfer_id": h'5f0c…', "session_token": h'9a41…(32 bytes)', "stream_index": 2}
```

### 0x06 DATA (sender → receiver)

Binary, not CBOR:

```
type=0x06 | file_id:u32 | offset:u64 | len:u32 | bytes[len]
```

- `len` must equal the number of bytes that follow, or the frame is invalid.
- The sender uses up to **64,512 bytes** (63 KiB) per DATA frame, so a DATA
  frame always fits in one record.
- A DATA frame never crosses a 4 MiB chunk boundary.
- Phase 1: DATA for a file arrives in order (`offset` equals the bytes
  received so far). Parallel streams in phase 3 relax this.

Example: 3 bytes `abc` at offset 1024 of file 7:

```
06 00000007 0000000000000400 00000003 616263
```

### 0x07 CHUNK_HASH (sender → receiver)

Sent right after the last DATA frame of each 4 MiB chunk (the last chunk of a
file may be shorter). Chunk `i` covers bytes `[i × 4 MiB, min((i+1) × 4 MiB, size))`.

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `chunk_index` | uint | 0-based. |
| `blake3` | hash | BLAKE3 of the chunk's bytes. |

```
{"file_id": 0, "chunk_index": 0, "blake3": h'af1349b9…(32 bytes)'}
```

The receiver compares this with the hash of the bytes it received. Phase 1:
a mismatch cancels the transfer. Phase 3: the chunk is requested again.

### 0x08 CHUNK_ACK (receiver → sender)

Defined now, used from phase 3. Batched about every 250 ms.

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `chunk_indices` | array of uint | Chunks verified and persisted since the last ack. |

```
{"file_id": 0, "chunk_indices": [12, 13, 14]}
```

### 0x09 FILE_DONE (sender → receiver)

Sent after the last CHUNK_HASH of a file. An empty file has no chunks, so it
gets only FILE_DONE.

| Field | Type | Meaning |
|---|---|---|
| `file_id` | uint | |
| `blake3_full` | hash | BLAKE3 of the whole file. |

```
{"file_id": 0, "blake3_full": h'af1349b9…(32 bytes)'}
```

The receiver checks:
1. Every chunk was received and verified.
2. The full-file hash matches.

It then renames `<name>.beam-part` to `<name>`. If a file with that name
appeared in the meantime, it saves as `<name> (n)` instead.

### 0x0A TRANSFER_COMPLETE (receiver → sender)

Sent when every file in the OFFER is verified and saved.

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | |
| `stats` | map | `{bytes: uint, files: uint, duration_ms: uint}`, measured by the receiver from ACCEPT to completion. More fields (peak speed, link type, stream count, retries) come in later phases. |

```
{"transfer_id": h'5f0c…', "stats": {"bytes": 2311042, "files": 1, "duration_ms": 412}}
```

Both sides then close the connection.

### 0x0B CANCEL (both directions)

| Field | Type | Meaning |
|---|---|---|
| `transfer_id` | uuid | |
| `reason` | text | Human-readable reason. |

```
{"transfer_id": h'5f0c…', "reason": "hash mismatch in IMG_0042.jpg (chunk 0)"}
```

When a receiver fails a transfer (a protocol violation, a hash mismatch or a
disk error), it sends CANCEL with the reason, deletes the `.beam-part` files
and closes. (From phase 3, partial files are kept for resume instead.)

### 0x0C PING / 0x0D PONG (both directions)

| Field | Type | Meaning |
|---|---|---|
| `nonce` | uint (u64) | PONG echoes the PING's nonce. |

```
{"nonce": 1}
```

Phase 1 answers PING with PONG during a transfer. The 5 s interval and 15 s
timeout come in phase 3.

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

- Unknown frame types, malformed CBOR, a bad DATA length, an oversized frame
  or a frame that arrives at the wrong time are protocol violations. The
  receiver sends CANCEL with a reason (when it can) and closes.
- A clean TCP close between frames ends the session. A close in the middle of
  a frame is an error.

## Versioning

`version.major` changes when old and new peers can't understand each other.
`version.minor` changes for compatible additions, such as new optional map
keys, which old decoders ignore. Unknown frame types are an error in
version 1.0.

## Discovery (mDNS)

To be written in phase 4.

## Discovery (BLE advertisement and GATT)

To be written in phase 7.

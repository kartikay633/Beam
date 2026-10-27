# Learnings

Bugs that took more than an hour to fix. For each one: symptom, root cause, fix.

## Template

### Title

- **Symptom:**
- **Root cause:**
- **Fix:**
- **Phase:**

## Windows dev setup: `cargo test` fails with "link: missing operand"

- **Symptom:** `cargo fmt` and `cargo clippy` worked, but `cargo test` failed with
  `link: missing operand after ' ■'` and "you may need to install Visual Studio
  build tools".
- **Root cause:** The `x86_64-pc-windows-msvc` toolchain needs Microsoft's
  `link.exe` from the C++ Build Tools. They weren't installed, so the first
  `link.exe` on PATH was Git for Windows' unrelated GNU `link` (it creates hard
  links). `check`/`clippy` never link, which is why they passed. The first
  Build Tools install also failed with `0x80070070` (disk full), and an earlier
  rustup install left a toolchain missing `cargo`/`clippy` for the same reason.
- **Fix:** Install the Build Tools with only `VC.Tools.x86.x64` and the Windows 11
  SDK, using `--installPath`, `--path cache=` and `--path shared=` on a drive
  with space. The Windows SDK still lands on C: (~1.9 GB). Reinstall Rust with
  `rustup set profile minimal` to skip the ~600 MB offline docs.
- **Phase:** 0

## One stream stuck at exactly 65 MB/s

- **Symptom:** With `--streams 1`, a 1 GB loopback transfer ran at 65 MB/s.
  With 4 streams it reached 650 MB/s. That was far more than a 4× difference,
  and suspiciously round.
- **Root cause:** Flow control and ack batching were fighting. Each stream may
  have 16 MiB unacknowledged. The receiver acked when 8 chunks were pending
  or every 250 ms. With one stream only 4 chunks fit in the window, so the
  8-chunk trigger never fired and acks came only from the timer: 16 MiB per
  250 ms = 64 MiB/s ≈ 67 MB/s.
- **Fix:** Also flush acks once 4 MiB (one chunk) is pending. One stream went
  to about 200 MB/s. Lesson: any batching threshold must be smaller than the
  window it feeds, or the timer becomes the throughput limit.
- **Phase:** 3

## 1 GB transfer used 950 MB of memory

- **Symptom:** The benchmark's memory probe showed about 950 MB resident on
  both sender and receiver for a 1 GB file, against a target of under
  100 MB.
- **Root cause:** Whole-file hashing used `blake3::Hasher::update_mmap_rayon`.
  A memory-mapped file's pages count toward the process's working set as they
  are read, so hashing a 1 GB file "used" 1 GB. It wasn't heap, but it looks
  exactly like a leak to any memory tool, and on a 20 GB file it would push
  everything else out of RAM.
- **Fix:** Read in 16 MiB blocks and hash each block with `update_rayon`
  (still multi-core). Peak memory for 1 GB with 4 streams is now 33 MB (sender) and 51 MB (receiver); the highest seen with 8 streams is 74 MB.
- **Phase:** 3

## 1,000 small files took 2–5 s, whatever the stream count

- **Symptom:** 1,000 × 10 KB files took the same 2–5 s with 1, 4 or 8 streams:
  some per-file cost was being paid one file at a time.
- **Root cause:** Three things together:
  1. **An fsync per finished file.**
  2. **A hidden fsync per file in handle eviction.** When more than 64 part
     files were open, the writer synced and closed them all. With thousands
     of tiny files waiting for FILE_DONE that happened constantly, so it was
     effectively one fsync per file.
  3. **The session loop waited for the writer** to close each file before
     handling the next frame, which made finishing strictly sequential.

  A probe also showed that on this machine, opening a *freshly written* file
  for reading costs about 8 ms (Windows Defender scanning it). So re-reading
  small files to hash them would have been just as bad.
- **Fix:**
  - Close fully written files at once, with no fsync. Checkpoints still sync
    partially written files.
  - Do closing, checking and renaming in a background task per file.
  - Use the verified chunk hash as the whole-file hash for one-chunk files.

  Now 1,000 files take about 1.3 s (≈1.3 ms per file, mostly file creation
  under on-access scanning).
- **Phase:** 3

## Cancelling on the receiver restarted the whole transfer

- **Symptom:** The test `receiver_can_cancel_mid_transfer` cancelled an
  incoming 300 MB transfer. The sender then reported **success**, with one
  reconnect, and the receiver held a complete copy of the file.
- **Root cause:** The receiver's teardown first shut down its tasks (closing
  the data connections) and only then sent CANCEL on the control connection.
  The sender saw the data connections break first, classified that as a
  network drop (retryable), and reconnected before reading the CANCEL. By
  then the receiver had deleted its progress file, so the reconnect looked
  like a brand-new offer and was accepted again.
- **Fix:**
  - The receiver sends CANCEL *before* closing anything.
  - On a broken data connection, the sender drains the control connection
    for up to 500 ms looking for a CANCEL before deciding to reconnect.

  Messages that end a session must always go out before the connections
  close, because peers read different connections in no particular order.
- **Phase:** 5

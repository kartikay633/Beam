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

## A UniFFI error field named `message` broke the Kotlin build

- **Symptom:** The generated Kotlin bindings failed to compile:
  "Conflicting declarations: val message: String" and "'message' hides
  member of supertype 'Throwable'".
- **Root cause:** `BeamError::Failed { message: String }` becomes a Kotlin
  exception class with a `message` property, which clashes with
  `Throwable.message`. The Rust side and its tests were fine; only the
  Android build showed it.
- **Fix:** Renamed the field to `reason`. Error fields must not use names
  that `Throwable` already has (`message`, `cause`).
- **Phase:** 6

## `local.properties` with Windows backslashes pointed at "D:androidsdk"

- **Symptom:** Gradle failed with "The filename, directory name, or volume
  label syntax is incorrect" while working out the dependencies of
  `compileDebugJavaWithJavac`; only `--stacktrace` showed it came from
  `SdkLocator`.
- **Root cause:** `local.properties` is a Java properties file, where `\` is
  an escape: `sdk.dir=D\:\android\sdk` reads as `D:androidsdk`.
- **Fix:** `sdk.dir=D\:/android/sdk`. Forward slashes work on Windows, or
  double the backslashes.
- **Phase:** 6

## Coroutines in `backgroundScope` never ran under `advanceUntilIdle`

- **Symptom:** Repository tests for sending and saving received files found
  nothing sent and nothing saved, although the same code worked in the app.
- **Root cause:** The repository was given the test's `backgroundScope`.
  `advanceUntilIdle()` returns once only background work is left, so
  coroutines launched there never ran.
- **Fix:** Pass the `TestScope` itself; `runTest` then also waits for that
  work before finishing.
- **Phase:** 6

## `GattServiceProvider` advertising aborted; a plain publisher worked

- **Symptom:** On the laptop, `GattServiceProvider.StartAdvertisingWithParameters`
  succeeded, but `AdvertisementStatus` was `Aborted` (3) a moment later,
  and the status-changed event said `Aborted` with error `Success`. The
  service and its three characteristics were created fine.
- **Root cause:** Unknown inside Windows. Ruled out by hardware tests
  (`cargo test -p beam-desktop -- --ignored`):
  - payload size: it aborts with no service data at all;
  - the connectable/discoverable flags: every combination aborts;
  - the radio: on the same Intel AX201, a plain
    `BluetoothLEAdvertisementPublisher` with the same service data reported
    `Started`, and the watcher heard ~50 advertisements in 3 s.
- **Fix:** Windows never hosts the GATT service. It advertises with the
  publisher and acts only as a GATT client; a phone that wants to send to it
  "calls" it instead (DECISIONS.md). Hardware smoke tests stay in the repo.
  Lesson: test each platform radio API on real hardware before building a
  protocol on it.
- **Phase:** 7

## Kotlin: `fun f() = synchronized(lock) { … runCatching { … } }` returns `Result`

- **Symptom:** "Return type mismatch: expected 'Result<Unit>', actual
  'Unit'" at an early `return` inside the `synchronized` block.
- **Root cause:** An expression-bodied function takes its type from the
  block's last expression, here `runCatching { … }`, so the function
  returned `Result<Unit>` and a bare `return` no longer matched.
- **Fix:** Declare `: Unit` and end the block with `Unit`, or use a block
  body.
- **Phase:** 7

## Android 13's Bluetooth write calls return `BluetoothStatusCodes`

- **Symptom:** Lint error *WrongConstant* on
  `gatt.writeCharacteristic(c, value, type) == BluetoothGatt.GATT_SUCCESS`.
- **Root cause:** The Android 13 overloads (`writeCharacteristic`,
  `writeDescriptor`, `notifyCharacteristicChanged`) return
  `BluetoothStatusCodes`, not GATT status codes. Both happen to use 0 for
  success, so it worked by accident.
- **Fix:** Compare with `BluetoothStatusCodes.SUCCESS`.
- **Phase:** 7

## A sender's cancel left the receiver waiting for a resume (CI only)

- **Symptom:** `sender_can_cancel_mid_transfer` failed on GitHub's
  runners, Linux and Windows, from Phase 7 on: the receiver never reported
  an outcome, and the test timed out after 120 s. It passed every time
  locally, even pinned to two cores.
- **Root cause:** The mirror image of the receiver-cancel bug from Phase 5.
  A cancelling sender sends CANCEL on the control connection and then drops
  every connection. The receiver's `select!` can see a data stream break,
  or a write on the control connection fail, before it reads the CANCEL.
  It treats that as a network drop (retryable) and keeps the transfer
  around for a resume that never comes. Phase 5 fixed only the sender side
  (`pending_cancel`); slower runners made the receiver side lose the race.
- **Fix:** `pending_cancel` moved to `transfer.rs`. When the receiver's
  session ends with a retryable error, it now drains the control
  connection for up to 500 ms looking for a CANCEL first, exactly like the
  sender. Lesson: when one side gets a race fix, check whether the other
  direction has the same race.
- **Phase:** 7

## The laptop kept announcing its old network's addresses

- **Symptom:** First device test. The phone, on the same network as the
  laptop (the phone's own hotspot), failed to send with "KARTIKAY didn't
  answer", and nothing appeared on the laptop. `beam-cli discover` showed
  the laptop announcing 169.254.x (Tailscale), loopback and an IPv6
  address, but not its Wi-Fi IPv4 address. Beam listens on IPv4 only, so
  none of those reached it.
- **Root cause:** Beam started while the laptop was on one Wi-Fi network,
  and a minute later Windows moved it to another. `mdns-sd` checks for
  address changes every 5 s, but here its daemon never started announcing
  the new IPv4 address. Restarting Beam fixed it at once.
- **Fix:** `Advertiser` and `Browser` (core/src/discovery.rs) check the
  machine's IPv4 addresses every 3 s. When they change, each replaces its
  `ServiceDaemon` with a fresh one: the advertiser re-registers, and the
  browser reports what it had seen as lost and browses again. Separately,
  a send to a LAN device that doesn't answer now falls back to Bluetooth,
  on Android and Windows, when the device is in range.
- **Phase:** 7

## Waiting for a CANCEL delayed saving resume progress

- **Symptom:** `resume_after_the_receiver_restarts` failed once on Linux
  CI: the new receiver resumed with only 4 MB on disk instead of about 100.
- **Root cause:** The cancel-race fix above made the receiver wait up to
  500 ms for a CANCEL before returning a retryable error, and only then
  save its progress file. The test starts a new receiver 500 ms after
  killing the old one, so on a slow runner the new receiver read a
  progress file from the last 2 s checkpoint. A real app killed right
  after a network drop would lose the same.
- **Fix:** On a retryable error, the session syncs what the writer has
  and saves progress first, then waits for a possible CANCEL. Lesson: a
  wait added to an error path delays everything after it, including
  saving state.
- **Phase:** 7

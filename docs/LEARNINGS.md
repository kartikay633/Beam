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

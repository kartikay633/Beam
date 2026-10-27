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

# Beam desktop app

Coming in a later phase (phase 5): a Tauri v2 app with a React + TypeScript UI
that links `beam-core` directly. See [docs/SPEC.md](../docs/SPEC.md) section 9.

Planned layout:

- `src-tauri/` — Rust side (Tauri commands, tray, Windows radio APIs)
- `ui/` — React + TypeScript + Vite front-end

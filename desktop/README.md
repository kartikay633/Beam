# Beam desktop app

Beam for Windows, built with Tauri v2. The Rust side (`src-tauri/`) uses
`beam-core` directly; the UI (`ui/`) is React + TypeScript + Vite. See
[docs/SPEC.md](../docs/SPEC.md) section 9.

## What it does

- **Tray icon:** open Beam, switch who can see you (everyone for 10 minutes /
  trusted devices only / off), quit. Closing the window keeps Beam running in
  the tray.
- **Main window:**
  - **Send & receive:** nearby devices (found with mDNS), a drop zone (drag
    files from Explorer), and a transfer list with progress, speed, time left
    and cancel.
  - **Trusted devices:** auto-accept on/off, remove.
  - **Pair a phone:** a QR code with this computer's id, key and addresses.
  - **Settings:** device name, download folder, parallel connections,
    visibility at start-up, Send To menu, start with Windows.
- **Incoming request popup:** an always-on-top window with the sender, the
  files, the 6-digit verification code, Accept / Decline, and "trust this
  device". It declines by itself after 60 seconds.
- **Explorer integration:** right-click files → **Send to → Beam**. Explorer
  starts Beam with the files; a single-instance check hands them to the
  running app, which stages them for sending.
- **Device key:** stored with Windows DPAPI (`identity.dpapi`), so it only
  decrypts for your Windows account on this machine.

## Run it

Requirements:
- Rust (stable) with the Visual Studio C++ Build Tools.
- Node.js 20 or newer.
- WebView2, which is built into Windows 10/11.

```bash
cd desktop
npm install
npm run tauri dev
```

Windows Firewall asks once to let Beam use the network. Allow it on private
networks, or other devices won't find it.

Build the installer:

```bash
npm run tauri build -- --bundles msi
```

The `.msi` lands in `target/release/bundle/msi/` at the repository root.

## Where things are stored

- **Settings, the encrypted key (`identity.dpapi`) and trusted devices:**
  `%APPDATA%\dev.beam.desktop\`.
- **Received files:** `Downloads\Beam` by default; change it in Settings.
- **Partial transfers (for resume):** the `.beam-partial` folder inside the
  download folder.

## Working on the UI

`npm run dev` serves the UI alone at http://localhost:1420. Outside Tauri it
runs in **demo mode** with made-up devices and transfers (a "demo" badge shows
next to the logo), so every screen can be checked in an ordinary browser.
`#/offer/demo` shows the incoming-request popup.

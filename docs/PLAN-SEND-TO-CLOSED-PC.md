# Plan: send to a PC where Beam isn't open, and choose where files land

Status: Stage 1 built in desktop 0.1.9; Stage 2 (well-known folders) built in desktop 0.2.0 / Android 0.1.6 (2026-11-03); Stage 3 is a proposal.

## The goal

On the phone you open Beam (or tap Share in any app). Your trusted PC shows up
even though nobody opened Beam on it. You tap it, pick **where on the PC** the
files should go (Downloads, Desktop, Documents, a pinned folder), and they
land there. The PC shows a small "Saved 3 photos to Desktop" notification. No
window opens, and nothing needs to be clicked on the PC.

## What already exists (and helps)

- **Tray app.** The desktop app already runs in the tray (`desktop/src-tauri/src/tray.rs`),
  and closing the window keeps it running (`main.rs`, `prevent_close`).
- **Launch at login.** `tauri-plugin-autostart` is wired up with a `--minimized`
  flag that keeps the window hidden. It is **off by default**
  (`settings.rs`: `start_with_windows: false`).
- **Trust.** Trusted devices can be auto-accepted, so no prompt is needed.
- **Offline link.** The Bluetooth call and Wi-Fi Direct work with no shared
  network, and the phone keeps mobile data.
- **Protocol room to grow.** HELLO carries `capabilities` (empty in 1.0), so
  new features can be negotiated without breaking older apps.
- **Android share sheet.** Trusted devices already appear as Direct Share
  targets.

In practice, "Beam not open on the PC" is mostly solved already, provided Beam
starts at login and stays in the tray. The real gaps are:
1. the default is off, and the tray process is heavier than it needs to be;
2. the phone can't choose a destination folder;
3. if someone quits Beam from the tray, nothing wakes it up again.

## Stage 1: "Always ready" (small; ship first)

1. **Ask once on first launch:** "Keep Beam ready in the background so your
   phone can send to this PC anytime?" Yes turns on start-at-login with
   `--minimized`. Settings keeps the switch.
2. **Make the tray process light.** When the window is hidden, close the
   webview (Tauri can destroy the window and recreate it from the tray).
   WebView2 is the heavy part. The Rust core, Bluetooth scanning and listener
   stay. Target: under 40 MB of RAM and no measurable CPU when idle. Measure
   before and after.
3. **Silent receive from trusted devices, with a Windows notification**
   (`tauri-plugin-notification`): "Saved IMG_0042.jpg to Desktop", with
   *Open* and *Show in folder* buttons. An untrusted sender still gets the
   existing prompt window.
4. **Clear quit.** The tray menu's Quit says "Your phone can't send to this
   PC until Beam starts again."

Acceptance: with the PC freshly rebooted and Beam never opened, the phone
finds it and sends a file. Check RAM and CPU while idle in the tray.

Built in 0.1.9:
- The one-time question on the Share page sets `askedBackground`.
- The main window is created on demand (`"create": false`) and destroyed
  on close. Measured on this PC: 185 MB across 7 processes with the window
  open, 8 MB in 1 process in the tray.
- Completed receives show a Windows notification when the window isn't
  open. It has no action buttons yet.
- The tray menu says "Quit Beam (stop receiving)".
- Not yet checked on hardware: the notification after a real phone send,
  and a reboot with start-at-login.

## Stage 2: "Choose where it lands" (the headline feature)

### Protocol (version 1.1, backward compatible)

- A new capability in HELLO: `"destinations-1"`.
- New message **0x0F DESTINATIONS** (receiver → sender, control). It is sent
  right after HELLO, and only when the peer is **trusted** and both sides
  advertise the capability:
  `[{id: text, label: text, kind: "downloads"|"desktop"|"documents"|"pictures"|"videos"|"custom"}]`
- OFFER gains an optional `destination: text` field (an id from that list).
  Older receivers ignore the unknown key and use their default folder.
- **The phone never sends a path.** The PC maps the id to a folder from its
  own allowlist. If the id is unknown, missing, or the folder is gone, the
  PC uses the default Beam folder and reports that in the transfer
  (`saved_to` in TRANSFER_COMPLETE). Filenames are sanitized as they are today.
- Later, optionally: `subfolder` (one path component, sanitized like a
  filename) for "Desktop/Trip photos". It is off unless the PC allows it.

### Desktop

- Settings → "Where your devices can save": Downloads (default), Desktop,
  Documents, Pictures, Videos, each with a switch, plus "Add a folder…" for
  pinned folders. Defaults: Downloads and Desktop on.
- These choices can be set per trusted device on the "Your devices" page.

### Android

- After picking files (or from the share sheet), the PC row shows a small
  **"Save to: Downloads ▾"** chip. Tapping it opens a bottom sheet with the
  PC's destinations. The last choice is remembered per PC.
- The phone caches each PC's destination list in its trust record, so the
  chip shows instantly without waiting for a connection. The list refreshes
  on every connection.
- In the share sheet: tapping a trusted PC sends to the remembered folder in
  one tap; a long press offers the other folders.

Acceptance: send to Desktop, Documents and a pinned folder, then check each
file's hash on the PC. Then:
- An old desktop build still receives (into its default folder).
- A forged id lands in the default folder.
- An untrusted phone never receives the list.

### Built (2026-11-03)

This is simpler than the design above:
- There is no separate DESTINATIONS message. The receiver puts
  `save-to:<id>` in its HELLO capabilities, only for trusted senders.
- OFFER carries an optional `destination`.
- The folders are Downloads, Desktop, Documents, Pictures and Videos, all
  allowed by default and switched in Settings. Pinned custom folders and
  subfolders come later.
- On Android, a trusted PC's row shows "Save to [folder ▾]" once the phone
  has learned that PC's list, so from the second send onward. The choice is
  remembered per PC.

## Stage 3: wake Beam when it isn't running (research spike first)

Beam ships through the Microsoft Store (MSIX), so it has package identity.
That allows a **background task** that Windows runs even when Beam is closed:

- `BluetoothLEAdvertisementWatcherTrigger` watches for Beam's Bluetooth
  advert from a trusted phone's "call" and wakes a small task. That task
  starts the tray process.
- Packaged desktop apps can register a full-trust COM background task with
  `BackgroundTaskBuilder.SetTaskEntryPointClsid` (Windows 10 2004 or later).
  The entry is an out-of-process COM server, which Rust can provide through
  `windows-rs`. The manifest declares a `windows.backgroundTasks` extension.

Unknowns to prove in a spike before committing:
- Do the Bluetooth triggers fire for a packaged Win32 app (not UWP) on this
  PC's Intel and Realtek radios?
- How long from the phone's call to Beam ready? The target is under 3 s.
- What happens with battery saver and Modern Standby?
- How does Windows treat a task that launches a full-trust process?

A **Windows service** is rejected. It runs outside the user's session, so it
can't show notifications or use the session-bound Wi-Fi Direct APIs, and it
writes into user folders as SYSTEM.

## Out of scope for now

- Waking a sleeping or lid-closed PC. Bluetooth is off in S3 sleep. Modern
  Standby machines might work later through Stage 3.
- PC → phone while Beam on the phone is closed. Android needs a foreground
  service and has its own limits; plan it separately.

## Order and size

| Stage | Work | Risk |
|---|---|---|
| 1 Always ready | ~2–3 days | Low: mostly settings, window lifecycle, notifications |
| 2 Destinations | ~1 week | Medium: protocol 1.1, core, FFI, both UIs, tests |
| 3 Wake on call | spike 2 days, then ~1 week | High: Windows background-task support for desktop apps |

Stages 1 and 2 together deliver the user-visible goal. Stage 3 only matters
for people who quit Beam from the tray.

## References

- [BackgroundTaskBuilder.SetTaskEntryPointClsid](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.background.backgroundtaskbuilder.settaskentrypointclsid?view=winrt-19041)
- [Create and register a background task (desktop apps)](https://learn.microsoft.com/en-us/windows/apps/develop/launch/create-and-register-a-background-task)
- [Windows.ApplicationModel.Background triggers](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.background)
- [tauri-plugin-autostart](https://www.npmjs.com/package/@tauri-apps/plugin-autostart)

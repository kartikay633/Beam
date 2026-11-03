# Beam interaction contract

Sources: docs/SPEC.md defines transfer acceptance, visibility and file lifecycle; docs/PROTOCOL.md defines device verification; docs/DECISIONS.md defines platform behavior. The current repair retains the ten-minute public-visibility policy and does not silently weaken trust.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Form | SettingsPage.tsx | Settings backend + this contract | Explicit save / undo; native select | Playwright draft persistence and navigation |
| Select/Listbox | Native HTML select in SettingsPage.tsx | DESIGN.md | OS-owned popup | Keyboard and browser checks |
| Scrollbar | desktop/ui/src/styles.css | DESIGN.md | Document-wide theme; main panel gutter | Narrow viewport Playwright check |
| Toast | InfoBar and semantic inline notices | This contract | Error / information / success | Pairing failure and retry test |
| Device trust | TrustedDevices in Devices.tsx | Core trust store | Toggle auto-accept; confirm remove | Shared owner on both screens |
| File sharing | SharePage / TransferList | Rust backend snapshot | Picker / drag / Explorer Send to | Production build + protocol tests |
| Pairing | PairPage | One-time backend QR token | QR / copied URI | Navigation and failure-path tests |
| Async actions | useAction.ts | Backend command result | One pending action per owner; inline retryable errors | Duplicate send and failure-path Playwright tests |

## Flow ledger

- Share is the initial screen. Every main screen remains reachable through the same navigation. A staged file is handed to the selected backend target without reopening the picker after a native drop.
- Only paired devices see trusted-only receivers. “Visible for 10 minutes” is an explicit discovery action. Expiry retains pairing/trust and switches to trusted-only.
- Incoming unknown senders require acceptance and expose the same six-digit verification code on both devices. Auto-accept requires an already trusted key.
- Pairing failure preserves the surrounding navigation and provides Try again. Late event subscriptions are detached when their view unmounts.
- Settings edits survive unrelated snapshots. Explicit Save applies settings; Undo restores the latest persisted settings. Native select geometry is intentional.
- Device name and download folder changes take effect after restarting Beam. The running receiver, its advertised port and its actual folder stay unchanged while settings are saved; Activity and Show in folder use the live folder.
- Visibility menus support arrow keys, Home/End and Escape, and restore trigger focus. Receive prompts show retryable load errors and disable acceptance after expiry.
- Snapshot events are subscribed before the initial read, so changes during startup are not lost.
- Both device-management screens use the same trust controls. Removing trust asks inline first, names the device, and explains that subsequent transfers require permission.
- Files have setup, waiting, active, reconnecting, completed, declined, cancelled, and failed states. Android performs its Downloads copy after protocol completion and reports saving separately.
- Native receive-offer windows are modeless, always-on-top app windows, not browser modal dialogs. Menus do not trap focus; Escape dismisses them and leaves focus on their trigger.
- Errors use live regions; keyboard focus is visible; reduced motion and dark theme are system preferences. No browser alert/confirm/prompt is part of a product flow.

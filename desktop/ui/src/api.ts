// Talks to the Rust backend. Outside Tauri (a plain browser, for UI work)
// it falls back to demo data so every screen can still be seen.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

export type VisibilityMode = "everyone" | "trusted" | "off";
export type TransferState =
  | "waiting"
  | "active"
  | "reconnecting"
  | "done"
  | "declined"
  | "failed"
  | "cancelled";

export interface Snapshot {
  me: { name: string; deviceId: string; port: number };
  visibility: { mode: VisibilityMode; everyoneUntilMs: number | null };
  nearby: Device[];
  transfers: Transfer[];
  trusted: TrustedDevice[];
  staged: StagedFile[];
  settings: Settings;
  /** Why Bluetooth isn't available, or null when it works. */
  bluetoothProblem: string | null;
  /** Something to show once, e.g. a failed Wi-Fi join. */
  notice: string | null;
}

export interface Device {
  deviceId: string;
  name: string;
  kind: "phone" | "laptop" | "device";
  address: string | null;
  trusted: boolean;
  /** "bluetooth": not on this network; reached through a private Wi-Fi link. */
  link: "lan" | "bluetooth";
  rssi: number | null;
}

export interface Transfer {
  id: string;
  direction: "send" | "receive";
  peerName: string;
  fileNames: string[];
  fileCount: number;
  totalBytes: number;
  doneBytes: number;
  state: TransferState;
  detail: string | null;
  speedBps: number;
  etaSecs: number | null;
  verificationCode: string | null;
  folder: string | null;
  startedMs: number;
}

export interface TrustedDevice {
  deviceId: string;
  name: string;
  autoAccept: boolean;
}

export interface StagedFile {
  path: string;
  name: string;
  size: number;
}

export interface Settings {
  deviceName: string;
  downloadDir: string;
  streams: number;
  visibility: VisibilityMode;
  sendToShortcut: boolean;
  startWithWindows: boolean;
}

export interface Offer {
  id: string;
  fromName: string;
  fromId: string;
  trusted: boolean;
  verificationCode: string;
  files: { name: string; size: number }[];
  totalBytes: number;
  expiresMs: number;
}

export interface PairingInfo {
  uri: string;
  svg: string;
  deviceId: string;
  addresses: string[];
  validForSecs: number;
}

export const isTauri = "__TAURI_INTERNALS__" in window;

export function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return isTauri ? tauriInvoke<T>(command, args) : demoInvoke<T>(command, args ?? {});
}

/** Calls `onSnapshot` with every new state from the backend. */
export async function onSnapshot(onSnapshot: (s: Snapshot) => void): Promise<() => void> {
  if (isTauri) {
    return tauriListen<Snapshot>("snapshot", (event) => onSnapshot(event.payload));
  }
  demoListeners.add(onSnapshot);
  return () => demoListeners.delete(onSnapshot);
}

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  offer: (id: string) => invoke<Offer | null>("get_offer", { id }),
  respond: (id: string, accept: boolean, trust: boolean) =>
    invoke<void>("respond_offer", { id, accept, trust }),
  stage: (paths: string[]) => invoke<void>("stage_files", { paths }),
  unstage: (path?: string) => invoke<void>("unstage_files", { path: path ?? null }),
  send: (target: string) => invoke<void>("send_staged", { target }),
  cancel: (id: string) => invoke<void>("cancel_transfer", { id }),
  clearFinished: () => invoke<void>("clear_finished"),
  dismissNotice: () => invoke<void>("dismiss_notice"),
  setVisibility: (mode: VisibilityMode) => invoke<void>("set_visibility", { mode }),
  setAutoAccept: (deviceId: string, autoAccept: boolean) =>
    invoke<void>("set_auto_accept", { deviceId, autoAccept }),
  removeTrusted: (deviceId: string) => invoke<void>("remove_trusted", { deviceId }),
  updateSettings: (settings: Settings) => invoke<void>("update_settings", { settings }),
  pairing: () => invoke<PairingInfo>("pairing_info"),
  openFolder: (path?: string) => invoke<void>("open_folder", { path: path ?? null }),
};

// ------------------------------------------------------------------ demo mode

const demoListeners = new Set<(s: Snapshot) => void>();
const now = Date.now();
const demo: Snapshot = {
  me: { name: "KARTIKAY-PC", deviceId: "80b4-eac6-82ed-d3d3", port: 9000 },
  visibility: { mode: "everyone", everyoneUntilMs: now + 7 * 60_000 },
  nearby: [
    { deviceId: "346e-029e-3df3-3625", name: "Kartikay's Pixel", kind: "phone", address: "192.168.1.23:9000", trusted: true, link: "lan", rssi: null },
    { deviceId: "9f12-77ab-0c44-e1d0", name: "Work ThinkPad", kind: "laptop", address: "192.168.1.40:9000", trusted: false, link: "lan", rssi: null },
    { deviceId: "ble-0a1b2c3d4e5f", name: "Galaxy S25", kind: "phone", address: null, trusted: false, link: "bluetooth", rssi: -58 },
  ],
  transfers: [
    {
      id: "t1", direction: "receive", peerName: "Kartikay's Pixel", fileNames: ["VID_20261027_181502.mp4"],
      fileCount: 1, totalBytes: 2_310_000_000, doneBytes: 1_470_000_000, state: "active", detail: null,
      speedBps: 48_200_000, etaSecs: 17, verificationCode: "482193", folder: "C:\\Users\\you\\Downloads\\Beam", startedMs: now - 30_000,
    },
    {
      id: "t2", direction: "send", peerName: "Work ThinkPad", fileNames: ["report.pdf", "slides.pptx", "notes.txt"],
      fileCount: 3, totalBytes: 18_400_000, doneBytes: 18_400_000, state: "done", detail: null,
      speedBps: 0, etaSecs: null, verificationCode: "093771", folder: null, startedMs: now - 600_000,
    },
    {
      id: "t3", direction: "send", peerName: "Kartikay's Pixel", fileNames: ["photo.heic"],
      fileCount: 1, totalBytes: 3_100_000, doneBytes: 0, state: "declined", detail: "declined",
      speedBps: 0, etaSecs: null, verificationCode: null, folder: null, startedMs: now - 900_000,
    },
  ],
  trusted: [{ deviceId: "346e-029e-3df3-3625", name: "Kartikay's Pixel", autoAccept: true }],
  staged: [],
  settings: {
    deviceName: "KARTIKAY-PC", downloadDir: "C:\\Users\\you\\Downloads\\Beam", streams: 4,
    visibility: "everyone", sendToShortcut: true, startWithWindows: false,
  },
  bluetoothProblem: null,
  notice: null,
};
const demoOffer: Offer = {
  id: "demo", fromName: "Kartikay's Pixel", fromId: "346e-029e-3df3-3625", trusted: false,
  verificationCode: "482193", totalBytes: 2_315_000_000, expiresMs: now + 60_000,
  files: [
    { name: "VID_20261027_181502.mp4", size: 2_310_000_000 },
    { name: "IMG_20261027_181433.jpg", size: 3_200_000 },
    { name: "IMG_20261027_181440.jpg", size: 1_800_000 },
  ],
};

function demoUpdate(change: (s: Snapshot) => void) {
  change(demo);
  const copy = structuredClone(demo);
  demoListeners.forEach((listener) => listener(copy));
}

async function demoInvoke<T>(command: string, args: Record<string, unknown>): Promise<T> {
  switch (command) {
    case "get_snapshot":
      return structuredClone(demo) as T;
    case "get_offer":
      return demoOffer as T;
    case "stage_files":
      demoUpdate((s) => {
        for (const path of args.paths as string[]) {
          s.staged.push({ path, name: path.split(/[\\/]/).pop() ?? path, size: 1_234_567 });
        }
      });
      break;
    case "unstage_files":
      demoUpdate((s) => {
        s.staged = args.path ? s.staged.filter((f) => f.path !== args.path) : [];
      });
      break;
    case "set_visibility":
      demoUpdate((s) => {
        s.visibility = {
          mode: args.mode as VisibilityMode,
          everyoneUntilMs: args.mode === "everyone" ? Date.now() + 600_000 : null,
        };
      });
      break;
    case "dismiss_notice":
      demoUpdate((s) => {
        s.notice = null;
      });
      break;
    case "clear_finished":
      demoUpdate((s) => {
        s.transfers = s.transfers.filter((t) => !["done", "declined", "failed", "cancelled"].includes(t.state));
      });
      break;
    case "pairing_info":
      return {
        uri: "beam://pair?v=1&id=80b4eac682edd3d3&key=…",
        svg: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><rect width="21" height="21" fill="#fff"/><path d="M1 1h7v7H1zM13 1h7v7h-7zM1 13h7v7H1z" fill="#000"/><path d="M3 3h3v3H3zM15 3h3v3h-3zM3 15h3v3H3z" fill="#fff"/><path d="M10 10h2v2h-2zM14 14h3v3h-3z" fill="#000"/></svg>',
        deviceId: demo.me.deviceId,
        addresses: ["192.168.1.17:9000"],
        validForSecs: 300,
      } as T;
    case "send_staged":
      demoUpdate((s) => {
        s.transfers.unshift({
          id: crypto.randomUUID(), direction: "send", peerName: "Kartikay's Pixel",
          fileNames: s.staged.map((f) => f.name).slice(0, 5), fileCount: s.staged.length,
          totalBytes: s.staged.reduce((n, f) => n + f.size, 0), doneBytes: 0, state: "waiting",
          detail: "Waiting for them to accept…", speedBps: 0, etaSecs: null, verificationCode: "771204",
          folder: null, startedMs: Date.now(),
        });
        s.staged = [];
      });
      break;
  }
  return undefined as T;
}

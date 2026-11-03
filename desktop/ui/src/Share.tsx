import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, isTauri, type Device, type Snapshot, type Transfer } from "./api";
import type { Page } from "./App";
import { useCountdown } from "./App";
import { TransferList } from "./Transfers";
import { useAction } from "./useAction";
import { formatBytes, formatMinutesLeft } from "./format";
import {
  BluetoothIcon,
  CheckIcon,
  ChevronDownIcon,
  CloseIcon,
  DeviceIcon,
  ErrorIcon,
  EyeIcon,
  EyeOffIcon,
  LaptopIcon,
  PhoneIcon,
  PlusIcon,
} from "./icons";

export function SharePage({ snapshot, onNavigate }: { snapshot: Snapshot; onNavigate: (page: Page) => void }) {
  const action = useAction();
  const { error } = action;
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const staged = snapshot.staged;

  /** Asks for files and stages them; false when none were picked. */
  const choose = async (): Promise<boolean> => {
    if (!isTauri) {
      await api.stage(["C:\\Users\\you\\Videos\\holiday.mp4", "C:\\Users\\you\\Documents\\notes.txt"]);
      return true;
    }
    const picked = await open({ multiple: true, title: "Choose files to send" });
    if (!picked || (Array.isArray(picked) && picked.length === 0)) return false;
    await api.stage(Array.isArray(picked) ? picked : [picked]);
    return true;
  };

  const sendTo = (deviceId: string) => action.run(async () => {
      if (snapshot.staged.length === 0 && !(await choose())) return;
      await api.send(deviceId);
  });
  const actionRef = useRef(action);
  actionRef.current = action;

  // Files dragged onto the window: dropped on a device they go straight
  // there; dropped anywhere else they wait for a device to be picked.
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    const deviceAt = (x: number, y: number) => {
      const ratio = window.devicePixelRatio || 1;
      const element = document.elementFromPoint(x / ratio, y / ratio);
      return element?.closest<HTMLElement>("[data-device]")?.dataset.device ?? null;
    };
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setDragging(true);
          setDropTarget(deviceAt(payload.position.x, payload.position.y));
        } else if (payload.type === "drop") {
          const target = deviceAt(payload.position.x, payload.position.y);
          setDragging(false);
          setDropTarget(null);
          void actionRef.current.run(async () => {
            await api.stage(payload.paths);
            if (target) await api.send(target);
          });
        } else {
          setDragging(false);
          setDropTarget(null);
        }
      })
      .then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; })
      .catch((e) => { if (!disposed) void actionRef.current.run(async () => { throw e; }); });
    return () => { disposed = true; stop?.(); };
  }, []);

  const recent = snapshot.transfers.slice(0, 3);
  return (
    <div className="page">
      <header className="page-header">
        <h1>Share</h1>
        <VisibilityButton snapshot={snapshot} />
      </header>

      {error && (
        <InfoBar kind="error" onClose={action.clearError}>
          {error}
        </InfoBar>
      )}
      {snapshot.notice && (
        <InfoBar kind="error" onClose={() => { void action.run(() => api.dismissNotice()); }}>
          {snapshot.notice}
        </InfoBar>
      )}
      {snapshot.bluetoothProblem && (
        <InfoBar kind="info">
          Bluetooth isn't available ({snapshot.bluetoothProblem}), so Beam only finds devices on the same Wi-Fi.
        </InfoBar>
      )}

      {!snapshot.settings.askedBackground && !snapshot.settings.startWithWindows && (
        <ReadyPrompt snapshot={snapshot} />
      )}

      {staged.length > 0 && (
        <div className="staged-bar">
          <span className="grow">
            <strong>
              {staged.length} file{staged.length === 1 ? "" : "s"} ready
            </strong>{" "}
            · {formatBytes(staged.reduce((sum, f) => sum + f.size, 0))}
          </span>
          <button className="button subtle" disabled={action.busy} onClick={() => action.run(choose)}>
            <PlusIcon size={16} /> Add more
          </button>
          <button className="button subtle" disabled={action.busy} onClick={() => action.run(() => api.unstage())}>
            Clear
          </button>
        </div>
      )}

      <DeviceList
        snapshot={snapshot}
        dragging={dragging}
        dropTarget={dropTarget}
        onSend={sendTo}
        onPair={() => onNavigate("devices")}
        busy={action.busy}
      />

      <section className="section" aria-labelledby="recent">
        <div className="section-header">
          <h2 id="recent">Recent</h2>
          {snapshot.transfers.length > 3 && (
            <button className="hyperlink" onClick={() => onNavigate("activity")}>
              See all
            </button>
          )}
        </div>
        {recent.length === 0 ? (
          <p className="secondary empty-line">Files you send and receive show up here.</p>
        ) : (
          <TransferList transfers={recent} />
        )}
      </section>
    </div>
  );
}

/** Nearby devices as plain rows: click one (or drop files on it) to send. */
function DeviceList({
  snapshot,
  dragging,
  dropTarget,
  onSend,
  onPair,
  busy,
}: {
  snapshot: Snapshot;
  dragging: boolean;
  dropTarget: string | null;
  onSend: (deviceId: string) => void;
  onPair: () => void;
  busy: boolean;
}) {
  const devices = snapshot.nearby;
  const ready = snapshot.staged.length > 0;
  const sending = (device: Device): Transfer | undefined =>
    snapshot.transfers.find(
      (t) => t.direction === "send" && t.peerName === device.name && (t.state === "active" || t.state === "waiting"),
    );
  return (
    <section className="section" aria-labelledby="nearby">
      <div className="section-header">
        <h2 id="nearby">Nearby devices</h2>
        {devices.length > 0 && (
          <span className="secondary small">
            {dragging ? "Drop on a device to send" : ready ? "Pick a device" : "Pick a device, then choose files"}
          </span>
        )}
      </div>
      {devices.length === 0 ? (
        <div className={`list device-empty ${dragging ? "dragging" : ""}`}>
          <span className="searching-dot" aria-hidden="true" />
          <div className="grow">
            <div className="strong">Looking for your devices…</div>
            <p className="secondary">
              Open Beam on your phone with Bluetooth and Wi-Fi on. No shared Wi-Fi needed.
            </p>
            <button className="hyperlink" onClick={onPair}>
              Pair a phone
            </button>
          </div>
        </div>
      ) : (
        <ul className={`list devices ${dragging ? "dragging" : ""}`}>
          {devices.map((device) => {
            const transfer = sending(device);
            const fraction = transfer && transfer.totalBytes > 0 ? transfer.doneBytes / transfer.totalBytes : null;
            const status = transfer
              ? transfer.state === "active" && fraction !== null
                ? `Sending · ${Math.round(fraction * 100)}%`
                : "Connecting…"
              : [device.trusted ? "Your device" : null, device.link === "bluetooth" ? "Nearby, no shared Wi-Fi" : "Same Wi-Fi"]
                  .filter(Boolean)
                  .join(" · ");
            return (
              <li key={device.deviceId}>
                <button
                  className={`device-row ${dropTarget === device.deviceId ? "target" : ""}`}
                  data-device={device.deviceId}
                  title={device.name}
                  onClick={() => onSend(device.deviceId)}
                  disabled={busy || !!transfer}
                  aria-busy={busy}
                >
                  <span className="device-icon" aria-hidden="true">
                    {device.kind === "phone" ? (
                      <PhoneIcon size={20} />
                    ) : device.kind === "laptop" ? (
                      <LaptopIcon size={20} />
                    ) : (
                      <DeviceIcon size={20} />
                    )}
                  </span>
                  <span className="grow device-text">
                    <span className="device-name">{device.name}</span>
                    <span className={`device-status ${transfer ? "active" : ""}`}>
                      {device.link === "bluetooth" && !transfer && <BluetoothIcon size={12} />}
                      {status}
                    </span>
                    {fraction !== null && (
                      <span className="device-progress" aria-hidden="true">
                        <span style={{ width: `${Math.round(fraction * 100)}%` }} />
                      </span>
                    )}
                  </span>
                  {!transfer && <span className="device-action">{ready ? "Send" : "Send files"}</span>}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/** Asked once: start with Windows, hidden in the tray, so phones can send anytime. */
function ReadyPrompt({ snapshot }: { snapshot: Snapshot }) {
  const action = useAction();
  const answer = (on: boolean) =>
    action.run(() =>
      api.updateSettings({
        ...snapshot.settings,
        startWithWindows: on || snapshot.settings.startWithWindows,
        askedBackground: true,
      }),
    );
  return (
    <div className="ready-prompt" role="region" aria-label="Keep Beam ready">
      <div className="grow">
        <div className="strong">Keep Beam ready in the background?</div>
        <p className="secondary">
          Beam starts with Windows and waits in the tray, so your phone can send to this PC anytime, even with
          this window closed.
        </p>
        {action.error && <p className="error-text">{action.error}</p>}
      </div>
      <div className="ready-actions">
        <button className="button primary" disabled={action.busy} onClick={() => answer(true)}>
          Turn on
        </button>
        <button className="button subtle" disabled={action.busy} onClick={() => answer(false)}>
          Not now
        </button>
      </div>
    </div>
  );
}

const MODES = [
  { id: "everyone", label: "Everyone", hint: "Anyone nearby, for 10 minutes" },
  { id: "trusted", label: "Your devices", hint: "Only devices you've paired or trusted" },
  { id: "off", label: "No one", hint: "Nobody can find or send to this PC" },
] as const;

/** "Everyone can find this PC · 9 min", opening a menu to change it. */
function VisibilityButton({ snapshot }: { snapshot: Snapshot }) {
  const { mode, everyoneUntilMs } = snapshot.visibility;
  const remaining = useCountdown(everyoneUntilMs);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const action = useAction();
  const dismiss = () => { setOpen(false); trigger.current?.focus(); };
  useEffect(() => {
    if (open) ref.current?.querySelector<HTMLButtonElement>('[role="menuitemradio"][aria-checked="true"]')?.focus();
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent) {
        if (e.key === "Escape") { e.preventDefault(); dismiss(); }
      } else if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [open]);
  const label =
    mode === "everyone"
      ? `Everyone can find this PC${remaining ? ` · ${formatMinutesLeft(remaining)}` : ""}`
      : mode === "trusted"
        ? "Your devices can find this PC"
        : "Hidden";
  return (
    <div className="menu-anchor" ref={ref}>
      <button ref={trigger} className="button" aria-haspopup="menu" aria-expanded={open} disabled={action.busy}
        onKeyDown={(e) => { if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); setOpen(true); } }}
        onClick={() => setOpen(!open)}>
        {mode === "off" ? <EyeOffIcon size={16} /> : <EyeIcon size={16} />}
        <span className={`dot ${mode}`} />
        {label}
        <ChevronDownIcon size={12} />
      </button>
      {open && (
        <div className="flyout" role="menu" onKeyDown={(e) => {
          const items = Array.from(e.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'));
          const index = items.indexOf(document.activeElement as HTMLButtonElement);
          if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
            e.preventDefault();
            const next = e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : (index + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
            items[next]?.focus();
          } else if (e.key === "Tab") setOpen(false);
        }}>
          {MODES.map((m) => (
            <button
              key={m.id}
              role="menuitemradio"
              aria-checked={mode === m.id}
              className="flyout-item"
              onClick={async () => {
                dismiss();
                await action.run(() => api.setVisibility(m.id));
                trigger.current?.focus();
              }}
            >
              <span className="flyout-check">{mode === m.id && <CheckIcon size={14} />}</span>
              <span className="grow">
                <span className="flyout-label">{m.label}</span>
                <span className="flyout-hint">{m.hint}</span>
              </span>
            </button>
          ))}
        </div>
      )}
      {action.error && <InfoBar kind="error" onClose={action.clearError}>{action.error}</InfoBar>}
    </div>
  );
}

/** A Windows InfoBar: a message with an icon and an optional close button. */
export function InfoBar({
  kind,
  children,
  onClose,
}: {
  kind: "error" | "info" | "success";
  children: React.ReactNode;
  onClose?: () => void;
}) {
  return (
    <div className={`infobar ${kind}`} role={kind === "error" ? "alert" : "status"}>
      <span className="infobar-icon">{kind === "success" ? <CheckIcon size={12} /> : <ErrorIcon size={16} />}</span>
      <span className="grow">{children}</span>
      {onClose && (
        <button className="icon-button" aria-label="Close" onClick={onClose}>
          <CloseIcon size={14} />
        </button>
      )}
    </div>
  );
}

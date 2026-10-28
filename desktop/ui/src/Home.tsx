import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, isTauri, type Device, type Snapshot } from "./api";
import { Transfers } from "./Transfers";
import { formatBytes } from "./format";
import { CloseIcon, DeviceIcon, FileIcon, LaptopIcon, PhoneIcon, RadarIcon, UploadIcon } from "./icons";

export function Home({ snapshot }: { snapshot: Snapshot }) {
  const [error, setError] = useState<string | null>(null);
  const send = (target: string) => {
    setError(null);
    api.send(target).catch((e) => setError(String(e)));
  };
  return (
    <div className="page">
      <header className="page-header">
        <h1>Send &amp; receive</h1>
        <p className="muted">
          Drop files, pick a device. Files sent to you are saved in{" "}
          <button className="link" onClick={() => api.openFolder()}>
            {snapshot.settings.downloadDir}
          </button>
          .
        </p>
      </header>
      <DropZone snapshot={snapshot} />
      {error && <div className="banner error">{error}</div>}
      <NearbyDevices snapshot={snapshot} onSend={send} />
      <Transfers transfers={snapshot.transfers} />
    </div>
  );
}

function DropZone({ snapshot }: { snapshot: Snapshot }) {
  const [dragging, setDragging] = useState(false);

  // Files dragged from Explorer onto the window.
  useEffect(() => {
    if (!isTauri) return;
    let stop: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const { type } = event.payload;
        if (type === "enter" || type === "over") setDragging(true);
        else if (type === "drop") {
          setDragging(false);
          api.stage(event.payload.paths);
        } else setDragging(false);
      })
      .then((unlisten) => (stop = unlisten));
    return () => stop?.();
  }, []);

  const choose = async () => {
    if (!isTauri) {
      api.stage(["C:\\Users\\you\\Pictures\\holiday.jpg", "C:\\Users\\you\\Documents\\notes.txt"]);
      return;
    }
    const picked = await open({ multiple: true, title: "Choose files to send" });
    if (picked) api.stage(Array.isArray(picked) ? picked : [picked]);
  };

  const staged = snapshot.staged;
  if (staged.length === 0) {
    return (
      <button className={`dropzone ${dragging ? "dragging" : ""}`} onClick={choose}>
        <UploadIcon size={28} />
        <strong>Drop files here</strong>
        <span className="muted">or click to choose · or right-click a file in Explorer → Send to → Beam</span>
      </button>
    );
  }
  const total = staged.reduce((sum, f) => sum + f.size, 0);
  return (
    <section className={`staged ${dragging ? "dragging" : ""}`}>
      <div className="staged-header">
        <div>
          <strong>
            {staged.length} file{staged.length === 1 ? "" : "s"} ready
          </strong>
          <span className="muted"> · {formatBytes(total)} · now pick a device below</span>
        </div>
        <div className="row-actions">
          <button className="ghost" onClick={choose}>
            Add more
          </button>
          <button className="ghost" onClick={() => api.unstage()}>
            Clear
          </button>
        </div>
      </div>
      <ul className="chips">
        {staged.map((file) => (
          <li key={file.path} className="chip" title={file.path}>
            <FileIcon size={16} />
            <span className="chip-name">{file.name}</span>
            <span className="muted">{formatBytes(file.size)}</span>
            <button className="icon-button" aria-label={`Remove ${file.name}`} onClick={() => api.unstage(file.path)}>
              <CloseIcon size={14} />
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function NearbyDevices({ snapshot, onSend }: { snapshot: Snapshot; onSend: (target: string) => void }) {
  const [address, setAddress] = useState("");
  const ready = snapshot.staged.length > 0;
  return (
    <section>
      <div className="section-header">
        <h2>Nearby</h2>
        <span className="muted small">Devices on this network running Beam</span>
      </div>
      {snapshot.nearby.length === 0 ? (
        <div className="empty">
          <RadarIcon size={28} />
          <div>
            <strong>Looking for devices…</strong>
            <div className="muted small">
              Make sure the other device is on the same Wi-Fi and has Beam open. Devices set to "trusted only" appear
              once you trust each other.
            </div>
          </div>
        </div>
      ) : (
        <div className="device-grid">
          {snapshot.nearby.map((device) => (
            <DeviceCard key={device.deviceId} device={device} ready={ready} onSend={onSend} />
          ))}
        </div>
      )}
      <form
        className="address-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (address.trim()) onSend(address.trim());
        }}
      >
        <span className="muted small">Not listed? Send to an address:</span>
        <input
          value={address}
          onChange={(e) => setAddress(e.target.value)}
          placeholder="192.168.1.20:9000"
          aria-label="Receiver address"
        />
        <button type="submit" className="ghost" disabled={!ready || !address.trim()}>
          Send
        </button>
      </form>
    </section>
  );
}

function DeviceCard({ device, ready, onSend }: { device: Device; ready: boolean; onSend: (target: string) => void }) {
  const icon =
    device.kind === "phone" ? <PhoneIcon size={26} /> : device.kind === "laptop" ? <LaptopIcon size={26} /> : <DeviceIcon size={26} />;
  return (
    <div className="device-card" title={device.deviceId}>
      <div className="device-avatar">{icon}</div>
      <div className="device-info">
        <div className="device-name">{device.name}</div>
        <div className="muted small">
          {device.kind === "phone" ? "Phone" : device.kind === "laptop" ? "Computer" : "Device"}
          {device.trusted ? " · Trusted" : ""}
        </div>
      </div>
      <button
        className="primary"
        disabled={!ready}
        title={ready ? `Send to ${device.name}` : "Choose some files first"}
        onClick={() => onSend(device.deviceId)}
      >
        Send
      </button>
    </div>
  );
}

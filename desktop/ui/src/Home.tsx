import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, isTauri, type Device, type Snapshot } from "./api";
import { Transfers } from "./Transfers";
import { formatBytes } from "./format";
import { BluetoothIcon, CloseIcon, DeviceIcon, FileIcon, LaptopIcon, PhoneIcon, PlusIcon, UploadIcon } from "./icons";

export function Home({ snapshot }: { snapshot: Snapshot }) {
  const [error, setError] = useState<string | null>(null);
  const dragging = useWindowDrop();

  /** Asks for files and stages them; false when the user picked none. */
  const choose = async (): Promise<boolean> => {
    if (!isTauri) {
      await api.stage(["C:\\Users\\you\\Pictures\\holiday.jpg", "C:\\Users\\you\\Documents\\notes.txt"]);
      return true;
    }
    const picked = await open({ multiple: true, title: "Choose files to send" });
    if (!picked || (Array.isArray(picked) && picked.length === 0)) return false;
    await api.stage(Array.isArray(picked) ? picked : [picked]);
    return true;
  };

  /** Sends what's staged, asking for files first when nothing is. */
  const sendTo = async (target: string) => {
    setError(null);
    try {
      if (snapshot.staged.length === 0 && !(await choose())) return;
      await api.send(target);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="page">
      {dragging && (
        <div className="drop-overlay" aria-hidden="true">
          <UploadIcon size={28} />
          <span>Drop to add files</span>
        </div>
      )}
      {error && (
        <div className="notice error" role="alert">
          <span>{error}</span>
          <button className="text-button" onClick={() => setError(null)}>
            Dismiss
          </button>
        </div>
      )}
      {snapshot.notice && (
        <div className="notice error" role="alert">
          <span>{snapshot.notice}</span>
          <button className="text-button" onClick={() => api.dismissNotice()}>
            Dismiss
          </button>
        </div>
      )}
      <SendPanel snapshot={snapshot} onChoose={choose} />
      <NearbyDevices snapshot={snapshot} onSend={sendTo} />
      <Transfers transfers={snapshot.transfers} />
      <p className="footnote">
        Received files go to <span className="path">{snapshot.settings.downloadDir}</span> ·{" "}
        <button className="text-button" onClick={() => api.openFolder()}>
          Open folder
        </button>
      </p>
    </div>
  );
}

/** Whether files are being dragged over the window; dropped files are staged. */
function useWindowDrop(): boolean {
  const [dragging, setDragging] = useState(false);
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
  return dragging;
}

function SendPanel({ snapshot, onChoose }: { snapshot: Snapshot; onChoose: () => Promise<boolean> }) {
  const staged = snapshot.staged;
  if (staged.length === 0) {
    return (
      <section className="send-strip" aria-label="Send files">
        <div className="tile">
          <UploadIcon size={20} />
        </div>
        <div className="grow">
          <p className="strong">Drop files anywhere to send</p>
          <p className="secondary">Or right-click any file in Explorer, then Send to › Beam</p>
        </div>
        <button className="button" onClick={onChoose}>
          Choose files
        </button>
      </section>
    );
  }
  const total = staged.reduce((sum, f) => sum + f.size, 0);
  return (
    <section className="staged" aria-label="Files to send">
      <div className="staged-header">
        <div className="grow">
          <p className="strong">
            {staged.length} file{staged.length === 1 ? "" : "s"} ready · {formatBytes(total)}
          </p>
          <p className="secondary">Pick a device to send {staged.length === 1 ? "it" : "them"} to</p>
        </div>
        <button className="text-button" onClick={onChoose}>
          <PlusIcon size={15} /> Add more
        </button>
        <button className="text-button" onClick={() => api.unstage()}>
          Clear
        </button>
      </div>
      <ul className="file-chips">
        {staged.map((file) => (
          <li key={file.path} title={file.path}>
            <FileIcon size={15} />
            <span className="ellipsis">{file.name}</span>
            <span className="secondary">{formatBytes(file.size)}</span>
            <button className="chip-remove" aria-label={`Remove ${file.name}`} onClick={() => api.unstage(file.path)}>
              <CloseIcon size={13} />
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function NearbyDevices({ snapshot, onSend }: { snapshot: Snapshot; onSend: (target: string) => void }) {
  const [address, setAddress] = useState("");
  const [byAddress, setByAddress] = useState(false);
  const count = snapshot.staged.length;
  return (
    <section aria-labelledby="nearby">
      <div className="section-header">
        <h2 id="nearby">Nearby</h2>
        <span className="secondary">
          {snapshot.bluetoothProblem
            ? `Same Wi-Fi only · Bluetooth unavailable: ${snapshot.bluetoothProblem}`
            : "Same Wi-Fi, or close by over Bluetooth"}
        </span>
      </div>
      <div className="list">
        {snapshot.nearby.length === 0 ? (
          <div className="list-empty">
            <span className="searching" aria-hidden="true" />
            <div>
              <p className="strong">Looking for devices…</p>
              <p className="secondary">
                Open Beam on your phone. It shows up here on the same Wi-Fi, or over Bluetooth with no shared network.
              </p>
            </div>
          </div>
        ) : (
          snapshot.nearby.map((device) => (
            <DeviceRow key={device.deviceId} device={device} count={count} onSend={onSend} />
          ))
        )}
      </div>
      {byAddress ? (
        <form
          className="address-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (address.trim()) onSend(address.trim());
          }}
        >
          <input
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            placeholder="192.168.1.20:9000"
            aria-label="Address to send to"
            autoFocus
          />
          <button type="submit" className="button">
            Send
          </button>
          <button type="button" className="text-button" onClick={() => setByAddress(false)}>
            Cancel
          </button>
        </form>
      ) : (
        <button className="text-button subtle" onClick={() => setByAddress(true)}>
          Send to an address instead
        </button>
      )}
    </section>
  );
}

function DeviceRow({ device, count, onSend }: { device: Device; count: number; onSend: (target: string) => void }) {
  const icon =
    device.kind === "phone" ? <PhoneIcon size={20} /> : device.kind === "laptop" ? <LaptopIcon size={20} /> : <DeviceIcon size={20} />;
  const details = [device.trusted ? "Trusted" : null, device.link === "bluetooth" ? "No Wi-Fi needed" : "Same Wi-Fi"]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="row" title={device.deviceId}>
      <div className="tile">{icon}</div>
      <div className="grow">
        <p className="strong">{device.name}</p>
        <p className="secondary with-icon">
          {device.link === "bluetooth" && <BluetoothIcon size={12} />}
          {details}
        </p>
      </div>
      <button className="button primary" onClick={() => onSend(device.deviceId)}>
        {count === 0 ? "Send files" : `Send ${count} file${count === 1 ? "" : "s"}`}
      </button>
    </div>
  );
}

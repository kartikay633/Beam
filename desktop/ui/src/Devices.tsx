import { useState } from "react";
import { api, type Snapshot } from "./api";
import { PairPage } from "./Pair";
import { DeviceIcon } from "./icons";
import { useAction } from "./useAction";

export function DevicesPage({ snapshot }: { snapshot: Snapshot }) {
  const action = useAction();
  return (
    <div className="page">
      <header className="page-header"><h1>Your devices</h1></header>
      <p className="secondary">Paired devices can find this PC in “Your devices” visibility mode.</p>
      {snapshot.visibility.mode !== "everyone" && <div className="notice">
        <span>To connect a new phone, let it discover this PC nearby.</span>
        <button className="button" disabled={action.busy} onClick={() => action.run(() => api.setVisibility("everyone"))}>Visible for 10 minutes</button>
      </div>}
      {action.error && <p className="notice error" role="alert">{action.error}</p>}
      <TrustedDevices devices={snapshot.trusted} />
      <PairPage />
    </div>
  );
}

export function TrustedDevices({ devices }: { devices: Snapshot["trusted"] }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);

  const change = async (id: string, action: () => Promise<void>) => {
    setBusy(id);
    setError(null);
    try {
      await action();
      setRemoving(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div>
      {error && <p className="notice error" role="alert">{error}</p>}
      <section aria-label="Paired devices" className="list">
        {devices.length === 0 ? (
          <p className="list-empty">No paired devices yet. Scan the code below from Beam on your phone.</p>
        ) : devices.map((device) => (
          <div className="row" key={device.deviceId}>
            <DeviceIcon size={24} />
            <div className="grow">
              <p className="strong">{device.name}</p>
              <p className="caption mono">{device.deviceId}</p>
              <label className="check">
                <input type="checkbox" checked={device.autoAccept} disabled={busy === device.deviceId}
                  onChange={(event) => change(device.deviceId, () => api.setAutoAccept(device.deviceId, event.target.checked))} />
                Automatically accept files from this device
              </label>
              {removing === device.deviceId && <p className="caption">Remove trust for {device.name}? Future transfers will ask for permission.</p>}
            </div>
            {removing === device.deviceId ? (
              <>
                <button className="button" disabled={busy === device.deviceId} onClick={() => setRemoving(null)}>Keep device</button>
                <button className="button danger" disabled={busy === device.deviceId}
                  onClick={() => change(device.deviceId, () => api.removeTrusted(device.deviceId))}>Remove device</button>
              </>
            ) : <button className="button" onClick={() => setRemoving(device.deviceId)}>Remove</button>}
          </div>
        ))}
      </section>
    </div>
  );
}

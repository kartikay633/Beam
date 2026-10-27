import { api, type Snapshot } from "./api";
import { ShieldIcon } from "./icons";

export function TrustedPage({ snapshot }: { snapshot: Snapshot }) {
  return (
    <div className="page">
      <header className="page-header">
        <h1>Trusted devices</h1>
        <p className="muted">
          Trusted devices can find you even in "trusted only" mode. With auto-accept on, their files arrive without
          asking. A device becomes trusted when you tick "Trust this device" while accepting its files, or by pairing.
        </p>
      </header>
      {snapshot.trusted.length === 0 ? (
        <div className="empty">
          <ShieldIcon size={28} />
          <div className="muted">No trusted devices yet.</div>
        </div>
      ) : (
        <ul className="card-list">
          {snapshot.trusted.map((device) => (
            <li key={device.deviceId} className="card-row">
              <div className="grow">
                <div className="device-name">{device.name}</div>
                <div className="muted small mono">{device.deviceId}</div>
              </div>
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={device.autoAccept}
                  onChange={(e) => api.setAutoAccept(device.deviceId, e.target.checked)}
                />
                <span>Auto-accept</span>
              </label>
              <button
                className="ghost danger"
                onClick={() => {
                  if (confirm(`Stop trusting ${device.name}?`)) api.removeTrusted(device.deviceId);
                }}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

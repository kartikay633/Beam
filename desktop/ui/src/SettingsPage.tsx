import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, isTauri, type Settings, type Snapshot } from "./api";

export function SettingsPage({ snapshot }: { snapshot: Snapshot }) {
  const [draft, setDraft] = useState<Settings>(snapshot.settings);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  useEffect(() => setDraft(snapshot.settings), [snapshot.settings]);

  const changed = JSON.stringify(draft) !== JSON.stringify(snapshot.settings);
  const nameChanged = draft.deviceName !== snapshot.settings.deviceName;
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => setDraft({ ...draft, [key]: value });

  const save = async () => {
    try {
      await api.updateSettings(draft);
      setMessage({
        kind: "ok",
        text: nameChanged ? "Saved. Others see the new name after Beam restarts." : "Saved",
      });
    } catch (e) {
      setMessage({ kind: "error", text: String(e) });
    }
  };

  const browse = async () => {
    if (!isTauri) return;
    const folder = await open({ directory: true, title: "Where should received files go?" });
    if (typeof folder === "string") set("downloadDir", folder);
  };

  return (
    <div className="page narrow">
      <form
        className="settings"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <section aria-labelledby="this-pc">
          <h2 id="this-pc" className="section-title">
            This computer
          </h2>
          <div className="list form-list">
            <label className="field">
              <span className="strong">Name</span>
              <span className="secondary">What other devices see</span>
              <input value={draft.deviceName} onChange={(e) => set("deviceName", e.target.value)} maxLength={63} />
            </label>
            <label className="field">
              <span className="strong">Save received files to</span>
              <div className="input-row">
                <input value={draft.downloadDir} onChange={(e) => set("downloadDir", e.target.value)} />
                <button type="button" className="button" onClick={browse}>
                  Browse
                </button>
              </div>
            </label>
            <label className="field">
              <span className="strong">Who can see this computer when Beam starts</span>
              <select
                value={draft.visibility}
                onChange={(e) => set("visibility", e.target.value as Settings["visibility"])}
              >
                <option value="everyone">Everyone, for 10 minutes</option>
                <option value="trusted">Trusted devices</option>
                <option value="off">Nobody (hidden)</option>
              </select>
            </label>
            <label className="field">
              <span className="strong">Parallel connections when sending</span>
              <span className="secondary">More can be faster on fast networks</span>
              <select value={draft.streams} onChange={(e) => set("streams", Number(e.target.value))}>
                {[1, 2, 4, 8].map((n) => (
                  <option key={n} value={n}>
                    {n}
                    {n === 4 ? " (recommended)" : ""}
                  </option>
                ))}
              </select>
            </label>
            <label className="switch-row">
              <span className="grow">
                <span className="strong">Show Beam in Explorer's Send to menu</span>
              </span>
              <input
                type="checkbox"
                role="switch"
                checked={draft.sendToShortcut}
                onChange={(e) => set("sendToShortcut", e.target.checked)}
              />
            </label>
            <label className="switch-row">
              <span className="grow">
                <span className="strong">Start with Windows</span>
                <span className="secondary">Beam waits in the tray, ready to receive</span>
              </span>
              <input
                type="checkbox"
                role="switch"
                checked={draft.startWithWindows}
                onChange={(e) => set("startWithWindows", e.target.checked)}
              />
            </label>
          </div>
          <div className="form-actions">
            <button type="submit" className="button primary" disabled={!changed}>
              Save changes
            </button>
            {changed && (
              <button type="button" className="text-button" onClick={() => setDraft(snapshot.settings)}>
                Undo
              </button>
            )}
            {message && (
              <span role="status" className={message.kind === "ok" ? "ok-text" : "error-text"}>
                {message.text}
              </span>
            )}
          </div>
        </section>
      </form>

      <section aria-labelledby="trusted">
        <h2 id="trusted" className="section-title">
          Trusted devices
        </h2>
        <p className="secondary section-intro">
          Trusted devices can always find you. With auto-accept on, their files arrive without asking. Pair a phone, or
          tick "Always accept" on an incoming request, to trust it.
        </p>
        {snapshot.trusted.length === 0 ? (
          <p className="secondary empty-line">No trusted devices yet.</p>
        ) : (
          <ul className="list">
            {snapshot.trusted.map((device) => (
              <li key={device.deviceId} className="row">
                <div className="grow">
                  <p className="strong">{device.name}</p>
                  <p className="secondary mono">{device.deviceId}</p>
                </div>
                <label className="inline-switch">
                  <span className="secondary">Auto-accept</span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={device.autoAccept}
                    onChange={(e) => api.setAutoAccept(device.deviceId, e.target.checked)}
                  />
                </label>
                <button
                  className="text-button danger"
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
      </section>

      <p className="footnote">
        {snapshot.me.name} · <span className="mono">{snapshot.me.deviceId}</span>
      </p>
    </div>
  );
}

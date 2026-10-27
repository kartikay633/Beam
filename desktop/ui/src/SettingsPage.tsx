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
        text: nameChanged ? "Saved. The new name is shown to others after Beam restarts." : "Saved.",
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
    <div className="page">
      <header className="page-header">
        <h1>Settings</h1>
      </header>
      <form
        className="settings"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <label className="field">
          <span>Device name</span>
          <input value={draft.deviceName} onChange={(e) => set("deviceName", e.target.value)} maxLength={63} />
          <small className="muted">What other devices see.</small>
        </label>

        <label className="field">
          <span>Save received files to</span>
          <div className="input-row">
            <input value={draft.downloadDir} onChange={(e) => set("downloadDir", e.target.value)} />
            <button type="button" className="ghost" onClick={browse}>
              Browse…
            </button>
          </div>
        </label>

        <label className="field">
          <span>Parallel connections when sending</span>
          <select value={draft.streams} onChange={(e) => set("streams", Number(e.target.value))}>
            {[1, 2, 4, 8].map((n) => (
              <option key={n} value={n}>
                {n}
                {n === 4 ? " (recommended)" : ""}
              </option>
            ))}
          </select>
          <small className="muted">More connections can be faster on fast networks.</small>
        </label>

        <label className="field">
          <span>Visibility when Beam starts</span>
          <select value={draft.visibility} onChange={(e) => set("visibility", e.target.value as Settings["visibility"])}>
            <option value="everyone">Everyone (for 10 minutes)</option>
            <option value="trusted">Trusted devices only</option>
            <option value="off">Off</option>
          </select>
        </label>

        <label className="toggle field-toggle">
          <input type="checkbox" checked={draft.sendToShortcut} onChange={(e) => set("sendToShortcut", e.target.checked)} />
          <span>
            Add Beam to Explorer's <em>Send to</em> menu
          </span>
        </label>

        <label className="toggle field-toggle">
          <input
            type="checkbox"
            checked={draft.startWithWindows}
            onChange={(e) => set("startWithWindows", e.target.checked)}
          />
          <span>Start Beam when I sign in to Windows (in the tray)</span>
        </label>

        <p className="muted small">
          Auto-accept is set per device, on the <strong>Trusted devices</strong> page.
        </p>

        <div className="form-actions">
          <button type="submit" className="primary" disabled={!changed}>
            Save
          </button>
          {changed && (
            <button type="button" className="ghost" onClick={() => setDraft(snapshot.settings)}>
              Undo changes
            </button>
          )}
          {message && <span className={message.kind === "ok" ? "ok-text" : "error-text"}>{message.text}</span>}
        </div>
      </form>
    </div>
  );
}

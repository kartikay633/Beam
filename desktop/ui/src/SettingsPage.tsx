import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, isTauri, SAVE_FOLDERS, type Settings, type Snapshot } from "./api";
import { TrustedDevices } from "./Devices";

export function SettingsPage({ snapshot }: { snapshot: Snapshot }) {
  const [draft, setDraft] = useState<Settings>(snapshot.settings);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const persisted = JSON.stringify(snapshot.settings);
  useEffect(() => setDraft(JSON.parse(persisted) as Settings), [persisted]);

  const changed = JSON.stringify(draft) !== JSON.stringify(snapshot.settings);
  const restartRequired = draft.deviceName !== snapshot.settings.deviceName || draft.downloadDir !== snapshot.settings.downloadDir;
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => setDraft({ ...draft, [key]: value });

  const save = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.updateSettings(draft);
      setMessage({
        kind: "ok",
        text: restartRequired ? "Saved. Restart Beam to apply the name or download folder change." : "Saved",
      });
    } catch (e) {
      setMessage({ kind: "error", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const browse = async () => {
    if (!isTauri) return;
    try {
      const folder = await open({ directory: true, title: "Where should received files go?" });
      if (typeof folder === "string") set("downloadDir", folder);
    } catch (e) { setMessage({ kind: "error", text: String(e) }); }
  };

  return (
    <div className="page narrow">
      <form
        noValidate
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
              <span className="secondary">What other devices see · applies after restart</span>
              <input disabled={busy} value={draft.deviceName} onChange={(e) => set("deviceName", e.target.value)} maxLength={63} />
            </label>
            <label className="field">
              <span className="strong">Save received files to</span>
              <span className="secondary">Folder changes apply after restart</span>
              <div className="input-row">
                <input disabled={busy} value={draft.downloadDir} onChange={(e) => set("downloadDir", e.target.value)} />
                <button type="button" className="button" disabled={busy} onClick={browse}>
                  Browse
                </button>
              </div>
            </label>
            <fieldset className="field save-folders">
              <legend className="strong">Your trusted devices can also save to</legend>
              <span className="secondary">They pick the folder when sending. Other devices always use the folder above.</span>
              <div className="checks">
                {SAVE_FOLDERS.map((folder) => (
                  <label key={folder.id} className="check">
                    <input
                      type="checkbox"
                      disabled={busy}
                      checked={draft.saveFolders.includes(folder.id)}
                      onChange={(e) =>
                        set(
                          "saveFolders",
                          e.target.checked
                            ? SAVE_FOLDERS.map((f) => f.id).filter((id) => id === folder.id || draft.saveFolders.includes(id))
                            : draft.saveFolders.filter((id) => id !== folder.id),
                        )
                      }
                    />
                    {folder.label}
                  </label>
                ))}
              </div>
            </fieldset>
            <label className="field">
              <span className="strong">Who can see this computer when Beam starts</span>
              <select
                disabled={busy}
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
              <select disabled={busy} value={draft.streams} onChange={(e) => set("streams", Number(e.target.value))}>
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
                disabled={busy}
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
                disabled={busy}
                onChange={(e) => set("startWithWindows", e.target.checked)}
              />
            </label>
          </div>
          <div className="form-actions">
            <button type="submit" className="button primary" disabled={!changed || busy} aria-busy={busy}>
              Save changes
            </button>
            {changed && (
              <button type="button" className="text-button" disabled={busy} onClick={() => setDraft(snapshot.settings)}>
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
        <h2 id="trusted" className="section-title">Trusted devices</h2>
        <TrustedDevices devices={snapshot.trusted} />
      </section>

      <p className="footnote">
        {snapshot.me.name} · <span className="mono">{snapshot.me.deviceId}</span>
      </p>
    </div>
  );
}

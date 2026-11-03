import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri, type PairingInfo } from "./api";
import { CheckIcon } from "./icons";

export function PairPage() {
  const [info, setInfo] = useState<PairingInfo | null>(null);
  const [paired, setPaired] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    setInfo(null);
    setError(null);
    const refresh = () => api.pairing()
      .then((value) => { if (!disposed) setInfo(value); })
      .catch((e) => { if (!disposed) setError(String(e)); });
    void refresh();
    if (isTauri) {
      listen<string>("paired", (event) => {
        if (!disposed) { setPaired(event.payload); void refresh(); }
      }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; })
        .catch((e) => { if (!disposed) setError(String(e)); });
    }
    return () => { disposed = true; stop?.(); };
  }, [attempt]);

  return (
    <section className="pair-section" aria-label="Pair a phone">
      {paired && <p className="notice ok" role="status"><CheckIcon size={16} /> Paired with {paired}.</p>}
      {error && <div className="notice error" role="alert">
        <span>{error}</span>
        <button className="button" onClick={() => setAttempt((n) => n + 1)}>Try again</button>
      </div>}
      {!info && !error && <p role="status">Preparing a pairing code…</p>}
      {info && (
        <div className="pair-card">
          <div className="qr" dangerouslySetInnerHTML={{ __html: info.svg }} />
          <div className="grow pair-steps">
            <h1>Pair a phone</h1>
            <ol>
              <li>Open Beam on your phone.</li>
              <li>Tap <strong>Scan code</strong> at the top and point the camera at this code.</li>
              <li>Check the phone shows <span className="mono">{info.deviceId}</span>.</li>
            </ol>
            <p className="secondary">You only do this once. After that, your phone's files arrive here straight away. Keep Wi-Fi and Bluetooth on; they don't need to be connected to anything.</p>
            <div className="pair-actions">
              <button className="text-button" onClick={() => setAttempt((n) => n + 1)}>New code</button>
              <button className="text-button" onClick={async () => {
                try {
                  await navigator.clipboard.writeText(info.uri);
                  setCopied(true);
                } catch (e) { setError(String(e)); }
              }}>{copied ? "Copied" : "Copy pairing link"}</button>
            </div>
            <p className="caption">Works once, for {Math.round(info.validForSecs / 60)} minutes · {info.addresses.length ? info.addresses.join(", ") : "Pairs over Bluetooth, no shared Wi-Fi needed"}</p>
          </div>
        </div>
      )}
    </section>
  );
}

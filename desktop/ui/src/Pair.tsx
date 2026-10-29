import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri, type PairingInfo } from "./api";
import { CheckIcon } from "./icons";

export function PairPage() {
  const [info, setInfo] = useState<PairingInfo | null>(null);
  const [paired, setPaired] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // Each code works once, for 5 minutes: show a fresh one on open and
  // after every successful pairing.
  useEffect(() => {
    api.pairing().then(setInfo);
    if (!isTauri) return;
    let stop: (() => void) | undefined;
    listen<string>("paired", (event) => {
      setPaired(event.payload);
      api.pairing().then(setInfo);
    }).then((unlisten) => (stop = unlisten));
    return () => stop?.();
  }, []);
  return (
    <div className="page narrow">
      {paired && (
        <div className="notice ok" role="status">
          <CheckIcon size={16} />
          <span>Paired with {paired}. It can now send to you without asking.</span>
        </div>
      )}
      {info && (
        <section className="pair-card">
          {/* The SVG comes from our own backend (the qrcode crate). */}
          <div className="qr" dangerouslySetInnerHTML={{ __html: info.svg }} />
          <div className="grow pair-steps">
            <h1>Pair a phone</h1>
            <ol>
              <li>Open Beam on your phone.</li>
              <li>Tap the pair icon at the top, then scan this code.</li>
              <li>
                Check the phone shows <span className="mono">{info.deviceId}</span>.
              </li>
            </ol>
            <p className="secondary">
              Paired devices trust each other: they always find each other, even when set to trusted devices only.
            </p>
            <div className="pair-actions">
              <button className="text-button" onClick={() => api.pairing().then(setInfo)}>
                New code
              </button>
              <button
                className="text-button"
                onClick={() =>
                  navigator.clipboard.writeText(info.uri).then(() => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 2000);
                  })
                }
              >
                {copied ? "Copied" : "Copy pairing link"}
              </button>
            </div>
            <p className="secondary small">
              Works once, for {Math.round(info.validForSecs / 60)} minutes ·{" "}
              {info.addresses.length ? info.addresses.join(", ") : "not on a network"}
            </p>
          </div>
        </section>
      )}
    </div>
  );
}

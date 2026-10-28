import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri, type PairingInfo } from "./api";

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
    <div className="page">
      <header className="page-header">
        <h1>Pair a phone</h1>
        <p className="muted">
          Scan this code with Beam on your phone. Your phone learns this computer's key, so the two trust each other
          and can always find each other, even in "trusted only" mode.
        </p>
      </header>
      {paired && <div className="banner ok">Paired with {paired}. It can now send to you without asking.</div>}
      {info && (
        <div className="pair">
          {/* The SVG comes from our own backend (the qrcode crate). */}
          <div className="qr" dangerouslySetInnerHTML={{ __html: info.svg }} />
          <dl className="pair-details">
            <dt>Device id</dt>
            <dd className="mono">{info.deviceId}</dd>
            <dt>Addresses</dt>
            <dd className="mono">{info.addresses.length ? info.addresses.join(", ") : "not on a network"}</dd>
            <dt>Valid for</dt>
            <dd>
              {Math.round(info.validForSecs / 60)} minutes, one phone.{" "}
              <button className="link" onClick={() => api.pairing().then(setInfo)}>
                New code
              </button>
            </dd>
            <dt>No camera?</dt>
            <dd>
              {/* The phone can paste this instead of scanning. */}
              <button
                className="link"
                onClick={() =>
                  navigator.clipboard.writeText(info.uri).then(() => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 2000);
                  })
                }
              >
                {copied ? "Copied" : "Copy pairing link"}
              </button>
            </dd>
            <dt>Check</dt>
            <dd className="muted small">
              After scanning, your phone should show the same device id. If it doesn't, someone else's code was
              scanned.
            </dd>
          </dl>
        </div>
      )}
    </div>
  );
}

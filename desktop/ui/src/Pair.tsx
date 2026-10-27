import { useEffect, useState } from "react";
import { api, type PairingInfo } from "./api";

export function PairPage() {
  const [info, setInfo] = useState<PairingInfo | null>(null);
  useEffect(() => {
    api.pairing().then(setInfo);
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
      {info && (
        <div className="pair">
          {/* The SVG comes from our own backend (the qrcode crate). */}
          <div className="qr" dangerouslySetInnerHTML={{ __html: info.svg }} />
          <dl className="pair-details">
            <dt>Device id</dt>
            <dd className="mono">{info.deviceId}</dd>
            <dt>Addresses</dt>
            <dd className="mono">{info.addresses.length ? info.addresses.join(", ") : "not on a network"}</dd>
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

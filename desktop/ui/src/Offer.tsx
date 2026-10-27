import { useEffect, useState } from "react";
import { api, type Offer } from "./api";
import { useCountdown } from "./App";
import { formatBytes, formatCode } from "./format";
import { FileIcon } from "./icons";

/** The always-on-top popup for an incoming offer. */
export function OfferWindow({ id }: { id: string }) {
  const [offer, setOffer] = useState<Offer | null | undefined>(undefined);
  const [trust, setTrust] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remaining = useCountdown(offer?.expiresMs ?? null);

  useEffect(() => {
    api.offer(id).then(setOffer);
  }, [id]);

  if (offer === undefined) return <div className="loading">Loading…</div>;
  if (offer === null) return <div className="loading">This request has expired.</div>;

  const answer = async (accept: boolean) => {
    setBusy(true);
    try {
      // The backend closes this window once it has the answer.
      await api.respond(id, accept, accept && trust);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const total = 60;
  const left = remaining ?? total;
  return (
    <div className="offer">
      <div className="offer-header">
        <div className="offer-title">Incoming files</div>
        <div className="offer-from">
          <strong>{offer.fromName}</strong> wants to send {offer.files.length} file
          {offer.files.length === 1 ? "" : "s"} ({formatBytes(offer.totalBytes)})
        </div>
        <div className="muted small mono">{offer.fromId}</div>
      </div>

      <div className={`code-box ${offer.trusted ? "trusted" : ""}`}>
        <div className="small muted">{offer.trusted ? "Trusted device" : "Verification code"}</div>
        <div className="code mono">{formatCode(offer.verificationCode)}</div>
        {!offer.trusted && <div className="small muted">Check that the sender sees the same code.</div>}
      </div>

      <ul className="offer-files">
        {offer.files.map((file, i) => (
          <li key={i}>
            <FileIcon size={16} />
            <span className="grow ellipsis" title={file.name}>
              {file.name}
            </span>
            <span className="muted small">{formatBytes(file.size)}</span>
          </li>
        ))}
      </ul>

      {!offer.trusted && (
        <label className="toggle">
          <input type="checkbox" checked={trust} onChange={(e) => setTrust(e.target.checked)} />
          <span>Trust this device and accept its files automatically</span>
        </label>
      )}

      {error && <div className="banner error">{error}</div>}

      <div className="offer-actions">
        <button className="ghost" disabled={busy} onClick={() => answer(false)}>
          Decline
        </button>
        <button className="primary" disabled={busy} onClick={() => answer(true)} autoFocus>
          Accept
        </button>
      </div>
      <div className="timeout" aria-label={`${Math.ceil(left)} seconds left`}>
        <div className="timeout-fill" style={{ width: `${(left / total) * 100}%` }} />
      </div>
    </div>
  );
}

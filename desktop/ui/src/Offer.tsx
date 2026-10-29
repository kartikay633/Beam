import { useEffect, useState } from "react";
import { api, type Offer } from "./api";
import { useCountdown } from "./App";
import { formatBytes, formatCode } from "./format";
import { ShieldIcon } from "./icons";

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

  const count = offer.files.length;
  const total = 60;
  const left = remaining ?? total;
  return (
    <div className="offer">
      <div className="offer-body">
        <div>
          <h1>
            {offer.fromName} wants to send {count === 1 ? "a file" : `${count} files`}
          </h1>
          <p className="secondary">{formatBytes(offer.totalBytes)} in total</p>
        </div>

        <ul className="offer-files">
          {offer.files.map((file, i) => (
            <li key={i}>
              <span className="ellipsis" title={file.name}>
                {file.name}
              </span>
              <span className="secondary numeric">{formatBytes(file.size)}</span>
            </li>
          ))}
        </ul>

        <div className="code-box">
          <div className="grow">
            <p className="secondary">{offer.trusted ? "Trusted device · code" : "Check this code matches the sender"}</p>
            <p className="code">{formatCode(offer.verificationCode)}</p>
          </div>
          <ShieldIcon size={22} />
        </div>

        {!offer.trusted && (
          <label className="check">
            <input type="checkbox" checked={trust} onChange={(e) => setTrust(e.target.checked)} />
            <span>Always accept from {offer.fromName}</span>
          </label>
        )}

        {error && <div className="notice error">{error}</div>}
      </div>

      <div className="offer-actions">
        <button className="button large" disabled={busy} onClick={() => answer(false)}>
          Decline
        </button>
        <button className="button primary large" disabled={busy} onClick={() => answer(true)} autoFocus>
          Accept
        </button>
      </div>
      <div className="timeout" aria-label={`${Math.ceil(left)} seconds left to answer`}>
        <div className="timeout-fill" style={{ width: `${(left / total) * 100}%` }} />
      </div>
    </div>
  );
}

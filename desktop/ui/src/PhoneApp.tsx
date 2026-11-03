import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { CloseIcon, PhoneIcon } from "./icons";

/**
 * "Get Beam on your phone": a button that opens a panel with a QR code of
 * the Android download, the link itself and a note that Google Play is
 * coming. It's a separate panel so it can't be mistaken for the pairing
 * code on Your devices.
 */
export function GetPhoneAppButton({ label = "Get Beam on your phone" }: { label?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button className="button" onClick={() => setOpen(true)}>
        <PhoneIcon size={16} /> {label}
      </button>
      {open && <PhoneAppPanel onClose={() => setOpen(false)} />}
    </>
  );
}

function PhoneAppPanel({ onClose }: { onClose: () => void }) {
  const [app, setApp] = useState<{ url: string; svg: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const closeButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    api.phoneApp().then(setApp, (e) => setError(String(e)));
    closeButton.current?.focus();
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const copy = async () => {
    if (!app) return;
    try {
      await navigator.clipboard.writeText(app.url);
      setCopied(true);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="scrim" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="phone-app" role="dialog" aria-modal="true" aria-labelledby="phone-app-title">
        <header>
          <h2 id="phone-app-title">Get Beam on your phone</h2>
          <button ref={closeButton} className="icon-button" aria-label="Close" onClick={onClose}>
            <CloseIcon size={14} />
          </button>
        </header>
        <div className="phone-app-body">
          {app ? <div className="qr" aria-label="QR code of the download link" dangerouslySetInnerHTML={{ __html: app.svg }} /> : <div className="qr placeholder" />}
          <div className="grow">
            <ol>
              <li>Point your Android phone's camera at this code.</li>
              <li>Open the link and download <strong>Beam</strong>.</li>
              <li>Tap <strong>Install</strong>. If your phone asks, allow installing apps from your browser.</li>
            </ol>
            <p className="secondary small">Or open this link on the phone:</p>
            <p className="mono link-line">{app?.url ?? "…"}</p>
            <div className="phone-app-actions">
              <button className="button" disabled={!app} onClick={copy}>{copied ? "Copied" : "Copy link"}</button>
            </div>
            <p className="soon"><span className="soon-badge">Google Play</span> Coming soon</p>
            <p className="secondary small">Needs Android 10 or later.</p>
            {error && <p className="error-text" role="alert">{error}</p>}
          </div>
        </div>
      </div>
    </div>
  );
}

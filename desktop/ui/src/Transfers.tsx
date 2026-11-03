import { api, type Transfer } from "./api";
import { formatAgo, formatBytes, formatCode, formatDuration, formatSpeed } from "./format";
import { FileGlyph } from "./icons";
import { useAction } from "./useAction";

const FINISHED = new Set(["done", "declined", "failed", "cancelled"]);

export function isFinished(t: Transfer): boolean {
  return FINISHED.has(t.state);
}

/** Transfers as a Windows list: file glyph, name, progress, what's next. */
export function TransferList({ transfers }: { transfers: Transfer[] }) {
  return (
    <ul className="card list">
      {transfers.map((t) => (
        <TransferRow key={t.id} transfer={t} />
      ))}
    </ul>
  );
}

function TransferRow({ transfer: t }: { transfer: Transfer }) {
  const action = useAction();
  const finished = isFinished(t);
  const fraction = t.totalBytes > 0 ? t.doneBytes / t.totalBytes : t.state === "done" ? 1 : 0;
  const files =
    t.fileCount === 1
      ? t.fileNames[0]
      : `${t.fileNames.slice(0, 2).join(", ")}${t.fileCount > 2 ? ` and ${t.fileCount - 2} more` : ""}`;
  const peer = `${t.direction === "send" ? "To" : "From"} ${t.peerName}`;

  let status: string;
  let right: string | null = null;
  if (t.state === "active") {
    status = peer;
    if (t.speedBps > 0) status += ` · ${formatSpeed(t.speedBps)}`;
    right = `${formatBytes(t.doneBytes)} of ${formatBytes(t.totalBytes)}`;
    if (t.etaSecs != null) right += ` · ${formatDuration(t.etaSecs)} left`;
  } else if (t.state === "done") {
    status = `${peer} · ${formatBytes(t.totalBytes)} · ${formatAgo(t.startedMs)}`;
  } else if (t.state === "waiting") {
    status = t.verificationCode
      ? `${peer} · waiting for them to accept · code ${formatCode(t.verificationCode)}`
      : (t.detail ?? `${peer} · connecting…`);
  } else if (t.state === "reconnecting") {
    status = t.detail ?? `${peer} · reconnecting…`;
  } else if (t.state === "declined") {
    const reason = t.detail && t.detail.toLowerCase() !== "declined" ? `: ${t.detail}` : "";
    status = `${peer} · declined${reason}`;
  } else if (t.state === "cancelled") {
    status = `${peer} · cancelled`;
  } else {
    status = t.detail ?? "Couldn't send";
  }

  return (
    <li className={`row transfer ${t.state}`}>
      <FileGlyph name={t.fileNames[0] ?? ""} />
      <div className="grow">
        <div className="row-title">
          <span className="ellipsis strong" title={t.fileNames.join("\n")}>
            {files}
          </span>
          {right && <span className="secondary numeric">{right}</span>}
        </div>
        {!finished && (
          <div
            className={`progress ${t.state === "active" ? "" : "indeterminate"}`}
            role="progressbar"
            aria-label={t.direction === "send" ? "Sending" : "Receiving"}
            aria-valuenow={t.state === "active" ? Math.round(Math.max(0, Math.min(1, fraction)) * 100) : undefined}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <div className="progress-fill" style={{ width: `${(fraction * 100).toFixed(1)}%` }} />
          </div>
        )}
        <div className={`caption ${t.state === "failed" ? "critical" : ""}`}>{status}</div>
        {action.error && <p className="error-text" role="alert">{action.error}</p>}
      </div>
      {t.direction === "receive" && t.state === "done" && (
        <button className="button" disabled={action.busy} onClick={() => action.run(() => api.openFolder(t.folder ?? undefined))}>
          Show in folder
        </button>
      )}
      {!finished && (
        <button className="button" disabled={action.busy} onClick={() => action.run(() => api.cancel(t.id))}>
          Cancel
        </button>
      )}
    </li>
  );
}

import { api, type Transfer } from "./api";
import { formatAgo, formatBytes, formatCode, formatDuration, formatSpeed } from "./format";
import { ArrowDownIcon, ArrowUpIcon, CloseIcon, FolderIcon } from "./icons";

const STATE_LABEL: Record<Transfer["state"], string> = {
  waiting: "Waiting",
  active: "In progress",
  reconnecting: "Reconnecting",
  done: "Done",
  declined: "Declined",
  failed: "Failed",
  cancelled: "Cancelled",
};

const FINISHED = new Set(["done", "declined", "failed", "cancelled"]);

export function Transfers({ transfers }: { transfers: Transfer[] }) {
  const anyFinished = transfers.some((t) => FINISHED.has(t.state));
  return (
    <section>
      <div className="section-header">
        <h2>Transfers</h2>
        {anyFinished && (
          <button className="ghost small" onClick={() => api.clearFinished()}>
            Clear finished
          </button>
        )}
      </div>
      {transfers.length === 0 ? (
        <div className="empty muted">Nothing sent or received yet.</div>
      ) : (
        <ul className="transfer-list">
          {transfers.map((t) => (
            <TransferRow key={t.id} transfer={t} />
          ))}
        </ul>
      )}
    </section>
  );
}

function TransferRow({ transfer: t }: { transfer: Transfer }) {
  const finished = FINISHED.has(t.state);
  const fraction = t.totalBytes > 0 ? t.doneBytes / t.totalBytes : t.state === "done" ? 1 : 0;
  const files =
    t.fileCount === 1
      ? t.fileNames[0]
      : `${t.fileNames.slice(0, 2).join(", ")}${t.fileCount > 2 ? ` and ${t.fileCount - 2} more` : ""}`;

  let status: string;
  if (t.state === "active") {
    status = `${formatBytes(t.doneBytes)} of ${formatBytes(t.totalBytes)}`;
    if (t.speedBps > 0) status += ` · ${formatSpeed(t.speedBps)}`;
    if (t.etaSecs != null) status += ` · ${formatDuration(t.etaSecs)} left`;
  } else if (t.state === "done") {
    status = `${formatBytes(t.totalBytes)} · ${formatAgo(t.startedMs)}`;
  } else {
    status = t.detail ?? STATE_LABEL[t.state];
  }

  return (
    <li className={`transfer ${t.state}`}>
      <div className={`direction ${t.direction}`} title={t.direction === "send" ? "Sent" : "Received"}>
        {t.direction === "send" ? <ArrowUpIcon size={18} /> : <ArrowDownIcon size={18} />}
      </div>
      <div className="transfer-main">
        <div className="transfer-title">
          <span className="transfer-files" title={t.fileNames.join("\n")}>
            {files}
          </span>
          <span className={`state-pill ${t.state}`}>{STATE_LABEL[t.state]}</span>
        </div>
        <div className="muted small">
          {t.direction === "send" ? "to" : "from"} {t.peerName}
          {t.verificationCode && t.state === "waiting" && (
            <>
              {" "}
              · code <span className="mono">{formatCode(t.verificationCode)}</span>
            </>
          )}
        </div>
        {!finished && (
          <div className="progress" role="progressbar" aria-valuenow={Math.round(fraction * 100)} aria-valuemin={0} aria-valuemax={100}>
            <div className="progress-fill" style={{ width: `${(fraction * 100).toFixed(1)}%` }} />
          </div>
        )}
        <div className={`small ${t.state === "failed" ? "error-text" : "muted"}`}>{status}</div>
      </div>
      <div className="row-actions">
        {t.direction === "receive" && t.state === "done" && (
          <button className="icon-button" title="Show in folder" onClick={() => api.openFolder(t.folder ?? undefined)}>
            <FolderIcon size={18} />
          </button>
        )}
        {!finished && (
          <button className="icon-button" title="Cancel" onClick={() => api.cancel(t.id)}>
            <CloseIcon size={18} />
          </button>
        )}
      </div>
    </li>
  );
}

import { api, type Transfer } from "./api";
import { formatAgo, formatBytes, formatCode, formatDuration, formatSpeed } from "./format";
import { AlertIcon, ArrowDownIcon, ArrowUpIcon, CheckIcon, CloseIcon } from "./icons";

const FINISHED = new Set(["done", "declined", "failed", "cancelled"]);

export function Transfers({ transfers }: { transfers: Transfer[] }) {
  const anyFinished = transfers.some((t) => FINISHED.has(t.state));
  return (
    <section aria-labelledby="transfers">
      <div className="section-header">
        <h2 id="transfers">Transfers</h2>
        <span className="grow" />
        {anyFinished && (
          <button className="text-button subtle" onClick={() => api.clearFinished()}>
            Clear finished
          </button>
        )}
      </div>
      {transfers.length === 0 ? (
        <p className="secondary empty-line">Files you send and receive show up here.</p>
      ) : (
        <ul className="list">
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
  const peer = `${t.direction === "send" ? "To" : "From"} ${t.peerName}`;

  let status: string;
  if (t.state === "active") {
    status = peer;
    if (t.speedBps > 0) status += ` · ${formatSpeed(t.speedBps)}`;
    if (t.etaSecs != null) status += ` · ${formatDuration(t.etaSecs)} left`;
  } else if (t.state === "done") {
    status = `${t.direction === "send" ? "Sent to" : "From"} ${t.peerName} · ${formatBytes(t.totalBytes)} · ${formatAgo(t.startedMs)}`;
  } else if (t.state === "waiting" && t.verificationCode) {
    status = `${peer} · waiting for them to accept · code ${formatCode(t.verificationCode)}`;
  } else if (t.state === "reconnecting") {
    status = t.detail ?? `${peer} · reconnecting…`;
  } else if (t.state === "waiting") {
    status = t.detail ?? `${peer} · connecting…`;
  } else if (t.state === "declined") {
    const reason = t.detail && t.detail.toLowerCase() !== "declined" ? `: ${t.detail}` : "";
    status = `${peer} · declined${reason}`;
  } else if (t.state === "cancelled") {
    status = `${peer} · cancelled`;
  } else {
    status = t.detail ?? "Failed";
  }

  const badge =
    t.state === "done" ? (
      <div className="badge ok">
        <CheckIcon size={16} />
      </div>
    ) : t.state === "failed" ? (
      <div className="badge bad">
        <AlertIcon size={16} />
      </div>
    ) : (
      <div className="badge" title={t.direction === "send" ? "Sending" : "Receiving"}>
        {t.direction === "send" ? <ArrowUpIcon size={16} /> : <ArrowDownIcon size={16} />}
      </div>
    );

  return (
    <li className={`row transfer ${t.state}`}>
      {badge}
      <div className="grow">
        <div className="title-line">
          <p className="strong ellipsis" title={t.fileNames.join("\n")}>
            {files}
          </p>
          {t.state === "active" && (
            <span className="secondary numeric">
              {formatBytes(t.doneBytes)} of {formatBytes(t.totalBytes)}
            </span>
          )}
        </div>
        {!finished && (
          <div
            className={`progress ${t.state === "active" ? "" : "pending"}`}
            role="progressbar"
            aria-label={t.direction === "send" ? "Sent" : "Received"}
            aria-valuenow={Math.round(fraction * 100)}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <div className="progress-fill" style={{ width: `${(fraction * 100).toFixed(1)}%` }} />
          </div>
        )}
        <p className={`numeric ${t.state === "failed" ? "error-text" : "secondary"}`}>{status}</p>
      </div>
      {t.direction === "receive" && t.state === "done" && (
        <button className="text-button" onClick={() => api.openFolder(t.folder ?? undefined)}>
          Show in folder
        </button>
      )}
      {!finished && (
        <button className="icon-button" aria-label="Cancel" title="Cancel" onClick={() => api.cancel(t.id)}>
          <CloseIcon size={16} />
        </button>
      )}
    </li>
  );
}

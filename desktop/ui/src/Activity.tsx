import { api, type Snapshot } from "./api";
import { TransferList, isFinished } from "./Transfers";
import { FolderIcon } from "./icons";
import { useAction } from "./useAction";
import { InfoBar } from "./Share";

export function ActivityPage({ snapshot }: { snapshot: Snapshot }) {
  const action = useAction();
  const transfers = snapshot.transfers;
  return (
    <div className="page">
      <header className="page-header">
        <h1>Activity</h1>
        <button className="button" disabled={action.busy} onClick={() => action.run(() => api.openFolder())}>
          <FolderIcon size={16} /> Open received files
        </button>
        {transfers.some(isFinished) && (
          <button className="button" disabled={action.busy} onClick={() => action.run(() => api.clearFinished())}>
            Clear finished
          </button>
        )}
      </header>
      {action.error && <InfoBar kind="error" onClose={action.clearError}>{action.error}</InfoBar>}
      {transfers.length === 0 ? (
        <div className="empty-state">
          <p className="strong">Nothing sent or received yet</p>
          <p className="secondary">Files you send and receive show up here.</p>
        </div>
      ) : (
        <TransferList transfers={transfers} />
      )}
      <p className="caption">Received files go to {snapshot.receiveDir}</p>
    </div>
  );
}

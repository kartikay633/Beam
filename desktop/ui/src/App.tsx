import { useEffect, useState, type ReactNode } from "react";
import { api, onSnapshot, type Snapshot } from "./api";
import { ActivityPage } from "./Activity";
import { DevicesPage } from "./Devices";
import { SettingsPage } from "./SettingsPage";
import { SharePage } from "./Share";
import { BeamLogo, ClockIcon, GearIcon, PhoneIcon, ShareIcon } from "./icons";

export type Page = "share" | "activity" | "devices" | "settings";

const MAIN: { id: Page; label: string; icon: ReactNode }[] = [
  { id: "share", label: "Share", icon: <ShareIcon size={18} /> },
  { id: "activity", label: "Activity", icon: <ClockIcon size={18} /> },
  { id: "devices", label: "Your devices", icon: <PhoneIcon size={18} /> },
];

/** The latest snapshot from the backend, kept up to date. */
export function useSnapshot() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let disposed = false;
    let receivedEvent = false;
    let stop: (() => void) | undefined;
    setError(null);
    const failed = (e: unknown) => { if (!disposed) setError(String(e)); };
    const connect = async () => {
      // Subscribe before fetching: otherwise a change between the snapshot
      // read and asynchronous listener registration can be lost indefinitely.
      const unlisten = await onSnapshot((value) => {
        receivedEvent = true;
        if (!disposed) { setSnapshot(value); setError(null); }
      });
      if (disposed) { unlisten(); return; }
      stop = unlisten;
      const value = await api.snapshot();
      if (!disposed && !receivedEvent) setSnapshot(value);
    };
    void connect().catch(failed);
    return () => { disposed = true; stop?.(); };
  }, [attempt]);
  return { snapshot, error, retry: () => setAttempt((n) => n + 1) };
}

export function App() {
  const { snapshot, error, retry } = useSnapshot();
  const [page, setPage] = useState<Page>("share");

  if (!snapshot) {
    return <div className="loading">{error ? <div role="alert"><p>{error}</p><button className="button" onClick={retry}>Try again</button></div> : "Starting Beam…"}</div>;
  }
  const active = snapshot.transfers.filter((t) => ["active", "waiting", "reconnecting"].includes(t.state)).length;
  return (
    <div className="app">
      <nav className="nav" aria-label="Beam">
        <div className="nav-brand">
          <BeamLogo size={18} />
          <span>Beam</span>
        </div>
        {MAIN.map((item) => (
          <NavItem key={item.id} item={item} current={page === item.id} onSelect={setPage}>
            {item.id === "activity" && active > 0 && <span className="nav-badge">{active}</span>}
          </NavItem>
        ))}
        <span className="grow" />
        <NavItem
          item={{ id: "settings", label: "Settings", icon: <GearIcon size={18} /> }}
          current={page === "settings"}
          onSelect={setPage}
        />
      </nav>
      <main className="layer">
        {error && <div className="notice error" role="alert"><span>{error}</span><button className="button" onClick={retry}>Reconnect</button></div>}
        {page === "share" && <SharePage snapshot={snapshot} onNavigate={setPage} />}
        {page === "activity" && <ActivityPage snapshot={snapshot} />}
        {page === "devices" && <DevicesPage snapshot={snapshot} />}
        {page === "settings" && <SettingsPage snapshot={snapshot} />}
      </main>
    </div>
  );
}

function NavItem({
  item,
  current,
  onSelect,
  children,
}: {
  item: { id: Page; label: string; icon: ReactNode };
  current: boolean;
  onSelect: (page: Page) => void;
  children?: ReactNode;
}) {
  return (
    <button className={`nav-item ${current ? "current" : ""}`} aria-current={current ? "page" : undefined} onClick={() => onSelect(item.id)}>
      {item.icon}
      <span className="grow">{item.label}</span>
      {children}
    </button>
  );
}

/** Seconds left until `untilMs`, updated every second. */
export function useCountdown(untilMs: number | null): number | null {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!untilMs) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [untilMs]);
  return untilMs ? Math.max(0, (untilMs - now) / 1000) : null;
}

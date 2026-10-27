import { useEffect, useState, type ReactNode } from "react";
import { api, isTauri, onSnapshot, type Snapshot, type VisibilityMode } from "./api";
import { Home } from "./Home";
import { PairPage } from "./Pair";
import { SettingsPage } from "./SettingsPage";
import { TrustedPage } from "./Trusted";
import { formatDuration } from "./format";
import { GearIcon, HomeIcon, QrIcon, ShieldIcon } from "./icons";

type Page = "home" | "trusted" | "pair" | "settings";

const PAGES: { id: Page; label: string; icon: ReactNode }[] = [
  { id: "home", label: "Send & receive", icon: <HomeIcon /> },
  { id: "trusted", label: "Trusted devices", icon: <ShieldIcon /> },
  { id: "pair", label: "Pair a phone", icon: <QrIcon /> },
  { id: "settings", label: "Settings", icon: <GearIcon /> },
];

/** The latest snapshot from the backend, kept up to date. */
export function useSnapshot(): Snapshot | null {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  useEffect(() => {
    let stop: (() => void) | undefined;
    onSnapshot(setSnapshot).then((unlisten) => (stop = unlisten));
    api.snapshot().then(setSnapshot);
    return () => stop?.();
  }, []);
  return snapshot;
}

export function App() {
  const snapshot = useSnapshot();
  const [page, setPage] = useState<Page>("home");

  if (!snapshot) {
    return <div className="loading">Starting Beam…</div>;
  }
  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <img src="/logo.png" alt="" width={28} height={28} />
          <span>Beam</span>
          {!isTauri && <span className="demo-badge">demo</span>}
        </div>
        <nav>
          {PAGES.map((p) => (
            <button
              key={p.id}
              className={`nav-item ${page === p.id ? "active" : ""}`}
              onClick={() => setPage(p.id)}
            >
              {p.icon}
              {p.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          <VisibilitySwitch snapshot={snapshot} />
          <div className="me">
            <div className="me-name">{snapshot.me.name}</div>
            <div className="me-id mono">{snapshot.me.deviceId}</div>
          </div>
        </div>
      </aside>
      <main className="content">
        {page === "home" && <Home snapshot={snapshot} />}
        {page === "trusted" && <TrustedPage snapshot={snapshot} />}
        {page === "pair" && <PairPage />}
        {page === "settings" && <SettingsPage snapshot={snapshot} />}
      </main>
    </div>
  );
}

const MODES: { id: VisibilityMode; label: string }[] = [
  { id: "everyone", label: "Everyone" },
  { id: "trusted", label: "Trusted" },
  { id: "off", label: "Off" },
];

function VisibilitySwitch({ snapshot }: { snapshot: Snapshot }) {
  const { mode, everyoneUntilMs } = snapshot.visibility;
  const remaining = useCountdown(everyoneUntilMs);
  const hint =
    mode === "everyone"
      ? `Anyone nearby can find you${remaining ? ` · ${formatDuration(remaining)} left` : ""}`
      : mode === "trusted"
        ? "Only your trusted devices can find you"
        : "Nobody can find or send to you";
  return (
    <div className="visibility">
      <div className="label">Who can see me</div>
      <div className="segmented" role="radiogroup" aria-label="Visibility">
        {MODES.map((m) => (
          <button
            key={m.id}
            role="radio"
            aria-checked={mode === m.id}
            className={mode === m.id ? `on ${m.id}` : ""}
            onClick={() => api.setVisibility(m.id)}
          >
            {m.label}
          </button>
        ))}
      </div>
      <div className="hint">{hint}</div>
    </div>
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

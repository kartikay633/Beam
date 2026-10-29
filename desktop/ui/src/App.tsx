import { useEffect, useRef, useState } from "react";
import { api, onSnapshot, type Snapshot, type VisibilityMode } from "./api";
import { Home } from "./Home";
import { PairPage } from "./Pair";
import { SettingsPage } from "./SettingsPage";
import { formatMinutesLeft } from "./format";
import { BeamLogo, CheckIcon, ChevronDownIcon, ChevronLeftIcon, QrIcon, SlidersIcon } from "./icons";

type Page = "home" | "pair" | "settings";

const TITLES: Record<Page, string> = { home: "Beam", pair: "Pair a phone", settings: "Settings" };

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
      <header className="topbar">
        {page === "home" ? (
          <div className="brand">
            <BeamLogo size={26} />
            <span>Beam</span>
          </div>
        ) : (
          <button className="back" onClick={() => setPage("home")}>
            <ChevronLeftIcon size={18} />
            <span>{TITLES[page]}</span>
          </button>
        )}
        <span className="grow" />
        <VisibilityChip snapshot={snapshot} />
        <button
          className={`icon-button ${page === "pair" ? "current" : ""}`}
          aria-label="Pair a phone"
          title="Pair a phone"
          onClick={() => setPage(page === "pair" ? "home" : "pair")}
        >
          <QrIcon size={19} />
        </button>
        <button
          className={`icon-button ${page === "settings" ? "current" : ""}`}
          aria-label="Settings"
          title="Settings"
          onClick={() => setPage(page === "settings" ? "home" : "settings")}
        >
          <SlidersIcon size={19} />
        </button>
      </header>
      <main className="content">
        {page === "home" && <Home snapshot={snapshot} />}
        {page === "pair" && <PairPage />}
        {page === "settings" && <SettingsPage snapshot={snapshot} />}
      </main>
    </div>
  );
}

const MODES: { id: VisibilityMode; label: string; hint: string }[] = [
  { id: "everyone", label: "Everyone", hint: "Anyone nearby can find you, for 10 minutes" },
  { id: "trusted", label: "Trusted devices", hint: "Only devices you trust can find you" },
  { id: "off", label: "Hidden", hint: "Nobody can find or send to you" },
];

/** "Visible to everyone · 9 min": opens a menu to change who can see you. */
function VisibilityChip({ snapshot }: { snapshot: Snapshot }) {
  const { mode, everyoneUntilMs } = snapshot.visibility;
  const remaining = useCountdown(everyoneUntilMs);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [open]);

  const label =
    mode === "everyone"
      ? `Visible to everyone${remaining ? ` · ${formatMinutesLeft(remaining)}` : ""}`
      : mode === "trusted"
        ? "Visible to trusted devices"
        : "Hidden";
  return (
    <div className="chip-menu" ref={ref}>
      <button className="chip" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className={`dot ${mode}`} />
        {label}
        <ChevronDownIcon size={14} />
      </button>
      {open && (
        <div className="menu" role="menu">
          {MODES.map((m) => (
            <button
              key={m.id}
              role="menuitemradio"
              aria-checked={mode === m.id}
              onClick={() => {
                api.setVisibility(m.id);
                setOpen(false);
              }}
            >
              <span className={`dot ${m.id}`} />
              <span className="menu-text">
                <span>{m.label}</span>
                <small>{m.hint}</small>
              </span>
              {mode === m.id && <CheckIcon size={16} />}
            </button>
          ))}
        </div>
      )}
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

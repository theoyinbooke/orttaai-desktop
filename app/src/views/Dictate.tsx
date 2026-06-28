import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Button,
  Card,
  ConfirmDialog,
  CopyButton,
  EmptyState,
  Icon,
  Kbd,
  LevelMeter,
  PageHeader,
  useToast,
} from "../ui";
import {
  STATE_LABEL,
  type DashboardStats,
  type EngineState,
  type HistoryItem,
  type ModelInfo,
  type Settings,
  type Tab,
} from "../types";

function ago(unix: number): string {
  const d = Date.now() / 1000 - unix;
  if (d < 60) return "now";
  if (d < 3600) return `${Math.floor(d / 60)}m`;
  if (d < 86400) return `${Math.floor(d / 3600)}h`;
  return `${Math.floor(d / 86400)}d`;
}

const SHORTCUTS: { tab: Tab; label: string; icon: string; hint: string }[] = [
  { tab: "models", label: "Models", icon: "models", hint: "Switch engine" },
  { tab: "assistant", label: "Assistant", icon: "assistant", hint: "Local AI chat" },
  { tab: "dictionary", label: "Dictionary", icon: "dictionary", hint: "Fixes & snippets" },
  { tab: "insights", label: "Insights", icon: "insights", hint: "Activity & trends" },
];

export default function Dictate(props: {
  engine: EngineState;
  settings: Settings | null;
  activeModel: ModelInfo | undefined;
  canStart: boolean;
  level: number;
  lastTranscript: string;
  historyVersion: number;
  onStart: () => void;
  onStop: () => void;
  onToggleRecord: () => void;
  onNavigate: (tab: Tab) => void;
}) {
  const { engine, settings, activeModel, canStart, level, onNavigate } = props;
  const running = engine !== "off";
  const recording = engine === "recording";
  const combo = settings?.push_to_talk ?? "Ctrl+Shift+Space";

  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [recent, setRecent] = useState<HistoryItem[]>([]);
  const [pendingDelete, setPendingDelete] = useState<HistoryItem | null>(null);
  const toast = useToast();

  useEffect(() => {
    invoke<DashboardStats>("dashboard_stats").then(setStats).catch(() => {});
    invoke<HistoryItem[]>("recent_history", { limit: 10 }).then(setRecent).catch(() => {});
  }, [props.historyVersion]);

  async function confirmDelete() {
    const item = pendingDelete;
    setPendingDelete(null);
    if (!item) return;
    try {
      await invoke("delete_transcription", { id: item.id });
    } catch (e) {
      toast(String(e), "error");
    }
  }

  const subtitle: Record<EngineState, string> = {
    off: "Engine stopped. Start the engine to begin dictating.",
    idle: "Ready. Press your shortcut anywhere and speak.",
    recording: "Listening — speak now, then press your shortcut again.",
    processing: "Transcribing your speech on-device…",
  };

  const action = !running ? (
    canStart ? (
      <Button variant="primary" icon="play" onClick={props.onStart}>
        Start engine
      </Button>
    ) : (
      <Button variant="primary" icon="download" onClick={() => onNavigate("models")}>
        Get a model
      </Button>
    )
  ) : (
    <Button variant="ghost" icon="stop" onClick={props.onStop}>
      Stop engine
    </Button>
  );

  const topApps = (stats?.top_apps ?? []).slice(0, 3);
  const maxApp = Math.max(1, ...topApps.map((a) => a.count));

  return (
    <div className="view dictate stagger">
      <PageHeader
        title="Dictate"
        desc="Press your shortcut, speak, and your words are typed into whatever app is focused."
        actions={action}
      />

      <Card className={`hero signal-${engine}`}>
        <div className="hero-signal">
          <span className="orb" aria-hidden="true">
            <span className="orb-core" />
          </span>
          <div className="hero-state">
            <div className="hero-label">{STATE_LABEL[engine]}</div>
            <p className="hero-sub">{subtitle[engine]}</p>
          </div>
          {running && (
            <Button
              className="hero-rec"
              variant={recording ? "danger" : "primary"}
              icon={recording ? "stop" : "play"}
              onClick={props.onToggleRecord}
            >
              {recording ? "Stop & insert" : "Click to record"}
            </Button>
          )}
        </div>

        <div className="hero-meter">
          <LevelMeter level={running ? level : 0} active={recording} />
        </div>

        <div className="hero-controls">
          <div className="shortcut-block">
            <Kbd combo={combo} />
            <span className="shortcut-note">
              Press to start, press again to insert.{" "}
              <span className="muted">Hold-to-talk on Windows/X11; a toggle on Wayland.</span>
            </span>
          </div>
          <button className="model-chip" onClick={() => onNavigate("models")}>
            <Icon name="models" size={15} />
            <span className="muted">Model</span>
            <span>{activeModel ? activeModel.name : settings?.model_id ?? "—"}</span>
            {activeModel && !activeModel.downloaded && <Badge tone="warn">not downloaded</Badge>}
            <Icon name="chevronRight" size={14} className="chip-caret" />
          </button>
        </div>
      </Card>

      <div className="dictate-stats">
        <MiniStat label="Words dictated" value={stats ? stats.total_words.toLocaleString() : "—"} />
        <MiniStat label="Words / minute" value={stats && stats.avg_wpm > 0 ? Math.round(stats.avg_wpm) : "—"} />
        <MiniStat label="Dictations" value={stats ? stats.total : "—"} />
      </div>

      <div className="dash-grid">
        <Card className="apps-card">
          <span className="overline">Top apps</span>
          {topApps.length === 0 ? (
            <p className="muted small">No data yet.</p>
          ) : (
            <ul className="hbar-list">
              {topApps.map((a) => (
                <li key={a.app} className="hbar-row">
                  <span className="hbar-label" title={a.app}>
                    {a.app}
                  </span>
                  <span className="hbar-track">
                    <span className="hbar-fill" style={{ width: `${(a.count / maxApp) * 100}%` }} />
                  </span>
                  <span className="hbar-val mono">{a.count}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card className="shortcuts-card">
          <span className="overline">Shortcuts</span>
          <div className="shortcut-grid">
            {SHORTCUTS.map((s) => (
              <button key={s.tab} className="shortcut-tile" onClick={() => onNavigate(s.tab)}>
                <span className="shortcut-ico">
                  <Icon name={s.icon} size={18} />
                </span>
                <span className="shortcut-meta">
                  <span className="shortcut-label">{s.label}</span>
                  <span className="shortcut-hint">{s.hint}</span>
                </span>
                <Icon name="chevronRight" size={15} className="shortcut-caret" />
              </button>
            ))}
          </div>
        </Card>
      </div>

      <Card className="recent-card recent-full">
        <div className="recent-head">
          <span className="overline">Recent</span>
          <button className="ghost-link" onClick={() => onNavigate("history")}>
            View all <Icon name="arrowRight" size={14} />
          </button>
        </div>
        {recent.length === 0 ? (
          <EmptyState icon="history" title="Nothing yet" desc="Your latest dictations will appear here." />
        ) : (
          <ul className="recent-list">
            {recent.map((r) => (
              <li key={r.id} className="recent-row">
                <span className="recent-text">{r.text}</span>
                <span className="recent-time mono">{ago(r.created_at)}</span>
                <CopyButton text={r.text} compact />
                <button
                  className="row-del"
                  aria-label="Delete"
                  title="Delete"
                  onClick={() => setPendingDelete(r)}
                >
                  <Icon name="trash" size={15} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <p className="foot-note">
        <Icon name="info" size={14} /> 100% on-device transcription. The text is typed into the
        focused window — keep your target app focused while you dictate.
      </p>

      <ConfirmDialog
        open={pendingDelete !== null}
        title="Delete this transcription?"
        body="It will be permanently removed from your history. This can't be undone."
        confirmLabel="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}

function MiniStat({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="mini-stat">
      <span className="mini-value mono">{value}</span>
      <span className="mini-label">{label}</span>
    </div>
  );
}

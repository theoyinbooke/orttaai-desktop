// Home (Mac HomeView): banner when there's something to do, the engine, Today
// and Performance, the 30-day trend, top apps, quick actions, recent
// dictations and insights. Owned by the analytics/home area.
import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import "../styles/analytics.css";
import { Button, ConfirmDialog, Icon, Kbd, Modal, PageHeader, Spinner, useToast } from "../ui";
import type { Tab } from "../types";
import { call, useEvent } from "../lib/api";
import { comboToString, useSettings } from "../lib/settings";
import {
  copyText,
  dayDate,
  fmt,
  modelDisplayName,
  pendingAnalyticsTab,
  plural,
  type DashboardPayload,
  type PerformanceHealth,
  type RecentDictation,
  type TodaySnapshot,
  type TopApp,
  type TranscriptionRecord,
  type TrendPoint,
} from "../lib/analytics";
import { CardIcon, LevelPill, TrendCard } from "./Analytics";
import { HistoryDetail } from "./History";
import EngineCard from "../components/EngineCard";
import InsightsPanel from "../components/InsightsPanel";
import { useEngine } from "../lib/engine";

const REPO_URL = "https://github.com/theoyinbooke/orttaai-desktop";

export default function Home({ onNavigate }: { onNavigate: (t: Tab) => void }) {
  const toast = useToast();
  const { settings, patch } = useSettings();
  const [payload, setPayload] = useState<DashboardPayload | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [prompt, setPrompt] = useState<"enjoyment" | "star" | null>(null);
  const [insightsOpen, setInsightsOpen] = useState(false);
  const { engine, snapshot } = useEngine();
  const checkedPrompt = useRef(false);
  const again = useRef(false);
  const running = useRef(false);

  // One refresh at a time; asked again meanwhile, it runs once more after.
  const refresh = useCallback(() => {
    if (running.current) {
      again.current = true;
      return;
    }
    running.current = true;
    setLoading(true);
    call<DashboardPayload>("home_summary")
      .then((p) => {
        setPayload(p);
        setError(null);
      })
      .catch((e) => setError(`Couldn't load dashboard data. ${e}`))
      .finally(() => {
        running.current = false;
        setLoading(false);
        if (again.current) {
          again.current = false;
          refresh();
        }
      });
  }, []);
  useEffect(refresh, [refresh]);
  useEvent("history-changed", refresh);

  useEffect(() => {
    if (!payload || checkedPrompt.current) return;
    checkedPrompt.current = true;
    call<boolean>("github_prompt_check")
      .then((show) => show && setPrompt("enjoyment"))
      .catch(() => {});
  }, [payload]);

  const openHistory = useCallback(() => {
    pendingAnalyticsTab.set("history");
    onNavigate("analytics");
  }, [onNavigate]);
  const runSetup = () => call("open_setup_window").catch((e) => toast(String(e), "error"));

  // Mac shortcuts: ⌘, settings, ⇧⌘H history, ⌘R refresh (Ctrl here).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k === "," && !e.shiftKey) {
        e.preventDefault();
        onNavigate("settings");
      } else if (k === "h" && e.shiftKey) {
        e.preventDefault();
        openHistory();
      } else if (k === "r" && !e.shiftKey) {
        e.preventDefault();
        refresh();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onNavigate, openHistory, refresh]);

  const header = (
    <div className="home-actions">
      {loading && payload && <Spinner />}
      <button className="chip-btn" onClick={openHistory} title="Open your dictation history">
        <Icon name="clockArrow" size={15} /> Open History
      </button>
      {!insightsOpen && (
        <button className="chip-btn" onClick={() => setInsightsOpen(true)} title="Open writing insights panel">
          <Icon name="waveSearch" size={15} /> Insight
        </button>
      )}
    </div>
  );

  if (!payload) {
    return (
      <div className="view home-view">
        <PageHeader title="Welcome back" actions={header} />
        {error ? <div className="an-error">{error}</div> : <div className="ms-loading"><Spinner /></div>}
      </div>
    );
  }

  const hasAny = payload.header.active_days > 0;
  const banner = !hasAny
    ? { title: "Start your first dictation", button: "Run Setup", action: runSetup }
    : payload.performance.level === "slow"
      ? { title: "Speed things up", button: "Open Model Settings", action: () => onNavigate("models") }
      : null;

  // The Mac needs no engine card: dictation just runs. Here it appears only when
  // there's something to do, or when the shortcut can't be relied on (Wayland),
  // since it carries the Click to record fallback.
  const showEngine = engine === "off" || engine === "loading" || engine === "error" || !!snapshot?.hotkey_error || !!snapshot?.wayland;

  return (
    <div className="view home-view">
      <PageHeader title="Welcome back" actions={header} />
      <div className="home-layout">
        <div className="home-main">
          {error && <div className="an-error">{error}</div>}

          {banner && (
            <div className="card home-banner" aria-label={`Banner. ${banner.title}`}>
              <Icon name="pulse" size={22} className="home-banner-icon" />
              <h2>{banner.title}</h2>
              <Button variant="primary" onClick={banner.action}>
                {banner.button}
              </Button>
            </div>
          )}

          {showEngine && <EngineCard onNavigate={onNavigate} />}

          <div className="home-row">
            <TodayCard today={payload.today} week={payload.trend30d.slice(-7)} usual={payload.usual_daily_words} />
            <PerformanceHealthCard health={payload.performance} />
          </div>

          <TrendCard points={payload.trend30d} allTime={payload.header} />

          <div className="home-row">
            <TopAppsCard apps={payload.top_apps30d} />
            <QuickActions
              onSettings={() => onNavigate("settings")}
              onModels={() => onNavigate("models")}
              onHistory={openHistory}
              onSetup={runSetup}
              onRefresh={refresh}
            />
          </div>

          <RecentCard
            entries={payload.recent}
            shortcut={comboToString(settings?.push_to_talk)}
            onOpenHistory={openHistory}
            onDeleted={refresh}
          />
        </div>
        {insightsOpen && (
          <div className="home-insights">
            <InsightsPanel onClose={() => setInsightsOpen(false)} />
          </div>
        )}
      </div>

      {prompt && (
        <GitHubPrompt
          step={prompt}
          onEnjoying={() => setPrompt("star")}
          onLater={() => setPrompt(null)}
          onStar={() => {
            setPrompt(null);
            patch({ github_star_prompt_completed: true }).catch(() => {});
            openUrl(REPO_URL).catch((e) => toast(String(e), "error"));
          }}
          onNever={() => {
            setPrompt(null);
            patch({ github_star_prompt_completed: true }).catch(() => {});
          }}
        />
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ today -- */

function TodayCard({ today, week, usual }: { today: TodaySnapshot; week: TrendPoint[]; usual: number | null }) {
  // Always seven days, so the strip keeps its shape before a week of history.
  const days: TrendPoint[] = [...week];
  while (days.length < 7) days.unshift({ day: "", words: 0, sessions: 0, average_wpm: 0 });
  const busiest = Math.max(1, ...days.map((d) => d.words));
  let comparison: { text: string; ahead: boolean } | null = null;
  if (today.sessions > 0 && usual && usual > 0) {
    const ratio = today.words / usual;
    if (ratio >= 1.1) comparison = { text: `${Math.round((ratio - 1) * 100)}% above your usual day`, ahead: true };
    else if (ratio <= 0.9) comparison = { text: `${fmt(usual)} words on a usual day`, ahead: false };
    else comparison = { text: "Right on your usual pace", ahead: false };
  }
  const activeDays = days.filter((d) => d.sessions > 0).length;
  const weekWords = days.reduce((a, d) => a + d.words, 0);
  const weekLine =
    comparison?.text ??
    (activeDays > 0 ? `${plural(weekWords, "word")} this week · ${plural(activeDays, "active day")}` : "No dictation this week yet");
  const activeTime = today.active_minutes === 0 ? (today.sessions > 0 ? "<1 min" : "0 min") : `${fmt(today.active_minutes)} min`;
  return (
    <div className="card home-card" aria-label={`Today: ${today.words} words, ${today.sessions} sessions, ${activeTime} active. ${weekLine}.`}>
      <div className="an-card-head">
        <h3>Today</h3>
        <span className="an-card-detail">{new Date().toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" })}</span>
      </div>
      <div className="home-big">
        <span className={`home-big-value ${today.sessions === 0 ? "faint" : ""}`}>{fmt(today.words)}</span>
        <span className="home-big-unit">{today.words === 1 ? "word" : "words"}</span>
        {today.sessions === 0 && <span className="home-big-note">Nothing dictated yet</span>}
      </div>
      <div className="week-strip" aria-hidden="true">
        {days.map((d, i) => {
          const isToday = i === days.length - 1;
          const name = isToday ? "Today" : d.day ? dayDate(d.day).toLocaleDateString(undefined, { weekday: "long" }) : "";
          return (
            <span
              key={i}
              className={`week-seg ${isToday ? "today" : ""} ${d.words > 0 ? "on" : ""}`}
              style={d.words > 0 ? { opacity: 0.3 + (0.7 * d.words) / busiest } : undefined}
              title={name ? (d.words > 0 ? `${name}: ${fmt(d.words)} words` : `${name}: no dictation`) : undefined}
            />
          );
        })}
      </div>
      <span className={`home-week-line ${comparison?.ahead ? "ahead" : ""}`}>{weekLine}</span>
      <div className="home-stats">
        <MiniStat icon="waveform" value={fmt(today.sessions)} label={today.sessions === 1 ? "session" : "sessions"} />
        <MiniStat icon="clock" value={activeTime} label="active" />
        <MiniStat icon="speedometer" value={today.average_wpm > 0 ? fmt(today.average_wpm) : "–"} label="WPM" />
      </div>
    </div>
  );
}

function MiniStat({ icon, value, label }: { icon: string; value: string; label: string }) {
  return (
    <div className="home-mini" aria-label={`${value} ${label}`}>
      <CardIcon name={icon} />
      <span className="home-mini-value">{value}</span>
      <span className="home-mini-label">{label}</span>
    </div>
  );
}

/* ------------------------------------------------------------ performance -- */

function PerformanceHealthCard({ health }: { health: PerformanceHealth }) {
  // Each stage's own typical time, in pipeline order.
  const segments = [
    { id: "transcribe", label: "Transcribe", ms: health.transcription_p50_ms ?? 0, p95: health.transcription_p95_ms, tone: "c2" },
    { id: "polish", label: "Rules & polish", ms: health.text_processing_p50_ms ?? 0, p95: health.text_processing_p95_ms, tone: "c3" },
    { id: "inject", label: "Insert", ms: health.injection_p50_ms ?? 0, p95: health.injection_p95_ms, tone: "c4" },
  ].filter((s) => s.ms > 0);
  const sum = Math.max(1, segments.reduce((a, s) => a + s.ms, 0));
  const total = health.processing_p50_ms;
  return (
    <div className="card home-card">
      <div className="an-card-head">
        <h3>Performance</h3>
        <LevelPill level={health.level} />
      </div>
      <div className="home-big">
        <span className={`home-big-value ${total == null ? "faint" : ""}`}>{total ?? "–"}</span>
        <span className="home-big-unit">ms per dictation</span>
        {total != null && <span className="home-big-note">{health.sample_count === 1 ? "Last dictation" : `Last ${health.sample_count} dictations`}</span>}
      </div>
      {total != null ? (
        <>
          <div className="perf-bar" aria-hidden="true">
            {segments.map((s) => (
              <span key={s.id} className={s.tone} style={{ flexGrow: s.ms / sum }} />
            ))}
          </div>
          <div className="perf-legend">
            {segments.map((s) => (
              <span key={s.id} title={s.p95 != null ? `${s.label}: typically ${s.ms} ms, ${s.p95} ms in the slowest 1 in 20` : `${s.label}: typically ${s.ms} ms`}>
                <i className={`dot ${s.tone}`} /> {s.label} <b>{fmt(s.ms)} ms</b>
              </span>
            ))}
          </div>
        </>
      ) : (
        <>
          <div className="perf-bar empty" aria-hidden="true" />
          <span className="home-week-line">Timings appear after your first dictation</span>
        </>
      )}
      <div className="home-stats">
        <div className="home-mini wide" title={health.current_model_id}>
          <CardIcon name="chip" />
          <span className="home-mini-value">{modelDisplayName(health.current_model_id)}</span>
          {health.is_express && <span className="express-chip">Express</span>}
        </div>
      </div>
    </div>
  );
}

/* --------------------------------------------------------- apps + actions -- */

function TopAppsCard({ apps }: { apps: TopApp[] }) {
  const shown = apps.slice(0, 4);
  return (
    <div className="card home-card fixed" aria-label="Top apps over 30 days">
      <h3 className="home-card-title">Top Apps (30d)</h3>
      {shown.length === 0 ? (
        <p className="an-empty tight">No app usage data yet.</p>
      ) : (
        <div className="home-apps">
          {shown.map((a) => (
            <div key={a.name} className="home-app" aria-label={`${a.name}, ${a.sessions} sessions, ${Math.round(a.session_share * 100)} percent of sessions.`}>
              <div className="home-app-top">
                <span className="home-app-name">{a.name}</span>
                <span className="muted">{plural(a.sessions, "session")}</span>
              </div>
              <div className="an-track">
                <span style={{ width: `${a.session_share * 100}%` }} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function QuickActions({
  onSettings,
  onModels,
  onHistory,
  onSetup,
  onRefresh,
}: {
  onSettings: () => void;
  onModels: () => void;
  onHistory: () => void;
  onSetup: () => void;
  onRefresh: () => void;
}) {
  const actions: { label: string; icon: string; onClick: () => void }[] = [
    { label: "Open Settings", icon: "gear", onClick: onSettings },
    { label: "Open Model Settings", icon: "cpu", onClick: onModels },
    { label: "Open Full History", icon: "clockArrow", onClick: onHistory },
    { label: "Run Setup", icon: "sliders", onClick: onSetup },
    { label: "Refresh Dashboard", icon: "refresh", onClick: onRefresh },
  ];
  return (
    <div className="card home-card fixed" aria-label="Quick actions">
      <h3 className="home-card-title">Quick Actions</h3>
      <div className="quick-list">
        {actions.map((a) => (
          <button key={a.label} className="quick-btn" onClick={a.onClick}>
            <Icon name={a.icon} size={15} />
            <span>{a.label}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

/* ----------------------------------------------------------------- recent -- */

function RecentCard({
  entries,
  shortcut,
  onOpenHistory,
  onDeleted,
}: {
  entries: RecentDictation[];
  shortcut: string;
  onOpenHistory: () => void;
  onDeleted: () => void;
}) {
  const toast = useToast();
  const [copied, setCopied] = useState<number | null>(null);
  const [pending, setPending] = useState<RecentDictation | null>(null);
  const [detail, setDetail] = useState<TranscriptionRecord | null>(null);

  const copy = async (text: string, id: number) => {
    try {
      await copyText(text, id);
      setCopied(id);
      window.setTimeout(() => setCopied((c) => (c === id ? null : c)), 1100);
    } catch (e) {
      toast(String(e), "error");
    }
  };
  const open = async (e: RecentDictation) => {
    try {
      const r = await call<TranscriptionRecord | null>("get_transcription", { id: e.id });
      if (r) setDetail(r);
      else toast("That dictation is no longer in local history.", "warn");
    } catch (err) {
      toast(String(err), "error");
    }
  };
  const remove = async (e: RecentDictation) => {
    setPending(null);
    try {
      await call("delete_transcription", { id: e.id });
      if (detail?.id === e.id) setDetail(null);
      onDeleted();
    } catch (err) {
      toast(`Couldn't delete dictation. ${err}`, "error");
    }
  };

  return (
    <div className="card an-card" aria-label="Recent dictations table">
      <div className="an-card-head">
        <h3>Recent Dictations</h3>
        <button className="ghost-link" onClick={onOpenHistory}>
          Open History
        </button>
      </div>
      {entries.length === 0 ? (
        <p className="an-empty tight">
          No dictations yet. {shortcut ? <>Hold <Kbd combo={shortcut} /> to get started.</> : "Hold your dictation shortcut to get started."}
        </p>
      ) : (
        <div className="recent-table" role="table">
          <div className="recent-tr recent-th" role="row">
            <span className="r-time">Time</span>
            <span className="r-app">App</span>
            <span className="r-text">Transcript</span>
            <span className="r-words">Words</span>
            <span className="r-lat">Latency</span>
            <span className="r-act">Actions</span>
          </div>
          {entries.map((e) => (
            <div
              key={e.id}
              className="recent-tr"
              role="row"
              tabIndex={0}
              onClick={() => open(e)}
              onKeyDown={(k) => k.key === "Enter" && open(e)}
              aria-label={`${e.app_name}, ${e.preview_text}, ${e.word_count} words, ${e.processing_ms} milliseconds processing.`}
            >
              <span className="r-time">{new Date(e.created_at * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</span>
              <span className="r-app">{e.app_name}</span>
              <span className="r-text">{e.preview_text}</span>
              <span className="r-words mono-num">{e.word_count}</span>
              <span className="r-lat mono-num">{fmt(e.processing_ms)} ms</span>
              <span className="r-act" onClick={(k) => k.stopPropagation()}>
                <button className={`row-icon ${copied === e.id ? "ok" : ""}`} onClick={() => copy(e.full_text, e.id)} title={copied === e.id ? "Copied" : "Copy"} aria-label="Copy">
                  <Icon name={copied === e.id ? "check" : "copy"} size={14} />
                </button>
                <button className="row-icon danger" onClick={() => setPending(e)} title="Delete" aria-label="Delete">
                  <Icon name="trash" size={14} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}
      <Modal open={!!detail} onClose={() => setDetail(null)} labelledBy="hist-detail-title">
        {detail && (
          <HistoryDetail
            record={detail}
            copied={copied === detail.id}
            onCopy={() => detail.id != null && copy(detail.text, detail.id)}
            onClose={() => setDetail(null)}
          />
        )}
      </Modal>
      <ConfirmDialog
        open={!!pending}
        title="Delete Dictation?"
        body="This removes the transcript from local history."
        confirmLabel="Delete"
        onConfirm={() => pending && remove(pending)}
        onCancel={() => setPending(null)}
      />
    </div>
  );
}

/* ---------------------------------------------------------- GitHub prompt -- */

function GitHubPrompt({
  step,
  onEnjoying,
  onLater,
  onStar,
  onNever,
}: {
  step: "enjoyment" | "star";
  onEnjoying: () => void;
  onLater: () => void;
  onStar: () => void;
  onNever: () => void;
}) {
  const enjoy = step === "enjoyment";
  return (
    <Modal open onClose={onLater} labelledBy="gh-title">
      <div className={`gh-prompt ${enjoy ? "" : "wide"}`}>
        <span className="gh-eyebrow">{enjoy ? "Community" : "GitHub"}</span>
        <h2 id="gh-title">{enjoy ? "Enjoying Orttaai?" : "Support the project"}</h2>
        <p className="muted">
          {enjoy
            ? "If Orttaai is helping your workflow, a GitHub star is the simplest way to support the project."
            : "Your star helps more people discover Orttaai and tells me the product is worth pushing further."}
        </p>
        <div className="gh-actions">
          {enjoy ? (
            <>
              <Button variant="ghost" onClick={onLater}>
                Not now
              </Button>
              <Button variant="primary" onClick={onEnjoying}>
                Yes, I am
              </Button>
            </>
          ) : (
            <>
              <Button variant="ghost" onClick={onLater}>
                Maybe later
              </Button>
              <Button variant="danger" onClick={onNever}>
                No thanks
              </Button>
              <Button variant="primary" onClick={onStar}>
                Star on GitHub
              </Button>
            </>
          )}
        </div>
      </div>
    </Modal>
  );
}

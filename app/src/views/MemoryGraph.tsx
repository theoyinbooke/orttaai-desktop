// Memory Graph (Mac SemanticMemoryView): Activity · Map · Insights over the
// user's own dictations. Owned by the semantic area.
import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent, type ReactNode } from "react";
import "../styles/semantic.css";
import { Button, ConfirmDialog, Icon, IconButton, PageHeader, Segmented, Spinner, Tabs, useToast } from "../ui";
import { useSettings, type Settings } from "../lib/settings";
import { useEvent } from "../lib/api";
import {
  GLYPHS,
  RANGES,
  askChat,
  mediumDateTime,
  monthDay,
  openSettings,
  pct,
  plural,
  rangePhrase,
  relative,
  semantic,
  useLlmStatus,
  useSemanticStatus,
  type ActivityReport,
  type ActivityShare,
  type ActivityState,
  type ConceptDetail,
  type ConceptGraph,
  type ConceptKind,
  type HighlightKind,
  type IndexProgress,
  type IndexRunResult,
  type IndexStatus,
  type InsightCard,
  type InsightChart,
  type InsightEvidence,
  type InsightFinding,
  type LlmStatus,
  type Range,
  type SessionNote,
  type SignalWithContext,
  type SignalsOverview,
  type StoredReport,
  type WorkActivity,
} from "../lib/semantic";

/* ---------------------------------------------------------------- helpers -- */

export function Glyph({ name, size = 16, className }: { name: string; size?: number; className?: string }) {
  const d = GLYPHS[name];
  if (!d) return <Icon name={name} size={size} className={className} />;
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={d} />
    </svg>
  );
}

function useStored<T extends string>(key: string, initial: T, allowed: readonly T[]) {
  const [value, setValue] = useState<T>(() => {
    try {
      const v = localStorage.getItem(key) as T | null;
      return v && allowed.includes(v) ? v : initial;
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (v: T) => {
      setValue(v);
      try {
        localStorage.setItem(key, v);
      } catch {
        /* storage unavailable */
      }
    },
    [key],
  );
  return [value, set] as const;
}

function Hint({ text }: { text: string }) {
  return (
    <span className="sm-hint" tabIndex={0} data-tip={text} aria-label={text}>
      <Icon name="info" size={13} />
    </span>
  );
}

function CardHead({ title, info, count }: { title: string; info?: string; count?: number }) {
  return (
    <div className="sm-card-head">
      <span className="sm-card-title">{title}</span>
      {info && <Hint text={info} />}
      {count !== undefined && count > 0 && <span className="sm-card-count">{count.toLocaleString()}</span>}
    </div>
  );
}

const ACTIVITY_COLOR: Record<WorkActivity, string> = {
  building: "var(--sm-blue)",
  fixing: "var(--sm-orange)",
  designing: "var(--sm-purple)",
  planning: "var(--sm-teal)",
  researching: "var(--sm-indigo)",
  writing: "var(--sm-green)",
  communicating: "var(--sm-pink)",
  setup: "var(--sm-gray)",
  reviewing: "var(--sm-mint)",
  other: "var(--faint)",
};
const ACTIVITY_GLYPH: Record<WorkActivity, string> = {
  building: "hammer",
  fixing: "wrench",
  designing: "brush",
  planning: "map",
  researching: "search",
  writing: "quote",
  communicating: "chat",
  setup: "gear",
  reviewing: "seal",
  other: "dotted",
};
const KIND_COLOR: Record<ConceptKind, string> = {
  person: "var(--sm-green)",
  organization: "var(--sm-teal)",
  place: "var(--sm-pink)",
  product: "var(--accent)",
  topic: "var(--sm-purple)",
};
const THEME_PALETTE = [
  "var(--accent)",
  "var(--sm-purple)",
  "var(--sm-teal)",
  "var(--sm-pink)",
  "var(--sm-blue)",
  "var(--sm-green)",
  "var(--sm-indigo)",
  "var(--sm-mint)",
  "var(--sm-red)",
  "var(--sm-yellow)",
];
const HIGHLIGHT_GLYPH: Record<HighlightKind, string> = {
  rising: "up",
  new: "zoomIn",
  pair: "link",
  person: "person",
  fading: "moonZ",
  focus: "appBadge",
};

function themeColor(themeId: string | null, graph: ConceptGraph) {
  if (!themeId) return "var(--faint)";
  const i = graph.themes.findIndex((t) => t.id === themeId);
  return i < 0 ? "var(--faint)" : THEME_PALETTE[i % THEME_PALETTE.length];
}

const TABS = ["activity", "map", "insights"] as const;
type TabId = (typeof TABS)[number];
const RANGE_VALUES = RANGES.map((r) => r.value);

/* ------------------------------------------------------------------ page --- */

export default function MemoryGraph() {
  const { settings, patch } = useSettings();
  const toast = useToast();
  const enabled = settings?.semantic_enabled ?? false;
  const sem = useSemanticStatus();
  const { llm, checking, check } = useLlmStatus(enabled);
  const [tab, setTab] = useStored<TabId>("memoryGraphTab", "activity", TABS);
  const [range, setRange] = useStored<Range>("memoryGraphRange", "month", RANGE_VALUES);
  const [message, setMessage] = useState<{ tone: "ok" | "error"; text: string } | null>(null);

  // Surface the outcome of each index pass, like the Mac status strip.
  useEvent<IndexRunResult | Record<string, unknown>>("semantic-updated", (r) => {
    if (!r || !("embedded_count" in r)) return;
    const run = r as IndexRunResult;
    if (run.error_message && run.embedded_count === 0) setMessage({ tone: "error", text: run.error_message });
    else if (run.source_count > 0) {
      const fallback = run.used_fallback ? " using lexical fallback" : "";
      setMessage({
        tone: "ok",
        text: `Indexed ${plural(run.embedded_count, "new chunk")} with ${run.model_id}${fallback}.`,
      });
    }
  });

  const enable = () =>
    patch({ semantic_enabled: true, semantic_auto_index: true })
      .then(() => {
        toast("Semantic memory is on. Indexing your dictations…", "success");
        sem.refresh();
        check();
      })
      .catch((e) => toast(String(e), "error"));

  const usesRange = tab !== "insights";
  return (
    <div className={`view sm-view ${tab === "map" ? "sm-wide" : ""}`}>
      <PageHeader
        title="Memory Graph"
        desc="Activity, concept map and insights from your dictations."
        actions={
          enabled && (
            <>
              <AboutButton status={sem.status?.index ?? null} />
              {usesRange ? (
                <Segmented value={range} options={RANGES} onChange={setRange} />
              ) : (
                <Button size="sm" icon="settings" onClick={openSettings}>
                  AI Settings
                </Button>
              )}
            </>
          )
        }
      />

      {!settings ? (
        <div className="card sm-spinner-row">
          <Spinner /> Loading…
        </div>
      ) : !enabled ? (
        <Onboarding onEnable={enable} />
      ) : (
        <>
          <div className="sm-tabs-row">
            <Tabs
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "activity", label: "Activity" },
                { value: "map", label: "Map" },
                { value: "insights", label: "Insights" },
              ]}
            />
          </div>
          <StatusStrip progress={sem.progress} message={message} onDismiss={() => setMessage(null)} />
          <LlmBanner llm={llm} settings={settings} checking={checking} onRetry={check} status={sem.status?.index ?? null} />
          {tab === "activity" && (
            <ActivityTab range={range} revision={sem.revision} activity={sem.activity} setActivity={sem.setActivity} />
          )}
          {tab === "map" && <MapTab range={range} revision={sem.revision} />}
          {tab === "insights" && (
            <InsightsTab
              settings={settings}
              llm={llm}
              status={sem.status?.index ?? null}
              running={!!sem.progress || !!sem.status?.running}
              revision={sem.revision}
              onMessage={setMessage}
              refreshStatus={sem.refresh}
            />
          )}
        </>
      )}
    </div>
  );
}

function AboutButton({ status }: { status: IndexStatus | null }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  return (
    <div className="sm-info-btn" ref={ref}>
      <IconButton name="info" label="About Memory Graph" onClick={() => setOpen((o) => !o)} />
      {open && (
        <div className="sm-popover" role="dialog">
          <h4>Local map of your dictations</h4>
          <div className="sm-popover-row">
            <Glyph name="shield" size={14} />
            Everything is computed on this computer; a local model (Ollama or LM Studio) only phrases what is found.
          </div>
          {status && (
            <>
              <div className="sm-popover-row">
                <Glyph name="stack" size={14} />
                {plural(status.indexed_transcription_count, "dictation")} indexed ·{" "}
                {plural(status.embedded_chunk_count, "chunk")}
              </div>
              <div className="sm-popover-row">
                <Glyph name="cpu" size={14} />
                {status.active_model ? `Embeddings by ${status.active_model}` : "Not indexed yet"}
                {status.using_fallback ? " (keyword fallback)" : ""}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function Onboarding({ onEnable }: { onEnable: () => void }) {
  const items = [
    "Activity: what kind of work your dictation was, what got in the way, decisions and open questions.",
    "Map: the people, projects and topics you talk about, how they connect, and what is rising or fading.",
    "Insights: open loops, rhythms and patterns, each linked back to what you actually said.",
    "Chat AI can search your dictations by meaning once they are indexed.",
  ];
  return (
    <div className="card sm-onboard">
      <div className="sm-onboard-icon">
        <Icon name="graph" size={22} />
      </div>
      <div>
        <h2>Turn on semantic memory</h2>
        <p>
          Orttaai can index your dictations on this computer to map what you talk about and point out patterns. Nothing
          leaves your machine: embeddings come from your local model server, with a keyword fallback when it is off.
        </p>
        <ul className="sm-onboard-list">
          {items.map((t) => (
            <li key={t}>
              <Glyph name="checkCircle" size={14} />
              {t}
            </li>
          ))}
        </ul>
        <div className="sm-onboard-actions">
          <Button variant="primary" icon="graph" onClick={onEnable}>
            Turn On Semantic Memory
          </Button>
          <Button variant="ghost" icon="settings" onClick={openSettings}>
            AI Settings
          </Button>
        </div>
      </div>
    </div>
  );
}

function StatusStrip({
  progress,
  message,
  onDismiss,
}: {
  progress: IndexProgress | null;
  message: { tone: "ok" | "error"; text: string } | null;
  onDismiss: () => void;
}) {
  if (progress) {
    const ratio = progress.total > 0 ? Math.min(1, progress.done / progress.total) : null;
    return (
      <div className="sm-strip sm-strip-info" role="status">
        <Spinner size={13} />
        <span className="sm-strip-text">{progress.message}</span>
        <span className={`sm-progress ${ratio === null ? "indeterminate" : ""}`}>
          <span style={ratio === null ? undefined : { width: `${Math.round(ratio * 100)}%` }} />
        </span>
      </div>
    );
  }
  if (!message) return null;
  return (
    <div className={`sm-strip ${message.tone === "ok" ? "sm-strip-ok" : "sm-strip-error"}`} role="status">
      <Icon name={message.tone === "ok" ? "check" : "alert"} size={14} />
      <span className="sm-strip-text">{message.text}</span>
      <button className="icon-btn" aria-label="Dismiss" onClick={onDismiss}>
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}

function LlmBanner({
  llm,
  settings,
  checking,
  onRetry,
  status,
}: {
  llm: LlmStatus | null;
  settings: Settings;
  checking: boolean;
  onRetry: () => void;
  status: IndexStatus | null;
}) {
  if (!llm) return null;
  const provider = settings.llm_provider === "lm_studio" ? "LM Studio" : "Ollama";
  const endpoint = settings.llm_provider === "lm_studio" ? settings.lmstudio_endpoint : settings.ollama_endpoint;
  if (!llm.reachable) {
    return (
      <div className="sm-banner sm-banner-warn" role="alert">
        <Icon name="alert" size={18} />
        <div className="sm-banner-body">
          <div className="sm-banner-title">
            {provider} isn't reachable at {endpoint}
          </div>
          <div className="sm-banner-text">
            Activity and the map still work from your wording.
            {settings.semantic_keyword_fallback
              ? " New dictations are indexed with the keyword fallback until it is back."
              : " Indexing waits until it is back."}
          </div>
        </div>
        <div className="sm-banner-actions">
          <Button size="sm" icon="refresh" loading={checking} onClick={onRetry}>
            Retry
          </Button>
          <Button size="sm" icon="settings" onClick={openSettings}>
            Settings
          </Button>
        </div>
      </div>
    );
  }
  if (!llm.embedding_model_installed) {
    const model = settings.semantic_embedding_model || "all-minilm";
    return (
      <div className="sm-banner sm-banner-warn">
        <Icon name="alert" size={18} />
        <div className="sm-banner-body">
          <div className="sm-banner-title">
            The embedding model {model} isn't installed in {provider}
          </div>
          <div className="sm-banner-text">
            {settings.llm_provider === "ollama" ? (
              <>
                Run <code className="mono">ollama pull {model}</code>, or pick one of your models in Settings.
              </>
            ) : (
              <>Load an embedding model in LM Studio, or pick one of your models in Settings.</>
            )}
            {llm.embedding_models.length > 0 && ` Installed: ${llm.embedding_models.slice(0, 4).join(", ")}.`}
          </div>
        </div>
        <div className="sm-banner-actions">
          <Button size="sm" icon="refresh" loading={checking} onClick={onRetry}>
            Check Again
          </Button>
        </div>
      </div>
    );
  }
  if (status?.using_fallback) {
    return (
      <div className="sm-banner sm-banner-accent">
        <Icon name="info" size={18} />
        <div className="sm-banner-body">
          <div className="sm-banner-title">{provider} is back</div>
          <div className="sm-banner-text">
            Your index used the keyword fallback while it was away. Index now to re-embed with {status.configured_model}.
          </div>
        </div>
        <div className="sm-banner-actions">
          <Button size="sm" icon="refresh" onClick={() => semantic.indexNow()}>
            Index Now
          </Button>
        </div>
      </div>
    );
  }
  return null;
}

/* -------------------------------------------------------------- activity --- */

function ActivityTab({
  range,
  revision,
  activity,
  setActivity,
}: {
  range: Range;
  revision: number;
  activity: ActivityState | null;
  setActivity: (s: ActivityState) => void;
}) {
  const [report, setReport] = useState<ActivityReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const started = useRef<Range | null>(null);

  useEffect(() => {
    let live = true;
    setLoading(true);
    semantic
      .activity(range)
      .then((r) => {
        if (!live) return;
        setReport(r);
        setError(null);
      })
      .catch((e) => live && setError(`Couldn't read your sessions: ${e}`))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [range, revision]);

  // Read sessions with local AI in the background (twice the range).
  useEffect(() => {
    const state = activity?.state ?? "idle";
    const idle = state === "idle" || state === "" || state === "finished";
    if (idle && started.current !== range) {
      started.current = range;
      semantic.activityStart(range).then(setActivity).catch(() => {});
    }
  }, [range, activity?.state, setActivity]);

  if (error) {
    return (
      <div className="sm-banner sm-banner-warn">
        <Icon name="alert" size={18} />
        <div className="sm-banner-body">{error}</div>
      </div>
    );
  }
  if (!report || (loading && report.session_count === 0)) {
    return (
      <div className="card sm-empty-card">
        <Spinner /> Reading your dictation history…
      </div>
    );
  }
  if (report.session_count === 0) {
    return (
      <div className="card sm-empty-card">
        No working sessions in {rangePhrase(range)} yet. Dictate a little more, or pick a longer range.
      </div>
    );
  }
  return <ActivityContent report={report} range={range} activity={activity} setActivity={setActivity} />;
}

function AnalysisStatus({
  activity,
  range,
  setActivity,
}: {
  activity: ActivityState | null;
  range: Range;
  setActivity: (s: ActivityState) => void;
}) {
  const start = () => semantic.activityStart(range).then(setActivity).catch(() => {});
  const state = activity?.state || "idle";
  switch (state) {
    case "checking":
      return (
        <>
          <Spinner size={12} /> Checking local AI…
        </>
      );
    case "running":
      return (
        <>
          <span className="sm-progress">
            <span style={{ width: `${Math.round(((activity?.done ?? 0) / Math.max(1, activity?.total ?? 1)) * 100)}%` }} />
          </span>
          <span className="mono">
            Reading {activity?.done ?? 0} of {activity?.total ?? 0}
          </span>
          <button className="sm-link" onClick={() => semantic.activityStop().then(setActivity)}>
            Stop
          </button>
        </>
      );
    case "paused":
      return (
        <>
          Paused while you dictate · {activity?.done ?? 0} of {activity?.total ?? 0}
        </>
      );
    case "finished":
      return (
        <span className="sm-chip sm-chip-ok">
          <Glyph name="checkCircle" size={12} /> Read by {activity?.model}
        </span>
      );
    case "unavailable":
      return (
        <>
          <span className="sm-chip sm-chip-warn" title={activity?.message ?? ""}>
            <Icon name="alert" size={12} /> Local AI off
          </span>
          <button className="sm-link" onClick={start} title={activity?.message ?? ""}>
            Try Again
          </button>
        </>
      );
    default:
      return (
        <Button size="sm" onClick={start}>
          <Glyph name="cpu" size={14} /> Read with local AI
        </Button>
      );
  }
}

function ChangeLabel({ change }: { change: number | null }) {
  if (change === null || Math.abs(change) < 3) return <span className="sm-change" />;
  return (
    <span className={`sm-change ${change > 0 ? "up" : "down"}`}>
      {change > 0 ? "↑" : "↓"}
      {Math.abs(change)}
    </span>
  );
}

function ActivityContent({
  report,
  range,
  activity,
  setActivity,
}: {
  report: ActivityReport;
  range: Range;
  activity: ActivityState | null;
  setActivity: (s: ActivityState) => void;
}) {
  const phrase = rangePhrase(range);
  const top = report.activities[0];
  const snagged = Math.round(report.friction_rate * report.session_count);
  const story = report.headline.slice(1).join(" ");
  const summaryQuestion = `Look at my dictation from ${phrase}. In short sections, tell me what I've been working on, the decisions I made, what's still open, and anything that kept getting in my way.`;
  const shares = report.activities.slice(0, 6);
  return (
    <>
      <div className="card sm-card">
        <div className="sm-summary-head">
          <span className="sm-summary-title">{phrase.charAt(0).toUpperCase() + phrase.slice(1)}</span>
          <span className="sm-summary-status">
            <AnalysisStatus activity={activity} range={range} setActivity={setActivity} />
          </span>
        </div>
        <div className="sm-stats">
          <Stat glyph="waveform" value={report.session_count.toLocaleString()} label={report.session_count === 1 ? "working session" : "working sessions"} />
          <Stat glyph="calendar" value={report.day_count.toLocaleString()} label={report.day_count === 1 ? "active day" : "active days"} />
          {top && (
            <Stat
              glyph={ACTIVITY_GLYPH[top.activity]}
              value={pct(top.share)}
              label={top.title.toLowerCase()}
              tint={ACTIVITY_COLOR[top.activity]}
            />
          )}
          <Stat
            glyph={report.friction_rate > 0 ? "alert" : "checkCircle"}
            value={pct(report.friction_rate)}
            label="hit a snag"
            tint={report.friction_rate >= 0.3 ? "var(--warn)" : "var(--accent)"}
          />
        </div>
        {story && <p className="sm-story">{story}</p>}
        <div className="sm-summary-foot">
          {report.model_analyzed_count < report.session_count && (
            <span className="sm-note">
              <Icon name="info" size={13} />
              {report.model_analyzed_count === 0
                ? "Estimated from your wording until local AI reads these sessions."
                : `${report.model_analyzed_count} of ${report.session_count} sessions read by local AI; the rest are estimated.`}
            </span>
          )}
          <Button size="sm" onClick={() => askChat(summaryQuestion)}>
            <Glyph name="chat" size={14} /> Ask Chat AI
          </Button>
        </div>
      </div>

      <div className="sm-pair">
        <div className="card sm-card">
          <CardHead
            title="Where your time goes"
            info="The main kind of work in each working session (dictations in one app, close together). The arrow compares with the period before."
          />
          <div className="sm-stackbar" role="img" aria-label="Share of sessions by kind of work">
            {shares.map((s) => (
              <span key={s.activity} style={{ flex: s.share, background: ACTIVITY_COLOR[s.activity] }} title={`${s.title} ${pct(s.share)}`} />
            ))}
          </div>
          <div className="sm-key">
            {shares.map((s) => (
              <ShareRow key={s.activity} share={s} />
            ))}
          </div>
        </div>
        <div className="card sm-card">
          <CardHead
            title="What gets in the way"
            info="Sessions where something went wrong or blocked you, grouped by kind, with your most recent example."
          />
          {report.frictions.length === 0 ? (
            <div className="sm-quote">
              <Glyph name="checkCircle" size={14} /> Nothing got in the way in this range.
            </div>
          ) : (
            <>
              <div className="sm-friction-sub">
                {snagged} of {report.session_count} sessions hit a snag
              </div>
              <div className="sm-friction-list">
                {report.frictions.slice(0, 4).map((g) => (
                  <div key={g.type}>
                    <div className="sm-friction-row-head">
                      <span>{g.title}</span>
                      <span>{plural(g.sessions, "session")}</span>
                    </div>
                    {g.examples[0] && <div className="sm-friction-example">“{g.examples[0].text}”</div>}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      </div>

      {report.projects.length > 0 && (
        <div className="card sm-card">
          <CardHead title="Projects" info="Named things you worked on, with the mix of work each got, what got in the way, and the latest goal." />
          <div className="sm-projects">
            {report.projects.map((p) => (
              <div className="sm-project" key={p.id}>
                <div className="sm-project-head">
                  <span className="sm-project-title">{p.title}</span>
                  <span className="sm-project-meta">
                    {p.sessions} sessions · last active {relative(p.last_active)}
                  </span>
                </div>
                <div className="sm-project-row">
                  {p.activities.map((a) => (
                    <span className="sm-chip" key={a.activity}>
                      <span style={{ color: ACTIVITY_COLOR[a.activity], display: "inline-flex" }}>
                        <Glyph name={ACTIVITY_GLYPH[a.activity]} size={12} />
                      </span>
                      {a.title} <strong>{pct(a.share)}</strong>
                    </span>
                  ))}
                </div>
                {(p.friction_sessions > 0 || p.latest_goal) && (
                  <div className="sm-project-foot">
                    {p.latest_goal && <span>Latest: {p.latest_goal}</span>}
                    {p.friction_sessions > 0 && (
                      <span>
                        Friction in {p.friction_sessions}, mostly {midSentence(p.top_friction_title ?? "Other")}
                      </span>
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="sm-pair">
        <NotesCard
          title="Decisions"
          info="Things you said you decided, newest first."
          glyph="seal"
          notes={report.decisions}
          empty="No decisions stated in this range."
        />
        <NotesCard
          title="Open questions"
          info="Things you asked or left unresolved, newest first."
          glyph="question"
          notes={report.open_questions}
          empty="Nothing left open in this range."
          onAsk={(n) =>
            askChat(
              `On ${new Date(n.date * 1000).toLocaleDateString([], { month: "long", day: "numeric" })} I left this open: “${n.text}”. Using what I've dictated about it, help me think it through and suggest a next step.`,
            )
          }
        />
      </div>
    </>
  );
}

/** Lowercases a title for use mid-sentence, leaving acronyms like "UI". */
function midSentence(title: string) {
  return title
    .split(" ")
    .map((w) => (w.length > 1 && w === w.toUpperCase() ? w : w.toLowerCase()))
    .join(" ");
}

function Stat({ glyph, value, label, tint = "var(--accent)" }: { glyph: string; value: string; label: string; tint?: string }) {
  return (
    <div className="sm-stat">
      <span style={{ color: tint, display: "inline-flex" }}>
        <Glyph name={glyph} size={15} />
      </span>
      <span className="sm-stat-value">{value}</span>
      <span className="sm-stat-label">{label}</span>
    </div>
  );
}

function ShareRow({ share }: { share: ActivityShare }) {
  return (
    <div className="sm-key-row">
      <span style={{ color: ACTIVITY_COLOR[share.activity], display: "inline-flex" }}>
        <Glyph name={ACTIVITY_GLYPH[share.activity]} size={14} />
      </span>
      <span>{share.title}</span>
      <span className="sm-key-num">{plural(share.sessions, "session")}</span>
      <span className="sm-key-share">{pct(share.share)}</span>
      <ChangeLabel change={share.change} />
    </div>
  );
}

function noteCaption(n: SessionNote) {
  return [n.project, monthDay(n.date), n.app_name].filter(Boolean).join(" · ");
}

function NotesCard({
  title,
  info,
  glyph,
  notes,
  empty,
  onAsk,
}: {
  title: string;
  info: string;
  glyph: string;
  notes: SessionNote[];
  empty: string;
  onAsk?: (n: SessionNote) => void;
}) {
  return (
    <div className="card sm-card">
      <CardHead title={title} info={info} count={notes.length} />
      {notes.length === 0 ? (
        <div className="sm-quote">{empty}</div>
      ) : (
        <div className="sm-notes">
          {notes.slice(0, 5).map((n) => (
            <div className="sm-note-row" key={n.id}>
              <Glyph name={glyph} size={14} />
              <div>
                <div className="sm-note-text">{n.text}</div>
                <div className="sm-note-caption">
                  {noteCaption(n)}
                  {n.is_estimate ? " · estimated" : ""}
                </div>
              </div>
              {onAsk ? (
                <Button size="sm" onClick={() => onAsk(n)}>
                  <Glyph name="chat" size={13} /> Ask
                </Button>
              ) : (
                <span />
              )}
            </div>
          ))}
          {notes.length > 5 && <div className="sm-note-caption">+{notes.length - 5} more</div>}
        </div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------- map --- */

const KIND_FILTERS = [
  { value: "all", label: "All" },
  { value: "people", label: "People" },
  { value: "names", label: "Projects and names" },
  { value: "topics", label: "Topics" },
] as const;
type KindFilter = (typeof KIND_FILTERS)[number]["value"];

function MapTab({ range, revision }: { range: Range; revision: number }) {
  const [kind, setKind] = useState<KindFilter>("all");
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [graph, setGraph] = useState<ConceptGraph | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [extraction, setExtraction] = useState<{ done: number; total: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [focusedTheme, setFocusedTheme] = useState<string | null>(null);

  useEffect(() => {
    const t = window.setTimeout(() => setDebounced(query.trim()), 250);
    return () => window.clearTimeout(t);
  }, [query]);

  useEvent<{ done: number; total: number }>("semantic-graph-progress", (p) =>
    setExtraction(p.done >= p.total ? null : p),
  );

  useEffect(() => {
    let live = true;
    semantic
      .conceptGraph(range, kind, debounced)
      .then((g) => {
        if (!live) return;
        setGraph(g);
        setError(null);
        setExtraction(null);
        setSelected((s) => (s && g.nodes.some((n) => n.id === s) ? s : null));
      })
      .catch((e) => live && setError(`Couldn't build the map: ${e}`));
    return () => {
      live = false;
    };
  }, [range, kind, debounced, revision]);

  const model = useMemo(() => {
    if (!graph) return null;
    const maxSalience = Math.max(0.0001, ...graph.nodes.map((n) => n.salience));
    return {
      nodes: graph.nodes.map((n) => ({
        id: n.id,
        title: n.title,
        color: themeColor(n.theme_id, graph),
        size: Math.sqrt(n.salience / maxSalience),
        group: n.theme_id,
      })),
      edges: graph.edges.map((e) => ({ source: e.source_id, target: e.target_id, strength: e.strength })),
    };
  }, [graph]);

  const highlighted = useMemo(
    () => new Set(graph?.themes.find((t) => t.id === focusedTheme)?.concept_ids ?? []),
    [graph, focusedTheme],
  );

  return (
    <>
      <div className="sm-controls">
        <Segmented value={kind} options={KIND_FILTERS.map((k) => ({ value: k.value, label: k.label }))} onChange={setKind} />
        <label className="sm-search">
          <Icon name="search" size={14} />
          <input
            className="text-input"
            placeholder="Search concepts"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search concepts"
          />
        </label>
        {graph && graph.nodes.length > 0 && (
          <span className="sm-controls-count">
            {graph.nodes.length} concepts from {plural(graph.dictation_count, "dictation")}
          </span>
        )}
      </div>

      {error ? (
        <div className="sm-banner sm-banner-warn">
          <Icon name="alert" size={18} />
          <div className="sm-banner-body">{error}</div>
        </div>
      ) : !graph ? (
        <div className="card sm-empty-card">
          {extraction ? (
            <>
              <span className="sm-progress">
                <span style={{ width: `${Math.round((extraction.done / Math.max(1, extraction.total)) * 100)}%` }} />
              </span>
              <span className="mono">
                Reading your dictations for the first time · {extraction.done} of {extraction.total}
              </span>
            </>
          ) : (
            <>
              <Spinner /> Building your map…
            </>
          )}
        </div>
      ) : graph.nodes.length === 0 ? (
        <div className="card sm-empty-card">
          {debounced
            ? `Nothing on the map matches “${debounced}”.`
            : "Not enough dictation in this range to map yet. Pick a longer range."}
        </div>
      ) : (
        <>
          {graph.highlights.length > 0 && (
            <div className="sm-highlights">
              {graph.highlights.map((h) => (
                <button
                  key={`${h.kind}:${h.concept_ids.join(",")}`}
                  className="sm-highlight"
                  title="Show on the map"
                  onClick={() => {
                    setFocusedTheme(null);
                    setSelected(h.concept_ids[0] ?? null);
                  }}
                >
                  <span className="sm-highlight-title">
                    <Glyph name={HIGHLIGHT_GLYPH[h.kind]} size={14} />
                    {h.title}
                  </span>
                  <span className="sm-highlight-detail">{h.detail}</span>
                </button>
              ))}
            </div>
          )}
          <div className="card sm-map">
            <div style={{ position: "relative", minHeight: 0 }}>
              {model && (
                <ConceptCanvas model={model} selected={selected} onSelect={setSelected} highlighted={highlighted} />
              )}
              <span className="sm-canvas-legend">Color is theme · size is how often it comes up</span>
            </div>
            <aside className="sm-map-side">
              {selected && graph.nodes.some((n) => n.id === selected) ? (
                <ConceptDetailPanel range={range} id={selected} graph={graph} onSelect={setSelected} />
              ) : (
                <ThemeList graph={graph} focused={focusedTheme} onFocus={setFocusedTheme} />
              )}
            </aside>
          </div>
        </>
      )}
    </>
  );
}

function ThemeList({
  graph,
  focused,
  onFocus,
}: {
  graph: ConceptGraph;
  focused: string | null;
  onFocus: (id: string | null) => void;
}) {
  const titleOf = (id: string) => graph.nodes.find((n) => n.id === id)?.title;
  return (
    <>
      <div className="sm-side-title">
        Themes <Hint text="Groups of concepts that come up in the same working sessions more than chance would suggest." />
      </div>
      {graph.themes.length === 0 && <div className="sm-footnote">No clear themes in this range yet.</div>}
      <div className="sm-themes">
        {graph.themes.slice(0, 10).map((t) => (
          <button
            key={t.id}
            className={`sm-theme ${focused === t.id ? "active" : ""}`}
            onClick={() => onFocus(focused === t.id ? null : t.id)}
          >
            <span className="sm-theme-title">
              <span className="sm-dot" style={{ background: themeColor(t.id, graph) }} />
              {t.title}
            </span>
            <span className="sm-theme-members">
              {t.concept_ids
                .slice(0, 5)
                .map(titleOf)
                .filter(Boolean)
                .join(", ")}
            </span>
          </button>
        ))}
      </div>
      <div className="sm-footnote">Click a concept on the map to see what you said about it.</div>
    </>
  );
}

function ConceptDetailPanel({
  range,
  id,
  graph,
  onSelect,
}: {
  range: Range;
  id: string;
  graph: ConceptGraph;
  onSelect: (id: string | null) => void;
}) {
  const node = graph.nodes.find((n) => n.id === id)!;
  const [detail, setDetail] = useState<ConceptDetail | null>(null);
  const [brief, setBrief] = useState<string | null>(null);
  const [briefError, setBriefError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);

  useEffect(() => {
    let live = true;
    setDetail(null);
    setBrief(null);
    setBriefError(null);
    semantic
      .conceptDetail(range, id)
      .then((d) => live && setDetail(d))
      .catch(() => live && setDetail(null));
    return () => {
      live = false;
    };
  }, [range, id]);

  const writeBrief = () => {
    setWriting(true);
    setBriefError(null);
    semantic
      .conceptBrief(range, id)
      .then(setBrief)
      .catch((e) => setBriefError(String(e)))
      .finally(() => setWriting(false));
  };

  const related =
    detail?.related ??
    graph.edges
      .filter((e) => e.source_id === id || e.target_id === id)
      .map((e) => {
        const other = graph.nodes.find((n) => n.id === (e.source_id === id ? e.target_id : e.source_id));
        return other
          ? { id: other.id, title: other.title, kind: other.kind, shared_sessions: e.shared_sessions, strength: e.strength }
          : null;
      })
      .filter((x): x is NonNullable<typeof x> => x !== null)
      .sort((a, b) => b.shared_sessions - a.shared_sessions || b.strength - a.strength)
      .slice(0, 6);

  return (
    <div className="sm-detail">
      <div className="sm-detail-top">
        <span className="sm-dot" style={{ background: KIND_COLOR[node.kind] }} />
        {node.kind_label}
        {node.trend_label && <span className="sm-chip sm-chip-accent">{node.trend_label}</span>}
        <IconButton name="x" label="Back to themes" onClick={() => onSelect(null)} />
      </div>
      <div>
        <div className="sm-detail-title">{node.title}</div>
        <div className="sm-detail-meta">
          {node.mention_count} dictations on {plural(node.day_count, "day")} · since {monthDay(node.first_seen)}
        </div>
        <div className="sm-detail-meta">Last came up {relative(node.last_seen)}</div>
      </div>
      {node.apps.length > 0 && (
        <div>
          <div className="sm-section-label">WHERE IT COMES UP</div>
          <div className="small">
            {node.apps
              .slice(0, 3)
              .map((a) => `${a.app_name} ${pct(a.share)}`)
              .join(" · ")}
          </div>
        </div>
      )}
      {related.length > 0 && (
        <div>
          <div className="sm-section-label">COMES UP WITH</div>
          <div className="sm-related">
            {related.map((r) => (
              <button key={r.id} onClick={() => onSelect(r.id)} title={`In the same working session ${r.shared_sessions} times`}>
                <span className="sm-dot" style={{ background: KIND_COLOR[r.kind], width: 6, height: 6 }} />
                <span>{r.title}</span>
                <span>{r.shared_sessions}×</span>
              </button>
            ))}
          </div>
        </div>
      )}
      <div>
        <div className="sm-section-label">WHAT YOU SAID</div>
        {!detail ? (
          <Spinner size={13} />
        ) : (
          <div className="sm-mentions">
            {detail.mentions.map((m) => (
              <div key={m.dictation_id}>
                <div className="sm-mention-text">{m.excerpt}</div>
                <div className="sm-mention-meta">{[monthDay(m.date), m.app_name].filter(Boolean).join(" · ")}</div>
              </div>
            ))}
          </div>
        )}
      </div>
      <div>
        <div className="sm-section-label">SUMMARY</div>
        {brief ? (
          <div className="sm-brief">{brief}</div>
        ) : (
          <>
            <Button
              size="sm"
              loading={writing}
              onClick={writeBrief}
              title="A local model reads your recent mentions and says what this is to you. Nothing leaves your computer."
            >
              <Glyph name="quote" size={13} /> {writing ? "Summarizing…" : "Summarize with local AI"}
            </Button>
            {briefError && <div className="sm-brief-error">{briefError}</div>}
          </>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------- canvas + layout --- */

type CanvasNode = { id: string; title: string; color: string; size: number; group: string | null };
type CanvasEdge = { source: string; target: string; strength: number };
type LaidNode = CanvasNode & { x: number; y: number; r: number };
type Layout = { nodes: LaidNode[]; edges: CanvasEdge[]; neighbors: Map<string, Set<string>>; bounds: { x: number; y: number; w: number; h: number } };

function djb2(s: string) {
  let h = 5381n;
  for (const ch of s) h = (h * 33n + BigInt(ch.codePointAt(0) ?? 0)) & 0xffffffffffffffffn;
  return h;
}

/** Force-directed, deterministic for a given model (Mac GraphCanvasLayoutEngine). */
export function buildLayout(nodes: CanvasNode[], edges: CanvasEdge[]): Layout {
  if (nodes.length === 0) return { nodes: [], edges: [], neighbors: new Map(), bounds: { x: 0, y: 0, w: 1, h: 1 } };
  const groups = [...new Set(nodes.map((n) => n.group).filter((g): g is string => !!g))].sort();
  const groupIndex = new Map(groups.map((g, i) => [g, i]));
  const index = new Map(nodes.map((n, i) => [n.id, i]));
  const bodies = nodes.map((n) => {
    const radius = 6 + n.size * 15;
    const h = djb2(n.id);
    const jx = Number(h % 41n) - 20;
    const jy = Number((h / 41n) % 41n) - 20;
    const g = n.group ? groupIndex.get(n.group) : undefined;
    if (g !== undefined) {
      const angle = (g / Math.max(1, groups.length)) * 2 * Math.PI;
      return { x: Math.cos(angle) * 240 + jx * 3, y: Math.sin(angle) * 240 + jy * 3, vx: 0, vy: 0, radius, group: g as number | null };
    }
    const angle = (Number(h % 360n) / 180) * Math.PI;
    return { x: Math.cos(angle) * 280 + jx, y: Math.sin(angle) * 280 + jy, vx: 0, vy: 0, radius, group: null as number | null };
  });
  const links = edges
    .map((e) => {
      const a = index.get(e.source);
      const b = index.get(e.target);
      return a === undefined || b === undefined ? null : ([a, b, Math.max(0.05, Math.min(1, e.strength))] as const);
    })
    .filter((l): l is readonly [number, number, number] => l !== null);
  const iterations = bodies.length > 120 ? 240 : 320;
  for (let it = 0; it < iterations; it++) {
    for (let i = 0; i < bodies.length; i++) {
      for (let j = i + 1; j < bodies.length; j++) {
        let dx = bodies[j].x - bodies[i].x;
        let dy = bodies[j].y - bodies[i].y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 0.01) {
          dx = 1;
          dy = 1;
          d2 = 2;
        }
        const dist = Math.sqrt(d2);
        const minimum = bodies[i].radius + bodies[j].radius + 22;
        const force = 900 / Math.max(40, d2) + (dist < minimum ? (minimum - dist) * 0.03 : 0);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        bodies[i].vx -= fx;
        bodies[i].vy -= fy;
        bodies[j].vx += fx;
        bodies[j].vy += fy;
      }
    }
    for (const [a, b, strength] of links) {
      const dx = bodies[b].x - bodies[a].x;
      const dy = bodies[b].y - bodies[a].y;
      const dist = Math.max(1, Math.sqrt(dx * dx + dy * dy));
      const same = bodies[a].group !== null && bodies[a].group === bodies[b].group;
      const target = (same ? 60 : 130) + (1 - strength) * 80;
      const force = (dist - target) * (0.008 + strength * 0.02) * (same ? 1 : 0.35);
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      bodies[a].vx += fx;
      bodies[a].vy += fy;
      bodies[b].vx -= fx;
      bodies[b].vy -= fy;
    }
    const centers = new Map<number, [number, number, number]>();
    for (const body of bodies) {
      if (body.group === null) continue;
      const c = centers.get(body.group) ?? [0, 0, 0];
      centers.set(body.group, [c[0] + body.x, c[1] + body.y, c[2] + 1]);
    }
    for (const body of bodies) {
      const c = body.group === null ? undefined : centers.get(body.group);
      if (c) {
        body.vx += (c[0] / c[2] - body.x) * 0.018;
        body.vy += (c[1] / c[2] - body.y) * 0.018;
      }
      body.vx -= body.x * 0.0025;
      body.vy -= body.y * 0.0025;
      body.vx = Math.max(-12, Math.min(12, body.vx * 0.84));
      body.vy = Math.max(-12, Math.min(12, body.vy * 0.84));
      body.x += body.vx;
      body.y += body.vy;
    }
  }
  const laid = nodes.map((n, i) => ({ ...n, x: bodies[i].x, y: bodies[i].y, r: bodies[i].radius }));
  const neighbors = new Map<string, Set<string>>();
  for (const e of edges) {
    if (!neighbors.has(e.source)) neighbors.set(e.source, new Set());
    if (!neighbors.has(e.target)) neighbors.set(e.target, new Set());
    neighbors.get(e.source)!.add(e.target);
    neighbors.get(e.target)!.add(e.source);
  }
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of laid) {
    minX = Math.min(minX, n.x - n.r);
    maxX = Math.max(maxX, n.x + n.r);
    minY = Math.min(minY, n.y - n.r);
    maxY = Math.max(maxY, n.y + n.r + 18);
  }
  return { nodes: laid, edges, neighbors, bounds: { x: minX, y: minY, w: Math.max(1, maxX - minX), h: Math.max(1, maxY - minY) } };
}

const MIN_SCALE = 0.28;
const MAX_SCALE = 4.2;
type Viewport = { scale: number; panX: number; panY: number };

function fitViewport(b: Layout["bounds"], w: number, h: number): Viewport {
  if (w <= 1 || h <= 1 || b.w <= 1 || b.h <= 1) return { scale: 1, panX: 0, panY: 0 };
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, Math.min((w - 72) / b.w, (h - 72) / b.h)));
  return { scale, panX: -(b.x + b.w / 2) * scale, panY: -(b.y + b.h / 2) * scale };
}

function ConceptCanvas({
  model,
  selected,
  onSelect,
  highlighted,
}: {
  model: { nodes: CanvasNode[]; edges: CanvasEdge[] };
  selected: string | null;
  onSelect: (id: string | null) => void;
  highlighted: Set<string>;
}) {
  const wrap = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<Viewport>({ scale: 1, panX: 0, panY: 0 });
  const [hovered, setHovered] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ x: number; y: number; panX: number; panY: number; moved: boolean } | null>(null);
  const signature = useMemo(
    () => model.nodes.map((n) => `${n.id}:${n.size.toFixed(3)}:${n.group ?? ""}`).join("|") + "#" + model.edges.map((e) => `${e.source}>${e.target}`).join("|"),
    [model],
  );
  const layout = useMemo(() => buildLayout(model.nodes, model.edges), [signature]); // eslint-disable-line react-hooks/exhaustive-deps
  // Colors can change (theme order) without moving anything.
  const colorById = useMemo(() => new Map(model.nodes.map((n) => [n.id, n.color])), [model]);

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (size.w > 1 && size.h > 1) setView(fitViewport(layout.bounds, size.w, size.h));
    setHovered(null);
  }, [layout, size.w > 1 && size.h > 1]); // eslint-disable-line react-hooks/exhaustive-deps

  const zoomAround = useCallback(
    (factor: number, sx: number, sy: number) =>
      setView((v) => {
        const next = Math.min(MAX_SCALE, Math.max(MIN_SCALE, v.scale * factor));
        if (next === v.scale || size.w <= 1) return v;
        const wx = (sx - size.w / 2 - v.panX) / v.scale;
        const wy = (sy - size.h / 2 - v.panY) / v.scale;
        return { scale: next, panX: sx - size.w / 2 - wx * next, panY: sy - size.h / 2 - wy * next };
      }),
    [size],
  );

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      zoomAround(Math.exp(-e.deltaY * 0.0015), e.clientX - rect.left, e.clientY - rect.top);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomAround]);

  const focusId = hovered ?? selected;
  const focus = useMemo(() => {
    if (focusId) return new Set([focusId, ...(layout.neighbors.get(focusId) ?? [])]);
    return highlighted;
  }, [focusId, layout, highlighted]);

  const project = (x: number, y: number) => ({
    x: size.w / 2 + x * view.scale + view.panX,
    y: size.h / 2 + y * view.scale + view.panY,
  });
  const radius = (r: number) => Math.max(4, Math.min(r * Math.pow(view.scale, 0.55), r * 1.8));
  const labelBudget = view.scale < 0.58 ? 12 : view.scale < 1.05 ? 24 : view.scale < 1.85 ? 48 : Infinity;
  const byId = new Map(layout.nodes.map((n) => [n.id, n]));

  // Labels: focus first, then the biggest; skip any that would overlap.
  const labelled = new Set([...layout.nodes].sort((a, b) => b.size - a.size).slice(0, labelBudget).map((n) => n.id));
  const placed: { x: number; y: number; w: number; h: number }[] = [];
  const labels: { id: string; x: number; y: number; text: string; size: number; dim: boolean }[] = [];
  const candidates = [...layout.nodes].sort((a, b) => {
    const fa = a.id === focusId ? 1 : 0;
    const fb = b.id === focusId ? 1 : 0;
    return fb - fa || b.size - a.size;
  });
  for (const n of candidates) {
    const isFocus = n.id === focusId;
    const related = focus.size === 0 || focus.has(n.id);
    if (!(isFocus || (related && (focus.size > 0 || labelled.has(n.id))))) continue;
    const p = project(n.x, n.y);
    const fs = isFocus ? 12.5 : Math.max(10, Math.min(12.5, 10 + view.scale * 1.2));
    const w = n.title.length * fs * 0.56 + 6;
    const rect = { x: p.x - w / 2, y: p.y + radius(n.r) + 2, w, h: fs + 4 };
    if (rect.x > size.w + 80 || rect.x + rect.w < -80 || rect.y > size.h + 80 || rect.y < -80) continue;
    if (!isFocus && placed.some((o) => rect.x < o.x + o.w && rect.x + rect.w > o.x && rect.y < o.y + o.h && rect.y + rect.h > o.y)) continue;
    placed.push(rect);
    labels.push({ id: n.id, x: p.x, y: rect.y, text: n.title, size: fs, dim: !related });
  }

  const onPointerDown = (e: RPointerEvent) => {
    drag.current = { x: e.clientX, y: e.clientY, panX: view.panX, panY: view.panY, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: RPointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (!d.moved && Math.hypot(dx, dy) < 4) return;
    d.moved = true;
    setDragging(true);
    setView((v) => ({ ...v, panX: d.panX + dx, panY: d.panY + dy }));
  };
  const onPointerUp = (e: RPointerEvent) => {
    const d = drag.current;
    drag.current = null;
    setDragging(false);
    if (d && !d.moved) {
      const target = (e.target as Element).closest?.("[data-node]");
      const id = target?.getAttribute("data-node") ?? null;
      onSelect(id && id === selected ? null : id);
    }
  };

  const center = () => ({ x: size.w / 2, y: size.h / 2 });
  return (
    <div
      ref={wrap}
      className={`sm-canvas ${dragging ? "dragging" : ""}`}
      style={{ position: "absolute", inset: 0 }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerLeave={() => setHovered(null)}
    >
      {size.w > 1 && (
        <svg role="img" aria-label="Concept map">
          <g>
            {layout.edges.map((e) => {
              const a = byId.get(e.source);
              const b = byId.get(e.target);
              if (!a || !b) return null;
              const pa = project(a.x, a.y);
              const pb = project(b.x, b.y);
              const touches = !!focusId && (e.source === focusId || e.target === focusId);
              const inFocus = focus.size === 0 || (focus.has(e.source) && focus.has(e.target));
              const opacity = touches ? 0.35 + e.strength * 0.5 : inFocus ? 0.1 + e.strength * 0.35 : 0.03;
              return (
                <line
                  key={`${e.source}|${e.target}`}
                  className="sm-edge"
                  x1={pa.x}
                  y1={pa.y}
                  x2={pb.x}
                  y2={pb.y}
                  style={{ strokeOpacity: opacity, strokeWidth: touches ? 1.2 + e.strength * 1.6 : 0.6 + e.strength * 1.2 }}
                />
              );
            })}
          </g>
          <g>
            {[...layout.nodes]
              .sort((a, b) => a.size - b.size)
              .map((n) => {
                const p = project(n.x, n.y);
                const related = focus.size === 0 || focus.has(n.id);
                return (
                  <g
                    key={n.id}
                    data-node={n.id}
                    className={`sm-node ${n.id === focusId ? "focus" : ""}`}
                    onPointerEnter={() => setHovered(n.id)}
                    onPointerLeave={() => setHovered((h) => (h === n.id ? null : h))}
                  >
                    <title>{n.title}</title>
                    <circle cx={p.x} cy={p.y} r={radius(n.r)} style={{ fill: colorById.get(n.id) ?? n.color, fillOpacity: related ? 0.92 : 0.18 }} />
                  </g>
                );
              })}
          </g>
          <g>
            {labels.map((l) => (
              <text
                key={l.id}
                className={`sm-label ${l.dim ? "dim" : ""}`}
                x={l.x}
                y={l.y}
                textAnchor="middle"
                dominantBaseline="hanging"
                style={{ fontSize: l.size, fontWeight: l.id === focusId ? 650 : 500 }}
              >
                {l.text}
              </text>
            ))}
          </g>
        </svg>
      )}
      <div className="sm-canvas-tools" onPointerDown={(e) => e.stopPropagation()}>
        <button title="Zoom out" aria-label="Zoom out" onClick={() => zoomAround(0.78, center().x, center().y)}>
          <Glyph name="zoomOut" size={14} />
        </button>
        <button title="Zoom in" aria-label="Zoom in" onClick={() => zoomAround(1.28, center().x, center().y)}>
          <Glyph name="zoomIn" size={14} />
        </button>
        <button title="Fit the map" aria-label="Fit the map" onClick={() => setView(fitViewport(layout.bounds, size.w, size.h))}>
          <Glyph name="fit" size={14} />
        </button>
      </div>
    </div>
  );
}

/* -------------------------------------------------------------- insights --- */

function InsightsTab({
  settings,
  llm,
  status,
  running,
  revision,
  onMessage,
  refreshStatus,
}: {
  settings: Settings;
  llm: LlmStatus | null;
  status: IndexStatus | null;
  running: boolean;
  revision: number;
  onMessage: (m: { tone: "ok" | "error"; text: string } | null) => void;
  refreshStatus: () => void;
}) {
  const [stored, setStored] = useState<StoredReport | null | undefined>(undefined);
  const [loops, setLoops] = useState<InsightFinding[]>([]);
  const [findings, setFindings] = useState<InsightFinding[]>([]);
  const [signals, setSignals] = useState<SignalsOverview | null>(null);
  const [generating, setGenerating] = useState(false);
  const toast = useToast();

  const load = useCallback(() => {
    semantic.insightReport().then(setStored).catch(() => setStored(null));
    semantic.openLoops().then(setLoops).catch(() => setLoops([]));
    semantic.findings().then(setFindings).catch(() => setFindings([]));
    semantic.signals(60).then(setSignals).catch(() => setSignals(null));
  }, []);
  useEffect(load, [load, revision]);

  const providerName = settings.llm_provider === "lm_studio" ? "LM Studio" : "Ollama";
  const model = settings.semantic_summary_model;
  const phrased = settings.semantic_summary_enabled && !!model;

  const generate = () => {
    setGenerating(true);
    onMessage(null);
    semantic
      .generateInsights()
      .then((r) => {
        setStored({ report: r, stale: false });
        semantic.openLoops().then(setLoops).catch(() => {});
        semantic.findings().then(setFindings).catch(() => {});
        const n = r.cards.length;
        if (n === 0) onMessage({ tone: "ok", text: "Build more semantic memory before insights can be generated." });
        else if (r.summary_model_name)
          onMessage({ tone: "ok", text: `Generated ${plural(n, "graph insight")} with ${r.summary_model_name}.` });
        else {
          const reason = r.coverage_notes.find((c) => c.startsWith("Local model fallback:"));
          onMessage({ tone: reason ? "error" : "ok", text: reason ?? `Generated ${plural(n, "heuristic graph insight")}.` });
        }
      })
      .catch((e) => onMessage({ tone: "error", text: String(e) }))
      .finally(() => setGenerating(false));
  };

  const setStatus = (f: InsightFinding, s: "resolved" | "dismissed") =>
    semantic
      .setFindingStatus(f.id, s)
      .then(() => {
        setLoops((ls) => ls.filter((x) => x.id !== f.id));
        setFindings((fs) => fs.filter((x) => x.id !== f.id));
        toast(s === "resolved" ? "Marked as done." : "Dismissed.", "success");
      })
      .catch((e) => toast(String(e), "error"));

  const modelLine = (
    <span className="sm-model-line">
      <Glyph name="cpu" size={13} />
      {phrased ? `Phrased by ${model} on ${providerName}` : "Plain phrasing, no model"}
      <button className="sm-link" onClick={openSettings}>
        Change
      </button>
    </span>
  );

  const graphEmpty = (status?.node_count ?? 0) === 0;
  const report = stored?.report;
  return (
    <>
      <SemanticIndexCard status={status} running={running} llm={llm} onChanged={refreshStatus} />
      {graphEmpty ? (
        <div className="card sm-ready">
          <div className="sm-ready-head">
            <div className="sm-icon-tile">
              <Glyph name="brain" size={20} />
            </div>
            <div>
              <h2>Your memory graph is still empty</h2>
              <p>It fills in as Orttaai indexes your dictations. Index now to build it from everything you've dictated so far.</p>
            </div>
          </div>
          <div className="sm-ready-foot">
            <Button variant="primary" icon="refresh" loading={running} onClick={() => semantic.indexNow().then(refreshStatus)}>
              {running ? "Indexing…" : "Index Now"}
            </Button>
          </div>
        </div>
      ) : generating && !report ? (
        <div className="card sm-spinner-row">
          <Spinner size={20} />
          <div>
            <strong>Reading {plural(status?.chunk_count ?? 0, "note")} and how they connect</strong>
            <span className="muted small">{phrased ? `${model} on ${providerName} phrases what it finds` : "Plain phrasing, no model"}</span>
          </div>
        </div>
      ) : report ? (
        <ReportView
          stored={stored!}
          loops={loops}
          generating={generating}
          onGenerate={generate}
          onResolve={(f) => setStatus(f, "resolved")}
          onDismiss={(f) => setStatus(f, "dismissed")}
        />
      ) : stored === undefined ? (
        <div className="card sm-spinner-row">
          <Spinner /> Loading insights…
        </div>
      ) : (
        <div className="card sm-ready">
          <div className="sm-ready-head">
            <div className="sm-icon-tile">
              <Glyph name="brain" size={20} />
            </div>
            <div>
              <h2>See what your notes add up to</h2>
              <p>
                Orttaai reads how your notes connect across topics, people and projects, then points out patterns, open loops and
                what's changing. Every insight links back to what you said.
              </p>
            </div>
          </div>
          <div className="sm-metrics">
            <span className="sm-chip">
              <strong>{(status?.chunk_count ?? 0).toLocaleString()}</strong> notes
            </span>
            <span className="sm-chip">
              <strong>{(status?.node_count ?? 0).toLocaleString()}</strong> concepts
            </span>
            <span className="sm-chip">
              <strong>{(status?.edge_count ?? 0).toLocaleString()}</strong> links
            </span>
          </div>
          <div className="sm-ready-foot">
            {modelLine}
            <Button variant="primary" loading={generating} onClick={generate}>
              <Glyph name="brain" size={15} /> Generate Insights
            </Button>
          </div>
        </div>
      )}
      {!report && !graphEmpty && (
        <>
          {loops.length > 0 && <OpenLoops loops={loops} onResolve={(f) => setStatus(f, "resolved")} onDismiss={(f) => setStatus(f, "dismissed")} />}
          <FindingsList findings={findings.filter((f) => f.kind !== "open_commitment" && f.kind !== "open_question")} onDismiss={(f) => setStatus(f, "dismissed")} />
        </>
      )}
      {signals && !graphEmpty && <SignalsSection signals={signals} />}
    </>
  );
}

/** Index state + Index Now / Rebuild / Clear (Mac Model › AI Features controls). */
export function SemanticIndexCard({
  status,
  running,
  llm,
  onChanged,
}: {
  status: IndexStatus | null;
  running: boolean;
  llm: LlmStatus | null;
  onChanged: () => void;
}) {
  const toast = useToast();
  const [confirm, setConfirm] = useState(false);
  if (!status) return null;
  const indexed = status.indexed_transcription_count;
  const total = status.transcription_count;
  return (
    <div className="card sm-card">
      <div className="sm-card-head">
        <span className="sm-card-title">Semantic index</span>
        <Hint text="Your dictations split into short passages and embedded on this computer, so Chat AI can search by meaning and the graph can link related notes." />
        <span className="sm-card-count">
          {indexed.toLocaleString()} of {plural(total, "dictation")} indexed
        </span>
      </div>
      <div className="sm-metrics">
        <span className="sm-chip">
          <strong>{status.embedded_chunk_count.toLocaleString()}</strong> chunks
        </span>
        <span className="sm-chip">
          <strong>{status.signal_count.toLocaleString()}</strong> signals
        </span>
        <span className={`sm-chip ${status.using_fallback ? "sm-chip-warn" : ""}`}>
          <Glyph name="cpu" size={12} />
          {status.active_model || status.configured_model}
          {status.using_fallback ? " · keyword fallback" : ""}
        </span>
        {status.needs_reindex && <span className="sm-chip sm-chip-accent">Re-indexing for {status.configured_model}</span>}
        {status.pending > 0 && <span className="sm-chip">{plural(status.pending, "dictation")} queued</span>}
        {status.failed > 0 && (
          <span className="sm-chip sm-chip-warn" title={status.last_error ?? ""}>
            {plural(status.failed, "dictation")} failed
          </span>
        )}
        {status.latest_indexed_at && <span className="sm-chip">Updated {relative(status.latest_indexed_at)}</span>}
        {llm && !llm.reachable && <span className="sm-chip sm-chip-warn">Local model server offline</span>}
      </div>
      <div className="sm-index-actions">
        <Button
          size="sm"
          icon="refresh"
          loading={running}
          onClick={() =>
            semantic
              .indexNow()
              .then((n) => {
                toast(n > 0 ? `Indexing ${plural(n, "dictation")}…` : "Everything is already indexed.", "info");
                onChanged();
              })
              .catch((e) => toast(String(e), "error"))
          }
        >
          {running ? "Indexing…" : "Index Now"}
        </Button>
        <Button size="sm" onClick={() => semantic.rebuildGraph().then(() => toast("Rebuilding the graph…", "info"))}>
          <Glyph name="rotate" size={14} /> Rebuild Graph
        </Button>
        <Button size="sm" variant="ghost" icon="trash" onClick={() => setConfirm(true)} style={{ marginLeft: "auto" }}>
          Clear Index
        </Button>
      </div>
      <ConfirmDialog
        open={confirm}
        title="Clear the semantic index?"
        body="This removes the semantic memory: chunks, embeddings, signals, the graph, findings and session readings. Your dictations stay. You can index again any time."
        confirmLabel="Clear Index"
        onCancel={() => setConfirm(false)}
        onConfirm={() => {
          setConfirm(false);
          semantic
            .clearIndex()
            .then(() => {
              toast("Semantic index cleared.", "success");
              onChanged();
            })
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </div>
  );
}

function ageText(f: InsightFinding) {
  if (!f.window_start) return "recent";
  const days = Math.max(0, Math.floor((Date.now() / 1000 - f.window_start) / 86_400));
  return days === 0 ? "today" : days === 1 ? "1 day open" : `${days} days open`;
}
function ageClass(f: InsightFinding) {
  if (!f.window_start) return "";
  const days = (Date.now() / 1000 - f.window_start) / 86_400;
  return days >= 7 ? "old" : days >= 3 ? "warn" : "";
}

function OpenLoops({
  loops,
  onResolve,
  onDismiss,
}: {
  loops: InsightFinding[];
  onResolve: (f: InsightFinding) => void;
  onDismiss: (f: InsightFinding) => void;
}) {
  return (
    <div className="card sm-card">
      <div className="sm-card-head">
        <span style={{ color: "var(--accent)", display: "inline-flex" }}>
          <Glyph name="turn" size={15} />
        </span>
        <span className="sm-card-title">Open Loops &amp; Commitments</span>
        <span className="sm-card-count">{loops.length}</span>
      </div>
      <div className="sm-loops">
        {loops.map((l) => {
          const question = l.kind === "open_question";
          return (
            <div className="sm-loop" key={l.id}>
              <span style={{ color: question ? "var(--warn)" : "var(--accent)", display: "inline-flex" }}>
                <Glyph name={question ? "question" : "hand"} size={15} />
              </span>
              <div>
                <div className="sm-loop-text">{l.detail}</div>
                <div className={`sm-loop-age ${ageClass(l)}`}>{ageText(l)}</div>
              </div>
              <div className="sm-loop-actions">
                <button className="icon-btn ok" title="Mark as done" aria-label="Mark resolved" onClick={() => onResolve(l)}>
                  <Glyph name="checkCircle" size={16} />
                </button>
                <button className="icon-btn" title="Dismiss" aria-label="Dismiss" onClick={() => onDismiss(l)}>
                  <Glyph name="xCircle" size={16} />
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

const FINDING_LABEL: Record<string, string> = {
  life_area: "Life Area",
  rhythm: "Rhythm",
  emerging_theme: "Emerging",
  fading_theme: "Fading",
  resurfacing_theme: "Back Again",
  anomaly: "Today",
};

function FindingsList({ findings, onDismiss }: { findings: InsightFinding[]; onDismiss: (f: InsightFinding) => void }) {
  if (findings.length === 0) return null;
  return (
    <>
      <div className="sm-section-title">Patterns</div>
      <div className="sm-masonry">
        {findings.slice(0, 12).map((f) => (
          <div className="card sm-card sm-insight-card" key={f.id}>
            <div className="sm-insight-title-row">
              <div>
                <div className="sm-insight-kind">{FINDING_LABEL[f.kind] ?? f.kind}</div>
                <div className="sm-insight-title">{f.title}</div>
              </div>
              <span className="sm-chip">{pct(f.confidence)}</span>
              <button className="icon-btn" title="Dismiss" aria-label="Dismiss" onClick={() => onDismiss(f)}>
                <Icon name="x" size={14} />
              </button>
            </div>
            <div className="sm-insight-body">{f.detail}</div>
          </div>
        ))}
      </div>
    </>
  );
}

function caveat(r: StoredReport["report"]) {
  const parts = [`Computed from ${r.source_chunk_count} transcript segments`];
  parts.push(
    r.summary_model_name
      ? `phrased by ${r.summary_model_name}; claims remain evidence-linked`
      : "deterministic phrasing (local model offline)",
  );
  if (r.source_chunk_count < 40) parts.push("early days: patterns sharpen as you dictate more");
  return parts.join(" · ");
}

function ReportView({
  stored,
  loops,
  generating,
  onGenerate,
  onResolve,
  onDismiss,
}: {
  stored: StoredReport;
  loops: InsightFinding[];
  generating: boolean;
  onGenerate: () => void;
  onResolve: (f: InsightFinding) => void;
  onDismiss: (f: InsightFinding) => void;
}) {
  const r = stored.report;
  return (
    <>
      <div className="card sm-card">
        <div className="sm-card-head" style={{ alignItems: "center" }}>
          <span className="sm-card-title">Summary</span>
          <span style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 8 }}>
            {r.summary_model_name && <span className="sm-chip sm-chip-accent">{r.summary_model_name}</span>}
            <span className="muted small">{mediumDateTime(r.generated_at)}</span>
            <Button size="sm" loading={generating} onClick={onGenerate}>
              <Glyph name="brain" size={14} /> {generating ? "Generating…" : "Regenerate"}
            </Button>
          </span>
        </div>
        <div className="sm-summary-lines">
          {r.summary.map((line) => (
            <div className="sm-summary-line" key={line}>
              <Glyph name="checkCircle" size={12} />
              <span>{line}</span>
            </div>
          ))}
        </div>
        <div className="sm-metrics">
          <span className="sm-chip">
            <strong>{r.source_node_count}</strong> nodes
          </span>
          <span className="sm-chip">
            <strong>{r.source_edge_count}</strong> links
          </span>
          <span className="sm-chip">
            <strong>{r.source_chunk_count}</strong> chunks
          </span>
          <span className="sm-chip">
            <strong>{r.analyzer_name}</strong> {r.used_fallback ? "fallback" : "analyzer"}
          </span>
        </div>
      </div>
      <div className="sm-caveat">
        <Glyph name="shield" size={13} />
        {caveat(r)}
      </div>
      {stored.stale && (
        <div className="sm-banner sm-banner-accent">
          <Glyph name="rotate" size={18} />
          <div className="sm-banner-body">
            <div className="sm-banner-title">Snapshot is older than the current graph</div>
            <div className="sm-banner-text">Regenerate to update the snapshot.</div>
          </div>
        </div>
      )}
      {loops.length > 0 && <OpenLoops loops={loops} onResolve={onResolve} onDismiss={onDismiss} />}
      {r.charts.length > 0 && (
        <>
          <div className="sm-section-title">Life Graph Charts</div>
          <div className="sm-charts">
            {r.charts.slice(0, 3).map((c) => (
              <ChartCard key={c.id} chart={c} />
            ))}
          </div>
        </>
      )}
      {r.clusters.length > 0 && (
        <>
          <div className="sm-section-title">Life &amp; Work Areas</div>
          <div className="sm-clusters">
            {r.clusters.slice(0, 3).map((c) => (
              <div className="card sm-card sm-cluster" key={c.id}>
                <div className="sm-cluster-title">{c.title}</div>
                <div className="sm-cluster-summary">{c.summary}</div>
                {c.evidence[0] && <EvidenceRow e={c.evidence[0]} />}
              </div>
            ))}
          </div>
        </>
      )}
      {r.comparisons.length > 0 && (
        <>
          <div className="sm-section-title">Comparative Signals</div>
          {r.comparisons.slice(0, 3).map((c) => (
            <div className="card sm-card" key={c.id}>
              <div className="sm-compare-head">
                <span className="sm-card-title">{c.title}</span>
                <span className="sm-chip sm-chip-accent">{c.trend}</span>
              </div>
              <div className="sm-insight-body" style={{ marginBottom: 10 }}>
                {c.detail}
              </div>
              <div className="sm-evidence-list" style={{ borderTop: "none", paddingTop: 0 }}>
                {c.evidence.slice(0, 2).map((e) => (
                  <EvidenceRow key={e.id} e={e} />
                ))}
              </div>
            </div>
          ))}
        </>
      )}
      {r.coverage_notes.length > 0 && (
        <div className="card sm-card">
          <CardHead title="Coverage Notes" />
          <div className="sm-coverage">
            {r.coverage_notes.map((n) => (
              <div className="sm-coverage-row" key={n}>
                <Icon name="info" size={13} />
                <span>{n}</span>
              </div>
            ))}
          </div>
        </div>
      )}
      {r.cards.length === 0 ? (
        <div className="card sm-empty-card">Not enough connected evidence yet.</div>
      ) : (
        <div className="sm-masonry">
          {r.cards.map((c) => (
            <InsightCardView key={c.id} card={c} />
          ))}
        </div>
      )}
    </>
  );
}

function ChartCard({ chart }: { chart: InsightChart }) {
  const points = chart.points.slice(0, 8);
  const max = Math.max(1, ...points.map((p) => p.value));
  const colors = ["var(--accent)", "var(--ok)", "var(--warn)", "var(--sm-blue)", "var(--sm-purple)"];
  const first = points.find((p) => p.evidence.length > 0)?.evidence[0];
  return (
    <div className="card sm-card">
      <div className="sm-chart-head">
        <span className="sm-card-title">{chart.title}</span>
        <span className="sm-chip" style={{ marginLeft: "auto" }}>
          {chart.unit}
        </span>
      </div>
      <div className="sm-chart-sub">{chart.subtitle}</div>
      <div className="sm-bars">
        {points.map((p, i) => {
          const ratio = Math.max(0, Math.min(1, p.value / max));
          return (
            <div className="sm-bar" key={p.id}>
              <div className="sm-bar-head">
                <span>{p.label}</span>
                <span>{Number.isInteger(p.value) ? p.value : p.value.toFixed(1)}</span>
              </div>
              <svg viewBox="0 0 100 8" preserveAspectRatio="none" aria-hidden="true">
                <rect x="0" y="0" width="100" height="8" rx="4" style={{ fill: "var(--surface-3)" }} />
                <rect x="0" y="0" width={Math.max(6, ratio * 100)} height="8" rx="4" style={{ fill: colors[i % colors.length] }} />
              </svg>
              {p.detail && <div className="sm-bar-detail">{p.detail}</div>}
            </div>
          );
        })}
      </div>
      {first && (
        <div className="sm-evidence-list" style={{ marginTop: 10 }}>
          <EvidenceRow e={first} />
        </div>
      )}
    </div>
  );
}

function InsightCardView({ card }: { card: InsightCard }) {
  return (
    <div className="card sm-card sm-insight-card">
      <div className="sm-insight-title-row">
        <div>
          <div className="sm-insight-kind">{card.kind}</div>
          <div className="sm-insight-title">{card.title}</div>
        </div>
        <span className="sm-chip">{pct(card.confidence)}</span>
      </div>
      <div className="sm-insight-body">{card.body}</div>
      <div className="sm-insight-action">
        <Glyph name="turn" size={12} />
        <span>{card.action_text}</span>
      </div>
      {card.evidence.length > 0 && (
        <div className="sm-evidence-list">
          <div className="sm-section-label" style={{ marginBottom: 0 }}>
            EVIDENCE
          </div>
          {card.evidence.map((e) => (
            <EvidenceRow key={e.id} e={e} />
          ))}
        </div>
      )}
    </div>
  );
}

function EvidenceRow({ e }: { e: InsightEvidence }) {
  return (
    <div className="sm-evidence">
      <div className="sm-evidence-head">
        <span>{e.source_app_name || "Dictation"}</span>
        {e.source_created_at && <span>{mediumDateTime(e.source_created_at)}</span>}
      </div>
      <div className="sm-evidence-text">{e.excerpt}</div>
    </div>
  );
}

function SignalList({ title, glyph, items, empty }: { title: string; glyph: string; items: SignalWithContext[]; empty: string }): ReactNode {
  return (
    <div className="card sm-card">
      <div className="sm-card-head">
        <span style={{ color: "var(--accent)", display: "inline-flex" }}>
          <Glyph name={glyph} size={14} />
        </span>
        <span className="sm-card-title">{title}</span>
        <span className="sm-card-count">{items.length}</span>
      </div>
      {items.length === 0 ? (
        <div className="sm-quote">{empty}</div>
      ) : (
        <div className="sm-signal-list">
          {items.slice(0, 6).map((s) => (
            <div className="sm-signal" key={s.id}>
              <span>{s.value}</span>
              <span className="sm-signal-meta">
                {[monthDay(s.source_created_at), s.app_name, s.model_id === "heuristic-v1" ? null : `read by ${s.model_id}`]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function SignalsSection({ signals }: { signals: SignalsOverview }) {
  const tone = signals.mix.tone ?? {};
  const intent = signals.mix.intent ?? {};
  const chips = [
    ...Object.entries(intent).map(([k, v]) => ({ k: `intent-${k}`, label: k, v })),
    ...Object.entries(tone).map(([k, v]) => ({ k: `tone-${k}`, label: k, v })),
  ].sort((a, b) => b.v - a.v);
  return (
    <>
      <div className="sm-section-title">Signals · last 60 days</div>
      {chips.length > 0 && (
        <div className="sm-mix">
          {chips.map((c) => (
            <span className="sm-chip" key={c.k}>
              {c.label} <strong>{c.v}</strong>
            </span>
          ))}
        </div>
      )}
      <div className="sm-signals">
        <SignalList title="Commitments" glyph="hand" items={signals.commitments} empty="No commitments voiced." />
        <SignalList title="Questions" glyph="question" items={signals.questions} empty="No questions asked." />
        <SignalList title="Decisions" glyph="seal" items={signals.decisions} empty="No decisions stated." />
        <SignalList title="Blockers" glyph="alert" items={signals.blockers} empty="Nothing got in the way." />
      </div>
    </>
  );
}

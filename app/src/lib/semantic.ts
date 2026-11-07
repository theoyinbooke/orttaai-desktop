// Semantic memory / Memory Graph: typed IPC wrappers and hooks. DTOs mirror
// orttaai_core::semantic (snake_case, Unix-second timestamps).
import { useCallback, useEffect, useRef, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { call, useEvent } from "./api";

/* ------------------------------------------------------------------ types -- */

export type Range = "week" | "month" | "quarter" | "all";
export const RANGES: { value: Range; label: string }[] = [
  { value: "week", label: "7 days" },
  { value: "month", label: "30 days" },
  { value: "quarter", label: "90 days" },
  { value: "all", label: "All time" },
];
export const rangeDays = (r: Range): number | null =>
  r === "week" ? 7 : r === "month" ? 30 : r === "quarter" ? 90 : null;
/** "the last 30 days" / "all time". */
export const rangePhrase = (r: Range) => {
  const d = rangeDays(r);
  return d ? `the last ${d} days` : "all time";
};

export type IndexStatus = {
  enabled: boolean;
  auto_index: boolean;
  configured_model: string;
  active_model: string;
  needs_reindex: boolean;
  using_fallback: boolean;
  pending: number;
  failed: number;
  last_error: string | null;
  transcription_count: number;
  indexed_transcription_count: number;
  chunk_count: number;
  embedded_chunk_count: number;
  node_count: number;
  edge_count: number;
  signal_count: number;
  latest_indexed_at: number | null;
};

export type IndexProgress = { phase: string; done: number; total: number; message: string };

export type IndexRunResult = {
  source_count: number;
  chunk_count: number;
  embedded_count: number;
  skipped_count: number;
  failed_count: number;
  signal_count: number;
  refined_count: number;
  graph_node_count: number;
  graph_edge_count: number;
  finding_count: number;
  provider_name: string;
  model_id: string;
  used_fallback: boolean;
  remaining: number;
  error_message: string | null;
};

export type ActivityState = {
  state: "idle" | "checking" | "running" | "paused" | "finished" | "unavailable" | "";
  done: number;
  total: number;
  model: string | null;
  message: string | null;
};

export type SemanticStatus = {
  index: IndexStatus;
  running: boolean;
  progress: IndexProgress | null;
  last_run: IndexRunResult | null;
  activity: ActivityState;
};

export type LlmStatus = {
  reachable: boolean;
  provider: string;
  models: string[];
  embedding_models: string[];
  embedding_model_installed: boolean;
  summary_model_installed: boolean;
  error: string | null;
};

export type ConceptKind = "person" | "organization" | "place" | "product" | "topic";
export type ConceptTrend = "new" | "rising" | "steady" | "fading";

export type ConceptNode = {
  id: string;
  kind: ConceptKind;
  kind_label: string;
  title: string;
  mention_count: number;
  day_count: number;
  first_seen: number;
  last_seen: number;
  recent_count: number;
  previous_count: number;
  trend: ConceptTrend;
  trend_label: string | null;
  apps: { app_name: string; count: number; share: number }[];
  dictation_ids: number[];
  theme_id: string | null;
  salience: number;
};
export type ConceptEdge = { source_id: string; target_id: string; shared_sessions: number; strength: number };
export type ConceptTheme = { id: string; title: string; concept_ids: string[]; mention_count: number; last_seen: number };
export type HighlightKind = "rising" | "new" | "pair" | "person" | "fading" | "focus";
export type ConceptHighlight = { kind: HighlightKind; title: string; detail: string; concept_ids: string[] };
export type ConceptGraph = {
  range: Range;
  dictation_count: number;
  nodes: ConceptNode[];
  edges: ConceptEdge[];
  themes: ConceptTheme[];
  highlights: ConceptHighlight[];
  aliases: Record<string, string>;
  generated_at: number;
};
export type ConceptMention = { dictation_id: number; date: number; app_name: string | null; excerpt: string };
export type RelatedConcept = { id: string; title: string; kind: ConceptKind; shared_sessions: number; strength: number };
export type ConceptDetail = { node: ConceptNode; related: RelatedConcept[]; mentions: ConceptMention[] };

export type WorkActivity =
  | "building"
  | "fixing"
  | "designing"
  | "planning"
  | "researching"
  | "writing"
  | "communicating"
  | "setup"
  | "reviewing"
  | "other";
export type ActivityShare = {
  activity: WorkActivity;
  title: string;
  sessions: number;
  share: number;
  previous_share: number | null;
  change: number | null;
};
export type SessionNote = {
  id: string;
  text: string;
  date: number;
  app_name: string | null;
  project: string | null;
  is_estimate: boolean;
};
export type FrictionGroup = { type: string; title: string; sessions: number; examples: SessionNote[] };
export type ProjectBreakdown = {
  id: string;
  title: string;
  sessions: number;
  last_active: number;
  activities: ActivityShare[];
  friction_sessions: number;
  top_friction: string | null;
  top_friction_title: string | null;
  latest_goal: string | null;
};
export type ActivityReport = {
  range: Range;
  session_count: number;
  day_count: number;
  model_analyzed_count: number;
  headline: string[];
  activities: ActivityShare[];
  friction_rate: number;
  frictions: FrictionGroup[];
  projects: ProjectBreakdown[];
  decisions: SessionNote[];
  open_questions: SessionNote[];
};

export type InsightEvidence = {
  id: string;
  chunk_id: number;
  title: string;
  excerpt: string;
  source_app_name: string | null;
  source_created_at: number | null;
  score: number;
};
export type InsightCard = {
  id: string;
  kind: string;
  title: string;
  body: string;
  confidence: number;
  action_text: string;
  related_node_ids: string[];
  evidence: InsightEvidence[];
};
export type InsightCluster = { id: string; title: string; summary: string; related_node_ids: string[]; evidence: InsightEvidence[] };
export type InsightComparison = { id: string; title: string; detail: string; trend: string; evidence: InsightEvidence[] };
export type InsightChartPoint = { id: string; label: string; value: number; detail: string | null; evidence: InsightEvidence[] };
export type InsightChart = { id: string; title: string; subtitle: string; kind: string; unit: string; points: InsightChartPoint[] };
export type InsightReport = {
  generated_at: number;
  graph_signature: string;
  analyzer_name: string;
  used_fallback: boolean;
  summary: string[];
  summary_model_name: string | null;
  clusters: InsightCluster[];
  comparisons: InsightComparison[];
  coverage_notes: string[];
  charts: InsightChart[];
  cards: InsightCard[];
  source_node_count: number;
  source_edge_count: number;
  source_chunk_count: number;
};
export type StoredReport = { report: InsightReport; stale: boolean };

export type InsightFinding = {
  id: number;
  kind: string;
  subject_key: string;
  title: string;
  detail: string;
  magnitude: number;
  confidence: number;
  window_start: number | null;
  window_end: number | null;
  evidence_chunk_ids: number[];
  first_seen_at: number;
  last_computed_at: number;
  last_shown_at: number | null;
  status: "active" | "expired" | "resolved" | "dismissed";
};
export type RankedFinding = InsightFinding & { label: string; action: string; score: number };

export type SignalWithContext = {
  id: number;
  chunk_id: number;
  family: string;
  value: string;
  confidence: number;
  model_id: string;
  extracted_at: number;
  chunk_text: string;
  source_created_at: number;
  app_name: string | null;
  transcription_id: number;
};
export type SignalsOverview = {
  commitments: SignalWithContext[];
  questions: SignalWithContext[];
  decisions: SignalWithContext[];
  blockers: SignalWithContext[];
  mix: Record<string, Record<string, number>>;
  chunk_count: number;
};

export type InsightStat = { id: string; icon: string; number: number | null; text: string | null; label: string };
export type HomeInsights = {
  enabled: boolean;
  stats: InsightStat[];
  findings: RankedFinding[];
  summary: string[];
  report_generated_at: number | null;
  report_model: string | null;
  stale: boolean;
  index: IndexStatus;
};

export type SemanticHit = {
  chunk_id: number | null;
  transcription_id: number;
  text: string;
  created_at: number;
  app: string | null;
  score: number;
};

/* -------------------------------------------------------------------- api -- */

/** Minutes east of UTC, so the backend buckets days/hours like this machine. */
export const tzOffset = () => -new Date().getTimezoneOffset();

export const semantic = {
  status: () => call<SemanticStatus>("semantic_status", { tzOffset: tzOffset() }),
  llmStatus: () => call<LlmStatus>("semantic_llm_status"),
  search: (query: string, limit = 8) => call<SemanticHit[]>("semantic_search", { query, limit }),
  indexNow: () => call<number>("semantic_index_now"),
  clearIndex: () => call<void>("semantic_clear_index"),
  rebuildGraph: () => call<void>("semantic_rebuild_graph"),
  conceptGraph: (range: Range, kind: string, query: string) =>
    call<ConceptGraph>("semantic_concept_graph", { range, kind, query, tzOffset: tzOffset() }),
  conceptDetail: (range: Range, id: string) =>
    call<ConceptDetail | null>("semantic_concept_detail", { range, id, tzOffset: tzOffset() }),
  conceptBrief: (range: Range, id: string) =>
    call<string>("semantic_concept_brief", { range, id, tzOffset: tzOffset() }),
  activity: (range: Range) => call<ActivityReport>("semantic_activity", { range, tzOffset: tzOffset() }),
  activityStart: (range: Range) => call<ActivityState>("semantic_activity_start", { range }),
  activityStop: () => call<ActivityState>("semantic_activity_stop"),
  insightReport: () => call<StoredReport | null>("semantic_insight_report"),
  generateInsights: () => call<InsightReport>("semantic_generate_insights", { tzOffset: tzOffset() }),
  findings: (kinds?: string[], status?: string) => call<InsightFinding[]>("semantic_findings", { kinds, status }),
  openLoops: () => call<InsightFinding[]>("semantic_open_loops"),
  setFindingStatus: (id: number, status: "resolved" | "dismissed" | "active") =>
    call<boolean>("semantic_set_finding_status", { id, status }),
  signals: (sinceDays?: number) => call<SignalsOverview>("semantic_signals", { sinceDays }),
  homeInsights: () => call<HomeInsights>("semantic_home_insights", { tzOffset: tzOffset() }),
};

/** Hand a question to Chat AI (it listens for `chat-prefill`) and switch to it. */
export async function askChat(question: string) {
  await emit("chat-prefill", question);
  await emit("navigate", "chat");
}

/** Model › AI Features, where semantic memory and the insight model live (as on the Mac). */
export const openSettings = () => emit("navigate", "ai-settings");

/* ------------------------------------------------------------------ hooks -- */

/** Index status kept fresh by `semantic-progress` / `semantic-updated` / `semantic-activity`. */
export function useSemanticStatus() {
  const [status, setStatus] = useState<SemanticStatus | null>(null);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [lastRun, setLastRun] = useState<IndexRunResult | null>(null);
  const [activity, setActivity] = useState<ActivityState | null>(null);
  const [revision, setRevision] = useState(0);
  const timer = useRef<number>(0);

  const refresh = useCallback(() => {
    semantic
      .status()
      .then((s) => {
        setStatus(s);
        setProgress(s.progress);
        if (s.last_run) setLastRun(s.last_run);
        setActivity(s.activity.state ? s.activity : null);
      })
      .catch(() => {});
  }, []);

  useEffect(refresh, [refresh]);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  useEvent<IndexProgress>("semantic-progress", (p) => {
    setProgress(p.phase === "done" ? null : p);
    // Counts move while indexing; refresh at most twice a second.
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(refresh, 500);
  });
  useEvent<IndexRunResult | Record<string, unknown>>("semantic-updated", (r) => {
    if (r && "embedded_count" in r) setLastRun(r as IndexRunResult);
    setProgress(null);
    setRevision((n) => n + 1);
    refresh();
  });
  useEvent<ActivityState>("semantic-activity", setActivity);
  useEvent<number>("semantic-activity-revision", () => setRevision((n) => n + 1));
  useEvent<unknown>("history-changed", () => {
    setRevision((n) => n + 1);
    refresh();
  });

  return { status, progress, lastRun, activity, revision, refresh, setActivity };
}

/** Local LLM reachability, re-checked on demand. */
export function useLlmStatus(enabled: boolean) {
  const [llm, setLlm] = useState<LlmStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const check = useCallback(() => {
    setChecking(true);
    semantic
      .llmStatus()
      .then(setLlm)
      .catch((e) =>
        setLlm({
          reachable: false,
          provider: "",
          models: [],
          embedding_models: [],
          embedding_model_installed: false,
          summary_model_installed: false,
          error: String(e),
        }),
      )
      .finally(() => setChecking(false));
  }, []);
  useEffect(() => {
    if (enabled) check();
  }, [enabled, check]);
  return { llm, checking, check };
}

/* ---------------------------------------------------------------- format -- */

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** "Mar 4". */
export const monthDay = (unix: number) => {
  const d = new Date(unix * 1000);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
};
/** "today" / "yesterday" / "3 days ago" / "2 weeks ago" / "in 2 months". */
export function relative(unix: number, now = Date.now() / 1000): string {
  const startOf = (t: number) => {
    const d = new Date(t * 1000);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };
  const days = Math.round((startOf(now) - startOf(unix)) / 86_400_000);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  if (days < 30) {
    const w = Math.round(days / 7);
    return `${w} week${w === 1 ? "" : "s"} ago`;
  }
  if (days < 365) {
    const m = Math.round(days / 30);
    return `${m} month${m === 1 ? "" : "s"} ago`;
  }
  const y = Math.round(days / 365);
  return `${y} year${y === 1 ? "" : "s"} ago`;
}
export const pct = (v: number) => `${Math.round(v * 100)}%`;
export const plural = (n: number, word: string, many = `${word}s`) =>
  `${n.toLocaleString()} ${n === 1 ? word : many}`;
/** "Mar 4, 2026, 14:05". */
export const mediumDateTime = (unix: number) =>
  new Date(unix * 1000).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });

/* ---------------------------------------------------------------- glyphs -- */

/** Extra 24×24 stroke glyphs the semantic views need (SF Symbol stand-ins). */
export const GLYPHS: Record<string, string> = {
  bulb: "M9 18h6M10 21h4M12 3a6 6 0 0 0-3.6 10.8c.6.5 1 1.2 1.1 2V17h5v-1.2c.1-.8.5-1.5 1.1-2A6 6 0 0 0 12 3Z",
  activity: "M4 4h16v16H4zM8 15v-3M12 15V9M16 15v-5",
  map: "M9 4 3 6.5v13.5l6-2.5 6 2.5 6-2.5V4l-6 2.5L9 4ZM9 4v13.5M15 6.5V20",
  checkCircle: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM8 12.5l2.5 2.5L16 9.5",
  xCircle: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM9 9l6 6M15 9l-6 6",
  question: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6M12 17h.01",
  hand: "M8 13V6.5a1.5 1.5 0 0 1 3 0V12M11 11V4.5a1.5 1.5 0 0 1 3 0V11M14 11V6a1.5 1.5 0 0 1 3 0v8a7 7 0 0 1-12.6 4.2L2.8 15.6a1.5 1.5 0 0 1 2.4-1.8L8 16",
  cpu: "M7 7h10v10H7zM10 10h4v4h-4zM9 3v4M15 3v4M9 17v4M15 17v4M3 9h4M3 15h4M17 9h4M17 15h4",
  shield: "M12 3 4.5 6v5.5c0 4.5 3.2 8.2 7.5 9.5 4.3-1.3 7.5-5 7.5-9.5V6L12 3ZM9 12l2 2 4-4",
  turn: "M5 4v7a4 4 0 0 0 4 4h10M15 11l4 4-4 4",
  quote: "M7 7h4v4c0 3-2 5-4 6M14 7h4v4c0 3-2 5-4 6",
  hammer: "M14 6 18 2l4 4-4 4M15 7 5 17a2 2 0 0 0 3 3L18 10",
  wrench: "M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-.6-.6-2.4 2.6-2.6Z",
  brush: "M18 3 9 12M7 14c-2 0-3 1.5-3 3.5S3 21 3 21s4 0 5.5-1.5S10 16 7 14ZM9 12l3 3",
  chat: "M4 5h16v11H8l-4 4V5ZM8 9h8M8 12h5",
  gear: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM19.4 13a7.6 7.6 0 0 0 0-2l2-1.5-2-3.4-2.4 1a7.4 7.4 0 0 0-1.7-1L15 3.5h-4l-.4 2.6a7.4 7.4 0 0 0-1.7 1l-2.4-1-2 3.4 2 1.5a7.6 7.6 0 0 0 0 2l-2 1.5 2 3.4 2.4-1c.5.4 1.1.8 1.7 1l.4 2.6h4l.4-2.6c.6-.2 1.2-.6 1.7-1l2.4 1 2-3.4-2-1.5Z",
  seal: "M12 3l2.2 1.6 2.7-.1.8 2.6 2.2 1.6-.9 2.6.9 2.6-2.2 1.6-.8 2.6-2.7-.1L12 21l-2.2-1.6-2.7.1-.8-2.6-2.2-1.6.9-2.6-.9-2.6 2.2-1.6.8-2.6 2.7.1L12 3ZM9 12l2 2 4-4",
  dotted: "M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4",
  up: "M7 17 17 7M9 7h8v8",
  person: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM4 21a8 8 0 0 1 16 0",
  appBadge: "M4 6h12v14H4zM18 3a3 3 0 1 1 0 6 3 3 0 0 1 0-6Z",
  zoomIn: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14ZM21 21l-4.3-4.3M11 8v6M8 11h6",
  zoomOut: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14ZM21 21l-4.3-4.3M8 11h6",
  fit: "M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5",
  waveform: "M3 12h2M7 8v8M11 5v14M15 9v6M19 7v10",
  moonZ: "M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5ZM14 3h4l-4 4h4",
  brain: "M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 6 1V5a2 2 0 0 0-3-1ZM15 4a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-6 1",
  rotate: "M4 12a8 8 0 0 1 14-5.3M20 4v5h-5M20 12a8 8 0 0 1-14 5.3M4 20v-5h5",
  stack: "M12 3 3 8l9 5 9-5-9-5ZM3 13l9 5 9-5M3 17.5l9 5 9-5",
};

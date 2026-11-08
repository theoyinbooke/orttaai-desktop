// Local AI: typed wrappers over the AI area's Tauri commands and events
// (app/src-tauri/src/ai.rs), plus small shared helpers.
import { call } from "./api";

// ---- Provider + models ------------------------------------------------------

export type Provider = "ollama" | "lm_studio";

export type ModelInfo = {
  name: string;
  size: number | null;
  embedding: boolean;
  thinking: boolean | null;
};

export type AiStatus = {
  provider: Provider;
  provider_name: string;
  endpoint: string;
  reachable: boolean;
  message: string;
  models: ModelInfo[];
  can_download: boolean;
};

export type Health = { reachable: boolean; models: string[]; message: string };

export type CatalogEntry = {
  name: string;
  size: number | null;
  kind: "chat" | "embedding";
  installed: boolean;
};

/** `ai-pull-progress` payload. */
export type PullEvent = {
  model: string;
  purpose: string;
  status: string;
  completed: number | null;
  total: number | null;
  fraction: number | null;
  done: boolean;
  error: string | null;
};

export type S1MiniStatus = {
  provider_name: string;
  reachable: boolean;
  installed: boolean;
  model: string | null;
  downloading: boolean;
  download: string;
  size: number;
  credit: string;
  message: string;
};

export type PolishOutcome = { text: string; polished: boolean; ms: number; error: string | null };

export const ai = {
  status: () => call<AiStatus>("ai_status"),
  test: (provider?: Provider, endpoint?: string) =>
    call<Health>("ai_test_connection", { provider, endpoint }),
  models: () => call<ModelInfo[]>("ai_models"),
  catalog: () => call<CatalogEntry[]>("ai_catalog"),
  pull: (model: string, purpose: string) => call("ai_pull_model", { model, purpose }),
  warm: () => call<{ model: string; ms: number }[]>("ai_warm_models"),
  s1Status: () => call<S1MiniStatus>("s1_mini_status"),
  s1Install: () => call("s1_mini_install"),
  polishTest: (text: string) => call<PolishOutcome>("ai_polish_test", { text }),
  rewrite: (selection: string, instruction: string) =>
    call<string>("ai_rewrite_selection", { selection, instruction }),
};

// ---- Chat AI ------------------------------------------------------------------

export type ChatConversation = {
  id: number;
  title: string;
  model: string | null;
  created_at: number;
  updated_at: number;
  message_count: number;
};

export type ChatGroup = { title: string; conversations: ChatConversation[] };

export type ChatSource = {
  transcription_id: number;
  text: string;
  created_at: number;
  app: string | null;
  score: number;
};

export type ChatMessage = {
  id: number;
  conversation_id: number;
  role: "user" | "assistant";
  content: string;
  reasoning: string | null;
  sources: ChatSource[];
  created_at: number;
};

export type ThinkingDepth = "low" | "medium" | "high";
export type ChatMode = "regular" | "my_tone";
export type ChatDocument = { name: string; content: string };

export type ChatRequest = {
  conversation_id: number | null;
  prompt: string;
  model: string;
  depth: ThinkingDepth;
  think: boolean;
  mode: ChatMode;
  documents: ChatDocument[];
};

export type PreparedTurn = {
  conversation_id: number;
  user_message: ChatMessage;
  assistant_message_id: number;
  model: string;
};

/** `chat-done` payload. */
export type TurnResult = {
  conversation_id: number;
  assistant_message_id: number;
  message: ChatMessage | null;
  stopped: boolean;
  error: string | null;
};

/** `chat-delta` payload (accumulated text so far). */
export type ChatDeltaEvent = {
  conversation_id: number;
  message_id: number;
  content: string;
  reasoning: string;
};

export type ChatErrorEvent = { conversation_id: number; message_id: number; error: string };

export type ChatInfo = {
  provider_name: string;
  endpoint: string;
  default_model: string;
  has_tone_profile: boolean;
  semantic_enabled: boolean;
  inflight: number[];
};

/** Seconds east of UTC, as the backend's local-time grouping expects. */
export const utcOffset = () => -new Date().getTimezoneOffset() * 60;

export const chat = {
  conversations: (search?: string) =>
    call<ChatGroup[]>("chat_conversations", { search: search || null, utc_offset: utcOffset() }),
  messages: (conversation_id: number) => call<ChatMessage[]>("chat_messages", { conversation_id }),
  rename: (conversation_id: number, title: string) => call("chat_rename", { conversation_id, title }),
  remove: (conversation_id: number) => call("chat_delete", { conversation_id }),
  models: () => call<ModelInfo[]>("chat_models"),
  info: () => call<ChatInfo>("chat_info"),
  send: (request: ChatRequest) => call<PreparedTurn>("chat_send", { request, utc_offset: utcOffset() }),
  stop: (conversation_id: number) => call<boolean>("chat_stop", { conversation_id }),
};

/** Reasoning-model families when the server doesn't say (Mac list). */
export function nameSupportsThinking(model: string): boolean {
  const n = model.toLowerCase();
  return ["qwen3", "qwen-3", "deepseek-r1", "deepseek-r", "glm-4", "gpt-oss", "reasoning", "think"].some((f) =>
    n.includes(f),
  );
}

/** The status line while a reply has no words yet (Mac `ChatAIReplyStatus`). */
export function replyStatusLine(elapsedSec: number, thinking: boolean, thought: string | null): string {
  const latest = latestThought(thought);
  if (latest) return latest;
  const steps: [number, string][] = thinking
    ? [
        [0, "Reading your message"],
        [1.5, "Thinking"],
        [8, "Thinking it through"],
        [20, "Still thinking"],
      ]
    : [
        [0, "Reading your message"],
        [2.5, "Working on your reply"],
        [7, "Longer chats take a little more time"],
        [15, "Still working on it"],
      ];
  let line = steps[0][1];
  for (const [after, text] of steps) if (elapsedSec >= after) line = text;
  return line;
}

/** The last finished sentence of the model's reasoning, one line long. */
export function latestThought(thinking: string | null | undefined): string | null {
  if (!thinking) return null;
  const sentences: string[] = [];
  let current = "";
  for (const ch of thinking) {
    if (ch === "\n") {
      sentences.push(current);
      current = "";
    } else {
      current += ch;
      if (".!?".includes(ch)) {
        sentences.push(current);
        current = "";
      }
    }
  }
  const found = [...sentences]
    .map((s) => s.trim())
    .reverse()
    .find((s) => s.split(" ").filter(Boolean).length >= 3);
  if (!found) return null;
  let s = found.endsWith(".") ? found.slice(0, -1) : found;
  if (s.length > 90) s = s.slice(0, 89) + "…";
  return s;
}

// ---- Writing insights + tone ------------------------------------------------------

export type TimeRange = "days7" | "days14" | "days30" | "days90" | "all";
export type GenerationMode = "balanced" | "deep";
export type AppFilterMode = "all_apps" | "include_only" | "exclude";

export type WritingInsightsRequest = {
  time_range: TimeRange;
  generation_mode: GenerationMode;
  app_filter_mode: AppFilterMode;
  selected_apps: string[];
};

export const DEFAULT_INSIGHTS_REQUEST: WritingInsightsRequest = {
  time_range: "days30",
  generation_mode: "balanced",
  app_filter_mode: "all_apps",
  selected_apps: [],
};

export type InsightSignal = { label: string; value: string; detail: string };
export type InsightPattern = { title: string; detail: string; evidence: string | null };
export type InsightRecommendation = {
  kind: "dictionary" | "snippet";
  source: string;
  target: string;
  rationale: string;
  confidence: number;
};

export type WritingInsightSnapshot = {
  generated_at: number;
  sample_count: number;
  analyzer_name: string;
  used_fallback: boolean;
  request: WritingInsightsRequest;
  summary: string;
  signals: InsightSignal[];
  patterns: InsightPattern[];
  strengths: string[];
  opportunities: string[];
  recommendations: InsightRecommendation[];
};

export type InsightHistoryItem = { id: number; is_pinned: boolean; snapshot: WritingInsightSnapshot };

export type WritingInsightsRun = {
  snapshot: WritingInsightSnapshot | null;
  snapshot_id: number | null;
  sample_count: number;
  analyzer_name: string;
  used_fallback: boolean;
  error: string | null;
  persistence_warning: string | null;
};

export type InsightsComparison = { older_id: number; newer_id: number; headline: string; bullets: string[] };

export type InsightFreshness = {
  new_session_count: number;
  latest_session_at: number | null;
  status: "fresh" | "aging" | "stale";
  should_auto_refresh: boolean;
};

export type WritingInsightsInfo = { analyzer_name: string; uses_llm: boolean; models: string[] };

export type ToneMetric = { name: string; value: number; label: string; detail: string };

export type ToneProfile = {
  generated_at: number;
  model: string;
  sample_count: number;
  word_count: number;
  sentence_count: number;
  overall_score: number;
  confidence: number;
  summary: string;
  descriptors: string[];
  signature_phrases: string[];
  avoidances: string[];
  signature_approaches: string[];
  recommendations: string[];
  metrics: ToneMetric[];
  prompt_guide: string;
  sample_excerpts: string[];
};

export type ToneRun = { profile: ToneProfile; snapshot_id: number | null; used_llm: boolean; error: string | null };
export type ToneHistoryItem = { id: number; is_pinned: boolean; profile: ToneProfile };

export const writing = {
  info: () => call<WritingInsightsInfo>("writing_insights_info"),
  generate: (request: WritingInsightsRequest) =>
    call<WritingInsightsRun>("writing_insights_generate", { request, utc_offset: utcOffset() }),
  quick: (request: WritingInsightsRequest) =>
    call<WritingInsightSnapshot | null>("writing_insights_quick", { request, utc_offset: utcOffset() }),
  latest: () => call<InsightHistoryItem | null>("writing_insights_latest"),
  history: (limit = 24) => call<InsightHistoryItem[]>("writing_insights_history", { limit }),
  get: (id: number) => call<InsightHistoryItem | null>("writing_insights_get", { id }),
  freshness: (generated_at: number) => call<InsightFreshness>("writing_insights_freshness", { generated_at }),
  compare: (a: number, b: number) =>
    call<InsightsComparison>("writing_insights_compare", { a, b, utc_offset: utcOffset() }),
  apps: () => call<string[]>("writing_insights_apps"),
  apply: (recommendation: InsightRecommendation) =>
    call<number>("writing_apply_recommendation", { recommendation }),
  pin: (id: number, pinned: boolean) => call<boolean>("insight_snapshot_pin", { id, pinned }),
  remove: (id: number) => call<boolean>("insight_snapshot_delete", { id }),
};

export const tone = {
  analyze: (model: string) => call<ToneRun | null>("tone_analyze", { model }),
  latest: () => call<ToneHistoryItem | null>("tone_latest"),
  history: (limit = 24) => call<ToneHistoryItem[]>("tone_history", { limit }),
  get: (id: number) => call<ToneHistoryItem | null>("tone_get", { id }),
  sampleCount: () => call<number>("tone_sample_count"),
};

// ---- Formatting -----------------------------------------------------------------

/** 7162000000 → "7.2 GB". */
export function formatBytes(n: number | null | undefined): string {
  if (!n || n <= 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

/** "3 minutes ago", "yesterday", "Oct 4". */
export function relativeTime(unix: number): string {
  const diff = Date.now() / 1000 - unix;
  if (diff < 45) return "just now";
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`;
  if (diff < 86_400) {
    const h = Math.round(diff / 3600);
    return `${h} hour${h === 1 ? "" : "s"} ago`;
  }
  if (diff < 2 * 86_400) return "yesterday";
  if (diff < 7 * 86_400) return `${Math.round(diff / 86_400)} days ago`;
  return new Date(unix * 1000).toLocaleDateString([], { month: "short", day: "numeric" });
}

/** "3:32 PM" today, "Tue" this week, "Oct 4" before (Mac `shortTimestamp`). */
export function shortTimestamp(unix: number): string {
  const d = new Date(unix * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString())
    return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (days < 7) return d.toLocaleDateString([], { weekday: "short" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

/** "Oct 21, 2026, 3:32 PM". */
export function dateTime(unix: number): string {
  return new Date(unix * 1000).toLocaleString([], {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

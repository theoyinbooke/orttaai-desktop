// Analytics/home area: DTOs mirroring `orttaai_core::{analytics, milestones,
// whatsnew}`, formatting helpers, the share-card content model (Mac
// `ShareCardContent` + `MilestoneComparisons`) and tiny cross-view stores.
import { useSyncExternalStore } from "react";
import { call } from "./api";

/* ------------------------------------------------------------- dashboards -- */

export type HeaderStats = { active_days: number; total_words: number; average_wpm: number };
export type TodaySnapshot = { words: number; sessions: number; active_minutes: number; average_wpm: number };
export type TrendPoint = { day: string; words: number; sessions: number; average_wpm: number };
export type TopApp = { name: string; sessions: number; words: number; session_share: number };
export type PerformanceLevel = "no_data" | "fast" | "normal" | "slow";
export type PerformanceHealth = {
  level: PerformanceLevel;
  sample_count: number;
  average_processing_ms: number | null;
  processing_p50_ms: number | null;
  processing_p95_ms: number | null;
  average_transcription_ms: number | null;
  transcription_p50_ms: number | null;
  transcription_p95_ms: number | null;
  average_text_processing_ms: number | null;
  text_processing_p50_ms: number | null;
  text_processing_p95_ms: number | null;
  average_injection_ms: number | null;
  injection_p50_ms: number | null;
  injection_p95_ms: number | null;
  current_model_id: string;
  is_express: boolean;
};
export type RecentDictation = {
  id: number;
  created_at: number;
  full_text: string;
  preview_text: string;
  app_name: string;
  word_count: number;
  processing_ms: number;
};
export type Streaks = { current_days: number; best_days: number; dictated_today: boolean };
export type DashboardPayload = {
  header: HeaderStats;
  today: TodaySnapshot;
  trend30d: TrendPoint[];
  top_apps30d: TopApp[];
  performance: PerformanceHealth;
  recent: RecentDictation[];
  usual_daily_words: number | null;
  streaks: Streaks;
};
export type TimeRange = "30d" | "90d" | "all";
export type PeriodTotals = {
  sessions: number;
  words: number;
  recording_minutes: number;
  average_wpm: number;
  active_days: number;
  minutes_saved: number;
};
export type PeriodTrend = {
  previous: PeriodTotals;
  words_change: number | null;
  sessions_change: number | null;
  active_minutes_change: number | null;
  wpm_change: number | null;
};
export type AnalyticsSnapshot = {
  range: TimeRange;
  payload: DashboardPayload;
  trend_points: TrendPoint[];
  daily_active_seconds: number[];
  hourly_activity: { hour: number; count: number }[];
  weekday_activity: { weekday: number; short_name: string; name: string; sessions: number }[];
  duration_buckets: { label: string; phrase: string; count: number; order: number }[];
  words_by_app: { name: string; words: number }[];
  median_duration_ms: number | null;
  total_sessions: number;
  total_words: number;
  total_recording_minutes: number;
  active_days: number;
  average_wpm: number;
  minutes_saved: number;
  streaks: Streaks;
  previous: PeriodTrend | null;
};

export const RANGES: { value: TimeRange; label: string; short: string; trendTitle: string; ticks: number }[] = [
  { value: "30d", label: "30 Days", short: "30d", trendTitle: "30-Day Trend", ticks: 8 },
  { value: "90d", label: "90 Days", short: "90d", trendTitle: "90-Day Trend", ticks: 10 },
  { value: "all", label: "All Time", short: "All time", trendTitle: "All-Time Trend", ticks: 8 },
];

export function normalizeRange(v: string | undefined | null): TimeRange {
  return v === "90d" || v === "all" ? v : "30d";
}

/* ---------------------------------------------------------------- history -- */

export type TranscriptionRecord = {
  id: number | null;
  text: string;
  raw_text: string | null;
  app: string | null;
  app_id: string | null;
  duration_ms: number;
  processing_ms: number;
  transcription_ms: number | null;
  text_processing_ms: number | null;
  polish_ms: number | null;
  injection_ms: number | null;
  model_id: string;
  audio_device: string | null;
  injection_method: string | null;
  entry_kind: string;
  edit_instruction: string | null;
  word_count: number;
  time_zone: string | null;
  polished: boolean;
  created_at: number;
  /** Set when it came from another device through a linked Mac; null is this PC. */
  source_device_id?: string | null;
  source_device_kind?: string | null;
};
export type HistoryQuery = {
  search?: string | null;
  app?: string | null;
  entry_kind?: string | null;
  since?: number | null;
  limit: number;
  offset: number;
};
export type HistoryPage = { items: TranscriptionRecord[]; total: number };

/* ------------------------------------------------------------- milestones -- */

export type Family = "words" | "streaks" | "speed_and_time" | "dictations" | "special";
export type Tier = "bronze" | "silver" | "gold";
export type Metric =
  | "words"
  | "dictations"
  | "best_streak_days"
  | "best_wpm"
  | "minutes_saved"
  | "distinct_apps"
  | "longest_dictation_seconds"
  | "night_owl_dictations"
  | "dictionary_words";
export type BadgeState =
  | { kind: "unlocked"; at: number }
  | { kind: "in_progress"; current: number; fraction: number }
  | { kind: "locked" };
export type BadgeView = {
  id: string;
  name: string;
  rule: string;
  family: Family;
  tier: Tier;
  symbol: string;
  metric: Metric;
  threshold: number;
  state: BadgeState;
  current: number;
  fraction: number;
  unlocked_at: number | null;
  celebrated: boolean;
  progress_text: string;
  celebration_line: string;
};
export type MilestoneStats = {
  total_words: number;
  dictations: number;
  speaking_ms: number;
  best_wpm: number;
  best_streak_days: number;
  current_streak_days: number;
  distinct_apps: number;
  longest_dictation_ms: number;
  night_owl_dictations: number;
  dictionary_words: number;
  first_dictation_at: number | null;
  minutes_saved: number;
  speaking_minutes: number;
  average_wpm: number;
};
export type MilestonesOverview = {
  stats: MilestoneStats;
  badges: BadgeView[];
  families: { id: Family; title: string; note: string }[];
  next_goal: string | null;
  next_up_detail: string | null;
  unlocked_count: number;
  total: number;
};

export const TIER_COLOR: Record<Tier, string> = { bronze: "#C9895A", silver: "#C3C7CF", gold: "#E2B04A" };
export const TIER_TITLE: Record<Tier, string> = { bronze: "Bronze", silver: "Silver", gold: "Gold" };

/* -------------------------------------------------------------- what's new -- */

export type WhatsNewArt =
  | { kind: "milestones" }
  | { kind: "learning" }
  | { kind: "updates" }
  | { kind: "symbol"; symbol: string };
export type WhatsNewItem = { id: string; title: string; detail: string; art: WhatsNewArt };
export type WhatsNewRelease = { version: string; items: WhatsNewItem[] };

/* ------------------------------------------------------------- formatting -- */

export const fmt = (n: number) => Math.round(n).toLocaleString("en-US");
export const plural = (n: number, one: string, many = `${one}s`) => `${fmt(n)} ${n === 1 ? one : many}`;

/** `YYYY-MM-DD` (a local calendar day) → a local Date at midnight. */
export function dayDate(day: string): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, (m || 1) - 1, d || 1);
}

export function percent(part: number, total: number): string {
  if (total <= 0) return "0%";
  return `${Math.round((part / total) * 100)}%`;
}

/** Dictations shorter than a minute still count as time spent ("<1m"). */
export function formatMinutes(minutes: number, hasSessions: boolean): string {
  if (minutes === 0) return hasSessions ? "<1m" : "0m";
  if (minutes < 60) return `${minutes}m`;
  const h = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${h}h` : `${h}h ${rest}m`;
}

/** 4,200 ms → "4 s"; 75,000 → "1 min 15 s". */
export function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds} s`;
  const m = Math.floor(seconds / 60);
  const r = seconds % 60;
  return r === 0 ? `${m} min` : `${m} min ${r} s`;
}

/** History's duration column: "4.2s", "1:05", "1:02:09". */
export function clockDuration(ms: number): string {
  const clamped = Math.max(0, ms);
  const total = Math.floor(clamped / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  if (h > 0) return `${h}:${two(m)}:${two(s)}`;
  if (m > 0) return `${m}:${two(s)}`;
  return `${(clamped / 1000).toFixed(1)}s`;
}

export function hourName(hour: number): string {
  if (hour === 0) return "12 am";
  if (hour === 12) return "12 pm";
  return hour < 12 ? `${hour} am` : `${hour - 12} pm`;
}

export function hourLabel(hour: number): string {
  if (hour === 0) return "12a";
  if (hour < 12) return `${hour}a`;
  if (hour === 12) return "12p";
  return `${hour - 12}p`;
}

/** "+12%" / "−8%" for a fractional change. */
export function changeText(change: number | null | undefined): string | null {
  if (change == null || !Number.isFinite(change)) return null;
  const pct = Math.round(change * 100);
  if (pct === 0) return "±0%";
  return `${pct > 0 ? "+" : "−"}${Math.abs(pct)}%`;
}

/** "ggml-large-v3-turbo-q5_0" → "Large V3 Turbo q5_0"; "…base.en…" → "(English)". */
export function modelDisplayName(id: string): string {
  if (!id || id === "Not set") return "Not set";
  if (id.startsWith("Express")) return id;
  let name = id.replace(/^ggml-/, "").replace(/^openai_whisper[-_]/, "Whisper ");
  const quant = name.match(/[-_.](q\d(_\d)?|f16|f32)$/i);
  if (quant) name = name.slice(0, quant.index);
  const english = /\.en$|[-_]en$/.test(name);
  name = name.replace(/\.en$|[-_]en$/, "");
  const words = name
    .split(/[-_. ]+/)
    .filter(Boolean)
    .map((w) => (/^v\d/i.test(w) && w.length <= 3 ? w.toUpperCase() : w[0].toUpperCase() + w.slice(1)));
  let out = words.join(" ");
  if (english) out += " (English)";
  if (quant) out += ` ${quant[1]}`;
  return out;
}

/** Relative "3 min ago"-style label (Mac RelativeDateTimeFormatter, short). */
export function relativeTime(unix: number, now = Date.now()): string {
  const diff = Math.round((now - unix * 1000) / 1000);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" });
  const abs = Math.abs(diff);
  if (abs < 60) return rtf.format(-diff, "second");
  if (abs < 3600) return rtf.format(-Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(-Math.round(diff / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(-Math.round(diff / 86400), "day");
  if (abs < 86400 * 365) return rtf.format(-Math.round(diff / (86400 * 30)), "month");
  return rtf.format(-Math.round(diff / (86400 * 365)), "year");
}

/** Copy text: the webview clipboard first, then the native tools (by id). */
export async function copyText(text: string, id?: number | null): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    /* fall through to the native path */
  }
  if (id != null) await call<string>("copy_transcription", { id });
  else throw new Error("Couldn't copy to the clipboard.");
}

/* ---------------------------------------------------------- shared stores -- */

function createStore<T>(initial: T) {
  let value = initial;
  const subs = new Set<() => void>();
  return {
    get: () => value,
    set: (v: T) => {
      value = v;
      subs.forEach((s) => s());
    },
    subscribe: (fn: () => void) => {
      subs.add(fn);
      return () => {
        subs.delete(fn);
      };
    },
  };
}

/** Celebrations wait for What's New to close. */
export const whatsNewOpen = createStore(false);
export const useWhatsNewOpen = () => useSyncExternalStore(whatsNewOpen.subscribe, whatsNewOpen.get);

/** Which Analytics tab to open next (Home's "Open History"). */
export type AnalyticsTab = "dashboard" | "tone" | "history";
export const pendingAnalyticsTab = createStore<AnalyticsTab | null>(null);

/* -------------------------------------------- comparisons (Mac, verbatim) -- */

export const C = {
  metresPerWord: 0.012,
  wordsPerPage: 250,
  keyPressesPerWord: 5.7,
  typingWPM: 40,
  handwritingWPM: 13,
  conversationWPM: 150,
  auctioneerWPM: 250,
  landmarks: [
    { name: "Statue of Liberty", metres: 93 },
    { name: "Eiffel Tower", metres: 330 },
    { name: "Burj Khalifa", metres: 828 },
    { name: "Kilimanjaro", metres: 5_895 },
    { name: "Everest", metres: 8_849 },
  ],
  books: [
    { title: "The Great Gatsby", words: 47_000 },
    { title: "an average novel", words: 90_000 },
    { title: "War and Peace", words: 587_000 },
  ],
};

export function wordsToReach(metres: number): number {
  const words = metres / C.metresPerWord;
  const step = words >= 100_000 ? 1_000 : 250;
  return Math.round(words / step) * step;
}
export const heightMetres = (words: number) => Math.max(0, words) * C.metresPerWord;
export function heightText(words: number): string {
  const m = heightMetres(words);
  return m >= 1000 ? `${(m / 1000).toFixed(1)} km` : `${Math.round(m)} m`;
}
export const nextLandmark = (words: number) => C.landmarks.find((l) => words < wordsToReach(l.metres));
export const pages = (words: number) => Math.round(Math.max(0, words) / C.wordsPerPage);
export const keyPresses = (words: number) => Math.round(Math.max(0, words) * C.keyPressesPerWord);
export function keyPressesText(words: number): string {
  const p = keyPresses(words);
  return `~${fmt(p >= 1000 ? Math.round(p / 100) * 100 : p)}`;
}
export const typingMinutes = (words: number) => Math.round(Math.max(0, words) / C.typingWPM);
export function multiple(wpm: number, baseline: number): string {
  if (wpm <= 0) return "0";
  const v = wpm / baseline;
  if (v >= 10) return String(Math.round(v));
  const r = Math.round(v * 10) / 10;
  return r === Math.round(r) ? String(r) : r.toFixed(1);
}
export const longestBookPassed = (words: number) => [...C.books].reverse().find((b) => words >= b.words);
export function durationText(minutes: number): string {
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (h >= 10 || rest === 0) {
    const whole = Math.round(minutes / 60);
    return whole === 1 ? "1 hour" : `${whole} hours`;
  }
  return `${h} h ${rest} min`;
}
export function timeBackEquivalent(minutes: number): string {
  if (minutes < 15) return "a coffee break";
  if (minutes < 75) return "a TV episode";
  if (minutes < 180) return "a feature film";
  if (minutes < 600) return "a working day";
  return `${Math.max(2, Math.floor(minutes / 480))} working days`;
}
export function secondsText(seconds: number): string {
  if (seconds < 60) return `${seconds} s`;
  const rest = seconds % 60;
  return rest === 0 ? `${Math.floor(seconds / 60)} min` : `${Math.floor(seconds / 60)} min ${rest} s`;
}

/* ------------------------------------------- share-card content (Mac port) -- */

export type CardTemplate =
  | "mountain" | "books" | "speed" | "hands" | "receipt" | "signal" | "editorial" | "medal"
  | "streak" | "race" | "stack" | "apps" | "stopwatch" | "dictionary" | "night";
export const TEMPLATE_TITLE: Record<CardTemplate, string> = {
  mountain: "Mountain", books: "Books", speed: "Speed", hands: "Hands Free", receipt: "Receipt",
  signal: "Signal", editorial: "Editorial", medal: "Medal", streak: "Streak", race: "Race",
  stack: "Stack", apps: "Apps", stopwatch: "Stopwatch", dictionary: "Dictionary", night: "Night",
};
export type CardSize = "portrait" | "square" | "story";
export const CARD_SIZES: { value: CardSize; label: string; w: number; h: number }[] = [
  { value: "portrait", label: "Portrait 4:5", w: 1080, h: 1350 },
  { value: "square", label: "Square", w: 1080, h: 1080 },
  { value: "story", label: "Story 9:16", w: 1080, h: 1920 },
];

export type CardKind =
  | "progress" | "firstWords" | "words" | "streak" | "speed" | "timeSaved"
  | "dictations" | "apps" | "marathon" | "dictionary" | "nightOwl";

export function cardKind(badge: BadgeView | null): CardKind {
  if (!badge) return "progress";
  if (badge.id === "first-words") return "firstWords";
  switch (badge.metric) {
    case "words": return "words";
    case "dictations": return "dictations";
    case "best_streak_days": return "streak";
    case "best_wpm": return "speed";
    case "minutes_saved": return "timeSaved";
    case "distinct_apps": return "apps";
    case "longest_dictation_seconds": return "marathon";
    case "dictionary_words": return "dictionary";
    case "night_owl_dictations": return "nightOwl";
  }
}

/** The designs for a milestone's kind, its own picture first. */
export function templatesFor(kind: CardKind): CardTemplate[] {
  switch (kind) {
    case "progress": return ["mountain", "books", "speed", "hands", "receipt", "signal", "editorial", "medal"];
    case "firstWords": return ["signal", "editorial", "receipt", "medal"];
    case "words": return ["mountain", "books", "editorial", "receipt", "signal", "medal"];
    case "streak": return ["streak", "signal", "editorial", "medal"];
    case "speed": return ["speed", "race", "signal", "editorial", "medal"];
    case "timeSaved": return ["hands", "receipt", "signal", "editorial", "medal"];
    case "dictations": return ["stack", "receipt", "signal", "editorial", "medal"];
    case "apps": return ["apps", "signal", "editorial", "medal"];
    case "marathon": return ["stopwatch", "signal", "editorial", "medal"];
    case "dictionary": return ["dictionary", "signal", "editorial", "medal"];
    case "nightOwl": return ["night", "signal", "editorial", "medal"];
  }
}

export type ShareStat = { value: string; label: string };

const longDate = (unix: number | null) =>
  new Date(unix ? unix * 1000 : Date.now()).toLocaleDateString("en-US", { month: "long", day: "numeric", year: "numeric" });
const shortDate = (unix: number | null) =>
  new Date(unix ? unix * 1000 : Date.now()).toLocaleDateString("en-US", { month: "short", day: "numeric" });

/** Everything a card shows: numbers only, never anything dictated. */
export class CardContent {
  constructor(
    readonly stats: MilestoneStats,
    readonly badge: BadgeView | null,
    readonly unlocked: BadgeView[],
  ) {}

  get earnedOn(): number | null {
    return this.badge?.unlocked_at ?? null;
  }
  get kind(): CardKind {
    return cardKind(this.badge);
  }
  /** A words badge's own count, else everything so far. */
  get words(): number {
    return this.badge?.metric === "words" ? this.badge.threshold : this.stats.total_words;
  }
  get pace(): number {
    if (this.badge?.metric === "best_wpm") return this.stats.best_wpm;
    return this.stats.average_wpm > 0 ? this.stats.average_wpm : this.stats.best_wpm;
  }
  get eyebrow(): string {
    return this.badge ? `MILESTONE · ${this.badge.name.toUpperCase()}` : `${fmt(this.words)} WORDS`;
  }
  get dateText(): string {
    return longDate(this.earnedOn);
  }
  get milestoneValue(): number {
    if (!this.badge) return this.stats.total_words;
    return this.badge.metric === "best_wpm" ? this.stats.best_wpm : this.badge.threshold;
  }
  get wordsPerDictation(): number {
    return this.stats.dictations > 0 ? Math.round(this.stats.total_words / this.stats.dictations) : 0;
  }
  get hero(): { value: string; label: string } {
    const b = this.badge;
    if (!b) return { value: fmt(this.words), label: "words dictated" };
    switch (b.metric) {
      case "words": return { value: fmt(b.threshold), label: "words dictated" };
      case "dictations": return b.threshold === 1 ? { value: "1st", label: "dictation" } : { value: fmt(b.threshold), label: "dictations" };
      case "best_streak_days": return { value: `${b.threshold}`, label: "days in a row" };
      case "best_wpm": return { value: `${this.stats.best_wpm}`, label: "words a minute" };
      case "minutes_saved": return { value: durationText(b.threshold), label: "of typing saved" };
      case "distinct_apps": return { value: `${b.threshold}`, label: "apps, one voice" };
      case "longest_dictation_seconds": return { value: `${Math.floor(this.stats.longest_dictation_ms / 1000)}s`, label: "in one go" };
      case "dictionary_words": return { value: `${b.threshold}`, label: "dictionary words" };
      case "night_owl_dictations": return { value: "After 12", label: "dictating at night" };
    }
  }
  get headline(): [string, string, string] {
    const s = this.stats;
    switch (this.kind) {
      case "progress": return [`I spoke ${fmt(s.total_words)} words to my computer and typed `, "none", " of them."];
      case "firstWords": return ["My first words to my computer, and ", "not one key", " pressed."];
      case "words": return ["I've spoken ", `${fmt(this.words)} words`, " to my computer instead of typing them."];
      case "streak": return ["I talked to my computer ", `${this.milestoneValue} days`, " in a row."];
      case "speed": return ["I dictate at ", `${s.best_wpm} words a minute`, ", faster than I can type."];
      case "timeSaved": return ["Talking gave me back ", durationText(this.milestoneValue), " of typing."];
      case "dictations": return ["I've made ", `${fmt(this.milestoneValue)} dictations`, ", every word spoken instead of typed."];
      case "apps": return ["One voice, ", `${this.milestoneValue} apps`, ". Orttaai types wherever I am."];
      case "marathon": return ["I talked for ", secondsText(Math.floor(s.longest_dictation_ms / 1000)), " in one go."];
      case "dictionary": return ["My computer knows ", `${this.milestoneValue} words`, " the way I spell them."];
      case "nightOwl": return ["After midnight, my ", "keyboard", " got the night off."];
    }
  }
  get supportLine(): string {
    const s = this.stats;
    switch (this.kind) {
      case "progress":
      case "words":
        return `${fmt(pages(this.words))} pages of a book. ${heightText(this.words)} of words, stood on end.`;
      case "firstWords": return `Started ${this.dateText}. Talking instead of typing from here on.`;
      case "streak": return "A dictation every day, and the keyboard got a rest.";
      case "speed":
        return `${multiple(s.best_wpm, C.typingWPM)}× typing at 40 words a minute, ${multiple(s.best_wpm, C.handwritingWPM)}× writing by hand.`;
      case "timeSaved": return `About ${timeBackEquivalent(this.milestoneValue)}, with both hands free.`;
      case "dictations": return `About ${this.wordsPerDictation} words each, spoken at the cursor.`;
      case "apps": return "Wherever the cursor is, my voice works.";
      case "marathon": return "One long dictation, hands free the whole time.";
      case "dictionary": return "Names and terms come out right, every time.";
      case "nightOwl": return `${plural(s.night_owl_dictations, "dictation")} after 12 a.m.`;
    }
  }
  /** Three numbers; the last is drawn in amber. */
  get supportingStats(): ShareStat[] {
    const s = this.stats;
    const dictations = { value: fmt(s.dictations), label: "dictations" };
    const totalWords = { value: fmt(s.total_words), label: "words so far" };
    const saved = { value: durationText(s.minutes_saved), label: "of typing saved" };
    switch (this.kind) {
      case "progress": return [totalWords, { value: this.pace > 0 ? `${this.pace}` : "–", label: "words a minute" }, saved];
      case "firstWords": return [{ value: shortDate(this.earnedOn), label: "first dictation" }, dictations, totalWords];
      case "words": return [
        { value: fmt(pages(this.words)), label: "pages of a book" },
        { value: keyPressesText(this.words), label: "keys I didn't press" },
        { value: durationText(typingMinutes(this.words)), label: "to type them" },
      ];
      case "streak": return [
        { value: `${s.best_streak_days}`, label: "best streak, days" },
        { value: `${s.current_streak_days}`, label: "days running now" },
        dictations,
      ];
      case "speed": return [
        { value: `${s.best_wpm}`, label: "best words a minute" },
        { value: s.average_wpm > 0 ? `${s.average_wpm}` : "–", label: "on average" },
        { value: `${multiple(s.best_wpm, C.typingWPM)}×`, label: "faster than typing" },
      ];
      case "timeSaved": return [totalWords, { value: durationText(s.speaking_minutes), label: "spent speaking" }, saved];
      case "dictations": return [
        { value: fmt(this.milestoneValue), label: "dictations" },
        { value: `${this.wordsPerDictation}`, label: "words each" },
        totalWords,
      ];
      case "apps": return [{ value: `${s.distinct_apps}`, label: "apps so far" }, dictations, totalWords];
      case "marathon": return [
        { value: secondsText(Math.floor(s.longest_dictation_ms / 1000)), label: "longest dictation" },
        { value: s.average_wpm > 0 ? `${s.average_wpm}` : "–", label: "words a minute" },
        dictations,
      ];
      case "dictionary": return [{ value: `${s.dictionary_words}`, label: "words in it now" }, dictations, totalWords];
      case "nightOwl": return [{ value: fmt(s.night_owl_dictations), label: "after midnight" }, dictations, totalWords];
    }
  }
  /** Time spent speaking the card's words, at the person's pace. */
  get speakingMinutes(): number {
    if (this.kind !== "words" || this.stats.average_wpm <= 0) return this.stats.speaking_minutes;
    return Math.round(this.words / this.stats.average_wpm);
  }
  get minutesSavedOnCard(): number {
    if (this.kind !== "words") return this.stats.minutes_saved;
    return Math.max(0, typingMinutes(this.words) - this.speakingMinutes);
  }
  get receiptLines(): [string, string][] {
    let lead: [string, string] | null = null;
    if (this.kind === "dictations") lead = ["DICTATIONS", fmt(this.milestoneValue)];
    else if (this.kind === "timeSaved") lead = ["TIME SAVED", durationText(this.milestoneValue).toUpperCase()];
    else if (this.kind === "firstWords") lead = ["FIRST DICTATION", shortDate(this.earnedOn).toUpperCase()];
    const rows: [string, string][] = [
      ["WORDS SPOKEN", fmt(this.words)],
      ["PAGES OF A BOOK", fmt(pages(this.words))],
      ["HEIGHT, STOOD UP", heightText(this.words).toUpperCase()],
      ["KEYS NOT PRESSED", keyPressesText(this.words)],
      ["TIME SPEAKING", `${this.speakingMinutes} MIN`],
      ["TIME TYPING IT", `${fmt(typingMinutes(this.words))} MIN`],
    ];
    return lead ? [lead, ...rows] : rows;
  }
  /** The badge a Medal card shows: the shared one, else the best earned. */
  get medalBadge(): BadgeView | null {
    if (this.badge) return this.badge;
    const order: Tier[] = ["bronze", "silver", "gold"];
    return this.unlocked.reduce<BadgeView | null>(
      (best, b) => (!best || order.indexOf(b.tier) > order.indexOf(best.tier) ? b : best),
      null,
    );
  }
}

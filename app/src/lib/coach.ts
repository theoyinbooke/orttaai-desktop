// Coach: typed wrappers over the `coach_*` commands, the shapes they return,
// Coach's words (Mac `CoachCopy` / `CoachFormat`), the WebView voice, and the
// turn detector for hands-free scenes. (Coach area.)
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { call, useEvent } from "./api";
import type { Settings } from "./settings";

// ---------------------------------------------------------------- types ----

export type Mode = "ai" | "people" | "writing" | "elsewhere";
export type ShapedMode = "ai" | "people" | "writing";
export const SHAPED: ShapedMode[] = ["ai", "people", "writing"];
export const MODES: Mode[] = ["ai", "people", "writing", "elsewhere"];
export type Rule = Mode | "off";
export type MeasureId = "fillers" | "phrases" | "pace" | "sentences" | "repeats" | "clarity";
export type TrackId = "prompting" | "messages" | "delivery" | "explaining";
export type PersonaId = "mentor" | "editor" | "socratic" | "sergeant" | "peer";
export type RepKind = "redo" | "drill" | "timed" | "scene";
export type Answer = "yes" | "partly" | "no";
export type HabitStatus = "watching" | "working" | "notMoving";
export type Mood = "pushing" | "weighing" | "agreed";

export type Summary = {
  dictations: number;
  words: number;
  raw_words: number;
  raw_dictations: number;
  um_uh: number;
  phrase_fillers: number;
  repeats: number;
  paced_dictations: number;
  pace: number | null;
  fast_pace: number | null;
  median_sentence: number | null;
  longest_sentence: number | null;
  clarity_dictations: number;
  polish_fixes: number;
};

export type Axis = { id: string; title: string; judged: boolean; now: number | null; then: number | null };

export type Shape = {
  mode: Mode | null;
  axes: Axis[];
  now: Partial<Record<MeasureId, number>>;
  then: Partial<Record<MeasureId, number>>;
  now_summary: Summary;
  then_summary: Summary;
  biggest_gain: string | null;
  is_complete: boolean;
  then_is_complete: boolean;
  waits_on_judge: boolean;
  changes: { axis: string; change: number }[];
};

export type Quote = { id: number; text: string; created_at: number; app_name: string | null };

export type FocusPick = { measure: MeasureId; mode: Mode | null; figure: number | null };

export type Verdict = {
  status: HabitStatus;
  before: boolean[];
  after: boolean[];
  since_count: number;
  before_share: number | null;
  after_share: number | null;
  before_value: number | null;
  after_value: number | null;
  control_drift: number | null;
  best_example_id: number | null;
  still_needed: number;
};

export type HabitRecord = { id: number; measure: string; mode: string | null; started_at: number; ended_at: number | null };

export type HabitState = {
  habit: HabitRecord;
  /** A measure id, or `q:` + a question id. */
  target: string;
  mode: Mode | null;
  verdict: Verdict;
  example: Quote | null;
  title: string;
  hit_title: string;
  measure: MeasureId | null;
  unit: string;
};

export type WeekDay = { date: number; dictated: boolean; practised: boolean; is_today: boolean; is_future: boolean };

export type AppRow = {
  key: string;
  app_id: string | null;
  app_name: string | null;
  dictations: number;
  first_at: number | null;
  display_name: string;
  rule: Rule;
  builtin: Mode;
};

export type SkillProgress = {
  reps: number;
  strong_reps: number;
  last_at: number | null;
  usual: number | null;
  mastered: boolean;
  review_due: boolean;
};

export type TrackProgress = { track: TrackId; level: number; mastered: number; skills: number; reps: number; current_skill: string };

export type Snapshot = {
  built_at: number;
  dictations: number;
  shapes: Record<ShapedMode, Shape>;
  all_shape: Shape;
  recent_by_mode: Partial<Record<Mode, number>>;
  busiest_mode: ShapedMode;
  focus: FocusPick | null;
  suggestions: FocusPick[];
  week: WeekDay[];
  dictations_this_week: number;
  words_this_week: number;
  habits: HabitState[];
  headline_habit: number | null;
  caught: { measure: MeasureId | null; quote: Quote; streak: number } | null;
  apps: AppRow[];
  first_dictation_at: number | null;
  skill_progress: Record<string, SkillProgress>;
  tracks: TrackProgress[];
  reps_this_week: number;
  todays_skill: string;
  todays_kind: RepKind;
};

export type Measures = {
  words: number;
  um_uh: number;
  phrase_fillers: number;
  wpm: number | null;
  sentence_lengths: number[];
  repeats: number;
  polish_change_ratio: number | null;
  raw_known: boolean;
};

export type CheckResult = { check: string; answer: Answer | null; confidence: number | null };

export type RepPlan = {
  skill_id: string;
  kind: RepKind;
  spoken: boolean;
  instruction: string;
  source: Quote | null;
  persona: PersonaId;
  mode: Mode | null;
};

export type RepFigure = { label: string; value: string; usual: string | null; better: boolean | null };

export type RepResult = {
  plan: RepPlan;
  text: string;
  measures: Measures;
  checks: CheckResult[];
  score: number | null;
  usual: number | null;
  skill_met: boolean | null;
  note: string;
  judge_model: string | null;
  figures: RepFigure[];
  duration_ms: number;
};

export type Scene = {
  id: string;
  track: TrackId;
  title: string;
  partner_name: string;
  partner_role: string;
  partner_brief: string;
  opening: string;
  goal: string;
  goal_question: string;
};

export type SceneTurn = { speaker: "you" | "partner"; text: string; duration_ms?: number | null; heard?: string | null };

export type SceneStart = {
  scene: Scene;
  skill: string;
  persona: PersonaId;
  usual: number | null;
  partner_model: string;
  problem: string | null;
};

export type Grade = {
  model: string;
  accuracy: number;
  yes_recall: number;
  no_recall: number;
  answered: number;
  total: number;
  ms_per_answer: number | null;
  version: number;
  graded_at: number;
};

export type JudgeState =
  | { state: "unknown" }
  | { state: "noServer" }
  | { state: "notInstalled"; model: string }
  | { state: "checking"; model: string; done: number; total: number }
  | { state: "passed"; grade: Grade }
  | { state: "failed"; grade: Grade }
  | { state: "unchecked"; model: string };

export type ModelSetup = {
  provider: "ollama" | "lm_studio";
  provider_name: string;
  endpoint: string;
  coach_model: string;
  judge_model: string;
  partner_model: string;
  scene_model: string;
  coach_automatic: boolean;
  judge_automatic: boolean;
};

export type ModelsInfo = {
  server_running: boolean;
  installed: string[];
  chat_models: string[];
  setup: ModelSetup;
  grades: Grade[];
  why_failed: Record<string, string>;
  best_passing: string | null;
  insights_model: string;
  judge_state: JudgeState;
};

export type Comparison = {
  question: string | null;
  rows: { model: string; agreement: number | null; ms_per_answer: number | null; answered: number }[];
};

export type StarterModel = { name: string; jobs: string[]; size: string };

export type SetupStatus = {
  server_running: boolean;
  installed: string[];
  models: StarterModel[];
  missing: string[];
  judge: string;
  judge_grade: Grade | null;
  is_set_up: boolean;
  provider_name: string;
  supports_install: boolean;
  memory: number;
};

export type SetupStep =
  | { step: "idle" }
  | { step: "downloading"; model: string; fraction: number | null }
  | { step: "checkingJudge"; model: string; done: number; total: number }
  | { step: "done" }
  | { step: "failed"; message: string };

export type MonthMove = { axis: string; title: string; mode: Mode | null; now: number; then: number; change: number };

export type MonthReport = {
  month: { start: number; end: number };
  previous: { start: number; end: number };
  is_current: boolean;
  first_month: number | null;
  dictations: number;
  words: number;
  days_dictated: number;
  reps: number;
  dictations_before: number;
  shapes: Record<ShapedMode, Shape>;
  all_shape: Shape;
  moves: MonthMove[];
  tracks: { track: TrackId; reps: number; mastered: string[] }[];
  best_rep: { skill: string; score: number; created_at: number } | null;
  habits: HabitState[];
  best_moment: Quote | null;
  best_moment_axis: string | null;
  is_empty: boolean;
  mastered_count: number;
  top_gain: MonthMove | null;
};

export type SkillInfo = {
  id: string;
  track: TrackId;
  title: string;
  why: string;
  drill: string;
  check: string;
  check_label: string;
  checks: string[];
  measure: MeasureId | null;
  speak_only: boolean;
  /** Habit target when the skill can become one. */
  habit: string | null;
};

export type Catalog = {
  tracks: { id: TrackId; title: string; mode: Mode | null; open_brief: string; skills: string[]; checks: string[] }[];
  skills: SkillInfo[];
  checks: { id: string; label: string; judged: boolean }[];
  scenes: Scene[];
  personas: { id: PersonaId; name: string; trait: string; sample: string; rate: number; pitch: number }[];
  measures: { id: MeasureId; axis_title: string; habit_title: string; hit_title: string; unit: string }[];
  questions: { id: string; label: string; axis_title: string }[];
  modes: { id: Mode; title: string; short_title: string }[];
  rep_kinds: { id: RepKind; title: string; time_limit: number }[];
  calibration: { examples: number; pass_accuracy: number; pass_recall: number };
  full_shape_dictations: number;
  habit_window: number;
  minimum_baseline: number;
  reps_to_master: number;
  max_scene_turns: number;
};

// ------------------------------------------------------------ commands ----

export const coachApi = {
  catalog: () => call<Catalog>("coach_catalog"),
  snapshot: () => call<Snapshot | null>("coach_snapshot"),
  judgeState: () => call<JudgeState>("coach_judge_state"),
  checkJudges: (models: string[]) => call("coach_check_judges", { models }),
  startHabit: (target: string, mode: Mode | null) => call("coach_start_habit", { target, mode }),
  endHabit: (id: number) => call("coach_end_habit", { id }),
  setAppRule: (app: { key: string; app_id: string | null; app_name: string | null }, rule: Rule) =>
    call("coach_set_app_rule", { key: app.key, appId: app.app_id, appName: app.app_name, rule }),
  removeAppRule: (key: string) => call("coach_remove_app_rule", { key }),
  setPersona: (persona: PersonaId, mode: Mode | null) => call("coach_set_persona", { persona, mode }),
  monthReport: (monthOf: number) => call<MonthReport>("coach_month_report", { monthOf }),
  beginRep: (skillId: string, kind: RepKind, spoken: boolean) =>
    call<RepPlan>("coach_begin_rep", { skillId, kind, spoken }),
  nextRep: (plan: RepPlan) => call<RepPlan>("coach_next_rep", { plan }),
  endSession: () => call("coach_end_session"),
  scoreRep: (plan: RepPlan, heard: string, written: string, durationMs: number, spoken: boolean) =>
    call<RepResult>("coach_score_rep", { plan, heard, written, durationMs, spoken }),
  arm: (token: number, polish: boolean) => call("coach_practice_arm", { token, polish }),
  disarm: () => call("coach_practice_disarm"),
  discard: () => call("coach_practice_discard"),
  sceneBegin: (sceneId: string) => call<SceneStart>("coach_scene_begin", { sceneId }),
  sceneWarm: () => call("coach_scene_warm"),
  sceneReply: (sceneId: string, turns: SceneTurn[], request: number) =>
    call<string | null>("coach_scene_reply", { sceneId, turns, request }),
  sceneMood: (sceneId: string, turns: SceneTurn[]) => call<Mood | null>("coach_scene_mood", { sceneId, turns }),
  sceneEnd: (sceneId: string, skillId: string, persona: PersonaId, usual: number | null, turns: SceneTurn[]) =>
    call<RepResult>("coach_scene_end", { sceneId, skillId, persona, usual, turns }),
  models: () => call<ModelsInfo>("coach_models"),
  compare: () => call<Comparison>("coach_compare_models"),
  setupStatus: () => call<SetupStatus>("coach_setup_status"),
  runSetup: () => call("coach_run_setup"),
  startOllama: () => call<boolean>("coach_start_ollama"),
  saveCard: (fileName: string, png: Uint8Array) => call<string>("coach_save_card", { fileName, png: Array.from(png) }),
  // The engine's own controls, for speaking a rep from the Coach page.
  toggleRecording: () => call("toggle_recording"),
};

export const OLLAMA_DOWNLOAD_URL = "https://ollama.com/download";

// --------------------------------------------------------------- hooks ----

let catalogPromise: Promise<Catalog> | null = null;

/** The static catalog, fetched once per launch. */
export function useCatalog(): Catalog | null {
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  useEffect(() => {
    catalogPromise ??= coachApi.catalog();
    let alive = true;
    catalogPromise.then((c) => alive && setCatalog(c)).catch(() => (catalogPromise = null));
    return () => {
      alive = false;
    };
  }, []);
  return catalog;
}

/** The snapshot and the judge's state, kept fresh as dictations and reps land. */
export function useCoachData() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [judge, setJudge] = useState<JudgeState>({ state: "unknown" });
  const timer = useRef<number>(0);
  const inFlight = useRef(false);
  const again = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) {
      again.current = true;
      return;
    }
    inFlight.current = true;
    try {
      const snap = await coachApi.snapshot();
      setSnapshot(snap);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoaded(true);
      inFlight.current = false;
      if (again.current) {
        again.current = false;
        refresh();
      }
    }
  }, []);

  const soon = useCallback(() => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(refresh, 600);
  }, [refresh]);

  useEffect(() => {
    refresh();
    coachApi.judgeState().then(setJudge).catch(() => {});
    return () => window.clearTimeout(timer.current);
  }, [refresh]);
  useEvent("coach-changed", soon);
  useEvent("history-changed", soon);
  useEvent<JudgeState>("coach-judge-state", setJudge);
  return { snapshot, loaded, error, judge, refresh };
}

/** Next unique id for captures and streamed replies. */
let nextToken = Math.floor(Date.now() % 1_000_000) * 1000;
export const newToken = () => ++nextToken;

export type Capture = { token: number; raw: string; text: string; duration_ms: number };

/** Resolves with the next `coach-capture` for `token` (the engine hands it over). */
export function waitForCapture(token: number): { promise: Promise<Capture>; cancel: () => void } {
  let cancel = () => {};
  const promise = new Promise<Capture>((resolve) => {
    let off: (() => void) | null = null;
    let dead = false;
    listen<Capture>("coach-capture", (e) => {
      if (e.payload.token === token) {
        off?.();
        resolve(e.payload);
      }
    }).then((u) => {
      if (dead) u();
      else off = u;
    });
    cancel = () => {
      dead = true;
      off?.();
    };
  });
  return { promise, cancel };
}

// ---------------------------------------------------- settings mirrors ----

/** The coach for a way of talking, as the backend picks it. */
export function personaFor(settings: Settings | null, mode: Mode | null): PersonaId {
  const valid = (p?: string): p is PersonaId => !!p && ["mentor", "editor", "socratic", "sergeant", "peer"].includes(p);
  const seats = settings?.coach_personas ?? {};
  const own = seats[mode ?? "elsewhere"];
  if (valid(own)) return own;
  if (valid(seats.ai)) return seats.ai;
  if (valid(settings?.coach_persona)) return settings!.coach_persona as PersonaId;
  return "mentor";
}

// ---------------------------------------------------------------- words ----

export const lowerFirst = (s: string) => (s ? s[0].toLowerCase() + s.slice(1) : s);

export function greeting(unix: number): string {
  const hour = new Date(unix * 1000).getHours();
  if (hour >= 5 && hour < 12) return "Good morning.";
  if (hour >= 12 && hour < 17) return "Good afternoon.";
  return "Good evening.";
}

export function figure(value: number | null | undefined, measure: MeasureId): string {
  if (value == null) return "–";
  switch (measure) {
    case "fillers":
    case "phrases":
    case "repeats":
      return value.toFixed(1);
    case "pace":
    case "sentences":
      return String(Math.round(value));
    case "clarity":
      return `${Math.round(value * 100)}%`;
  }
}

export const percent = (share: number | null | undefined) => (share == null ? "–" : `${Math.round(share * 100)}%`);

export function points(drift: number): string {
  const v = Math.round(drift * 100);
  return v > 0 ? `+${v}%` : `${v}%`;
}

export function modeTitle(mode: Mode | null | undefined): string {
  switch (mode) {
    case "ai":
      return "Talking to AI";
    case "people":
      return "Talking to people";
    case "writing":
      return "Thinking and writing";
    case "elsewhere":
      return "Elsewhere";
    default:
      return "Everywhere";
  }
}

export const modeShort: Record<Mode, string> = { ai: "AI", people: "People", writing: "Writing", elsewhere: "Elsewhere" };

export const statusTitle: Record<HabitStatus, string> = { watching: "Watching", working: "Working", notMoving: "Not moving" };

export function judgeLine(judge: JudgeState): string {
  switch (judge.state) {
    case "noServer":
      return "Start Ollama or LM Studio to judge the rest.";
    case "notInstalled":
      return `${judge.model} isn't installed. Pick a judge in Cast › Models.`;
    case "checking":
      return `Checking whether ${judge.model} can judge fairly…`;
    case "unchecked":
      return `Checking whether ${judge.model} can judge fairly…`;
    case "failed":
      return `${judge.grade.model} didn't pass the judge check. Try another in Cast › Models.`;
    default:
      return "Your local model is judging the rest, a few a day.";
  }
}

export function insight(shape: Shape, recent: number, judge: JudgeState, full: number): string {
  if (shape.waits_on_judge) return judgeLine(judge);
  if (!shape.is_complete) {
    if (recent < full) return `${recent} of ${full} dictations so far; your shape fills in.`;
    const waiting = shape.axes.filter((a) => a.now == null && !a.judged).map((a) => a.title);
    if (waiting.length) return `${listJoin(waiting)} needs a few more dictations.`;
    return judgeLine(judge);
  }
  const gain = shape.axes.find((a) => a.id === shape.biggest_gain);
  if (gain) return `${gain.title} grew most in 4 weeks`;
  if (!shape.then_is_complete) return "Four weeks from now, you'll see how it moved";
  return "Holding steady over 4 weeks";
}

export function listJoin(items: string[]): string {
  type ListFormatCtor = new (locale?: string, opts?: { style: string; type: string }) => { format: (items: string[]) => string };
  const ListFormat = (Intl as unknown as { ListFormat?: ListFormatCtor }).ListFormat;
  try {
    if (ListFormat) return new ListFormat(undefined, { style: "long", type: "conjunction" }).format(items);
    throw new Error("no ListFormat");
  } catch {
    return items.length <= 1 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
  }
}

/** The coach introducing today's rep, in its own way. */
export function repIntro(persona: PersonaId, skillTitle: string, kind: RepKind): string {
  const what = lowerFirst(skillTitle);
  const redo = kind === "redo";
  switch (persona) {
    case "mentor":
      return redo ? `Let's take one of yours and work on ${what}.` : `Today, let's work on ${what}.`;
    case "editor":
      return `Today: ${what}.`;
    case "socratic":
      return `What would change if you focused on ${what} today?`;
    case "sergeant":
      return redo ? `One of yours, redone. Focus: ${what}.` : `Today's drill: ${what}. No excuses.`;
    case "peer":
      return redo ? `Want to redo one of yours? Let's try ${what}.` : `Quick one today: ${what}.`;
  }
}

export function progressLine(v: Verdict, minimumBaseline = 10): string {
  switch (v.status) {
    case "watching":
      if (v.before.length < minimumBaseline) return `Building a baseline: ${v.since_count} dictations so far`;
      return `${v.still_needed} more dictations to know`;
    case "working":
      return "It's working on your real dictations";
    case "notMoving":
      return "Not moving yet. Keep at it.";
  }
}

/** "3 hours ago", "yesterday"… */
export function relative(unix: number, now = Date.now() / 1000): string {
  const diff = unix - now;
  const abs = Math.abs(diff);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (abs < 60) return rtf.format(Math.round(diff), "second");
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86_400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 7 * 86_400) return rtf.format(Math.round(diff / 86_400), "day");
  if (abs < 30 * 86_400) return rtf.format(Math.round(diff / (7 * 86_400)), "week");
  return rtf.format(Math.round(diff / (30 * 86_400)), "month");
}

export function caughtLabel(q: Quote): string {
  const when = relative(q.created_at);
  return q.app_name ? `Caught in the wild · ${q.app_name}, ${when}` : `Caught in the wild · ${when}`;
}

export const monthDay = (unix: number) =>
  new Date(unix * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" });
export const monthYear = (unix: number) =>
  new Date(unix * 1000).toLocaleDateString(undefined, { month: "long", year: "numeric" });
export const monthName = (unix: number) => new Date(unix * 1000).toLocaleDateString(undefined, { month: "long" });
export const weekdayShort = (unix: number) => new Date(unix * 1000).toLocaleDateString(undefined, { weekday: "short" });
export const weekdayNarrow = (unix: number) => new Date(unix * 1000).toLocaleDateString(undefined, { weekday: "narrow" });

export function timeText(seconds: number): string {
  const total = Math.floor(seconds);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Words and sentences, as Coach counts them (Orttakey's Paragrapher). */
export function draftCounts(text: string): { words: number; sentences: number } {
  const words = text.split(/\s+/).filter(Boolean).length;
  const chars = Array.from(text);
  let count = 0;
  let current = "";
  chars.forEach((c, i) => {
    current += c;
    if (!".?!".includes(c)) return;
    const next = chars[i + 1];
    if (next === undefined || next === " ") {
      const after = chars.slice(i + 1).find((x) => x !== " ");
      if (after === undefined || /\p{Lu}|\p{N}/u.test(after) || "\"“'(".includes(after)) {
        if (current.trim().split(/\s+/).filter(Boolean).length > 0) count++;
        current = "";
      }
    }
  });
  if (current.trim().split(/\s+/).filter(Boolean).length > 0) count++;
  return { words, sentences: count };
}

// ---------------------------------------------------------------- voice ----

/** Partners' voices: a natural system voice of the right kind for each. */
const PARTNER_KIND: Record<string, "male" | "female"> = {
  ola: "male",
  atlas: "male",
  dami: "male",
  priya: "female",
  sam: "female",
};
const FEMALE = /(female|woman|zira|aria|jenny|sonia|libby|natasha|clara|emma|ava|samantha|karen|moira|tessa|serena|susan|hazel|heather|linda|joanna|salli|kimberly|ivy|kendra|amy|olivia|michelle|elizabeth|sara|nancy|michelle)/i;
const MALE = /(\bmale\b|\bman\b|david|mark|guy|ryan|thomas|daniel|alex|fred|george|james|christopher|eric|brian|roger|steffan|andrew|william|liam|tony|davis)/i;
const NATURAL = /(natural|neural|online|premium|enhanced|wavenet)/i;

function synth(): SpeechSynthesis | null {
  return typeof window !== "undefined" && "speechSynthesis" in window ? window.speechSynthesis : null;
}

let voicesCache: SpeechSynthesisVoice[] = [];
function loadVoices(): SpeechSynthesisVoice[] {
  const s = synth();
  if (!s) return [];
  const v = s.getVoices();
  if (v.length) voicesCache = v;
  return voicesCache;
}
if (synth()) {
  loadVoices();
  synth()!.addEventListener?.("voiceschanged", loadVoices);
}

/** English voices, natural ones first. */
export function availableVoices(): SpeechSynthesisVoice[] {
  const all = loadVoices();
  const english = all.filter((v) => v.lang.toLowerCase().startsWith("en"));
  const list = english.length ? english : all;
  return [...list].sort((a, b) => Number(NATURAL.test(b.name)) - Number(NATURAL.test(a.name)));
}

export const speechSupported = () => synth() !== null;

function pickVoice(preferred: string, speaker?: string): SpeechSynthesisVoice | null {
  const voices = availableVoices();
  if (!voices.length) return null;
  if (!speaker && preferred) {
    const chosen = voices.find((v) => v.name === preferred);
    if (chosen) return chosen;
  }
  if (speaker) {
    const kind = PARTNER_KIND[speaker.toLowerCase()];
    const want = kind === "female" ? FEMALE : kind === "male" ? MALE : null;
    const fits = want ? voices.filter((v) => want.test(v.name) && !(kind === "male" && FEMALE.test(v.name))) : voices;
    const pool = fits.length ? fits : voices;
    // The same partner always gets the same voice.
    const hash = Array.from(speaker).reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);
    const natural = pool.filter((v) => NATURAL.test(v.name));
    const from = natural.length ? natural : pool;
    return from[hash % from.length];
  }
  return voices[0];
}

type SpeakOptions = { speaker?: string; rate?: number; pitch?: number; voice?: string };

/** Speaks Coach's words with the WebView's voices (Mac `CoachVoice`). */
class CoachVoice {
  private queue: string[] = [];
  private speaking = false;
  private opts: SpeakOptions = {};
  private onIdle: (() => void)[] = [];
  private firstSound: (() => void) | null = null;
  private generation = 0;

  get isSpeaking() {
    return this.speaking || this.queue.length > 0;
  }

  /** Says it once, replacing anything already playing. */
  speak(text: string, opts: SpeakOptions = {}): Promise<void> {
    this.stop();
    this.opts = opts;
    this.enqueue(text);
    return this.idle();
  }

  /** Starts a reply that arrives sentence by sentence. */
  begin(opts: SpeakOptions = {}, onFirstSound?: () => void) {
    this.stop();
    this.opts = opts;
    this.firstSound = onFirstSound ?? null;
  }

  enqueue(sentence: string) {
    const text = sentence.trim();
    if (!text) return;
    this.queue.push(text);
    if (!this.speaking) this.next(this.generation);
  }

  /** Resolves once everything queued has been heard (or stopped). */
  idle(): Promise<void> {
    if (!this.isSpeaking) return Promise.resolve();
    return new Promise((r) => this.onIdle.push(r));
  }

  stop() {
    this.generation++;
    this.queue = [];
    this.speaking = false;
    this.firstSound = null;
    synth()?.cancel();
    this.flushIdle();
  }

  private flushIdle() {
    const waiting = this.onIdle;
    this.onIdle = [];
    waiting.forEach((r) => r());
  }

  private next(gen: number) {
    const s = synth();
    const text = this.queue.shift();
    if (gen !== this.generation) return;
    if (!text) {
      this.speaking = false;
      this.flushIdle();
      return;
    }
    this.speaking = true;
    const first = this.firstSound;
    this.firstSound = null;
    if (!s) {
      // No voices in this WebView (some Linux builds): read silently, at speaking pace.
      first?.();
      window.setTimeout(() => this.next(gen), Math.min(6000, 350 + text.split(/\s+/).length * 330));
      return;
    }
    const u = new SpeechSynthesisUtterance(text);
    const voice = pickVoice(this.opts.voice ?? "", this.opts.speaker);
    if (voice) u.voice = voice;
    u.rate = this.opts.rate ?? 0.95;
    u.pitch = this.opts.pitch ?? 1;
    u.onstart = () => first?.();
    const done = () => {
      if (gen === this.generation) this.next(gen);
    };
    u.onend = done;
    u.onerror = done;
    s.speak(u);
  }
}

export const coachVoice = new CoachVoice();

/** How a coach sounds, from the catalog's persona rate and pitch. */
export function personaVoice(catalog: Catalog | null, persona: PersonaId, settings: Settings | null): SpeakOptions {
  const p = catalog?.personas.find((x) => x.id === persona);
  return { rate: p?.rate ?? 0.95, pitch: p?.pitch ?? 1, voice: settings?.coach_voice ?? "" };
}

// ------------------------------------------------------ turn detection ----

export type TurnEvent = "none" | "started" | "ended" | "gaveUp";

/** Decides when you've finished speaking, from the mic level (Mac `TurnDetector`). */
export class TurnDetector {
  private calibration = 0.3;
  private startedAt: number | null = null;
  private patienceFrom: number | null = null;
  private floor: number[] = [];
  private threshold = 0.08;
  private lastLoud: number | null = null;
  private loudSince: number | null = null;
  private done = false;
  heardSpeech = false;
  speechStartedAt: number | null = null;

  constructor(
    public pause = 0.8,
    public patience = 12,
  ) {}

  restartPatience(time: number) {
    this.patienceFrom = time;
  }

  feed(level: number, time: number): TurnEvent {
    if (this.done) return "none";
    if (this.startedAt == null) this.startedAt = time;
    const elapsed = time - this.startedAt;
    if (elapsed < this.calibration) {
      this.floor.push(level);
      const sorted = [...this.floor].sort((a, b) => a - b);
      this.threshold = Math.max(0.08, sorted[Math.floor(sorted.length / 2)] * 3 + 0.03);
      return "none";
    }
    if (level >= this.threshold) {
      this.loudSince ??= time;
      this.lastLoud = time;
      if (!this.heardSpeech && time - this.loudSince >= 0.2) {
        this.heardSpeech = true;
        this.speechStartedAt = this.loudSince;
        return "started";
      }
    } else {
      this.loudSince = null;
    }
    if (this.heardSpeech && this.lastLoud != null && time - this.lastLoud >= this.pause) {
      this.done = true;
      return "ended";
    }
    if (!this.heardSpeech && time - (this.patienceFrom ?? this.startedAt) >= this.patience) {
      this.done = true;
      return "gaveUp";
    }
    return "none";
  }
}

/** Hands on each sentence of a reply as soon as it's complete (Mac `SentenceStream`). */
export class SentenceStream {
  private handedOn = 0;

  feed(text: string): string[] {
    const chars = Array.from(text);
    const out: string[] = [];
    let start = this.handedOn;
    for (let i = this.handedOn; i < chars.length; i++) {
      const next = chars[i + 1];
      if (".?!".includes(chars[i]) && (next === " " || next === "\n")) {
        const s = chars.slice(start, i + 1).join("").trim();
        if (Array.from(s).length > 1) out.push(s);
        start = i + 1;
      }
    }
    this.handedOn = start;
    return out;
  }

  finish(text: string): string | null {
    const chars = Array.from(text);
    if (this.handedOn >= chars.length) return null;
    const rest = chars.slice(this.handedOn).join("").trim();
    this.handedOn = chars.length;
    return rest || null;
  }
}

const QUOTES = /^["“”]+|["“”]+$/g;
/** A sentence ready to be spoken: stray quotation marks at its ends add nothing. */
export const spokenSentence = (s: string) => s.replace(QUOTES, "").trim();

/** The finished reply without a wrapping pair of quotes (Mac `cleaned`). */
export function cleanedReply(reply: string): string {
  const text = reply.trim();
  const chars = Array.from(text);
  if (!chars.length || !"\"“”".includes(chars[0])) return text;
  const inner = chars.slice(1);
  const close = inner.findIndex((c) => "\"“”".includes(c));
  if (close >= 0 && close === inner.length - 1) return inner.slice(0, close).join("").trim();
  return text;
}

// --------------------------------------------------------- share cards ----

/** Draws an SVG element to a PNG at its own size. */
export async function svgToPng(svg: SVGSVGElement, width: number, height: number): Promise<Uint8Array> {
  const clone = svg.cloneNode(true) as SVGSVGElement;
  clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  clone.setAttribute("width", String(width));
  clone.setAttribute("height", String(height));
  const xml = new XMLSerializer().serializeToString(clone);
  const url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(xml)}`;
  const img = new Image();
  await new Promise<void>((resolve, reject) => {
    img.onload = () => resolve();
    img.onerror = () => reject(new Error("Couldn't draw the card."));
    img.src = url;
  });
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Couldn't draw the card.");
  ctx.drawImage(img, 0, 0, width, height);
  const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
  if (!blob) throw new Error("Couldn't draw the card.");
  return new Uint8Array(await blob.arrayBuffer());
}

/** Copies a PNG to the clipboard where the WebView allows it. */
export async function copyPng(png: Uint8Array): Promise<boolean> {
  try {
    const Item = (window as unknown as { ClipboardItem?: typeof ClipboardItem }).ClipboardItem;
    if (!Item || !navigator.clipboard?.write) return false;
    await navigator.clipboard.write([new Item({ "image/png": new Blob([png as BlobPart], { type: "image/png" }) })]);
    return true;
  } catch {
    return false;
  }
}

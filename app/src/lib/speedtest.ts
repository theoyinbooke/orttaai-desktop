// The speed test (Settings › Model › Performance and the card that offers it):
// it runs in the background in the app, which reports progress and the result
// as events.
import type { SpeedTestRecord } from "./settings";

export type SpeedTestProgress = { threads: number; index: number; total: number; ms: number | null };

export type SpeedTestResult = {
  model: string;
  trials: { threads: number; ms: number }[];
  best: number;
  recommended: number;
  sample_seconds: number;
  /** Fluid Speech Mini with integer maths at `best` threads, when timed. */
  int8_math_ms: number | null;
  /** Whether integer maths was clearly faster, and is now used. */
  int8_math: boolean;
};

export type SpeedTestDone = { result?: SpeedTestResult; error?: string };

export type SpeedTestStatus = {
  running: boolean;
  progress: SpeedTestProgress | null;
  offer: boolean;
  last: SpeedTestRecord | null;
};

/** Seconds of sample speech each trial decodes (core `SAMPLE_SECONDS`). */
export const SAMPLE_SECONDS = 8;

export const seconds = (ms: number) => `${(ms / 1000).toFixed(2)} s`;

export const progressLabel = (p: SpeedTestProgress | null) =>
  p ? `${p.threads} threads… ${p.index + 1}/${p.total}` : "Starting…";

/** One line on a finished test, for the toast. */
export function doneMessage(r: SpeedTestResult): string {
  const best = r.trials.find((t) => t.threads === r.best);
  const int8 =
    r.int8_math && r.int8_math_ms != null && best
      ? ` Integer maths made it ${Math.round((1 - r.int8_math_ms / best.ms) * 100)}% faster again (${seconds(r.int8_math_ms)}), so it's on.`
      : "";
  return threadsMessage(r) + int8;
}

function threadsMessage(r: SpeedTestResult): string {
  const best = r.trials.find((t) => t.threads === r.best);
  const auto = r.trials.find((t) => t.threads === r.recommended);
  if (!best) return `Speed test done. Decode Threads is now ${r.best}.`;
  if (r.best === r.recommended || !auto)
    return `Speed test done: ${r.best} threads was already the fastest on this computer (${seconds(best.ms)} for ${r.sample_seconds} s of speech).`;
  const faster = Math.round((1 - best.ms / auto.ms) * 100);
  if (faster < 1)
    return `Speed test done: dictation now uses ${r.best} threads, as fast as ${r.recommended} on fewer cores (${seconds(best.ms)} for ${r.sample_seconds} s of speech).`;
  return `Speed test done: dictation now uses ${r.best} threads, ${faster}% faster than before (${seconds(best.ms)} instead of ${seconds(auto.ms)} for ${r.sample_seconds} s of speech).`;
}

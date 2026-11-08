// A rep, full screen (Mac CoachSessionView + CoachPracticeSession): what to
// do, then doing it by voice or by typing, then the reveal. Speaking uses
// dictation's own engine: the next dictation is captured as the rep instead
// of being typed anywhere. (Coach area.)
import { useCallback, useEffect, useRef, useState } from "react";
import "./practice.css";
import { Icon } from "../../ui";
import { useEvent } from "../../lib/api";
import { useEngine } from "../../lib/engine";
import { useSettings } from "../../lib/settings";
import {
  coachApi,
  coachVoice,
  draftCounts,
  monthDay,
  newToken,
  personaVoice,
  timeText,
  waitForCapture,
  type Capture,
  type Quote,
  type RepPlan,
  type RepResult,
} from "../../lib/coach";
import { CoachIcon } from "./icons";
import { Btn, Spinner, kindLimit, kindTitle, skillOf, useCoach } from "./parts";
import Reveal from "./Reveal";

type Phase =
  | { k: "ready" }
  | { k: "recording"; started: number }
  | { k: "scoring"; message: string }
  | { k: "revealed"; result: RepResult }
  | { k: "failed"; message: string };

const NO_MIC_HINT = " You can type your rep instead.";

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable;
}

function sourceLabel(q: Quote): string {
  const when = monthDay(q.created_at);
  return q.app_name ? `You said, to ${q.app_name} · ${when}` : `You said · ${when}`;
}

export default function RepSession({ plan, onPlan, onExit }: { plan: RepPlan; onPlan: (p: RepPlan) => void; onExit: () => void }) {
  const { catalog } = useCoach();
  const { settings } = useSettings();
  const { engine, level } = useEngine();
  const skill = skillOf(catalog, plan.skill_id);
  const limit = kindLimit(catalog, plan.kind);
  const canType = !skill.speak_only;

  const [phase, setPhase] = useState<Phase>({ k: "ready" });
  const [spoken, setSpoken] = useState(plan.spoken || skill.speak_only);
  const [typed, setTyped] = useState("");
  const [elapsed, setElapsed] = useState(0);

  // The capture we're waiting for, and whether we started the engine recording.
  const capture = useRef<{ token: number; cancel: () => void } | null>(null);
  const recordingByUs = useRef(false);
  const listening = useRef(false);
  const engineRef = useRef(engine);
  engineRef.current = engine;
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  const cancelCapture = useCallback(() => {
    capture.current?.cancel();
    capture.current = null;
    listening.current = false;
  }, []);

  // A new plan (Next rep): start over.
  useEffect(() => {
    cancelCapture();
    setPhase({ k: "ready" });
    setTyped("");
    setSpoken(plan.spoken || skill.speak_only);
  }, [plan, skill.speak_only, cancelCapture]);

  // Leaving: drop anything in flight.
  useEffect(
    () => () => {
      capture.current?.cancel();
      coachVoice.stop();
      if (recordingByUs.current && engineRef.current === "recording") {
        coachApi
          .discard()
          .then(() => coachApi.toggleRecording())
          .catch(() => {});
      } else {
        coachApi.disarm().catch(() => {});
      }
    },
    [],
  );

  const fail = useCallback(
    (message: string) => {
      cancelCapture();
      recordingByUs.current = false;
      coachApi.disarm().catch(() => {});
      setPhase({ k: "failed", message });
    },
    [cancelCapture],
  );

  const score = useCallback(
    async (heard: string, written: string, durationMs: number, wasSpoken: boolean) => {
      setPhase({ k: "scoring", message: wasSpoken ? "Listening back…" : "Reading it back…" });
      try {
        const result = await coachApi.scoreRep(plan, heard, written, durationMs, wasSpoken);
        setPhase({ k: "revealed", result });
        if (settings?.coach_read_aloud) coachVoice.speak(result.note, personaVoice(catalog, plan.persona, settings));
      } catch (e) {
        setPhase({ k: "failed", message: String(e) });
      }
    },
    [plan, settings, catalog],
  );

  useEvent<{ message: string }>("coach-rep-phase", (p) => {
    if (phaseRef.current.k === "scoring") setPhase({ k: "scoring", message: p.message });
  });

  useEvent<string>("engine-error", (message) => {
    if (phaseRef.current.k === "recording" || listening.current) fail(`${message}${NO_MIC_HINT}`);
  });

  const start = useCallback(async () => {
    if (engineRef.current === "off" || engineRef.current === "loading") {
      setPhase({ k: "failed", message: "Dictation isn't ready yet. Start the engine on Home, or type your rep." });
      return;
    }
    coachVoice.stop();
    cancelCapture();
    const token = newToken();
    const wait = waitForCapture(token);
    capture.current = { token, cancel: wait.cancel };
    listening.current = true;
    wait.promise.then((c: Capture) => {
      if (capture.current?.token !== token) return;
      capture.current = null;
      listening.current = false;
      recordingByUs.current = false;
      score(c.raw, c.text, c.duration_ms, true);
    });
    try {
      await coachApi.arm(token, true);
      await coachApi.toggleRecording();
      recordingByUs.current = true;
      setElapsed(0);
      setPhase({ k: "recording", started: Date.now() });
    } catch (e) {
      fail(`${String(e)}${NO_MIC_HINT}`);
    }
  }, [cancelCapture, fail, score]);

  const finish = useCallback(() => {
    if (phaseRef.current.k !== "recording") return;
    setPhase({ k: "scoring", message: "Listening back…" });
    coachApi.toggleRecording().catch((e) => fail(String(e)));
  }, [fail]);

  const startOver = useCallback(() => {
    cancelCapture();
    recordingByUs.current = false;
    coachApi
      .discard()
      .then(() => (engineRef.current === "recording" ? coachApi.toggleRecording() : undefined))
      .catch(() => {});
    setPhase({ k: "ready" });
  }, [cancelCapture]);

  // The clock, and the time limit.
  useEffect(() => {
    if (phase.k !== "recording") return;
    const started = phase.started;
    const id = window.setInterval(() => {
      const s = (Date.now() - started) / 1000;
      setElapsed(s);
      if (s >= limit) finish();
    }, 100);
    return () => window.clearInterval(id);
  }, [phase, limit, finish]);

  // Stopped with the dictation hotkey: the capture is on its way.
  useEffect(() => {
    if (phase.k === "recording" && engine === "processing") setPhase({ k: "scoring", message: "Listening back…" });
  }, [engine, phase.k]);

  // The engine finished but nothing reached Coach (nothing heard).
  useEffect(() => {
    if (engine !== "idle" || !listening.current || phase.k === "recording") return;
    const id = window.setTimeout(() => {
      if (listening.current) fail("Coach didn't hear that rep. Try again, or type it.");
    }, 2500);
    return () => window.clearTimeout(id);
  }, [engine, phase.k, fail]);

  const again = useCallback(() => {
    cancelCapture();
    coachVoice.stop();
    setTyped("");
    setPhase({ k: "ready" });
  }, [cancelCapture]);

  const counts = draftCounts(typed);
  const submitTyped = useCallback(() => {
    const text = typed.trim();
    if (counts.words < 3) return;
    score(text, text, 0, false);
  }, [typed, counts.words, score]);

  // Space speaks, Escape leaves.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (document.querySelector(".coach-pop")) return;
        e.preventDefault();
        onExit();
        return;
      }
      if (e.key !== " " || isTyping(e.target) || e.repeat) return;
      const k = phaseRef.current.k;
      if (k === "recording") {
        e.preventDefault();
        finish();
      } else if (k === "ready" && spoken) {
        e.preventDefault();
        start();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [spoken, start, finish, onExit]);

  const recording = phase.k === "recording";

  return (
    <div className="coach-focus coach-rep">
      <header className="coach-focus-head">
        <button type="button" className="coach-icon-btn" onClick={onExit} aria-label="Leave the studio">
          <Icon name="x" size={16} />
        </button>
        <div className="middle">
          {skill.title} · {kindTitle(catalog, plan.kind)}
        </div>
        {phase.k === "ready" && canType ? (
          <div className="coach-mode-switch" role="radiogroup" aria-label="Speak or type">
            <button type="button" role="radio" aria-checked={spoken} onClick={() => setSpoken(true)}>
              Speak
            </button>
            <button type="button" role="radio" aria-checked={!spoken} onClick={() => setSpoken(false)}>
              Type
            </button>
          </div>
        ) : (
          <span style={{ width: 44, height: 44 }} />
        )}
      </header>

      {phase.k === "revealed" && (
        <Reveal
          result={phase.result}
          onAgain={again}
          onNext={() => {
            coachVoice.stop();
            coachApi.nextRep(phase.result.plan).then(onPlan).catch((e) => setPhase({ k: "failed", message: String(e) }));
          }}
          onDone={onExit}
        />
      )}

      {phase.k === "scoring" && (
        <div className="coach-rep-center" role="status">
          <Spinner />
          <p className="coach-rep-status">{phase.message}</p>
        </div>
      )}

      {phase.k === "failed" && (
        <div className="coach-rep-center">
          <p className="coach-rep-failed">{phase.message}</p>
          <Btn kind="primary" onClick={again}>
            Try again
          </Btn>
        </div>
      )}

      {(phase.k === "ready" || phase.k === "recording") && (
        <div className="coach-rep-doing">
          <div className="coach-rep-brief">
            {plan.source && (
              <>
                <span className="coach-label">{sourceLabel(plan.source)}</span>
                <p className="coach-rep-source">“{plan.source.text}”</p>
              </>
            )}
            <h2 className={`coach-display coach-rep-instruction ${plan.source ? "" : "roomy"}`}>{plan.instruction}</h2>
          </div>

          {spoken ? (
            <div className="coach-rep-speaking">
              <button
                type="button"
                className={`coach-rep-mic ${recording ? "live" : ""}`}
                onClick={() => (recording ? finish() : start())}
                aria-label={recording ? "Finish and reveal" : "Start speaking"}
                style={{ ["--level" as string]: recording ? Math.min(1, level) : 0 }}
              >
                <svg className="coach-rep-ring" viewBox="0 0 280 280" aria-hidden="true">
                  <circle cx="140" cy="140" r="138" className="track" />
                  {recording && (
                    <circle
                      cx="140"
                      cy="140"
                      r="138"
                      className="progress"
                      pathLength={1}
                      strokeDasharray={`${Math.min(1, elapsed / limit)} 1`}
                    />
                  )}
                </svg>
                <span className="coach-rep-halo outer" aria-hidden="true" />
                <span className="coach-rep-halo inner" aria-hidden="true" />
                <span className="coach-rep-core" aria-hidden="true">
                  <CoachIcon name={recording ? "stopFill" : "micFill"} size={44} />
                </span>
              </button>
              <p className="coach-rep-time">
                {recording ? `${timeText(elapsed)} of ${timeText(limit)}` : "Press to start, or the space bar"}
              </p>
              <div className="coach-rep-actions">
                {recording ? (
                  <>
                    <Btn onClick={startOver}>Start over</Btn>
                    <Btn kind="primary" onClick={finish}>
                      Finish and reveal
                    </Btn>
                  </>
                ) : (
                  <span className="coach-label">Coach stays quiet until you finish.</span>
                )}
              </div>
            </div>
          ) : (
            <div className="coach-rep-typing">
              <div className="coach-rep-chips">
                <span className="coach-chip">
                  {counts.words} {counts.words === 1 ? "word" : "words"}
                </span>
                <span className="coach-chip">
                  {counts.sentences} {counts.sentences === 1 ? "sentence" : "sentences"}
                </span>
                <span className="coach-chip">{skill.drill}</span>
              </div>
              <textarea
                className="coach-rep-editor"
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    submitTyped();
                  }
                }}
                aria-label="Your attempt"
                autoFocus
              />
              <div className="coach-rep-footer">
                <span className="coach-label">Typed reps score what you wrote. Speak it to also hear your pace.</span>
                <Btn kind="primary" onClick={submitTyped} disabled={counts.words < 3}>
                  Reveal
                </Btn>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

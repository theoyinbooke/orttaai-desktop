// Setting Coach up in one go (Mac CoachModelSetupView / CoachModelSetupCenter):
// a card that walks through what a scene feels like and ends on the models
// Coach recommends for this machine's memory, downloaded through the person's
// own Ollama, the judge checked, and each job pointed at the right model.
import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Icon } from "../../ui";
import { useEvent } from "../../lib/api";
import {
  availableVoices,
  coachApi,
  OLLAMA_DOWNLOAD_URL,
  speechSupported,
  type JudgeState,
  type SetupStatus,
  type SetupStep,
  type StarterModel,
} from "../../lib/coach";
import { Btn, Spinner } from "./parts";

/** Opens the setup card the first time Coach opens without its models. */
export function useSetupOffer(when: boolean, open: () => void) {
  const asked = useRef(false);
  useEffect(() => {
    if (!when || asked.current) return;
    asked.current = true;
    coachApi
      .setupStatus()
      .then((s) => !s.is_set_up && open())
      .catch(() => {});
  }, [when, open]);
}

/** Setup's state, kept current as steps arrive. */
function useSetup() {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [checking, setChecking] = useState(true);
  const [step, setStep] = useState<SetupStep>({ step: "idle" });
  const [judge, setJudge] = useState<JudgeState | null>(null);
  const refresh = useCallback(async () => {
    setChecking(true);
    try {
      setStatus(await coachApi.setupStatus());
    } catch {
      setStatus(null);
    } finally {
      setChecking(false);
    }
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);
  useEvent<SetupStep>("coach-setup-step", (s) => {
    setStep(s);
    if (s.step === "done" || s.step === "failed") refresh();
  });
  useEvent<JudgeState>("coach-judge-state", (j) => {
    setJudge(j);
    if (j.state === "passed" || j.state === "failed") refresh();
  });
  const working = step.step === "downloading" || step.step === "checkingJudge" || judge?.state === "checking";
  const done = step.step === "done" || !!status?.is_set_up;
  const run = () => {
    setStep({ step: "downloading", model: status?.missing[0] ?? "", fraction: null });
    coachApi.runSetup().catch((e) => setStep({ step: "failed", message: String(e) }));
  };
  const actionTitle = done ? "All set" : working ? "Setting up…" : status && status.missing.length === 0 ? "Use these" : "Download and set up";
  const canAct = !!status?.server_running && !working && !done;
  const progress = (() => {
    if (step.step === "downloading") return step.fraction;
    if (judge?.state === "checking") return judge.done / Math.max(judge.total, 1);
    if (step.step === "checkingJudge") return 0;
    return done ? 1 : 0;
  })();
  return { status, checking, step, judge, working, done, run, refresh, actionTitle, canAct, progress };
}

type Setup = ReturnType<typeof useSetup>;

// ------------------------------------------------------------- the card ----

const PAGES = ["talk", "handsFree", "local", "setUp"] as const;

export function SetupHost({ atSetup, onClose }: { atSetup: boolean; onClose: () => void }) {
  const setup = useSetup();
  const [index, setIndex] = useState(atSetup ? 3 : 0);
  const page = PAGES[index];
  const last = page === "setUp";
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);
  const title = {
    talk: "Talk it through with Ola",
    handsFree: "Just talk. Pause when you're done.",
    local: "Two models, both on this computer",
    setUp: setup.done ? "You're ready to talk" : "Get everything in one go",
  }[page];
  const detail = {
    talk: "Scenes are a conversation. Ola, a client in Lagos, answers out loud, sentence by sentence as she thinks.",
    handsFree:
      "The microphone opens when Ola stops, and your turn ends when you pause. To cut in, press Space. Your coach scores the scene at the end.",
    local:
      "A quick model plays the people in scenes, so replies come in a blink. A careful one judges your reps and writes your notes. They run in Ollama, and nothing leaves this computer.",
    setUp: "",
  }[page];
  return (
    <div className="coach-setup-overlay" role="presentation">
      <div className="coach-setup-card" role="dialog" aria-modal="true" aria-label="Set up Coach">
        <div className="coach-setup-art">
          <button type="button" className="coach-setup-close" onClick={onClose} aria-label="Close setup" title="Close">
            <Icon name="x" size={13} />
          </button>
          <div className="coach-setup-scene" key={page}>
            {page === "talk" && <TalkScene />}
            {page === "handsFree" && <TurnScene />}
            {page === "local" && <LocalScene models={setup.status?.models ?? []} />}
            {page === "setUp" && <ProgressScene progress={setup.progress} done={setup.done} working={setup.working} />}
          </div>
        </div>
        <div className="coach-setup-body">
          <span className="coach-setup-eyebrow">{last ? "SET UP COACH" : `COACH · ${index + 1} OF ${PAGES.length}`}</span>
          <h2>{title}</h2>
          {last ? <ModelSetupList compact setup={setup} /> : <p>{detail}</p>}
          <div className="coach-setup-foot">
            <div className="coach-setup-pager" aria-label={`${index + 1} of ${PAGES.length}`}>
              {PAGES.map((p, i) => (
                <button key={p} type="button" className={i === index ? "on" : ""} onClick={() => setIndex(i)} aria-label={`Page ${i + 1}`} />
              ))}
            </div>
            <span className="coach-spacer" />
            {last ? (
              setup.done ? (
                <Btn kind="primary" compact onClick={onClose}>
                  Done
                </Btn>
              ) : (
                <>
                  <button type="button" className="coach-link quiet" onClick={onClose}>
                    Not now
                  </button>
                  <Btn kind="primary" compact disabled={!setup.canAct} onClick={setup.run}>
                    {setup.actionTitle}
                  </Btn>
                </>
              )
            ) : (
              <>
                <button type="button" className="coach-link quiet" onClick={() => setIndex(3)}>
                  Skip to setup
                </button>
                <Btn kind="primary" compact onClick={() => setIndex(index + 1)}>
                  Next
                </Btn>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ------------------------------------------------------------- the list ----

/** The recommended models and the voice, each with where it stands. Compact
 *  in the card; roomy, with its own button, in the welcome and Settings. */
export function ModelSetupList({ compact = false, setup: given }: { compact?: boolean; setup?: Setup }) {
  const own = useSetup();
  const setup = given ?? own;
  const { status, checking, step, judge } = setup;
  const voices = availableVoices();

  const modelStatus = (model: StarterModel) => {
    if (step.step === "downloading" && step.model === model.name) return <Progress fraction={step.fraction} label="Downloading" compact={compact} />;
    if (judge?.state === "checking" && judge.model === model.name)
      return <Progress fraction={judge.done / Math.max(judge.total, 1)} label="Checking" compact={compact} />;
    if (checking && !status) return <span className="coach-setup-status">Looking…</span>;
    if (!status?.installed.includes(model.name)) return <span className="coach-setup-status">{model.size}</span>;
    if (model.name === status.judge) {
      const g = status.judge_grade;
      if (g) {
        const passes = g.accuracy >= 0.8 && g.yes_recall >= 0.7 && g.no_recall >= 0.7 && g.answered >= g.total * 0.9;
        return (
          <span className="coach-setup-status" style={{ color: passes ? "var(--c-gain)" : "var(--c-warm-ink)" }}>
            {passes ? `Passed · ${Math.round(g.accuracy * 100)}%` : "Failed the check"}
          </span>
        );
      }
      return <span className="coach-setup-status">Judge check next</span>;
    }
    return <Ready />;
  };

  return (
    <div className={`coach-setup-list ${compact ? "compact" : ""}`}>
      {status && !status.server_running && (
        <ServerMissing compact={compact} provider={status.provider_name} canStart={status.supports_install} onRefresh={setup.refresh} />
      )}
      {(status?.models ?? []).map((m) => (
        <div key={m.name} className="coach-setup-row">
          <span className="icon">
            <Icon name={m.jobs.includes("Scene partner") ? "assistant" : "shield"} size={compact ? 14 : 18} />
          </span>
          <span className="text">
            <strong>{m.name}</strong>
            <span>{m.jobs.join(" · ")}</span>
          </span>
          {modelStatus(m)}
        </div>
      ))}
      <div className="coach-setup-row">
        <span className="icon">
          <Icon name="volume" size={compact ? 14 : 18} />
        </span>
        <span className="text">
          <strong>Voices</strong>
          <span>{speechSupported() && voices.length ? `This computer's voices · ${voices.length} available` : "Scene partners speak with this computer's voices"}</span>
        </span>
        {speechSupported() && voices.length ? <Ready /> : <span className="coach-setup-status">Text only here</span>}
      </div>
      {step.step === "failed" ? (
        <p className="coach-setup-message warn">{step.message}</p>
      ) : status && !status.supports_install ? (
        <p className="coach-setup-message">Download these in {status.provider_name}, then come back.</p>
      ) : null}
      {!compact && (
        <div className="coach-row" style={{ marginTop: 6 }}>
          <span className="coach-row" style={{ gap: 6, fontSize: 14, color: "var(--c-ink-3)" }}>
            <Icon name="shield" size={15} />
            Through {status?.provider_name ?? "Ollama"}, on this computer
          </span>
          <span className="coach-spacer" />
          <Btn kind="primary" disabled={!setup.canAct} onClick={setup.run}>
            {setup.actionTitle}
          </Btn>
        </div>
      )}
    </div>
  );
}

function ServerMissing({
  compact,
  provider,
  canStart,
  onRefresh,
}: {
  compact: boolean;
  provider: string;
  canStart: boolean;
  onRefresh: () => Promise<void>;
}) {
  const [starting, setStarting] = useState(false);
  const [installed, setInstalled] = useState<boolean | null>(null);
  const start = async () => {
    setStarting(true);
    const ok = await coachApi.startOllama().catch(() => false);
    setInstalled(ok);
    if (!ok) {
      openUrl(OLLAMA_DOWNLOAD_URL).catch(() => {});
      setStarting(false);
      return;
    }
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => window.setTimeout(r, 1000));
      const s = await coachApi.setupStatus().catch(() => null);
      if (s?.server_running) break;
    }
    await onRefresh();
    setStarting(false);
  };
  return (
    <div className={`coach-surface warm coach-setup-missing ${compact ? "compact" : ""}`}>
      <div className="coach-col" style={{ gap: 3, flex: 1 }}>
        <strong>{!canStart ? `${provider} isn't running` : installed === false ? "Coach's models run in Ollama" : "Ollama isn't running"}</strong>
        <span>
          {!canStart
            ? "Start its local server, then check again."
            : installed === false
              ? "It's free and runs models on this computer."
              : "Start it and Coach downloads the rest."}
        </span>
      </div>
      {canStart && (
        <Btn compact onClick={start} disabled={starting}>
          {starting ? "Starting…" : installed === false ? "Get Ollama" : "Start Ollama"}
        </Btn>
      )}
      <button type="button" className="coach-link" onClick={() => onRefresh()}>
        Check again
      </button>
    </div>
  );
}

function Ready() {
  return (
    <span className="coach-setup-status ready">
      <Icon name="check" size={14} />
      Ready
    </span>
  );
}

function Progress({ fraction, label, compact }: { fraction: number | null; label: string; compact: boolean }) {
  return (
    <span className="coach-setup-progress">
      {!compact && <span>{label}</span>}
      {fraction == null ? (
        <Spinner small />
      ) : (
        <>
          <span className="coach-bar" style={{ width: compact ? 70 : 110, height: 6 }}>
            <span style={{ width: `${Math.round(fraction * 100)}%` }} />
          </span>
          <span className="pct">{Math.round(fraction * 100)}%</span>
        </>
      )}
    </span>
  );
}

// ------------------------------------------------------------ pictures ----

function TalkScene() {
  return (
    <div className="coach-art-talk">
      <div className="disc">
        <span className="r1" />
        <span className="r2" />
        <span className="face coach-display">O</span>
      </div>
      <div className="bubbles">
        <div className="bubble partner">
          Can we also get the PDF export in by Friday?
          <span className="wave" aria-hidden="true">
            {[4, 9, 14, 8, 12, 5, 10, 15, 7, 11, 4].map((h, i) => (
              <i key={i} style={{ height: h }} />
            ))}
          </span>
        </div>
        <div className="bubble you">Friday for the calendar. PDF the week after?</div>
      </div>
    </div>
  );
}

function TurnScene() {
  const levels = [6, 10, 22, 34, 28, 40, 30, 18, 36, 26, 12, 5, 3, 3, 3, 3];
  return (
    <div className="coach-art-turn">
      <div className="levels" aria-hidden="true">
        {levels.map((h, i) => (
          <i key={i} className={i < 11 ? "on" : ""} style={{ height: h }} />
        ))}
        <span>pause</span>
      </div>
      <div className="pill">
        <span className="mic">
          <Icon name="mic" size={14} />
        </span>
        <span>Listening. Pause when you're done.</span>
        <kbd>Space</kbd>
      </div>
    </div>
  );
}

function LocalScene({ models }: { models: StarterModel[] }) {
  return (
    <div className="coach-art-local">
      <div className="chips">
        {models.map((m) => (
          <div key={m.name} className="chip">
            <Icon name={m.jobs.includes("Scene partner") ? "assistant" : "shield"} size={12} />
            <span>
              <strong>{m.name}</strong>
              <em>{m.jobs.join(" · ")}</em>
            </span>
          </div>
        ))}
      </div>
      <Icon name="arrowRight" size={14} />
      <div className="laptop" aria-hidden="true">
        <span className="screen">
          <Icon name="shield" size={20} />
        </span>
        <span className="base" />
      </div>
    </div>
  );
}

function ProgressScene({ progress, done, working }: { progress: number | null; done: boolean; working: boolean }) {
  const value = done ? 1 : progress ?? (working ? 0.08 : 0);
  const r = 42;
  const c = 2 * Math.PI * r;
  return (
    <svg className="coach-art-ring" viewBox="0 0 160 160" width={160} height={160} aria-hidden="true">
      <circle cx={80} cy={80} r={75} fill="none" stroke="var(--accent)" strokeOpacity={0.16} />
      <circle cx={80} cy={80} r={59} fill="none" stroke="var(--accent)" strokeOpacity={0.16} />
      <circle cx={80} cy={80} r={r} fill="none" stroke="var(--w-16)" strokeWidth={7} />
      <circle
        cx={80}
        cy={80}
        r={r}
        fill="none"
        stroke="var(--accent)"
        strokeWidth={7}
        strokeLinecap="round"
        strokeDasharray={`${c * value} ${c}`}
        transform="rotate(-90 80 80)"
        style={{ transition: "stroke-dasharray 0.3s ease-out" }}
      />
      {done ? (
        <path d="M66 81 L76 91 L95 70" fill="none" stroke="var(--accent)" strokeWidth={6} strokeLinecap="round" strokeLinejoin="round" />
      ) : (
        <path d="M80 66 L80 94 M68 83 L80 95 L92 83" fill="none" stroke="var(--accent)" strokeWidth={6} strokeLinecap="round" strokeLinejoin="round" />
      )}
    </svg>
  );
}

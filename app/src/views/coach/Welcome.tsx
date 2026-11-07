// Coach's first run (Mac CoachWelcomeView): what it found on this computer,
// which apps it may read, who coaches you, its models, and your first shape.
import { useEffect, useState } from "react";
import { Icon } from "../../ui";
import { coachApi, MODES, modeTitle, modeShort, percent, personaFor, monthYear, type AppRow, type Mode } from "../../lib/coach";
import { ModelSetupList } from "./ModelSetup";
import PersonaGlyph from "./PersonaGlyph";
import VoiceShape from "./VoiceShape";
import { Btn, Eyebrow, Loading, measureOf, Tag, useCoach } from "./parts";

const STEPS = 5;
const MODE_COLOR: Record<Mode, string> = {
  ai: "var(--accent)",
  people: "#B07A2E",
  writing: "#6B5532",
  elsewhere: "#4A4642",
};

export default function Welcome({ onDone }: { onDone: () => void }) {
  const { snapshot, refresh } = useCoach();
  const [step, setStep] = useState(1);
  const [installed, setInstalled] = useState<string[] | null>(null);
  useEffect(() => {
    if (step === 4) coachApi.setupStatus().then((s) => setInstalled(s.installed)).catch(() => setInstalled([]));
  }, [step]);
  useEffect(() => {
    refresh();
  }, [step, refresh]);
  return (
    <div className="coach-welcome">
      <header className="coach-welcome-head">
        <div className="coach-row" style={{ gap: 8 }}>
          <Eyebrow>Coach</Eyebrow>
          <Tag text="Beta" color="var(--accent)" />
        </div>
        <div className="coach-welcome-steps" role="img" aria-label={`Step ${step} of ${STEPS}`}>
          {Array.from({ length: STEPS }, (_, i) => (
            <span key={i} className={`${i + 1 <= step ? "on" : ""} ${i + 1 === step ? "current" : ""}`} />
          ))}
        </div>
        <button type="button" className="coach-link" style={{ fontSize: 14, fontWeight: 500 }} onClick={onDone}>
          Skip setup
        </button>
      </header>
      <div className="coach-welcome-body">
        {!snapshot ? (
          <Loading />
        ) : step === 1 ? (
          <Found />
        ) : step === 2 ? (
          <Apps />
        ) : step === 3 ? (
          <PickCoach />
        ) : step === 4 ? (
          <Models />
        ) : (
          <FirstShape />
        )}
      </div>
      <footer className="coach-welcome-foot">
        {step > 1 && <Btn onClick={() => setStep(step - 1)}>Back</Btn>}
        <span className="coach-spacer" />
        {step === 4 && (installed?.length ?? 0) === 0 && <Btn onClick={() => setStep(5)}>Start without them</Btn>}
        {step < STEPS ? (
          <Btn kind="primary" onClick={() => setStep(step + 1)}>
            {step === 1 ? "Show me" : "Continue"}
          </Btn>
        ) : (
          <Btn kind="primary" onClick={onDone}>
            Open Today
          </Btn>
        )}
      </footer>
    </div>
  );
}

function Found() {
  const { snapshot } = useCoach();
  const apps = snapshot.apps.filter((a) => a.key);
  const total = snapshot.apps.reduce((n, a) => n + a.dictations, 0);
  const first = snapshot.apps.map((a) => a.first_at).filter((t): t is number => t != null);
  const since = first.length ? `dictations since ${monthYear(Math.min(...first))}` : "dictations";
  const counts: Partial<Record<Mode, number>> = {};
  for (const a of snapshot.apps) {
    if (a.rule === "off") continue;
    counts[a.rule] = (counts[a.rule] ?? 0) + a.dictations;
  }
  const sum = Math.max(1, Object.values(counts).reduce((n, v) => n + (v ?? 0), 0));
  const split = MODES.map((m) => ({ mode: m, share: (counts[m] ?? 0) / sum })).filter((p) => p.share > 0);
  return (
    <div className="coach-welcome-found">
      <div className="coach-col" style={{ gap: 22, flex: 1, minWidth: 280 }}>
        <h1 className="coach-display coach-welcome-title" style={{ fontSize: 52 }}>
          Coach learns from how you already talk.
        </h1>
        <p className="coach-welcome-lead">No quizzes. It starts from the dictations already on this computer.</p>
      </div>
      <div className="coach-col" style={{ gap: 26, width: 400, maxWidth: "100%" }}>
        <BigStat value={total.toLocaleString()} label={since} />
        <BigStat value={String(apps.length)} label={apps.length === 1 ? "app you talk to" : "apps you talk to"} />
        {split.length > 0 && (
          <div className="coach-col" style={{ gap: 10 }}>
            <div className="coach-split-bar">
              {split.map((p) => (
                <span key={p.mode} style={{ flexGrow: p.share, background: MODE_COLOR[p.mode] }} />
              ))}
            </div>
            <div className="coach-row coach-wrap" style={{ gap: 18 }}>
              {split.map((p) => (
                <span key={p.mode} style={{ fontSize: 13, color: "var(--c-ink-2)" }}>
                  {modeShort[p.mode]} {percent(p.share)}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function BigStat({ value, label }: { value: string; label: string }) {
  return (
    <div className="coach-col" style={{ gap: 6 }}>
      <span className="coach-display" style={{ fontSize: 60, lineHeight: 1 }}>
        {value}
      </span>
      <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>{label}</span>
    </div>
  );
}

function Apps() {
  const { snapshot, refresh } = useCoach();
  const listed = snapshot.apps.filter((a) => a.key);
  const home = (a: AppRow): Mode => (a.rule === "off" ? a.builtin : a.rule);
  const toggle = (a: AppRow, on: boolean) => coachApi.setAppRule(a, on ? a.builtin : "off").then(refresh);
  return (
    <div className="coach-col" style={{ gap: 26, width: "100%" }}>
      <div className="coach-row coach-wrap" style={{ alignItems: "flex-end" }}>
        <h1 className="coach-display coach-welcome-title" style={{ fontSize: 46, flex: 1 }}>
          Choose what Coach may read.
        </h1>
        <span className="coach-row" style={{ gap: 6, fontSize: 15, color: "var(--c-body)" }}>
          <Icon name="shield" size={16} />
          Stays on this computer
        </span>
      </div>
      <div className="coach-welcome-apps">
        {MODES.map((mode) => {
          const members = listed.filter((a) => home(a) === mode);
          return (
            <section key={mode} className="coach-surface deep coach-col" style={{ padding: 20, gap: 4 }}>
              <span className="coach-label" style={{ paddingBottom: 6 }}>
                {modeTitle(mode)}
              </span>
              <div className="coach-welcome-applist">
                {members.length === 0 && <span style={{ fontSize: 14, color: "var(--c-muted)" }}>None yet</span>}
                {members.map((a) => (
                  <label key={a.key} className="coach-check">
                    <input type="checkbox" checked={a.rule !== "off"} onChange={(e) => toggle(a, e.target.checked)} />
                    <span className="coach-ellipsis">{a.display_name}</span>
                  </label>
                ))}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

function PickCoach() {
  const { catalog, settings } = useCoach();
  const chosen = personaFor(settings, "ai");
  const sample = catalog.personas.find((p) => p.id === chosen)?.sample ?? "";
  return (
    <div className="coach-col" style={{ gap: 30, width: "100%" }}>
      <div className="coach-row coach-wrap" style={{ alignItems: "flex-end" }}>
        <h1 className="coach-display coach-welcome-title" style={{ fontSize: 46, flex: 1 }}>
          Who should coach you?
        </h1>
        <span style={{ fontSize: 15, color: "var(--c-body)" }}>You can mix them later</span>
      </div>
      <div className="coach-welcome-coaches">
        {catalog.personas.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`coach-surface deep coach-plain coach-welcome-coach ${p.id === chosen ? "selected" : ""}`}
            aria-pressed={p.id === chosen}
            onClick={() => coachApi.setPersona(p.id, null)}
          >
            <PersonaGlyph persona={p.id} size={76} />
            <strong style={{ fontSize: 18 }}>{p.name}</strong>
            <span style={{ fontSize: 13, color: "var(--c-ink-2)" }}>{p.trait}</span>
          </button>
        ))}
      </div>
      <p style={{ fontSize: 19, color: "var(--c-ink-2)", margin: 0 }}>“{sample}”</p>
    </div>
  );
}

function Models() {
  return (
    <div className="coach-col" style={{ gap: 26, width: "100%" }}>
      <div className="coach-row coach-wrap" style={{ alignItems: "flex-end" }}>
        <h1 className="coach-display coach-welcome-title" style={{ fontSize: 46, flex: 1 }}>
          Give Coach its voice.
        </h1>
        <span style={{ fontSize: 15, color: "var(--c-body)" }}>Change these any time in Cast › Models.</span>
      </div>
      <ModelSetupList />
    </div>
  );
}

function FirstShape() {
  const { snapshot, catalog } = useCoach();
  const mode = snapshot.busiest_mode;
  const shape = snapshot.shapes[mode];
  const entries = Object.entries(shape.now) as [string, number][];
  // Swift's max/min(by:) keep the first on a tie.
  const strongest = entries.reduce<[string, number] | null>((b, e) => (!b || e[1] > b[1] ? e : b), null)?.[0];
  const weakest = entries.reduce<[string, number] | null>((b, e) => (!b || e[1] < b[1] ? e : b), null)?.[0];
  const room = snapshot.focus?.measure ?? weakest;
  const to = mode === "ai" ? "AI" : mode === "people" ? "people" : "the page";
  return (
    <div className="coach-welcome-found">
      <div className="coach-col" style={{ gap: 26, flex: 1, minWidth: 280 }}>
        <h1 className="coach-display coach-welcome-title" style={{ fontSize: 50 }}>
          {entries.length === 0 ? "Your shape starts with your next dictation." : `Here's how you talk to ${to}.`}
        </h1>
        {strongest && room && strongest !== room && (
          <div className="coach-row" style={{ gap: 40 }}>
            <div className="coach-col" style={{ gap: 6 }}>
              <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>Strongest</span>
              <strong style={{ fontSize: 24 }}>{measureOf(catalog, strongest).axis_title}</strong>
            </div>
            <div className="coach-col" style={{ gap: 6 }}>
              <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>Most room</span>
              <strong style={{ fontSize: 24, color: "var(--accent)" }}>{measureOf(catalog, room).axis_title}</strong>
            </div>
          </div>
        )}
        <p className="coach-welcome-lead">
          {shape.is_complete ? "It sharpens as you keep dictating." : "Keep dictating as usual; the rest fills in on its own."}
        </p>
      </div>
      <div style={{ width: 440, maxWidth: "100%" }}>
        <VoiceShape shape={shape} showThen={false} />
      </div>
    </div>
  );
}

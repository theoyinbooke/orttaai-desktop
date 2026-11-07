// The moment after a rep (Mac CoachRevealView): the score against your usual,
// the coach's note, your words before and after, each check as the judge or
// the count saw it, and what to do next. (Coach area.)
import { useState } from "react";
import "./practice.css";
import { Icon, useToast } from "../../ui";
import { coachApi, coachVoice, monthDay, personaVoice, type CheckResult, type RepResult } from "../../lib/coach";
import { CoachIcon } from "./icons";
import { Btn, InfoButton, Tag, checkLabel, personaOf, skillOf, trackOf, useCoach } from "./parts";
import PersonaGlyph from "./PersonaGlyph";

type Props = {
  result: RepResult;
  nextTitle?: string;
  onAgain: () => void;
  onNext: () => void;
  onDone: () => void;
};

export default function Reveal({ result, nextTitle = "Next rep", onAgain, onNext, onDone }: Props) {
  const { catalog, snapshot, settings, refresh } = useCoach();
  const toast = useToast();
  const [habitStarted, setHabitStarted] = useState(false);
  const skill = skillOf(catalog, result.plan.skill_id);
  const persona = personaOf(catalog, result.plan.persona);
  const habitTarget = skill.habit;
  const habitMode = trackOf(catalog, skill.track).mode ?? snapshot.busiest_mode;
  const alreadyHabit = !!habitTarget && snapshot.habits.some((h) => h.target === habitTarget);

  const startHabit = () => {
    if (!habitTarget) return;
    coachApi
      .startHabit(habitTarget, habitMode)
      .then(() => {
        setHabitStarted(true);
        refresh();
      })
      .catch((e) => toast(String(e), "error"));
  };

  const hear = () => coachVoice.speak(result.note, personaVoice(catalog, result.plan.persona, settings));

  return (
    <div className="coach-reveal">
      <div className="coach-reveal-inner">
        <div className="coach-reveal-top">
          <ScoreBlock result={result} />
          <div className="coach-surface warm coach-reveal-note">
            <PersonaGlyph persona={result.plan.persona} size={60} />
            <div className="coach-reveal-note-text">
              <span className="coach-reveal-persona">{persona.name.toUpperCase()}</span>
              <p>“{result.note}”</p>
            </div>
            <button type="button" className="coach-reveal-play" onClick={hear} aria-label={`Hear ${persona.name} say it`}>
              <CoachIcon name="playFill" size={18} />
            </button>
          </div>
        </div>

        {result.plan.source ? (
          <div className="coach-reveal-words two">
            <WordsCard title={`Before · ${monthDay(result.plan.source.created_at)}`} text={result.plan.source.text} warm={false} />
            <span className="coach-reveal-arrow" aria-hidden="true">
              <Icon name="arrowRight" size={20} />
            </span>
            <WordsCard title="After · just now" text={result.text} warm />
          </div>
        ) : (
          <div className="coach-reveal-words">
            <WordsCard title="What you said" text={result.text} warm />
          </div>
        )}

        <div className="coach-surface coach-pad coach-reveal-checks">
          <div className="coach-row" style={{ gap: 8 }}>
            <h3 className="coach-h3">How it was scored</h3>
            <InfoButton
              title="How it was scored"
              text={
                result.judge_model
                  ? `Counted checks are exact. The others were each asked as one yes or no question of ${result.judge_model} on this computer, with how sure it was. Your coach doesn't change the score.`
                  : "Only the counted checks ran. The rest need a local model (Ollama or LM Studio) that has passed the judge check in Cast › Models."
              }
            />
          </div>
          <div className="coach-reveal-grid">
            {result.checks.map((c) => (
              <CheckCell key={c.check} check={c} label={checkLabel(catalog, c.check)} isSkill={c.check === skill.check} />
            ))}
          </div>
          {result.figures.length > 0 && (
            <>
              <hr className="coach-reveal-rule" />
              <div className="coach-reveal-figures">
                {result.figures.map((f) => (
                  <div key={f.label} className="coach-reveal-figure">
                    <span className="coach-label">{f.label}</span>
                    <strong style={{ color: f.better === true ? "var(--c-gain)" : "var(--c-ink)" }}>{f.value}</strong>
                    {f.usual && <span className="coach-reveal-usual">usual {f.usual}</span>}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>

        <div className="coach-reveal-actions">
          <Btn onClick={onAgain}>Again</Btn>
          {habitTarget && (
            <Btn kind="primary" onClick={startHabit} disabled={habitStarted || alreadyHabit}>
              {habitStarted ? "Habit started" : "Make it a habit"}
            </Btn>
          )}
          <Btn onClick={onNext}>
            {nextTitle}
            <Icon name="chevronRight" size={16} />
          </Btn>
          <button type="button" className="coach-link quiet coach-reveal-done" onClick={onDone}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

function ScoreBlock({ result }: { result: RepResult }) {
  let tag;
  if (result.score != null && result.usual != null) {
    const change = result.score - result.usual;
    const text =
      change >= 0.05
        ? `+${change.toFixed(1)} on your usual`
        : change <= -0.05
          ? `${change.toFixed(1)} on your usual`
          : "Level with your usual";
    tag = <Tag text={text} color={change >= 0.05 ? "var(--c-gain)" : "var(--c-ink-2)"} />;
  } else {
    tag = <Tag text="Your first rep at this skill" color="var(--c-ink-2)" />;
  }
  return (
    <div className="coach-reveal-score">
      <div className="coach-row" style={{ alignItems: "baseline", gap: 10 }}>
        <span className="coach-display coach-reveal-big">{result.score != null ? result.score.toFixed(1) : "–"}</span>
        <span className="coach-reveal-of">/ 5</span>
      </div>
      {tag}
    </div>
  );
}

function WordsCard({ title, text, warm }: { title: string; text: string; warm: boolean }) {
  return (
    <div className={`coach-surface deep coach-reveal-card ${warm ? "warm-edge" : ""}`}>
      <span className="coach-label" style={{ color: warm ? "var(--c-warm-ink)" : undefined }}>
        {title}
      </span>
      <p className={warm ? "" : "before"}>“{text}”</p>
    </div>
  );
}

function CheckCell({ check, label, isSkill }: { check: CheckResult; label: string; isSkill: boolean }) {
  let text: string;
  let color: string;
  let fill: number;
  switch (check.answer) {
    case "yes":
      [text, color, fill] = ["Yes", "var(--c-gain)", check.confidence ?? 1];
      break;
    case "partly":
      [text, color, fill] = ["Partly", "var(--c-warm-ink)", check.confidence ?? 0.5];
      break;
    case "no":
      [text, color, fill] = ["Not yet", "var(--accent)", check.confidence ?? 1];
      break;
    default:
      [text, color, fill] = ["Not judged", "var(--c-muted)", 0];
  }
  return (
    <div className="coach-reveal-check">
      <div className="coach-reveal-check-label">
        <span style={{ fontWeight: isSkill ? 600 : 400 }}>{label}</span>
        {isSkill && (
          <span className="coach-reveal-target" aria-label="This skill" title="This skill">
            <Icon name="scope" size={13} />
          </span>
        )}
      </div>
      <div className="coach-bar thin">
        <span style={{ width: `${Math.round(fill * 100)}%`, background: color }} />
      </div>
      <span className="coach-reveal-caption">{check.confidence != null ? `${text} · ${check.confidence.toFixed(2)}` : text}</span>
    </div>
  );
}

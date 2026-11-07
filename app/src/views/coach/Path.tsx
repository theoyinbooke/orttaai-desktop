// Each track as a row of steps (Mac CoachPathView): mastered skills lit, the
// current one glowing, reviews flagged, and the reps for whichever skill you pick.
import { useState } from "react";
import { Icon } from "../../ui";
import type { RepKind, SkillInfo, SkillProgress, TrackId } from "../../lib/coach";
import { Btn, CoachPage, kindTitle, skillOf, trackOf, useCoach } from "./parts";

const EMPTY: SkillProgress = { reps: 0, strong_reps: 0, last_at: null, usual: null, mastered: false, review_due: false };
const PRACTICE: RepKind[] = ["redo", "drill", "timed"];
const KIND_ICON: Record<RepKind, string> = { redo: "restart", drill: "edit", timed: "flame", scene: "assistant" };

export default function PathView() {
  const { snapshot, catalog } = useCoach();
  const [track, setTrack] = useState<TrackId | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const current = track ?? skillOf(catalog, snapshot.todays_skill).track;
  const t = trackOf(catalog, current);
  const progress = snapshot.tracks.find((p) => p.track === current);
  const currentSkill = skillOf(catalog, progress?.current_skill ?? t.skills[0]);
  const skill = t.skills.includes(selected ?? "") ? skillOf(catalog, selected!) : currentSkill;
  const prog = (id: string) => snapshot.skill_progress[id] ?? EMPTY;

  return (
    <CoachPage
      eyebrow="Path"
      title={t.title}
      accessory={
        <div className="coach-path-tabs" role="tablist" aria-label="Tracks">
          {catalog.tracks.map((item) => {
            const p = snapshot.tracks.find((x) => x.track === item.id);
            const on = item.id === current;
            return (
              <button
                key={item.id}
                type="button"
                role="tab"
                aria-selected={on}
                onClick={() => {
                  setTrack(item.id);
                  setSelected(null);
                }}
              >
                <strong>{item.title}</strong>
                <span>{!p || p.mastered === 0 ? `${item.skills.length} skills` : `Level ${p.level} · ${p.mastered} of ${item.skills.length}`}</span>
              </button>
            );
          })}
        </div>
      }
    >
      <Journey skills={t.skills.map((id) => skillOf(catalog, id))} current={currentSkill.id} selected={skill.id} onSelect={setSelected} prog={prog} />
      <div className="coach-path-split">
        <RepsCard skill={skill} progress={prog(skill.id)} />
        <section className="coach-surface coach-pad coach-col" style={{ gap: 14 }}>
          <h2 className="coach-h2">This track</h2>
          <Stat value={String(progress?.level ?? 1)} label="level" />
          <Stat value={`${progress?.mastered ?? 0} of ${t.skills.length}`} label="skills mastered" />
          <Stat value={String(progress?.reps ?? 0)} label={progress?.reps === 1 ? "rep so far" : "reps so far"} />
          <p className="coach-label" style={{ margin: 0 }}>
            A skill is mastered after three reps scoring 4 or more with its own check met.
          </p>
        </section>
      </div>
    </CoachPage>
  );
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="coach-row" style={{ alignItems: "baseline", gap: 8 }}>
      <span className="coach-display" style={{ fontSize: 26 }}>
        {value}
      </span>
      <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>{label}</span>
    </div>
  );
}

/** The skills in order on one line: the part behind you solid, the part ahead dotted. */
function Journey({
  skills,
  current,
  selected,
  onSelect,
  prog,
}: {
  skills: SkillInfo[];
  current: string;
  selected: string;
  onSelect: (id: string) => void;
  prog: (id: string) => SkillProgress;
}) {
  const index = Math.max(0, skills.findIndex((s) => s.id === current));
  const step = 100 / skills.length;
  const first = step / 2;
  const reached = first + step * index;
  const last = first + step * (skills.length - 1);
  return (
    <div className="coach-journey">
      <svg className="coach-journey-line" viewBox="0 0 100 4" preserveAspectRatio="none" aria-hidden="true">
        <line x1={reached} y1={2} x2={last} y2={2} className="ahead" />
        <line x1={first} y1={2} x2={reached} y2={2} className="behind" />
      </svg>
      {skills.map((skill, i) => {
        const p = prog(skill.id);
        const isCurrent = skill.id === current;
        const label = `${skill.title}${p.mastered ? ", mastered" : ""}${p.review_due ? ", due for a review" : ""}${isCurrent ? ", you are here" : ""}`;
        return (
          <div key={skill.id} className="coach-journey-step">
            <button
              type="button"
              className={`node ${isCurrent ? "current" : p.mastered ? "mastered" : ""} ${skill.id === selected && !isCurrent ? "selected" : ""}`}
              onClick={() => onSelect(skill.id)}
              title={p.review_due ? `${skill.title} · due for a review` : skill.title}
              aria-label={label}
            >
              {p.mastered && !isCurrent ? <Icon name="check" size={16} /> : i + 1}
              {p.review_due && <span className="due" />}
            </button>
            <span className={`title ${isCurrent || p.mastered ? "strong" : ""}`}>{skill.title}</span>
            {isCurrent ? (
              <span className="here">You are here</span>
            ) : p.review_due ? (
              <span className="review">Review due</span>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

function RepsCard({ skill, progress }: { skill: SkillInfo; progress: SkillProgress }) {
  const { catalog, beginRep } = useCoach();
  return (
    <section className="coach-surface coach-pad coach-col" style={{ gap: 16 }}>
      <div className="coach-row coach-wrap">
        <h2 className="coach-h2" style={{ fontSize: 20 }}>
          {skill.title}
        </h2>
        <span className="coach-spacer" />
        <div className="coach-row" style={{ gap: 6 }}>
          {Array.from({ length: catalog.reps_to_master }, (_, i) => (
            <span key={i} className={`coach-dot ${i < progress.strong_reps ? "on" : ""}`} style={{ width: 10, height: 10, background: i < progress.strong_reps ? undefined : "var(--w-12)" }} />
          ))}
          <span className="coach-label">
            {progress.mastered ? "Mastered" : `${Math.min(progress.strong_reps, catalog.reps_to_master)} of ${catalog.reps_to_master} strong reps`}
          </span>
        </div>
      </div>
      <p style={{ margin: 0, fontSize: 15, color: "var(--c-ink-2)" }}>{skill.why}</p>
      <div className="coach-col" style={{ gap: 10 }}>
        {PRACTICE.map((kind) => (
          <div key={kind} className="coach-rep-row">
            <span className="icon">
              <Icon name={KIND_ICON[kind]} size={18} />
            </span>
            <div className="coach-col" style={{ gap: 2, flex: 1, minWidth: 0 }}>
              <strong style={{ fontSize: 15 }}>{kindTitle(catalog, kind)}</strong>
              <span className="coach-label coach-ellipsis">
                {kind === "drill" ? skill.drill : kind === "redo" ? "One of your own dictations, said better" : "A minute, from your own work"}
              </span>
            </div>
            <Btn kind="primary" onClick={() => beginRep(skill.id, kind, true)}>
              Speak
            </Btn>
            {!skill.speak_only && kind !== "timed" && <Btn onClick={() => beginRep(skill.id, kind, false)}>Type</Btn>}
          </div>
        ))}
      </div>
    </section>
  );
}

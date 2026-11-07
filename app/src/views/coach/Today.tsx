// Coach's home (Mac CoachTodayView): your voice shape for each way of
// talking, today's rep, the week, your habit or a real moment caught in the wild.
import { useState } from "react";
import { Icon } from "../../ui";
import {
  caughtLabel,
  greeting,
  insight,
  percent,
  personaFor,
  progressLine,
  repIntro,
  SHAPED,
  modeTitle,
  weekdayNarrow,
  type HabitState,
  type ShapedMode,
} from "../../lib/coach";
import PersonaGlyph from "./PersonaGlyph";
import VoiceShape from "./VoiceShape";
import { Btn, CoachPage, DotRow, Eyebrow, InfoButton, kindTitle, Lens, skillOf, StatusTag, trackOf, useCoach } from "./parts";

export default function Today({ onShowProof, onShareWeek }: { onShowProof: () => void; onShareWeek: () => void }) {
  const { snapshot } = useCoach();
  const [lens, setLens] = useState<ShapedMode | null>(null);
  const mode = lens ?? snapshot.busiest_mode;
  return (
    <CoachPage
      eyebrow="Coach"
      title={greeting(snapshot.built_at)}
      accessory={
        <Btn onClick={onShareWeek}>
          <Icon name="share" size={16} />
          Share this week
        </Btn>
      }
    >
      <div className="coach-split" style={{ ["--left" as string]: "500px" }}>
        <VoiceCard mode={mode} setLens={setLens} />
        <div className="coach-col" style={{ gap: 16 }}>
          <TodaysRepCard />
          <WeekCard />
          {snapshot.habits[0] && !snapshot.caught ? (
            <HabitCard state={snapshot.habits[0]} onShowProof={onShowProof} />
          ) : (
            <CaughtCard onShowProof={onShowProof} />
          )}
        </div>
      </div>
    </CoachPage>
  );
}

function VoiceCard({ mode, setLens }: { mode: ShapedMode; setLens: (m: ShapedMode) => void }) {
  const { snapshot, judge, catalog } = useCoach();
  const shape = snapshot.shapes[mode];
  const recent = snapshot.recent_by_mode[mode] ?? 0;
  const empty = shape.axes.every((a) => a.now == null);
  return (
    <section className="coach-surface coach-pad coach-col" style={{ gap: 10 }}>
      <div className="coach-row" style={{ gap: 6 }}>
        <h2 className="coach-h2">Your voice</h2>
        <InfoButton
          title="Your voice"
          text="Six things about your last 28 days of dictation in this way of talking, from 0 at the centre to best at the edge. The dashed outline is the 28 days before. Points marked with a check are judged by your local model, a few dictations a day; the rest are counted. Each point needs 8 dictations."
        />
      </div>
      <Lens
        label="Way of talking"
        value={mode}
        options={SHAPED.map((m) => ({ value: m, label: modeTitle(m) }))}
        onChange={(m) => m && setLens(m)}
      />
      <div className="coach-today-shape">
        <VoiceShape shape={shape} />
        {empty && (
          <div className="coach-today-count">
            <span className="coach-display">{recent}</span>
            <span className="coach-label">of {catalog.full_shape_dictations} dictations</span>
          </div>
        )}
      </div>
      <div className="coach-row coach-wrap" style={{ gap: 16 }}>
        <Legend title="Now" />
        {shape.axes.some((a) => a.then != null) && <Legend title="4 weeks ago" dashed />}
        <span className="coach-spacer" />
        <span className="coach-today-insight">{insight(shape, recent, judge, catalog.full_shape_dictations)}</span>
      </div>
    </section>
  );
}

export function Legend({ title, dashed }: { title: string; dashed?: boolean }) {
  return (
    <span className="coach-legend">
      <span className={dashed ? "dash" : "solid"} />
      <span style={{ color: dashed ? "var(--c-ink-3)" : "var(--c-ink-2)" }}>{title}</span>
    </span>
  );
}

function TodaysRepCard() {
  const { snapshot, catalog, settings, beginRep } = useCoach();
  const skill = skillOf(catalog, snapshot.todays_skill);
  const track = trackOf(catalog, skill.track);
  const persona = personaFor(settings, track.mode ?? snapshot.busiest_mode);
  return (
    <section className="coach-surface warm coach-today-rep">
      <div className="coach-row" style={{ gap: 6 }}>
        <Eyebrow>Today's rep</Eyebrow>
        <span className="coach-warm-label">
          · {track.title} · {kindTitle(catalog, snapshot.todays_kind)}
        </span>
      </div>
      <div className="coach-row" style={{ alignItems: "flex-start", gap: 14 }}>
        <PersonaGlyph persona={persona} size={44} />
        <p className="coach-today-intro">“{repIntro(persona, skill.title, snapshot.todays_kind)}”</p>
      </div>
      <div className="coach-row coach-wrap" style={{ marginTop: "auto" }}>
        <div className="coach-col" style={{ gap: 2, flex: 1, minWidth: 180 }}>
          <strong style={{ fontSize: 16 }}>{skill.title}</strong>
          <span className="coach-warm-label coach-ellipsis">{skill.why}</span>
        </div>
        <Btn kind="primary" onClick={() => beginRep(skill.id, snapshot.todays_kind, true)}>
          <Icon name="mic" size={16} />
          Speak it
        </Btn>
        {!skill.speak_only && <Btn onClick={() => beginRep(skill.id, snapshot.todays_kind, false)}>Type it</Btn>}
      </div>
    </section>
  );
}

function HabitCard({ state, onShowProof }: { state: HabitState; onShowProof: () => void }) {
  const { catalog } = useCoach();
  const v = state.verdict;
  return (
    <section className="coach-surface warm coach-today-habit">
      <div className="coach-row" style={{ gap: 6 }}>
        <Eyebrow>Your habit</Eyebrow>
        <span className="coach-warm-label">· {modeTitle(state.mode)}</span>
        <span className="coach-spacer" />
        <StatusTag status={v.status} />
      </div>
      <h3 className="coach-display" style={{ fontSize: 28, margin: 0 }}>
        {state.title}
      </h3>
      <div className="coach-row" style={{ alignItems: "baseline", gap: 10 }}>
        <span className="coach-display" style={{ fontSize: 24, color: "var(--c-muted)" }}>
          {percent(v.before_share)}
        </span>
        <span style={{ color: "var(--c-ink-3)" }}>→</span>
        <span className="coach-display" style={{ fontSize: 44, color: v.status === "working" ? "var(--c-gain)" : "var(--c-ink)" }}>
          {percent(v.after_share)}
        </span>
        <span className="coach-label">of dictations</span>
      </div>
      <DotRow hits={v.after} total={catalog.habit_window} />
      <div className="coach-row coach-wrap" style={{ marginTop: "auto" }}>
        <span className="coach-warm-label">{progressLine(v, catalog.minimum_baseline)}</span>
        <span className="coach-spacer" />
        <Btn kind="primary" onClick={onShowProof}>
          See proof
        </Btn>
      </div>
    </section>
  );
}

function WeekCard() {
  const { snapshot } = useCoach();
  return (
    <section className="coach-surface coach-today-week">
      <div className="coach-col" style={{ gap: 2, width: 130 }}>
        <span className="coach-label">This week</span>
        <span className="coach-row" style={{ gap: 6, alignItems: "baseline" }}>
          <span className="coach-display" style={{ fontSize: 26 }}>
            {snapshot.reps_this_week}
          </span>
          <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>{snapshot.reps_this_week === 1 ? "rep" : "reps"}</span>
        </span>
      </div>
      <div className="coach-week-days">
        {snapshot.week.map((day) => {
          const label = day.is_today ? "Today" : weekdayNarrow(day.date);
          const state = day.practised ? "practised" : day.dictated ? "dictated" : day.is_future ? "future" : "";
          return (
            <div
              key={day.date}
              className="coach-week-day"
              role="img"
              aria-label={`${label}: ${day.practised ? "practised" : day.dictated ? "dictated, no rep" : "nothing yet"}`}
            >
              <span className={`ring ${state}`} />
              <span className={day.is_today ? "today" : ""}>{label}</span>
            </div>
          );
        })}
      </div>
    </section>
  );
}

function CaughtCard({ onShowProof }: { onShowProof: () => void }) {
  const { snapshot } = useCoach();
  const caught = snapshot.caught;
  if (!caught) {
    return (
      <section className="coach-surface dashed coach-today-caught">
        <span style={{ color: "var(--c-muted)" }}>
          <Icon name="search" size={22} />
        </span>
        <div className="coach-col" style={{ gap: 4 }}>
          <strong style={{ fontSize: 16 }}>Caught in the wild</strong>
          <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>Your best real moments will show up here.</span>
        </div>
      </section>
    );
  }
  return (
    <button type="button" className="coach-surface coach-today-caught coach-plain" onClick={onShowProof}>
      <div className="coach-col" style={{ gap: 8, flex: 1, textAlign: "left" }}>
        <span className="coach-label">{caughtLabel(caught.quote)}</span>
        <span className="coach-quote coach-clamp-3">“{caught.quote.text}”</span>
      </div>
      {caught.streak >= 2 && (
        <div className="coach-col" style={{ gap: 2, alignItems: "flex-end" }}>
          <span className="coach-display" style={{ fontSize: 34, color: "var(--c-gain)" }}>
            {caught.streak}
          </span>
          <span className="coach-label">in a row</span>
        </div>
      )}
    </button>
  );
}

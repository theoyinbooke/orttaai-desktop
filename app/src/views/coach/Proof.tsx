// Proof (Mac CoachProofView): every habit as an experiment on real
// dictations — 20 before it started against the latest 20 since — with how
// much the skills nobody practised moved, so a good week doesn't take the credit.
import { Icon } from "../../ui";
import {
  coachApi,
  figure,
  modeTitle,
  monthDay,
  percent,
  points,
  progressLine,
  weekdayShort,
  type FocusPick,
  type HabitState,
} from "../../lib/coach";
import { Btn, CoachPage, DotRow, InfoButton, measureOf, StatusTag, useCoach } from "./parts";

export function habitFigure(state: HabitState, value: number | null): string {
  return state.measure ? figure(value, state.measure) : percent(value);
}

export default function Proof({ onShowMonth }: { onShowMonth: () => void }) {
  const { snapshot, refresh } = useCoach();
  const drift = snapshot.habits.map((h) => h.verdict.control_drift).find((d) => d != null);
  const stop = (s: HabitState) => coachApi.endHabit(s.habit.id).then(refresh);
  const [featured, ...rest] = snapshot.habits;
  return (
    <CoachPage
      eyebrow="Proof"
      title="In the wild"
      accessory={
        <>
          {drift != null && (
            <span className="coach-drift">
              <span>Skills you didn't practise moved</span>
              <strong>{points(drift)}</strong>
              <InfoButton
                title="A fair test"
                text="Over the same dictations, Coach checks the measures you weren't working on. A habit only counts as working when it moved more than they did."
              />
            </span>
          )}
          <Btn onClick={onShowMonth}>
            <Icon name="calendar" size={16} />
            Your month
          </Btn>
        </>
      }
    >
      {featured ? (
        <>
          <FeaturedHabit state={featured} onStop={() => stop(featured)} />
          {rest.length > 0 && (
            <div className="coach-grid" style={{ ["--min" as string]: "240px" }}>
              {rest.map((s) => (
                <HabitCard key={s.habit.id} state={s} onStop={() => stop(s)} />
              ))}
            </div>
          )}
          {snapshot.habits.length < 3 && snapshot.suggestions.length > 0 && (
            <Suggestions title="Try another" picks={snapshot.suggestions.slice(0, 3 - snapshot.habits.length)} />
          )}
        </>
      ) : (
        <>
          <section className="coach-surface coach-proof-empty">
            <div className="coach-col" style={{ gap: 7 }} aria-hidden="true">
              {[0, 1].map((r) => (
                <div key={r} className="coach-dots">
                  {Array.from({ length: 10 }, (_, i) => (
                    <span key={i} className="coach-dot empty" style={{ width: 11, height: 11 }} />
                  ))}
                </div>
              ))}
            </div>
            <div className="coach-col" style={{ gap: 10 }}>
              <span className="coach-display" style={{ fontSize: 22 }}>
                No habits yet.
              </span>
              <div className="coach-row coach-wrap" style={{ gap: 22 }}>
                {["Pick a habit", "Dictate as usual", "See if it stuck"].map((text, i) => (
                  <span key={text} className="coach-row" style={{ gap: 10 }}>
                    <span className="coach-step-num">{i + 1}</span>
                    <span style={{ fontSize: 14, color: "var(--c-ink-2)" }}>{text}</span>
                  </span>
                ))}
              </div>
            </div>
          </section>
          {snapshot.suggestions.length > 0 && <Suggestions title="Picked from your dictations" picks={snapshot.suggestions} highlightFirst />}
        </>
      )}
    </CoachPage>
  );
}

function FeaturedHabit({ state, onStop }: { state: HabitState; onStop: () => void }) {
  const { catalog } = useCoach();
  const v = state.verdict;
  return (
    <section className="coach-surface coach-featured" aria-label={`${state.title}, ${v.status}`}>
      <div className="coach-col" style={{ gap: 10 }}>
        <StatusTag status={v.status} />
        <div className="coach-col" style={{ gap: 2 }}>
          <strong style={{ fontSize: 19 }}>{state.title}</strong>
          <span className="coach-label">{modeTitle(state.mode)}</span>
        </div>
        <div className="coach-row" style={{ alignItems: "baseline", gap: 10 }}>
          <span className="coach-display" style={{ fontSize: 28, color: "var(--c-muted)" }}>
            {percent(v.before_share)}
          </span>
          <span style={{ color: "var(--c-ink-3)" }}>→</span>
          <span className="coach-display" style={{ fontSize: 30, color: v.status === "working" ? "var(--c-gain)" : "var(--c-ink)" }}>
            {percent(v.after_share)}
          </span>
        </div>
        <span className="coach-label">
          {v.since_count} dictations since {monthDay(state.habit.started_at)}
        </span>
        <button type="button" className="coach-link" onClick={onStop} style={{ alignSelf: "flex-start", marginTop: 4 }}>
          Stop this habit
        </button>
      </div>
      <div className="coach-col" style={{ gap: 14, minWidth: 0 }}>
        <div className="coach-col" style={{ gap: 8 }}>
          <span className="coach-label">{v.before.length} dictations before</span>
          <DotRow hits={v.before} total={catalog.habit_window} size={14} />
        </div>
        <div className="coach-col" style={{ gap: 8 }}>
          <span className="coach-label" style={{ color: "var(--c-warm-ink)" }}>
            {v.after.length ? `${v.after.length} since you started` : "Since you started"}
          </span>
          <DotRow hits={v.after} total={catalog.habit_window} size={14} />
        </div>
        <div className="coach-row coach-wrap" style={{ gap: 16 }}>
          <span className="coach-key">
            <span className="coach-dot on" style={{ width: 10, height: 10 }} />
            {state.hit_title}
          </span>
          <span className="coach-key">
            <span className="coach-dot" style={{ width: 10, height: 10 }} />
            Not this time
          </span>
        </div>
      </div>
      <div className="coach-proof-example">
        {state.example ? (
          <>
            <span className="coach-label">
              {state.example.app_name
                ? `Best example · ${state.example.app_name}, ${weekdayShort(state.example.created_at)}`
                : `Best example · ${weekdayShort(state.example.created_at)}`}
            </span>
            <span className="coach-quote coach-clamp-5" style={{ fontSize: 15 }}>
              “{state.example.text}”
            </span>
          </>
        ) : (
          <>
            <span className="coach-label">Best example</span>
            <span style={{ fontSize: 15, color: "var(--c-ink-3)" }}>Your first dictation that does it will show here.</span>
          </>
        )}
      </div>
    </section>
  );
}

function HabitCard({ state, onStop }: { state: HabitState; onStop: () => void }) {
  const { catalog } = useCoach();
  const v = state.verdict;
  const since = Math.min(v.since_count, catalog.habit_window);
  return (
    <section className="coach-surface coach-pad coach-col" style={{ gap: 10 }}>
      <div className="coach-row">
        <strong style={{ fontSize: 16, flex: 1 }}>{state.title}</strong>
        <StatusTag status={v.status} />
      </div>
      <span className="coach-label">{modeTitle(state.mode)}</span>
      <div className="coach-row coach-wrap" style={{ alignItems: "baseline", gap: 8 }}>
        <span className="coach-display" style={{ fontSize: 22, color: "var(--c-muted)" }}>
          {habitFigure(state, v.before_value)}
        </span>
        <span style={{ color: "var(--c-ink-3)" }}>→</span>
        <span className="coach-display" style={{ fontSize: 40 }}>
          {habitFigure(state, v.after_value)}
        </span>
        <span className="coach-label">{state.unit}</span>
      </div>
      <div className="coach-bar" role="img" aria-label={`${since} of ${catalog.habit_window} dictations since it started`}>
        <span style={{ width: `${(since / catalog.habit_window) * 100}%` }} />
      </div>
      <div className="coach-row">
        <span className="coach-label" style={{ flex: 1 }}>
          {progressLine(v, catalog.minimum_baseline)}
        </span>
        <button type="button" className="coach-link" onClick={onStop}>
          Stop
        </button>
      </div>
    </section>
  );
}

function Suggestions({ title, picks, highlightFirst }: { title: string; picks: FocusPick[]; highlightFirst?: boolean }) {
  const { catalog, refresh } = useCoach();
  return (
    <section className="coach-col">
      <h3 className="coach-h3">{title}</h3>
      <div className="coach-grid" style={{ ["--min" as string]: "220px" }}>
        {picks.map((pick, i) => {
          const m = measureOf(catalog, pick.measure);
          const on = !!highlightFirst && i === 0;
          return (
            <section key={`${pick.measure}-${pick.mode ?? "all"}`} className={`coach-surface ${on ? "warm" : ""} coach-suggestion`}>
              <span style={{ fontSize: 12, color: on ? "var(--c-warm-ink)" : "var(--c-ink-3)" }}>{modeTitle(pick.mode)}</span>
              <strong className="coach-ellipsis" style={{ fontSize: 15 }}>
                {m.habit_title}
              </strong>
              <div className="coach-row" style={{ alignItems: "baseline", gap: 6 }}>
                <span className="coach-display" style={{ fontSize: 28 }}>
                  {figure(pick.figure, pick.measure)}
                </span>
                <span className="coach-ellipsis" style={{ fontSize: 12, color: "var(--c-ink-3)" }}>
                  {m.unit} now
                </span>
              </div>
              <Btn kind={on ? "primary" : "secondary"} compact onClick={() => coachApi.startHabit(pick.measure, pick.mode).then(refresh)} style={{ alignSelf: "flex-start", marginTop: 4 }}>
                Start this habit
              </Btn>
            </section>
          );
        })}
      </div>
    </section>
  );
}

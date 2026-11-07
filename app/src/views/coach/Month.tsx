// Your month (Mac CoachMonthView): a calendar month looked back on and
// stepped back through — the numbers, the voice shape against the month
// before, what moved, practice and habits, and the month's best moment.
import { useEffect, useState } from "react";
import { Icon } from "../../ui";
import {
  coachApi,
  modeTitle,
  monthDay,
  monthName,
  monthYear,
  percent,
  points,
  SHAPED,
  type MonthMove,
  type MonthReport,
  type Shape,
  type ShapedMode,
} from "../../lib/coach";
import { MonthShareSheet } from "./Cards";
import { Legend } from "./Today";
import VoiceShape from "./VoiceShape";
import { Btn, CoachPage, InfoButton, Lens, Loading, skillOf, STATUS_COLOR, Tag, trackOf, useCoach } from "./parts";

export default function Month({ onBack }: { onBack: () => void }) {
  const { catalog } = useCoach();
  const [monthOf, setMonthOf] = useState(() => Math.floor(Date.now() / 1000));
  const [report, setReport] = useState<MonthReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lens, setLens] = useState<ShapedMode | null>(null);
  const [sharing, setSharing] = useState(false);

  useEffect(() => {
    let alive = true;
    setError(null);
    coachApi
      .monthReport(monthOf)
      .then((r) => alive && setReport(r))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [monthOf]);

  const start = report?.month.start ?? monthOf;
  const canBack = report?.first_month != null && report.first_month < start;
  const canForward = report ? !report.is_current : false;

  return (
    <CoachPage
      eyebrow="Your month"
      title={monthYear(start)}
      accessory={
        <>
          <div className="coach-stepper">
            <button type="button" aria-label="Previous month" disabled={!canBack} onClick={() => report && setMonthOf(report.month.start - 1)}>
              <Icon name="chevronLeft" size={15} />
            </button>
            <button type="button" aria-label="Next month" disabled={!canForward} onClick={() => report && setMonthOf(report.month.end)}>
              <Icon name="chevronRight" size={15} />
            </button>
          </div>
          {report && !report.is_empty && (
            <Btn onClick={() => setSharing(true)}>
              <Icon name="share" size={16} />
              Share
            </Btn>
          )}
          <Btn onClick={onBack}>Proof</Btn>
        </>
      }
    >
      {error ? (
        <div className="coach-notice">Coach couldn't work out this month: {error}</div>
      ) : !report ? (
        <Loading text="Looking back…" />
      ) : report.is_empty ? (
        <section className="coach-surface coach-col" style={{ padding: 28, gap: 10 }}>
          <span className="coach-display" style={{ fontSize: 26 }}>
            {report.is_current ? "This month has only just begun." : `Nothing to look back on in ${monthName(report.month.start)}.`}
          </span>
          <span style={{ fontSize: 15, color: "var(--c-ink-2)" }}>Dictate as usual or do a rep, and your month fills in here.</span>
        </section>
      ) : (
        <>
          <div className="coach-month-stats">
            <Stat value={String(report.dictations)} label="dictations" note={comparison(report.dictations, report.dictations_before)} />
            <Stat value={report.words.toLocaleString()} label="words" />
            <Stat value={String(report.days_dictated)} label={report.days_dictated === 1 ? "day dictated" : "days dictated"} />
            <Stat
              value={String(report.reps)}
              label={report.reps === 1 ? "rep" : "reps"}
              note={report.mastered_count > 0 ? `${report.mastered_count} mastered` : undefined}
            />
          </div>
          <ShapeRow report={report} lens={lens} setLens={setLens} />
          <div className="coach-month-pair">
            <section className="coach-surface coach-pad coach-col" style={{ gap: 14 }}>
              <h2 className="coach-h2">Practice</h2>
              {report.reps === 0 ? (
                <span style={{ fontSize: 15, color: "var(--c-ink-2)" }}>No reps this month.</span>
              ) : (
                <>
                  {report.tracks
                    .filter((t) => t.reps > 0)
                    .map((t) => (
                      <div key={t.track} className="coach-col" style={{ gap: 4 }}>
                        <div className="coach-row">
                          <strong style={{ fontSize: 15, flex: 1 }}>{trackOf(catalog, t.track).title}</strong>
                          <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>
                            {t.reps} {t.reps === 1 ? "rep" : "reps"}
                          </span>
                        </div>
                        {t.mastered.length > 0 && (
                          <span style={{ fontSize: 13, color: "var(--c-gain)" }}>
                            Mastered {t.mastered.map((id) => skillOf(catalog, id).title).join(", ")}
                          </span>
                        )}
                      </div>
                    ))}
                  {report.best_rep && (
                    <>
                      <hr className="coach-hr" />
                      <div className="coach-row" style={{ alignItems: "baseline", gap: 10 }}>
                        <span className="coach-display" style={{ fontSize: 30, color: "var(--accent)" }}>
                          {report.best_rep.score.toFixed(1)}
                        </span>
                        <div className="coach-col" style={{ gap: 2 }}>
                          <strong style={{ fontSize: 14 }}>Best rep: {skillOf(catalog, report.best_rep.skill).title}</strong>
                          <span className="coach-label">{monthDay(report.best_rep.created_at)}</span>
                        </div>
                      </div>
                    </>
                  )}
                </>
              )}
            </section>
            <section className="coach-surface coach-pad coach-col" style={{ gap: 14 }}>
              <div className="coach-row" style={{ gap: 6 }}>
                <h2 className="coach-h2">Habits</h2>
                <InfoButton
                  title="Habits this month"
                  text="Each habit that ran this month, judged on your dictations up to the month's end: the share that met it in the 20 before it started, and in the latest 20 since."
                />
              </div>
              {report.habits.length === 0 ? (
                <span style={{ fontSize: 15, color: "var(--c-ink-2)" }}>No habits ran this month.</span>
              ) : (
                report.habits.map((h) => (
                  <div key={h.habit.id} className="coach-row">
                    <div className="coach-col" style={{ gap: 2, flex: 1 }}>
                      <strong style={{ fontSize: 15 }}>{h.title}</strong>
                      <span style={{ fontSize: 13, color: "var(--c-ink-3)" }}>
                        {percent(h.verdict.before_share)} → {percent(h.verdict.after_share)} · {modeTitle(h.mode)}
                      </span>
                    </div>
                    {h.habit.ended_at == null ? (
                      <Tag text={{ watching: "Watching", working: "Working", notMoving: "Not moving" }[h.verdict.status]} color={STATUS_COLOR[h.verdict.status]} />
                    ) : (
                      <Tag text="Stopped" color="var(--c-muted)" />
                    )}
                  </div>
                ))
              )}
            </section>
          </div>
          {report.best_moment && (
            <section className="coach-surface warm coach-col" style={{ padding: 24, gap: 10 }}>
              <span className="coach-label" style={{ color: "var(--c-warm-ink)" }}>
                {[
                  "Best moment",
                  report.all_shape.axes.concat(...SHAPED.map((m) => report.shapes[m].axes)).find((a) => a.id === report.best_moment_axis)?.title,
                  report.best_moment.app_name,
                  monthDay(report.best_moment.created_at),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
              <span className="coach-clamp-5" style={{ fontSize: 19, fontWeight: 500, lineHeight: 1.4, userSelect: "text" }}>
                “{report.best_moment.text}”
              </span>
            </section>
          )}
        </>
      )}
      {sharing && report && <MonthShareSheet report={report} onClose={() => setSharing(false)} />}
    </CoachPage>
  );
}

function comparison(now: number, before: number): string | undefined {
  if (before <= 0) return undefined;
  const change = (now - before) / before;
  if (Math.abs(change) < 0.05) return "about the same as last month";
  return `${change > 0 ? "+" : ""}${Math.round(change * 100)}% on last month`;
}

function Stat({ value, label, note }: { value: string; label: string; note?: string }) {
  return (
    <section className="coach-surface coach-col" style={{ padding: 20, gap: 4 }}>
      <span className="coach-display" style={{ fontSize: 38 }}>
        {value}
      </span>
      <span style={{ fontSize: 14, color: "var(--c-ink-2)" }}>{label}</span>
      <span className="coach-label">{note ?? " "}</span>
    </section>
  );
}

function ShapeRow({ report, lens, setLens }: { report: MonthReport; lens: ShapedMode | null; setLens: (m: ShapedMode | null) => void }) {
  const shape: Shape = lens ? report.shapes[lens] : report.all_shape;
  const moves = report.moves.filter((m) => m.mode === lens);
  return (
    <section className="coach-surface coach-pad coach-col" style={{ gap: 14 }}>
      <div className="coach-row coach-wrap" style={{ gap: 6 }}>
        <h2 className="coach-h2">Your voice this month</h2>
        <InfoButton
          title="Your voice this month"
          text="The month so far, or the whole month once it's over, against the month before as the dashed outline. Points marked with a check are judged by your local model; the rest are counted. Each needs 8 dictations in the month."
        />
        <span className="coach-spacer" />
        <Lens
          small
          label="Way of talking"
          value={lens}
          options={[{ value: null, label: modeTitle(null) }, ...SHAPED.map((m) => ({ value: m as ShapedMode | null, label: modeTitle(m) }))]}
          onChange={setLens}
        />
      </div>
      <div className="coach-month-shape">
        <div className="shape">
          <VoiceShape shape={shape} />
        </div>
        <MovesList moves={moves} shape={shape} />
      </div>
      <div className="coach-row" style={{ gap: 16 }}>
        <Legend title="This month" />
        {shape.axes.some((a) => a.then != null) && <Legend title={monthName(report.previous.start)} dashed />}
      </div>
    </section>
  );
}

function MovesList({ moves, shape }: { moves: MonthMove[]; shape: Shape }) {
  return (
    <div className="coach-col" style={{ gap: 12, flex: 1, minWidth: 240 }}>
      <span className="coach-eyebrow" style={{ color: "var(--c-ink-3)" }}>
        What moved
      </span>
      {moves.length === 0 ? (
        <span style={{ fontSize: 15, color: "var(--c-ink-2)" }}>
          {shape.then_is_complete ? "Everything held within 3 points of last month." : "Last month didn't have enough dictations here to compare."}
        </span>
      ) : (
        moves.map((m) => (
          <div key={`${m.axis}-${m.mode ?? "all"}`} className="coach-row">
            <strong style={{ fontSize: 15, flex: 1 }}>{m.title}</strong>
            <span style={{ fontSize: 14, color: "var(--c-ink-3)" }}>
              {percent(m.then)} → {percent(m.now)}
            </span>
            <Tag text={points(m.change)} color={m.change > 0 ? "var(--c-gain)" : "var(--c-warm-ink)"} />
          </div>
        ))
      )}
    </div>
  );
}

// The share cards (Mac CoachWeekCard / CoachMonthCard): a headline, the voice
// shape and three numbers, never anything you said. Drawn as SVG with fixed
// dark colours so the saved PNG looks the same everywhere.
import { useRef, useState, type ReactNode } from "react";
import { Modal } from "../../ui";
import {
  coachApi,
  copyPng,
  modeTitle,
  monthDay,
  monthName,
  monthYear,
  percent,
  svgToPng,
  type HabitState,
  type MonthReport,
  type Shape,
} from "../../lib/coach";
import VoiceShape from "./VoiceShape";
import { Btn, useCoach } from "./parts";

export const CARD_W = 1080;
export const CARD_H = 1350;
const FONT = "Inter, 'Inter Variable', 'Segoe UI', system-ui, sans-serif";
const C = {
  ground: "#161517",
  ink: "#F4F1EC",
  ink3: "#A29E99",
  muted: "#8F8B86",
  accent: "#D4952A",
};
const SHAPE_PALETTE = {
  accent: C.accent,
  ring: "rgba(255,255,255,0.06)",
  ringOuter: "rgba(255,255,255,0.12)",
  ghost: C.muted,
  unknown: "rgba(255,255,255,0.18)",
  label: "#D9D5CF",
  labelMuted: C.muted,
  judged: "#E8B062",
};

type Part = { text: string; accent?: boolean };

/** Greedy word wrap by an estimated glyph width (Inter ≈ 0.53 em). */
function wrap(parts: Part[], size: number, width: number): Part[][] {
  const words: Part[] = parts.flatMap((p) => (p.text.match(/\S+\s*|\s+/g) ?? []).map((w) => ({ text: w, accent: p.accent })));
  const per = width / (size * 0.53);
  const lines: Part[][] = [[]];
  let used = 0;
  for (const w of words) {
    const len = w.text.trimEnd().length;
    if (used > 0 && used + len > per) {
      lines.push([]);
      used = 0;
    }
    lines[lines.length - 1].push(w);
    used += w.text.length;
  }
  return lines;
}

function Headline({ parts, y, size }: { parts: Part[]; y: number; size: number }) {
  const lines = wrap(parts, size, CARD_W - 160);
  return (
    <>
      {lines.map((line, i) => (
        <text key={i} x={80} y={y + i * size * 1.12} fontSize={size} fontWeight={600} fontFamily={FONT} fill={C.ink}>
          {line.map((w, j) => (
            <tspan key={j} fill={w.accent ? C.accent : C.ink}>
              {w.text}
            </tspan>
          ))}
        </text>
      ))}
    </>
  );
}

function headlineHeight(parts: Part[], size: number) {
  return wrap(parts, size, CARD_W - 160).length * size * 1.12;
}

function BrandBars({ x, y }: { x: number; y: number }) {
  // The Orttaai mark: four rounded bars, 42 × 36.
  const s = 36 / 24;
  return (
    <g transform={`translate(${x} ${y}) scale(${s})`} fill={C.accent}>
      <rect x={3} y={9} width={2.6} height={6} rx={1.3} />
      <rect x={8} y={5} width={2.6} height={14} rx={1.3} />
      <rect x={13.4} y={2.5} width={2.6} height={19} rx={1.3} />
      <rect x={18.8} y={8} width={2.6} height={8} rx={1.3} />
    </g>
  );
}

function CardFrame({
  cardRef,
  right,
  headline,
  shape,
  stats,
  footer,
  headlineSize,
}: {
  cardRef?: React.Ref<SVGSVGElement>;
  right: string;
  headline: Part[];
  shape: Shape;
  stats: [string, string][];
  footer: string;
  headlineSize: number;
}) {
  const top = 80;
  const headY = top + 36 + 40 + headlineSize;
  const shapeY = headY + headlineHeight(headline, headlineSize) - headlineSize * 0.2 + 30;
  const shapeSize = 540;
  const shapeW = shapeSize * (620 / 440);
  const shapeH = shapeSize * (480 / 440);
  const statsY = shapeY + shapeH + 30;
  return (
    <svg ref={cardRef} xmlns="http://www.w3.org/2000/svg" viewBox={`0 0 ${CARD_W} ${CARD_H}`} width={CARD_W} height={CARD_H}>
      <rect width={CARD_W} height={CARD_H} fill={C.ground} />
      <BrandBars x={80} y={top} />
      <text x={140} y={top + 27} fontSize={22} fontWeight={600} letterSpacing={2.6} fontFamily={FONT} fill={C.accent}>
        COACH
      </text>
      <text x={CARD_W - 80} y={top + 27} fontSize={22} textAnchor="end" fontFamily={FONT} fill={C.ink3}>
        {right}
      </text>
      <Headline parts={headline} y={headY} size={headlineSize} />
      <VoiceShape
        shape={shape}
        labelSize={20}
        palette={SHAPE_PALETTE}
        fontFamily={FONT}
        frame={{ x: (CARD_W - shapeW) / 2, y: shapeY, width: shapeW, height: shapeH }}
      />
      {stats.map(([value, label], i) => {
        const x = 80 + i * ((CARD_W - 160) / 3);
        return (
          <g key={label}>
            <text x={x} y={statsY + 60} fontSize={64} fontWeight={600} fontFamily={FONT} fill={C.ink}>
              {value}
            </text>
            <text x={x} y={statsY + 100} fontSize={22} fontFamily={FONT} fill={C.ink3}>
              {label}
            </text>
          </g>
        );
      })}
      <text x={80} y={CARD_H - 80} fontSize={20} fontFamily={FONT} fill={C.ink3}>
        {footer}
      </text>
      <text x={CARD_W - 80} y={CARD_H - 80} fontSize={20} textAnchor="end" fontFamily={FONT} fill={C.ink3}>
        Made with Orttaai
      </text>
    </svg>
  );
}

function habitPhrase(habit: HabitState): string {
  if (!habit.measure) return `met "${habit.title.toLowerCase()}"`;
  switch (habit.measure) {
    case "fillers":
      return "kept ums and uhs down";
    case "phrases":
      return "kept filler phrases out";
    case "pace":
      return "kept a steady pace";
    case "sentences":
      return "kept my sentences short";
    case "repeats":
      return "said each word once";
    case "clarity":
      return "said it right first time";
  }
}

export function WeekCard({ cardRef }: { cardRef?: React.Ref<SVGSVGElement> }) {
  const { snapshot } = useCoach();
  const habit = snapshot.habits.find((h) => h.habit.id === snapshot.headline_habit);
  const days = snapshot.week.filter((d) => d.dictated).length;
  const headline: Part[] =
    habit && habit.verdict.after_share != null
      ? [{ text: `I ${habitPhrase(habit)} in ` }, { text: percent(habit.verdict.after_share), accent: true }, { text: " of my recent dictations." }]
      : [{ text: "I talked instead of typed on " }, { text: `${days} ${days === 1 ? "day" : "days"}`, accent: true }, { text: " this week." }];
  const weekStart = snapshot.week[0]?.date ?? snapshot.built_at;
  return (
    <CardFrame
      cardRef={cardRef}
      right={`Week of ${monthDay(weekStart)}`}
      headline={headline}
      headlineSize={76}
      shape={snapshot.all_shape}
      stats={[
        [String(snapshot.dictations_this_week), "dictations"],
        [snapshot.words_this_week.toLocaleString(), "words"],
        [String(days), "days dictated"],
      ]}
      footer="Counted on my computer"
    />
  );
}

export function MonthCard({ report, cardRef }: { report: MonthReport; cardRef?: React.Ref<SVGSVGElement> }) {
  const gain = report.top_gain;
  const name = monthName(report.month.start);
  const shape = gain?.mode && gain.mode !== "elsewhere" ? report.shapes[gain.mode] : report.all_shape;
  const headline: Part[] = gain
    ? [{ text: gain.title, accent: true }, { text: ` grew most in ${name}: ${percent(gain.then)} to ${percent(gain.now)}.` }]
    : [
        { text: "I talked instead of typed on " },
        { text: `${report.days_dictated} ${report.days_dictated === 1 ? "day" : "days"}`, accent: true },
        { text: ` in ${name}.` },
      ];
  return (
    <CardFrame
      cardRef={cardRef}
      right={monthYear(report.month.start)}
      headline={headline}
      headlineSize={72}
      shape={shape}
      stats={[
        [String(report.dictations), "dictations"],
        [String(report.days_dictated), "days dictated"],
        [String(report.reps), "reps"],
      ]}
      footer={gain?.mode ? `Judged and counted on my computer · ${modeTitle(gain.mode)}` : "Counted on my computer"}
    />
  );
}

/** Preview a card, then save it as a PNG or copy it. */
export function CardShareSheet({
  title,
  fileName,
  onClose,
  render,
}: {
  title: string;
  fileName: string;
  onClose: () => void;
  render: (ref: React.Ref<SVGSVGElement>) => ReactNode;
}) {
  const ref = useRef<SVGSVGElement>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const png = async () => {
    if (!ref.current) throw new Error("Couldn't draw the card.");
    return svgToPng(ref.current, CARD_W, CARD_H);
  };
  const save = async () => {
    setBusy(true);
    try {
      const path = await coachApi.saveCard(fileName, await png());
      setFeedback(`Saved ${path}`);
    } catch (e) {
      setFeedback(`Couldn't save: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  const copy = async () => {
    setBusy(true);
    try {
      setFeedback((await copyPng(await png())) ? "Copied. Paste it into a post or a message." : "Copying images isn't available here. Save it instead.");
    } catch (e) {
      setFeedback(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal open onClose={onClose} labelledBy="coach-share-title">
      <div className="coach coach-share">
        <div className="coach-row">
          <h2 id="coach-share-title" className="coach-h2" style={{ flex: 1 }}>
            {title}
          </h2>
          <button type="button" className="coach-link quiet" onClick={onClose}>
            Done
          </button>
        </div>
        <div className="coach-share-preview" aria-label="Card preview" role="img">
          {render(ref)}
        </div>
        <div className="coach-row">
          <Btn onClick={save} disabled={busy}>
            Save image…
          </Btn>
          <Btn onClick={copy} disabled={busy}>
            Copy
          </Btn>
        </div>
        {feedback && <p className="coach-label coach-share-feedback">{feedback}</p>}
      </div>
    </Modal>
  );
}

const isoDay = (unix: number) => {
  const d = new Date(unix * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

export function WeekShareSheet({ onClose }: { onClose: () => void }) {
  const { snapshot } = useCoach();
  return (
    <CardShareSheet
      title="Share this week"
      fileName={`Orttaai-Coach-week-${isoDay(snapshot.built_at)}.png`}
      onClose={onClose}
      render={(ref) => <WeekCard cardRef={ref} />}
    />
  );
}

export function MonthShareSheet({ report, onClose }: { report: MonthReport; onClose: () => void }) {
  return (
    <CardShareSheet
      title="Share this month"
      fileName={`Orttaai-Coach-${isoDay(report.month.start).slice(0, 7)}.png`}
      onClose={onClose}
      render={(ref) => <MonthCard report={report} cardRef={ref} />}
    />
  );
}

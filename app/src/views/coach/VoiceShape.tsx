// The voice shape (Mac VoiceShapeView): six axes, now filled in amber over a
// dashed outline of four weeks ago. Axes without enough dictations yet show
// as grey points on the rim, and only the sides between known axes are
// drawn. Judged axes carry a small bubble with a check (judged by the local
// model). Drawn at its real size, so the Mac's radius, dots and 13 pt labels
// land where they do there. Hand-rolled SVG.
import type { Axis, Shape } from "../../lib/coach";

type Props = {
  shape: Shape;
  showThen?: boolean;
  showLabels?: boolean;
  /** The box it's drawn in; the shape fits the shorter side. */
  width?: number;
  height?: number;
  labelSize?: number;
  className?: string;
  /** Fixed colours, for share cards drawn outside the theme. */
  palette?: Partial<Palette>;
  /** Placement when nested inside another SVG (share cards). */
  at?: { x: number; y: number };
  /** Font for the labels when drawn outside the page (share cards). */
  fontFamily?: string;
};

type Palette = {
  accent: string;
  ring: string;
  ringOuter: string;
  ghost: string;
  unknown: string;
  label: string;
  labelMuted: string;
  judged: string;
};

const DEFAULTS: Palette = {
  accent: "var(--accent)",
  ring: "var(--w-06)",
  ringOuter: "var(--w-12)",
  ghost: "var(--c-muted)",
  unknown: "var(--w-18)",
  label: "var(--c-ink-2)",
  labelMuted: "var(--c-muted)",
  judged: "var(--c-warm-ink)",
};

/** How wide a label draws, so a judged axis's text and mark centre together. */
let measurer: CanvasRenderingContext2D | null = null;
function textWidth(text: string, size: number, family: string): number {
  measurer ??= document.createElement("canvas").getContext("2d");
  if (!measurer) return text.length * size * 0.56;
  measurer.font = `600 ${size}px ${family}`;
  return measurer.measureText(text).width;
}

export default function VoiceShape({
  shape,
  showThen = true,
  showLabels = true,
  width = 440,
  height = 440,
  labelSize = 13,
  className = "",
  palette,
  at,
  fontFamily,
}: Props) {
  const p = { ...DEFAULTS, ...palette };
  const axes = shape.axes;
  const n = Math.max(axes.length, 1);
  const side = Math.min(width, height);
  const cx = width / 2;
  const cy = height / 2;
  const radius = (side / 2) * (showLabels ? 0.68 : 0.92);

  const point = (i: number, value: number, r = radius) => {
    const angle = ((-90 + (360 / n) * i) * Math.PI) / 180;
    return [cx + r * value * Math.cos(angle), cy + r * value * Math.sin(angle)] as const;
  };
  const polygon = (values: number[]) =>
    values.map((v, i) => point(i, v)).map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ") + " Z";
  const sides = (get: (a: Axis) => number | null) => {
    let d = "";
    axes.forEach((a, i) => {
      const b = axes[(i + 1) % axes.length];
      const va = get(a);
      const vb = get(b);
      if (va == null || vb == null) return;
      const [x1, y1] = point(i, va);
      const [x2, y2] = point((i + 1) % axes.length, vb);
      d += `M${x1.toFixed(1)},${y1.toFixed(1)} L${x2.toFixed(1)},${y2.toFixed(1)} `;
    });
    return d;
  };

  const then = (a: Axis) => (showThen ? a.then : null);
  const allThen = showThen && axes.every((a) => a.then != null);
  const allNow = axes.every((a) => a.now != null);

  const label = axes
    .map((a) => {
      const name = a.judged ? `${a.title}, judged` : a.title;
      return a.now == null ? `${name}, not known yet` : `${name} ${Math.round(a.now * 100)}%`;
    })
    .join(", ");

  return (
    <svg
      className={`coach-shape ${className}`}
      viewBox={`0 0 ${width} ${height}`}
      width={width}
      height={height}
      {...at}
      role="img"
      aria-label={`Voice shape: ${label}`}
    >
      {[1, 2 / 3, 1 / 3].map((ring) => (
        <path
          key={ring}
          d={polygon(axes.map(() => ring))}
          fill="none"
          stroke={ring === 1 ? p.ringOuter : p.ring}
          strokeWidth={1}
        />
      ))}

      {allThen ? (
        <path
          d={polygon(axes.map((a) => a.then ?? 0))}
          fill="none"
          stroke={p.ghost}
          strokeWidth={1.5}
          strokeDasharray="4 5"
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      ) : (
        <>
          <path d={sides(then)} fill="none" stroke={p.ghost} strokeWidth={1.5} strokeDasharray="4 5" strokeLinecap="round" />
          {axes.map((a, i) => {
            const v = then(a);
            if (v == null) return null;
            const [x, y] = point(i, v);
            return <circle key={a.id} cx={x} cy={y} r={4} fill="none" stroke={p.ghost} strokeWidth={1.5} />;
          })}
        </>
      )}

      {allNow ? (
        <path
          d={polygon(axes.map((a) => a.now ?? 0))}
          fill={p.accent}
          fillOpacity={0.2}
          stroke={p.accent}
          strokeWidth={2.5}
          strokeLinejoin="round"
        />
      ) : (
        <path d={sides((a) => a.now)} fill="none" stroke={p.accent} strokeWidth={2.5} strokeLinecap="round" />
      )}

      {axes.map((a, i) => {
        const [x, y] = point(i, a.now ?? 1);
        return a.now != null ? (
          <g key={a.id}>
            <circle cx={x} cy={y} r={9} fill={p.accent} fillOpacity={0.2} />
            <circle cx={x} cy={y} r={5} fill={p.accent} />
          </g>
        ) : (
          <circle key={a.id} cx={x} cy={y} r={5} fill={p.unknown} />
        );
      })}

      {showLabels &&
        axes.map((a, i) => {
          const [x, y] = point(i, 1, radius + (34 * side) / 440);
          const fill = a.now == null ? p.labelMuted : p.label;
          if (!a.judged) {
            return (
              <text key={a.id} x={x} y={y} textAnchor="middle" dominantBaseline="middle" fontSize={labelSize} fontWeight={600} fontFamily={fontFamily} fill={fill}>
                {a.title}
              </text>
            );
          }
          // SwiftUI centres the text and its checkmark.bubble as one run.
          const mark = labelSize * 1.15;
          const gap = labelSize * 0.3;
          const family = fontFamily ?? getComputedStyle(document.body).fontFamily;
          const left = x - (textWidth(a.title, labelSize, family) + gap + mark) / 2;
          const markX = left + textWidth(a.title, labelSize, family) + gap;
          return (
            <g key={a.id}>
              <text x={left} y={y} dominantBaseline="middle" fontSize={labelSize} fontWeight={600} fontFamily={fontFamily} fill={fill}>
                {a.title}
              </text>
              <g
                transform={`translate(${markX} ${y - mark / 2}) scale(${mark / 24})`}
                fill="none"
                stroke={p.judged}
                strokeWidth={2.2}
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M6 3h12a4 4 0 0 1 4 4v8a4 4 0 0 1-4 4h-7l-5 3.5V19a4 4 0 0 1-4-4V7a4 4 0 0 1 4-4Z" />
                <path d="M8 11.2l2.8 2.8 5.2-5.6" />
              </g>
            </g>
          );
        })}
    </svg>
  );
}

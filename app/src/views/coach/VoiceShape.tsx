// The voice shape (Mac VoiceShapeView): six axes, now filled in amber over a
// dashed outline of four weeks ago. Axes without enough dictations yet show
// as grey points on the rim, and only the sides between known axes are
// drawn. Judged axes carry a small check mark (judged by the local model). Hand-rolled SVG.
import type { Axis, Shape } from "../../lib/coach";

type Props = {
  shape: Shape;
  showThen?: boolean;
  showLabels?: boolean;
  /** Label size at the drawing's own scale (440 units across). */
  labelSize?: number;
  className?: string;
  /** Fixed colours, for share cards drawn outside the theme. */
  palette?: Partial<Palette>;
  /** Placement when nested inside another SVG (share cards). */
  frame?: { x: number; y: number; width: number; height: number };
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

const SIDE = 440;

export default function VoiceShape({
  shape,
  showThen = true,
  showLabels = true,
  labelSize = 17,
  className = "",
  palette,
  frame,
  fontFamily,
}: Props) {
  const p = { ...DEFAULTS, ...palette };
  const axes = shape.axes;
  const n = Math.max(axes.length, 1);
  const padX = showLabels ? 90 : 0;
  const padY = showLabels ? 20 : 0;
  const width = SIDE + padX * 2;
  const height = SIDE + padY * 2;
  const cx = width / 2;
  const cy = height / 2;
  const radius = (SIDE / 2) * (showLabels ? 0.68 : 0.92);

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
      {...frame}
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
          const [x, y] = point(i, 1, radius + 34);
          const anchor = Math.abs(x - cx) < 4 ? "middle" : x < cx ? "end" : "start";
          return (
            <text
              key={a.id}
              x={x}
              y={y}
              textAnchor={anchor}
              dominantBaseline="middle"
              fontSize={labelSize}
              fontWeight={600}
              fontFamily={fontFamily}
              fill={a.now == null ? p.labelMuted : p.label}
            >
              {a.title}
              {a.judged && (
                <tspan fill={p.judged} dx={5} fontSize={labelSize * 0.95}>
                  {"\u2713"}
                </tspan>
              )}
            </text>
          );
        })}
    </svg>
  );
}

// Milestone badge, drawn the Mac way (MilestoneBadgeView): an outer ring (the
// tier colour once earned, an amber progress arc while in progress), a dotted
// tier ring of 40 ticks, and the badge's glyph. One geometry for the SVG
// (pages, toasts) and the canvas (share cards). Analytics/home area.
import { TIER_COLOR, type BadgeView, type Tier } from "../lib/analytics";

/** 24×24 stroke glyphs standing in for the Mac's SF Symbols. */
export const GLYPHS: Record<string, string> = {
  waveform: "M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2",
  "text.alignleft": "M4 6h16M4 10h11M4 14h16M4 18h9",
  "book.closed": "M6 3h12v18H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2ZM4 17a2 2 0 0 1 2-2h12M9 3v7l2-1.4 2 1.4V3",
  "books.vertical": "M3 4h4v16H3zM9 4h4v16H9zM15 5.2l3.8-1 3.2 14.6-3.8 1z",
  crown: "M3 8l4.5 4L12 5l4.5 7L21 8l-2 11H5L3 8Z",
  flame: "M12 3c1 3.5 5 5.5 5 10a5 5 0 0 1-10 0c0-2.2 1-3.6 2.2-4.6.2 1.6 1 2.6 2.1 3C11 9.2 11 6 12 3Z",
  "flame.fill": "M12 3c1 3.5 5 5.5 5 10a5 5 0 0 1-10 0c0-2.2 1-3.6 2.2-4.6.2 1.6 1 2.6 2.1 3C11 9.2 11 6 12 3Z",
  bolt: "M13 2 4 14h7l-1 8 9-12h-7l1-8Z",
  "bolt.fill": "M13 2 4 14h7l-1 8 9-12h-7l1-8Z",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18ZM12 7v5l3 2",
  "clock.badge.checkmark": "M19 12a8 8 0 1 1-6.5-7.9M11 7.5v4.5l3 1.8M15 5.5l2 2 4-4",
  "square.stack": "M7 8h11a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1ZM9 4.5h10.5a1.5 1.5 0 0 1 1.5 1.5v10",
  "square.stack.3d.up": "M12 3 3 7.5l9 4.5 9-4.5L12 3ZM3 12l9 4.5 9-4.5M3 16.5 12 21l9-4.5",
  "square.stack.3d.up.fill": "M12 3 3 7.5l9 4.5 9-4.5L12 3ZM3 12l9 4.5 9-4.5M3 16.5 12 21l9-4.5",
  "square.grid.2x2": "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z",
  stopwatch: "M12 6.5a7.25 7.25 0 1 0 0 14.5 7.25 7.25 0 0 0 0-14.5ZM12 10.5v3.5l2.5 1.5M9.5 2.5h5M12 2.5v4M18 6.5l1.6-1.6",
  "character.book.closed": "M6 3h12v18H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2ZM9.2 14.5l2.8-7.5 2.8 7.5M10.2 12h3.6",
  moon: "M20 13.5A8 8 0 1 1 10.5 4a6.2 6.2 0 0 0 9.5 9.5Z",
  mic: "M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM5 11a7 7 0 0 0 14 0M12 18v3",
  check: "M5 13l4 4L19 7",
};
/** Glyphs drawn solid (the Mac's `.fill` variants). */
const FILLED = new Set(["flame.fill", "bolt.fill", "square.stack.3d.up.fill"]);

export type BadgeLook = "unlocked" | "in_progress" | "locked";

const AMBER = "#D4952A";
const AMBER_TEXT = "#E8B45A";

function lookOf(b: Pick<BadgeView, "state">): BadgeLook {
  return b.state.kind;
}

function ringGeometry(size: number) {
  const tickR = size / 2 - size * 0.105;
  const period = (2 * Math.PI * tickR) / 40;
  return {
    border: size * 0.025,
    outerW: size * 0.033,
    tickR,
    tickW: size * 0.05,
    dash: [period * 0.25, period * 0.75] as [number, number],
    innerR: size / 2 - size * 0.175,
    glyph: size * 0.34,
  };
}

function colors(tier: Tier, look: BadgeLook, faint: string) {
  const tierColor = TIER_COLOR[tier];
  return {
    ticks: look === "unlocked" ? tierColor : AMBER,
    ticksOpacity: look === "unlocked" ? 1 : look === "in_progress" ? 0.15 : 0,
    icon: look === "unlocked" ? tierColor : look === "in_progress" ? AMBER_TEXT : faint,
  };
}

export function MilestoneBadge({
  badge,
  size = 72,
  forceUnlocked = false,
}: {
  badge: Pick<BadgeView, "tier" | "symbol" | "state" | "fraction" | "name">;
  size?: number;
  forceUnlocked?: boolean;
}) {
  const look: BadgeLook = forceUnlocked ? "unlocked" : lookOf(badge);
  const g = ringGeometry(size);
  // Follows the theme's accent on pages; share cards (canvas) stay the dark look.
  const c = { ...colors(badge.tier, look, "var(--tx3)"), ...(look === "unlocked" ? null : { ticks: "var(--accent)" }) };
  const half = size / 2;
  const arcR = half - g.outerW / 2;
  const circ = 2 * Math.PI * arcR;
  const fraction = look === "in_progress" ? badge.fraction : 0;
  const path = GLYPHS[badge.symbol] ?? GLYPHS.waveform;
  const gScale = g.glyph / 24;
  return (
    <svg
      className="ms-badge"
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      role="img"
      aria-label={badge.name}
      style={{ opacity: look === "locked" ? 0.75 : 1 }}
    >
      <circle cx={half} cy={half} r={half} fill="#26262A" />
      <circle cx={half} cy={half} r={half - g.border / 2} fill="none" stroke="var(--surface-3)" strokeWidth={g.border} />
      {look === "unlocked" && (
        <circle cx={half} cy={half} r={arcR} fill="none" stroke={TIER_COLOR[badge.tier]} strokeWidth={g.outerW} />
      )}
      {look === "in_progress" && (
        <circle
          cx={half}
          cy={half}
          r={arcR}
          fill="none"
          stroke="var(--accent)"
          strokeWidth={g.outerW}
          strokeLinecap="round"
          strokeDasharray={`${circ * fraction} ${circ}`}
          transform={`rotate(-90 ${half} ${half})`}
        />
      )}
      <circle
        cx={half}
        cy={half}
        r={g.tickR}
        fill="none"
        stroke={c.ticks}
        strokeWidth={g.tickW}
        strokeDasharray={g.dash.join(" ")}
        opacity={c.ticksOpacity}
      />
      <circle cx={half} cy={half} r={g.innerR} className="ms-badge-core" />
      <g
        transform={`translate(${half - g.glyph / 2} ${half - g.glyph / 2}) scale(${gScale})`}
        fill={FILLED.has(badge.symbol) ? c.icon : "none"}
        stroke={c.icon}
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d={path} />
      </g>
    </svg>
  );
}

/** Stroke (or fill) a glyph centred at (cx, cy), `box` px square. */
export function drawGlyph(
  ctx: CanvasRenderingContext2D,
  symbol: string,
  cx: number,
  cy: number,
  box: number,
  color: string,
  opts: { filled?: boolean; weight?: number } = {},
) {
  const p = new Path2D(GLYPHS[symbol] ?? GLYPHS.waveform);
  ctx.save();
  ctx.translate(cx - box / 2, cy - box / 2);
  ctx.scale(box / 24, box / 24);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.lineWidth = opts.weight ?? 1.8;
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  if (opts.filled ?? FILLED.has(symbol)) ctx.fill(p);
  ctx.stroke(p);
  ctx.restore();
}

/** The badge on a canvas, centred at (cx, cy); always the dark Mac look. */
export function drawBadge(
  ctx: CanvasRenderingContext2D,
  badge: Pick<BadgeView, "tier" | "symbol">,
  cx: number,
  cy: number,
  size: number,
) {
  const g = ringGeometry(size);
  const half = size / 2;
  const tier = TIER_COLOR[badge.tier];
  ctx.save();
  ctx.beginPath();
  ctx.arc(cx, cy, half, 0, Math.PI * 2);
  ctx.fillStyle = "#26262A";
  ctx.fill();
  ctx.beginPath();
  ctx.arc(cx, cy, half - g.border / 2, 0, Math.PI * 2);
  ctx.strokeStyle = "#3A3A3C";
  ctx.lineWidth = g.border;
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(cx, cy, half - g.outerW / 2, 0, Math.PI * 2);
  ctx.strokeStyle = tier;
  ctx.lineWidth = g.outerW;
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(cx, cy, g.tickR, -Math.PI / 2, Math.PI * 1.5);
  ctx.setLineDash(g.dash);
  ctx.lineWidth = g.tickW;
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.beginPath();
  ctx.arc(cx, cy, g.innerR, 0, Math.PI * 2);
  ctx.fillStyle = "#1C1C1E";
  ctx.fill();
  ctx.restore();
  drawGlyph(ctx, badge.symbol, cx, cy, g.glyph, tier);
}

/** The brand mark (Mac SignalCursorGlyph): three bars and a text cursor. */
export const BRAND_ASPECT = 100 / 84;
export function drawBrandMark(ctx: CanvasRenderingContext2D, x: number, y: number, height: number, color: string) {
  const scale = height / 84;
  const caps: [number, number, number, number][] = [
    [0, 44, 18, 36],
    [25, 28, 18, 52],
    [50, 12, 18, 68],
    [85, 3, 6, 78],
    [77, 0, 22, 6],
    [77, 78, 22, 6],
  ];
  ctx.save();
  ctx.fillStyle = color;
  for (const [cx, cy, w, h] of caps) {
    const rx = x + cx * scale;
    const ry = y + cy * scale;
    const rw = w * scale;
    const rh = h * scale;
    const r = Math.min(rw, rh) / 2;
    ctx.beginPath();
    ctx.roundRect(rx, ry, rw, rh, r);
    ctx.fill();
  }
  ctx.restore();
}

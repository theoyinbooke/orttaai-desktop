// Share cards drawn on a canvas at their exact export size (Mac
// ShareCards/*: Classic, Comparison, Milestone pictures). The preview and the
// saved PNG come from the same drawing. Cards show numbers only, never
// anything dictated. Analytics/home area.
import { useEffect, useRef } from "react";
// The card faces (already loaded app-wide; imported here so cards never
// fall back), plus true serif italic for Editorial and Books emphasis.
import "@fontsource-variable/inter";
import "@fontsource-variable/source-serif-4";
import "@fontsource-variable/source-serif-4/wght-italic.css";
import "@fontsource-variable/jetbrains-mono";
import {
  C,
  CARD_SIZES,
  CardContent,
  durationText,
  fmt,
  heightMetres,
  heightText,
  keyPressesText,
  multiple,
  nextLandmark,
  pages,
  secondsText,
  timeBackEquivalent,
  typingMinutes,
  wordsToReach,
  TIER_COLOR,
  type CardSize,
  type CardTemplate,
  type ShareStat,
} from "../lib/analytics";
import { BRAND_ASPECT, drawBadge, drawBrandMark, drawGlyph } from "./ShareCardBadge";

/* ---------------------------------------------------------------- palette -- */

const S = {
  charcoal: "#1C1C1E",
  night: "#141416",
  cream: "#F5F3F0",
  paper: "#EFE8DC",
  amber: "#D4952A",
  gold: "#E2B04A",
  grey: "#A1A1A6",
  dim: "#8E8E93",
  tile: "#232325",
  line: "#3A3A3C",
  soft: "#C7C7CC",
  tagline: "Giving you back both hands.",
  product: "Voice typing for your desktop",
};

const SANS = '"Inter Variable", "Inter", system-ui, sans-serif';
const SERIF = '"Source Serif 4 Variable", Georgia, serif';
const MONO = '"JetBrains Mono Variable", ui-monospace, monospace';

/* ---------------------------------------------------------- text toolkit -- */

type Family = "sans" | "serif" | "mono";
type Font = { size: number; weight?: number; family?: Family; italic?: boolean; tracking?: number };
type Run = { text: string; color: string; italic?: boolean };

function setFont(ctx: CanvasRenderingContext2D, f: Font) {
  const fam = f.family === "serif" ? SERIF : f.family === "mono" ? MONO : SANS;
  ctx.font = `${f.italic ? "italic " : ""}${f.weight ?? 400} ${f.size}px ${fam}`;
}

function width(ctx: CanvasRenderingContext2D, s: string, f: Font): number {
  setFont(ctx, f);
  const t = f.tracking ?? 0;
  return ctx.measureText(s).width + (t !== 0 ? t * Math.max(0, [...s].length - 1) : 0);
}

/** Draw one line whose top is `y`; returns its width. */
function line(
  ctx: CanvasRenderingContext2D,
  s: string,
  x: number,
  y: number,
  f: Font,
  color: string,
  align: "left" | "center" | "right" = "left",
): number {
  const w = width(ctx, s, f);
  const left = align === "center" ? x - w / 2 : align === "right" ? x - w : x;
  setFont(ctx, f);
  ctx.fillStyle = color;
  ctx.textBaseline = "alphabetic";
  ctx.textAlign = "left";
  const base = y + f.size * 0.8;
  const t = f.tracking ?? 0;
  if (t === 0) ctx.fillText(s, left, base);
  else {
    let cx = left;
    for (const ch of s) {
      ctx.fillText(ch, cx, base);
      cx += ctx.measureText(ch).width + t;
    }
  }
  return w;
}

/** The largest size ≤ `f.size` (≥ min × size) at which `s` fits `maxW`. */
function fit(ctx: CanvasRenderingContext2D, s: string, f: Font, maxW: number, minScale = 0.5): Font {
  const w = width(ctx, s, f);
  if (w <= maxW) return f;
  return { ...f, size: Math.max(f.size * minScale, (f.size * maxW) / w), tracking: (f.tracking ?? 0) * (maxW / w) };
}

type Laid = { runs: { text: string; color: string; italic?: boolean; w: number }[]; w: number }[];

/** Greedy word wrap over styled runs; "\n" forces a break. */
function layout(ctx: CanvasRenderingContext2D, runs: Run[], f: Font, maxW: number): Laid {
  const lines: Laid = [{ runs: [], w: 0 }];
  for (const run of runs) {
    const parts = run.text.split("\n");
    parts.forEach((part, pi) => {
      if (pi > 0) lines.push({ runs: [], w: 0 });
      const tokens = part.split(/(\s+)/).filter((t) => t.length > 0);
      for (const tok of tokens) {
        const rf = { ...f, italic: run.italic || f.italic };
        const w = width(ctx, tok, rf);
        let cur = lines[lines.length - 1];
        const isSpace = /^\s+$/.test(tok);
        if (!isSpace && cur.w + w > maxW && cur.runs.length > 0) {
          // Drop trailing space before the break.
          const last = cur.runs[cur.runs.length - 1];
          if (/^\s+$/.test(last.text)) {
            cur.runs.pop();
            cur.w -= last.w;
          }
          cur = { runs: [], w: 0 };
          lines.push(cur);
        }
        if (isSpace && cur.runs.length === 0) continue;
        cur.runs.push({ text: tok, color: run.color, italic: run.italic, w });
        cur.w += w;
      }
    });
  }
  return lines;
}

/** Draw wrapped rich text from top `y`; returns the height used. */
function rich(
  ctx: CanvasRenderingContext2D,
  runs: Run[],
  x: number,
  y: number,
  f: Font,
  maxW: number,
  lineHeight = 1.15,
  align: "left" | "center" | "right" = "left",
): number {
  const lines = layout(ctx, runs, f, maxW);
  lines.forEach((ln, i) => {
    let cx = align === "center" ? x - ln.w / 2 : align === "right" ? x - ln.w : x;
    for (const r of ln.runs) {
      line(ctx, r.text, cx, y + i * f.size * lineHeight, { ...f, italic: r.italic || f.italic }, r.color);
      cx += r.w;
    }
  });
  return lines.length * f.size * lineHeight;
}

function richHeight(ctx: CanvasRenderingContext2D, runs: Run[], f: Font, maxW: number, lineHeight = 1.15): number {
  return layout(ctx, runs, f, maxW).length * f.size * lineHeight;
}

/* ---------------------------------------------------------- shape toolkit -- */

function rr(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const rad = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.moveTo(x + rad, y);
  ctx.arcTo(x + w, y, x + w, y + h, rad);
  ctx.arcTo(x + w, y + h, x, y + h, rad);
  ctx.arcTo(x, y + h, x, y, rad);
  ctx.arcTo(x, y, x + w, y, rad);
  ctx.closePath();
}

function fillRR(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number, fill: string) {
  rr(ctx, x, y, w, h, r);
  ctx.fillStyle = fill;
  ctx.fill();
}

/** A stroke drawn inside the shape's edge (SwiftUI strokeBorder). */
function borderRR(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number, color: string, lw: number) {
  rr(ctx, x + lw / 2, y + lw / 2, w - lw, h - lw, Math.max(0, r - lw / 2));
  ctx.strokeStyle = color;
  ctx.lineWidth = lw;
  ctx.stroke();
}

function polygon(ctx: CanvasRenderingContext2D, pts: [number, number][], fill: string) {
  ctx.beginPath();
  pts.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
  ctx.closePath();
  ctx.fillStyle = fill;
  ctx.fill();
}

function hline(ctx: CanvasRenderingContext2D, x1: number, x2: number, y: number, color: string, lw: number, dash: number[] = []) {
  ctx.beginPath();
  ctx.setLineDash(dash);
  ctx.moveTo(x1, y);
  ctx.lineTo(x2, y);
  ctx.strokeStyle = color;
  ctx.lineWidth = lw;
  ctx.stroke();
  ctx.setLineDash([]);
}

/* ------------------------------------------------------------ card pieces -- */

/** Mark and name; returns the width. `y` is the top. */
function brand(ctx: CanvasRenderingContext2D, x: number, y: number, h: number, color = S.amber, textColor = S.cream, align: "left" | "center" = "left") {
  const markW = h * BRAND_ASPECT;
  const f: Font = { size: h * 0.8, weight: 600 };
  const total = markW + 14 + width(ctx, "Orttaai", f);
  const left = align === "center" ? x - total / 2 : x;
  drawBrandMark(ctx, left, y, h, color);
  line(ctx, "Orttaai", left + markW + 14, y + (h - f.size) / 2 - f.size * 0.04, f, textColor);
  return total;
}

function footer(ctx: CanvasRenderingContext2D, x1: number, x2: number, y: number, color = S.dim, taglineColor = S.soft, trailing = S.product) {
  line(ctx, S.tagline, x1, y, { size: 24, weight: 600 }, taglineColor);
  line(ctx, trailing, x2, y, { size: 24 }, color, "right");
}
const FOOTER_H = 30;

function topRow(ctx: CanvasRenderingContext2D, pad: number, W: number, y: number, label: string, labelColor: string, markH = 40, brandColors?: [string, string]) {
  brand(ctx, pad, y, markH, brandColors?.[0], brandColors?.[1]);
  line(ctx, label, W - pad, y + (markH - 22) / 2, { size: 22, weight: 700, tracking: 3 }, labelColor, "right");
}

/** A rounded stat tile; returns nothing (fixed height `h`). */
function tile(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  stat: ShareStat,
  opts: { accent: boolean; valueSize: number; fill: string; stroke?: string; pad?: number; labelColor?: string },
) {
  const pad = opts.pad ?? 24;
  fillRR(ctx, x, y, w, h, 22, opts.fill);
  borderRR(ctx, x, y, w, h, 22, opts.accent ? S.amber : opts.stroke ?? S.line, 1);
  const vf = fit(ctx, stat.value, { size: opts.valueSize, weight: 800 }, w - pad * 2, 0.5);
  line(ctx, stat.value, x + pad, y + pad, vf, opts.accent ? S.amber : S.cream);
  rich(ctx, [{ text: stat.label, color: opts.labelColor ?? S.soft }], x + pad, y + pad + opts.valueSize * 1.2 + 6, { size: 21 }, w - pad * 2, 1.2);
}

/* ------------------------------------------------------------------ cards -- */

type Ctx = { ctx: CanvasRenderingContext2D; c: CardContent; W: number; H: number; size: CardSize };

function signal({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = S.charcoal;
  ctx.fillRect(0, 0, W, H);
  // The brand mark, large and faint, behind everything.
  ctx.save();
  ctx.globalAlpha = 0.12;
  const markW = 560;
  const markH = markW / BRAND_ASPECT;
  drawBrandMark(ctx, W - markW + 70, H - markH - (sq ? 300 : 470), markH, S.amber);
  ctx.restore();

  const pad = 88;
  brand(ctx, pad, pad, 54);
  const capF: Font = { size: 22, weight: 700, tracking: 2.2 };
  const capW = width(ctx, "MILESTONE", capF) + 52;
  borderRR(ctx, W - pad - capW, pad + 1, capW, 52, 26, S.amber, 2);
  line(ctx, "MILESTONE", W - pad - capW / 2, pad + 1 + 15, capF, S.amber, "center");

  // Bottom up: footer, stats, then the hero block above them.
  const inner = W - pad * 2;
  let y = H - pad - FOOTER_H;
  footer(ctx, pad, W - pad, y);
  y -= sq ? 40 : 64;
  const statsH = 36 + 68 + 6 + 30;
  y -= statsH;
  hline(ctx, pad, W - pad, y, S.line, 2);
  const colW = inner / 3;
  c.supportingStats.forEach((st, i) => {
    const x = pad + i * colW;
    const vf = fit(ctx, st.value, { size: 56, weight: 800 }, colW - 16, 0.6);
    line(ctx, st.value, x, y + 36, vf, i === 2 ? S.amber : S.cream);
    line(ctx, st.label, x, y + 36 + 68 + 6, { size: 24 }, S.grey);
  });
  y -= sq ? 40 : 72;
  const supF: Font = { size: 32 };
  const supH = richHeight(ctx, [{ text: c.supportLine, color: S.grey }], supF, inner, 1.25);
  y -= supH;
  rich(ctx, [{ text: c.supportLine, color: S.grey }], pad, y, supF, inner, 1.25);
  y -= 14 + 64 * 1.2;
  line(ctx, c.hero.label, pad, y, { size: 64, weight: 700 }, S.cream);
  const heroF = fit(ctx, c.hero.value, { size: sq ? 200 : 260, weight: 900, tracking: -10 }, inner, 0.5);
  y -= 14 + heroF.size * 1.0;
  line(ctx, c.hero.value, pad, y, heroF, S.cream);
}

function editorial({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = S.cream;
  ctx.fillRect(0, 0, W, H);
  const pad = 96;
  const inner = W - pad * 2;
  const top: Font = { size: 22, weight: 600, tracking: 3 };
  line(ctx, c.eyebrow, pad, pad, top, S.charcoal);
  line(ctx, c.dateText.toUpperCase(), W - pad, pad, top, "#6E6E73", "right");
  ctx.fillStyle = S.charcoal;
  ctx.fillRect(pad, pad + 22 + 26, inner, 3);
  let y = pad + 22 + 26 + 3 + (sq ? 48 : 88);
  const [lead, em, tail] = c.headline;
  const hf: Font = { size: sq ? 92 : 112, family: "serif" };
  y += rich(ctx, [{ text: lead, color: S.charcoal }, { text: em, color: "#9A6315", italic: true }, { text: tail, color: S.charcoal }], pad, y, hf, inner, 1.08);
  y += 48;
  fillRR(ctx, pad, y + 14, 72, 6, 3, S.amber);
  rich(ctx, [{ text: c.supportLine, color: "#48484A" }], pad + 88, y, { size: 30 }, inner - 88, 1.25);

  // Bottom: brand row, then the stats strip above it.
  let by = H - pad - 44;
  brand(ctx, pad, by, 44, S.charcoal, S.charcoal);
  line(ctx, S.tagline, W - pad, by + 11, { size: 22 }, "#5A5A5E", "right");
  by -= 56 + 130;
  ctx.fillStyle = S.charcoal;
  ctx.fillRect(pad, by, inner, 2);
  const colW = inner / 3;
  c.supportingStats.forEach((st, i) => {
    const x = pad + i * colW + (i > 0 ? 24 : 0);
    if (i > 0) {
      ctx.fillStyle = "#C7C3BC";
      ctx.fillRect(pad + i * colW, by + 2, 1, 128);
    }
    const vf = fit(ctx, st.value, { size: 66, family: "serif" }, colW - 32, 0.6);
    line(ctx, st.value, x, by + 28, vf, S.charcoal);
    line(ctx, st.label, x, by + 28 + 66 * 1.15 + 10, { size: 22 }, "#5A5A5E");
  });
}

function medal({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = S.charcoal;
  ctx.fillRect(0, 0, W, H);
  const badge = c.medalBadge;
  const tier = badge ? TIER_COLOR[badge.tier] : S.gold;
  const bs = sq ? 340 : 440;
  const pad = 88;
  const name = badge?.name ?? "Orttaai";
  const nameF = fit(ctx, name, { size: 112, weight: 900, tracking: -3 }, W - pad * 2, 0.5);
  const textH = 26 * 1.2 + 16 + nameF.size * 1.1 + 18 + 36 * 1.2;
  const zone = bs + 60;
  const free = H - pad * 2 - 37 - zone - textH - FOOTER_H;
  const gap = Math.max(20, free / 3);
  brand(ctx, W / 2, pad, 37, S.amber, S.cream, "center");
  const cy = pad + 37 + gap + zone / 2;
  const k = bs / 440;
  ctx.save();
  for (const r of [300, 380, 460, 540]) {
    ctx.beginPath();
    ctx.arc(W / 2, cy, r * k, 0, Math.PI * 2);
    ctx.strokeStyle = "#2E2E31";
    ctx.lineWidth = 2;
    ctx.stroke();
  }
  ctx.globalAlpha = 0.5;
  for (let i = 0; i < 24; i++) {
    const long = i % 2 === 0;
    ctx.save();
    ctx.translate(W / 2, cy);
    ctx.rotate((i * 15 * Math.PI) / 180);
    const w = long ? 5 : 3;
    const h = (long ? 44 : 22) * k;
    const off = (long ? 262 : 252) * k;
    fillRR(ctx, -w / 2, -off - h / 2, w, h, w / 2, tier);
    ctx.restore();
  }
  ctx.restore();
  if (badge) drawBadge(ctx, badge, W / 2, cy, bs);
  let y = pad + 37 + gap + zone + gap;
  line(ctx, "BADGE UNLOCKED", W / 2, y, { size: 26, weight: 700, tracking: 5 }, tier, "center");
  y += 26 * 1.2 + 16;
  line(ctx, name, W / 2, y, nameF, S.cream, "center");
  y += nameF.size * 1.1 + 18;
  const sub = `${badge?.rule ?? "My stats so far"} · ${c.dateText}`;
  line(ctx, sub, W / 2, y, fit(ctx, sub, { size: 36 }, W - pad * 2, 0.6), S.soft, "center");
  line(ctx, `${S.tagline} Your voice never leaves your computer.`, W / 2, H - pad - 28, { size: 24 }, S.dim, "center");
}

function receipt({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = S.amber;
  ctx.fillRect(0, 0, W, H);
  const lines = sq ? c.receiptLines.filter(([k]) => k !== "PAGES OF A BOOK" && k !== "HEIGHT, STOOD UP") : c.receiptLines;
  const PW = 780;
  const padX = 64;
  const ink = S.charcoal;
  const greyInk = "#5A5A5E";
  const mono = (s: number, w = 400, t = 0): Font => ({ size: s, weight: w, family: "mono", tracking: t });
  // Measure the paper.
  const head = 60 + 12 + 40 * 1.2 + 12 + 22 * 1.2 + 12 + 20 * 1.2;
  const rows = lines.length * (26 * 1.2 + 24);
  const totals = 64 * 1.1 + 22 * 1.3;
  const stamp = 28 + 26 * 1.2 + 24 + 10;
  const bar = sq ? 0 : 30 + 84;
  const tailH = 18 + 20 * 1.2 + 6 + 18 * 1.2;
  const paperH = (sq ? 40 : 60) + head + 30 + 3 + 22 + rows + 22 + 3 + 22 + totals + stamp + bar + tailH + 34;
  const topPad = sq ? 48 : 84;
  ctx.save();
  ctx.translate(W / 2, topPad + paperH / 2);
  ctx.rotate((-1.5 * Math.PI) / 180);
  ctx.translate(-PW / 2, -paperH / 2);
  ctx.shadowColor = "rgba(28,28,30,0.35)";
  ctx.shadowBlur = 30;
  ctx.shadowOffsetY = 26;
  ctx.fillStyle = "#FBF8F2";
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.lineTo(PW, 0);
  ctx.lineTo(PW, paperH);
  let zx = PW;
  let down = true;
  while (zx > 0) {
    zx -= 13;
    ctx.lineTo(Math.max(0, zx), down ? paperH + 20 : paperH + 2);
    down = !down;
  }
  ctx.lineTo(0, paperH);
  ctx.closePath();
  ctx.fill();
  ctx.shadowColor = "transparent";

  const L = padX;
  const R = PW - padX;
  const mid = PW / 2;
  let y = sq ? 40 : 60;
  drawBrandMark(ctx, mid - (60 * BRAND_ASPECT) / 2, y, 60, ink);
  y += 60 + 12;
  line(ctx, "ORTTAAI", mid, y, mono(40, 800, 7), ink, "center");
  y += 40 * 1.2 + 12;
  line(ctx, "DICTATION RECEIPT", mid, y, mono(22, 400, 2.6), ink, "center");
  y += 22 * 1.2 + 12;
  line(ctx, `No. ${String(c.stats.dictations).padStart(4, "0")} · ${c.dateText.toUpperCase()}`, mid, y, mono(20), greyInk, "center");
  y += 20 * 1.2 + 30;
  hline(ctx, L, R, y, ink, 3, [10, 8]);
  y += 3 + 22;
  for (const [k, v] of lines) {
    y += 12;
    const kw = line(ctx, k, L, y, mono(26), ink);
    const vw = line(ctx, v, R, y, mono(26, 700), ink, "right");
    ctx.save();
    ctx.setLineDash([0.1, 9]);
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(L + kw + 12, y + 20);
    ctx.lineTo(R - vw - 12, y + 20);
    ctx.strokeStyle = "#9A9A9E";
    ctx.lineWidth = 3;
    ctx.stroke();
    ctx.restore();
    y += 26 * 1.2 + 12;
  }
  y += 22;
  hline(ctx, L, R, y, ink, 3, [10, 8]);
  y += 3 + 22;
  const big = `${fmt(c.minutesSavedOnCard)} MIN`;
  line(ctx, "TIME BACK", L, y + 64 * 0.8 - 34 * 0.8, mono(34, 800), ink);
  line(ctx, big, R, y, fit(ctx, big, mono(64, 800), R - L - 260, 0.6), ink, "right");
  y += 64 * 1.1;
  line(ctx, "vs typing the same words at 40 WPM", L, y, mono(22), greyInk);
  y += 22 * 1.3 + 28;
  const stampText = c.badge ? `MILESTONE: ${c.badge.name.toUpperCase()}` : "MY STATS SO FAR";
  const sf = fit(ctx, stampText, mono(26, 800, 2), R - L - 60, 0.6);
  const sw = width(ctx, stampText, sf) + 44;
  const sh = sf.size * 1.2 + 24;
  ctx.save();
  ctx.translate(mid, y + sh / 2);
  ctx.rotate((-3 * Math.PI) / 180);
  borderRR(ctx, -sw / 2, -sh / 2, sw, sh, 0, ink, 3);
  line(ctx, stampText, 0, -sh / 2 + 12, sf, ink, "center");
  ctx.restore();
  y += sh + 10;
  if (!sq) {
    y += 30;
    const widths = Array.from({ length: 48 }, (_, i) => [3, 6, 3, 9, 3, 6][i % 6]);
    const total = widths.reduce((a, b) => a + b, 0) + 5 * 47;
    let bx = mid - total / 2;
    widths.forEach((w, i) => {
      const h = 40 + 44 * Math.abs(Math.sin(i * 0.9));
      fillRR(ctx, bx, y + 84 - h, w, h, 2, ink);
      bx += w + 5;
    });
    y += 84;
  }
  y += 18;
  line(ctx, "BOTH HANDS BACK. THANK YOU FOR TALKING.", mid, y, mono(20, 400, 1.2), ink, "center");
  y += 20 * 1.2 + 6;
  line(ctx, "Processed on this computer. Nothing was sent.", mid, y, mono(18), greyInk, "center");
  ctx.restore();

  // Brand line at the bottom.
  const by = H - (sq ? 44 : 64) - 34;
  const nameF: Font = { size: 30, weight: 700 };
  const tagF: Font = { size: 24, weight: 500 };
  const tag = `· ${S.tagline}`;
  const total = 34 * BRAND_ASPECT + 14 + width(ctx, "Orttaai", nameF) + 14 + width(ctx, tag, tagF);
  let bx = W / 2 - total / 2;
  drawBrandMark(ctx, bx, by, 34, ink);
  bx += 34 * BRAND_ASPECT + 14;
  bx += line(ctx, "Orttaai", bx, by + 1, nameF, ink) + 14;
  ctx.save();
  ctx.globalAlpha = 0.75;
  line(ctx, tag, bx, by + 6, tagF, ink);
  ctx.restore();
}

function mountain({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = S.night;
  ctx.fillRect(0, 0, W, H);
  const padX = 84;
  const padY = 80;
  topRow(ctx, padX, W, padY, c.eyebrow, S.grey);
  const metres = heightMetres(c.words);
  const [value, unit] = metres >= 1000 ? [(metres / 1000).toFixed(1), "kilometres"] : [`${Math.round(metres)}`, "metres"];
  const next = nextLandmark(c.words);
  const closing = next
    ? next.name === "Kilimanjaro" || next.name === "Everest"
      ? `tall. ${next.name}, here I come.`
      : `tall. ${next.name} next.`
    : "tall. Higher than Everest.";
  let y = padY + 40 + (sq ? 36 : 64);
  line(ctx, "Stood on end, my words are", padX, y, { size: 52, weight: 700 }, S.soft);
  y += 52 * 1.2 + 8;
  const bigF = fit(ctx, `${value} ${unit}`, { size: sq ? 150 : 190, weight: 900, tracking: -6 }, W - padX * 2, 0.6);
  const vw = line(ctx, value, padX, y, bigF, S.amber);
  line(ctx, ` ${unit}`, padX + vw, y, bigF, S.cream);
  y += bigF.size * 1.05 + 8;
  line(ctx, closing, padX, y, { size: 48, weight: 700 }, S.cream);
  y += 48 * 1.2;

  const chartH = size === "portrait" ? 600 : sq ? 420 : 980;
  const bottomY = H - padY - FOOTER_H;
  const chartTop = Math.max(y + 20, y + (bottomY - 28 - y - chartH) / 2);
  drawMountainChart(ctx, c.words, chartTop, chartH);
  line(ctx, "One printed word is about 1.2 cm. Heights on a log scale.", padX, bottomY + 6, { size: 20 }, S.dim);
  line(ctx, S.tagline, W - padX, bottomY, { size: 24, weight: 600 }, S.soft, "right");
}

function drawMountainChart(ctx: CanvasRenderingContext2D, words: number, top0: number, chartH: number) {
  const g = chartH - 40;
  const top = 70;
  const yM = (m: number) => g - ((Math.log10(Math.min(Math.max(m, 10), 10_000)) - 1) / 3) * (g - top);
  const yN = (name: string) => yM(C.landmarks.find((l) => l.name === name)?.metres ?? 10);
  const me = yM(heightMetres(words));
  const everest = yN("Everest"), kili = yN("Kilimanjaro"), burj = yN("Burj Khalifa");
  const eiffel = yN("Eiffel Tower"), liberty = yN("Statue of Liberty");
  ctx.save();
  ctx.translate(0, top0);
  polygon(ctx, [[560, g], [700, everest + (g - everest) * 0.55], [750, everest + (g - everest) * 0.62], [860, everest], [930, everest + (g - everest) * 0.35], [1000, everest + (g - everest) * 0.24], [1080, everest + (g - everest) * 0.33], [1080, g]], "#202024");
  polygon(ctx, [[842, everest + 16], [860, everest], [880, everest + 50], [866, everest + 44], [856, everest + 60], [848, everest + 40]], "rgba(245,243,240,0.8)");
  polygon(ctx, [[500, g], [630, kili + (g - kili) * 0.17], [668, kili + 8], [738, kili], [782, kili + 18], [840, kili + (g - kili) * 0.3], [960, g]], "#2B2B30");
  polygon(ctx, [[668, kili + 8], [738, kili], [782, kili + 18], [766, kili + 46], [744, kili + 26], [722, kili + 50], [698, kili + 30], [660, kili + 46]], "rgba(245,243,240,0.92)");
  polygon(ctx, [[472, g], [478, burj + (g - burj) * 0.36], [482, burj], [486, burj + (g - burj) * 0.36], [492, g]], "#5A5A60");
  polygon(ctx, [[340, g], [352, eiffel + (g - eiffel) * 0.48], [358, eiffel + (g - eiffel) * 0.16], [360, eiffel], [362, eiffel + (g - eiffel) * 0.16], [368, eiffel + (g - eiffel) * 0.48], [380, g], [370, g], [360, g - 26], [350, g]], "#5A5A60");
  const pedestal = liberty + (g - liberty) * 0.5;
  polygon(ctx, [[214, g], [214, pedestal], [238, pedestal], [238, g]], "#5A5A60");
  polygon(ctx, [[220, pedestal], [222, liberty + (g - liberty) * 0.2], [226, liberty + (g - liberty) * 0.1], [228, liberty], [232, liberty + (g - liberty) * 0.1], [232, pedestal]], "#5A5A60");
  ctx.save();
  ctx.lineCap = "round";
  hline(ctx, 140, 1040, me, "rgba(212,149,42,0.55)", 3, [2, 12]);
  ctx.restore();
  const barTop = Math.min(me, g - 18);
  fillRR(ctx, 92, barTop, 44, Math.max(18, g - me), 3, S.amber);
  hline(ctx, 40, 1040, g, S.line, 3);
  const label = (title: string, detail: string, x: number, y: number, hi = false) => {
    const yy = Math.max(0, y);
    line(ctx, title, x, yy, { size: hi ? 26 : 20, weight: hi ? 800 : 700 }, hi ? S.amber : S.cream);
    line(ctx, detail, x, yy + (hi ? 26 : 20) * 1.2 + 2, { size: 18 }, S.grey);
  };
  const reach = (n: string) => fmt(wordsToReach(C.landmarks.find((l) => l.name === n)?.metres ?? 0));
  label("ME", heightText(words), 84, barTop - 64, true);
  label("Statue of Liberty", `93 m · ${reach("Statue of Liberty")}`, 160, liberty - 70);
  label("Eiffel Tower", `330 m · ${reach("Eiffel Tower")}`, 300, eiffel - 70);
  label("Burj Khalifa", `828 m · ${reach("Burj Khalifa")}`, 418, burj - 70);
  label("Kilimanjaro", `5,895 m · ${reach("Kilimanjaro")}`, 520, kili - 74);
  label("Everest", `${reach("Everest")} words`, 900, everest - 62);
  ctx.restore();
}

function books({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  const GATSBY = 47_000, NOVEL = 90_000, WAR = 587_000;
  ctx.fillStyle = S.paper;
  ctx.fillRect(0, 0, W, H);
  const pad = 84;
  const inner = W - pad * 2;
  const ink = S.charcoal;
  const accent = "#9A6315";
  topRow(ctx, pad, W, pad, c.eyebrow, "#6B5A43", 40, [ink, ink]);
  const words = c.words;
  const pg = pages(words);
  let head: Run[];
  let sub: string;
  if (words >= WAR) {
    head = [{ text: "I've out-written ", color: ink }, { text: "War and Peace.", color: accent, italic: true }];
    sub = `${fmt(words)} words, more than War and Peace's 587,000, and I didn't type a page of it.`;
  } else if (words >= NOVEL * 2) {
    const n = Math.floor(words / NOVEL);
    head = [{ text: "I've dictated ", color: ink }, { text: `${n} novels.`, color: accent, italic: true }];
    sub = `${fmt(words)} words: ${n} average-length novels, and I didn't type a page of them.`;
  } else if (words >= GATSBY) {
    head = [{ text: "I've dictated a ", color: ink }, { text: "whole novel.", color: accent, italic: true }];
    sub = `${fmt(words)} words. That's longer than The Great Gatsby, and I didn't type a page of it.`;
  } else {
    head = [{ text: `I've dictated ${fmt(pg)} `, color: ink }, { text: pg === 1 ? "page." : "pages.", color: accent, italic: true }];
    const pct = Math.max(1, Math.round((words / GATSBY) * 100));
    sub = `${fmt(words)} words, ${pct}% of the way to The Great Gatsby, and I didn't type a page of it.`;
  }
  let y = pad + 40 + (sq ? 36 : 52);
  y += rich(ctx, head, pad, y, { size: sq ? 96 : 116, family: "serif" }, inner, 1.05);
  y += 20;
  y += rich(ctx, [{ text: sub, color: "#48484A" }], pad, y, { size: 34 }, inner, 1.35);

  const yard =
    words >= NOVEL * 2 ? ["587,000", "words in War and Peace"] : words >= GATSBY ? ["90,000", "words in an average novel"] : ["47,000", "words in The Great Gatsby"];
  let by = H - pad - FOOTER_H;
  footer(ctx, pad, W - pad, by, "#5A5045", ink);
  by -= 40 + 146;
  ctx.fillStyle = ink;
  ctx.fillRect(pad, by, inner, 2);
  const stats: [string, string][] = [
    [fmt(pg), "pages, at 250 words a page"],
    [yard[0], yard[1]],
    [durationText(typingMinutes(words)), "to type it at 40 words a minute"],
  ];
  const colW = inner / 3;
  stats.forEach(([v, l], i) => {
    const x = pad + i * colW + (i > 0 ? 20 : 0);
    if (i > 0) {
      ctx.fillStyle = "#C9BFAE";
      ctx.fillRect(pad + i * colW, by + 26, 1, 120);
    }
    line(ctx, v, x, by + 26 + 24, fit(ctx, v, { size: 60, family: "serif" }, colW - 28, 0.6), ink);
    rich(ctx, [{ text: l, color: "#5A5045" }], x, by + 26 + 24 + 60 * 1.15 + 8, { size: 21 }, colW - 28, 1.2);
  });

  const artH = size === "story" ? 620 : sq ? 250 : 390;
  const artTop = y + Math.max(16, (by - y - artH) / 2);
  if (words < GATSBY) drawPagesBesideBook(ctx, W, artTop, artH, pg, words / GATSBY);
  else drawBookPile(ctx, W, artTop, artH, Math.min(8, Math.max(1, Math.ceil((words / NOVEL) * 1.5))));
}

function drawPagesBesideBook(ctx: CanvasRenderingContext2D, W: number, top: number, h: number, pg: number, fraction: number) {
  const bookH = h - 70;
  const sheets = Math.min(Math.max(pg, 1), 24);
  const stackH = sheets * 7 + (sheets - 1) * 3;
  const leftCx = W / 2 - 120 / 2 - 150;
  const rightCx = W / 2 + 120 / 2 + 75;
  const labelY = top + h - 30;
  const base = labelY - 18;
  const offsets = [0, 6, -4, 3, -7, 5, -2];
  for (let i = 0; i < sheets; i++) {
    const y = base - stackH + i * 10;
    const x = leftCx - 150 + offsets[i % 7];
    fillRR(ctx, x, y, 300, 7, 2, "#FBF8F2");
    borderRR(ctx, x, y, 300, 7, 2, "rgba(28,28,30,0.55)", 1.5);
  }
  line(ctx, `my ${fmt(pg)} page${pg === 1 ? "" : "s"}`, leftCx, labelY, { size: 24, weight: 600 }, S.charcoal, "center");
  const bx = rightCx - 75;
  const by = base - bookH;
  ctx.save();
  rr(ctx, bx, by, 150, bookH, 12);
  ctx.clip();
  const fillH = Math.max(10, bookH * Math.min(1, fraction));
  fillRR(ctx, bx, by + bookH - fillH, 150, fillH, 12, S.amber);
  ctx.restore();
  ctx.save();
  ctx.setLineDash([10, 8]);
  borderRR(ctx, bx, by, 150, bookH, 12, S.charcoal, 3);
  ctx.restore();
  line(ctx, `The Great Gatsby · ${Math.max(1, Math.round(fraction * 100))}%`, rightCx, labelY, { size: 24, weight: 600 }, S.charcoal, "center");
}

function drawBookPile(ctx: CanvasRenderingContext2D, W: number, top: number, h: number, count: number) {
  const specs: [number, number, string, string, number][] = [
    [640, 70, "#1C1C1E", "#F5F3F0", -20], [600, 58, "#D4952A", "#1C1C1E", 10], [660, 64, "#3A3A3C", "#F5F3F0", -40],
    [580, 52, "#FBF8F2", "#1C1C1E", 30], [620, 66, "#C9895A", "#1C1C1E", -10], [560, 54, "#2C2C2E", "#F5F3F0", 40],
    [600, 60, "#E2B04A", "#1C1C1E", 0], [620, 56, "#8E8E93", "#1C1C1E", -30],
  ];
  const booksArr = specs.slice(0, count);
  const total = booksArr.reduce((a, s) => a + s[1] + 2, 0);
  const scale = Math.min(1, h / Math.max(total + 20, 1));
  // Shadow under the pile, then books bottom-up (the first spec at the bottom).
  const shadowY = top + h - 14;
  ctx.save();
  ctx.fillStyle = "rgba(0,0,0,0.12)";
  ctx.beginPath();
  ctx.ellipse(W / 2, shadowY + 7, (720 * scale) / 2, 7, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
  let y = shadowY - 2 * scale;
  booksArr.forEach(([w0, h0, fill, ink, inset], index) => {
    const w = w0 * scale, bh = h0 * scale;
    y -= bh;
    const x = W / 2 - w / 2 + inset * scale;
    fillRR(ctx, x, y, w, bh, 9, fill);
    if (fill === "#FBF8F2") borderRR(ctx, x, y, w, bh, 9, S.charcoal, 2);
    ctx.save();
    ctx.globalAlpha = 0.28;
    ctx.fillStyle = ink;
    ctx.fillRect(x + 36 * scale, y, 6, bh);
    ctx.fillRect(x + w - 6 - 12 - 6 - 6, y, 6, bh);
    ctx.restore();
    ctx.save();
    ctx.globalAlpha = 0.4;
    fillRR(ctx, x + w / 2 - (180 * scale) / 2, y + bh / 2 - 4.5, 180 * scale, 9, 4.5, ink);
    ctx.restore();
    fillRR(ctx, x + w - 6 - 12, y + 6, 12, bh - 12, 3, "#FBF8F2");
    if (index === booksArr.length - 1) {
      // A ribbon bookmark hanging from the top book.
      const rx = x + w * 0.68;
      polygon(ctx, [[rx, y + 6], [rx, y + 56], [rx + 14, y + 44], [rx + 28, y + 56], [rx + 28, y + 6]], S.amber);
    }
    y -= 2 * scale;
  });
}

function speed({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = S.charcoal;
  ctx.fillRect(0, 0, W, H);
  const padX = 84, padY = 80;
  const pace = c.pace;
  topRow(ctx, padX, W, padY, c.badge?.name.toUpperCase() ?? "MY PACE", S.amber);
  const typing = multiple(pace, C.typingWPM);
  const head: Run[] =
    pace > C.typingWPM
      ? [{ text: "My voice is ", color: S.cream }, { text: `${typing}×`, color: S.amber }, { text: " faster than my keyboard.", color: S.cream }]
      : [{ text: "I dictate at ", color: S.cream }, { text: `${pace}`, color: S.amber }, { text: " words a minute.", color: S.cream }];
  let y = padY + 40 + (sq ? 40 : 68);
  y += rich(ctx, head, padX, y, { size: sq ? 52 : 58, weight: 800 }, W - padX * 2, 1.2);

  let by = H - padY - FOOTER_H;
  footer(ctx, padX, W - padX, by);
  const tileH = 24 * 2 + 56 * 1.2 + 6 + 24 * 1.25;
  by -= 40 + tileH;
  const tw = (W - padX * 2 - 24) / 2;
  const tiles: ShareStat[] = [
    { value: `${multiple(pace, C.handwritingWPM)}×`, label: "faster than writing by hand" },
    { value: keyPressesText(c.words), label: "key presses I didn't make" },
  ];
  tiles.forEach((t, i) => {
    const x = padX + i * (tw + 24);
    fillRR(ctx, x, by, tw, tileH, 22, S.tile);
    borderRR(ctx, x, by, tw, tileH, 22, S.line, 1);
    line(ctx, t.value, x + 28, by + 24, fit(ctx, t.value, { size: 56, weight: 800 }, tw - 56, 0.5), S.amber);
    line(ctx, t.label, x + 28, by + 24 + 56 * 1.2 + 6, { size: 24 }, S.soft);
  });
  const dialH = sq ? 500 : 640;
  const dialTop = y + Math.max(10, (by - y - dialH) / 2);
  ctx.save();
  ctx.translate(0, dialTop);
  if (sq) {
    ctx.translate(W / 2, 0);
    ctx.scale(0.78, 0.78);
    ctx.translate(-W / 2, 0);
  }
  drawSpeedDial(ctx, pace);
  ctx.restore();
}

function drawSpeedDial(ctx: CanvasRenderingContext2D, pace: number) {
  const cx = 540, cy = 560, R = 400;
  const max = Math.max(250, pace + 20);
  const pt = (wpm: number, r: number): [number, number] => {
    const a = Math.PI * (1 - Math.min(wpm, max) / max);
    return [cx + r * Math.cos(a), cy - r * Math.sin(a)];
  };
  ctx.save();
  ctx.lineCap = "round";
  ctx.lineWidth = 44;
  ctx.strokeStyle = "#2C2C30";
  ctx.beginPath();
  ctx.arc(cx, cy, R, Math.PI, Math.PI * 2);
  ctx.stroke();
  const frac = Math.min(1, pace / max);
  if (frac > 0) {
    ctx.strokeStyle = S.amber;
    ctx.beginPath();
    ctx.arc(cx, cy, R, Math.PI, Math.PI + Math.PI * frac);
    ctx.stroke();
  }
  ctx.lineWidth = 3;
  ctx.strokeStyle = "#55555A";
  for (let w = 0; w <= 250; w += 25) {
    const [x1, y1] = pt(w, w % 50 === 0 ? 352 : 360);
    const [x2, y2] = pt(w, 372);
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.stroke();
  }
  ctx.restore();
  for (const w of [0, 50, 100, 150, 200, 250]) {
    const [x, y] = pt(w, 324);
    line(ctx, `${w}`, x, y - 12, { size: 20 }, S.dim, "center");
  }
  const marker = (name: string, wpm: number, lx: number, ly: number) => {
    const [x, y] = pt(wpm, R);
    ctx.beginPath();
    ctx.arc(x, y, 11, 0, Math.PI * 2);
    ctx.fillStyle = S.cream;
    ctx.fill();
    ctx.beginPath();
    ctx.arc(x, y, 9, 0, Math.PI * 2);
    ctx.strokeStyle = S.charcoal;
    ctx.lineWidth = 4;
    ctx.stroke();
    line(ctx, name, lx, ly - 27, { size: 24, weight: 700 }, S.cream, "center");
    line(ctx, `${wpm} WPM`, lx, ly + 2, { size: 20 }, S.grey, "center");
  };
  marker("Handwriting", C.handwritingWPM, 150, 618);
  marker("Typing", C.typingWPM, 112, 316);
  marker("Conversation", C.conversationWPM, 760, 128);
  marker("Auctioneer", C.auctioneerWPM, 940, 618);
  const [a1, a2] = [pt(pace, 450), pt(pace - 3, 418)];
  const a3 = pt(pace + 3, 418);
  polygon(ctx, [a1, a2, a3], S.amber);
  line(ctx, `${pace}`, 540, 430 - 145, { size: 210, weight: 900, tracking: -8 }, S.cream, "center");
  line(ctx, "words a minute, out loud", 540, 430 + 30, { size: 34, weight: 600 }, S.soft, "center");
}

function hands({ ctx, c, W, H, size }: Ctx) {
  const sq = size === "square";
  ctx.fillStyle = "#18181A";
  ctx.fillRect(0, 0, W, H);
  const pad = 84;
  const saved = c.stats.minutes_saved;
  topRow(ctx, pad, W, pad, "HANDS FREE", S.amber);
  const head: Run[] =
    saved > 0
      ? [{ text: durationText(saved), color: S.amber }, { text: "\nwith both hands free.", color: S.cream }]
      : [{ text: "Both hands free\n", color: S.cream }, { text: "while I write.", color: S.amber }];
  let y = pad + 40 + (sq ? 36 : 64);
  y += rich(ctx, head, pad, y, { size: sq ? 80 : 92, weight: 900, tracking: -2 }, W - pad * 2, 1.1);

  let by = H - pad - 44 * 1.15 * 2;
  rich(ctx, [{ text: "Giving you back\nboth hands.", color: S.cream }], pad, by, { size: 44, weight: 800 }, 600, 1.15);
  line(ctx, S.product, W - pad, by + 44 * 2.3 - 22 * 2.5, { size: 22 }, S.dim, "right");
  line(ctx, "Typing time at 40 words a minute", W - pad, by + 44 * 2.3 - 22 * 1.25, { size: 22 }, S.dim, "right");
  by -= 40 + 150;
  const tw = (W - pad * 2 - 36) / 3;
  const tiles: [ShareStat, boolean][] = [
    [{ value: durationText(typingMinutes(c.words)), label: `to type ${fmt(c.words)} words` }, false],
    [{ value: durationText(c.stats.speaking_minutes), label: "to say them instead" }, false],
    [{ value: timeBackEquivalent(saved).replace(/^a /, "1 "), label: "given back to me" }, true],
  ];
  tiles.forEach(([st, accent], i) => tile(ctx, pad + i * (tw + 18), by, tw, 150, st, { accent, valueSize: 44, fill: S.tile }));

  const artH = sq ? 270 : size === "story" ? 560 : 430;
  const scale = sq ? 0.62 : size === "story" ? 1.2 : 1;
  const top = y + Math.max(10, (by - y - artH) / 2);
  ctx.save();
  ctx.translate(W / 2, top + artH / 2);
  ctx.scale(scale, scale);
  drawRaisedHands(ctx);
  ctx.restore();
}

/** Two raised hands, the brand bars between them, waves on the sides. Drawn around (0, 0). */
function drawRaisedHands(ctx: CanvasRenderingContext2D) {
  const parts: [number, number, number, number, number, number][] = [
    [34, 120, 70, -10, 17, 38],
    [32, 150, -60, -110, 16, 0],
    [34, 190, -23, -130, 17, 0],
    [34, 196, 15, -130, 17, 0],
    [32, 168, 52, -112, 16, 0],
    [148, 150, -4, -20, 56, 0],
    [108, 40, -4, 80, 12, 0],
  ];
  const hand = (side: number) => {
    ctx.save();
    ctx.translate(side * 150, 0);
    ctx.rotate((side * 10 * Math.PI) / 180);
    ctx.scale(side, 1);
    ctx.translate(0, 30);
    for (const [w, h, x, y, r, a] of parts) {
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate((a * Math.PI) / 180);
      fillRR(ctx, -w / 2, -h / 2, w, h, r, "#2C2C30");
      borderRR(ctx, -w / 2, -h / 2, w, h, r, S.amber, 5);
      ctx.restore();
    }
    ctx.restore();
  };
  hand(-1);
  hand(1);
  const bars: [number, number][] = [[12, 26], [12, 38], [12, 52], [5, 60]];
  const total = bars.reduce((a, b) => a + b[0], 0) + 18;
  let bx = -total / 2;
  for (const [w, h] of bars) {
    fillRR(ctx, bx, -10 + 30 - h, w, h, w / 2, S.amber);
    bx += w + 6;
  }
  ctx.save();
  ctx.lineCap = "round";
  ctx.lineWidth = 5;
  ctx.strokeStyle = S.line;
  for (const side of [-1, 1]) {
    for (const radius of [60, 100]) {
      ctx.beginPath();
      const cx = side * 330 + side * radius;
      const cy = -80;
      if (side > 0) ctx.arc(cx, cy, radius, (160 * Math.PI) / 180, (210 * Math.PI) / 180);
      else ctx.arc(cx, cy, radius, (-30 * Math.PI) / 180, (20 * Math.PI) / 180);
      ctx.stroke();
    }
  }
  ctx.restore();
}

/* ------------------------------------------------- milestone picture cards -- */

function milestoneLayout(
  { ctx, c, W, H, size }: Ctx,
  headline: Run[],
  picture: (top: number, height: number) => void,
  background = S.charcoal,
) {
  const sq = size === "square";
  ctx.fillStyle = background;
  ctx.fillRect(0, 0, W, H);
  const pad = 84;
  topRow(ctx, pad, W, pad, c.badge?.name.toUpperCase() ?? "MILESTONE", S.amber);
  let y = pad + 40 + (sq ? 32 : 56);
  y += rich(ctx, headline, pad, y, { size: sq ? 72 : 84, weight: 900, tracking: -2 }, W - pad * 2, 1.12);
  let by = H - pad - FOOTER_H;
  footer(ctx, pad, W - pad, by);
  by -= (sq ? 28 : 40) + 140;
  const tw = (W - pad * 2 - 36) / 3;
  c.supportingStats.forEach((st, i) =>
    tile(ctx, pad + i * (tw + 18), by, tw, 140, st, { accent: i === 2, valueSize: 46, fill: "rgba(255,255,255,0.05)" }),
  );
  const ph = size === "portrait" ? 560 : sq ? 360 : 980;
  const avail = by - y;
  const h = Math.min(ph, avail - 32);
  picture(y + (avail - h) / 2, h);
}

const amber = (text: string): Run => ({ text, color: S.amber });
const cream = (text: string): Run => ({ text, color: S.cream });

function streak(x: Ctx) {
  const days = Math.max(1, x.c.milestoneValue);
  milestoneLayout(x, [amber(`${days} days`), cream("\nin a row.")], (top, height) => {
    const { ctx, W } = x;
    const cols = days <= 35 ? 7 : 10;
    const rows = Math.ceil(days / cols);
    const gap = days <= 35 ? 16 : 10;
    const cell = Math.min((912 - gap * (cols - 1)) / cols, (height - gap * (rows - 1)) / rows, 116);
    const gw = cols * cell + (cols - 1) * gap;
    const gh = rows * cell + (rows - 1) * gap;
    const left = W / 2 - gw / 2;
    const t = top + (height - gh) / 2;
    for (let r = 0; r < rows; r++)
      for (let col = 0; col < cols; col++) {
        const i = r * cols + col;
        const cx = left + col * (cell + gap);
        const cy = t + r * (cell + gap);
        const filled = i < days;
        const last = i === days - 1;
        if (filled) {
          ctx.save();
          ctx.globalAlpha = last ? 1 : 0.85;
          fillRR(ctx, cx, cy, cell, cell, cell * 0.24, S.amber);
          ctx.restore();
          drawGlyph(ctx, last ? "flame.fill" : "flame", cx + cell / 2, cy + cell / 2, cell * 0.5, S.charcoal, { weight: 2.2 });
        } else {
          fillRR(ctx, cx, cy, cell, cell, cell * 0.24, "rgba(255,255,255,0.04)");
          borderRR(ctx, cx, cy, cell, cell, cell * 0.24, S.line, 2);
        }
      }
  });
}

function race(x: Ctx) {
  const pace = x.c.stats.best_wpm;
  milestoneLayout(x, [cream("I out-talk my keyboard "), amber(`${multiple(pace, C.typingWPM)}×`), cream(".")], (top, height) => {
    const { ctx } = x;
    const entries = [
      { name: "Handwriting", wpm: C.handwritingWPM, me: false },
      { name: "Typing", wpm: C.typingWPM, me: false },
      { name: "Conversation", wpm: C.conversationWPM, me: false },
      { name: "Auctioneer", wpm: C.auctioneerWPM, me: false },
      { name: "Me, talking", wpm: pace, me: true },
    ].sort((a, b) => a.wpm - b.wpm);
    const topWpm = Math.max(250, pace);
    const rowH = Math.min(96, height / 5 - 14);
    const blockH = 5 * rowH + 4 * 14 + 14 + 24;
    let y = top + (height - blockH) / 2;
    const barX = 84 + 252;
    const barMax = 1080 - 84 - barX;
    for (const e of entries) {
      line(ctx, e.name, 84, y + rowH / 2 - 17, { size: 28, weight: e.me ? 800 : 600 }, e.me ? S.amber : S.soft);
      const bw = Math.max(rowH, (barMax * e.wpm) / topWpm);
      fillRR(ctx, barX, y, bw, rowH, rowH * 0.28, e.me ? S.amber : "#3A3A3E");
      line(ctx, `${e.wpm}`, barX + bw - 18, y + rowH / 2 - 18, { size: 30, weight: 800 }, e.me ? S.charcoal : S.cream, "right");
      y += rowH + 14;
    }
    line(ctx, "Words a minute", barX, y, { size: 20 }, S.dim);
  });
}

function stack(x: Ctx) {
  const count = x.c.milestoneValue;
  milestoneLayout(x, [amber(`${fmt(count)} dictations`), cream(",\nnot one typed.")], (top, height) => {
    const { ctx, W } = x;
    const layers = Math.min(9, Math.max(3, Math.floor(Math.log10(Math.max(count, 1)) * 3) + 2));
    const cardH = Math.min(220, height * 0.42);
    const step = (height - cardH) / layers;
    for (let i = 0; i < layers; i++) {
      const depth = layers - 1 - i;
      const w = 720 - depth * 26;
      const isTop = i === layers - 1;
      ctx.save();
      ctx.translate(W / 2, top + i * step + cardH / 2);
      ctx.rotate(((isTop ? 0 : i % 2 === 0 ? -1.5 : 1.5) * Math.PI) / 180);
      fillRR(ctx, -w / 2, -cardH / 2, w, cardH, 30, isTop ? S.tile : "#202022");
      borderRR(ctx, -w / 2, -cardH / 2, w, cardH, 30, isTop ? S.amber : S.line, 2);
      ctx.restore();
    }
    const cy = top + (layers - 1) * step + cardH / 2;
    const levels = [0.35, 0.7, 1.0, 0.55, 0.85, 0.4, 0.65];
    const glyphH = cardH * 0.45;
    const label = `No. ${fmt(count)}`;
    const labelW = Math.max(width(ctx, label, { size: 54, weight: 900 }), width(ctx, "spoken, at the cursor", { size: 24 }));
    const groupW = 150 + 28 + labelW;
    let gx = W / 2 - groupW / 2;
    const bw = 12;
    const spacing = (150 - bw * 7) / 6;
    levels.forEach((lv) => {
      const h = glyphH * lv;
      fillRR(ctx, gx, cy - h / 2, bw, h, bw / 2, S.amber);
      gx += bw + spacing;
    });
    const tx = W / 2 - groupW / 2 + 150 + 28;
    line(ctx, label, tx, cy - 54 * 0.9, { size: 54, weight: 900 }, S.cream);
    line(ctx, "spoken, at the cursor", tx, cy + 54 * 0.35, { size: 24 }, S.grey);
  });
}

function apps(x: Ctx) {
  const n = Math.max(x.c.milestoneValue, x.c.stats.distinct_apps);
  milestoneLayout(x, [cream("One voice.\n"), amber(`${n} apps.`)], (top, height) => {
    const { ctx } = x;
    const shown = Math.min(n, 10);
    const radius = Math.min(390, height / 2 - 70);
    const tileS = Math.min(170, radius * 0.55);
    const cx = 84 + 456;
    const cy = top + height / 2;
    const pos = (i: number): [number, number] => {
      const a = -Math.PI / 2 + (i / shown) * 2 * Math.PI;
      return [cx + radius * 1.1 * Math.cos(a), cy + radius * Math.sin(a)];
    };
    for (let i = 0; i < shown; i++) {
      const [px, py] = pos(i);
      ctx.save();
      ctx.lineCap = "round";
      ctx.setLineDash([2, 12]);
      ctx.strokeStyle = "rgba(212,149,42,0.35)";
      ctx.lineWidth = 3;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(px, py);
      ctx.stroke();
      ctx.restore();
    }
    for (let i = 0; i < shown; i++) {
      const [px, py] = pos(i);
      fillRR(ctx, px - tileS / 2, py - tileS / 2, tileS, tileS, tileS * 0.26, ["#2C2C30", "#34312C", "#2A2D33"][i % 3]);
      borderRR(ctx, px - tileS / 2, py - tileS / 2, tileS, tileS, tileS * 0.26, S.line, 2);
      // A text field with the cursor in it: no app names.
      const fw = tileS * 0.34, fh = tileS * 0.07, cw = tileS * 0.06, ch = tileS * 0.34, gap = tileS * 0.05;
      const gx = px - (fw + gap + cw) / 2;
      fillRR(ctx, gx, py - fh / 2, fw, fh, fh / 2, "#6E6E73");
      fillRR(ctx, gx + fw + gap, py - ch / 2, cw, ch, cw / 2, S.amber);
    }
    if (n > shown) line(ctx, `+${n - shown}`, cx, cy + radius + tileS / 2 + 14, { size: 34, weight: 800 }, S.amber, "center");
    ctx.save();
    ctx.shadowColor = "rgba(212,149,42,0.4)";
    ctx.shadowBlur = 30;
    ctx.beginPath();
    ctx.arc(cx, cy, (tileS * 1.3) / 2, 0, Math.PI * 2);
    ctx.fillStyle = S.amber;
    ctx.fill();
    ctx.restore();
    drawGlyph(ctx, "mic", cx, cy, tileS * 0.62, S.charcoal, { weight: 2.4 });
  });
}

function stopwatch(x: Ctx) {
  const seconds = Math.floor(x.c.stats.longest_dictation_ms / 1000);
  milestoneLayout(x, [cream("I talked for "), amber(secondsText(seconds)), cream(" in one go.")], (top, height) => {
    const { ctx, W } = x;
    const d = Math.min(height - 40, 620);
    const cx = W / 2;
    const cy = top + height / 2 + d * 0.04;
    const laps = Math.floor(seconds / 60);
    const rest = (seconds % 60) / 60;
    fillRR(ctx, cx - d * 0.08, cy - d * 0.56 - d * 0.05, d * 0.16, d * 0.1, d * 0.05, "#3A3A3E");
    ctx.beginPath();
    ctx.arc(cx, cy, d / 2, 0, Math.PI * 2);
    ctx.fillStyle = S.tile;
    ctx.fill();
    ctx.beginPath();
    ctx.arc(cx, cy, d / 2 - d * 0.0175, 0, Math.PI * 2);
    ctx.strokeStyle = "#3A3A3E";
    ctx.lineWidth = d * 0.035;
    ctx.stroke();
    const trim = laps > 0 && rest === 0 ? 1 : rest;
    if (trim > 0) {
      ctx.save();
      ctx.lineCap = "round";
      ctx.beginPath();
      ctx.arc(cx, cy, d / 2 - d * 0.035, -Math.PI / 2, -Math.PI / 2 + trim * Math.PI * 2);
      ctx.strokeStyle = S.amber;
      ctx.lineWidth = d * 0.035;
      ctx.stroke();
      ctx.restore();
    }
    for (let t = 0; t < 60; t++) {
      const major = t % 5 === 0;
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate((t * 6 * Math.PI) / 180);
      const w = major ? 5 : 3;
      const h = major ? d * 0.05 : d * 0.03;
      fillRR(ctx, -w / 2, -d * 0.4 - h / 2, w, h, w / 2, major ? S.cream : "#55555A");
      ctx.restore();
    }
    const time = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
    line(ctx, time, cx, cy - d * 0.17, { size: d * 0.24, weight: 900 }, S.cream, "center");
    line(ctx, laps > 0 ? `${laps} full lap${laps === 1 ? "" : "s"} of the dial` : "seconds", cx, cy + d * 0.1, { size: d * 0.045 }, S.grey, "center");
  });
}

function dictionary(x: Ctx) {
  const count = Math.max(x.c.milestoneValue, x.c.stats.dictionary_words);
  milestoneLayout(x, [cream("My computer spells "), amber(`${x.c.milestoneValue} words`), cream(" my way.")], (top, height) => {
    const { ctx, W } = x;
    const pw = 820;
    const left = W / 2 - pw / 2 - 20;
    ctx.save();
    ctx.translate(left + pw / 2, top + height / 2);
    ctx.rotate((-1.2 * Math.PI) / 180);
    ctx.translate(-pw / 2, -height / 2);
    ctx.save();
    ctx.shadowColor = "rgba(0,0,0,0.4)";
    ctx.shadowBlur = 24;
    ctx.shadowOffsetY = 18;
    fillRR(ctx, 0, 0, pw, height, 10, S.cream);
    ctx.restore();
    // Thumb index down the right edge.
    const letters = ["A", "F", "K", "P", "U", "Z"];
    const tabTop = height / 2 - (letters.length * 52 + 5 * 6) / 2;
    letters.forEach((l, i) => {
      const ty = tabTop + i * 58;
      fillRR(ctx, pw - 20 + 10, ty, 40, 52, 6, S.amber);
      line(ctx, l, pw + 10, ty + 16, { size: 20, weight: 700, family: "serif" }, S.charcoal, "center");
    });
    const px = 56;
    let y = 44;
    line(ctx, "My dictionary", px, y, { size: 40, weight: 600, family: "serif" }, S.charcoal);
    line(ctx, `${count} entries`, pw - px, y + 14, { size: 22, family: "serif", italic: true }, "#6E6E73", "right");
    y += 48 + 18;
    ctx.fillStyle = S.charcoal;
    ctx.fillRect(px, y, pw - px * 2, 2);
    y += 2 + 18;
    const entries = Math.max(0, Math.min(count, Math.floor((height - 150) / 52)));
    for (let i = 0; i < entries; i++) {
      const a = [150, 210, 120, 180, 240, 140][i % 6];
      const b = [300, 220, 360, 260, 200, 320][i % 6];
      fillRR(ctx, px, y + 7, a, 16, 8, S.charcoal);
      drawGlyph(ctx, "check", px + a + 14 + 10, y + 15, 22, "#9A6315", { weight: 3 });
      fillRR(ctx, px + a + 14 + 20 + 14, y + 9, b, 12, 6, "#C7C3BC");
      y += 30 + 22;
    }
    line(ctx, "Words kept private. Only the count is shared.", px, height - 44 - 24, { size: 20, family: "serif", italic: true }, "#6E6E73");
    ctx.restore();
  });
}

function night(x: Ctx) {
  milestoneLayout(
    x,
    [cream("After midnight,\nmy keyboard got "), amber("the night off.")],
    (top, height) => {
      const { ctx } = x;
      const ox = 84;
      const stars: [number, number, number][] = [
        [60, 0.12, 4], [180, 0.3, 3], [300, 0.08, 5], [420, 0.22, 3], [760, 0.1, 4],
        [860, 0.32, 3], [620, 0.05, 3], [90, 0.5, 3], [880, 0.6, 4], [520, 0.4, 2],
      ];
      ctx.fillStyle = "rgba(245,243,240,0.8)";
      for (const [sx, sy, r] of stars) {
        ctx.beginPath();
        ctx.arc(ox + sx, top + sy * height, r, 0, Math.PI * 2);
        ctx.fill();
      }
      const moon = Math.min(230, height * 0.4);
      const clock = Math.min(300, height * 0.4);
      const kb = Math.min(190, height * 0.34);
      // Crescent: a gold disc with an offset disc cut out of it.
      const off = document.createElement("canvas");
      off.width = off.height = Math.ceil(moon);
      const o = off.getContext("2d");
      if (o) {
        o.fillStyle = S.gold;
        o.beginPath();
        o.arc(moon / 2, moon / 2, moon / 2, 0, Math.PI * 2);
        o.fill();
        o.globalCompositeOperation = "destination-out";
        o.beginPath();
        o.arc(moon / 2 + moon * 0.3, moon / 2 - moon * 0.22, (moon * 0.87) / 2, 0, Math.PI * 2);
        o.fill();
        ctx.save();
        ctx.shadowColor = "rgba(226,176,74,0.45)";
        ctx.shadowBlur = 50;
        ctx.drawImage(off, ox + 680 - moon / 2, top + 10);
        ctx.restore();
      }
      // Clock at 1:12.
      const ccx = ox + 260, ccy = top + clock / 2 + 10;
      ctx.beginPath();
      ctx.arc(ccx, ccy, clock / 2, 0, Math.PI * 2);
      ctx.fillStyle = "#1B2030";
      ctx.fill();
      ctx.beginPath();
      ctx.arc(ccx, ccy, clock / 2 - clock * 0.015, 0, Math.PI * 2);
      ctx.strokeStyle = "#3A4058";
      ctx.lineWidth = clock * 0.03;
      ctx.stroke();
      for (let h = 0; h < 12; h++) {
        ctx.save();
        ctx.translate(ccx, ccy);
        ctx.rotate((h * 30 * Math.PI) / 180);
        fillRR(ctx, -2, -clock * 0.38 - clock * 0.035, 4, clock * 0.07, 2, "#8088A0");
        ctx.restore();
      }
      const handDraw = (len: number, w: number, deg: number) => {
        ctx.save();
        ctx.translate(ccx, ccy);
        ctx.rotate((deg * Math.PI) / 180);
        fillRR(ctx, -w / 2, -clock * len, w, clock * len, w / 2, S.amber);
        ctx.restore();
      };
      handDraw(0.24, 10, 37);
      handDraw(0.34, 6, 72);
      ctx.beginPath();
      ctx.arc(ccx, ccy, clock * 0.035, 0, Math.PI * 2);
      ctx.fillStyle = S.amber;
      ctx.fill();
      // A keyboard, asleep.
      const kw = 760;
      const kx = ox + 456 - kw / 2;
      const ky = top + height - kb;
      fillRR(ctx, kx, ky, kw, kb, 26, "#161A26");
      borderRR(ctx, kx, ky, kw, kb, 26, "#2A3044", 2);
      const key = (kw - 60 - 11 * 10) / 12;
      const keyH = Math.min(key * 0.8, (kb - 80) / 3);
      const rowsH = keyH * 3 + 20;
      for (let r = 0; r < 3; r++)
        for (let k = 0; k < 12; k++)
          fillRR(ctx, kx + 30 + k * (key + 10), ky + (kb - rowsH) / 2 + r * (keyH + 10), key, keyH, 8, "#1E2232");
      ctx.save();
      ctx.globalAlpha = 0.8;
      line(ctx, "z z z", kx + kw - 20, ky - 56, { size: 40, weight: 800 }, S.gold, "right");
      ctx.restore();
    },
    "#10131C",
  );
}

const DRAW: Record<CardTemplate, (x: Ctx) => void> = {
  signal, editorial, medal, receipt, mountain, books, speed, hands,
  streak, race, stack, apps, stopwatch, dictionary, night,
};

let fontsReady: Promise<unknown> | null = null;
/** Wait for the card fonts (Inter, Source Serif, JetBrains Mono) once. */
export function loadCardFonts(): Promise<unknown> {
  if (!fontsReady) {
    const specs = [
      `400 40px ${SANS}`, `600 40px ${SANS}`, `700 40px ${SANS}`, `800 40px ${SANS}`, `900 40px ${SANS}`,
      `400 40px ${SERIF}`, `italic 400 40px ${SERIF}`, `600 40px ${SERIF}`,
      `400 40px ${MONO}`, `700 40px ${MONO}`, `800 40px ${MONO}`,
    ];
    fontsReady = Promise.all(specs.map((s) => document.fonts.load(s).catch(() => null)));
  }
  return fontsReady;
}

/** Draw `template` onto `canvas` at `scale` × its export size (resizing it). */
export function renderCard(
  canvas: HTMLCanvasElement,
  template: CardTemplate,
  content: CardContent,
  size: CardSize,
  scale = 1,
) {
  const dims = CARD_SIZES.find((s) => s.value === size) ?? CARD_SIZES[0];
  canvas.width = Math.round(dims.w * scale);
  canvas.height = Math.round(dims.h * scale);
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, dims.w, dims.h);
  DRAW[template]({ ctx, c: content, W: dims.w, H: dims.h, size });
}

/** The card as a PNG data URL, at its exact export size. */
export async function cardPng(template: CardTemplate, content: CardContent, size: CardSize): Promise<string> {
  await loadCardFonts();
  const canvas = document.createElement("canvas");
  renderCard(canvas, template, content, size);
  return canvas.toDataURL("image/png");
}

/** The card as a PNG blob (for the clipboard). */
export async function cardBlob(template: CardTemplate, content: CardContent, size: CardSize): Promise<Blob> {
  await loadCardFonts();
  const canvas = document.createElement("canvas");
  renderCard(canvas, template, content, size);
  return new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("Couldn't draw the card."))), "image/png"),
  );
}

/** A live preview, drawn at screen resolution and `displayWidth` wide. */
export function ShareCardCanvas({
  template,
  content,
  size,
  displayWidth,
  className = "",
}: {
  template: CardTemplate;
  content: CardContent;
  size: CardSize;
  displayWidth: number;
  className?: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const dims = CARD_SIZES.find((s) => s.value === size) ?? CARD_SIZES[0];
  const scale = Math.min(1, (displayWidth * (window.devicePixelRatio || 1)) / dims.w);
  useEffect(() => {
    let live = true;
    loadCardFonts().then(() => {
      if (live && ref.current) renderCard(ref.current, template, content, size, scale);
    });
    return () => {
      live = false;
    };
  }, [template, content, size, scale]);
  return (
    <canvas
      ref={ref}
      className={`share-canvas ${className}`}
      style={{ width: displayWidth, height: (displayWidth * dims.h) / dims.w }}
      aria-hidden="true"
    />
  );
}

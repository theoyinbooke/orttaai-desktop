// The animated scene at the top of each Setup step (Mac SetupArtwork): your
// voice turning into text, a shield for permissions, a chip fed by a small
// network for the engine, and a key press for Ready. One canvas at 30 fps; holds
// still under Reduce Motion.
import { useEffect, useRef } from "react";

type Palette = { surface: string; bg: string; tile: string; text: string; accent: string; ok: string };
type Ctx = CanvasRenderingContext2D;

const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

function alpha(hex: string, a: number) {
  const h = hex.trim().replace("#", "");
  const n = parseInt(h.length === 3 ? h.split("").map((c) => c + c).join("") : h, 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${Math.max(0, Math.min(1, a))})`;
}

function readPalette(): Palette {
  const css = getComputedStyle(document.documentElement);
  const v = (k: string) => css.getPropertyValue(k).trim();
  return { surface: v("--surface"), bg: v("--bg"), tile: v("--surface-3"), text: v("--text"), accent: v("--accent"), ok: v("--ok") };
}

function rr(c: Ctx, x: number, y: number, w: number, h: number, r: number) {
  c.beginPath();
  c.roundRect(x, y, w, h, r);
}

function glow(c: Ctx, x: number, y: number, size: number, color: string, opacity: number) {
  const g = c.createRadialGradient(x, y, 0, x, y, size / 2);
  g.addColorStop(0, alpha(color, opacity));
  g.addColorStop(1, alpha(color, 0));
  c.fillStyle = g;
  c.fillRect(x - size / 2, y - size / 2, size, size);
}

/** Draws a 24-unit icon path at (cx, cy). */
function icon(c: Ctx, d: string, cx: number, cy: number, size: number, color: string, width = 2) {
  c.save();
  c.translate(cx - size / 2, cy - size / 2);
  c.scale(size / 24, size / 24);
  c.strokeStyle = color;
  c.lineWidth = width;
  c.lineCap = "round";
  c.lineJoin = "round";
  c.stroke(new Path2D(d));
  c.restore();
}

const MIC = "M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM5 11a7 7 0 0 0 14 0M12 18v3";
const CURSOR = "M10.5 10.5 20 14l-4 1.8L14.2 20l-3.7-9.5ZM10.5 3v2.2M4.6 5.1l1.6 1.6M3 10.5h2.2M4.6 15.9l1.6-1.6M16.4 5.1l-1.6 1.6";
const KEYS = "M3 7h18v10H3zM6.5 10h1M10 10h1M13.5 10h1M17 10h.5M7 14h10";
const LOCK = "M7.5 11V8a4.5 4.5 0 0 1 9 0v3M5.5 11h13v9.5h-13z";
const WAVE = "M4 10.5v3M8 7v10M12 3.5v17M16 7.5v9M20 10.5v3";

function stage(c: Ctx, w: number, h: number, t: number, p: Palette, moving: boolean) {
  const g = c.createLinearGradient(0, 0, 0, h);
  g.addColorStop(0, p.surface);
  g.addColorStop(1, p.bg);
  c.fillStyle = g;
  c.fillRect(0, 0, w, h);
  c.fillStyle = alpha(p.text, 0.07);
  for (let y = 8; y < h; y += 16) {
    for (let x = 8; x < w; x += 16) {
      c.beginPath();
      c.arc(x, y, 0.75, 0, Math.PI * 2);
      c.fill();
    }
  }
  const drift = moving ? Math.sin(t * 0.35) * 40 : 0;
  const cx = w / 2 + drift;
  const cy = h * 0.55;
  const r = w * 0.42;
  const a = c.createRadialGradient(cx, cy, 4, cx, cy, r);
  a.addColorStop(0, alpha(p.accent, 0.26));
  a.addColorStop(1, alpha(p.accent, 0));
  c.fillStyle = a;
  c.fillRect(0, 0, w, h);
}

function voiceToText(c: Ctx, w: number, h: number, t: number, p: Palette) {
  const midY = h / 2;
  const barW = 5;
  const barGap = 6;
  const waveStart = w * 0.2 - (9 * (barW + barGap)) / 2;
  for (let i = 0; i < 9; i++) {
    const envelope = 1 - Math.abs(i - 4) / 5.5;
    const level = Math.abs(Math.sin(t * 2.3 + i * 0.72)) * envelope;
    const bh = 10 + level * 58;
    const x = waveStart + i * (barW + barGap);
    const y = midY - bh / 2;
    const g = c.createLinearGradient(0, y, 0, y + bh);
    g.addColorStop(0, p.accent);
    g.addColorStop(1, alpha(p.accent, 0.45));
    c.fillStyle = g;
    rr(c, x, y, barW, bh, barW / 2);
    c.fill();
  }
  const from = { x: w * 0.33, y: midY };
  const to = { x: w * 0.56, y: midY };
  const ctl = { x: w * 0.445, y: midY - 46 };
  c.strokeStyle = alpha(p.accent, 0.16);
  c.lineWidth = 1;
  c.setLineDash([3, 4]);
  c.beginPath();
  c.moveTo(from.x, from.y);
  c.quadraticCurveTo(ctl.x, ctl.y, to.x, to.y);
  c.stroke();
  c.setLineDash([]);
  for (let i = 0; i < 7; i++) {
    const ph = (t * 0.5 + i / 7) % 1;
    const u = 1 - ph;
    const x = u * u * from.x + 2 * u * ph * ctl.x + ph * ph * to.x;
    const y = u * u * from.y + 2 * u * ph * ctl.y + ph * ph * to.y;
    c.fillStyle = alpha(p.accent, Math.sin(ph * Math.PI));
    c.beginPath();
    c.arc(x, y, 2.2 + 1.3 * Math.sin(ph * Math.PI), 0, Math.PI * 2);
    c.fill();
  }
  const card = { x: w * 0.58, y: midY - 42, w: w * 0.3, h: 84 };
  rr(c, card.x, card.y, card.w, card.h, 12);
  c.fillStyle = alpha(p.text, 0.05);
  c.fill();
  c.strokeStyle = alpha(p.text, 0.12);
  c.lineWidth = 1;
  c.stroke();
  const typing = Math.min(1.15, (t * 0.32) % 1.4) * 3;
  let caret: { x: number; y: number } | null = null;
  [0.86, 0.72, 0.5].forEach((share, i) => {
    const y = card.y + 22 + i * 20;
    const full = (card.w - 28) * share;
    const typed = full * Math.max(0, Math.min(1, typing - i));
    rr(c, card.x + 14, y - 2.5, full, 5, 2.5);
    c.fillStyle = alpha(p.text, 0.08);
    c.fill();
    if (typed > 0) {
      rr(c, card.x + 14, y - 2.5, typed, 5, 2.5);
      c.fillStyle = alpha(p.text, 0.75);
      c.fill();
      caret = { x: card.x + 14 + typed + 3, y };
    }
  });
  if (caret && Math.sin(t * 7) > -0.2) {
    const k = caret as { x: number; y: number };
    rr(c, k.x, k.y - 7, 2, 14, 1);
    c.fillStyle = p.accent;
    c.fill();
  }
}

function shieldPath(x: number, y: number, w: number, h: number) {
  const q = new Path2D();
  const mid = x + w / 2;
  q.moveTo(mid, y);
  q.lineTo(x + w, y + h * 0.17);
  q.lineTo(x + w, y + h * 0.48);
  q.bezierCurveTo(x + w, y + h * 0.78, mid + w * 0.22, y + h - h * 0.06, mid, y + h);
  q.bezierCurveTo(mid - w * 0.22, y + h - h * 0.06, x, y + h * 0.78, x, y + h * 0.48);
  q.lineTo(x, y + h * 0.17);
  q.closePath();
  return q;
}

function orbiter(c: Ctx, d: string, angle: number, rx: number, ry: number, cx: number, cy: number, p: Palette, small = false) {
  const depth = (Math.sin(angle) + 1) / 2;
  const size = (small ? 24 : 30) * (0.82 + 0.22 * depth);
  const x = cx + rx * Math.cos(angle);
  const y = cy + ry * Math.sin(angle);
  c.save();
  c.globalAlpha = 0.55 + 0.45 * depth;
  c.beginPath();
  c.arc(x, y, size / 2, 0, Math.PI * 2);
  c.fillStyle = p.tile;
  c.fill();
  c.strokeStyle = alpha(p.accent, 0.45);
  c.lineWidth = 1;
  c.stroke();
  icon(c, d, x, y, small ? 12 : 15, alpha(p.text, 0.9), 2);
  c.restore();
}

function shield(c: Ctx, w: number, h: number, t: number, p: Palette, done: boolean, shown: number) {
  const cx = w / 2;
  const cy = h / 2;
  const tint = done ? p.ok : p.accent;
  [[250, 74, 1], [170, 118, -1]].forEach(([ew, eh, dir]) => {
    c.strokeStyle = alpha(p.text, 0.13);
    c.lineWidth = 1;
    c.setLineDash([2, 6]);
    c.lineDashOffset = -t * 10 * dir;
    c.beginPath();
    c.ellipse(cx, cy, ew / 2, eh / 2, 0, 0, Math.PI * 2);
    c.stroke();
  });
  c.setLineDash([]);
  c.lineDashOffset = 0;
  glow(c, cx, cy, 150, tint, done ? 0.4 : 0.28);
  const s = shieldPath(cx - 35, cy - 42, 70, 84);
  c.fillStyle = alpha(tint, done ? 0.22 : 0.1);
  c.fill(s);
  const drawn = Math.min(1, shown / 1.3);
  c.strokeStyle = tint;
  c.lineWidth = 2.5;
  c.lineCap = "round";
  c.lineJoin = "round";
  c.setLineDash([300 * (1 - (1 - drawn) ** 2), 300]);
  c.stroke(s);
  c.setLineDash([]);
  if (done) {
    c.strokeStyle = p.ok;
    c.lineWidth = 4;
    c.beginPath();
    c.moveTo(cx - 14, cy + 3);
    c.lineTo(cx - 3.4, cy + 12);
    c.lineTo(cx + 14, cy - 8);
    c.stroke();
  } else {
    icon(c, LOCK, cx, cy + 2, 22 * (1 + 0.06 * Math.sin(t * 2.4)), tint, 2.2);
  }
  orbiter(c, MIC, t * 0.7, 125, 37, cx, cy, p);
  orbiter(c, CURSOR, t * 0.7 + Math.PI, 125, 37, cx, cy, p);
  orbiter(c, KEYS, -t * 0.5 + Math.PI / 2, 85, 59, cx, cy, p, true);
}

function engine(c: Ctx, w: number, h: number, t: number, p: Palette, done: boolean) {
  const cx = w / 2;
  const cy = h / 2;
  const tint = done ? p.ok : p.accent;
  for (let i = 0; i < 10; i++) {
    const a = (i / 10) * 2 * Math.PI + 0.2;
    const bob = Math.sin(t * 1.3 + i) * 3;
    const nx = cx + 205 * Math.cos(a);
    const ny = cy + 54 * Math.sin(a) + bob;
    c.strokeStyle = alpha(p.text, 0.09);
    c.lineWidth = 1;
    c.beginPath();
    c.moveTo(nx, ny);
    c.lineTo(cx, cy);
    c.stroke();
    const ph = (t * 0.55 + i * 0.17) % 1;
    c.fillStyle = alpha(done ? p.ok : p.accent, Math.sin(ph * Math.PI));
    c.beginPath();
    c.arc(nx + (cx - nx) * ph, ny + (cy - ny) * ph, 2.5, 0, Math.PI * 2);
    c.fill();
    c.beginPath();
    c.arc(nx, ny, 4, 0, Math.PI * 2);
    c.fillStyle = p.tile;
    c.fill();
    c.strokeStyle = alpha(p.accent, 0.6);
    c.lineWidth = 1.2;
    c.stroke();
  }
  glow(c, cx, cy, 170, tint, 0.3 + 0.15 * Math.sin(t * 2));
  for (let side = 0; side < 4; side++) {
    c.save();
    c.translate(cx, cy);
    c.rotate((side * Math.PI) / 2);
    c.fillStyle = alpha(tint, 0.7);
    for (let k = 0; k < 4; k++) {
      rr(c, -14.5 + k * 9.5 - 1.25, -44.5, 2.5, 9, 1.25);
      c.fill();
    }
    c.restore();
  }
  rr(c, cx - 33, cy - 33, 66, 66, 14);
  c.fillStyle = p.tile;
  c.fill();
  c.strokeStyle = tint;
  c.lineWidth = 1.5;
  c.stroke();
  if (done) {
    c.strokeStyle = tint;
    c.lineWidth = 4;
    c.lineCap = "round";
    c.lineJoin = "round";
    c.beginPath();
    c.moveTo(cx - 11, cy + 1);
    c.lineTo(cx - 3, cy + 9);
    c.lineTo(cx + 12, cy - 8);
    c.stroke();
  } else {
    icon(c, WAVE, cx, cy, 28, tint, 2.4);
  }
}

function keyPress(c: Ctx, w: number, h: number, t: number, p: Palette) {
  const cx = w / 2;
  const cy = h / 2 + 14;
  const period = 1.8;
  const cycle = (t % period) / period;
  const press = cycle < 0.12 ? Math.sin((cycle / 0.12) * Math.PI) : 0;
  c.lineCap = "round";
  for (let i = 0; i < 16; i++) {
    const a = (i / 16) * 2 * Math.PI + t * 0.15;
    c.strokeStyle = alpha(p.accent, 0.22);
    c.lineWidth = 1.5;
    c.beginPath();
    c.moveTo(cx + 70 * Math.cos(a), cy - 8 + 70 * Math.sin(a));
    c.lineTo(cx + 84 * Math.cos(a), cy - 8 + 84 * Math.sin(a));
    c.stroke();
  }
  for (let ring = 0; ring < 2; ring++) {
    const ph = (cycle + ring * 0.5) % 1;
    const r = 46 + ph * 110;
    c.strokeStyle = alpha(p.accent, 0.4 * (1 - ph));
    c.lineWidth = 1.5;
    c.beginPath();
    c.ellipse(cx, cy - 8, r, r * 0.62, 0, 0, Math.PI * 2);
    c.stroke();
  }
  const waveY = cy - 62;
  const amp = 12 * (1 - cycle);
  c.strokeStyle = alpha(p.accent, 0.8);
  c.lineWidth = 2;
  c.lineJoin = "round";
  c.beginPath();
  for (let s = 0; s <= 60; s++) {
    const x = cx - 90 + s * 3;
    const y = waveY + Math.sin(s * 0.45 - t * 6) * amp * Math.sin((s / 60) * Math.PI);
    if (s === 0) c.moveTo(x, y);
    else c.lineTo(x, y);
  }
  c.stroke();
  glow(c, cx, cy, 190, p.accent, 0.18 + 0.32 * press);
  rr(c, cx - 52, cy - 32 + 6, 104, 64, 14);
  c.fillStyle = p.bg;
  c.fill();
  const top = cy - 32 + press * 5;
  const g = c.createLinearGradient(0, top, 0, top + 64);
  g.addColorStop(0, p.tile);
  g.addColorStop(1, p.surface);
  rr(c, cx - 52, top, 104, 64, 14);
  c.fillStyle = g;
  c.fill();
  c.strokeStyle = alpha(p.accent, 0.35 + 0.5 * press);
  c.lineWidth = 1.2;
  c.stroke();
  const ink = press > 0 ? p.accent : alpha(p.text, 0.85);
  icon(c, MIC, cx - 19, cy + press * 5, 16, ink, 2.2);
  c.fillStyle = ink;
  c.font = "600 13px \"Inter Variable\", sans-serif";
  c.textBaseline = "middle";
  c.fillText("hold", cx - 8, cy + 1 + press * 5);
}

export default function SetupArt({ step, complete }: { step: number; complete: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const live = useRef({ step, complete, shownAt: performance.now() });
  live.current.complete = complete;
  if (live.current.step !== step) {
    live.current.step = step;
    live.current.shownAt = performance.now();
  }

  useEffect(() => {
    const el = canvas.current;
    const c = el?.getContext("2d");
    if (!el || !c) return;
    let frame = 0;
    let last = 0;
    let palette = readPalette();
    const observer = new MutationObserver(() => {
      palette = readPalette();
      if (reducedMotion()) draw(performance.now());
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

    function draw(now: number) {
      if (!el || !c) return;
      const dpr = window.devicePixelRatio || 1;
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (el.width !== Math.round(w * dpr)) {
        el.width = Math.round(w * dpr);
        el.height = Math.round(h * dpr);
      }
      const still = reducedMotion();
      const t = still ? 1.3 : now / 1000;
      c.setTransform(dpr, 0, 0, dpr, 0, 0);
      c.clearRect(0, 0, w, h);
      stage(c, w, h, t, palette, !still);
      const { step: s, complete: done, shownAt } = live.current;
      if (s === 0) voiceToText(c, w, h, t, palette);
      else if (s === 1) shield(c, w, h, t, palette, done, still ? 2 : (now - shownAt) / 1000);
      else if (s === 2) engine(c, w, h, t, palette, done);
      else keyPress(c, w, h, t, palette);
    }
    const tick = (now: number) => {
      frame = requestAnimationFrame(tick);
      if (now - last < 33) return;
      last = now;
      draw(now);
    };
    if (reducedMotion()) draw(performance.now());
    else frame = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [step, complete]);

  return <canvas ref={canvas} className="setup-art" aria-hidden="true" />;
}

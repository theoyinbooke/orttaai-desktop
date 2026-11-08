// Coach's SF Symbol look-alikes, drawn on the same 24-unit grid as `ui.tsx`'s
// icons. Only the symbols Coach uses that `ui.tsx` has no match for. (Coach area.)
import type { ReactNode } from "react";

const lobes = (() => {
  // checkmark.seal's scalloped edge: sixteen points, alternating in and out.
  const points: string[] = [];
  for (let i = 0; i < 16; i++) {
    const r = i % 2 ? 8.1 : 9.7;
    const a = (Math.PI * 2 * i) / 16 - Math.PI / 2;
    points.push(`${(12 + r * Math.cos(a)).toFixed(2)} ${(12 + r * Math.sin(a)).toFixed(2)}`);
  }
  return `M${points.join("L")}Z`;
})();

const LINES: Record<string, ReactNode> = {
  // sun.max
  sunMax: (
    <>
      <circle cx="12" cy="12" r="4.2" />
      <path d="M12 2.5v2.4M12 19.1v2.4M2.5 12h2.4M19.1 12h2.4M5.3 5.3 7 7M17 17l1.7 1.7M18.7 5.3 17 7M7 17l-1.7 1.7" />
    </>
  ),
  // point.topleft.down.to.point.bottomright.curvepath
  curvePath: (
    <>
      <circle cx="4.8" cy="5" r="2.1" />
      <circle cx="19.2" cy="19" r="2.1" />
      <path d="M7 5h7a3.5 3.5 0 0 1 0 7h-4a3.5 3.5 0 0 0 0 7h7" />
    </>
  ),
  // chart.line.uptrend.xyaxis
  chartUp: <path d="M3.5 3.5v15a2 2 0 0 0 2 2h15M7.5 15.5l3.6-4.2 3.2 2.6 5.2-6.9" />,
  // person.2
  person2: (
    <>
      <circle cx="9" cy="8" r="3.6" />
      <path d="M2.8 20c0-3.5 2.7-5.5 6.2-5.5s6.2 2 6.2 5.5" />
      <path d="M15.4 4.7a3.6 3.6 0 0 1 0 6.6M17.6 14.8c2.2.6 3.6 2.4 3.6 5.2" />
    </>
  ),
  // square.and.arrow.up
  shareUp: <path d="M12 14.5V3M8.2 6.6 12 2.8l3.8 3.8M8.5 10.2H7a1.5 1.5 0 0 0-1.5 1.5v8A1.5 1.5 0 0 0 7 21.2h10a1.5 1.5 0 0 0 1.5-1.5v-8a1.5 1.5 0 0 0-1.5-1.5h-1.5" />,
  // text.bubble
  textBubble: <path d="M5 3.8h14a2.2 2.2 0 0 1 2.2 2.2v9a2.2 2.2 0 0 1-2.2 2.2h-7l-4.2 3.3v-3.3H5A2.2 2.2 0 0 1 2.8 15V6A2.2 2.2 0 0 1 5 3.8ZM7.2 8.5h9.6M7.2 12.5h6.4" />,
  // checkmark.seal
  checkSeal: (
    <>
      <path d={lobes} />
      <path d="M8.2 12.2l2.6 2.6 5-5.4" />
    </>
  ),
  // bubble.left.and.bubble.right
  bubbles: <path d="M4.6 3.2h10a2.2 2.2 0 0 1 2.2 2.2v6a2.2 2.2 0 0 1-2.2 2.2H9.2l-3.2 2.7v-2.7H4.6a2.2 2.2 0 0 1-2.2-2.2v-6a2.2 2.2 0 0 1 2.2-2.2ZM19.6 8.2a2.2 2.2 0 0 1 2 2.2v5.7a2.2 2.2 0 0 1-2.2 2.2H18v2.7l-3.2-2.7h-3.4a2.2 2.2 0 0 1-2-1.3" />,
  // arrow.counterclockwise
  arrowCcw: <path d="M12 4.2a8.2 8.2 0 1 1-8.2 8.2M12 1.4 8.6 4.2 12 7" />,
  // square.text.square
  squareText: (
    <>
      <rect x="3.2" y="3.2" width="17.6" height="17.6" rx="4.4" />
      <path d="M7.6 8.8h8.8M7.6 12.2h8.8M7.6 15.6h5" />
    </>
  ),
  // timer
  timer: <path d="M12 3.2a8.8 8.8 0 1 1-6.2 2.6M12 3.2v3.6M12 12 7.6 7.6" />,
  // checkmark, heavier
  checkmark: <path d="M5 12.8l4.4 4.4L19 7.2" />,
};

const SOLIDS: Record<string, ReactNode> = {
  // mic.fill
  micFill: (
    <>
      <rect x="8.4" y="2" width="7.2" height="13.4" rx="3.6" />
      <path d="M5.4 10.6a.9.9 0 0 1 .9.9 5.7 5.7 0 0 0 11.4 0 .9.9 0 1 1 1.8 0 7.5 7.5 0 0 1-6.6 7.44V21h2.7a.9.9 0 1 1 0 1.8H8.4a.9.9 0 1 1 0-1.8h2.7v-2.06a7.5 7.5 0 0 1-6.6-7.44.9.9 0 0 1 .9-.9Z" />
    </>
  ),
  // stop.fill
  stopFill: <rect x="4.5" y="4.5" width="15" height="15" rx="2.6" />,
  // play.fill
  playFill: <path d="M7 3.9c0-1 1.1-1.6 2-1.1l11.4 8.1c.8.5.8 1.7 0 2.2L9 21.2c-.9.5-2-.1-2-1.1V3.9Z" />,
};

export function CoachIcon({ name, size = 18, weight = 1.75 }: { name: string; size?: number; weight?: number }) {
  const solid = SOLIDS[name];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={solid ? "currentColor" : "none"}
      stroke={solid ? "none" : "currentColor"}
      strokeWidth={solid ? undefined : weight}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ flex: "none" }}
    >
      {solid ?? LINES[name] ?? null}
    </svg>
  );
}

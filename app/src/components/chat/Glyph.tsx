// The SF Symbols the Mac's ChatAI page uses, drawn as line icons. A symbol set at
// N points fills about 1.2 N pixels, so callers pass that size.
import type { ReactNode } from "react";

const SHAPES: Record<string, ReactNode> = {
  history: (
    <>
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5M12 7v5l4 2" />
    </>
  ),
  compose: (
    <>
      <path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
      <path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
    </>
  ),
  x: <path d="M6 6l12 12M18 6 6 18" />,
  pencilLine: (
    <>
      <path d="M12 20h9" />
      <path d="M16.376 3.622a1 1 0 0 1 3.002 3.002L7.368 18.635a2 2 0 0 1-.855.506l-2.872.838a.5.5 0 0 1-.62-.62l.838-2.872a2 2 0 0 1 .506-.854z" />
      <path d="m15 5 3 3" />
    </>
  ),
  personWave: (
    <>
      <circle cx="12" cy="8.5" r="3.2" />
      <path d="M6.5 20.5c.4-3.6 2.7-5.6 5.5-5.6s5.1 2 5.5 5.6" />
      <path d="M6.4 5a8 8 0 0 0 0 7M3.4 3a12 12 0 0 0 0 11M17.6 5a8 8 0 0 1 0 7M20.6 3a12 12 0 0 1 0 11" />
    </>
  ),
  textSearch: (
    <>
      <path d="M3 6h18M3 12h7M3 18h7" />
      <circle cx="16.5" cy="15.5" r="3.5" />
      <path d="m19 18 2.5 2.5" />
    </>
  ),
  textBubble: (
    <>
      <path d="M5 4h14a2.5 2.5 0 0 1 2.5 2.5v9A2.5 2.5 0 0 1 19 18h-8l-4.5 3.5V18H5a2.5 2.5 0 0 1-2.5-2.5v-9A2.5 2.5 0 0 1 5 4Z" />
      <path d="M7.5 9h9M7.5 13h6" />
    </>
  ),
  bookClosed: (
    <>
      <path d="M6.5 3H19v18H6.5A2.5 2.5 0 0 1 4 18.5v-13A2.5 2.5 0 0 1 6.5 3Z" />
      <path d="M4 18.5A2.5 2.5 0 0 1 6.5 16H19M9 7.5h6M9 11h6" />
    </>
  ),
  laptop: (
    <>
      <path d="M5 5.5A1.5 1.5 0 0 1 6.5 4h11A1.5 1.5 0 0 1 19 5.5V16H5Z" />
      <path d="M2.5 19.5h19" />
    </>
  ),
  clip: <path d="m21 11-8.6 8.6a5 5 0 0 1-7-7l8.5-8.6a3.3 3.3 0 0 1 4.7 4.7l-8.6 8.6a1.7 1.7 0 0 1-2.3-2.3l7.9-8" />,
  gear: (
    <>
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  refresh: (
    <>
      <path d="M21 12a9 9 0 1 1-2.64-6.36L21 8" />
      <path d="M21 3v5h-5" />
    </>
  ),
  sliders: (
    <>
      <path d="M3 7h11M18 7h3M3 17h3M10 17h11" />
      <circle cx="16" cy="7" r="2" />
      <circle cx="8" cy="17" r="2" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  bolt: <path d="M13 2 4 14h7l-1 8 10-13h-7z" fill="currentColor" />,
  brain: (
    <path d="M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 6 1V5a2 2 0 0 0-3-1Zm6 0a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-6 1" />
  ),
  chevron: <path d="m5 9 7 7 7-7" />,
  arrowUp: <path d="M12 20V5M5.5 11.5 12 5l6.5 6.5" />,
  doc: <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5ZM14 3v5h5M9 13h6M9 17h4" />,
  copy: (
    <>
      <rect x="9" y="9" width="12" height="12" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </>
  ),
  check: <path d="m5 12.5 4.5 4.5L19 7" />,
  trash: <path d="M3.5 6h17M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 6l1 14a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-14M10 11v6M14 11v6" />,
  bubble: <path d="M5 4h14a2.5 2.5 0 0 1 2.5 2.5v9A2.5 2.5 0 0 1 19 18h-8l-4.5 3.5V18H5a2.5 2.5 0 0 1-2.5-2.5v-9A2.5 2.5 0 0 1 5 4Z" />,
  bubbles: <path d="M3 5.5h11.5v8H8l-3 2.6v-2.6H3v-8ZM17 9h4v8h-1.8v2.4L16.4 17H11v-1.6" />,
  xCircle: (
    <>
      <circle cx="12" cy="12" r="10" fill="currentColor" stroke="none" />
      <path d="m8.5 8.5 7 7M15.5 8.5l-7 7" stroke="var(--bg)" strokeWidth={2.4} />
    </>
  ),
  stop: <rect x="6.5" y="6.5" width="11" height="11" rx="2" fill="currentColor" stroke="none" />,
};

export type GlyphName = keyof typeof SHAPES;

/** `weight` is the stroke in icon units: about 1.6 regular, 2 medium, 3 bold. */
export function Glyph({ name, size = 13, weight = 2, className }: { name: GlyphName; size?: number; weight?: number; className?: string }) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={weight}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {SHAPES[name]}
    </svg>
  );
}

/** The Mac's signal-cursor mark: three rising bars and a text cursor. */
export function SignalMark({ height, className }: { height: number; className?: string }) {
  return (
    <svg className={className} viewBox="0 0 100 84" width={(height * 100) / 84} height={height} aria-hidden="true">
      <rect x="0" y="44" width="18" height="36" rx="9" />
      <rect x="25" y="28" width="18" height="52" rx="9" />
      <rect x="50" y="12" width="18" height="68" rx="9" />
      <rect x="85" y="3" width="6" height="78" rx="3" />
      <rect x="77" y="0" width="22" height="6" rx="3" />
      <rect x="77" y="78" width="22" height="6" rx="3" />
    </svg>
  );
}

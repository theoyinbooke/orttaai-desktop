// Shared building blocks for every settings-style page (Mac SettingsComponents).
// Rows, cards, info affordances and tabs come from here so they size, align and
// behave the same on every page.
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon, Switch, useToast } from "../ui";
import { useSettings, type Settings, type SettingsPatch } from "../lib/settings";
import "../styles/settings.css";

/* ---------------------------------------------------------------- glyphs -- */

/** SF Symbol look-alikes the Mac's settings pages use and `ui.tsx` doesn't have. */
const GLYPHS: Record<string, ReactNode> = {
  alignLeft: <path d="M4 6h16M4 10h10M4 14h16M4 18h10" />,
  textPlus: <path d="M4 6h16M4 11h9M4 16h6M17 13v7M13.5 16.5h7" />,
  textInsert: <path d="M4 5h16M4 19h16M4 12h8M9 8.5l3.5 3.5L9 15.5M17 8v8" />,
  bulb: (
    <>
      <path d="M12 3a6.5 6.5 0 0 1 4 11.6c-.7.6-1 1.3-1 2.1v.3H9v-.3c0-.8-.3-1.5-1-2.1A6.5 6.5 0 0 1 12 3Z" />
      <path d="M9.5 20.5h5" />
    </>
  ),
  brain: (
    <>
      <path d="M12 5.5a3 3 0 0 0-5.6-1.2A3.2 3.2 0 0 0 4 9.8a3.4 3.4 0 0 0 .6 6A3.3 3.3 0 0 0 9.5 20c1.4 0 2.5-1 2.5-2.4V5.5Z" />
      <path d="M12 5.5a3 3 0 0 1 5.6-1.2A3.2 3.2 0 0 1 20 9.8a3.4 3.4 0 0 1-.6 6 3.3 3.3 0 0 1-4.9 4.2c-1.4 0-2.5-1-2.5-2.4" />
    </>
  ),
  eyeSlash: (
    <>
      <path d="M9.9 5.2A10 10 0 0 1 12 5c5.5 0 9 5 9.5 7a13 13 0 0 1-2.8 3.8M6.2 7.4A13 13 0 0 0 2.5 12c.5 2 4 7 9.5 7a9.8 9.8 0 0 0 4.4-1" />
      <path d="M10 10a3 3 0 0 0 4.2 4.2M4 4l16 16" />
    </>
  ),
  seal: (
    <>
      <path d="M12 2.8l2.4 1.8 3-.2.9 2.9 2.4 1.8-1 2.9 1 2.9-2.4 1.8-.9 2.9-3-.2L12 21.2l-2.4-1.8-3 .2-.9-2.9-2.4-1.8 1-2.9-1-2.9 2.4-1.8.9-2.9 3 .2L12 2.8Z" />
      <path d="M8.5 12l2.5 2.5 4.5-5" />
    </>
  ),
  ant: (
    <>
      <circle cx="12" cy="6.5" r="2.5" />
      <ellipse cx="12" cy="11.5" rx="2" ry="2.5" />
      <ellipse cx="12" cy="18" rx="3" ry="4" />
      <path d="M10 5l-2-2.5M14 5l2-2.5M10 11l-5-2M14 11l5-2M9.5 15.5L5 14.5M14.5 15.5l4.5-1M9.5 19L5.5 21M14.5 19l4 2" />
    </>
  ),
  lifebuoy: (
    <>
      <circle cx="12" cy="12" r="9" />
      <circle cx="12" cy="12" r="3.5" />
      <path d="M5.6 5.6l3.9 3.9M14.5 14.5l3.9 3.9M18.4 5.6l-3.9 3.9M9.5 14.5l-3.9 3.9" />
    </>
  ),
  externalSquare: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="3" />
      <path d="M9.5 14.5L15 9M10.5 9H15v4.5" />
    </>
  ),
  playRect: (
    <>
      <rect x="3" y="5" width="18" height="14" rx="3" />
      <path d="M10 9l5 3-5 3V9Z" />
    </>
  ),
  code: <path d="M8 7l-5 5 5 5M16 7l5 5-5 5M14 4l-4 16" />,
  cylinder: (
    <>
      <ellipse cx="12" cy="6" rx="8" ry="3" />
      <path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6" />
    </>
  ),
  downCircle: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5v9M8 12.5l4 4 4-4" />
    </>
  ),
  cpu: (
    <>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" />
    </>
  ),
  clearFill: (
    <>
      <circle cx="12" cy="12" r="10" fill="currentColor" stroke="none" />
      <path d="M8.8 8.8l6.4 6.4M15.2 8.8l-6.4 6.4" stroke="var(--surface)" />
    </>
  ),
};

/** A local outline glyph, drawn like the `Icon`s in `ui.tsx`. */
export function Glyph({ name, size = 13, weight = 2 }: { name: string; size?: number; weight?: number }) {
  return (
    <svg
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
      {GLYPHS[name] ?? null}
    </svg>
  );
}

/* ------------------------------------------------------------------ tabs -- */

/** Icon-and-title capsules beside the page title (Mac OrttaaiTabBar). */
export function TabBar<T extends string>({
  tabs,
  value,
  onChange,
  label,
}: {
  tabs: { value: T; label: string; icon: ReactNode }[];
  value: T;
  onChange: (v: T) => void;
  label?: string;
}) {
  return (
    <div className="st-tabbar" role="tablist" aria-label={label}>
      {tabs.map((t) => (
        <button
          key={t.value}
          type="button"
          role="tab"
          aria-selected={value === t.value}
          className={`st-tab ${value === t.value ? "active" : ""}`}
          onClick={() => onChange(t.value)}
        >
          <span className="st-tab-icon">{t.icon}</span>
          {t.label}
        </button>
      ))}
    </div>
  );
}

/* ------------------------------------------------------------------ info -- */

const POPOVER_WIDTH = 260;

/**
 * Small info icon beside a label. Hover shows a tooltip; clicking opens a popover
 * so the explanation is also reachable without a mouse hover (Mac InfoButton).
 */
export function InfoButton({ title, text }: { title: ReactNode; text: ReactNode }) {
  const [rect, setRect] = useState<DOMRect | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const popover = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!rect) return;
    const close = () => setRect(null);
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (button.current?.contains(t) || popover.current?.contains(t)) return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [rect]);

  const place = () => {
    if (!rect) return undefined;
    const center = rect.left + rect.width / 2;
    const left = Math.max(8, Math.min(window.innerWidth - POPOVER_WIDTH - 8, center - POPOVER_WIDTH / 2));
    const above = window.innerHeight - rect.bottom < 160;
    const style: CSSProperties & { "--st-arrow": string } = {
      left,
      width: POPOVER_WIDTH,
      ...(above ? { bottom: window.innerHeight - rect.top + 8 } : { top: rect.bottom + 8 }),
      "--st-arrow": `${center - left}px`,
    };
    return { above, style };
  };
  const placed = place();
  const name = typeof title === "string" ? title : "this setting";

  return (
    <>
      <button
        ref={button}
        type="button"
        className={`st-info ${rect ? "open" : ""}`}
        aria-label={`About ${name}`}
        aria-expanded={!!rect}
        title={typeof text === "string" && !rect ? text : undefined}
        onClick={() => setRect((r) => (r ? null : (button.current?.getBoundingClientRect() ?? null)))}
      >
        <Icon name="info" size={13} />
      </button>
      {placed &&
        createPortal(
          <div ref={popover} className={`st-popover ${placed.above ? "above" : ""}`} role="tooltip" style={placed.style}>
            {text}
          </div>,
          document.body,
        )}
    </>
  );
}

/* --------------------------------------------------------- cards and rows -- */

/**
 * A titled group of rows (Mac SettingsCard). `desc` is the title's info text;
 * `accessory` sits at the trailing edge of the title line.
 */
export function Section({
  title,
  desc,
  accessory,
  className = "",
  children,
}: {
  title?: string;
  desc?: ReactNode;
  accessory?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={`set-section ${className}`}>
      {title && (
        <header className="set-section-head">
          <span className="set-section-title">
            <h2>{title}</h2>
            {desc && <InfoButton title={title} text={desc} />}
          </span>
          {accessory && <div className="set-section-accessory">{accessory}</div>}
        </header>
      )}
      <div className="set-rows">{children}</div>
    </section>
  );
}

/** Label on the leading edge with its info icon, any control on the trailing edge. */
export function Row({
  label,
  hint,
  children,
  disabled,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`set-row ${disabled ? "is-disabled" : ""}`}>
      <div className="set-row-text">
        <span className="set-row-label">{label}</span>
        {hint && <InfoButton title={label} text={hint} />}
      </div>
      <div className="set-row-control">{children}</div>
    </div>
  );
}

/** Save a patch immediately, toasting any error. */
export function usePatch() {
  const { patch } = useSettings();
  const toast = useToast();
  return (p: SettingsPatch) => patch(p).catch((e) => toast(String(e), "error"));
}

type BoolKey = { [K in keyof Settings]: Settings[K] extends boolean ? K : never }[keyof Settings];

/** A row bound to a boolean setting. */
export function ToggleRow({
  k,
  label,
  hint,
  disabled,
}: {
  k: BoolKey;
  label: ReactNode;
  hint?: ReactNode;
  disabled?: boolean;
}) {
  const { settings } = useSettings();
  const save = usePatch();
  if (!settings) return null;
  return (
    <Row label={label} hint={hint} disabled={disabled}>
      <Switch
        checked={settings[k] as boolean}
        label={typeof label === "string" ? label : undefined}
        onChange={(v: boolean) => save({ [k]: v } as SettingsPatch)}
        disabled={disabled}
      />
    </Row>
  );
}

/** Number slider with a live value readout. */
export function SliderRow({
  label,
  hint,
  value,
  min,
  max,
  step = 1,
  format = (v) => String(v),
  onCommit,
  disabled,
}: {
  label: ReactNode;
  hint?: ReactNode;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  onCommit: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <Row label={label} hint={hint} disabled={disabled}>
      <div className="slider">
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          defaultValue={value}
          key={value}
          disabled={disabled}
          onChange={(e) => {
            const out = e.currentTarget.parentElement?.querySelector("output");
            if (out) out.textContent = format(Number(e.currentTarget.value));
          }}
          onPointerUp={(e) => onCommit(Number((e.target as HTMLInputElement).value))}
          onKeyUp={(e) => onCommit(Number((e.target as HTMLInputElement).value))}
        />
        <output className="mono small">{format(value)}</output>
      </div>
    </Row>
  );
}

/* --------------------------------------------------------------- notices -- */

const NOTICE_ICON = { error: "alert", warning: "alert", success: "check", info: "info" } as const;

/** Inline status message shown next to the action that produced it (Mac SettingsNotice). */
export function Notice({
  kind = "info",
  children,
  onDismiss,
}: {
  kind?: "error" | "warning" | "success" | "info";
  children: ReactNode;
  onDismiss?: () => void;
}) {
  return (
    <div className={`st-notice st-notice-${kind}`} role="status">
      <Icon name={NOTICE_ICON[kind]} size={13} />
      <span className="st-notice-text">{children}</span>
      {onDismiss && (
        <button type="button" className="st-notice-x" aria-label="Dismiss" onClick={onDismiss}>
          <Icon name="x" size={10} />
        </button>
      )}
    </div>
  );
}

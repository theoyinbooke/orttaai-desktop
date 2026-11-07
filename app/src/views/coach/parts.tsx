// Coach's building blocks (Mac CoachStyle + CoachShellView pieces): the page
// frame, eyebrow, tags, the ⓘ popover, buttons, the lens picker, and the
// context every Coach page reads. (Coach area.)
import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type {
  Catalog,
  HabitStatus,
  JudgeState,
  PersonaId,
  RepKind,
  SkillInfo,
  Snapshot,
} from "../../lib/coach";
import { statusTitle } from "../../lib/coach";
import { Icon } from "../../ui";
import type { Settings } from "../../lib/settings";

export type CoachTab = "today" | "path" | "studio" | "proof" | "cast";

export type CoachContextValue = {
  snapshot: Snapshot;
  catalog: Catalog;
  judge: JudgeState;
  settings: Settings | null;
  refresh: () => void;
  beginRep: (skillId: string, kind: RepKind, spoken: boolean) => void;
  beginScene: (sceneId: string) => void;
  setTab: (tab: CoachTab) => void;
  openSetup: (atSetup?: boolean) => void;
};

export const CoachCtx = createContext<CoachContextValue | null>(null);

export function useCoach(): CoachContextValue {
  const v = useContext(CoachCtx);
  if (!v) throw new Error("useCoach outside Coach");
  return v;
}

/** Catalog lookups. */
export function skillOf(catalog: Catalog, id: string): SkillInfo {
  return catalog.skills.find((s) => s.id === id) ?? catalog.skills[0];
}
export function personaOf(catalog: Catalog, id: PersonaId) {
  return catalog.personas.find((p) => p.id === id) ?? catalog.personas[0];
}
export function checkLabel(catalog: Catalog, id: string): string {
  return catalog.checks.find((c) => c.id === id)?.label ?? (id.startsWith("g-") ? "Reached the goal" : id);
}
export function trackOf(catalog: Catalog, id: string) {
  return catalog.tracks.find((t) => t.id === id) ?? catalog.tracks[0];
}
export function measureOf(catalog: Catalog, id: string) {
  return catalog.measures.find((m) => m.id === id) ?? catalog.measures[0];
}
export function kindTitle(catalog: Catalog, id: RepKind): string {
  return catalog.rep_kinds.find((k) => k.id === id)?.title ?? id;
}
export function kindLimit(catalog: Catalog, id: RepKind): number {
  return catalog.rep_kinds.find((k) => k.id === id)?.time_limit ?? 60;
}

// ---------------------------------------------------------------- frame ----

export function CoachPage({
  eyebrow,
  title,
  accessory,
  children,
}: {
  eyebrow: string;
  title: string;
  accessory?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="coach-page">
      <header className="coach-page-head">
        <div className="titles">
          <Eyebrow>{eyebrow}</Eyebrow>
          <h1>{title}</h1>
        </div>
        {accessory && <div className="accessory">{accessory}</div>}
      </header>
      {children}
    </div>
  );
}

export function Eyebrow({ children }: { children: ReactNode }) {
  return <span className="coach-eyebrow">{children}</span>;
}

export function Tag({ text, color }: { text: string; color: string }) {
  return (
    <span className="coach-tag" style={{ ["--tag" as string]: color }}>
      {text}
    </span>
  );
}

export const STATUS_COLOR: Record<HabitStatus, string> = {
  watching: "var(--c-ink-2)",
  working: "var(--c-gain)",
  notMoving: "var(--c-warm-ink)",
};

export function StatusTag({ status }: { status: HabitStatus }) {
  return <Tag text={statusTitle[status]} color={STATUS_COLOR[status]} />;
}

export function Btn({
  kind = "secondary",
  compact,
  children,
  ...rest
}: { kind?: "primary" | "secondary"; compact?: boolean; children: ReactNode } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" className={`coach-btn ${kind} ${compact ? "compact" : ""}`} {...rest}>
      {children}
    </button>
  );
}

/** The round ⓘ beside a heading: a short explanation on click. */
export function InfoButton({ title, text }: { title: string; text: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  useLayoutEffect(() => {
    if (!open || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    const left = Math.min(Math.max(12, r.left + r.width / 2 - 146), window.innerWidth - 304);
    const below = r.bottom + 8;
    setPos({ top: below + 160 > window.innerHeight ? Math.max(12, r.top - 168) : below, left });
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [open]);
  return (
    <>
      <button
        ref={ref}
        type="button"
        className="coach-info"
        aria-label={`About ${title}`}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="infoCircle" size={13} />
      </button>
      {open &&
        pos &&
        createPortal(
          <div className="coach-pop" role="dialog" aria-label={title} style={{ top: pos.top, left: pos.left }}>
            {text}
          </div>,
          document.body,
        )}
    </>
  );
}

/** A row of choices, Mac lens-picker style. */
export function Lens<T extends string | null>({
  value,
  options,
  onChange,
  small,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  small?: boolean;
  label?: string;
}) {
  return (
    <div className={`coach-lens ${small ? "small" : ""}`} role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Spinner({ small }: { small?: boolean }) {
  return <span className={`coach-spinner ${small ? "small" : ""}`} aria-hidden="true" />;
}

export function Loading({ text = "Counting your dictations…" }: { text?: string }) {
  return (
    <div className="coach-loading">
      <div>
        <Spinner />
        <p>{text}</p>
      </div>
    </div>
  );
}

/** 20 dots: met, not met, and dashed for still to come. */
export function DotRow({ hits, total, size = 12 }: { hits: boolean[]; total: number; size?: number }) {
  return (
    <div className="coach-dots" aria-label={`${hits.filter(Boolean).length} of ${hits.length} met it`} role="img">
      {Array.from({ length: total }, (_, i) => (
        <span
          key={i}
          className={`coach-dot ${i < hits.length ? (hits[i] ? "on" : "") : "empty"}`}
          style={{ width: size, height: size }}
        />
      ))}
    </div>
  );
}

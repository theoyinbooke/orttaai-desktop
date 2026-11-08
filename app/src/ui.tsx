// Orttaai design system — mirrors the macOS app (Orttaai/Design).
// Theme, icon set, and reusable primitives. All visuals come from CSS tokens in
// App.css; this file owns behavior + markup only.

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";

/* ------------------------------------------------------------------ theme -- */

export type ThemeChoice = "system" | "light" | "dark";
const THEME_KEY = "orttaai-theme";

function systemPrefersDark() {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
}
function resolve(choice: ThemeChoice): "light" | "dark" {
  return choice === "system" ? (systemPrefersDark() ? "dark" : "light") : choice;
}
export function applyTheme(choice: ThemeChoice) {
  document.documentElement.dataset.theme = resolve(choice);
}
/** The cached appearance; falls back to "system", the setting's default. */
export function storedTheme(): ThemeChoice {
  let v: string | null = null;
  try {
    v = localStorage.getItem(THEME_KEY);
  } catch {
    /* storage unavailable */
  }
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

const ThemeCtx = createContext<{
  choice: ThemeChoice;
  resolved: "light" | "dark";
  setChoice: (c: ThemeChoice) => void;
}>({ choice: "system", resolved: "dark", setChoice: () => {} });

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [choice, setChoiceState] = useState<ThemeChoice>(storedTheme);
  const [resolved, setResolved] = useState<"light" | "dark">(() => resolve(choice));

  useEffect(() => {
    const sync = (c: ThemeChoice) => {
      const r = resolve(c);
      applyTheme(c);
      setResolved(r);
      // Match the native window chrome (titlebar) to the app theme.
      getCurrentWindow()
        .setTheme(r)
        .catch(() => {});
    };
    sync(choice);
    if (choice !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => sync("system");
    mq.addEventListener?.("change", onChange);
    return () => mq.removeEventListener?.("change", onChange);
  }, [choice]);

  const setChoice = useCallback((c: ThemeChoice) => {
    localStorage.setItem(THEME_KEY, c);
    setChoiceState(c);
  }, []);

  return (
    <ThemeCtx.Provider value={{ choice, resolved, setChoice }}>{children}</ThemeCtx.Provider>
  );
}
export const useTheme = () => useContext(ThemeCtx);

/* ------------------------------------------------------------------ icons -- */

const P = (d: string) => <path d={d} />;
const ICONS: Record<string, ReactNode> = {
  // Sidebar set, drawn after the Mac's SF Symbols.
  house: P("M3.5 11.2 12 4l8.5 7.2M6 9.6V20h4.2v-5.5h3.6V20H18V9.6"),
  chatBubbles: P("M3 5.5h11.5v8H8l-3 2.6v-2.6H3v-8ZM6 8.2h5.5M6 10.8h3.5M17 9h4v8h-1.8v2.4L16.4 17H11v-1.6"),
  connected: (
    <>
      <circle cx="5.5" cy="6" r="2.2" />
      <circle cx="18.5" cy="6" r="2.2" />
      <circle cx="12" cy="18" r="2.2" />
      <path strokeDasharray="1.6 2.2" d="M7.7 6h8.6M6.6 8l4.2 7.9M17.4 8l-4.2 7.9" />
    </>
  ),
  barsAxis: P("M3.5 20.5h17M6 17.5v-6M10 17.5V5.5M14 17.5v-9M18 17.5v-4"),
  rosette: (
    <>
      <circle cx="12" cy="9" r="5.5" />
      <path d="M8.6 13.4 7.5 21l4.5-2.4 4.5 2.4-1.1-7.6" />
    </>
  ),
  scope: (
    <>
      <circle cx="12" cy="12" r="7.5" />
      <path d="M12 2.5v5M12 16.5v5M2.5 12h5M16.5 12h5" />
    </>
  ),
  book: P("M6.5 3.5H18a1 1 0 0 1 1 1V20.5H6.5A2.5 2.5 0 0 1 4 18V6a2.5 2.5 0 0 1 2.5-2.5ZM4 18a2.5 2.5 0 0 1 2.5-2.5H19M8.5 7.5h6M8.5 10.5h6"),
  cpu: (
    <>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <rect x="9.5" y="9.5" width="5" height="5" rx="0.8" />
      <path d="M9.5 2.8V6M14.5 2.8V6M9.5 18v3.2M14.5 18v3.2M2.8 9.5H6M2.8 14.5H6M18 9.5h3.2M18 14.5h3.2" />
    </>
  ),
  gear: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M10.3 2.8h3.4l.5 2.6 1.5.6 2.2-1.5 2.4 2.4-1.5 2.2.6 1.5 2.6.5v3.4l-2.6.5-.6 1.5 1.5 2.2-2.4 2.4-2.2-1.5-1.5.6-.5 2.6h-3.4l-.5-2.6-1.5-.6-2.2 1.5-2.4-2.4 1.5-2.2-.6-1.5-2.6-.5v-3.4l2.6-.5.6-1.5-1.5-2.2 2.4-2.4 2.2 1.5 1.5-.6.5-2.6Z" />
    </>
  ),
  infoCircle: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5.5M12 7.6v.2M10.2 16.8h3.6" />
    </>
  ),
  sliders: (
    <>
      <path d="M3 7h9M16 7h5M3 12h3M10 12h11M3 17h11M18 17h3" />
      <circle cx="14" cy="7" r="2" />
      <circle cx="8" cy="12" r="2" />
      <circle cx="16" cy="17" r="2" />
    </>
  ),
  sidebarLeft: (
    <>
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" />
      <path d="M9.5 4.5v15M5.5 8h1.6M5.5 11h1.6M5.5 14h1.6" />
    </>
  ),
  clockArrow: (
    <>
      <path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.4" />
      <path d="M3.5 4v4.4h4.4M12 7.5V12l3.2 2" />
    </>
  ),
  waveSearch: (
    <>
      <path d="M3 10.5v3M6.2 7.5v9M9.4 4.5v15M12.6 8v5" />
      <circle cx="16.5" cy="14.5" r="3.6" />
      <path d="m19.2 17.2 2.3 2.3" />
    </>
  ),
  pulse: P("M2.5 12.5h4.2l2.2-6 3.4 12 2.8-9 1.5 3h5.4"),
  waveform: P("M4 10.5v3M8 7v10M12 3.5v17M16 7.5v9M20 10.5v3"),
  clock: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7v5l3.4 2" />
    </>
  ),
  speedometer: (
    <>
      <path d="M4.2 18.5a9 9 0 1 1 15.6 0" />
      <path d="m12 14 3.6-4.6M12 14v0" />
      <circle cx="12" cy="14" r="1.4" />
    </>
  ),
  lock: P("M7.5 11V8a4.5 4.5 0 0 1 9 0v3M5.5 11h13v9.5h-13z"),
  cursorRays: P("M10.5 10.5 20 14l-4 1.8L14.2 20l-3.7-9.5ZM10.5 3v2.2M4.6 5.1l1.6 1.6M3 10.5h2.2M4.6 15.9l1.6-1.6M16.4 5.1l-1.6 1.6"),
  shieldLock: P("M12 3 4.5 6v5.5c0 4.5 3.2 8.2 7.5 9.5 4.3-1.3 7.5-5 7.5-9.5V6L12 3ZM9.5 11V9.5a2.5 2.5 0 0 1 5 0V11M8.8 11h6.4v4.5H8.8z"),
  textCheck: P("M4 6.5h13M4 11h9M4 15.5h6M13 16.5l2.6 2.6L21 13.7"),
  drive: (
    <>
      <rect x="3" y="7" width="18" height="10" rx="2.5" />
      <path d="M15 12h.2M18 12h.2" />
    </>
  ),
  chip: P("M6 6h12v12H6zM9.5 2.8V6M14.5 2.8V6M9.5 18v3.2M14.5 18v3.2M2.8 9.5H6M2.8 14.5H6M18 9.5h3.2M18 14.5h3.2"),
  memoryChip: P("M3 8h18v8H3zM7 8v8M12 8v8M17 8v8M6 16v3M18 16v3"),
  minimize: P("M6 12h12"),
  maximize: <rect x="6.5" y="6.5" width="11" height="11" rx="1.2" />,
  close: P("M7 7l10 10M17 7 7 17"),
  codeBrackets: P("M8.5 7 4 12l4.5 5M15.5 7 20 12l-4.5 5M13.5 5l-3 14"),
  external: P("M13.5 4H20v6.5M20 4l-9 9M9 5.5H5A1.5 1.5 0 0 0 3.5 7v12A1.5 1.5 0 0 0 5 20.5h12a1.5 1.5 0 0 0 1.5-1.5v-4"),
  home: P("M3 11.5 12 4l9 7.5M5.5 9.8V20h13V9.8M10 20v-5h4v5"),
  graph: (
    <>
      <circle cx="6" cy="6" r="2.5" />
      <circle cx="18" cy="7" r="2.5" />
      <circle cx="12" cy="18" r="2.5" />
      <path d="M8.3 7.1 15.6 6.6M7.2 8.2l3.6 7.6M16.9 9.3l-3.7 6.6" />
    </>
  ),
  trophy: P("M8 4h8v5a4 4 0 0 1-8 0V4ZM8 6H4.5a3 3 0 0 0 3.8 4.4M16 6h3.5a3 3 0 0 1-3.8 4.4M12 13v4M8.5 20h7M10 17h4v3h-4z"),
  coach: P("M12 3v3M5.6 5.6l2.1 2.1M3 12h3M18 12h3M16.3 7.7l2.1-2.1M9 21h6M10 17.5h4M12 9a4 4 0 0 0-2.5 7.1V17.5h5v-1.4A4 4 0 0 0 12 9Z"),
  restart: P("M4 12a8 8 0 1 0 2.3-5.6M4 4v4h4"),
  sparkle: P("M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3ZM19 16l.8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8L19 16Z"),
  mic: P("M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM5 11a7 7 0 0 0 14 0M12 18v3"),
  keyboard: P("M3 7h18v10H3zM6.5 10h1M10 10h1M13.5 10h1M17 10h.5M7 14h10"),
  shield: P("M12 3 4.5 6v5.5c0 4.5 3.2 8.2 7.5 9.5 4.3-1.3 7.5-5 7.5-9.5V6L12 3Z"),
  bolt: P("M13 2 4 14h7l-1 8 9-12h-7l1-8Z"),
  share: P("M12 3v12M7.5 7.5 12 3l4.5 4.5M5 13v6h14v-6"),
  edit: P("M4 20h4L19 9l-4-4L4 16v4ZM13.5 6.5l4 4"),
  pin: P("M9 4h6l-1 6 3 3H7l3-3-1-6ZM12 13v7"),
  globe: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3c2.5 2.5 3.8 5.5 3.8 9s-1.3 6.5-3.8 9c-2.5-2.5-3.8-5.5-3.8-9S9.5 5.5 12 3Z" />
    </>
  ),
  volume: P("M4 9v6h4l5 4V5L8 9H4ZM16 9.5a3.5 3.5 0 0 1 0 5M18.5 7a7 7 0 0 1 0 10"),
  calendar: P("M4 6h16v14H4zM4 10h16M8 3v4M16 3v4"),
  flame: P("M12 3c1 3.5 5 5.5 5 10a5 5 0 0 1-10 0c0-2.2 1-3.6 2.2-4.6.2 1.6 1 2.6 2.1 3C11 9.2 11 6 12 3Z"),
  target: (
    <>
      <circle cx="12" cy="12" r="9" />
      <circle cx="12" cy="12" r="5" />
      <circle cx="12" cy="12" r="1.5" />
    </>
  ),
  link: P("M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"),
  // Orttaai Link: a Mac and the devices linked to it.
  devices: P("M3 15.5V6.5A1.5 1.5 0 0 1 4.5 5h11A1.5 1.5 0 0 1 17 6.5V8M1.5 18h12M16 10.5h4.5a1 1 0 0 1 1 1v7.5a1 1 0 0 1-1 1H16a1 1 0 0 1-1-1v-7.5a1 1 0 0 1 1-1ZM17.8 17.5h1"),
  laptop: P("M4.5 6h15a1 1 0 0 1 1 1v9h-17V7a1 1 0 0 1 1-1ZM2 18.5h20"),
  pc: P("M3.5 4.5h17a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1v-10a1 1 0 0 1 1-1ZM9 20.5h6M12 16.5v4"),
  phone: P("M8 2.5h8a1.5 1.5 0 0 1 1.5 1.5v16a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 20V4A1.5 1.5 0 0 1 8 2.5ZM11 18.5h2"),
  filter: P("M4 5h16l-6 7.5V19l-4 2v-8.5L4 5Z"),
  dictate: P("M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM5 11a7 7 0 0 0 14 0M12 18v3"),
  history: (
    <>
      <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
      <path d="M3 4v4h4" />
      <path d="M12 8v4l3 2" />
    </>
  ),
  insights: (
    <>
      <path d="M4 20V10M10 20V4M16 20v-7M22 20H2" />
    </>
  ),
  dictionary: (
    <>
      <path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3V4Z" />
      <path d="M5 17a3 3 0 0 1 3-3h11" />
      <path d="M9 8h6M9 11h4" />
    </>
  ),
  models: (
    <>
      <path d="M12 2 3 7v10l9 5 9-5V7l-9-5Z" />
      <path d="M3 7l9 5 9-5M12 12v10" />
    </>
  ),
  assistant: (
    <>
      <path d="M12 3a7 7 0 0 1 7 7c0 4-3 6-3 8H8c0-2-3-4-3-8a7 7 0 0 1 7-7Z" />
      <path d="M9 21h6" />
    </>
  ),
  settings: (
    <>
      <path d="M20 7h-9M14 17H5" />
      <circle cx="17" cy="17" r="3" />
      <circle cx="7" cy="7" r="3" />
    </>
  ),
  play: P("M7 4v16l13-8L7 4Z"),
  stop: <rect x="6" y="6" width="12" height="12" rx="2" />,
  download: P("M12 3v12m0 0 4-4m-4 4-4-4M5 21h14"),
  check: P("M5 13l4 4L19 7"),
  trash: (
    <>
      <path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2" />
      <path d="M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13" />
    </>
  ),
  x: P("M6 6l12 12M18 6 6 18"),
  chevronDown: P("M6 9l6 6 6-6"),
  chevronLeft: P("M15 6l-6 6 6 6"),
  chevronRight: P("M9 6l6 6-6 6"),
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M19 5l-1.5 1.5M6.5 17.5 5 19" />
    </>
  ),
  moon: P("M21 12.8A8 8 0 1 1 11.2 3a6 6 0 0 0 9.8 9.8Z"),
  monitor: (
    <>
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M8 20h8M12 16v4" />
    </>
  ),
  copy: (
    <>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V5a2 2 0 0 1 2-2h10" />
    </>
  ),
  plus: P("M12 5v14M5 12h14"),
  arrowRight: P("M5 12h14M13 6l6 6-6 6"),
  refresh: P("M21 12a9 9 0 1 1-2.6-6.4M21 4v5h-5"),
  alert: (
    <>
      <path d="M12 9v4M12 17h.01" />
      <path d="M10.3 3.9 2 18a2 2 0 0 0 1.7 3h16.6a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5M12 8h.01" />
    </>
  ),
  send: P("M22 2 11 13M22 2l-7 20-4-9-9-4 20-7Z"),
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
    </>
  ),
};

export function Icon({ name, size = 20, className }: { name: string; size?: number; className?: string }) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {ICONS[name] ?? null}
    </svg>
  );
}

/* ------------------------------------------------------------- primitives -- */

type ButtonVariant = "primary" | "danger" | "ghost" | "subtle";
export function Button({
  variant = "ghost",
  size = "md",
  icon,
  loading,
  children,
  className = "",
  ...rest
}: {
  variant?: ButtonVariant;
  size?: "sm" | "md";
  icon?: string;
  loading?: boolean;
  children?: ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={`btn btn-${variant} btn-${size} ${className}`}
      disabled={loading || rest.disabled}
      {...rest}
    >
      {loading ? <Spinner /> : icon ? <Icon name={icon} size={size === "sm" ? 15 : 17} /> : null}
      {children && <span>{children}</span>}
    </button>
  );
}

export function IconButton({
  name,
  label,
  className = "",
  ...rest
}: { name: string; label: string } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button className={`icon-btn ${className}`} aria-label={label} title={label} {...rest}>
      <Icon name={name} size={18} />
    </button>
  );
}

export function CopyButton({
  text,
  label = "Copy",
  compact = false,
  className = "",
}: {
  text: string;
  label?: string;
  compact?: boolean;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number>(0);
  const onClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    navigator.clipboard?.writeText(text);
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 1400);
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (
    <button
      className={`copy-btn ${copied ? "copied" : ""} ${compact ? "compact" : ""} ${className}`}
      onClick={onClick}
      aria-label={copied ? "Copied" : label}
      title={label}
    >
      <span className="copy-ico">
        <Icon name={copied ? "check" : "copy"} size={15} />
      </span>
      {!compact && <span className="copy-label">{copied ? "Copied" : label}</span>}
    </button>
  );
}

export function Spinner({ size = 15 }: { size?: number }) {
  return <span className="spinner" style={{ width: size, height: size }} aria-hidden="true" />;
}

export function Card({ children, className = "", ...rest }: { children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={`card ${className}`} {...rest}>
      {children}
    </div>
  );
}

/** The window chrome the page header needs: the sidebar toggle. Empty in a
 * window without a sidebar. */
export const ShellContext = createContext<{ toggleSidebar?: () => void }>({});

/**
 * The page's title strip (Mac `workspaceHeader`): sidebar toggle, title, optional
 * controls beside it, actions at the trailing edge. It doubles as the window's
 * drag region and stays put while the page scrolls.
 */
export function PageHeader({
  title,
  leading,
  actions,
}: {
  title: string;
  leading?: ReactNode;
  actions?: ReactNode;
}) {
  const { toggleSidebar } = useContext(ShellContext);
  return (
    <header className="page-head" data-tauri-drag-region>
      {toggleSidebar && (
        <button
          className="page-head-toggle"
          onClick={toggleSidebar}
          title="Show or hide the sidebar"
          aria-label="Toggle sidebar"
        >
          <Icon name="sidebarLeft" size={17} />
        </button>
      )}
      <h1 data-tauri-drag-region>{title}</h1>
      {leading}
      <div className="page-head-fill" data-tauri-drag-region />
      {actions && <div className="page-head-actions">{actions}</div>}
    </header>
  );
}

export function Badge({ children, tone = "neutral" }: { children: ReactNode; tone?: string }) {
  return <span className={`badge badge-${tone}`}>{children}</span>;
}

export function Kbd({ combo }: { combo: string }) {
  const keys = combo.split("+").map((k) => k.trim()).filter(Boolean);
  return (
    <span className="kbd-combo">
      {keys.map((k, i) => (
        <kbd key={i}>{k}</kbd>
      ))}
    </span>
  );
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="field-hint">{hint}</span>}
    </label>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: ReactNode;
  hint?: ReactNode;
}) {
  return (
    <label className="toggle-row">
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        className={`switch ${checked ? "on" : ""}`}
        onClick={() => onChange(!checked)}
      >
        <span className="switch-knob" />
      </button>
      <span className="toggle-text">
        <span className="toggle-label">{label}</span>
        {hint && <span className="field-hint">{hint}</span>}
      </span>
    </label>
  );
}

/** A bare on/off switch (no label) for settings rows. */
export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={`switch ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="switch-knob" />
    </button>
  );
}

/** Segmented control (Mac-style picker). */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className="segmented" role="radiogroup">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          disabled={disabled}
          className={`seg-btn ${value === o.value ? "active" : ""}`}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Tab strip for in-page sub-sections (Mac picker tabs). */
export function Tabs<T extends string>({
  value,
  tabs,
  onChange,
}: {
  value: T;
  tabs: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((t) => (
        <button
          key={t.value}
          role="tab"
          aria-selected={value === t.value}
          className={`tab ${value === t.value ? "active" : ""}`}
          onClick={() => onChange(t.value)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

/* --------------------------------------------------------------- select ---- */

export function Select<T extends string>({
  value,
  onChange,
  options,
  placeholder = "Select…",
  ariaLabel,
  className = "",
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  placeholder?: string;
  ariaLabel?: string;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [rect, setRect] = useState<{ top: number; left: number; width: number } | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLUListElement>(null);
  const selected = options.find((o) => o.value === value);

  // The menu is portaled to <body>, so no ancestor card's `overflow`/stacking
  // context can clip it or paint over it. Anchor it under the trigger using the
  // trigger's viewport rect.
  const place = () => {
    const r = ref.current?.getBoundingClientRect();
    if (r) setRect({ top: r.bottom + 6, left: r.left, width: r.width });
  };

  useEffect(() => {
    if (!open) return;
    place();
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node;
      if (ref.current?.contains(t) || menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    // A scroll/resize would detach the fixed menu from its trigger — just close.
    const onShift = () => setOpen(false);
    document.addEventListener("mousedown", onDoc);
    window.addEventListener("resize", onShift);
    window.addEventListener("scroll", onShift, true);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      window.removeEventListener("resize", onShift);
      window.removeEventListener("scroll", onShift, true);
    };
  }, [open]);

  useEffect(() => {
    if (open) setActive(Math.max(0, options.findIndex((o) => o.value === value)));
  }, [open, options, value]);

  const pick = (i: number) => {
    const o = options[i];
    if (o) {
      onChange(o.value);
      setOpen(false);
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || (e.key === "Enter" && !open)) {
      e.preventDefault();
      if (!open) setOpen(true);
      else setActive((a) => Math.min(options.length - 1, a + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === "Enter" && open) {
      e.preventDefault();
      pick(active);
    } else if (e.key === "Escape") {
      setOpen(false);
    }
  };

  return (
    <div className={`select ${className} ${open ? "open" : ""}`} ref={ref}>
      <button
        type="button"
        className="select-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={onKey}
      >
        <span className={selected ? "" : "select-placeholder"}>
          {selected ? selected.label : placeholder}
        </span>
        <Icon name="chevronDown" size={16} className="select-caret" />
      </button>
      {open &&
        rect &&
        createPortal(
          <ul
            className="select-menu"
            role="listbox"
            ref={menuRef}
            style={{ top: rect.top, left: rect.left, width: rect.width }}
          >
            {options.length === 0 && <li className="select-empty">No options</li>}
            {options.map((o, i) => (
              <li
                key={o.value}
                role="option"
                aria-selected={o.value === value}
                className={`select-option ${i === active ? "active" : ""} ${
                  o.value === value ? "selected" : ""
                }`}
                onMouseEnter={() => setActive(i)}
                onClick={() => pick(i)}
              >
                <span>{o.label}</span>
                {o.value === value && <Icon name="check" size={15} />}
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </div>
  );
}

/* ---------------------------------------------------------------- modal ---- */

export function Modal({
  open,
  onClose,
  children,
  labelledBy,
}: {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  labelledBy?: string;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return createPortal(
    <div className="modal-overlay" onMouseDown={onClose}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}

export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  tone = "danger",
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  body: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "danger" | "primary";
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <Modal open={open} onClose={onCancel} labelledBy="confirm-title">
      <div className="confirm">
        <div className={`confirm-icon confirm-${tone}`}>
          <Icon name={tone === "danger" ? "alert" : "info"} size={22} />
        </div>
        <h2 id="confirm-title">{title}</h2>
        <div className="confirm-body">{body}</div>
        <div className="confirm-actions">
          <Button variant="ghost" onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button variant={tone} onClick={onConfirm} autoFocus>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

/* ------------------------------------------------------------ empty state -- */

export function EmptyState({
  icon = "info",
  title,
  desc,
  action,
}: {
  icon?: string;
  title: string;
  desc?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">
        <Icon name={icon} size={26} />
      </div>
      <h3>{title}</h3>
      {desc && <p>{desc}</p>}
      {action && <div className="empty-action">{action}</div>}
    </div>
  );
}

/* ---------------------------------------------------------------- meter ---- */

export function LevelMeter({ level, active }: { level: number; active: boolean }) {
  const segments = 28;
  const lit = Math.round(Math.min(1, Math.sqrt(Math.max(0, level)) * 1.1) * segments);
  return (
    <div className={`meter ${active ? "live" : ""}`} role="meter" aria-label="Microphone input level">
      {Array.from({ length: segments }, (_, i) => (
        <span
          key={i}
          className={`seg ${i < lit ? "on" : ""} ${i > segments * 0.8 ? "hot" : ""}`}
        />
      ))}
    </div>
  );
}

/* ---------------------------------------------------------------- toast ---- */

type Tone = "error" | "warn" | "success" | "info";
type ToastItem = { id: number; tone: Tone; message: string };
const ToastCtx = createContext<(message: string, tone?: Tone) => void>(() => {});
export const useToast = () => useContext(ToastCtx);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const idRef = useRef(0);

  const push = useCallback((message: string, tone: Tone = "info") => {
    const id = ++idRef.current;
    setItems((xs) => [...xs, { id, tone, message }]);
    const ttl = tone === "error" ? 9000 : 6000;
    window.setTimeout(() => setItems((xs) => xs.filter((x) => x.id !== id)), ttl);
  }, []);

  const dismiss = (id: number) => setItems((xs) => xs.filter((x) => x.id !== id));
  const iconFor: Record<Tone, string> = {
    error: "alert",
    warn: "alert",
    success: "check",
    info: "info",
  };

  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toast-viewport">
        {items.map((t) => (
          <div key={t.id} className={`toast toast-${t.tone}`} role="status">
            <Icon name={iconFor[t.tone]} size={18} className="toast-icon" />
            <span className="toast-msg">{t.message}</span>
            <button className="toast-x" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              <Icon name="x" size={15} />
            </button>
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

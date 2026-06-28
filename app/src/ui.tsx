// Orttaai design system — "Signal / studio console".
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
export function storedTheme(): ThemeChoice {
  const v = localStorage.getItem(THEME_KEY) as ThemeChoice | null;
  return v === "light" || v === "dark" || v === "system" ? v : "dark";
}

const ThemeCtx = createContext<{
  choice: ThemeChoice;
  resolved: "light" | "dark";
  setChoice: (c: ThemeChoice) => void;
}>({ choice: "dark", resolved: "dark", setChoice: () => {} });

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

export function PageHeader({
  title,
  desc,
  actions,
}: {
  title: string;
  desc?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="page-head">
      <div className="page-head-text">
        <h1>{title}</h1>
        {desc && <p>{desc}</p>}
      </div>
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
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
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
      {open && (
        <ul className="select-menu" role="listbox">
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
        </ul>
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

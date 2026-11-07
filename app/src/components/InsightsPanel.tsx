// The Overview's Insights panel (Mac HomeInsightsPanel + InsightStatTicker): a
// ticker of the numbers behind the insight, the summary, a few patterns and
// takeaways, and options folded away, sized to read on one screen. Reads the
// writing insights service, as on the Mac; semantic memory isn't involved.
import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import "../styles/semantic.css";
import { Button, Icon, Select, Spinner } from "../ui";
import { call, useEvent } from "../lib/api";
import type { DashboardPayload } from "../lib/analytics";
import { GLYPHS } from "../lib/semantic";
import {
  DEFAULT_INSIGHTS_REQUEST,
  dateTime,
  relativeTime,
  writing,
  type AppFilterMode,
  type GenerationMode,
  type InsightFreshness,
  type InsightHistoryItem,
  type InsightsComparison,
  type TimeRange,
  type WritingInsightSnapshot,
  type WritingInsightsRequest,
} from "../lib/ai";

/** One number (or name) the ticker cycles through. */
type InsightStat = { id: string; icon: string; number: number | null; text: string | null; label: string };

const TIME_RANGES: { value: TimeRange; label: string }[] = [
  { value: "days7", label: "7d" },
  { value: "days14", label: "14d" },
  { value: "days30", label: "30d" },
  { value: "days90", label: "90d" },
  { value: "all", label: "All" },
];
const MODES: { value: GenerationMode; label: string }[] = [
  { value: "balanced", label: "Balanced" },
  { value: "deep", label: "Deep" },
];
const APP_FILTERS: { value: AppFilterMode; label: string }[] = [
  { value: "all_apps", label: "All apps" },
  { value: "include_only", label: "Include" },
  { value: "exclude", label: "Exclude" },
];
const titleOf = <T extends string>(options: { value: T; label: string }[], value: T) =>
  options.find((o) => o.value === value)?.label ?? value;
const optionsLine = (r: WritingInsightsRequest) =>
  `${titleOf(TIME_RANGES, r.time_range)} · ${titleOf(MODES, r.generation_mode)} · ${titleOf(APP_FILTERS, r.app_filter_mode)}`;

/** SF Symbol stand-ins the shared glyph sets don't have. */
const PANEL_GLYPHS: Record<string, string> = {
  wordSpacing: "M4 7h6M14 7h6M4 12h16M4 17h9M16 17h4",
  window: "M3 5h18v14H3zM3 9h18M6 7h.01M8.5 7h.01",
  number: "M9 4 7 20M17 4l-2 16M4 9h16M3 15h16",
  textQuote: "M4 5v14M8 6h12M8 12h12M8 18h9",
  docSearch: "M13 3H6v18h6M13 3l5 5v4M13 3v5h5M16.5 21a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7ZM21.5 22.5 19 20",
  plusCircle: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM12 8v8M8 12h8",
  ellipsisCircle: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM7.8 12h.01M12 12h.01M16.2 12h.01",
  filterCircle: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM7.5 9.5h9M9.5 12.5h5M11 15.5h2",
};

function Glyph({ name, size = 16, weight = 1.75 }: { name: string; size?: number; weight?: number }) {
  const d = PANEL_GLYPHS[name] ?? GLYPHS[name];
  if (!d) return <Icon name={name} size={size} />;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={weight} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

/** The two filled takeaway marks (checkmark.circle.fill, arrow.up.right.circle.fill). */
function FilledMark({ kind }: { kind: "keep" | "next" }) {
  return (
    <svg width={12} height={12} viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="11" fill="currentColor" />
      <path
        d={kind === "keep" ? "M7 12.5l3.4 3.4L17 8.8" : "M8.2 15.8 15.8 8.2M9.6 8.2h6.2v6.2"}
        fill="none"
        style={{ stroke: "var(--surface)" }}
        strokeWidth={2.6}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

const HOLD_MS = 3200;
const prefersReducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/** Rolls a number up from zero. */
function useRollUp(target: number, key: string) {
  const [shown, setShown] = useState(target);
  useEffect(() => {
    if (prefersReducedMotion()) {
      setShown(target);
      return;
    }
    let frame = 0;
    const start = performance.now();
    const tick = (t: number) => {
      const p = Math.min(1, (t - start) / 900);
      setShown(target * (1 - Math.pow(1 - p, 3)));
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    setShown(0);
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [target, key]);
  return shown;
}

/**
 * One statistic at a time: a scanning ring, the number rolling up, a thin bar
 * draining while it's shown, then the next. Holds on the first under Reduce Motion.
 */
function InsightStatTicker({ stats, scanning }: { stats: InsightStat[]; scanning: boolean }) {
  const [index, setIndex] = useState(0);
  const gradient = useId();
  const signature = stats.map((s) => s.id).join("|");
  useEffect(() => {
    setIndex(0);
    if (stats.length < 2 || prefersReducedMotion()) return;
    const t = window.setInterval(() => setIndex((i) => (i + 1) % stats.length), HOLD_MS);
    return () => window.clearInterval(t);
  }, [signature]); // eslint-disable-line react-hooks/exhaustive-deps
  const current = stats[index % stats.length];
  const value = useRollUp(current?.number ?? 0, `${current?.id ?? ""}-${index}`);
  if (!current) return null;
  const label = stats.map((s) => `${s.text ?? (s.number !== null ? Math.round(s.number).toLocaleString() : "")} ${s.label}`).join(", ");
  return (
    <div className={`ip-ticker ${scanning ? "scanning" : ""}`} role="group" aria-label={label}>
      <div className="ip-ring" aria-hidden="true">
        <svg className="ip-ring-art" viewBox="0 0 46 46">
          <defs>
            <linearGradient id={gradient} gradientUnits="userSpaceOnUse" x1="46" y1="23" x2="18.7" y2="45.6">
              <stop offset="0" style={{ stopColor: "var(--accent)", stopOpacity: 0 }} />
              <stop offset="1" style={{ stopColor: "var(--accent)" }} />
            </linearGradient>
          </defs>
          <circle cx="23" cy="23" r="23" fill="none" style={{ stroke: "var(--accent)", strokeOpacity: 0.18 }} strokeWidth="2" />
          <g className="ip-sweep">
            <path d="M46 23 A23 23 0 0 1 18.7 45.6" fill="none" stroke={`url(#${gradient})`} strokeWidth="2.5" strokeLinecap="round" />
          </g>
        </svg>
        <span key={`blip-${index}`} className="ip-blip" />
        <Glyph name={current.icon} size={15} weight={2} />
      </div>
      <div className="ip-ticker-text">
        <div className="ip-value" key={`v-${current.id}`}>
          {current.text ? <span className="ip-swap">{current.text}</span> : Math.round(value).toLocaleString()}
        </div>
        <div className="ip-label ip-swap" key={`l-${current.id}-${index}`}>
          {current.label}
        </div>
        <div className="ip-drain">
          <span key={`d-${index}`} />
        </div>
      </div>
      <div className="ip-dots" aria-hidden="true">
        {stats.map((s, i) => (
          <span key={s.id} className={i === index % stats.length ? "on" : ""} />
        ))}
      </div>
    </div>
  );
}

function progressLine(elapsed: number, sampleCount: number | null) {
  if (elapsed < 3) return sampleCount != null ? `Reading your last ${sampleCount.toLocaleString()} dictations` : "Reading your recent dictations";
  if (elapsed < 10) return "Finding patterns in how you write";
  if (elapsed < 25) return "Writing your summary";
  if (elapsed < 60) return "Still working. Larger models take a little longer";
  return "Almost there";
}

/** What the analysis is doing, moving on as the wait grows. */
function ProgressRow({ startedAt, sampleCount }: { startedAt: number; sampleCount: number | null }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  const line = progressLine((now - startedAt) / 1000, sampleCount);
  return (
    <div className="ip-progress" role="status">
      <Spinner size={13} />
      <span className="ip-progress-line" key={line}>
        {line}
      </span>
    </div>
  );
}

/** Closes on an outside click; for the panel's small menus. */
function useDismiss(open: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open, close]);
  return ref;
}

/** A folding section whose whole header row opens and closes it. */
function CollapsibleRow({ title, detail, children }: { title: string; detail: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="ip-collapsible">
      <button className="ip-collapsible-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className={`ip-chevron ${open ? "open" : ""}`}>
          <Glyph name="chevronRight" size={10} />
        </span>
        <span className="ip-collapsible-title">{title}</span>
        <span className="ip-collapsible-detail">{detail}</span>
      </button>
      {open && children}
    </div>
  );
}

function PickerColumn({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="ip-picker">
      <div className="ip-picker-title">{title}</div>
      {children}
    </div>
  );
}

function AboutRow({ icon, tint, children }: { icon: string; tint?: "warn" | "ok"; children: ReactNode }) {
  return (
    <div className={`ip-about-row ${tint ?? ""}`}>
      <Glyph name={icon} size={12} weight={2} />
      <span>{children}</span>
    </div>
  );
}

function HistoryRow({
  item,
  selected,
  compareIndex,
  onLoad,
  onPin,
  onCompare,
  onDelete,
}: {
  item: InsightHistoryItem;
  selected: boolean;
  compareIndex: number;
  onLoad: () => void;
  onPin: () => void;
  onCompare: () => void;
  onDelete: () => void;
}) {
  const [menu, setMenu] = useState(false);
  const close = useCallback(() => setMenu(false), []);
  const ref = useDismiss(menu, close);
  const pick = (action: () => void) => () => {
    setMenu(false);
    action();
  };
  return (
    <div className={`ip-run ${selected ? "selected" : ""}`}>
      <div className="ip-run-text">
        <div className="ip-run-when">
          <span>{dateTime(item.snapshot.generated_at)}</span>
          {item.is_pinned && <Icon name="pin" size={10} className="ip-run-pin" />}
          {compareIndex >= 0 && <span className="ip-compare-pill">{compareIndex === 0 ? "A" : "B"}</span>}
        </div>
        <div className="ip-run-meta">
          {item.snapshot.sample_count} sessions • {item.snapshot.analyzer_name}
        </div>
      </div>
      <Button onClick={onLoad} disabled={selected}>
        {selected ? "Loaded" : "Load"}
      </Button>
      <div className="ip-menu-wrap" ref={ref}>
        <button className="ip-run-more" aria-label="More" aria-expanded={menu} onClick={() => setMenu((m) => !m)}>
          <Glyph name="ellipsisCircle" size={15} weight={2} />
        </button>
        {menu && (
          <div className="ip-menu" role="menu">
            <button role="menuitem" onClick={pick(onPin)}>
              {item.is_pinned ? "Unpin" : "Pin"}
            </button>
            <button role="menuitem" onClick={pick(onCompare)}>
              {compareIndex < 0 ? "Add to Compare" : "Remove from Compare"}
            </button>
            <button role="menuitem" className="danger" onClick={pick(onDelete)}>
              Delete
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function AppsMenu({
  apps,
  selected,
  onToggle,
  onClear,
}: {
  apps: string[];
  selected: string[];
  onToggle: (app: string) => void;
  onClear: () => void;
}) {
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useDismiss(open, close);
  return (
    <div className="ip-apps">
      <div className="ip-apps-row">
        <div className="ip-menu-wrap" ref={ref}>
          <Button aria-expanded={open} onClick={() => setOpen((o) => !o)}>
            <span className="ip-inline-label">
              <Glyph name="filterCircle" size={13} />
              {selected.length === 0 ? "Select apps" : `Apps (${selected.length})`}
            </span>
          </Button>
          {open && (
            <div className="ip-menu left" role="menu">
              {apps.length === 0 ? (
                <div className="ip-menu-empty">No recent apps</div>
              ) : (
                apps.map((app) => (
                  <button key={app} role="menuitemcheckbox" aria-checked={selected.includes(app)} onClick={() => onToggle(app)}>
                    <span className="ip-menu-check">{selected.includes(app) && <Icon name="check" size={12} />}</span>
                    {app}
                  </button>
                ))
              )}
            </div>
          )}
        </div>
        {selected.length > 0 && (
          <button className="btn btn-ghost btn-md ip-destructive" onClick={onClear}>
            <span>Clear</span>
          </button>
        )}
      </div>
      {selected.length > 0 && <div className="ip-apps-list">{selected.join(", ")}</div>}
    </div>
  );
}

const capitalize = (s: string) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

/** What the ticker cycles through, counted over the last 30 days. */
function statsFrom(payload: DashboardPayload | null): InsightStat[] {
  if (!payload) return [];
  const words = payload.trend30d.reduce((n, p) => n + p.words, 0);
  const sessions = payload.trend30d.reduce((n, p) => n + p.sessions, 0);
  const activeDays = payload.trend30d.filter((p) => p.sessions > 0).length;
  const wpm = payload.header.average_wpm;
  const stats: InsightStat[] = [];
  const add = (id: string, icon: string, number: number, label: string) => stats.push({ id, icon, number, text: null, label });
  if (sessions > 0) add("sessions", "waveform", sessions, sessions === 1 ? "dictation in 30 days" : "dictations in 30 days");
  if (words > 0) add("words", "wordSpacing", words, "words in 30 days");
  if (wpm > 0) add("wpm", "speedometer", wpm, "words per minute");
  if (activeDays > 0) add("days", "calendar", activeDays, activeDays === 1 ? "active day" : "active days");
  const app = payload.top_apps30d[0];
  if (app) {
    stats.push({ id: "app", icon: "window", number: null, text: app.name, label: `where you dictate most · ${Math.round(app.session_share * 100)}%` });
  }
  return stats;
}

/** The counted statistics, plus any number the model counted itself. */
function tickerStats(stats: InsightStat[], snapshot: WritingInsightSnapshot | null): InsightStat[] {
  const fromModel = (snapshot?.signals ?? []).flatMap((signal): InsightStat[] => {
    const label = signal.label.trim();
    if (!label || stats.some((s) => s.label.toLowerCase().includes(label.toLowerCase()))) return [];
    const cleaned = signal.value.replace(/,/g, "");
    const number = cleaned.trim() === "" ? NaN : Number(cleaned);
    return [
      Number.isFinite(number)
        ? { id: `signal-${label}`, icon: "number", number, text: null, label: label.toLowerCase() }
        : { id: `signal-${label}`, icon: "number", number: null, text: signal.value, label: label.toLowerCase() },
    ];
  });
  return [...stats, ...fromModel].slice(0, 6);
}

export default function InsightsPanel({ onClose }: { onClose?: () => void }) {
  const [payload, setPayload] = useState<DashboardPayload | null>(null);
  const [request, setRequest] = useState<WritingInsightsRequest>(DEFAULT_INSIGHTS_REQUEST);
  const [snapshot, setSnapshot] = useState<WritingInsightSnapshot | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [preliminary, setPreliminary] = useState(false);
  const [history, setHistory] = useState<InsightHistoryItem[]>([]);
  const [freshness, setFreshness] = useState<InsightFreshness | null>(null);
  const [apps, setApps] = useState<string[]>([]);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [about, setAbout] = useState(false);
  const [compareIds, setCompareIds] = useState<number[]>([]);
  const [comparison, setComparison] = useState<InsightsComparison | null>(null);
  const closeAbout = useCallback(() => setAbout(false), []);
  const aboutRef = useDismiss(about, closeAbout);
  const generating = startedAt !== null;

  // The running pass reads these without re-subscribing every effect below.
  const busy = useRef(false);
  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;
  const preliminaryRef = useRef(preliminary);
  preliminaryRef.current = preliminary;

  const loadSummary = useCallback(() => {
    call<DashboardPayload>("home_summary").then(setPayload).catch(() => {});
  }, []);
  const loadHistory = useCallback(() => writing.history(24).then((items) => (setHistory(items), items)), []);

  const generate = useCallback(
    async (req: WritingInsightsRequest) => {
      if (busy.current) return;
      busy.current = true;
      setStartedAt(Date.now());
      setError(null);
      setStatus(null);
      try {
        // Something useful at once: the quick look fills an empty panel while
        // the AI analysis runs.
        if (!snapshotRef.current) {
          const quick = await writing.quick(req).catch(() => null);
          if (quick) {
            setSnapshot(quick);
            setPreliminary(true);
          }
        }
        const run = await writing.generate(req);
        if (run.error) {
          if (preliminaryRef.current) setStatus("The AI analysis didn't finish, so this is the quick look. Try Regenerate.");
          else setError(run.error);
          return;
        }
        if (!run.snapshot) {
          setStatus("No history yet to analyze.");
          return;
        }
        setSnapshot(run.snapshot);
        setPreliminary(false);
        setSelectedId(run.snapshot_id);
        loadHistory().catch(() => {});
        writing.freshness(run.snapshot.generated_at).then(setFreshness).catch(() => {});
        const fallback = run.snapshot.used_fallback ? " (fallback used)" : "";
        const line = `Generated from ${run.snapshot.sample_count} sessions using ${run.snapshot.analyzer_name}${fallback}.`;
        setStatus(run.persistence_warning ? `${line} ${run.persistence_warning}` : line);
      } catch (e) {
        setError(String(e));
      } finally {
        busy.current = false;
        setStartedAt(null);
      }
    },
    [loadHistory],
  );

  useEffect(() => {
    loadSummary();
    writing.apps().then(setApps).catch(() => {});
    loadHistory()
      .then(async (items) => {
        const first = items[0];
        if (!first) {
          generate(DEFAULT_INSIGHTS_REQUEST);
          return;
        }
        setSnapshot(first.snapshot);
        setRequest(first.snapshot.request);
        setSelectedId(first.id);
        const fresh = await writing.freshness(first.snapshot.generated_at);
        setFreshness(fresh);
        if (fresh.should_auto_refresh) {
          setStatus(`Found ${fresh.new_session_count} new sessions since this insight. Regenerating...`);
          generate(first.snapshot.request);
        }
      })
      .catch((e) => setError(String(e)));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEvent<unknown>("history-changed", () => {
    loadSummary();
    const current = snapshotRef.current;
    if (current) writing.freshness(current.generated_at).then(setFreshness).catch(() => {});
  });

  useEffect(() => {
    if (compareIds.length !== 2) {
      setComparison(null);
      return;
    }
    writing.compare(compareIds[0], compareIds[1]).then(setComparison).catch(() => setComparison(null));
  }, [compareIds]);

  const changeRequest = (next: Partial<WritingInsightsRequest>, message: string) => {
    setRequest((r) => ({ ...r, ...next }));
    setStatus(message);
  };

  const loadItem = (item: InsightHistoryItem) => {
    setSnapshot(item.snapshot);
    setPreliminary(false);
    setRequest(item.snapshot.request);
    setSelectedId(item.id);
    setStatus(`Loaded snapshot from ${dateTime(item.snapshot.generated_at)}.`);
    setError(null);
    writing.freshness(item.snapshot.generated_at).then(setFreshness).catch(() => {});
  };

  const togglePin = (item: InsightHistoryItem) =>
    writing
      .pin(item.id, !item.is_pinned)
      .then(() => {
        setStatus(item.is_pinned ? "Removed pin." : "Pinned to top.");
        setError(null);
        return loadHistory();
      })
      .catch((e) => setError(String(e)));

  const toggleCompare = (item: InsightHistoryItem) => {
    if (compareIds.includes(item.id)) {
      setCompareIds(compareIds.filter((id) => id !== item.id));
      setStatus("Removed from compare.");
      return;
    }
    const next = [...compareIds.slice(-1), item.id];
    setCompareIds(next);
    setStatus(next.length === 2 ? "Comparison updated." : "Select one more snapshot to compare.");
  };

  const clearComparison = () => {
    setCompareIds([]);
    setStatus("Comparison cleared.");
  };

  const deleteItem = (item: InsightHistoryItem) =>
    writing
      .remove(item.id)
      .then(() => loadHistory())
      .then((items) => {
        setCompareIds((ids) => ids.filter((id) => id !== item.id));
        setError(null);
        if (selectedId !== item.id) {
          setStatus("Deleted snapshot.");
          return;
        }
        if (items[0]) {
          loadItem(items[0]);
          return;
        }
        setSnapshot(null);
        setSelectedId(null);
        setFreshness(null);
        setStatus("Deleted snapshot.");
      })
      .catch((e) => setError(String(e)));

  const toggleApp = (app: string) => {
    const set = new Set(request.selected_apps);
    if (set.has(app)) set.delete(app);
    else set.add(app);
    const selected_apps = [...set].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
    changeRequest(
      {
        selected_apps,
        app_filter_mode: request.app_filter_mode === "all_apps" && selected_apps.length > 0 ? "include_only" : request.app_filter_mode,
      },
      "App selection updated. Click Regenerate to refresh.",
    );
  };

  const newCount = freshness?.new_session_count ?? 0;
  const stats = tickerStats(statsFrom(payload), snapshot);
  const appOptions = [...new Set([...apps, ...request.selected_apps])].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));

  return (
    <section className="ip-panel" aria-label="Writing insights panel">
      <header className="ip-head">
        <div className="ip-head-text">
          <div className="ip-title">
            Insights
            {snapshot && (
              <div className="ip-menu-wrap" ref={aboutRef}>
                <button
                  className="ip-about-btn"
                  title="About this insight"
                  aria-label="About this insight"
                  aria-expanded={about}
                  onClick={() => setAbout((a) => !a)}
                >
                  <Icon name="infoCircle" size={12} />
                </button>
                {about && (
                  <div className="ip-about" role="dialog" aria-label="About this insight">
                    <h4>About this insight</h4>
                    <AboutRow icon="cpu">Analysed by {snapshot.analyzer_name}</AboutRow>
                    <AboutRow icon="clock">
                      {preliminary
                        ? "Quick look now; the AI analysis is running"
                        : `Ran ${relativeTime(snapshot.generated_at)}, ${dateTime(snapshot.generated_at)}`}
                    </AboutRow>
                    <AboutRow icon="docSearch">
                      Read {snapshot.sample_count === 1 ? "1 dictation" : `${snapshot.sample_count.toLocaleString()} dictations`}
                    </AboutRow>
                    <AboutRow icon="sliders">{optionsLine(request)}</AboutRow>
                    {freshness &&
                      !preliminary &&
                      (newCount > 0 ? (
                        <AboutRow icon="plusCircle" tint="warn">
                          {newCount} new {newCount === 1 ? "dictation" : "dictations"} since. Regenerate to include them.
                        </AboutRow>
                      ) : (
                        <AboutRow icon="checkCircle" tint="ok">
                          Up to date with your dictation
                        </AboutRow>
                      ))}
                  </div>
                )}
              </div>
            )}
          </div>
          {snapshot && (
            <div className="ip-model">
              <span>{snapshot.analyzer_name}</span>
              {newCount > 0 && !preliminary && <span className="ip-stale-dot" aria-label="Newer dictation since this ran" />}
            </div>
          )}
        </div>
        <Button
          size="sm"
          icon="refresh"
          className="ip-regen"
          disabled={generating}
          onClick={() => generate(request)}
          title="Analyze your dictation history again"
        >
          {snapshot ? "Regenerate" : "Generate"}
        </Button>
        {onClose && (
          <button className="ip-close" title="Close insights panel" aria-label="Close insights panel" onClick={onClose}>
            <Glyph name="x" size={12} />
          </button>
        )}
      </header>

      <div className="ip-body">
        {stats.length > 0 && <InsightStatTicker stats={stats} scanning={generating} />}
        {startedAt !== null && <ProgressRow startedAt={startedAt} sampleCount={snapshot?.sample_count ?? null} />}
        {error && <div className="ip-error">{error}</div>}

        {snapshot ? (
          <>
            <div className="ip-card">
              <div className="ip-card-label accent">
                <Glyph name="textQuote" size={12} /> Summary
              </div>
              <div className="ip-summary">{snapshot.summary}</div>
            </div>
            {snapshot.patterns.length > 0 && (
              <div className="ip-card">
                <div className="ip-card-label">
                  <Icon name="connected" size={12} /> Patterns
                </div>
                {snapshot.patterns.slice(0, 3).map((p) => (
                  <div className="ip-pattern" key={p.title} title={p.evidence ? `${p.detail}\n\n${p.evidence}` : p.detail}>
                    <span className="ip-pattern-dot" />
                    <div>
                      <div className="ip-pattern-title">{capitalize(p.title)}</div>
                      <div className="ip-pattern-detail">{p.detail}</div>
                    </div>
                  </div>
                ))}
              </div>
            )}
            {(snapshot.strengths.length > 0 || snapshot.opportunities.length > 0) && (
              <div className="ip-card">
                <div className="ip-card-label">
                  <Glyph name="bulb" size={12} /> Takeaways
                </div>
                {snapshot.strengths.slice(0, 3).map((text) => (
                  <div className="ip-takeaway keep" key={`k-${text}`} title={`Keep doing: ${text}`}>
                    <FilledMark kind="keep" />
                    <span>{text}</span>
                  </div>
                ))}
                {snapshot.opportunities.slice(0, 3).map((text) => (
                  <div className="ip-takeaway next" key={`n-${text}`} title={`Try next: ${text}`}>
                    <FilledMark kind="next" />
                    <span>{text}</span>
                  </div>
                ))}
              </div>
            )}
          </>
        ) : (
          !generating && (
            <div className="ip-card ip-empty">
              <div className="ip-empty-title">
                <Glyph name="brain" size={14} /> No insights yet
              </div>
              <div className="ip-empty-text">Generate an analysis of how you write, from your dictation history.</div>
            </div>
          )
        )}

        {status && <div className="ip-status">{status}</div>}

        <div className="ip-options">
          <CollapsibleRow title="Options" detail={optionsLine(request)}>
            <div className="ip-card ip-filters">
              <div className="ip-pickers">
                <PickerColumn title="Range">
                  <Select
                    ariaLabel="Range"
                    value={request.time_range}
                    options={TIME_RANGES}
                    onChange={(time_range) => changeRequest({ time_range }, "Range updated. Click Regenerate to refresh.")}
                  />
                </PickerColumn>
                <PickerColumn title="Mode">
                  <Select
                    ariaLabel="Mode"
                    value={request.generation_mode}
                    options={MODES}
                    onChange={(generation_mode) => changeRequest({ generation_mode }, "Mode updated. Click Regenerate to refresh.")}
                  />
                </PickerColumn>
                <PickerColumn title="Apps">
                  <Select
                    ariaLabel="Apps"
                    value={request.app_filter_mode}
                    options={APP_FILTERS}
                    onChange={(app_filter_mode) =>
                      changeRequest(
                        { app_filter_mode, selected_apps: app_filter_mode === "all_apps" ? [] : request.selected_apps },
                        "App filter updated. Click Regenerate to refresh.",
                      )
                    }
                  />
                </PickerColumn>
              </div>
              {request.app_filter_mode !== "all_apps" && (
                <AppsMenu
                  apps={appOptions}
                  selected={request.selected_apps}
                  onToggle={toggleApp}
                  onClear={() => changeRequest({ selected_apps: [] }, "App filter cleared. Click Regenerate to refresh.")}
                />
              )}
            </div>
          </CollapsibleRow>
        </div>

        {history.length > 0 && (
          <CollapsibleRow title="Recent runs" detail={String(history.length)}>
            <div className="ip-runs">
              {history.slice(0, 8).map((item) => (
                <HistoryRow
                  key={item.id}
                  item={item}
                  selected={selectedId === item.id}
                  compareIndex={compareIds.indexOf(item.id)}
                  onLoad={() => loadItem(item)}
                  onPin={() => togglePin(item)}
                  onCompare={() => toggleCompare(item)}
                  onDelete={() => deleteItem(item)}
                />
              ))}
              {comparison && (
                <div className="ip-card ip-comparison">
                  <div className="ip-comparison-head">
                    <span>Comparison</span>
                    <button className="btn btn-ghost btn-md ip-destructive" onClick={clearComparison}>
                      <span>Clear</span>
                    </button>
                  </div>
                  <div className="ip-comparison-headline">{comparison.headline}</div>
                  {comparison.bullets.map((bullet) => (
                    <div className="ip-comparison-bullet" key={bullet}>
                      <span>•</span>
                      <span>{bullet}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </CollapsibleRow>
        )}
      </div>
    </section>
  );
}

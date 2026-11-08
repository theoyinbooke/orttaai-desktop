// Analytics (Mac AnalyticsView + AnalyticsDashboardView): Dashboard, Tone of
// Voice and History tabs in the title strip; the dashboard's range lives in
// `settings.analytics_time_range`. Charts are hand-rolled SVG with hover
// tooltips. Owned by the analytics/home area.
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import "../styles/analytics.css";
import { Icon, PageHeader, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import {
  RANGES,
  changeText,
  dayDate,
  durationText,
  fmt,
  formatDuration,
  formatMinutes,
  hourLabel,
  hourName,
  modelDisplayName,
  normalizeRange,
  pendingAnalyticsTab,
  percent,
  plural,
  type AnalyticsSnapshot,
  type AnalyticsTab,
  type HeaderStats,
  type PerformanceHealth,
  type PerformanceLevel,
  type TimeRange,
  type TrendPoint,
} from "../lib/analytics";
import History from "./History";
import ToneOfVoice from "./analytics/ToneOfVoice";
import WritingInsights from "./analytics/WritingInsights";

const TABS: { value: AnalyticsTab; label: string; icon: string }[] = [
  { value: "dashboard", label: "Dashboard", icon: "barsAxis" },
  { value: "tone", label: "Tone of Voice", icon: "pulse" },
  { value: "history", label: "History", icon: "clockArrow" },
];

export default function Analytics() {
  const [tab, setTab] = useState<AnalyticsTab>(() => {
    const pending = pendingAnalyticsTab.get();
    pendingAnalyticsTab.set(null);
    return pending ?? "dashboard";
  });
  const { settings, patch } = useSettings();
  const toast = useToast();
  const range = normalizeRange(settings?.analytics_time_range);
  // Home's "Open History" while Analytics is already open.
  useEffect(
    () =>
      pendingAnalyticsTab.subscribe(() => {
        const t = pendingAnalyticsTab.get();
        if (t) {
          pendingAnalyticsTab.set(null);
          setTab(t);
        }
      }),
    [],
  );

  return (
    <div className={`view an-view ${tab === "history" ? "an-fill" : ""}`}>
      <PageHeader
        title="Analytics"
        leading={
          <div className="an-tabbar" role="tablist" aria-label="Analytics sections">
            {TABS.map((t) => (
              <button key={t.value} role="tab" aria-selected={tab === t.value} className={`an-tabbar-btn ${tab === t.value ? "active" : ""}`} onClick={() => setTab(t.value)}>
                <Icon name={t.icon} size={13} />
                {t.label}
              </button>
            ))}
          </div>
        }
        actions={
          // Only the dashboard has a range to pick.
          tab === "dashboard" ? (
            <div className="an-picker" role="radiogroup" aria-label="Time range">
              {RANGES.map((r) => (
                <button
                  key={r.value}
                  role="radio"
                  aria-checked={range === r.value}
                  className={`an-picker-btn ${range === r.value ? "active" : ""}`}
                  onClick={() => patch({ analytics_time_range: r.value }).catch((e) => toast(String(e), "error"))}
                >
                  {r.label}
                </button>
              ))}
            </div>
          ) : undefined
        }
      />
      <div className="an-tab-body">
        {tab === "dashboard" && <Dashboard range={range} />}
        {tab === "tone" && <ToneOfVoice />}
        {tab === "history" && <History />}
      </div>
    </div>
  );
}

/* --------------------------------------------------------------- dashboard -- */

function Dashboard({ range }: { range: TimeRange }) {
  const [snap, setSnap] = useState<AnalyticsSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const gen = useRef(0);
  const load = useCallback(() => {
    const g = ++gen.current;
    call<AnalyticsSnapshot>("analytics_dashboard", { range })
      .then((s) => {
        // An older refresh never replaces a newer one's numbers.
        if (g !== gen.current) return;
        setSnap(s);
        setError(null);
      })
      .catch((e) => g === gen.current && setError(String(e)));
  }, [range]);
  useEffect(load, [load]);
  useEvent("history-changed", load);

  // Drawn once the numbers are in: before them it reads as an empty archive.
  if (!snap) {
    return error ? <div className="an-error">Failed to load analytics. {error}</div> : null;
  }
  const meta = RANGES.find((r) => r.value === range) ?? RANGES[0];
  const today = snap.payload.today;
  const trend = snap.trend_points;
  const prev = snap.previous;
  const vs = range === "all" ? "" : ` vs prev ${meta.short}`;
  // The change on the last period lives in the card's tooltip: the Mac's cards
  // have no room set aside for it.
  const hint = (change: number | null | undefined) => {
    const text = changeText(change);
    return text ? `${text}${vs}` : undefined;
  };

  return (
    <div className="an-dash">
      {error && <div className="an-error">Failed to refresh analytics. {error}</div>}

      <div className="an-summary">
        <SummaryMetric
          warm
          icon={<WordsGlyph />}
          title="Words"
          value={fmt(snap.total_words)}
          detail={snap.active_days > 0 ? `${fmt(snap.total_words / snap.active_days)} per active day` : "No dictation yet"}
          change={hint(prev?.words_change)}
          series={trend.map((p) => p.words)}
        />
        <SummaryMetric
          icon={<Icon name="waveform" size={13} />}
          title="Sessions"
          value={fmt(snap.total_sessions)}
          detail={`${fmt(today.sessions)} today`}
          change={hint(prev?.sessions_change)}
          series={trend.map((p) => p.sessions)}
        />
        <SummaryMetric
          icon={<Icon name="clock" size={13} />}
          title="Active Time"
          value={formatMinutes(snap.total_recording_minutes, snap.total_sessions > 0)}
          detail={`${formatMinutes(today.active_minutes, today.sessions > 0)} today`}
          change={hint(prev?.active_minutes_change)}
          series={snap.daily_active_seconds}
        />
        <SummaryMetric
          icon={<Icon name="speedometer" size={13} />}
          title="Avg WPM"
          value={snap.average_wpm > 0 ? fmt(snap.average_wpm) : "–"}
          detail={snap.active_days === 1 ? "over 1 active day" : `over ${fmt(snap.active_days)} active days`}
          change={hint(prev?.wpm_change)}
          // Quiet days have no speed, so they'd only draw dips.
          series={trend.filter((p) => p.sessions > 0).map((p) => p.average_wpm)}
        />
      </div>

      <TrendCard title={meta.trendTitle} points={trend} axisTicks={meta.ticks} emptyMessage="No activity in this range yet." />

      <div className="an-pair">
        <WeekdayCard snap={snap} />
        <HourCard snap={snap} />
      </div>
      <div className="an-pair">
        <AppsCard snap={snap} />
        <DurationCard snap={snap} />
      </div>

      <PerformanceCard health={snap.payload.performance} />

      <ActivityCard snap={snap} rangeLabel={meta.short} />

      <WritingInsights />
    </div>
  );
}

/** The Mac's `character.cursor.ibeam`: a capital A beside a text cursor. */
function WordsGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M2.5 19 8.5 5l6 14M4.6 14.5h7.8M19 4v16M16.5 4h5M16.5 20h5" />
    </svg>
  );
}

function SummaryMetric({
  title,
  value,
  detail,
  icon,
  series,
  warm,
  change,
}: {
  title: string;
  value: string;
  detail: string;
  icon: ReactNode;
  series: number[];
  warm?: boolean;
  change?: string;
}) {
  // Long values shrink to fit, as the Mac's minimumScaleFactor.
  const fontSize = value.length <= 5 ? 30 : value.length === 6 ? 28 : value.length === 7 ? 25 : 21;
  return (
    <div className={`card an-metric ${warm ? "warm" : ""}`} title={change} aria-label={`${title}: ${value}, ${detail}${change ? `, ${change}` : ""}`}>
      <div className="an-metric-head">
        <span className="card-icon" aria-hidden="true">
          {icon}
        </span>
        <span>{title}</span>
      </div>
      <div className="an-metric-row">
        <div className="an-metric-text">
          <span className="an-metric-value" style={{ fontSize }}>
            {value}
          </span>
          <span className="an-metric-detail">{detail}</span>
        </div>
        <Sparkline values={series} />
      </div>
    </div>
  );
}

export function CardIcon({ name }: { name: string }) {
  return (
    <span className="card-icon" aria-hidden="true">
      <Icon name={name} size={13} />
    </span>
  );
}


/** A small area line of daily values; long ranges averaged into ≤ 45 points. */
export function Sparkline({ values, width = 84, height = 30 }: { values: number[]; width?: number; height?: number }) {
  const limit = 45;
  let pts = values;
  if (values.length > limit) {
    const size = values.length / limit;
    pts = Array.from({ length: limit }, (_, i) => {
      const chunk = values.slice(Math.floor(i * size), Math.min(values.length, Math.floor((i + 1) * size)));
      return chunk.length ? chunk.reduce((a, b) => a + b, 0) / chunk.length : 0;
    });
  }
  const peak = Math.max(1, ...pts);
  const inset = 1.5;
  const h = height - inset * 2;
  const xy = pts.map((v, i) => [pts.length > 1 ? (width * i) / (pts.length - 1) : width, inset + h * (1 - v / peak)] as const);
  const has = xy.length >= 2 && pts.some((v) => v > 0);
  const id = useRef(`spark-${Math.random().toString(36).slice(2)}`).current;
  return (
    <svg className="sparkline" width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
      {has ? (
        <>
          <defs>
            <linearGradient id={id} x1="0" x2="0" y1="0" y2="1">
              <stop offset="0" stopColor="var(--accent)" stopOpacity="0.28" />
              <stop offset="1" stopColor="var(--accent)" stopOpacity="0.02" />
            </linearGradient>
          </defs>
          <path d={`M${xy[0][0]},${height} ${xy.map(([x, y]) => `L${x},${y}`).join(" ")} L${xy[xy.length - 1][0]},${height}Z`} fill={`url(#${id})`} />
          <path d={`M${xy.map(([x, y]) => `${x},${y}`).join(" L")}`} fill="none" stroke="var(--accent)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </>
      ) : (
        <line x1="0" x2={width} y1={height - inset} y2={height - inset} stroke="var(--faint)" strokeOpacity="0.6" strokeDasharray="2 3" />
      )}
    </svg>
  );
}

/** Track an element's content width. */
export function useWidth<T extends HTMLElement>(): [(el: T | null) => void, number] {
  const [w, setW] = useState(0);
  const ro = useRef<ResizeObserver | null>(null);
  // A callback ref, so an element that appears later (after an empty state) is measured too.
  const ref = useCallback((el: T | null) => {
    ro.current?.disconnect();
    ro.current = null;
    if (!el) return;
    setW(el.clientWidth);
    const observer = new ResizeObserver(() => setW(el.clientWidth));
    observer.observe(el);
    ro.current = observer;
  }, []);
  useEffect(() => () => ro.current?.disconnect(), []);
  return [ref, w];
}

/** Monotone cubic (Fritsch–Carlson) path through points, as Swift Charts' `.monotone`. */
function monotonePath(pts: [number, number][]): string {
  const n = pts.length;
  if (n === 0) return "";
  if (n === 1) return `M${pts[0][0]},${pts[0][1]}`;
  const dx: number[] = [];
  const slope: number[] = [];
  for (let i = 0; i < n - 1; i++) {
    dx.push(pts[i + 1][0] - pts[i][0]);
    slope.push((pts[i + 1][1] - pts[i][1]) / (dx[i] || 1));
  }
  const m: number[] = [slope[0]];
  for (let i = 1; i < n - 1; i++) m.push(slope[i - 1] * slope[i] <= 0 ? 0 : (slope[i - 1] + slope[i]) / 2);
  m.push(slope[n - 2]);
  for (let i = 0; i < n - 1; i++) {
    if (slope[i] === 0) {
      m[i] = 0;
      m[i + 1] = 0;
      continue;
    }
    const a = m[i] / slope[i];
    const b = m[i + 1] / slope[i];
    const s = a * a + b * b;
    if (s > 9) {
      const t = 3 / Math.sqrt(s);
      m[i] = t * a * slope[i];
      m[i + 1] = t * b * slope[i];
    }
  }
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 0; i < n - 1; i++) {
    const h = dx[i] / 3;
    d += ` C${pts[i][0] + h},${pts[i][1] + m[i] * h} ${pts[i + 1][0] - h},${pts[i + 1][1] - m[i + 1] * h} ${pts[i + 1][0]},${pts[i + 1][1]}`;
  }
  return d;
}

/** Round axis steps of 1, 2 or 5 × 10ⁿ, about four to the chart, as Swift Charts picks. */
function niceScale(v: number): { top: number; step: number } {
  if (v <= 0) return { top: 10, step: 2.5 };
  const raw = v / 4;
  const exp = Math.pow(10, Math.floor(Math.log10(raw)));
  const f = raw / exp;
  const step = (f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10) * exp;
  return { top: Math.ceil(v / step) * step, step };
}

const shortDay = (d: Date) => d.toLocaleDateString(undefined, { month: "short", day: "numeric" });

/** Words per day as a soft area, average WPM as a line over it (Mac TrendCardView). */
export function TrendCard({
  title = "30-Day Trend",
  points,
  allTime,
  axisTicks = 8,
  emptyMessage = "No activity yet. Your last 30 days will appear after you dictate.",
}: {
  title?: string;
  points: TrendPoint[];
  allTime?: HeaderStats;
  axisTicks?: number;
  emptyMessage?: string;
}) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const empty = points.every((p) => p.words === 0 && p.sessions === 0);
  const H = 210;
  const { top, step } = niceScale(Math.max(1, ...points.map((p) => Math.max(p.words, p.average_wpm))));
  const pad = { l: fmt(top).length * 6.4 + 8, r: 0, t: 6, b: 22 };
  const iw = Math.max(1, w - pad.l - pad.r);
  const ih = H - pad.t - pad.b;
  // Each day owns a band of the axis; its point sits mid-band, as a mark with `unit: .day`.
  const band = iw / Math.max(1, points.length);
  const tickX = (i: number) => Math.round(pad.l + band * i) + 0.5;
  const x = (i: number) => pad.l + band * (i + 0.5);
  const y = (v: number) => pad.t + ih * (1 - v / top);
  const words = points.map((p, i) => [x(i), y(p.words)] as [number, number]);
  const wpm = points.map((p, i) => [x(i), y(p.average_wpm)] as [number, number]);
  const wordsPath = monotonePath(words);
  const yTicks = Array.from({ length: Math.round(top / step) + 1 }, (_, i) => i * step);
  const stride = Math.max(1, Math.ceil(points.length / axisTicks));
  const gradId = useRef(`trend-${Math.random().toString(36).slice(2)}`).current;
  const hp = hover != null ? points[hover] : null;

  const onMove = (e: ReactMouseEvent<SVGRectElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const i = Math.floor(((e.clientX - rect.left) / rect.width) * points.length);
    setHover(Math.max(0, Math.min(points.length - 1, i)));
  };
  const totalWords = points.reduce((a, p) => a + p.words, 0);
  const spoken = points.filter((p) => p.sessions > 0);

  return (
    <div className="card an-card an-trend-card">
      <div className="an-card-head">
        <h3>{title}</h3>
        {allTime && allTime.active_days > 0 && (
          <span className="an-card-detail mono-num" title="Since your first dictation">
            All time · {plural(allTime.total_words, "word")} · {plural(allTime.active_days, "day")} · {allTime.average_wpm} WPM
          </span>
        )}
      </div>
      {empty ? (
        <p className="an-empty">{emptyMessage}</p>
      ) : (
        <>
          <div className="an-trend" ref={ref}>
            {w > 0 && (
              <svg
                width={w}
                height={H}
                role="img"
                aria-label={`${title} chart: ${spoken.length} active days, ${fmt(totalWords)} total words`}
              >
                <defs>
                  <linearGradient id={gradId} x1="0" x2="0" y1="0" y2="1">
                    <stop offset="0" stopColor="var(--accent)" stopOpacity="0.32" />
                    <stop offset="1" stopColor="var(--accent)" stopOpacity="0.02" />
                  </linearGradient>
                </defs>
                {yTicks.map((t) => (
                  <g key={t}>
                    <line x1={pad.l} x2={w - pad.r} y1={Math.round(y(t)) + 0.5} y2={Math.round(y(t)) + 0.5} className="an-grid" />
                    <text x={pad.l - 6} y={y(t) + 4} textAnchor="end" className="an-axis">
                      {fmt(t)}
                    </text>
                  </g>
                ))}
                {points.map((p, i) =>
                  i % stride === 0 ? (
                    <g key={p.day}>
                      <line x1={tickX(i)} x2={tickX(i)} y1={pad.t} y2={pad.t + ih} className="an-grid" />
                      <line x1={tickX(i)} x2={tickX(i)} y1={pad.t + ih} y2={pad.t + ih + 4} className="an-tick" />
                      {tickX(i) + 36 <= w && (
                        <text x={tickX(i)} y={H - 4} textAnchor="start" className="an-axis">
                          {shortDay(dayDate(p.day))}
                        </text>
                      )}
                    </g>
                  ) : null,
                )}
                <path d={`${wordsPath} L${x(points.length - 1)},${y(0)} L${x(0)},${y(0)}Z`} fill={`url(#${gradId})`} />
                <path d={wordsPath} fill="none" stroke="var(--accent)" strokeOpacity="0.45" strokeWidth="1.5" />
                <path d={monotonePath(wpm)} fill="none" stroke="var(--accent)" strokeWidth="2" />
                {hp && hover != null && (
                  <g>
                    <line x1={x(hover)} x2={x(hover)} y1={pad.t} y2={pad.t + ih} className="an-rule" />
                    <circle cx={x(hover)} cy={y(hp.words)} r="3.4" fill="var(--accent)" fillOpacity="0.6" />
                    <circle cx={x(hover)} cy={y(hp.average_wpm)} r="3.9" fill="var(--accent)" />
                  </g>
                )}
                <rect
                  x={pad.l}
                  y={pad.t}
                  width={iw}
                  height={ih}
                  fill="transparent"
                  onMouseMove={onMove}
                  onMouseLeave={() => setHover(null)}
                />
              </svg>
            )}
            {hp && hover != null && (
              <ChartTip x={x(hover)} width={w}>
                <span className="tip-date">{dayDate(hp.day).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" })}</span>
                {hp.sessions === 0 ? (
                  <span className="tip-muted">No dictation</span>
                ) : (
                  <>
                    <span className="tip-row">
                      <i className="dot soft" />
                      <b>{fmt(hp.words)}</b>
                      <small>{hp.words === 1 ? "word" : "words"}</small>
                    </span>
                    <span className="tip-row">
                      <i className="dot" />
                      <b>{fmt(hp.average_wpm)}</b>
                      <small>WPM</small>
                    </span>
                    <span className="tip-muted">{plural(hp.sessions, "session")}</span>
                  </>
                )}
              </ChartTip>
            )}
          </div>
          <div className="an-legend">
            <span>
              <i className="dot soft" /> Words/day
            </span>
            <span>
              <i className="dot" /> Avg WPM/day
            </span>
          </div>
        </>
      )}
    </div>
  );
}

function ChartTip({ x, width, children }: { x: number; width: number; children: ReactNode }) {
  const left = Math.max(70, Math.min(width - 70, x));
  return (
    <div className="chart-tip" style={{ left }} role="tooltip">
      {children}
    </div>
  );
}

/** A bar chart with a focused bar (hovered, else the busiest) and counts on top. */
function Bars({
  items,
  focus,
  onHover,
  showCounts = true,
  label,
  tickFor,
  height = 132,
  radius = 4,
  ariaLabel,
}: {
  items: { key: string; value: number }[];
  focus: string | null;
  onHover?: (key: string | null) => void;
  showCounts?: boolean;
  label: (key: string) => string;
  tickFor?: (key: string) => boolean;
  height?: number;
  radius?: number;
  ariaLabel: string;
}) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const { top: scaleTop } = niceScale(Math.max(1, ...items.map((i) => i.value)));
  const top = showCounts ? 16 : 2;
  const bottom = 20;
  const ih = height - top - bottom;
  const slot = w / Math.max(1, items.length);
  const gap = slot * 0.28;
  const bw = Math.max(2, slot - gap);
  return (
    <div className="an-bars" ref={ref}>
      {w > 0 && (
        <svg width={w} height={height} role="img" aria-label={ariaLabel} onMouseLeave={() => onHover?.(null)}>
          {items.map((it, i) => {
            const h = (ih * it.value) / scaleTop;
            const bx = i * slot + gap / 2;
            const by = top + ih - h;
            return (
              <g key={it.key} onMouseEnter={() => onHover?.(it.key)}>
                <rect x={i * slot} y={0} width={slot} height={height} fill="transparent" />
                {it.value > 0 && <rect x={bx} y={by} width={bw} height={h} rx={Math.min(radius, bw / 2, h / 2)} className={`an-bar ${it.key === focus ? "on" : ""}`} />}
                {showCounts && it.value > 0 && (
                  <text x={bx + bw / 2} y={by - 4} textAnchor="middle" className="an-axis">
                    {fmt(it.value)}
                  </text>
                )}
                {(!tickFor || tickFor(it.key)) && (
                  <text x={bx + bw / 2} y={height - 4} textAnchor="middle" className="an-axis">
                    {label(it.key)}
                  </text>
                )}
              </g>
            );
          })}
        </svg>
      )}
    </div>
  );
}

function AnCard({ title, detail, children }: { title: string; detail: string; children: ReactNode }) {
  return (
    <div className="card an-card">
      <div className="an-card-head">
        <h3>{title}</h3>
        <span className="an-card-detail">{detail}</span>
      </div>
      {children}
    </div>
  );
}

function Headline({ text, detail }: { text: string; detail: string }) {
  return (
    <div className="an-headline">
      <span className="an-headline-text">{text}</span>
      <span className="an-headline-detail">{detail}</span>
    </div>
  );
}

function WeekdayCard({ snap }: { snap: AnalyticsSnapshot }) {
  const [hover, setHover] = useState<string | null>(null);
  const days = snap.weekday_activity;
  const total = days.reduce((a, d) => a + d.sessions, 0);
  const busiest = days.reduce((a, d) => (d.sessions > a.sessions ? d : a), days[0]);
  const focus = days.find((d) => d.short_name === hover) ?? busiest;
  return (
    <AnCard title="Weekly Rhythm" detail="Sessions by weekday">
      {total === 0 ? (
        <p className="an-empty">No sessions in this range yet.</p>
      ) : (
        <>
          <Headline
            text={focus.weekday === busiest.weekday && hover == null ? `Busiest on ${focus.name}s` : focus.name}
            detail={`${plural(focus.sessions, "session")} · ${percent(focus.sessions, total)} of the range`}
          />
          <Bars
            items={days.map((d) => ({ key: d.short_name, value: d.sessions }))}
            focus={focus.short_name}
            onHover={setHover}
            label={(k) => k}
            ariaLabel="Sessions by weekday"
          />
        </>
      )}
    </AnCard>
  );
}

function HourCard({ snap }: { snap: AnalyticsSnapshot }) {
  const [hover, setHover] = useState<string | null>(null);
  const hours = snap.hourly_activity;
  const total = hours.reduce((a, h) => a + h.count, 0);
  const peak = hours.reduce((a, h) => (h.count > a.count ? h : a), hours[0]);
  const focus = hours.find((h) => hourLabel(h.hour) === hover) ?? peak;
  return (
    <AnCard title="Time of Day" detail="Sessions by hour">
      {total === 0 ? (
        <p className="an-empty">No sessions in this range yet.</p>
      ) : (
        <>
          <Headline
            text={hover == null ? `Most active around ${hourName(focus.hour)}` : `${hourName(focus.hour)} to ${hourName((focus.hour + 1) % 24)}`}
            detail={`${plural(focus.count, "session")} · ${percent(focus.count, total)} of the range`}
          />
          <Bars
            items={hours.map((h) => ({ key: hourLabel(h.hour), value: h.count }))}
            focus={hourLabel(focus.hour)}
            onHover={setHover}
            showCounts={false}
            radius={2}
            label={(k) => k}
            // Every sixth hour, so the labels breathe.
            tickFor={(k) => ["12a", "6a", "12p", "6p"].includes(k)}
            ariaLabel="Sessions by hour of day"
          />
        </>
      )}
    </AnCard>
  );
}

function AppsCard({ snap }: { snap: AnalyticsSnapshot }) {
  const apps = snap.words_by_app;
  const total = Math.max(1, snap.total_words);
  return (
    <AnCard title="Words by App" detail={`Top ${Math.max(1, apps.length)}`}>
      {apps.length === 0 ? (
        <p className="an-empty">No app data yet.</p>
      ) : (
        <>
          <Headline text={`${apps[0].name} gets the most`} detail={`${percent(apps[0].words, total)} of your words in this range`} />
          <div className="an-share-list">
            {apps.map((a, i) => (
              <ShareRow key={a.name} name={a.name} value={fmt(a.words)} share={a.words / total} opacity={Math.max(0.35, 1 - i * 0.16)} />
            ))}
          </div>
        </>
      )}
    </AnCard>
  );
}

export function ShareRow({ name, value, share, opacity = 1, sub }: { name: string; value: string; share: number; opacity?: number; sub?: string }) {
  return (
    <div className="an-share-row" aria-label={`${name}, ${value}, ${Math.round(share * 100)} percent`}>
      <div className="an-share-top">
        <span className="an-share-name">{name}</span>
        {sub && <span className="an-share-sub">{sub}</span>}
        <span className="an-share-value">{value}</span>
        <span className="an-share-pct">{Math.round(share * 100)}%</span>
      </div>
      <div className="an-track">
        <span style={{ width: `max(4px, ${share * 100}%)`, opacity }} />
      </div>
    </div>
  );
}

function DurationCard({ snap }: { snap: AnalyticsSnapshot }) {
  const [hover, setHover] = useState<string | null>(null);
  const buckets = snap.duration_buckets;
  const total = buckets.reduce((a, b) => a + b.count, 0);
  const common = buckets.reduce((a, b) => (b.count > a.count ? b : a), buckets[0]);
  const focus = buckets.find((b) => b.label === hover) ?? common;
  return (
    <AnCard title="Session Length" detail="How long you speak">
      {total === 0 ? (
        <p className="an-empty">No session data yet.</p>
      ) : (
        <>
          <Headline
            text={hover == null ? `Most last ${common.phrase}` : `${plural(focus.count, "session")} ${focus.phrase}`}
            detail={snap.median_duration_ms != null ? `Median ${formatDuration(snap.median_duration_ms)} · ${plural(total, "session")}` : plural(total, "session")}
          />
          <Bars items={buckets.map((b) => ({ key: b.label, value: b.count }))} focus={focus.label} onHover={setHover} label={(k) => k} ariaLabel="Sessions by length" />
        </>
      )}
    </AnCard>
  );
}

/** Streaks, typing time saved, and a day-by-day heatmap of the range. */
function ActivityCard({ snap, rangeLabel }: { snap: AnalyticsSnapshot; rangeLabel: string }) {
  const [hover, setHover] = useState<TrendPoint | null>(null);
  const days = snap.trend_points;
  if (days.length === 0) return null;
  const first = dayDate(days[0].day);
  const lead = first.getDay(); // Sunday-first columns
  const cells: (TrendPoint | null)[] = [...Array(lead).fill(null), ...days];
  const weeks = Math.ceil(cells.length / 7);
  const peak = Math.max(1, ...days.map((d) => d.words));
  const level = (w: number) => (w <= 0 ? 0 : Math.min(4, Math.ceil((w / peak) * 4)));
  const st = snap.streaks;
  const gap = 3;
  const cell = 14;
  const active = days.filter((d) => d.sessions > 0).length;
  return (
    <AnCard title="Streaks & Activity" detail="Days with a dictation">
      <div className="an-activity">
        <div className="an-streaks">
          <StreakStat icon="flame" value={plural(st.current_days, "day")} label={st.dictated_today || st.current_days === 0 ? "current streak" : "current streak · dictate today to keep it"} />
          <StreakStat icon="trophy" value={plural(st.best_days, "day")} label="best streak" />
          <StreakStat icon="history" value={durationText(snap.minutes_saved)} label={`typing saved (${rangeLabel}) at 40 WPM`} />
        </div>
        <div className="an-heat-wrap">
          <svg
            className="an-heat"
            width={weeks * (cell + gap)}
            height={7 * (cell + gap)}
            role="img"
            aria-label={`Daily activity: ${active} active days of ${days.length}`}
            onMouseLeave={() => setHover(null)}
          >
            {cells.map((d, i) =>
              d ? (
                <rect
                  key={d.day}
                  x={Math.floor(i / 7) * (cell + gap)}
                  y={(i % 7) * (cell + gap)}
                  width={cell}
                  height={cell}
                  rx={3}
                  className={`heat l${level(d.words)} ${hover?.day === d.day ? "on" : ""}`}
                  onMouseEnter={() => setHover(d)}
                />
              ) : null,
            )}
          </svg>
          <div className="an-heat-foot">
            <span className="an-headline-detail">
              {hover
                ? `${dayDate(hover.day).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" })}: ${hover.sessions === 0 ? "no dictation" : `${plural(hover.words, "word")} · ${plural(hover.sessions, "session")}`}`
                : `${fmt(active)} of ${fmt(days.length)} days active`}
            </span>
            <span className="an-heat-key" aria-hidden="true">
              Less {[0, 1, 2, 3, 4].map((l) => <i key={l} className={`heat l${l}`} />)} More
            </span>
          </div>
        </div>
      </div>
    </AnCard>
  );
}

function StreakStat({ icon, value, label }: { icon: string; value: string; label: string }) {
  return (
    <div className="an-streak">
      <CardIcon name={icon} />
      <div>
        <span className="an-streak-value">{value}</span>
        <span className="an-streak-label">{label}</span>
      </div>
    </div>
  );
}

export const levelLabel: Record<PerformanceLevel, string> = { no_data: "No Data", fast: "Fast", normal: "Normal", slow: "Slow" };
export function LevelPill({ level }: { level: PerformanceLevel }) {
  return <span className={`level-pill level-${level}`}>{levelLabel[level]}</span>;
}

function PerformanceCard({ health }: { health: PerformanceHealth }) {
  const has = health.sample_count > 0;
  // One scale for all four, so their bars compare.
  const scale = Math.max(
    1,
    ...[health.processing_p95_ms, health.average_processing_ms, health.transcription_p95_ms, health.text_processing_p95_ms, health.injection_p95_ms].filter(
      (v): v is number => v != null,
    ),
  );
  return (
    <div className="card an-card">
      <div className="an-card-head">
        <h3>Performance</h3>
        <span className="an-card-detail" title={health.current_model_id}>
          {has && `${health.sample_count === 1 ? "Last dictation" : `Last ${fmt(health.sample_count)} dictations`} · ${modelDisplayName(health.current_model_id)}`}
        </span>
        <LevelPill level={health.level} />
      </div>
      {!has ? (
        <p className="an-empty">Timings appear after your first dictation.</p>
      ) : (
        <div className="an-latency">
          <LatencyColumn title="Total" avg={health.average_processing_ms} p50={health.processing_p50_ms} p95={health.processing_p95_ms} scale={scale} tone="c1" />
          <LatencyColumn title="Transcribe" avg={health.average_transcription_ms} p50={health.transcription_p50_ms} p95={health.transcription_p95_ms} scale={scale} tone="c2" />
          <LatencyColumn title="Rules & polish" avg={health.average_text_processing_ms} p50={health.text_processing_p50_ms} p95={health.text_processing_p95_ms} scale={scale} tone="c3" />
          <LatencyColumn title="Insert" avg={health.average_injection_ms} p50={health.injection_p50_ms} p95={health.injection_p95_ms} scale={scale} tone="c4" />
        </div>
      )}
    </div>
  );
}

/** The average as the number and solid bar; the slowest 5% (p95) as the faint bar behind it. */
function LatencyColumn({ title, avg, p50, p95, scale, tone }: { title: string; avg: number | null; p50: number | null; p95: number | null; scale: number; tone: string }) {
  const line = `${p50 != null ? `typical ${fmt(p50)} ms` : "typical –"} · ${p95 != null ? `slowest ${fmt(p95)} ms` : "slowest –"}`;
  return (
    <div className={`an-lat ${tone}`} title={`${title}: the solid bar is the average, the faint one the slowest 5% (p95)`} aria-label={`${title}: average ${avg != null ? `${avg} milliseconds` : "no data"}. ${line}.`}>
      <span className="an-lat-title">
        <i className="dot" /> {title}
      </span>
      <span className="an-lat-value">
        {avg != null ? fmt(avg) : "–"} <small>ms avg</small>
      </span>
      <span className="an-track tall">
        {p95 != null && <span className="p95" style={{ width: `${Math.min(1, p95 / scale) * 100}%` }} />}
        {avg != null && <span style={{ width: `max(4px, ${Math.min(1, avg / scale) * 100}%)` }} />}
      </span>
      <span className="an-lat-line">{line}</span>
    </div>
  );
}

// Home insights panel (Mac HomeInsightsPanel + InsightStatTicker): a ticker of
// the numbers behind the insight, the summary, the top patterns and
// takeaways, sized to read on one screen. Self-contained — the Home view only
// renders <InsightsPanel />. Owned by the semantic area.
import { useCallback, useEffect, useRef, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import "../styles/semantic.css";
import { Button, Icon, IconButton, useToast } from "../ui";
import { useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import {
  GLYPHS,
  mediumDateTime,
  plural,
  relative,
  semantic,
  type HomeInsights,
  type InsightStat,
  type RankedFinding,
} from "../lib/semantic";

function Glyph({ name, size = 16 }: { name: string; size?: number }) {
  const d = GLYPHS[name];
  if (!d) return <Icon name={name} size={size} />;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.75} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

const HOLD_MS = 3200;

function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/** Rolls a number up from zero (spring-ish ease-out). */
function useRollUp(target: number, key: string) {
  const [shown, setShown] = useState(target);
  useEffect(() => {
    if (prefersReducedMotion()) {
      setShown(target);
      return;
    }
    let frame = 0;
    const start = performance.now();
    const duration = 900;
    const tick = (t: number) => {
      const p = Math.min(1, (t - start) / duration);
      const eased = 1 - Math.pow(1 - p, 3);
      setShown(target * eased);
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
export function InsightStatTicker({ stats, scanning = false }: { stats: InsightStat[]; scanning?: boolean }) {
  const [index, setIndex] = useState(0);
  const signature = stats.map((s) => s.id).join("|");
  useEffect(() => {
    setIndex(0);
    if (stats.length < 2 || prefersReducedMotion()) return;
    const t = window.setInterval(() => setIndex((i) => (i + 1) % stats.length), HOLD_MS);
    return () => window.clearInterval(t);
  }, [signature]); // eslint-disable-line react-hooks/exhaustive-deps
  const current = stats.length ? stats[index % stats.length] : null;
  const value = useRollUp(current?.number ?? 0, `${current?.id ?? ""}-${index}`);
  if (!current) return null;
  const label = stats.map((s) => `${s.text ?? (s.number !== null ? Math.round(s.number).toLocaleString() : "")} ${s.label}`).join(", ");
  return (
    <div className={`ip-ticker ${scanning ? "scanning" : ""}`} role="group" aria-label={label}>
      <div className="ip-ring" aria-hidden="true">
        <svg className="ip-ring-art" viewBox="0 0 46 46">
          <circle cx="23" cy="23" r="21" fill="none" style={{ stroke: "var(--accent-line)" }} strokeWidth="2" />
          <g className="ip-sweep">
            <path d="M23 2 A21 21 0 0 1 43.5 18" fill="none" style={{ stroke: "var(--accent)" }} strokeWidth="2.5" strokeLinecap="round" />
          </g>
        </svg>
        <span key={`blip-${index}`} className="ip-blip" />
        <Glyph name={current.icon} size={15} />
      </div>
      <div className="ip-ticker-text">
        <div className="ip-value" key={`v-${current.id}`}>
          {current.text ? <span className="ip-swap">{current.text}</span> : Math.round(value).toLocaleString()}
        </div>
        <div className="ip-label ip-swap" key={`l-${current.id}-${index}`}>
          {current.label}
        </div>
        <div className="ip-drain" key={`d-${index}`}>
          <span />
        </div>
      </div>
      {stats.length > 1 && (
        <div className="ip-dots" aria-hidden="true">
          {stats.map((s, i) => (
            <span key={s.id} className={i === index % stats.length ? "on" : ""} />
          ))}
        </div>
      )}
    </div>
  );
}

function progressLine(elapsed: number, count: number | null) {
  if (elapsed < 3) return count ? `Reading your last ${count.toLocaleString()} dictations` : "Reading your recent dictations";
  if (elapsed < 10) return "Finding patterns in how your notes connect";
  if (elapsed < 25) return "Writing your summary";
  if (elapsed < 60) return "Still working. Larger models take a little longer";
  return "Almost there";
}

function ProgressRow({ startedAt, count }: { startedAt: number; count: number | null }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  const line = progressLine((now - startedAt) / 1000, count);
  return (
    <div className="ip-progress" role="status">
      <span className="spinner" style={{ width: 13, height: 13 }} aria-hidden="true" />
      <span className="ip-progress-line" key={line}>
        {line}
      </span>
    </div>
  );
}

const KEEP_KINDS = new Set(["rhythm", "life_area"]);

const capitalize = (s: string) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

export default function InsightsPanel({ onClose, onOpenGraph }: { onClose?: () => void; onOpenGraph?: () => void }) {
  const { settings, patch } = useSettings();
  const toast = useToast();
  const [home, setHome] = useState<HomeInsights | null>(null);
  const [generating, setGenerating] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [about, setAbout] = useState(false);
  const aboutRef = useRef<HTMLDivElement>(null);
  const enabled = settings?.semantic_enabled ?? false;

  const load = useCallback(() => {
    semantic
      .homeInsights()
      .then((h) => {
        setHome(h);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(load, [load, enabled]);
  useEvent<unknown>("semantic-updated", load);
  useEvent<unknown>("history-changed", load);

  useEffect(() => {
    if (!about) return;
    const close = (e: MouseEvent) => {
      if (!aboutRef.current?.contains(e.target as Node)) setAbout(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [about]);

  const regenerate = () => {
    setGenerating(Date.now());
    setError(null);
    semantic
      .generateInsights()
      .then((r) => {
        setStatus(
          r.summary_model_name
            ? `Analysed by ${r.summary_model_name} · ${plural(r.cards.length, "insight")}`
            : `${plural(r.cards.length, "pattern")} found on this computer`,
        );
        load();
      })
      .catch((e) => setError(String(e)))
      .finally(() => setGenerating(null));
  };

  const openGraph = () => (onOpenGraph ? onOpenGraph() : emit("navigate", "graph"));

  const findings: RankedFinding[] = home?.findings ?? [];
  const patterns = findings.slice(0, 3);
  const keep = findings.filter((f) => KEEP_KINDS.has(f.kind)).slice(0, 3);
  const next = findings.filter((f) => !KEEP_KINDS.has(f.kind)).slice(0, 3);
  const summary = home?.summary.length ? home.summary.slice(0, 3).join(" ") : null;
  const analyzer = home?.report_model ?? (home?.report_generated_at ? "Orttaai (on this computer)" : null);

  return (
    <section className="ip-panel" aria-label="Insights">
      <header className="ip-head">
        <div className="ip-head-text">
          <div className="ip-title">
            Insights
            {home?.report_generated_at && (
              <div className="sm-info-btn" ref={aboutRef}>
                <IconButton name="info" label="About this insight" onClick={() => setAbout((a) => !a)} />
                {about && (
                  <div className="sm-popover" style={{ left: 0, right: "auto" }} role="dialog">
                    <h4>About this insight</h4>
                    <div className="sm-popover-row">
                      <Glyph name="cpu" size={14} /> Analysed by {analyzer}
                    </div>
                    <div className="sm-popover-row">
                      <Icon name="history" size={14} /> Ran {relative(home.report_generated_at)},{" "}
                      {mediumDateTime(home.report_generated_at)}
                    </div>
                    <div className="sm-popover-row">
                      <Icon name="search" size={14} /> Read {plural(home.index.indexed_transcription_count, "dictation")}
                    </div>
                    <div className="sm-popover-row" style={{ color: home.stale ? "var(--warn)" : "var(--ok)" }}>
                      <Glyph name={home.stale ? "rotate" : "checkCircle"} size={14} />
                      {home.stale ? "New dictation since. Regenerate to include it." : "Up to date with your dictation"}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
          {analyzer && (
            <div className="ip-model">
              {analyzer}
              {home?.stale && <span className="ip-stale-dot" title="New dictation since this ran" />}
            </div>
          )}
        </div>
        {enabled && (
          <Button size="sm" icon="refresh" loading={generating !== null} onClick={regenerate} title="Analyze your dictation history again">
            {home?.report_generated_at ? "Regenerate" : "Generate"}
          </Button>
        )}
        {onClose && <IconButton name="x" label="Close insights panel" onClick={onClose} />}
      </header>

      <div className="ip-body">
        {home && home.stats.length > 0 && <InsightStatTicker stats={home.stats} scanning={generating !== null} />}
        {generating !== null && <ProgressRow startedAt={generating} count={home?.index.indexed_transcription_count ?? null} />}
        {error && <div className="ip-error">{error}</div>}

        {!settings ? null : !enabled ? (
          <div className="ip-card">
            <div className="ip-card-label">
              <Glyph name="brain" size={14} /> No insights yet
            </div>
            <div className="ip-pattern-detail">
              Turn on semantic memory to see open loops, rhythms and patterns from your dictation. It runs on this computer.
            </div>
            <div>
              <Button
                size="sm"
                variant="primary"
                onClick={() =>
                  patch({ semantic_enabled: true, semantic_auto_index: true })
                    .then(() => toast("Semantic memory is on. Indexing your dictations…", "success"))
                    .catch((e) => toast(String(e), "error"))
                }
              >
                Turn On
              </Button>
            </div>
          </div>
        ) : !home ? null : findings.length === 0 && !summary ? (
          generating === null && (
            <div className="ip-card">
              <div className="ip-card-label">
                <Glyph name="brain" size={14} /> No insights yet
              </div>
              <div className="ip-pattern-detail">
                {home.index.chunk_count === 0
                  ? "Your dictations are being indexed. Patterns appear after a few more dictations."
                  : "Generate an analysis of what your dictation adds up to."}
              </div>
            </div>
          )
        ) : (
          <>
            {summary && (
              <div className="ip-card">
                <div className="ip-card-label">
                  <Glyph name="quote" size={14} /> Summary
                </div>
                <div className="ip-summary">{summary}</div>
              </div>
            )}
            {patterns.length > 0 && (
              <div className="ip-card">
                <div className="ip-card-label">
                  <Icon name="graph" size={14} /> Patterns
                </div>
                {patterns.map((f) => (
                  <div className="ip-pattern" key={f.id} title={`${f.label} · ${Math.round(f.confidence * 100)}% confidence`}>
                    <span className="sm-dot" />
                    <div>
                      <div className="ip-pattern-title">
                        {f.kind === "open_commitment" || f.kind === "open_question"
                          ? `${f.label} · ${f.title.toLowerCase()}`
                          : capitalize(f.title)}
                      </div>
                      <div className="ip-pattern-detail">{f.detail}</div>
                    </div>
                  </div>
                ))}
              </div>
            )}
            {(keep.length > 0 || next.length > 0) && (
              <div className="ip-card">
                <div className="ip-card-label">
                  <Glyph name="bulb" size={14} /> Takeaways
                </div>
                {keep.map((f) => (
                  <div className="ip-takeaway keep" key={`k-${f.id}`} title={`Keep doing: ${f.action}`}>
                    <Glyph name="checkCircle" size={14} />
                    <span>{f.action}</span>
                  </div>
                ))}
                {next.map((f) => (
                  <div className="ip-takeaway next" key={`n-${f.id}`} title={`Try next: ${f.action}`}>
                    <Glyph name="up" size={14} />
                    <span>
                      {f.action}
                      {(f.kind === "open_commitment" || f.kind === "open_question") && (
                        <span className="ip-pattern-detail" style={{ display: "block" }}>
                          {f.detail.length > 90 ? `${f.detail.slice(0, 89)}…` : f.detail}
                        </span>
                      )}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </>
        )}

        {status && <div className="ip-status">{status}</div>}
        {enabled && (
          <div className="ip-foot">
            <span className="ip-status">
              {home ? `${plural(home.index.indexed_transcription_count, "dictation")} in memory` : ""}
            </span>
            <button className="sm-link" onClick={openGraph}>
              Open Memory Graph →
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

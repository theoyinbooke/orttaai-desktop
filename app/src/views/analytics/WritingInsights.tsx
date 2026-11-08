// Writing Insights (Mac HomeInsightsPanel + WritingInsightsService). Owned by
// the ai area; a self-contained component the analytics/home areas embed.
// A local model reads your dictation history and writes a summary, patterns,
// signals and takeaways; a deterministic reading stands in when no model is
// set up. Runs are saved: load, pin, compare and delete them.
import { useCallback, useEffect, useRef, useState } from "react";
import { Button, ConfirmDialog, Icon, IconButton, Segmented, Spinner, useToast } from "../../ui";
import { useEvent } from "../../lib/api";
import {
  DEFAULT_INSIGHTS_REQUEST,
  dateTime,
  relativeTime,
  writing,
  type AppFilterMode,
  type InsightFreshness,
  type InsightHistoryItem,
  type InsightRecommendation,
  type InsightsComparison,
  type TimeRange,
  type WritingInsightSnapshot,
  type WritingInsightsInfo,
  type WritingInsightsRequest,
} from "../../lib/ai";
import "../../styles/ai.css";

const RANGES: { value: TimeRange; label: string }[] = [
  { value: "days7", label: "7d" },
  { value: "days14", label: "14d" },
  { value: "days30", label: "30d" },
  { value: "days90", label: "90d" },
  { value: "all", label: "All" },
];

const FILTER_TITLE: Record<AppFilterMode, string> = { all_apps: "All apps", include_only: "Include", exclude: "Exclude" };

function progressLine(elapsed: number, sampleCount: number | null): string {
  if (elapsed < 3)
    return sampleCount != null ? `Reading your last ${sampleCount.toLocaleString()} dictations` : "Reading your recent dictations";
  if (elapsed < 10) return "Finding patterns in how you write";
  if (elapsed < 25) return "Writing your summary";
  if (elapsed < 60) return "Still working. Larger models take a little longer";
  return "Almost there";
}

const lsKey = "writingInsightsRequest";
function loadRequest(): WritingInsightsRequest {
  try {
    const raw = localStorage.getItem(lsKey);
    if (raw) return { ...DEFAULT_INSIGHTS_REQUEST, ...JSON.parse(raw) };
  } catch {
    /* storage unavailable */
  }
  return DEFAULT_INSIGHTS_REQUEST;
}

export default function WritingInsights({ compact = false }: { compact?: boolean }) {
  const toast = useToast();
  const [request, setRequestState] = useState<WritingInsightsRequest>(loadRequest);
  const [snapshot, setSnapshot] = useState<WritingInsightSnapshot | null>(null);
  const [snapshotId, setSnapshotId] = useState<number | null>(null);
  const [preliminary, setPreliminary] = useState(false);
  const [history, setHistory] = useState<InsightHistoryItem[]>([]);
  const [freshness, setFreshness] = useState<InsightFreshness | null>(null);
  const [info, setInfo] = useState<WritingInsightsInfo | null>(null);
  const [apps, setApps] = useState<string[]>([]);
  const [generating, setGenerating] = useState(false);
  const [startedAt, setStartedAt] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [compareIds, setCompareIds] = useState<number[]>([]);
  const [comparison, setComparison] = useState<InsightsComparison | null>(null);
  const [deleting, setDeleting] = useState<InsightHistoryItem | null>(null);
  const autoRefreshed = useRef<number | null>(null);
  const snapshotRef = useRef<WritingInsightSnapshot | null>(null);
  snapshotRef.current = snapshot;

  const setRequest = (r: WritingInsightsRequest) => {
    setRequestState(r);
    try {
      localStorage.setItem(lsKey, JSON.stringify(r));
    } catch {
      /* storage unavailable */
    }
  };

  const loadHistory = useCallback(() => writing.history(24).then(setHistory).catch(() => {}), []);

  const generate = useCallback(
    async (req: WritingInsightsRequest) => {
      if (generating) return;
      setGenerating(true);
      setStartedAt(Date.now());
      setError(null);
      setStatus(null);
      // A quick, non-AI reading while the analysis runs.
      writing
        .quick(req)
        .then((q) => {
          if (q && !snapshotRef.current) {
            setSnapshot(q);
            setPreliminary(true);
          }
        })
        .catch(() => {});
      try {
        const run = await writing.generate(req);
        if (run.snapshot) {
          setSnapshot(run.snapshot);
          setSnapshotId(run.snapshot_id);
          setPreliminary(false);
          setStatus(run.used_fallback ? `${run.analyzer_name} stood in: the local model didn't answer.` : null);
          if (run.persistence_warning) setError(run.persistence_warning);
          setFreshness(await writing.freshness(run.snapshot.generated_at));
        } else if (run.error) {
          setError(run.error);
          setPreliminary(false);
        } else {
          setSnapshot(null);
          setPreliminary(false);
          setStatus("No dictations in this range yet.");
        }
        loadHistory();
      } catch (e) {
        setError(String(e));
      } finally {
        setGenerating(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [generating, loadHistory],
  );

  useEffect(() => {
    writing.info().then(setInfo).catch(() => {});
    writing.apps().then(setApps).catch(() => {});
    loadHistory();
    writing
      .latest()
      .then(async (latest) => {
        if (latest) {
          setSnapshot(latest.snapshot);
          setSnapshotId(latest.id);
          const f = await writing.freshness(latest.snapshot.generated_at);
          setFreshness(f);
          if (f.should_auto_refresh && autoRefreshed.current !== latest.id) {
            autoRefreshed.current = latest.id;
            setStatus(`Found ${f.new_session_count} new sessions since this insight. Regenerating...`);
            generate(request);
          }
        } else {
          generate(request);
        }
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEvent("settings-changed", () => writing.info().then(setInfo).catch(() => {}));
  useEvent("history-changed", () => {
    if (snapshot) writing.freshness(snapshot.generated_at).then(setFreshness).catch(() => {});
  });

  useEffect(() => {
    if (!generating) return;
    const t = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(t);
  }, [generating]);

  useEffect(() => {
    if (compareIds.length !== 2) {
      setComparison(null);
      return;
    }
    writing.compare(compareIds[0], compareIds[1]).then(setComparison).catch(() => setComparison(null));
  }, [compareIds]);

  const load = async (item: InsightHistoryItem) => {
    setSnapshot(item.snapshot);
    setSnapshotId(item.id);
    setPreliminary(false);
    setRequest(item.snapshot.request);
    setFreshness(await writing.freshness(item.snapshot.generated_at).catch(() => null));
  };

  const toggleCompare = (id: number) =>
    setCompareIds((ids) => (ids.includes(id) ? ids.filter((x) => x !== id) : [...ids.slice(-1), id]));

  const apply = (rec: InsightRecommendation) =>
    writing
      .apply(rec)
      .then(() => toast(rec.kind === "dictionary" ? "Added to Dictionary" : "Added as Snippet", "success"))
      .catch((e) => toast(String(e), "error"));

  const analyzer = (name: string) => name.replace("Apple Foundation Models", "Apple Intelligence");

  return (
    <section className={`card wi ${compact ? "compact" : ""}`} aria-label="Writing insights panel">
      <header className="wi-head">
        <div className="wi-head-title">
          <Icon name="sparkle" size={17} />
          <h3>Insights</h3>
          {snapshot && (
            <button className="wi-model" onClick={() => setAboutOpen((o) => !o)} title="About this insight">
              {analyzer(snapshot.analyzer_name)}
              {(freshness?.new_session_count ?? 0) > 0 && !preliminary && (
                <span className="wi-dot" aria-label="Newer dictation since this ran" />
              )}
            </button>
          )}
        </div>
        <div className="wi-head-actions">
          <IconButton name="filter" label="Options" onClick={() => setOptionsOpen((o) => !o)} />
          <IconButton name="history" label="Recent runs" onClick={() => setHistoryOpen((o) => !o)} />
          <Button size="sm" icon="refresh" loading={generating} onClick={() => generate(request)} title="Analyze your dictation history again">
            {snapshot ? "Regenerate" : "Generate"}
          </Button>
        </div>
      </header>

      {generating && (
        <div className="wi-progress" role="status">
          <Spinner size={13} />
          <span>{progressLine((now - startedAt) / 1000, snapshot?.sample_count ?? null)}</span>
        </div>
      )}
      {status && !generating && <p className="muted small">{status}</p>}
      {error && (
        <div className="ai-notice warn" role="status">
          <Icon name="alert" size={15} />
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError(null)}>
            <Icon name="x" size={13} />
          </button>
        </div>
      )}

      {optionsOpen && (
        <div className="wi-options">
          <div className="wi-option">
            <span className="overline">Range</span>
            <Segmented value={request.time_range} options={RANGES} onChange={(v) => setRequest({ ...request, time_range: v })} />
          </div>
          <div className="wi-option">
            <span className="overline">Mode</span>
            <Segmented
              value={request.generation_mode}
              options={[
                { value: "balanced", label: "Balanced" },
                { value: "deep", label: "Deep" },
              ]}
              onChange={(v) => setRequest({ ...request, generation_mode: v })}
            />
          </div>
          <div className="wi-option">
            <span className="overline">Apps</span>
            <Segmented
              value={request.app_filter_mode}
              options={[
                { value: "all_apps", label: "All apps" },
                { value: "include_only", label: "Include" },
                { value: "exclude", label: "Exclude" },
              ]}
              onChange={(v) => setRequest({ ...request, app_filter_mode: v })}
            />
          </div>
          {request.app_filter_mode !== "all_apps" && (
            <div className="wi-apps">
              {apps.length === 0 && <span className="muted small">No recent apps</span>}
              {apps.map((a) => {
                const on = request.selected_apps.some((s) => s.toLowerCase() === a.toLowerCase());
                return (
                  <button
                    key={a}
                    className={`ai-chip toggle ${on ? "on" : ""}`}
                    onClick={() =>
                      setRequest({
                        ...request,
                        selected_apps: on
                          ? request.selected_apps.filter((s) => s.toLowerCase() !== a.toLowerCase())
                          : [...request.selected_apps, a],
                      })
                    }
                  >
                    {a}
                  </button>
                );
              })}
              {request.selected_apps.length > 0 && (
                <button className="link-btn small" onClick={() => setRequest({ ...request, selected_apps: [] })}>
                  Clear
                </button>
              )}
            </div>
          )}
          {info && (
            <p className="muted small">
              {info.uses_llm
                ? `Analyzed by ${info.analyzer_name} with ${info.models.join(", then ")}.`
                : "No local model is set for insights, so a quick on-device reading is used. Turn on Writing Insights in Model › AI Features for a model's analysis."}
            </p>
          )}
        </div>
      )}

      {aboutOpen && snapshot && (
        <div className="wi-about">
          <h4>About this insight</h4>
          <p><Icon name="models" size={13} /> Analysed by {analyzer(snapshot.analyzer_name)}</p>
          <p>
            <Icon name="history" size={13} />{" "}
            {preliminary ? "Quick look now; the AI analysis is running" : `Ran ${relativeTime(snapshot.generated_at)}, ${dateTime(snapshot.generated_at)}`}
          </p>
          <p><Icon name="search" size={13} /> Read {snapshot.sample_count === 1 ? "1 dictation" : `${snapshot.sample_count.toLocaleString()} dictations`}</p>
          <p>
            <Icon name="filter" size={13} />{" "}
            {RANGES.find((r) => r.value === snapshot.request.time_range)?.label} ·{" "}
            {snapshot.request.generation_mode === "deep" ? "Deep" : "Balanced"} · {FILTER_TITLE[snapshot.request.app_filter_mode]}
          </p>
          {freshness && !preliminary &&
            (freshness.new_session_count > 0 ? (
              <p className="warn-text">
                <Icon name="plus" size={13} /> {freshness.new_session_count} new {freshness.new_session_count === 1 ? "dictation" : "dictations"} since. Regenerate to include them.
              </p>
            ) : (
              <p className="ok-text">
                <Icon name="check" size={13} /> Up to date with your dictation
              </p>
            ))}
        </div>
      )}

      {historyOpen && (
        <div className="wi-history">
          <div className="ai-card-head">
            <h4>Recent runs</h4>
            {compareIds.length > 0 && (
              <span className="muted small">{compareIds.length === 1 ? "Pick one more to compare" : "Comparing two runs"}</span>
            )}
          </div>
          {history.length === 0 && <p className="muted small">No saved runs yet.</p>}
          {history.map((h) => {
            const loaded = h.id === snapshotId;
            const inCompare = compareIds.includes(h.id);
            return (
              <div key={h.id} className={`ai-history-row ${loaded ? "active" : ""}`}>
                <div className="ai-history-text">
                  <span>
                    {h.is_pinned && <Icon name="pin" size={12} />} {dateTime(h.snapshot.generated_at)}
                  </span>
                  <span className="muted small">
                    {h.snapshot.sample_count} sessions • {analyzer(h.snapshot.analyzer_name)}
                  </span>
                </div>
                <div className="ai-history-actions">
                  <Button size="sm" disabled={loaded} onClick={() => load(h)}>
                    {loaded ? "Loaded" : "Load"}
                  </Button>
                  <Button size="sm" onClick={() => writing.pin(h.id, !h.is_pinned).then(loadHistory).catch((e) => toast(String(e), "error"))}>
                    {h.is_pinned ? "Unpin" : "Pin"}
                  </Button>
                  <Button size="sm" variant={inCompare ? "subtle" : "ghost"} onClick={() => toggleCompare(h.id)}>
                    {inCompare ? "Remove from Compare" : "Add to Compare"}
                  </Button>
                  <IconButton name="trash" label="Delete" onClick={() => setDeleting(h)} />
                </div>
              </div>
            );
          })}
          {comparison && (
            <div className="wi-compare">
              <div className="ai-card-head">
                <h4>Comparison</h4>
                <button className="link-btn small" onClick={() => setCompareIds([])}>
                  Clear
                </button>
              </div>
              <p className="muted small">{comparison.headline}</p>
              <ul>
                {comparison.bullets.map((b) => (
                  <li key={b}>• {b}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}

      {snapshot ? (
        <div className={`wi-body ${preliminary ? "preliminary" : ""}`}>
          <div className="wi-summary">
            <span className="overline">Summary</span>
            <p>{snapshot.summary}</p>
          </div>
          {snapshot.signals.length > 0 && (
            <div className="wi-signals">
              {snapshot.signals.map((s) => (
                <div key={s.label} className="wi-signal" title={s.detail}>
                  <div className="stat-value">{s.value}</div>
                  <div className="stat-label">{s.label}</div>
                </div>
              ))}
            </div>
          )}
          {snapshot.patterns.length > 0 && (
            <div className="wi-block">
              <span className="overline">Patterns</span>
              <div className="wi-patterns">
                {snapshot.patterns.map((p) => (
                  <Pattern key={p.title} title={p.title} detail={p.detail} evidence={p.evidence} />
                ))}
              </div>
            </div>
          )}
          {(snapshot.strengths.length > 0 || snapshot.opportunities.length > 0) && (
            <div className="wi-block">
              <span className="overline">Takeaways</span>
              <ul className="wi-takeaways">
                {snapshot.strengths.map((s) => (
                  <li key={`k-${s}`} title={`Keep doing: ${s}`}>
                    <span className="ok-text"><Icon name="check" size={14} /></span>
                    <span>{s}</span>
                  </li>
                ))}
                {snapshot.opportunities.map((s) => (
                  <li key={`t-${s}`} title={`Try next: ${s}`}>
                    <span className="accent-text"><Icon name="target" size={14} /></span>
                    <span>{s}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {snapshot.recommendations.length > 0 && (
            <div className="wi-block">
              <span className="overline">Suggestions</span>
              {snapshot.recommendations.map((r) => (
                <div key={`${r.kind}|${r.source}|${r.target}`} className="wi-rec">
                  <span>
                    <strong>{r.source}</strong> → {r.target}
                    <span className="muted small"> {r.rationale}</span>
                  </span>
                  <Button size="sm" onClick={() => apply(r)}>
                    {r.kind === "dictionary" ? "Add to Dictionary" : "Add as Snippet"}
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        !generating && (
          <div className="wi-empty">
            <Icon name="insights" size={22} />
            <strong>No insights yet</strong>
            <p className="muted small">Generate an analysis of how you write, from your dictation history.</p>
          </div>
        )
      )}

      <ConfirmDialog
        open={deleting != null}
        title="Delete this insight run?"
        body="The saved analysis will be removed. Your dictations stay."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const d = deleting;
          setDeleting(null);
          if (!d) return;
          writing
            .remove(d.id)
            .then(() => {
              setCompareIds((ids) => ids.filter((x) => x !== d.id));
              if (d.id === snapshotId) {
                setSnapshot(null);
                setSnapshotId(null);
              }
              loadHistory();
            })
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </section>
  );
}

function Pattern({ title, detail, evidence }: { title: string; detail: string; evidence: string | null }) {
  const [open, setOpen] = useState(false);
  return (
    <button className={`wi-pattern ${open ? "open" : ""}`} onClick={() => setOpen((o) => !o)} aria-expanded={open}>
      <span className="wi-pattern-title">{title}</span>
      <span className="wi-pattern-detail">{detail}</span>
      {open && evidence && <span className="wi-pattern-evidence">{evidence}</span>}
    </button>
  );
}

// Tone of Voice (Mac ToneOfVoiceView). Owned by the ai area.
// How you sound: a voice-match score, tone and style metrics, language stats,
// signature phrases, and the prompt guide ChatAI's My Tone mode writes with.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Button, ConfirmDialog, Icon, Select, Spinner, useToast } from "../../ui";
import { call, useEvent } from "../../lib/api";
import type { HistoryPage } from "../../lib/analytics";
import { useSettings } from "../../lib/settings";
import { ai, dateTime, tone, writing, type ToneHistoryItem, type ToneMetric, type ToneProfile } from "../../lib/ai";
import "../../styles/ai.css";

type Section = "overview" | "style" | "language" | "guide";

const SECTIONS: { value: Section; label: string }[] = [
  { value: "overview", label: "Overview" },
  { value: "style", label: "Tone & Style" },
  { value: "language", label: "Language" },
  { value: "guide", label: "Guide" },
];

/** 24×24 stroke look-alikes for the SF Symbols this page uses. */
const GLYPHS: Record<string, ReactNode> = {
  // checkmark.circle
  recommend: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m8 12.5 2.8 2.8L16 9.5" />
    </>
  ),
  // hand.thumbsup
  thumbsup: <path d="M7.5 10.5v9H4.2a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1h3.3Zm0 0 3.6-7c1.6 0 2.7 1.2 2.7 2.8V9.5h5a2 2 0 0 1 2 2.4l-1.2 6a2 2 0 0 1-2 1.6H7.5" />,
  // hand.raised
  raised: <path d="M8 12V5.2a1.5 1.5 0 0 1 3 0V11m0-.5V3.7a1.5 1.5 0 0 1 3 0V11m0-6a1.5 1.5 0 0 1 3 0v7m0-3.5a1.5 1.5 0 0 1 3 0V15a6.5 6.5 0 0 1-6.5 6.5h-1.3a6 6 0 0 1-4.6-2.1L3.4 15a1.6 1.6 0 0 1 2.3-2.2L8 15" />,
  // text.quote
  quote: <path d="M5 6h15M9 11h11M9 15h11M5 20h15M5 10.5v5" />,
  // building.columns
  columns: <path d="M3.5 9.5 12 4l8.5 5.5h-17ZM6 12.5v5M12 12.5v5M18 12.5v5M3.5 20.5h17" />,
  // heart
  heart: <path d="M12 20.5s-8.5-5-8.5-11A4.8 4.8 0 0 1 12 6.6a4.8 4.8 0 0 1 8.5 2.9c0 6-8.5 11-8.5 11Z" />,
  // arrow.right.circle
  direct: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M7.5 12h9M13 8.5l3.5 3.5-3.5 3.5" />
    </>
  ),
  // flame
  flame: <path d="M12 3c1 3.5 5 5.5 5 10a5 5 0 0 1-10 0c0-2.2 1-3.6 2.2-4.6.2 1.6 1 2.6 2.1 3C11 9.2 11 6 12 3Z" />,
  // square.stack.3d.up
  stack: <path d="M12 3 3 7.5l9 4.5 9-4.5L12 3ZM3 12l9 4.5 9-4.5M3 16.5 12 21l9-4.5" />,
  // bubble.left.and.bubble.right
  bubbles: <path d="M15.5 4.5h-11A1.5 1.5 0 0 0 3 6v6.5A1.5 1.5 0 0 0 4.5 14H6v3l3.5-3h6a1.5 1.5 0 0 0 1.5-1.5V6a1.5 1.5 0 0 0-1.5-1.5ZM20 9.5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-1.2V20l-3-2.5H12" />,
  // checkmark.seal
  seal: <path d="m12 2.8 2.5 1.8 3-.2 1 2.9 2.5 1.8-.9 2.9.9 2.9-2.5 1.8-1 2.9-3-.2L12 21.2l-2.5-1.8-3 .2-1-2.9L3 14.9l.9-2.9L3 9.1l2.5-1.8 1-2.9 3 .2L12 2.8ZM8.5 12l2.5 2.5 4.5-5" />,
  // chart.bar
  bars: <path d="M3.5 20.5h17M6 17.5v-6M12 17.5V5.5M18 17.5v-9" />,
};

function Glyph({ name, size = 13, weight = 2 }: { name: string; size?: number; weight?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={weight} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {GLYPHS[name]}
    </svg>
  );
}

const METRIC_GLYPH: Record<string, string> = {
  Formality: "columns",
  Warmth: "heart",
  Directness: "direct",
  Enthusiasm: "flame",
  Complexity: "stack",
  Conversation: "bubbles",
};

const METRIC_TINT: Record<string, string> = {
  Formality: "var(--accent)",
  Warmth: "#e86f51",
  Directness: "#e0b14a",
  Enthusiasm: "#f97316",
  Complexity: "#a855f7",
  Conversation: "var(--ok)",
};

export default function ToneOfVoice() {
  const toast = useToast();
  const { settings } = useSettings();
  const [section, setSection] = useState<Section>("overview");
  const [item, setItem] = useState<ToneHistoryItem | null>(null);
  const [history, setHistory] = useState<ToneHistoryItem[]>([]);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadingModels, setLoadingModels] = useState(true);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [deleting, setDeleting] = useState<ToneHistoryItem | null>(null);

  const defaultModel = (settings?.insights_model || "").trim() || "gemma4:e2b";

  const loadHistory = useCallback(() => {
    tone.history().then(setHistory).catch(() => {});
  }, []);

  const refreshModels = useCallback(() => {
    setLoadingModels(true);
    ai.models()
      .then((m) => {
        const names = m.filter((x) => !x.embedding).map((x) => x.name).sort();
        setModels(names);
        setModel((cur) => {
          if (cur && names.includes(cur)) return cur;
          if (names.includes(defaultModel)) return defaultModel;
          return names[0] ?? cur ?? defaultModel;
        });
        setError(null);
      })
      .catch(() => {
        setModels([]);
        setModel((cur) => cur || defaultModel);
        setError(`The local AI server isn't reachable. Local metrics can still run.`);
      })
      .finally(() => setLoadingModels(false));
  }, [defaultModel]);

  useEffect(() => {
    tone.latest().then(setItem).catch(() => {});
    loadHistory();
    refreshModels();
  }, [loadHistory, refreshModels]);

  // The first visit with dictations to read but no profile analyzes straight
  // away, once the model list is in, as on the Mac.
  const autoRan = useRef(false);
  useEffect(() => {
    if (autoRan.current || loadingModels || !model) return;
    autoRan.current = true;
    (async () => {
      const latest = await tone.latest().catch(() => null);
      if (latest) return;
      const page = await call<HistoryPage>("history_page", { query: { limit: 1, offset: 0 } }).catch(() => null);
      if (page && page.total > 0) runRef.current();
    })();
  }, [loadingModels, model]);

  useEvent("tone-updated", () => {
    tone.latest().then(setItem).catch(() => {});
    loadHistory();
  });

  const run = async () => {
    if (loading) return;
    setLoading(true);
    setStatus("Analyzing tone of voice...");
    setError(null);
    try {
      const r = await tone.analyze(model);
      if (!r) {
        setStatus(null);
        setError("No writing history is available yet. Dictate a few samples, then run tone analysis.");
        return;
      }
      setItem({ id: r.snapshot_id ?? -1, is_pinned: false, profile: r.profile });
      setStatus(r.used_llm ? `Tone profile updated with ${r.profile.model}.` : "Tone profile updated from local metrics.");
      setError(r.error);
      loadHistory();
    } catch (e) {
      setStatus(null);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const runRef = useRef(run);
  runRef.current = run;

  const profile = item?.profile ?? null;
  const modelOptions = [...new Set([model, ...models].filter(Boolean))].map((m) => ({ value: m, label: m }));

  return (
    <div className="tone">
      <div className="tone-controls">
        <div className="tone-seg" role="radiogroup" aria-label="Section">
          {SECTIONS.map((o) => (
            <button key={o.value} role="radio" aria-checked={section === o.value} className={section === o.value ? "active" : ""} onClick={() => setSection(o.value)}>
              {o.label}
            </button>
          ))}
        </div>
        <span className="tone-controls-spacer" />
        {status && !loading && <span className="tone-status">{status}</span>}
        <Select value={model} onChange={setModel} options={modelOptions} placeholder="Choose model" ariaLabel="The model that analyzes your voice" className="tone-model" />
        <Button size="sm" className="tone-icon-btn" icon="refresh" aria-label="Refresh the model list" title="Refresh the model list" onClick={refreshModels} disabled={loadingModels} />
        <Button size="sm" className="tone-icon-btn" icon="history" aria-label="Past profiles" title="Past profiles" onClick={() => setShowHistory((s) => !s)} />
        <Button variant="primary" size="sm" loading={loading} onClick={run} title="Analyze your recent dictation again">
          {loading ? "Analyzing…" : profile ? "Rerun" : "Analyze"}
        </Button>
      </div>

      {error && (
        <div className="ai-notice warn" role="status">
          <Icon name="alert" size={15} />
          <span>{error}</span>
          <button aria-label="Dismiss" onClick={() => setError(null)}>
            <Icon name="x" size={13} />
          </button>
        </div>
      )}

      {showHistory && (
        <div className="card tone-card tone-history">
          <div className="ai-card-head">
            <h3>Past profiles</h3>
            <span className="muted small">Pinned profiles stay at the top.</span>
          </div>
          {history.length === 0 && <p className="muted small">No saved profiles yet.</p>}
          {history.map((h) => (
            <div key={h.id} className={`ai-history-row ${item?.id === h.id ? "active" : ""}`}>
              <div className="ai-history-text">
                <span>
                  {h.is_pinned && <Icon name="pin" size={12} />} {dateTime(h.profile.generated_at)}
                </span>
                <span className="muted small">
                  {h.profile.overall_score} voice match · {h.profile.model || "local metrics"} · {h.profile.sample_count.toLocaleString()} dictations
                </span>
              </div>
              <div className="ai-history-actions">
                <Button size="sm" onClick={() => setItem(h)} disabled={item?.id === h.id}>
                  {item?.id === h.id ? "Loaded" : "Load"}
                </Button>
                <Button
                  size="sm"
                  onClick={() =>
                    writing
                      .pin(h.id, !h.is_pinned)
                      .then(loadHistory)
                      .catch((e) => toast(String(e), "error"))
                  }
                >
                  {h.is_pinned ? "Unpin" : "Pin"}
                </Button>
                <Button size="sm" className="tone-icon-btn" icon="trash" aria-label="Delete" title="Delete" onClick={() => setDeleting(h)} />
              </div>
            </div>
          ))}
        </div>
      )}

      {loading && !profile ? (
        <div className="card tone-card tone-loading">
          <Spinner /> <span>Analyzing your tone of voice…</span>
        </div>
      ) : profile ? (
        <>
          {section === "overview" && <Overview p={profile} />}
          {section === "style" && <Style p={profile} />}
          {section === "language" && <Language p={profile} />}
          {section === "guide" && <Guide p={profile} />}
        </>
      ) : (
        <div className="card tone-card tone-empty">
          <h3>No tone profile yet</h3>
          <p>Analyze your dictation history to see how you sound and get a guide ChatAI can write in.</p>
        </div>
      )}

      <ConfirmDialog
        open={deleting != null}
        title="Delete this profile?"
        body="This saved tone profile will be removed. Your dictations stay."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const d = deleting;
          setDeleting(null);
          if (!d) return;
          writing
            .remove(d.id)
            .then(() => {
              if (item?.id === d.id) tone.latest().then(setItem).catch(() => {});
              loadHistory();
            })
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </div>
  );
}

function pct(v: number) {
  return Math.round(v * 100);
}

/** The Mac's InfoButton: an info icon with a tooltip, and a popover on click. */
function InfoButton({ title, text }: { title: string; text: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  return (
    <span className="tone-info" ref={ref}>
      <button onClick={() => setOpen((o) => !o)} title={text} aria-label={`About ${title}`} aria-expanded={open}>
        <Icon name="infoCircle" size={12} />
      </button>
      {open && <span className="tone-info-pop">{text}</span>}
    </span>
  );
}

function Label({ title, info, className = "" }: { title: string; info?: string; className?: string }) {
  return (
    <span className={`tone-label ${className}`}>
      <span className="tone-label-text">{title}</span>
      {info && <InfoButton title={title} text={info} />}
    </span>
  );
}

/** The Mac's SettingsCard: an optional title line with an accessory, then the content. */
function ToneCard({ title, info, accessory, children, className = "" }: { title?: string; info?: string; accessory?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={`card tone-card ${className}`}>
      {title && (
        <div className="tone-card-head">
          <Label title={title} info={info} className="heading" />
          <span className="tone-controls-spacer" />
          {accessory}
        </div>
      )}
      {children}
    </div>
  );
}

function ScoreRing({ score }: { score: number }) {
  const r = 47;
  const c = 2 * Math.PI * r;
  const v = Math.max(0, Math.min(100, score));
  return (
    <svg className="tone-ring" viewBox="0 0 104 104" role="img" aria-label={`Voice match ${score} out of 100`}>
      <circle cx="52" cy="52" r={r} className="tone-ring-track" />
      <circle cx="52" cy="52" r={r} className="tone-ring-value" strokeDasharray={`${(v / 100) * c} ${c}`} transform="rotate(-90 52 52)" />
      <text x="52" y="48" className="tone-ring-score" textAnchor="middle" dominantBaseline="central">
        {score}
      </text>
      <text x="52" y="72" className="tone-ring-label" textAnchor="middle">
        voice match
      </text>
    </svg>
  );
}

function Chips({ values, tint = "var(--accent)" }: { values: string[]; tint?: string }) {
  return (
    <div className="ai-chips">
      {values.map((v) => (
        <span key={v} className="ai-chip" style={{ color: tint, background: `color-mix(in srgb, ${tint} 12%, transparent)` }}>
          {v}
        </span>
      ))}
    </div>
  );
}

function ListCard({ title, icon, values, tint }: { title: string; icon: string; values: string[]; tint: string }) {
  return (
    <div className="card tone-card tone-list">
      <h3 className="tone-list-title">
        <span style={{ color: tint }}>
          <Glyph name={icon} />
        </span>
        {title}
      </h3>
      {values.length === 0 && <p className="tone-foot">Nothing yet. Rerun after more dictation.</p>}
      {values.slice(0, 5).map((v) => (
        <p key={v} className="tone-list-item">
          <span className="ai-dot" style={{ background: tint }} />
          <span>{v}</span>
        </p>
      ))}
    </div>
  );
}

function metaLine(p: ToneProfile) {
  return [
    `${Math.round(Math.max(0, Math.min(1, p.confidence)) * 100)}% confidence`,
    `${p.word_count.toLocaleString()} words from ${p.sample_count.toLocaleString()} dictations`,
    p.model || "local metrics",
    `updated ${new Date(p.generated_at * 1000).toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" })}`,
  ].join(" · ");
}

function Overview({ p }: { p: ToneProfile }) {
  return (
    <div className="tone-stack">
      <ToneCard className="tone-hero">
        <ScoreRing score={p.overall_score} />
        <div className="tone-hero-text">
          <Chips values={p.descriptors.slice(0, 6).map((d) => d.charAt(0).toUpperCase() + d.slice(1))} />
          <p className="tone-summary">{p.summary}</p>
          <p className="tone-meta">{metaLine(p)}</p>
        </div>
      </ToneCard>
      {p.signature_phrases.length > 0 && (
        <ToneCard title="Signature phrases" info="Phrases you use often enough that they sound like you.">
          <div className="tone-pad">
            <Chips values={p.signature_phrases.slice(0, 10).map((s) => `“${s}”`)} />
          </div>
        </ToneCard>
      )}
      <div className="tone-row">
        <ListCard title="Recommendations" icon="recommend" values={p.recommendations} tint="var(--warn)" />
        <ListCard title="Use this voice" icon="thumbsup" values={p.signature_approaches} tint="var(--ok)" />
        <ListCard title="Avoid" icon="raised" values={p.avoidances} tint="var(--danger)" />
      </div>
    </div>
  );
}

function MetricRow({ m }: { m: ToneMetric }) {
  const tint = METRIC_TINT[m.name] ?? "var(--accent)";
  return (
    <div className="tone-metric">
      <span className="tone-metric-icon" style={{ color: tint }}>
        <Glyph name={METRIC_GLYPH[m.name] ?? "bars"} size={13} />
      </span>
      <Label title={m.name} info={m.detail || undefined} className="tone-metric-name" />
      <span className="tone-metric-label" style={{ color: tint }}>
        {m.label}
      </span>
      <span className="tone-bar">
        <span style={{ width: `max(6px, ${pct(Math.max(0, Math.min(1, m.value)))}%)`, background: tint }} />
      </span>
      <span className="tone-metric-pct">{pct(m.value)}%</span>
    </div>
  );
}

function Style({ p }: { p: ToneProfile }) {
  const adequate = p.word_count >= 650;
  return (
    <ToneCard
      title="Tone and style"
      info="How your dictation reads on each dimension, measured across your recent history."
      accessory={
        <span
          className="tone-status-chip"
          style={{ color: adequate ? "var(--ok)" : "var(--warn)", background: `color-mix(in srgb, ${adequate ? "var(--ok)" : "var(--warn)"} 10%, transparent)` }}
          title={adequate ? `Based on ${p.word_count.toLocaleString()} words, an adequate sample.` : "More dictation will make this more reliable."}
        >
          <Glyph name="seal" size={12} weight={1.8} />
          {Math.round(p.confidence * 100)}% confidence
        </span>
      }
    >
      <div className="tone-metrics">
        {p.metrics.map((m) => (
          <MetricRow key={m.name} m={m} />
        ))}
      </div>
    </ToneCard>
  );
}

function Language({ p }: { p: ToneProfile }) {
  const wps = p.word_count / Math.max(1, p.sentence_count);
  const metric = (n: string) => p.metrics.find((m) => m.name.toLowerCase() === n.toLowerCase())?.value ?? 0.5;
  const complexity = metric("Complexity");
  const conversation = metric("Conversation");
  const grade = Math.min(12, Math.max(3, Math.round(4 + wps / 8 + complexity * 4)));
  const tiles: [string, string, string][] = [
    [`Grade ${grade}`, "Reading level", "An estimate from sentence length and vocabulary."],
    [`~${Math.round(wps)}`, "Words per sentence", "Average sentence length in your dictation."],
    [conversation > 0.68 ? "High" : conversation > 0.38 ? "Medium" : "Low", "Contractions", "How often you use contractions like “it's” and “we'll”."],
    [complexity > 0.68 ? "Layered" : complexity > 0.38 ? "Clear" : "Simple", "Vocabulary", "How varied and complex your word choice is."],
  ];
  return (
    <div className="tone-stack">
      <div className="tone-row four">
        {tiles.map(([v, l, info]) => (
          <div key={l} className="card tone-card tone-tile">
            <span className="tone-tile-value">{v}</span>
            <Label title={l} info={info} className="caption" />
          </div>
        ))}
      </div>
      {p.signature_approaches.length > 0 && <ListCard title="Notable traits" icon="quote" values={p.signature_approaches.slice(0, 5)} tint="var(--accent)" />}
      {p.sample_excerpts.length > 0 && (
        <ToneCard title="In your words" info="Excerpts from your dictation the profile was built on.">
          <div className="tone-excerpts">
            {p.sample_excerpts.map((e) => (
              <blockquote key={e}>{e}</blockquote>
            ))}
          </div>
        </ToneCard>
      )}
    </div>
  );
}

function GuideCopy({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (
    <Button
      size="sm"
      className={copied ? "tone-copied" : ""}
      icon={copied ? "check" : "copy"}
      onClick={() => {
        navigator.clipboard?.writeText(text);
        setCopied(true);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => setCopied(false), 1600);
      }}
    >
      {copied ? "Copied" : "Copy"}
    </Button>
  );
}

function Guide({ p }: { p: ToneProfile }) {
  return (
    <div className="tone-stack">
      <ToneCard title="ChatAI prompt guide" info="What ChatAI's My Tone style sends to the model so replies sound like you." accessory={<GuideCopy text={p.prompt_guide.trim()} />}>
        <pre className="tone-guide">{p.prompt_guide.trim()}</pre>
      </ToneCard>
      <div className="tone-row">
        <ListCard title="Use this voice" icon="thumbsup" values={p.signature_approaches} tint="var(--ok)" />
        <ListCard title="Avoid" icon="raised" values={p.avoidances} tint="var(--danger)" />
        <ListCard title="Recommendations" icon="recommend" values={p.recommendations} tint="var(--warn)" />
      </div>
    </div>
  );
}

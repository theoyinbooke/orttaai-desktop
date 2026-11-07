// Tone of Voice (Mac ToneOfVoiceView). Owned by the ai area.
// How you sound: a voice-match score, tone and style metrics, language stats,
// signature phrases, and the prompt guide ChatAI's My Tone mode writes with.
import { useCallback, useEffect, useState } from "react";
import { Button, ConfirmDialog, CopyButton, Icon, IconButton, Segmented, Select, Spinner, useToast } from "../../ui";
import { useEvent } from "../../lib/api";
import { useSettings } from "../../lib/settings";
import { ai, dateTime, tone, writing, type ToneHistoryItem, type ToneMetric, type ToneProfile } from "../../lib/ai";
import "../../styles/ai.css";

type Section = "overview" | "style" | "language" | "guide";

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
  const [loadingModels, setLoadingModels] = useState(false);
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

  const profile = item?.profile ?? null;
  const modelOptions = [...new Set([model, ...models].filter(Boolean))].map((m) => ({ value: m, label: m }));

  return (
    <div className="tone">
      <div className="tone-controls">
        <Segmented
          value={section}
          onChange={setSection}
          options={[
            { value: "overview", label: "Overview" },
            { value: "style", label: "Tone & Style" },
            { value: "language", label: "Language" },
            { value: "guide", label: "Guide" },
          ]}
        />
        <span className="tone-controls-spacer" />
        {status && !loading && <span className="muted small tone-status">{status}</span>}
        <Select value={model} onChange={setModel} options={modelOptions} placeholder="Choose model" ariaLabel="The model that analyzes your voice" className="tone-model" />
        <IconButton name="refresh" label="Refresh the model list" onClick={refreshModels} disabled={loadingModels} />
        <IconButton name="history" label="Past profiles" onClick={() => setShowHistory((s) => !s)} />
        <Button variant="primary" size="sm" loading={loading} onClick={run}>
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
        <div className="card tone-history">
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
                <IconButton name="trash" label="Delete" onClick={() => setDeleting(h)} />
              </div>
            </div>
          ))}
        </div>
      )}

      {loading && !profile ? (
        <div className="card tone-loading">
          <Spinner /> <span className="muted">Analyzing your tone of voice…</span>
        </div>
      ) : profile ? (
        <>
          {section === "overview" && <Overview p={profile} />}
          {section === "style" && <Style p={profile} />}
          {section === "language" && <Language p={profile} />}
          {section === "guide" && <Guide p={profile} />}
        </>
      ) : (
        <div className="card tone-empty">
          <h3>No tone profile yet</h3>
          <p className="muted">Analyze your dictation history to see how you sound and get a guide ChatAI can write in.</p>
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

function ScoreRing({ score }: { score: number }) {
  const r = 44;
  const c = 2 * Math.PI * r;
  const v = Math.max(0, Math.min(100, score));
  return (
    <svg className="tone-ring" viewBox="0 0 104 104" role="img" aria-label={`Voice match ${score} out of 100`}>
      <circle cx="52" cy="52" r={r} className="tone-ring-track" />
      <circle
        cx="52"
        cy="52"
        r={r}
        className="tone-ring-value"
        strokeDasharray={`${(v / 100) * c} ${c}`}
        transform="rotate(-90 52 52)"
      />
      <text x="52" y="47" className="tone-ring-score" textAnchor="middle" dominantBaseline="central">
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
        <span key={v} className="ai-chip" style={{ color: tint, background: `color-mix(in srgb, ${tint} 13%, transparent)` }}>
          {v}
        </span>
      ))}
    </div>
  );
}

function ListCard({ title, icon, values, tint }: { title: string; icon: string; values: string[]; tint: string }) {
  return (
    <div className="card ai-list-card">
      <h3 className="ai-list-title">
        <span style={{ color: tint }}>
          <Icon name={icon} size={15} />
        </span>
        {title}
      </h3>
      {values.length === 0 && <p className="muted small">Nothing yet. Rerun after more dictation.</p>}
      <ul>
        {values.slice(0, 5).map((v) => (
          <li key={v}>
            <span className="ai-dot" style={{ background: tint }} />
            <span>{v}</span>
          </li>
        ))}
      </ul>
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
      <div className="card tone-hero">
        <ScoreRing score={p.overall_score} />
        <div className="tone-hero-text">
          <Chips values={p.descriptors.slice(0, 6).map((d) => d.charAt(0).toUpperCase() + d.slice(1))} />
          <p className="tone-summary">{p.summary}</p>
          <p className="muted small">{metaLine(p)}</p>
        </div>
      </div>
      {p.signature_phrases.length > 0 && (
        <div className="card">
          <div className="ai-card-head">
            <h3>Signature phrases</h3>
            <span className="muted small">Phrases you use often enough that they sound like you.</span>
          </div>
          <Chips values={p.signature_phrases.slice(0, 10).map((s) => `“${s}”`)} />
        </div>
      )}
      <div className="grid-3 ai-equal">
        <ListCard title="Recommendations" icon="check" values={p.recommendations} tint="var(--warn)" />
        <ListCard title="Use this voice" icon="sparkle" values={p.signature_approaches} tint="var(--ok)" />
        <ListCard title="Avoid" icon="x" values={p.avoidances} tint="var(--danger)" />
      </div>
    </div>
  );
}

function MetricRow({ m }: { m: ToneMetric }) {
  const tint = METRIC_TINT[m.name] ?? "var(--accent)";
  return (
    <div className="tone-metric" title={m.detail}>
      <span className="tone-metric-name">{m.name}</span>
      <span className="tone-metric-label" style={{ color: tint }}>
        {m.label}
      </span>
      <span className="tone-bar">
        <span style={{ width: `${Math.max(4, pct(Math.max(0, Math.min(1, m.value))))}%`, background: tint }} />
      </span>
      <span className="mono small tone-metric-pct">{pct(m.value)}%</span>
      {m.detail && <span className="muted small tone-metric-detail">{m.detail}</span>}
    </div>
  );
}

function Style({ p }: { p: ToneProfile }) {
  const adequate = p.word_count >= 650;
  return (
    <div className="card">
      <div className="ai-card-head">
        <h3>Tone and style</h3>
        <span
          className={`ai-badge ${adequate ? "ok" : "warn"}`}
          title={adequate ? `Based on ${p.word_count.toLocaleString()} words, an adequate sample.` : "More dictation will make this more reliable."}
        >
          {Math.round(p.confidence * 100)}% confidence
        </span>
      </div>
      <p className="muted small">How your dictation reads on each dimension, measured across your recent history.</p>
      <div className="tone-metrics">
        {p.metrics.map((m) => (
          <MetricRow key={m.name} m={m} />
        ))}
      </div>
    </div>
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
      <div className="grid-4">
        {tiles.map(([v, l, info]) => (
          <div key={l} className="card ai-stat" title={info}>
            <div className="stat-value">{v}</div>
            <div className="stat-label">{l}</div>
          </div>
        ))}
      </div>
      {p.signature_approaches.length > 0 && (
        <ListCard title="Notable traits" icon="edit" values={p.signature_approaches.slice(0, 5)} tint="var(--accent)" />
      )}
      {p.sample_excerpts.length > 0 && (
        <div className="card">
          <div className="ai-card-head">
            <h3>In your words</h3>
            <span className="muted small">Excerpts from your dictation the profile was built on.</span>
          </div>
          <div className="tone-excerpts">
            {p.sample_excerpts.map((e) => (
              <blockquote key={e}>{e}</blockquote>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function Guide({ p }: { p: ToneProfile }) {
  return (
    <div className="tone-stack">
      <div className="card">
        <div className="ai-card-head">
          <h3>ChatAI prompt guide</h3>
          <CopyButton text={p.prompt_guide.trim()} />
        </div>
        <p className="muted small">What ChatAI's My Tone style sends to the model so replies sound like you.</p>
        <pre className="tone-guide">{p.prompt_guide.trim()}</pre>
      </div>
      <div className="grid-3 ai-equal">
        <ListCard title="Use this voice" icon="sparkle" values={p.signature_approaches} tint="var(--ok)" />
        <ListCard title="Avoid" icon="x" values={p.avoidances} tint="var(--danger)" />
        <ListCard title="Recommendations" icon="check" values={p.recommendations} tint="var(--warn)" />
      </div>
    </div>
  );
}

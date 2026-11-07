// Settings → AI Features (Mac ModelSettingsView, AI half). Owned by the ai area.
// Provider + endpoint with a connection check, warm-up, Text Polish (S1-mini or
// a local LLM), Writing Insights, Semantic Memory settings, Graph Insights and
// curated one-click downloads. Everything runs on your own hardware.
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button, Icon, Select, Spinner, useToast } from "../../ui";
import { useEvent } from "../../lib/api";
import { useSettings, type Settings } from "../../lib/settings";
import { Row, Section, SliderRow, ToggleRow, usePatch } from "../../components/SettingsRows";
import {
  ai,
  formatBytes,
  type AiStatus,
  type CatalogEntry,
  type PolishOutcome,
  type Provider,
  type PullEvent,
  type S1MiniStatus,
} from "../../lib/ai";
import "../../styles/ai.css";

const RECOMMENDED_EMBEDDING = "nomic-embed-text";
const DEFAULTS: Record<Provider, string> = { ollama: "http://127.0.0.1:11434", lm_studio: "http://127.0.0.1:1234" };

export default function AISettings() {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [pulls, setPulls] = useState<Record<string, PullEvent>>({});
  const [s1, setS1] = useState<S1MiniStatus | null>(null);
  const [warming, setWarming] = useState(false);
  const [warmNote, setWarmNote] = useState<{ ok: boolean; text: string } | null>(null);

  const check = useCallback(async () => {
    setChecking(true);
    try {
      const st = await ai.status();
      setStatus(st);
      ai.catalog().then(setCatalog).catch(() => setCatalog([]));
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setChecking(false);
    }
    ai.s1Status().then(setS1).catch(() => {});
  }, [toast]);

  useEffect(() => {
    check();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings?.llm_provider, settings?.ollama_endpoint, settings?.lmstudio_endpoint]);

  useEvent<PullEvent>("ai-pull-progress", (p) => {
    setPulls((m) => ({ ...m, [p.model]: p }));
    if (p.done) {
      if (p.error) toast(`Couldn't download ${p.model}: ${p.error}`, "error");
      else toast(`${p.purpose === "s1-mini" ? "S1-mini" : p.model} is ready.`, "success");
      window.setTimeout(() => setPulls((m) => {
        const next = { ...m };
        delete next[p.model];
        return next;
      }), 2500);
      check();
    }
  });

  if (!settings) return null;
  const provider = settings.llm_provider;
  const providerName = provider === "lm_studio" ? "LM Studio" : "Ollama";
  const endpointKey = provider === "lm_studio" ? "lmstudio_endpoint" : "ollama_endpoint";
  const chatModels = (status?.models ?? []).filter((m) => !m.embedding).map((m) => m.name);
  const embeddingModels = (status?.models ?? []).filter((m) => m.embedding).map((m) => m.name);
  const installedRecommended = (status?.models ?? []).find((m) => m.name.toLowerCase().includes(RECOMMENDED_EMBEDDING))?.name;

  const pull = (model: string, purpose: string, select?: Partial<Settings>) => {
    ai.pull(model, purpose)
      .then(() => {
        setPulls((m) => ({
          ...m,
          [model]: { model, purpose, status: "Starting download…", completed: null, total: null, fraction: null, done: false, error: null },
        }));
        if (select) save(select);
      })
      .catch((e) => toast(String(e), "error"));
  };

  const warm = async () => {
    setWarming(true);
    setWarmNote(null);
    try {
      const r = await ai.warm();
      setWarmNote({ ok: true, text: `Warmed ${r.map((x) => `${x.model} (${(x.ms / 1000).toFixed(1)} s)`).join(", ")}.` });
    } catch (e) {
      setWarmNote({ ok: false, text: String(e) });
    } finally {
      setWarming(false);
    }
  };

  return (
    <>
      <Section
        title="AI Provider"
        desc="Where AI features run. Everything stays on your own hardware: Ollama, or LM Studio. Cloud models are never used, even ones Ollama or LM Studio offer."
      >
        <Row label="Provider" hint={`${providerName} server address. It must be on this computer or your local network.`}>
          <Select<Provider>
            value={provider}
            onChange={(v) => save({ llm_provider: v })}
            options={[
              { value: "ollama", label: "Ollama" },
              { value: "lm_studio", label: "LM Studio" },
            ]}
            ariaLabel="AI provider"
          />
          <EndpointInput
            key={provider}
            value={settings[endpointKey]}
            placeholder={DEFAULTS[provider]}
            onCommit={(v) => save({ [endpointKey]: v.trim() || DEFAULTS[provider] } as Partial<Settings>)}
          />
          <Button size="sm" onClick={check} loading={checking} icon={status?.reachable ? "check" : "refresh"}>
            {status?.reachable ? "Connected" : "Check"}
          </Button>
        </Row>
        {status && (
          <div className={`ai-row-notice ${status.reachable ? "ok" : "danger"}`}>
            <Icon name={status.reachable ? "check" : "alert"} size={14} />
            <span>{status.message}</span>
          </div>
        )}
        <Row label="Warm Models" hint="Loads the enabled models into memory so the first polish after launch is fast.">
          <Button size="sm" icon="bolt" onClick={warm} loading={warming} disabled={!status?.reachable}>
            {warming ? "Warming…" : "Warm Now"}
          </Button>
        </Row>
        {warmNote && (
          <div className={`ai-row-notice ${warmNote.ok ? "ok" : "danger"}`}>
            <Icon name={warmNote.ok ? "check" : "alert"} size={14} />
            <span>{warmNote.text}</span>
          </div>
        )}
      </Section>

      <Section
        title="Text Polish"
        desc="Fixes grammar and punctuation in each dictation with a language model before it's inserted. Voice editing uses the same model."
      >
        <ToggleRow
          k="polish_enabled"
          label="Local Text Polish"
          hint="Polishes each dictation on this computer before it's inserted, a paragraph at a time. Whatever can't be polished safely or in time is inserted unpolished."
        />
        <Row label="Polish Engine" hint="S1-mini is a small model trained only to clean up dictation, served by your local AI server. The local LLM option uses the model you pick below.">
          <Select
            value={settings.polish_engine}
            onChange={(v) => save({ polish_engine: v })}
            options={[
              { value: "s1_mini", label: "S1-mini" },
              { value: "local_llm", label: "Local LLM server" },
            ]}
            ariaLabel="Polish engine"
          />
        </Row>
        {settings.polish_engine === "local_llm" ? (
          <>
            <Row label="Polish Model">
              <ModelSelect value={settings.polish_model} models={chatModels} onChange={(v) => save({ polish_model: v })} />
            </Row>
            <SliderRow
              label="Polish Timeout"
              hint="How long to wait for polish before inserting the unpolished text."
              value={settings.polish_timeout_ms}
              min={500}
              max={8000}
              step={100}
              format={(v) => `${v} ms`}
              onCommit={(v) => save({ polish_timeout_ms: v })}
            />
            <SliderRow
              label="Max Characters"
              hint="Longer dictations skip polish to stay responsive."
              value={settings.polish_max_chars}
              min={80}
              max={2000}
              step={20}
              onCommit={(v) => save({ polish_max_chars: v })}
            />
            <div className="ai-row-notice">
              <Icon name="info" size={14} />
              <span>{polishRecommendation(settings)}</span>
            </div>
          </>
        ) : (
          <S1MiniRow s1={s1} pulls={pulls} onInstall={() => ai.s1Install().then(() => ai.s1Status().then(setS1)).catch((e) => toast(String(e), "error"))} />
        )}
        <PolishTry />
      </Section>

      <Section title="Writing Insights" desc="Uses a local model to surface speaking and writing patterns.">
        <ToggleRow k="insights_enabled" label="Writing Insights" hint="Without a model, insights use a quick on-device reading of your history." />
        {settings.insights_enabled && (
          <>
            <Row label="Insights Model">
              <ModelSelect value={settings.insights_model || "gemma4:e2b"} models={chatModels} onChange={(v) => save({ insights_model: v })} />
            </Row>
            <SliderRow
              label="Context Window"
              hint="How much of your history the model reads at once. Larger windows need more memory."
              value={Math.max(8192, settings.insights_context_tokens)}
              min={8192}
              max={131072}
              step={8192}
              format={(v) => `${v / 1024}K tokens`}
              onCommit={(v) => save({ insights_context_tokens: v })}
            />
            <ToggleRow
              k="insights_thinking"
              label="Thinking"
              hint="Lets the model reason step by step for deeper analysis. Slower, and uses more tokens."
            />
            <div className="ai-row-notice">
              <Icon name="info" size={14} />
              <span>{insightsRecommendation(settings)}</span>
            </div>
          </>
        )}
      </Section>

      <Section
        title="Semantic Memory"
        desc="Indexes your dictations by meaning so ChatAI and Memory can find related notes, not only exact words."
      >
        <ToggleRow k="semantic_enabled" label="Semantic Memory" />
        {settings.semantic_enabled && (
          <>
            <Row label="Embedding Model" hint="Changing the model rebuilds the index.">
              <ModelSelect
                value={settings.semantic_embedding_model}
                models={embeddingModels}
                onChange={(v) => save({ semantic_embedding_model: v })}
              />
            </Row>
            <Row
              label="Recommended"
              hint={`nomic-embed-text finds related notes by meaning. It's optional: without it, semantic memory uses keywords. ${providerName} downloads, stores and serves it; Orttaai doesn't host models.`}
            >
              {installedRecommended ? (
                settings.semantic_embedding_model === installedRecommended ? (
                  <span className="ai-badge ok">In use</span>
                ) : (
                  <Button size="sm" onClick={() => save({ semantic_embedding_model: installedRecommended })}>
                    Use {installedRecommended}
                  </Button>
                )
              ) : status?.reachable ? (
                <PullButton
                  model={provider === "lm_studio" ? "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF" : RECOMMENDED_EMBEDDING}
                  label={provider === "lm_studio" ? "Download" : "Download 274 MB"}
                  pulls={pulls}
                  onPull={(m) => pull(m, "Semantic memory", provider === "ollama" ? { semantic_embedding_model: RECOMMENDED_EMBEDDING } : undefined)}
                />
              ) : (
                <span className="muted small">Start {providerName} to download</span>
              )}
            </Row>
            <ToggleRow
              k="semantic_auto_index"
              label="Auto-index for ChatAI"
              hint="Indexes new dictations in the background so ChatAI can use them right away."
            />
            <ToggleRow
              k="semantic_keyword_fallback"
              label="Keyword Fallback"
              hint="Falls back to keyword search when the embedding model isn't available."
            />
          </>
        )}
      </Section>

      <Section
        title="Graph Insights"
        desc="Memory Graph insights are computed from the connections between your notes. A local model phrases them; with it off, they use plain phrasing."
      >
        <ToggleRow k="semantic_summary_enabled" label="Graph Insights Model" />
        {settings.semantic_summary_enabled && (
          <Row label="Insight Model" hint={`The model that phrases Memory Graph insights on ${providerName}.`}>
            <ModelSelect
              value={settings.semantic_summary_model || "gemma4:e2b"}
              models={chatModels}
              onChange={(v) => save({ semantic_summary_model: v })}
            />
          </Row>
        )}
      </Section>

      <Section
        title="Model Downloads"
        desc={`Curated lightweight models from ${providerName}. Installing one also selects it for that feature.`}
      >
        {provider === "lm_studio" ? (
          <Row label="LM Studio" hint="LM Studio manages its own model library. Download models in LM Studio, then pick them above.">
            <span className="muted small">{status?.models.length ?? 0} local models</span>
          </Row>
        ) : !status?.reachable ? (
          <Row label="Curated models" hint={`${providerName} must be reachable before loading downloadable models.`}>
            <Button size="sm" icon="refresh" onClick={check} loading={checking}>
              Check again
            </Button>
          </Row>
        ) : (
          <>
            <DownloadRow
              title="Polish"
              entries={catalog.filter((c) => c.kind === "chat")}
              pulls={pulls}
              onPull={(m) => pull(m, "Polish", { polish_model: m, polish_engine: "local_llm" })}
            />
            <DownloadRow
              title="Insights"
              entries={catalog.filter((c) => c.kind === "chat")}
              pulls={pulls}
              onPull={(m) => pull(m, "Insights", { insights_model: m })}
            />
            <DownloadRow
              title="Semantic"
              entries={catalog.filter((c) => c.kind === "embedding")}
              pulls={pulls}
              onPull={(m) => pull(m, "Semantic memory", { semantic_embedding_model: m })}
            />
          </>
        )}
        {Object.values(pulls)
          .filter((p) => p.purpose !== "s1-mini")
          .map((p) => (
            <PullProgressRow key={p.model} p={p} />
          ))}
      </Section>
    </>
  );
}

function polishRecommendation(s: Settings): string {
  const lower = (s.polish_model || "gemma4:e2b").toLowerCase();
  if (lower.includes(":4b")) return "This model is usually too heavy for fast polish. `gemma4:e2b` is the eval-proven default.";
  const recommended = lower.includes("gemma4:e4b") ? 3000 : lower.includes("gemma4:e2b") ? 2500 : 600;
  if (s.polish_timeout_ms < recommended) return "Current timeout is aggressive for this model. Expect cold-start fallbacks until it is warmed.";
  return "Warm the model once after launch to keep polish inside the timeout budget.";
}

function insightsRecommendation(s: Settings): string {
  if (s.insights_thinking) return "Thinking is enabled for deeper analysis and can use more tokens.";
  if ((s.insights_model || "").toLowerCase().includes("qwen3.5:4b")) return "This model can take longer for deeper on-device insights.";
  return "Thinking is off by default to keep insight runs lean.";
}

function EndpointInput({ value, placeholder, onCommit }: { value: string; placeholder: string; onCommit: (v: string) => void }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      className="text-input ai-endpoint"
      value={v}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") setV(value);
      }}
      aria-label="Server address"
    />
  );
}

function ModelSelect({ value, models, onChange }: { value: string; models: string[]; onChange: (v: string) => void }) {
  const options = useMemo(() => {
    // `all-minilm` and `all-minilm:latest` are the same model.
    const same = (a: string, b: string) => {
      const norm = (s: string) => (s.includes(":") ? s : `${s}:latest`).toLowerCase();
      return norm(a) === norm(b);
    };
    const names = [...models];
    if (value && !names.some((n) => same(n, value))) names.unshift(value);
    return names.map((n) => ({
      value: n,
      label: models.includes(n) ? n : `${n} (not installed)`,
    }));
  }, [models, value]);
  const selected = options.find((o) => o.value === value)
    ? value
    : (options.find((o) => o.value.toLowerCase() === `${value}:latest`.toLowerCase())?.value ?? value);
  if (options.length === 0) return <span className="muted small">No models installed</span>;
  return <Select value={selected} onChange={onChange} options={options} ariaLabel="Model" className="ai-model-select" />;
}

function PullButton({
  model,
  label,
  pulls,
  onPull,
}: {
  model: string;
  label: string;
  pulls: Record<string, PullEvent>;
  onPull: (m: string) => void;
}) {
  const p = pulls[model];
  if (p && !p.done) return <span className="muted small mono">{p.fraction != null ? `${Math.round(p.fraction * 100)}%` : <Spinner size={12} />}</span>;
  return (
    <Button size="sm" variant="primary" icon="download" onClick={() => onPull(model)}>
      {label}
    </Button>
  );
}

function PullProgressRow({ p }: { p: PullEvent }) {
  return (
    <div className="ai-progress-row">
      <div className="ai-progress-text">
        <span className="mono small">{p.model}</span>
        <span className="muted small">
          {p.error ? p.error : p.status}
          {p.fraction != null && !p.done ? ` · ${Math.round(p.fraction * 100)}%` : ""}
          {p.total ? ` · ${formatBytes(p.completed)} of ${formatBytes(p.total)}` : ""}
        </span>
      </div>
      <div className="ai-progress">
        <span style={{ width: `${Math.round((p.done && !p.error ? 1 : p.fraction ?? 0) * 100)}%` }} className={p.error ? "failed" : ""} />
      </div>
    </div>
  );
}

function DownloadRow({
  title,
  entries,
  pulls,
  onPull,
}: {
  title: string;
  entries: CatalogEntry[];
  pulls: Record<string, PullEvent>;
  onPull: (model: string) => void;
}) {
  const [pick, setPick] = useState(entries.find((e) => !e.installed)?.name ?? entries[0]?.name ?? "");
  useEffect(() => {
    if (!entries.some((e) => e.name === pick)) setPick(entries.find((e) => !e.installed)?.name ?? entries[0]?.name ?? "");
  }, [entries, pick]);
  const entry = entries.find((e) => e.name === pick);
  if (entries.length === 0) return null;
  return (
    <Row label={title}>
      <Select
        value={pick}
        onChange={setPick}
        options={entries.map((e) => ({
          value: e.name,
          label: `${e.name}${e.size ? ` · ${formatBytes(e.size)}` : ""}${e.installed ? " · Downloaded" : ""}`,
        }))}
        ariaLabel={`${title} model to download`}
        className="ai-download-select"
      />
      {entry?.installed ? (
        <span className="ai-badge ok">Downloaded</span>
      ) : (
        <PullButton model={pick} label="Install" pulls={pulls} onPull={onPull} />
      )}
    </Row>
  );
}

function S1MiniRow({
  s1,
  pulls,
  onInstall,
}: {
  s1: S1MiniStatus | null;
  pulls: Record<string, PullEvent>;
  onInstall: () => void;
}) {
  const p = s1 ? pulls[s1.download] : undefined;
  const downloading = (p && !p.done) || s1?.downloading;
  const size = formatBytes(s1?.size ?? 484_219_808);
  return (
    <>
      <Row
        label="S1-mini"
        hint={`A 0.6B model trained only to clean up dictation: it removes fillers, resolves self-corrections, punctuates and writes lists, without rewriting what you said. Downloaded once, ${size}.`}
      >
        {!s1 ? (
          <Spinner />
        ) : s1.installed ? (
          <span className="ai-badge ok">
            <Icon name="check" size={12} /> Ready
          </span>
        ) : downloading ? (
          <span className="muted small mono">{p?.fraction != null ? `${Math.round(p.fraction * 100)}%` : <Spinner size={12} />}</span>
        ) : (
          <Button size="sm" variant="primary" icon="download" onClick={onInstall} disabled={!s1.reachable}>
            Install S1-mini · {size}
          </Button>
        )}
      </Row>
      {p && <PullProgressRow p={p} />}
      {s1 && (
        <div className={`ai-row-notice ${s1.reachable ? "" : "danger"}`}>
          <Icon name={s1.reachable ? "info" : "alert"} size={14} />
          <span>
            {s1.message} {s1.credit}
          </span>
        </div>
      )}
    </>
  );
}

function PolishTry() {
  const [text, setText] = useState("um so the the meeting is moved to 3pm on friday you know");
  const [busy, setBusy] = useState(false);
  const [out, setOut] = useState<PolishOutcome | null>(null);
  return (
    <div className="ai-try">
      <div className="ai-try-head">
        <span className="set-row-label">Try polish</span>
        <span className="set-row-hint">Runs your current polish settings on this text. Nothing is typed anywhere.</span>
      </div>
      <div className="ai-try-body">
        <input className="text-input" value={text} onChange={(e) => setText(e.target.value)} aria-label="Text to polish" />
        <Button
          size="sm"
          loading={busy}
          onClick={() => {
            setBusy(true);
            ai.polishTest(text)
              .then(setOut)
              .catch((e) => setOut({ text, polished: false, ms: 0, error: String(e) }))
              .finally(() => setBusy(false));
          }}
        >
          Polish
        </Button>
      </div>
      {out && (
        <div className={`ai-try-out ${out.polished ? "ok" : ""}`}>
          <span>{out.text}</span>
          <span className="muted small">
            {out.polished ? `Polished in ${out.ms} ms` : out.error ? `Not polished: ${out.error}` : "No changes needed"}
          </span>
        </div>
      )}
    </div>
  );
}


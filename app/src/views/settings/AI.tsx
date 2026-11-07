// Model → AI Features (Mac ModelSettingsView, AI half). Owned by the ai area.
// AI Provider with a connection check and warm-up, Text Polish (S1-mini or a
// local LLM), Writing Insights, Semantic Memory, Graph Insights and curated
// one-click downloads. Everything runs on your own hardware.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog, Select, Spinner } from "../../ui";
import { useEvent } from "../../lib/api";
import { useSettings, type Settings } from "../../lib/settings";
import { usePatch } from "../../components/SettingsRows";
import {
  Glyph,
  MButton,
  MCard,
  MDivider,
  MFootnote,
  MNotice,
  MProgress,
  MRow,
  MSliderRow,
  MStepperRow,
  MSwitch,
  MToggleRow,
} from "../../components/ExpressCard";
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
import { semantic, useSemanticStatus } from "../../lib/semantic";
import "../../styles/engine.css";

const RECOMMENDED_EMBEDDING = "nomic-embed-text";
const RECOMMENDED_EMBEDDING_LM = "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF";
const RECOMMENDED_EMBEDDING_SIZE = "274 MB";
const DEFAULTS: Record<Provider, string> = { ollama: "http://127.0.0.1:11434", lm_studio: "http://127.0.0.1:1234" };

/** `all-minilm` and `all-minilm:latest` are the same model. */
const canonical = (name: string) => {
  const n = name.trim().toLowerCase();
  return n.includes(":") ? n : `${n}:latest`;
};

/** The "Download this model?" question (Mac LocalModelDownloader). */
type DownloadRequest = { model: string; download: string; purpose: string; size: string | null; then: () => void };

export default function AISettings() {
  const { settings } = useSettings();
  const save = usePatch();
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [pulls, setPulls] = useState<Record<string, PullEvent>>({});
  const [s1, setS1] = useState<S1MiniStatus | null>(null);
  const [warming, setWarming] = useState(false);
  const [warmNote, setWarmNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [installNote, setInstallNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [s1Error, setS1Error] = useState<string | null>(null);
  const [request, setRequest] = useState<DownloadRequest | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [indexMessage, setIndexMessage] = useState<string | null>(null);
  const afterPull = useRef<Record<string, () => void>>({});
  const index = useSemanticStatus();

  const check = useCallback(async () => {
    setChecking(true);
    let st: AiStatus | null = null;
    try {
      st = await ai.status();
      setStatus(st);
      setCatalogLoading(true);
      ai.catalog()
        .then(setCatalog)
        .catch(() => setCatalog([]))
        .finally(() => setCatalogLoading(false));
    } catch (e) {
      setStatus(null);
      setWarmNote({ ok: false, text: String(e) });
    } finally {
      setChecking(false);
    }
    ai.s1Status().then(setS1).catch(() => {});
    return st;
  }, []);

  useEffect(() => {
    check();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings?.llm_provider, settings?.ollama_endpoint, settings?.lmstudio_endpoint]);

  useEvent<PullEvent>("ai-pull-progress", (p) => {
    setPulls((m) => ({ ...m, [p.model]: p }));
    if (p.done) {
      setPulls((m) => {
        const next = { ...m };
        delete next[p.model];
        return next;
      });
      const then = afterPull.current[p.model];
      delete afterPull.current[p.model];
      if (p.purpose === "s1-mini") {
        if (p.error) setS1Error(p.error);
      } else if (p.purpose === "Semantic memory") {
        if (p.error) setDownloadError(`Couldn't download ${p.model}: ${p.error}`);
      } else {
        setInstallNote(p.error ? { ok: false, text: `Install failed for ${p.model}: ${p.error}` } : { ok: true, text: `Installed ${p.model}.` });
      }
      check();
      if (!p.error) then?.();
    }
  });

  if (!settings) return null;
  const provider = settings.llm_provider;
  const providerName = provider === "lm_studio" ? "LM Studio" : "Ollama";
  const endpointKey = provider === "lm_studio" ? "lmstudio_endpoint" : "ollama_endpoint";
  const installed = (status?.models ?? []).map((m) => m.name);
  const chatModels = (status?.models ?? []).filter((m) => !m.embedding).map((m) => m.name);
  const embeddingModels = (status?.models ?? []).filter((m) => m.embedding).map((m) => m.name);
  const installedRecommended = installed.find((m) => m.toLowerCase().includes(RECOMMENDED_EMBEDDING));
  const reachable = status ? status.reachable : null;
  const recommendedDownload = provider === "lm_studio" ? RECOMMENDED_EMBEDDING_LM : RECOMMENDED_EMBEDDING;
  const semanticPull = Object.values(pulls).find((p) => p.purpose === "Semantic memory");
  const installPulls = Object.values(pulls).filter((p) => p.purpose !== "s1-mini" && p.purpose !== "Semantic memory");
  const s1Pull = s1 ? pulls[s1.download] : undefined;
  const pulling = Object.values(pulls).some((p) => !p.done);

  const pull = (model: string, purpose: string, select?: Partial<Settings>, then?: () => void) => {
    setInstallNote(null);
    setDownloadError(null);
    ai.pull(model, purpose)
      .then(() => {
        setPulls((m) => ({
          ...m,
          [model]: { model, purpose, status: "Starting download…", completed: null, total: null, fraction: null, done: false, error: null },
        }));
        if (then) afterPull.current[model] = then;
        if (select) save(select);
      })
      .catch((e) =>
        purpose === "Semantic memory"
          ? setDownloadError(`Couldn't download ${model}: ${String(e)}`)
          : setInstallNote({ ok: false, text: `Install failed for ${model}: ${String(e)}` }),
      );
  };

  const warm = async () => {
    setWarming(true);
    setWarmNote(null);
    try {
      const r = await ai.warm();
      setWarmNote({ ok: true, text: `Warm-up complete: ${r.map((x) => `${x.model} (${x.ms} ms)`).join(", ")}` });
    } catch (e) {
      setWarmNote({ ok: false, text: `Warm-up failed: ${String(e)}` });
    } finally {
      setWarming(false);
    }
  };

  const pickEmbedding = (model: string) => save({ semantic_embedding_model: model });

  const downloadRecommended = () =>
    setRequest({
      model: RECOMMENDED_EMBEDDING,
      download: recommendedDownload,
      purpose: "Semantic memory",
      size: provider === "ollama" ? RECOMMENDED_EMBEDDING_SIZE : null,
      then: () => {
        check().then((st) => {
          const found = st?.models.map((m) => m.name).find((m) => m.toLowerCase().includes(RECOMMENDED_EMBEDDING));
          if (found) pickEmbedding(found);
        });
      },
    });

  const runIndex = () => {
    setIndexMessage(null);
    semantic
      .indexNow()
      .then(index.refresh)
      .catch((e) => setIndexMessage(String(e)));
  };
  const indexNow = () => {
    const model = settings.semantic_embedding_model.trim();
    const missing = reachable === true && model && !installed.some((m) => canonical(m) === canonical(model));
    if (missing) {
      const size = catalog.find((c) => canonical(c.name) === canonical(model))?.size;
      setRequest({ model, download: model, purpose: "Semantic memory", size: size ? formatBytes(size) : null, then: runIndex });
    } else {
      runIndex();
    }
  };

  const indexing = Boolean(index.status?.running) || index.progress !== null;
  const stats = index.status?.index;
  const indexStatusText =
    indexMessage ??
    (indexing
      ? "Indexing…"
      : !stats || stats.chunk_count === 0
        ? "Nothing indexed yet"
        : `${stats.embedded_chunk_count.toLocaleString()} of ${stats.chunk_count.toLocaleString()} notes · ${stats.active_model || "keywords"}`);

  const connection = checking ? "Checking…" : reachable === true ? "Ready" : reachable === false ? "Retry" : "Check";
  const connectionTone = checking ? "" : reachable === true ? "ok" : reachable === false ? "warn" : "";
  const connectionGlyph = reachable === true ? "checkCircleFill" : reachable === false ? "alertCircleFill" : "bolt";

  return (
    <div className="mset settings-ai">
      <MCard
        title="AI Provider"
        info="Where AI features run. Everything stays on your own hardware: Ollama, or LM Studio. Cloud models are never used, even ones Ollama or LM Studio offer."
        accessory={
          <span className="mset-accessory">
            <Select<Provider>
              value={provider}
              onChange={(v) => save({ llm_provider: v })}
              options={[
                { value: "ollama", label: "Ollama" },
                { value: "lm_studio", label: "LM Studio" },
              ]}
              ariaLabel="AI provider"
              className="mset-dd w150"
            />
            <EndpointInput
              key={provider}
              value={settings[endpointKey]}
              placeholder={DEFAULTS[provider]}
              title={`${providerName} server address. It must be on this computer or your local network.`}
              onCommit={(v) => save({ [endpointKey]: v.trim() || DEFAULTS[provider] } as Partial<Settings>)}
            />
            <button
              type="button"
              className={`mset-btn secondary mset-check ${connectionTone}`}
              disabled={checking || pulling}
              title={status?.message ?? "Check connection to validate local model availability."}
              aria-label={`Connection: ${connection}`}
              onClick={check}
            >
              {checking ? <Spinner size={11} /> : <Glyph name={connectionGlyph} size={13} />}
              {connection}
            </button>
          </span>
        }
      >
        {reachable === false && status && <MNotice kind="error">{status.message}</MNotice>}
        <MRow title="Warm Models" info="Loads the enabled models into memory so the first polish after launch is fast.">
          {warming && (
            <span className="mset-inline-status plain">
              <Spinner size={13} /> Priming models...
            </span>
          )}
          <MButton glyph="bolt" disabled={checking || warming || pulling} onClick={warm}>
            {warming ? "Warming…" : "Warm Now"}
          </MButton>
        </MRow>
        {warmNote && (
          <MNotice kind={warmNote.ok ? "success" : "error"} onDismiss={() => setWarmNote(null)}>
            {warmNote.text}
          </MNotice>
        )}
      </MCard>

      <MCard
        title="Text Polish"
        info="Fixes grammar and punctuation in each dictation with a language model before it's inserted. Voice editing uses the same model."
      >
        <MToggleRow
          k="polish_enabled"
          title="Local Text Polish"
          info="Polishes each dictation on this computer before it's inserted, a paragraph at a time. Whatever can't be polished safely or in time is inserted unpolished."
        />
        <MDivider />
        <MRow
          title="Polish Engine"
          info="S1-mini is a small model trained only to clean up dictation, served by your local AI server. The local LLM server uses the model you pick below."
        >
          <Select
            value={settings.polish_engine}
            onChange={(v) => save({ polish_engine: v })}
            options={[
              { value: "s1_mini", label: "S1-mini" },
              { value: "local_llm", label: "Local LLM server" },
            ]}
            ariaLabel="Polish engine"
            className="mset-dd"
          />
        </MRow>
        <MDivider />
        {settings.polish_engine === "local_llm" ? (
          <>
            <MRow title="Polish Model">
              <ModelSelect label="Polish model" value={settings.polish_model} models={chatModels} onChange={(v) => save({ polish_model: v })} />
            </MRow>
            <MDivider />
            <MSliderRow
              title="Polish Timeout"
              info="How long to wait for polish before inserting the unpolished text."
              value={settings.polish_timeout_ms}
              min={500}
              max={4000}
              step={10}
              format={(v) => `${v} ms`}
              onCommit={(v) => save({ polish_timeout_ms: v })}
            />
            <MDivider />
            <MStepperRow
              title="Max Characters"
              info="Longer dictations skip polish to stay responsive."
              value={settings.polish_max_chars}
              min={80}
              max={2000}
              step={20}
              onChange={(v) => save({ polish_max_chars: v })}
            />
            <MFootnote>{polishRecommendation(settings)}</MFootnote>
          </>
        ) : (
          <S1MiniRow
            s1={s1}
            pull={s1Pull}
            error={s1Error}
            onDismissError={() => setS1Error(null)}
            onInstall={() => {
              setS1Error(null);
              ai.s1Install()
                .then(() => ai.s1Status().then(setS1))
                .catch((e) => setS1Error(String(e)));
            }}
          />
        )}
        <MDivider />
        <PolishTry />
      </MCard>

      <MCard
        title="Writing Insights"
        info="Uses a local model to surface speaking and writing patterns."
        accessory={<MSwitch k="insights_enabled" title="Writing Insights" />}
      >
        {settings.insights_enabled && (
          <>
            <MRow title="Insights Model">
              <ModelSelect
                label="Insights model"
                value={settings.insights_model || "gemma4:e2b"}
                models={chatModels}
                onChange={(v) => save({ insights_model: v })}
              />
            </MRow>
            <MDivider />
            <MStepperRow
              title="Context Window"
              info="How much of your history the model reads at once. Larger windows need more memory."
              value={Math.max(8192, settings.insights_context_tokens)}
              min={8192}
              max={131072}
              step={8192}
              format={(v) => `${v / 1024}K tokens`}
              onChange={(v) => save({ insights_context_tokens: v })}
            />
            <MDivider />
            <MToggleRow
              k="insights_thinking"
              title="Thinking"
              info="Lets the model reason step by step for deeper analysis. Slower, and uses more tokens."
            />
            <MFootnote>{insightsRecommendation(settings)}</MFootnote>
          </>
        )}
      </MCard>

      <MCard
        title="Semantic Memory"
        info="Indexes your dictations by meaning so ChatAI and Memory can find related notes, not only exact words."
        accessory={<MSwitch k="semantic_enabled" title="Semantic Memory" />}
      >
        {settings.semantic_enabled && (
          <>
            <MRow title="Embedding Model" info="Changing the model rebuilds the index.">
              <ModelSelect
                label="Embedding model"
                value={settings.semantic_embedding_model}
                models={embeddingModels.length > 0 ? embeddingModels : installed}
                onChange={pickEmbedding}
              />
            </MRow>
            <MDivider />
            <MRow
              title="Recommended"
              info={`nomic-embed-text finds related notes by meaning. It's optional: without it, semantic memory uses keywords. ${providerName} downloads, stores and serves it; Orttaai doesn't host models.`}
            >
              <span className="mset-mono dim">{RECOMMENDED_EMBEDDING}</span>
              {installedRecommended ? (
                canonical(settings.semantic_embedding_model) === canonical(installedRecommended) ? (
                  <span className="mset-ready">
                    <Glyph name="checkCircleFill" size={11} /> In use
                  </span>
                ) : (
                  <MButton
                    glyph="checkCircle"
                    title={`Use ${installedRecommended} for semantic memory. The index rebuilds with it.`}
                    onClick={() => pickEmbedding(installedRecommended)}
                  >
                    Use
                  </MButton>
                )
              ) : reachable ? (
                <MButton
                  glyph="downloadCircle"
                  disabled={Boolean(semanticPull)}
                  title={`${providerName} downloads it and serves it from this computer.`}
                  onClick={downloadRecommended}
                >
                  {semanticPull ? "Downloading…" : provider === "ollama" ? `Download (${RECOMMENDED_EMBEDDING_SIZE})` : "Download"}
                </MButton>
              ) : (
                <span className="mset-caption dim">Start {providerName} to download</span>
              )}
            </MRow>
            {semanticPull && (
              <MProgress
                label={semanticPull.total ? `${providerName} is downloading ${semanticPull.model}: ${formatBytes(semanticPull.completed)} of ${formatBytes(semanticPull.total)}` : `${providerName} is downloading ${semanticPull.model}…`}
                fraction={semanticPull.fraction}
              />
            )}
            {downloadError && (
              <MNotice kind="error" onDismiss={() => setDownloadError(null)}>
                {downloadError}
              </MNotice>
            )}
            <MDivider />
            <MRow title="Index" info="Auto-index keeps this current; Index Now catches up straight away.">
              <span className="mset-caption text2 mset-index-status" title={indexStatusText}>
                {indexStatusText}
              </span>
              <MButton glyph="sync" disabled={indexing || Boolean(semanticPull)} onClick={indexNow}>
                {indexing ? "Indexing…" : "Index Now"}
              </MButton>
            </MRow>
            <MDivider />
            <MToggleRow
              k="semantic_auto_index"
              title="Auto-index for ChatAI"
              info="Indexes new dictations in the background so ChatAI can use them right away."
            />
            <MDivider />
            <MToggleRow
              k="semantic_keyword_fallback"
              title="Keyword Fallback"
              info="Falls back to keyword search when the embedding model isn't available."
            />
            <MDivider />
            <MRow
              title="Clear Index"
              info="Deletes the index and the Memory Graph built from it. Your dictations stay; the index rebuilds from them."
            >
              <MButton kind="danger" glyph="trash" disabled={indexing || (stats?.chunk_count ?? 0) === 0} onClick={() => setConfirmClear(true)}>
                Clear…
              </MButton>
            </MRow>
          </>
        )}
      </MCard>

      <MCard
        title="Graph Insights"
        info="Memory Graph insights are computed from the connections between your notes. A local model phrases them; with it off, they use plain phrasing."
        accessory={<MSwitch k="semantic_summary_enabled" title="Graph Insights Model" />}
      >
        {settings.semantic_summary_enabled && (
          <MRow title="Insight Model" info={`If it isn't on ${providerName} yet, Generate Insights offers to download it first.`}>
            <ModelSelect
              label="Insight model"
              value={settings.semantic_summary_model || "gemma4:e2b"}
              models={chatModels}
              onChange={(v) => save({ semantic_summary_model: v })}
            />
          </MRow>
        )}
      </MCard>

      {provider === "ollama" && (
        <MCard title="Model Downloads" info={`Curated lightweight models from ${providerName}. Installing one also selects it for that feature.`}>
          {catalogLoading && catalog.length === 0 ? (
            <span className="mset-inline-status plain mset-pad">
              <Spinner size={13} /> Loading curated models…
            </span>
          ) : !reachable ? (
            <MFootnote>{providerName} must be reachable before loading downloadable models.</MFootnote>
          ) : catalog.length === 0 ? (
            <MFootnote>No curated lightweight models configured.</MFootnote>
          ) : (
            <>
              <DownloadRow
                title="Polish"
                current={settings.polish_model}
                entries={catalog.filter((c) => c.kind === "chat")}
                pulls={pulls}
                busy={checking}
                onPull={(m) => pull(m, "Polish", { polish_model: m, polish_engine: "local_llm" })}
              />
              <MDivider />
              <DownloadRow
                title="Insights"
                current={settings.insights_model}
                entries={catalog.filter((c) => c.kind === "chat")}
                pulls={pulls}
                busy={checking}
                onPull={(m) => pull(m, "Insights", { insights_model: m })}
              />
              <MDivider />
              <DownloadRow
                title="Semantic"
                current={settings.semantic_embedding_model}
                entries={catalog.filter((c) => c.kind === "embedding")}
                pulls={pulls}
                busy={checking}
                onPull={(m) => pull(m, "Semantic", { semantic_embedding_model: m })}
              />
            </>
          )}
          {installPulls.map((p) => (
            <MProgress
              key={p.model}
              label={`${p.status || `Downloading ${p.model}...`}${p.fraction != null && p.total ? ` (${Math.round(p.fraction * 100)}% • ${formatBytes(p.completed)}/${formatBytes(p.total)})` : ""}`}
              fraction={p.fraction}
            />
          ))}
          {installNote && (
            <MNotice kind={installNote.ok ? "success" : "error"} onDismiss={() => setInstallNote(null)}>
              {installNote.text}
            </MNotice>
          )}
        </MCard>
      )}

      <ConfirmDialog
        open={request !== null}
        tone="primary"
        title={`Download ${request?.model ?? "model"}?`}
        body={
          request &&
          `${request.purpose} uses ${request.model}, which isn't in ${providerName} yet. ${providerName} will download it${
            request.size ? ` (about ${request.size})` : ""
          } and run it on this computer; Orttaai doesn't store models. Then ${request.purpose.toLowerCase()} continues.`
        }
        confirmLabel="Download and Continue"
        onCancel={() => setRequest(null)}
        onConfirm={() => {
          const r = request;
          setRequest(null);
          if (r) pull(r.download, r.purpose, undefined, r.then);
        }}
      />
      <ConfirmDialog
        open={confirmClear}
        title="Clear the semantic memory index?"
        body="This deletes the index and the Memory Graph built from it. Your dictations stay, and the index rebuilds from them."
        confirmLabel="Clear Index"
        onCancel={() => setConfirmClear(false)}
        onConfirm={() => {
          setConfirmClear(false);
          setIndexMessage(null);
          semantic
            .clearIndex()
            .then(index.refresh)
            .catch((e) => setIndexMessage(`Couldn't clear the index: ${String(e)}`));
        }}
      />
    </div>
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

function EndpointInput({
  value,
  placeholder,
  title,
  onCommit,
}: {
  value: string;
  placeholder: string;
  title: string;
  onCommit: (v: string) => void;
}) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      className="mset-input mset-endpoint"
      value={v}
      placeholder={placeholder}
      title={title}
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

/** Installed models, with the stored value on top (flagged) when the server doesn't have it. */
function ModelSelect({ label, value, models, onChange }: { label: string; value: string; models: string[]; onChange: (v: string) => void }) {
  const options = useMemo(() => {
    const names = [...models];
    if (value && !names.some((n) => canonical(n) === canonical(value))) names.unshift(value);
    return names.map((n) => ({ value: n, label: models.includes(n) ? n : `${n} (not installed)` }));
  }, [models, value]);
  const selected = options.find((o) => canonical(o.value) === canonical(value))?.value ?? value;
  return <Select value={selected} onChange={onChange} options={options} ariaLabel={label} className="mset-dd" placeholder="Select" />;
}

function DownloadRow({
  title,
  current,
  entries,
  pulls,
  busy,
  onPull,
}: {
  title: string;
  current: string;
  entries: CatalogEntry[];
  pulls: Record<string, PullEvent>;
  busy: boolean;
  onPull: (model: string) => void;
}) {
  const initial = entries.find((e) => canonical(e.name) === canonical(current))?.name ?? entries[0]?.name ?? "";
  const [pick, setPick] = useState(initial);
  useEffect(() => {
    if (!entries.some((e) => e.name === pick)) setPick(initial);
  }, [entries, pick, initial]);
  const entry = entries.find((e) => e.name === pick);
  const installing = Boolean(pulls[pick] && !pulls[pick].done);
  if (entries.length === 0) return null;
  return (
    <MRow title={title}>
      <Select
        value={pick}
        onChange={setPick}
        options={entries.map((e) => ({ value: e.name, label: e.size ? `${e.name} (${formatBytes(e.size)})` : e.name }))}
        ariaLabel={`${title} model to download`}
        className="mset-dd w260"
      />
      <MButton
        className="mset-install"
        glyph={entry?.installed ? "checkCircle" : "downloadCircle"}
        disabled={!pick || busy || installing || entry?.installed}
        onClick={() => onPull(pick)}
      >
        {installing ? "Installing…" : entry?.installed ? "Installed" : "Install"}
      </MButton>
    </MRow>
  );
}

/** S1-mini: its download, whether it's ready, and the credit its licence asks for. */
function S1MiniRow({
  s1,
  pull,
  error,
  onDismissError,
  onInstall,
}: {
  s1: S1MiniStatus | null;
  pull?: PullEvent;
  error: string | null;
  onDismissError: () => void;
  onInstall: () => void;
}) {
  const downloading = Boolean((pull && !pull.done) || s1?.downloading);
  const size = formatBytes(s1?.size ?? 484_219_808);
  return (
    <>
      <MRow
        title="S1-mini"
        info={`A 0.6B model trained only to clean up dictation: it removes fillers, resolves self-corrections, punctuates and writes lists, without rewriting what you said. Downloaded once, ${size}.`}
      >
        {!s1 ? (
          <Spinner size={13} />
        ) : s1.installed ? (
          <span className="mset-ready">
            <Glyph name="checkCircleFill" size={11} /> Ready
          </span>
        ) : downloading ? (
          <span className="mset-caption text2 mset-tabular">{pull?.fraction != null ? `${Math.round(pull.fraction * 100)}%` : <Spinner size={12} />}</span>
        ) : (
          <MButton kind="primary" disabled={!s1.reachable} onClick={onInstall}>
            Download {size}
          </MButton>
        )}
      </MRow>
      {pull && !pull.done && <MProgress label={`Downloading S1-mini… ${Math.round((pull.fraction ?? 0) * 100)}%`} fraction={pull.fraction} />}
      {s1 && !s1.reachable && <MNotice kind="error">{s1.message}</MNotice>}
      {error && (
        <MNotice kind="error" onDismiss={onDismissError}>
          S1-mini couldn't start: {error}
        </MNotice>
      )}
      {s1 && <MFootnote>{s1.credit}</MFootnote>}
    </>
  );
}

function PolishTry() {
  const [text, setText] = useState("um so the the meeting is moved to 3pm on friday you know");
  const [busy, setBusy] = useState(false);
  const [out, setOut] = useState<PolishOutcome | null>(null);
  return (
    <>
      <MRow title="Try Polish" info="Runs your current polish settings on this text. Nothing is typed anywhere.">
        <input className="mset-input mset-try-input" value={text} onChange={(e) => setText(e.target.value)} aria-label="Text to polish" />
        <MButton
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
        </MButton>
      </MRow>
      {out && (
        <MNotice kind={out.polished ? "success" : out.error ? "error" : "info"} onDismiss={() => setOut(null)}>
          {out.polished ? `${out.text} (${out.ms} ms)` : out.error ? `Not polished: ${out.error}` : "No changes needed."}
        </MNotice>
      )}
    </>
  );
}

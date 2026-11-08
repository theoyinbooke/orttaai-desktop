// Model — the Mac's ModelSettingsView: a Speech section (Express Engine,
// Transcription Model, Model Storage, Performance) and an AI Features section
// beside the title. Owned by the engine area.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AI_SETTINGS_ROUTE, takeAISection } from "../lib/navigation";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ConfirmDialog, PageHeader, Segmented, Spinner, Switch } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import { formatBytes, useDownloads, useExpressInUse, type DownloadState, type HardwareInfo, type ModelInfo } from "../lib/engine";
import { usePatch } from "../components/SettingsRows";
import ExpressCard, {
  Glyph,
  MButton,
  MCard,
  MDivider,
  MFootnote,
  MNotice,
  MRow,
  MSliderRow,
  MStepperRow,
  MToggleRow,
  StatusChip,
} from "../components/ExpressCard";
import AISettings from "./settings/AI";
import "../styles/engine.css";

type Storage = { dir: string; default_dir: string; custom: boolean; used_bytes: number; free_gb: number };
type VadStatus = { installed: boolean; downloading: boolean; path: string; size_bytes: number };
type SortMode = "size" | "recommended";
type PageSection = "speech" | "ai";

const SECTIONS: { id: PageSection; title: string; glyph: string }[] = [
  { id: "speech", title: "Speech", glyph: "waveform" },
  { id: "ai", title: "AI Features", glyph: "brain" },
];

const PROFILE_INFO =
  "Fast: Fastest response for quick dictation.\nBalanced: Balanced speed and reliable accuracy.\nAccuracy: Higher resilience for difficult audio.";
const PROFILE_TITLE = { fast: "Fast", balanced: "Balanced", accuracy: "Accuracy" };

const SORT_KEY = "modelSortMode";
const DISMISSED_KEY = "quantizedMigrationDismissedFamilies";

function stored(key: string): string {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}
function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // The choice just won't outlive this window.
  }
}

function sortModels(models: ModelInfo[], mode: SortMode) {
  const bySize = (a: ModelInfo, b: ModelInfo) => a.size_bytes - b.size_bytes;
  if (mode === "size") return [...models].sort(bySize);
  return [...models].sort(
    (a, b) =>
      Number(b.recommended) - Number(a.recommended) ||
      Number(b.downloaded) - Number(a.downloaded) ||
      Number(b.supported) - Number(a.supported) ||
      bySize(a, b),
  );
}

/** `ggml-large-v3-turbo-q5_0` → `Whisper Large V3 Turbo`; `ggml-base.en` → `Whisper Base (English)`. */
function displayName(id: string): string {
  const raw = id.replace(/^ggml-/, "").replace(/-q\d_\d$/, "");
  const [family, ...rest] = raw.split(".");
  const name = family
    .split("-")
    .map((p) => (/^v\d/.test(p) ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)))
    .join(" ");
  return `Whisper ${name}${rest.includes("en") ? " (English)" : ""}`;
}

const sizeText = (m: ModelInfo) => (m.downloaded ? formatBytes(m.size_bytes) : `${m.approx_size_mb}MB`);

/** The curated quantized build of a full-precision model, when one exists. */
const quantizedSibling = (m: ModelInfo, all: ModelInfo[]) =>
  m.quantized ? undefined : all.find((o) => o.quantized && o.id.replace(/-q\d_\d$/, "") === m.id);

function Badge({ tone, children }: { tone: "accent" | "plain" | "dim" | "warn" | "ok"; children: string }) {
  return <span className={`mset-badge ${tone}`}>{children}</span>;
}

function ModelBadges({ m }: { m: ModelInfo }) {
  return (
    <span className="mset-badges">
      {m.recommended ? (
        <Badge tone="accent">Recommended</Badge>
      ) : m.supported ? (
        <Badge tone="plain">Supported</Badge>
      ) : (
        <Badge tone="warn">Heavy</Badge>
      )}
      <Badge tone="dim">{m.english_only ? "English" : "Multilingual"}</Badge>
      <Badge tone={m.quantized ? "ok" : "warn"}>{m.quantized ? "Quantized" : "Full precision"}</Badge>
    </span>
  );
}

function ModelRow({
  m,
  selected,
  dl,
  busy,
  onPick,
  onCancel,
  onDelete,
}: {
  m: ModelInfo;
  selected: boolean;
  dl?: DownloadState;
  busy: boolean;
  onPick: () => void;
  onCancel: () => void;
  onDelete: () => void;
}) {
  const downloading = Boolean((dl && !dl.error) || m.downloading);
  const pct = Math.round((dl?.fraction ?? 0) * 100);
  return (
    <div className="mset-model-line">
      <button
        type="button"
        className={`mset-model ${selected ? "is-selected" : ""} ${!m.supported ? "is-heavy" : ""}`}
        disabled={!m.supported || busy}
        title={m.supported ? m.description : `Needs ${m.min_ram_gb} GB of memory`}
        aria-pressed={selected}
        onClick={onPick}
      >
        {downloading ? (
          <Spinner size={13} />
        ) : (
          <span className={selected ? "mset-radio on" : "mset-radio"}>
            <Glyph name={selected ? "checkCircleFill" : "circle"} size={13} />
          </span>
        )}
        <span className="mset-model-name">{displayName(m.id)}</span>
        <span className="mset-model-size">{sizeText(m)}</span>
        <ModelBadges m={m} />
        <span className="mset-spacer" />
        {downloading ? (
          <span className="mset-model-state accent">Downloading… {pct}%</span>
        ) : selected ? (
          <span className="mset-model-state accent">Current</span>
        ) : m.downloaded ? (
          <span className="mset-model-state">Downloaded</span>
        ) : null}
      </button>
      {downloading ? (
        <MButton onClick={onCancel}>Cancel</MButton>
      ) : (
        m.downloaded &&
        !selected && (
          <MButton
            kind="danger"
            className="mset-icon-btn"
            glyph="trash"
            title="Remove this model's downloaded files"
            aria-label={`Remove downloaded files for ${displayName(m.id)}`}
            disabled={busy}
            onClick={onDelete}
          />
        )
      )}
    </div>
  );
}

function MigrationOffer({
  reclaim,
  migrating,
  disabled,
  onMigrate,
  onDismiss,
}: {
  reclaim: string;
  migrating: boolean;
  disabled: boolean;
  onMigrate: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="mset-offer">
      <span className="mset-offer-icon">
        <Glyph name="downRightCircle" size={13} />
      </span>
      <div className="mset-offer-text">
        <strong>Quantized build available</strong>
        <span>Smaller, faster, and verified before replacement.</span>
      </div>
      <span className="mset-spacer" />
      {migrating ? (
        <span className="mset-inline-status">
          <Spinner size={13} /> Migrating...
        </span>
      ) : (
        <>
          <MButton title="Dismiss this suggestion" disabled={disabled} onClick={onDismiss}>
            Keep full precision
          </MButton>
          <MButton kind="primary" title="Verify the quantized build before replacement" disabled={disabled} onClick={onMigrate}>
            Switch to quantized (~{reclaim} reclaimed)
          </MButton>
        </>
      )}
    </div>
  );
}

function SpeechSection({ onShowPolishSettings }: { onShowPolishSettings: () => void }) {
  const { settings } = useSettings();
  const save = usePatch();
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [storage, setStorage] = useState<Storage | null>(null);
  const [vad, setVad] = useState<VadStatus | null>(null);
  const [vadError, setVadError] = useState(false);
  const [hwError, setHwError] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [sort, setSortState] = useState<SortMode>(() => (stored(SORT_KEY) === "recommended" ? "recommended" : "size"));
  const [expanded, setExpanded] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<ModelInfo | null>(null);
  const [storageError, setStorageError] = useState<string | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [folder, setFolder] = useState("");
  const [dismissed, setDismissed] = useState(() => new Set(stored(DISMISSED_KEY).split(",").filter(Boolean)));
  const [migrating, setMigrating] = useState<{ full: string; quantized: string } | null>(null);
  const [migrationError, setMigrationError] = useState<string | null>(null);
  const [migrationDone, setMigrationDone] = useState<string | null>(null);
  // A click on a model that isn't here yet downloads it, then switches to it.
  const switchAfterDownload = useRef<string | null>(null);

  const setSort = (v: SortMode) => {
    setSortState(v);
    store(SORT_KEY, v);
  };

  const refreshStorage = useCallback(() => {
    setStorageError(null);
    return call<Storage>("models_storage").then(setStorage).catch((e) => setStorageError(String(e)));
  }, []);

  const refresh = useCallback((visibly = false) => {
    if (visibly) setFetching(true);
    call<ModelInfo[]>("list_models")
      .then(setModels)
      .catch((e) => setSwitchError(String(e)))
      .finally(() => setFetching(false));
    refreshStorage();
    call<VadStatus>("vad_status")
      .then((v) => {
        setVad(v);
        setVadError(false);
      })
      .catch(() => setVadError(true));
  }, [refreshStorage]);

  useEffect(() => {
    refresh(true);
    call<HardwareInfo>("hardware_info")
      .then(setHw)
      .catch(() => setHwError(true));
  }, [refresh]);

  const switchTo = useCallback(
    (id: string) => {
      setSwitchError(null);
      setSwitching(id);
      return call("use_model", { id })
        .then(() => {
          setExpanded(false);
          refresh();
        })
        .catch((e) => setSwitchError(String(e)))
        .finally(() => setSwitching(null));
    },
    [refresh],
  );

  // Replaces a full-precision build once its quantized sibling is on disk.
  const finishMigration = (full: string, quantized: string) => {
    const f = models.find((m) => m.id === full);
    const q = models.find((m) => m.id === quantized);
    const reclaimed = f && q ? formatBytes(f.size_bytes - q.size_bytes) : "";
    (settings?.model_id === full ? call("use_model", { id: quantized }) : Promise.resolve())
      .then(() => call("delete_model", { id: full }))
      .then(() => setMigrationDone(`Switched to ${displayName(quantized)} (Quantized). Reclaimed ~${reclaimed}.`))
      .catch((e) => setMigrationError(String(e)))
      .finally(() => {
        setMigrating(null);
        refresh();
      });
  };

  const onDownloaded = (id: string) => {
    refresh();
    if (switchAfterDownload.current === id) {
      switchAfterDownload.current = null;
      switchTo(id);
    }
    if (migrating?.quantized === id) finishMigration(migrating.full, id);
  };
  const { downloads, start, cancel, clear } = useDownloads(onDownloaded);

  useEvent("settings-changed", () => refresh());
  useEvent<{ id: string; error: string }>("model-error", (e) => {
    if (switchAfterDownload.current === e.id) switchAfterDownload.current = null;
    if (migrating?.quantized === e.id) {
      setMigrationError(e.error);
      setMigrating(null);
    }
    refresh();
  });
  useEvent<{ id: string }>("model-cancelled", (e) => {
    if (switchAfterDownload.current === e.id) switchAfterDownload.current = null;
    if (migrating?.quantized === e.id) setMigrating(null);
    refresh();
  });

  const sorted = useMemo(() => sortModels(models, sort), [models, sort]);
  const usingExpress = useExpressInUse();
  if (!settings) return null;

  const current = models.find((m) => m.id === settings.model_id);
  const currentName = displayName(current?.id ?? settings.model_id);
  const adv = settings.advanced_decoding;
  const vadDl = downloads["silero-vad"];
  const downloadedCount = models.filter((m) => m.downloaded).length;
  const usage =
    downloadedCount === 0
      ? "No models downloaded"
      : `${downloadedCount} model${downloadedCount === 1 ? "" : "s"} downloaded • ${formatBytes(storage?.used_bytes ?? 0)}`;
  const busy = switching !== null || migrating !== null || switchAfterDownload.current !== null;
  const rowErrors = Object.entries(downloads).filter(([id, d]) => d.error && id !== "silero-vad");

  const pick = (m: ModelInfo) => {
    if (m.downloaded && m.id === settings.model_id) return setExpanded(false);
    if (m.downloaded) return void switchTo(m.id);
    setSwitchError(null);
    switchAfterDownload.current = m.id;
    start(m.id);
  };

  const dismissOffer = (family: string) => {
    const next = new Set(dismissed).add(family);
    setDismissed(next);
    store(DISMISSED_KEY, [...next].sort().join(","));
  };

  const applyFolder = (path: string | null) =>
    call<Storage>("set_models_dir", { path })
      .then((s) => {
        setStorage(s);
        setChoosing(false);
        setFolder("");
        refresh();
      })
      .catch((e) => setStorageError(String(e)));

  const chooseFolder = () =>
    openDialog({ directory: true, defaultPath: storage?.dir })
      .then((picked) => {
        if (typeof picked === "string") applyFolder(picked);
      })
      .catch((e) => setStorageError(String(e)));

  // Express ignores the Whisper decoding controls below (Decode Threads still applies).
  const whisperOnly = (info: string) => (usingExpress ? `Whisper only. ${info}` : info);

  const computeLabel = !hw ? (hwError ? "Unknown" : "…") : hw.gpu_backend === "cuda" ? "NVIDIA GPU (CUDA)" : hw.gpu_backend === "vulkan" ? "GPU (Vulkan)" : "CPU";

  return (
    <>
      <ExpressCard onShowPolishSettings={onShowPolishSettings} />

      <MCard
        title="Transcription Model"
        info="The speech-recognition model that turns your voice into text. Larger models are more accurate but slower and use more memory."
        accessory={
          <span className="mset-accessory">
            {fetching && <Spinner size={13} />}
            <Segmented
              value={sort}
              options={[
                { value: "size", label: "Size" },
                { value: "recommended", label: "Recommended" },
              ]}
              onChange={setSort}
            />
            <MButton
              className="mset-icon-btn"
              glyph="refresh"
              title="Refresh the model list"
              aria-label="Refresh model list"
              disabled={fetching}
              onClick={() => refresh(true)}
            />
          </span>
        }
      >
        <div className="mset-picker">
          <button
            type="button"
            className="mset-trigger"
            aria-expanded={expanded}
            aria-label={`Current model: ${currentName}`}
            onClick={() => setExpanded((v) => !v)}
          >
            <span className="mset-trigger-text">
              <span className="mset-trigger-title">
                <span className="mset-model-name">{currentName}</span>
                {current && <ModelBadges m={current} />}
              </span>
              <span className="mset-trigger-meta">
                {current ? `${sizeText(current)} • ${current.speed} • ${current.accuracy} accuracy` : "No model selected"}
              </span>
            </span>
            <span className="mset-spacer" />
            <span className="mset-trigger-chev">
              <Glyph name={expanded ? "chevronUp" : "chevronDown"} size={11} />
            </span>
          </button>

          {expanded && (
            <div className="mset-model-list">
              {sorted.map((m) => {
                const sibling = quantizedSibling(m, models);
                const offerable = m.downloaded && sibling && !dismissed.has(m.id);
                return (
                  <div key={m.id} className="mset-model-item">
                    <ModelRow
                      m={m}
                      selected={m.id === settings.model_id}
                      dl={downloads[m.id]}
                      busy={busy && !(downloads[m.id] && !downloads[m.id].error)}
                      onPick={() => pick(m)}
                      onCancel={() => cancel(m.id)}
                      onDelete={() => {
                        setDeleteError(null);
                        setPendingDelete(m);
                      }}
                    />
                    {offerable && sibling && (
                      <MigrationOffer
                        reclaim={formatBytes(m.size_bytes - sibling.size_bytes)}
                        migrating={migrating?.full === m.id}
                        disabled={busy}
                        onDismiss={() => dismissOffer(m.id)}
                        onMigrate={() => {
                          setMigrationError(null);
                          setMigrationDone(null);
                          setMigrating({ full: m.id, quantized: sibling.id });
                          if (sibling.downloaded) finishMigration(m.id, sibling.id);
                          else start(sibling.id);
                        }}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          )}

          {models.length === 0 && !fetching && <MFootnote>Loading models…</MFootnote>}
          {switching && (
            <span className="mset-inline-status">
              <Spinner size={13} /> Preparing {displayName(switching)}...
            </span>
          )}
          {switchError && (
            <MNotice kind="error" onDismiss={() => setSwitchError(null)}>
              Couldn't switch models: {switchError}
            </MNotice>
          )}
          {rowErrors.map(([id, d]) => (
            <MNotice key={id} kind="error" onDismiss={() => clear(id)}>
              Couldn't download {displayName(id)}: {d.error}
            </MNotice>
          ))}
          {deleteError && (
            <MNotice kind="error" onDismiss={() => setDeleteError(null)}>
              Couldn't remove the model: {deleteError}
            </MNotice>
          )}
          {migrationError && (
            <MNotice kind="error" onDismiss={() => setMigrationError(null)}>
              Quantized migration failed: {migrationError}
            </MNotice>
          )}
          {migrationDone && (
            <MNotice kind="success" onDismiss={() => setMigrationDone(null)}>
              {migrationDone}
            </MNotice>
          )}
          {hw?.hint && <MNotice kind="info">{hw.hint}</MNotice>}
        </div>
      </MCard>

      <MCard
        title="Model Storage"
        info="Where downloaded models are kept. Choose a folder on another drive to save space on this computer."
        accessory={<StatusChip title={storage?.custom ? "Custom folder" : "Default folder"} glyph="checkCircleFill" tone="ok" />}
      >
        <div className="mset-storage">
          <div className="mset-storage-path">
            <span className="mset-storage-icon">
              <Glyph name="drive" size={14} />
            </span>
            <span className="mset-mono" title={storage?.dir}>
              {storage?.dir ?? "…"}
            </span>
          </div>
          <span className="mset-caption" title={storage ? `${storage.free_gb.toFixed(1)} GB free on this drive` : undefined}>
            {storage ? usage : "Checking downloaded models..."}
          </span>
          {storageError && (
            <MNotice kind="error" onDismiss={() => setStorageError(null)}>
              {storageError}
            </MNotice>
          )}
          {choosing && (
            <div className="mset-folder">
              <input
                className="mset-input"
                autoFocus
                spellCheck={false}
                placeholder={storage?.default_dir ?? "Folder path"}
                value={folder}
                aria-label="Custom models folder"
                onChange={(e) => setFolder(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && folder.trim()) applyFolder(folder.trim());
                  if (e.key === "Escape") setChoosing(false);
                }}
              />
              <MButton kind="primary" disabled={!folder.trim()} onClick={() => applyFolder(folder.trim())}>
                Use Folder
              </MButton>
              <MButton onClick={() => setChoosing(false)}>Cancel</MButton>
            </div>
          )}
          <div className="mset-storage-actions">
            <MButton glyph="folderPlus" onClick={chooseFolder}>
              Choose Folder…
            </MButton>
            <MButton
              title="Type a folder path instead"
              onClick={() => {
                setFolder(storage?.custom ? storage.dir : "");
                setChoosing((v) => !v);
              }}
            >
              Enter Path…
            </MButton>
            <MButton
              glyph="folder"
              disabled={!storage}
              onClick={() => storage && revealItemInDir(storage.dir).catch((e) => setStorageError(String(e)))}
            >
              Show in Folder
            </MButton>
            {storage?.custom && <MButton onClick={() => applyFolder(null)}>Use Default</MButton>}
            <span className="mset-spacer" />
            <MButton
              className="mset-icon-btn"
              glyph="refresh"
              title="Refresh model storage status"
              aria-label="Refresh model storage status"
              onClick={refreshStorage}
            />
          </div>
        </div>
      </MCard>

      <MCard title="Performance" info="Changes apply from your next dictation.">
        {usingExpress && (
          <MFootnote>Decoding Profile, Advanced Decoding and Recognize Dictionary Words are Whisper only. Express uses Word Boost for dictionary words.</MFootnote>
        )}
        <MRow title="Compute Mode" info="Which chip runs the speech model on this computer. It follows the Orttaai build you installed.">
          <span className="mset-value">{computeLabel}</span>
        </MRow>
        {hwError && !hw && <MFootnote>Couldn't load this computer's hardware details.</MFootnote>}
        <MDivider />
        <MStepperRow
          title="Decode Threads"
          info={
            usingExpress
              ? "CPU threads for transcription. Auto picks about your performance cores. Express uses the new count after the engine restarts."
              : "CPU threads for transcription. Auto picks about your performance cores."
          }
          value={settings.decode.n_threads}
          min={0}
          max={Math.max(4, hw?.logical_cores ?? 16)}
          format={(v) => (v === 0 ? "Auto" : String(v))}
          onChange={(v) => save({ decode: { n_threads: v } })}
        />
        <MDivider />
        <MRow title="Decoding Profile" info={whisperOnly(PROFILE_INFO)} disabled={usingExpress}>
          <Segmented
            value={settings.decode.preset}
            disabled={usingExpress}
            options={[
              { value: "fast", label: "Fast" },
              { value: "balanced", label: "Balanced" },
              { value: "accuracy", label: "Accuracy" },
            ]}
            onChange={(v) => save({ decode: { preset: v } })}
          />
        </MRow>
        <MDivider />
        <MToggleRow
          k="warm_models"
          title="Keep the Model Ready"
          info="Runs a silent warm-up when the engine starts so the first dictation is as fast as the rest."
        />
        <MDivider />
        <MToggleRow
          k="vocabulary_bias"
          title="Recognize Dictionary Words"
          info={
            usingExpress
              ? "Whisper only. Express uses Word Boost for dictionary words."
              : "Primes the recognizer with your dictionary's names and terms so they're heard right."
          }
          disabled={usingExpress}
        />
        <MDivider />
        <MRow
          title="Silero VAD"
          info="Tells your voice from silence however loud or quiet the microphone, so hands-free recordings stop when you've finished. Without it Orttaai listens for loudness instead."
        >
          {!vad && vadError ? (
            <span className="mset-caption">Couldn't load the voice detector status.</span>
          ) : vad?.installed ? (
            <span className="mset-ready">
              <Glyph name="checkCircleFill" size={11} /> Installed
            </span>
          ) : vadDl && !vadDl.error ? (
            <span className="mset-caption mset-tabular">{Math.round(vadDl.fraction * 100)}%</span>
          ) : (
            <MButton glyph="downloadCircle" onClick={() => start("silero-vad")}>
              Download ({formatBytes(vad?.size_bytes ?? 885098)})
            </MButton>
          )}
        </MRow>
        {vadDl?.error && (
          <MNotice kind="warning" onDismiss={() => clear("silero-vad")}>
            Couldn't download the voice detector: {vadDl.error}
          </MNotice>
        )}
        <MDivider />
        <MRow
          title="Advanced Decoding"
          info={whisperOnly("Replaces the decoding profile with manual Whisper settings. Leave it off unless you are tuning accuracy.")}
          disabled={usingExpress}
        >
          <Switch
            checked={adv.enabled}
            disabled={usingExpress}
            label="Advanced Decoding"
            onChange={(v) => save({ advanced_decoding: { enabled: v } })}
          />
        </MRow>
        {adv.enabled && (
          <>
            <MFootnote>
              {PROFILE_TITLE[settings.decode.preset]} stays selected, but its values are paused while Advanced Decoding is on.
            </MFootnote>
            <div className="mset-advanced">
              <MSliderRow
                title="Temperature"
                info="Higher values increase randomness. Lower is more deterministic."
                value={adv.temperature}
                min={0}
                max={1}
                step={0.05}
                format={(v) => v.toFixed(2)}
                disabled={usingExpress}
                onCommit={(v) => save({ advanced_decoding: { temperature: v } })}
              />
              <MStepperRow
                title="Top-K"
                info="Candidates compared when a fallback samples above zero temperature."
                value={adv.best_of}
                min={1}
                max={10}
                disabled={usingExpress}
                onChange={(v) => save({ advanced_decoding: { best_of: v } })}
              />
              <MStepperRow
                title="Fallback Count"
                info="Retry attempts when decode confidence is low."
                value={adv.temperature_fallback_count}
                min={0}
                max={10}
                disabled={usingExpress}
                onChange={(v) => save({ advanced_decoding: { temperature_fallback_count: v } })}
              />
              <MSliderRow
                title="No-Speech Threshold"
                info="Higher values make silence detection stricter."
                value={adv.no_speech_threshold}
                min={0}
                max={1}
                step={0.05}
                format={(v) => v.toFixed(2)}
                disabled={usingExpress}
                onCommit={(v) => save({ advanced_decoding: { no_speech_threshold: v } })}
              />
              <MSliderRow
                title="Log-Prob Threshold"
                info="Minimum token confidence before a fallback triggers."
                value={adv.logprob_threshold}
                min={-3}
                max={0}
                step={0.1}
                format={(v) => v.toFixed(1)}
                disabled={usingExpress}
                onCommit={(v) => save({ advanced_decoding: { logprob_threshold: v } })}
              />
              <MSliderRow
                title="Compression Threshold"
                info="Detects repetitive output. Lower values can trigger more fallbacks."
                value={adv.compression_ratio_threshold}
                min={1.5}
                max={4}
                step={0.1}
                format={(v) => v.toFixed(1)}
                disabled={usingExpress}
                onCommit={(v) => save({ advanced_decoding: { compression_ratio_threshold: v } })}
              />
            </div>
          </>
        )}
      </MCard>

      <ConfirmDialog
        open={pendingDelete !== null}
        title="Remove Downloaded Model?"
        body="Removes the model from this shared folder. You can download it again."
        confirmLabel="Remove Downloaded Files"
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          const id = pendingDelete!.id;
          setPendingDelete(null);
          call("delete_model", { id })
            .then(() => refresh())
            .catch((e) => setDeleteError(String(e)));
        }}
      />
    </>
  );
}

export default function Models() {
  const [section, setSection] = useState<PageSection>(() => (takeAISection() ? "ai" : "speech"));
  useEvent<string>("navigate", (route) => {
    if (route === AI_SETTINGS_ROUTE) {
      takeAISection();
      setSection("ai");
    }
  });
  const [aiOpened, setAiOpened] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  const show = (next: PageSection) => {
    setSection(next);
    if (next === "ai") setAiOpened(true);
    // Each section starts at its top, as the Mac's page does.
    root.current?.closest(".content")?.scrollTo({ top: 0 });
  };

  return (
    <div className="view" ref={root}>
      <PageHeader
        title="Model"
        leading={
          <div className="mset-tabs" role="tablist" aria-label="Model sections">
            {SECTIONS.map((s) => (
              <button
                key={s.id}
                type="button"
                role="tab"
                aria-selected={section === s.id}
                className={section === s.id ? "active" : ""}
                onClick={() => show(s.id)}
              >
                <Glyph name={s.glyph} size={11} />
                {s.title}
              </button>
            ))}
          </div>
        }
      />
      <div className="mset mset-column" hidden={section !== "speech"}>
        <SpeechSection onShowPolishSettings={() => show("ai")} />
      </div>
      {aiOpened && (
        <div className="mset-column" hidden={section !== "ai"}>
          <AISettings />
        </div>
      )}
    </div>
  );
}


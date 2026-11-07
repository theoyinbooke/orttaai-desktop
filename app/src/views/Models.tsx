// Models — speech models on this computer (Mac ModelSettingsView, speech half):
// hardware card, model list (sort, download/cancel/delete/use), decoding
// profile, Advanced Decoding, Low Latency, warm-up, storage folder and the
// Silero voice-detector status. Owned by the engine area.
import { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Button, Card, ConfirmDialog, Icon, PageHeader, Segmented, Switch, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import {
  formatBytes,
  useDownloads,
  type DownloadState,
  type HardwareInfo,
  type ModelInfo,
} from "../lib/engine";
import { Row, Section, SliderRow, ToggleRow, usePatch } from "../components/SettingsRows";
import ExpressCard from "../components/ExpressCard";
import "../styles/engine.css";

type Storage = { dir: string; default_dir: string; custom: boolean; used_bytes: number; free_gb: number };
type VadStatus = { installed: boolean; downloading: boolean; path: string; size_bytes: number };

const PRESET_INFO: Record<string, string> = {
  fast: "Fast: quickest results, fine for short notes.",
  balanced: "Balanced: the default — accurate without waiting.",
  accuracy: "Accuracy: retries hard audio at higher temperatures. Slower.",
};

function sortModels(models: ModelInfo[], mode: "recommended" | "size") {
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

export function HardwareCard({ hw }: { hw: HardwareInfo | null }) {
  if (!hw) {
    return (
      <Card className="hw-card">
        <span className="muted small">Checking this computer…</span>
      </Card>
    );
  }
  const gpu = hw.gpus[0];
  return (
    <Card className="hw-card">
      <div className="hw-head">
        <div>
          <span className="overline">This computer</span>
          <h3 className="hw-cpu">{hw.cpu_model || "Processor"}</h3>
        </div>
        <Badge tone="accent">{hw.tier_label}</Badge>
      </div>
      <div className="hw-grid">
        <div>
          <span className="stat-value">{hw.logical_cores}</span>
          <span className="stat-label">
            threads{hw.physical_cores ? ` · ${hw.physical_cores} cores` : ""}
          </span>
        </div>
        <div>
          <span className="stat-value">{hw.ram_gb || "—"}</span>
          <span className="stat-label">GB memory</span>
        </div>
        <div>
          <span className="stat-value">{hw.free_disk_gb.toFixed(hw.free_disk_gb < 10 ? 1 : 0)}</span>
          <span className="stat-label">GB free</span>
        </div>
        <div>
          <span className="stat-value hw-gpu">{gpu ? `${Math.round(gpu.vram_mb / 1024)} GB` : "—"}</span>
          <span className="stat-label">{gpu ? gpu.name : "No NVIDIA GPU"}</span>
        </div>
      </div>
      <p className="small muted">
        Speech runs on the {hw.gpu_backend === "cuda" ? "NVIDIA GPU (CUDA)" : "CPU"}. Everything stays on this
        computer.
      </p>
      {hw.hint && <div className="eng-notice">{hw.hint}</div>}
    </Card>
  );
}

function ModelRow({
  m,
  active,
  dl,
  onDownload,
  onCancel,
  onUse,
  onDelete,
}: {
  m: ModelInfo;
  active: boolean;
  dl?: DownloadState;
  onDownload: () => void;
  onCancel: () => void;
  onUse: () => void;
  onDelete: () => void;
}) {
  const downloading = (dl && !dl.error) || m.downloading;
  return (
    <div className={`model-row ${active ? "is-active" : ""}`}>
      <div className="model-main">
        <div className="model-title">
          <span className="model-name">{m.name}</span>
          {m.recommended && <Badge tone="accent">Recommended</Badge>}
          {active && <Badge tone="accent">In use</Badge>}
          {m.english_only && <Badge>English</Badge>}
          {!m.supported && <Badge tone="warn">Needs {m.min_ram_gb} GB RAM</Badge>}
        </div>
        <span className="small muted">
          {formatBytes(m.size_bytes)} • {m.speed} • {m.accuracy} accuracy — {m.description}
        </span>
        {downloading && (
          <div className="dl-line">
            <div className="dl-track">
              <div className="dl-bar" style={{ width: `${Math.round((dl?.fraction ?? 0) * 100)}%` }} />
            </div>
            <span className="small mono">
              {Math.round((dl?.fraction ?? 0) * 100)}%
              {dl && dl.total > 0 ? ` · ${formatBytes(dl.downloaded)} of ${formatBytes(dl.total)}` : ""}
            </span>
          </div>
        )}
        {dl?.error && <span className="small eng-err">Couldn't download: {dl.error}</span>}
      </div>
      <div className="model-actions">
        {downloading ? (
          <Button size="sm" variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        ) : m.downloaded ? (
          <>
            {!active && (
              <Button size="sm" variant="primary" onClick={onUse}>
                Use
              </Button>
            )}
            <Button size="sm" variant="ghost" icon="trash" onClick={onDelete} aria-label={`Delete ${m.name}`} />
          </>
        ) : (
          <Button size="sm" icon="download" onClick={onDownload}>
            {dl?.error ? "Retry" : "Download"}
          </Button>
        )}
      </div>
    </div>
  );
}

export default function Models() {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [storage, setStorage] = useState<Storage | null>(null);
  const [vad, setVad] = useState<VadStatus | null>(null);
  const [sort, setSort] = useState<"recommended" | "size">("recommended");
  const [confirmDelete, setConfirmDelete] = useState<ModelInfo | null>(null);
  const [folder, setFolder] = useState("");

  const refresh = useCallback(() => {
    call<ModelInfo[]>("list_models").then(setModels).catch((e) => toast(String(e), "error"));
    call<Storage>("models_storage").then(setStorage).catch(() => {});
    call<VadStatus>("vad_status").then(setVad).catch(() => {});
  }, [toast]);

  useEffect(() => {
    refresh();
    call<HardwareInfo>("hardware_info").then(setHw).catch(() => {});
  }, [refresh]);
  useEffect(() => {
    if (storage) setFolder(storage.custom ? storage.dir : "");
  }, [storage?.dir]); // eslint-disable-line react-hooks/exhaustive-deps

  const { downloads, start, cancel } = useDownloads(() => refresh());
  useEvent("settings-changed", () => refresh());
  useEvent("model-error", () => refresh());
  useEvent("model-cancelled", () => refresh());

  const sorted = useMemo(() => sortModels(models, sort), [models, sort]);
  if (!settings) return null;
  const adv = settings.advanced_decoding;
  const vadDl = downloads["silero-vad"];

  const use = (id: string) =>
    call("use_model", { id })
      .then(() => toast("Speech model switched. It applies from your next dictation.", "success"))
      .catch((e) => toast(String(e), "error"));

  const applyFolder = (path: string | null) =>
    call<Storage>("set_models_dir", { path })
      .then((s) => {
        setStorage(s);
        refresh();
        toast(path ? "Models folder changed." : "Using the default models folder.", "success");
      })
      .catch((e) => toast(String(e), "error"));

  return (
    <div className="view">
      <PageHeader
        title="Models"
        desc="Speech models on this computer. Everything is transcribed locally."
        actions={
          <Segmented
            value={sort}
            options={[
              { value: "recommended", label: "Recommended" },
              { value: "size", label: "Size" },
            ]}
            onChange={setSort}
          />
        }
      />

      <HardwareCard hw={hw} />

      <ExpressCard />

      <Section
        title="Speech Models"
        desc={
          storage
            ? `${models.filter((m) => m.downloaded).length} downloaded • ${formatBytes(storage.used_bytes)} on disk`
            : undefined
        }
      >
        {sorted.map((m) => (
          <ModelRow
            key={m.id}
            m={m}
            active={m.id === settings.model_id}
            dl={downloads[m.id]}
            onDownload={() => start(m.id)}
            onCancel={() => cancel(m.id)}
            onUse={() => use(m.id)}
            onDelete={() => setConfirmDelete(m)}
          />
        ))}
      </Section>

      <Section title="Performance" desc="Changes apply from your next dictation.">
        <ToggleRow
          k="low_latency"
          label="Low Latency Mode"
          hint="Optimizes for the fastest result. Accuracy may drop slightly on difficult audio, and the dictation language can't be Auto-detect."
        />
        <Row label="Decoding Profile" hint={PRESET_INFO[settings.decode.preset]} disabled={adv.enabled}>
          <Segmented
            value={settings.decode.preset}
            options={[
              { value: "fast", label: "Fast" },
              { value: "balanced", label: "Balanced" },
              { value: "accuracy", label: "Accuracy" },
            ]}
            onChange={(v) => save({ decode: { preset: v } })}
            disabled={adv.enabled}
          />
        </Row>
        <ToggleRow
          k="warm_models"
          label="Keep the Model Ready"
          hint="Runs a silent warm-up when the engine starts so the first dictation is as fast as the rest."
        />
        <ToggleRow
          k="vocabulary_bias"
          label="Recognize Dictionary Words"
          hint="Primes the recognizer with your dictionary's names and terms so they're heard right."
        />
        <SliderRow
          label="Decode Threads"
          hint="CPU threads for transcription. Auto picks about your performance cores."
          value={settings.decode.n_threads}
          min={0}
          max={Math.max(4, hw?.logical_cores ?? 16)}
          format={(v) => (v === 0 ? "Auto" : String(v))}
          onCommit={(v) => save({ decode: { n_threads: v } })}
        />
      </Section>

      <Section title="Advanced Decoding">
        <Row
          label="Advanced Decoding"
          hint="Replaces the decoding profile with manual Whisper settings. Leave it off unless you are tuning accuracy."
        >
          <Switch
            checked={adv.enabled}
            label="Advanced decoding"
            onChange={(v) => save({ advanced_decoding: { enabled: v } })}
          />
        </Row>
        {adv.enabled && (
          <>
            <SliderRow
              label="Temperature"
              hint="Higher values increase randomness. Lower is more deterministic."
              value={adv.temperature}
              min={0}
              max={1}
              step={0.1}
              format={(v) => v.toFixed(1)}
              onCommit={(v) => save({ advanced_decoding: { temperature: v } })}
            />
            <SliderRow
              label="Top-K"
              hint="Candidates compared when a fallback samples above zero temperature."
              value={adv.best_of}
              min={1}
              max={10}
              onCommit={(v) => save({ advanced_decoding: { best_of: v } })}
            />
            <SliderRow
              label="Fallback Count"
              hint="Retry attempts when decode confidence is low."
              value={adv.temperature_fallback_count}
              min={0}
              max={10}
              onCommit={(v) => save({ advanced_decoding: { temperature_fallback_count: v } })}
            />
            <SliderRow
              label="No-Speech Threshold"
              hint="Higher values make silence detection stricter."
              value={adv.no_speech_threshold}
              min={0}
              max={1}
              step={0.05}
              format={(v) => v.toFixed(2)}
              onCommit={(v) => save({ advanced_decoding: { no_speech_threshold: v } })}
            />
            <SliderRow
              label="Log-Prob Threshold"
              hint="Minimum token confidence before a fallback triggers."
              value={adv.logprob_threshold}
              min={-5}
              max={0}
              step={0.1}
              format={(v) => v.toFixed(1)}
              onCommit={(v) => save({ advanced_decoding: { logprob_threshold: v } })}
            />
            <SliderRow
              label="Compression Threshold"
              hint="Detects repetitive output. Lower values can trigger more fallbacks."
              value={adv.compression_ratio_threshold}
              min={1}
              max={5}
              step={0.1}
              format={(v) => v.toFixed(1)}
              onCommit={(v) => save({ advanced_decoding: { compression_ratio_threshold: v } })}
            />
          </>
        )}
      </Section>

      <Section title="Voice Detector">
        <Row
          label="Silero VAD"
          hint="Tells your voice from silence however loud or quiet the microphone, so hands-free recordings stop when you've finished. Without it Orttaai listens for loudness instead."
        >
          {vad?.installed ? (
            <Badge tone="accent">
              <Icon name="check" size={12} /> Installed
            </Badge>
          ) : vadDl && !vadDl.error ? (
            <span className="small mono">{Math.round(vadDl.fraction * 100)}%</span>
          ) : (
            <Button size="sm" icon="download" onClick={() => start("silero-vad")}>
              Download ({formatBytes(vad?.size_bytes ?? 885098)})
            </Button>
          )}
        </Row>
        {vadDl?.error && <div className="eng-notice warn">Couldn't download the voice detector: {vadDl.error}</div>}
      </Section>

      <Section title="Storage">
        <Row label="Models Folder" hint={storage ? storage.dir : "…"}>
          <span className="small muted">{storage ? `${storage.free_gb.toFixed(1)} GB free` : ""}</span>
        </Row>
        <Row
          label="Custom Folder"
          hint="Keep models on another drive. Models already in that folder are picked up; existing downloads stay where they are."
        >
          <div className="eng-folder">
            <input
              className="text-input"
              placeholder={storage?.default_dir ?? "Folder path"}
              value={folder}
              onChange={(e) => setFolder(e.target.value)}
              aria-label="Custom models folder"
            />
            <Button size="sm" onClick={() => applyFolder(folder.trim() || null)} disabled={!folder.trim()}>
              Use Folder
            </Button>
            {storage?.custom && (
              <Button size="sm" variant="ghost" onClick={() => applyFolder(null)}>
                Use Default
              </Button>
            )}
          </div>
        </Row>
      </Section>

      <ConfirmDialog
        open={confirmDelete !== null}
        title={`Delete ${confirmDelete?.name ?? "model"}?`}
        body={
          confirmDelete?.id === settings.model_id
            ? "This is the model in use. Dictation keeps working until the engine restarts, then you'll need another model."
            : `Frees ${formatBytes(confirmDelete?.size_bytes ?? 0)}. You can download it again any time.`
        }
        confirmLabel="Delete"
        onCancel={() => setConfirmDelete(null)}
        onConfirm={() => {
          const id = confirmDelete!.id;
          setConfirmDelete(null);
          call("delete_model", { id })
            .then(refresh)
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </div>
  );
}


// Live engine state shared by the shell, Home, Settings, Models and Setup.
// Owned by the engine area. Backend events (app/src-tauri/src/engine.rs):
//   engine-state (phase) · engine-status (EngineSnapshot) · audio-level ·
//   audio-monitor-level · sound-cue · transcript · model-progress/done/error/cancelled
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { call, useEvent } from "./api";
import "../styles/engine.css";

export type EngineState = "off" | "loading" | "idle" | "recording" | "processing" | "error";

export const STATE_LABEL: Record<EngineState, string> = {
  off: "Off",
  loading: "Starting…",
  idle: "Ready",
  recording: "Recording",
  processing: "Transcribing",
  error: "Error",
};

export type CoordinatorStatus = {
  state: "idle" | "recording" | "processing" | "error";
  mode: "push_to_talk" | "hands_free";
  kind: "dictation" | "edit";
  elapsed_secs: number;
  countdown: number | null;
  estimate_secs: number | null;
  error: string | null;
  recovering: boolean;
  target_app: string | null;
};

export type EngineSnapshot = {
  running: boolean;
  phase: EngineState;
  status: CoordinatorStatus | null;
  model_id: string | null;
  hotkey: string;
  edit_hotkey: string | null;
  hotkey_backend: string | null;
  hotkey_error: string | null;
  hotkeys_suspended: boolean;
  injector: string | null;
  clipboard: string | null;
  voice_detector: boolean;
  last_transcript: string | null;
  wayland: boolean;
  portal_hotkeys: boolean;
};

type Ctx = {
  engine: EngineState;
  /** Recording level 0..1 (0 when not recording). */
  level: number;
  /** Settings/Setup meter level 0..1 (falls back to the recording level). */
  monitorLevel: number;
  lastTranscript: string;
  snapshot: EngineSnapshot | null;
  refresh: () => void;
};

const EngineCtx = createContext<Ctx>({
  engine: "off",
  level: 0,
  monitorLevel: 0,
  lastTranscript: "",
  snapshot: null,
  refresh: () => {},
});

/* ------------------------------------------------------------ sound cues -- */
// Short synthesized tones (Mac cues: Glass, Pop, Funk, Tink) — WebAudio, so the
// app needs no audio-output dependency.
let audioCtx: AudioContext | null = null;
function tone(freqs: number[], dur: number, gain: number, type: OscillatorType = "sine", gap = 0.0) {
  try {
    audioCtx = audioCtx ?? new AudioContext();
    const ctx = audioCtx;
    if (ctx.state === "suspended") void ctx.resume();
    let t = ctx.currentTime + 0.01;
    for (const f of freqs) {
      const osc = ctx.createOscillator();
      const g = ctx.createGain();
      osc.type = type;
      osc.frequency.setValueAtTime(f, t);
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(gain, t + 0.008);
      g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
      osc.connect(g).connect(ctx.destination);
      osc.start(t);
      osc.stop(t + dur + 0.02);
      t += dur * 0.55 + gap;
    }
  } catch {
    /* audio output unavailable — cues are best-effort */
  }
}

export function playCue(kind: string) {
  switch (kind) {
    case "start": // Glass, 0.6
      tone([1046, 1568], 0.16, 0.6 * 0.18);
      break;
    case "stop": // Pop, 0.45
      tone([660, 440], 0.09, 0.45 * 0.2);
      break;
    case "error": // Funk, 0.5
      tone([233, 196], 0.14, 0.5 * 0.12, "triangle", 0.02);
      break;
    case "hands_free": // Tink, 0.35
      tone([2093], 0.08, 0.35 * 0.18);
      break;
  }
}

export function EngineProvider({ children }: { children: ReactNode }) {
  const [engine, setEngine] = useState<EngineState>("off");
  const [level, setLevel] = useState(0);
  const [monitorLevel, setMonitor] = useState(0);
  const [lastTranscript, setLast] = useState("");
  const [snapshot, setSnapshot] = useState<EngineSnapshot | null>(null);

  const refresh = useCallback(() => {
    call<EngineSnapshot>("engine_status")
      .then((s) => {
        setSnapshot(s);
        setEngine(s.phase);
        if (s.last_transcript) setLast(s.last_transcript);
      })
      .catch(() => {});
  }, []);
  useEffect(refresh, [refresh]);

  useEvent<string>("engine-state", (s) => setEngine(s as EngineState));
  useEvent<EngineSnapshot>("engine-status", setSnapshot);
  useEvent<number>("audio-level", setLevel);
  useEvent<number>("audio-monitor-level", setMonitor);
  useEvent<string>("transcript", setLast);
  useEvent<string>("sound-cue", playCue);

  return (
    <EngineCtx.Provider
      value={{
        engine,
        level,
        monitorLevel: engine === "recording" ? Math.max(level, monitorLevel) : monitorLevel,
        lastTranscript,
        snapshot,
        refresh,
      }}
    >
      {children}
    </EngineCtx.Provider>
  );
}

export const useEngine = () => useContext(EngineCtx);

/* ---------------------------------------------------------- downloads ----- */

export type DownloadState = { fraction: number; downloaded: number; total: number; error?: string };

/** Live download progress keyed by model id (shared by Models and Setup). */
export function useDownloads(onDone?: (id: string) => void) {
  const [downloads, setDownloads] = useState<Record<string, DownloadState>>({});
  const done = useRef(onDone);
  done.current = onDone;
  useEvent<{ id: string; fraction: number; downloaded: number; total: number }>("model-progress", (p) =>
    setDownloads((d) => ({ ...d, [p.id]: { fraction: p.fraction, downloaded: p.downloaded, total: p.total } })),
  );
  const clear = (id: string) =>
    setDownloads((d) => {
      const { [id]: _gone, ...rest } = d;
      return rest;
    });
  useEvent<{ id: string }>("model-done", (p) => {
    clear(p.id);
    done.current?.(p.id);
  });
  useEvent<{ id: string }>("model-cancelled", (p) => clear(p.id));
  useEvent<{ id: string; error: string }>("model-error", (p) =>
    setDownloads((d) => ({ ...d, [p.id]: { fraction: 0, downloaded: 0, total: 0, error: p.error } })),
  );
  const start = useCallback((id: string) => {
    setDownloads((d) => ({ ...d, [id]: { fraction: 0, downloaded: 0, total: 0 } }));
    call("download_model", { id }).catch(() => {});
  }, []);
  const cancel = useCallback((id: string) => {
    call<boolean>("cancel_download", { id }).catch(() => {});
  }, []);
  return { downloads, start, cancel, clear };
}

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}

/* ------------------------------------------------------------ models ------ */

export type ModelInfo = {
  id: string;
  name: string;
  approx_size_mb: number;
  size_bytes: number;
  sha256: string;
  multilingual: boolean;
  english_only: boolean;
  quantized: boolean;
  url: string;
  speed: "Fastest" | "Fast" | "Moderate" | "Slow";
  accuracy: "Basic" | "Good" | "Great" | "Best";
  description: string;
  min_ram_gb: number;
  downloaded: boolean;
  path: string;
  recommended: boolean;
  supported: boolean;
  downloading: boolean;
};

export type HardwareInfo = {
  cpu_model: string;
  logical_cores: number;
  physical_cores: number | null;
  ram_gb: number;
  gpus: { name: string; vram_mb: number }[];
  gpu_backend: "cpu" | "cuda" | "vulkan";
  free_disk_gb: number;
  tier: "entry" | "standard" | "performance" | "gpu";
  tier_label: string;
  recommended_model: string;
  hint: string | null;
};

export type AudioDevice = { name: string; is_default: boolean };

/** Start the engine, surfacing the backend's error. */
export function startEngine() {
  return call("start_dictation", { modelPath: null });
}

/** Whole seconds → "1:05". */
export function clock(secs: number) {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

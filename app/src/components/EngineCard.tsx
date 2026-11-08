// Engine status + start/stop + click-to-record card shown on Home.
// Owned by the engine area.
import { useState } from "react";
import { Badge, Button, CopyButton, Kbd, LevelMeter, useToast } from "../ui";
import { call } from "../lib/api";
import { useSettings } from "../lib/settings";
import { clock, startEngine, STATE_LABEL, useEngine } from "../lib/engine";
import type { Tab } from "../types";

/** `ggml-base.en-q5_1` → `Base · English (Q5)`. */
export function prettyModel(id: string | null | undefined): string {
  if (!id) return "No model";
  const raw = id.replace(/^ggml-/, "");
  const q = raw.match(/-(q\d_\d)$/)?.[1];
  const base = raw.replace(/-(q\d_\d)$/, "");
  const [family, ...rest] = base.split(".");
  const name = family
    .split("-")
    .map((p) => (p.startsWith("v") && /\d/.test(p) ? p : p.charAt(0).toUpperCase() + p.slice(1)))
    .join(" ");
  const en = rest.includes("en") ? " · English" : "";
  return `${name}${en}${q ? ` (${q.slice(0, 2).toUpperCase()})` : ""}`;
}

export default function EngineCard({ onNavigate }: { onNavigate?: (t: Tab) => void }) {
  const { engine, level, lastTranscript, snapshot } = useEngine();
  const { settings } = useSettings();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const running = engine !== "off" && engine !== "loading";
  const recording = engine === "recording";
  const status = snapshot?.status;

  const start = () => {
    setBusy(true);
    startEngine()
      .catch((e) => toast(String(e), "error"))
      .finally(() => setBusy(false));
  };
  const stop = () => call("stop_dictation").catch((e) => toast(String(e), "error"));
  const toggle = () => call("toggle_recording").catch((e) => toast(String(e), "error"));

  const hint = settings?.hands_free_enabled
    ? "Hold to talk, or tap for hands-free. Tap again or pause to stop."
    : "Hold the shortcut and speak; release to insert the text.";

  return (
    <section className={`card engine-card phase-${engine}`} aria-label="Dictation engine">
      <header className="ec-head">
        <div className="ec-title">
          <span className={`ec-dot dot-${engine}`} aria-hidden="true" />
          <div>
            <span className="overline">Dictation</span>
            <h2 className="ec-state">
              {recording && status?.kind === "edit"
                ? "Recording edit instruction"
                : recording && status?.mode === "hands_free"
                  ? "Recording · hands-free"
                  : STATE_LABEL[engine]}
              {recording && <span className="ec-timer mono"> {clock(status?.elapsed_secs ?? 0)}</span>}
            </h2>
          </div>
        </div>
        <div className="ec-actions">
          {engine === "off" ? (
            <Button variant="primary" icon="play" onClick={start} loading={busy}>
              Start dictation
            </Button>
          ) : engine === "loading" ? (
            <Button variant="ghost" loading>
              Loading model…
            </Button>
          ) : (
            <Button variant="ghost" icon="stop" onClick={stop}>
              Stop engine
            </Button>
          )}
        </div>
      </header>

      {running && (
        <button
          className={`ec-record ${recording ? "is-recording" : ""}`}
          onClick={toggle}
          disabled={engine === "processing"}
          aria-label={recording ? "Stop and insert" : "Click to record"}
        >
          <span className="ec-record-dot" aria-hidden="true" />
          <span>
            {recording ? "Stop & insert" : engine === "processing" ? "Transcribing…" : "Click to record"}
          </span>
        </button>
      )}

      {recording && <LevelMeter level={level} active />}

      <div className="ec-chips">
        <span className="ec-chip">
          <Kbd combo={snapshot?.hotkey || "Ctrl+Shift+Space"} />
        </span>
        {snapshot?.edit_hotkey && (
          <span className="ec-chip" title="Edit selected text with your voice">
            <Kbd combo={snapshot.edit_hotkey} />
            <span className="small muted">edit</span>
          </span>
        )}
        <button className="ec-chip ec-chip-btn" onClick={() => onNavigate?.("models")} title="Speech model">
          <span className="small">{prettyModel(snapshot?.model_id ?? settings?.model_id)}</span>
        </button>
        {snapshot?.voice_detector && <Badge tone="neutral">Silero VAD</Badge>}
      </div>
      <p className="small muted ec-hint">{hint}</p>
      {snapshot?.hotkey_error && running && (
        <p className="small ec-warn">
          The shortcut isn't available ({snapshot.hotkey_error}). Use “Click to record” or the tray.
        </p>
      )}
      {snapshot?.wayland && running && !snapshot?.hotkey_error && (
        <p className="small muted">
          Wayland: if the shortcut does nothing, assign it in your desktop's Settings › Keyboard, or use “Click to
          record”.
        </p>
      )}

      {lastTranscript && (
        <div className="ec-last">
          <div className="ec-last-head">
            <span className="overline">Last dictation</span>
            <CopyButton text={lastTranscript} compact />
          </div>
          <p className="ec-last-text">{lastTranscript}</p>
        </div>
      )}
    </section>
  );
}

// The floating recording overlay (a separate borderless, non-focusable Tauri
// window) — the Mac pill: live waveform from `audio-level`, elapsed timer, the
// final-20-seconds countdown, hands-free and EDIT badges, a stop button, the
// processing estimate, and the error state. It never shows transcript text.
// The backend shows/hides the window (hidden before injection on Linux).
// On Windows, while dictation is ready, the backend can also keep the window
// up as the idle pill (Mac's floating handle): a small bar that shows the
// shortcut on hover and starts a hands-free dictation when clicked.
import { useEffect, useRef, useState, type MutableRefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { clock, type CoordinatorStatus, type EngineSnapshot } from "./lib/engine";
import "./styles/engine.css";

const BARS = 16;

function useLevel() {
  const level = useRef(0);
  useEffect(() => {
    const un = listen<number>("audio-level", (e) => {
      level.current = e.payload;
    });
    return () => {
      un.then((off) => off());
    };
  }, []);
  return level;
}

/** Mac WaveformView: two drifting waves + 16 level bars, drawn at ~30 fps. */
function Waveform({ countingDown, level }: { countingDown: boolean; level: MutableRefObject<number> }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    let raf = 0;
    let last = 0;
    const draw = (now: number) => {
      raf = requestAnimationFrame(draw);
      if (now - last < 33) return;
      last = now;
      const dpr = window.devicePixelRatio || 1;
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (el.width !== Math.round(w * dpr)) {
        el.width = Math.round(w * dpr);
        el.height = Math.round(h * dpr);
      }
      const ctx = el.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const styles = getComputedStyle(el);
      const tint = (countingDown ? styles.getPropertyValue("--danger") : styles.getPropertyValue("--accent")).trim();
      const t = reduce ? 0 : now / 1000;
      const lv = Math.max(0, Math.min(1, level.current));

      const wave = (amp: number, freq: number, yOff: number, phase: number, alpha: number) => {
        ctx.beginPath();
        ctx.moveTo(0, h);
        const step = Math.max(4, w / 32);
        for (let x = 0; x <= w; x += step) {
          const y = yOff + Math.sin((x / w) * freq * Math.PI * 2 + (t + phase) * 2.8) * amp;
          ctx.lineTo(x, y);
        }
        ctx.lineTo(w, h);
        ctx.closePath();
        ctx.globalAlpha = alpha;
        ctx.fillStyle = tint;
        ctx.fill();
        ctx.globalAlpha = 1;
      };
      wave(5 + lv * 6, 1.8, h * 0.48, 0, 0.14);
      wave(4 + lv * 4, 2.5, h * 0.6, 0.7, 0.08);

      const bw = 2.5;
      const gap = 2;
      const minH = 6;
      const maxH = 18;
      const boosted = Math.min(1, lv * 2.5 + 0.15);
      ctx.fillStyle = tint;
      ctx.shadowColor = tint;
      ctx.shadowBlur = 6 * lv;
      for (let i = 0; i < BARS; i++) {
        const centerDist = Math.abs(i - (BARS - 1) / 2);
        const envelope = 1 - (centerDist / ((BARS - 1) / 2)) * 0.4;
        const wv = Math.sin(i * 1.2 + lv * 12 + t * 0.8) * 0.25 + 0.75;
        const bh = minH + (maxH - minH) * boosted * wv * envelope;
        const x = 14 + i * (bw + gap);
        const y = (h - bh) / 2;
        if (typeof ctx.roundRect === "function") {
          ctx.beginPath();
          ctx.roundRect(x, y, bw, bh, bw / 2);
          ctx.fill();
        } else {
          ctx.fillRect(x, y, bw, bh);
        }
      }
      ctx.shadowBlur = 0;
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [countingDown, level]);
  return <canvas ref={canvas} className="pill-canvas" aria-hidden="true" />;
}

/** Mac FloatingPanelHintView: the idle handle, and the hint while hovered. */
function IdlePill({ hotkey, handsFree }: { hotkey: string; handsFree: boolean }) {
  const [failed, setFailed] = useState<string | null>(null);
  useEffect(() => {
    if (!failed) return;
    const t = window.setTimeout(() => setFailed(null), 4000);
    return () => window.clearTimeout(t);
  }, [failed]);
  const start = () => {
    setFailed(null);
    invoke("toggle_recording").catch((e) => setFailed(String(e)));
  };
  return (
    <div
      className={`idle-pill ${failed ? "failed" : ""}`}
      role="button"
      tabIndex={-1}
      aria-label="Start dictation"
      title={failed ?? undefined}
      // Don't let the press move focus inside the panel; the click alone starts.
      onMouseDown={(e) => e.preventDefault()}
      onClick={start}
    >
      <span className="idle-handle" aria-hidden="true" />
      <span className="idle-hint">
        <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true" className="idle-ico">
          <path
            d="M2 12h3l2.5-6 4 13 3-9 2 4H22"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        <span className="idle-text">
          <span className="idle-quiet">{handsFree ? "Hold or tap" : "Hold"}</span>
          <kbd className="idle-key">{hotkey || "your shortcut"}</kbd>
          <span className="idle-quiet">to dictate</span>
        </span>
        <span className="idle-mic" title="Start dictation" aria-hidden="true">
          <svg width="11" height="11" viewBox="0 0 24 24">
            <rect x="8" y="2" width="8" height="13" rx="4" fill="currentColor" />
            <path d="M5 11a7 7 0 0 0 14 0M12 18v4" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
          </svg>
        </span>
      </span>
    </div>
  );
}

export default function Panel() {
  const [status, setStatus] = useState<CoordinatorStatus | null>(null);
  const [phase, setPhase] = useState<string>("idle");
  const [hotkey, setHotkey] = useState("");
  const [handsFreeOn, setHandsFreeOn] = useState(true);
  // The stop button's failure, shown briefly in the pill (it has no room for more).
  const [stopError, setStopError] = useState<string | null>(null);
  const level = useLevel();

  useEffect(() => {
    if (!stopError) return;
    const t = window.setTimeout(() => setStopError(null), 4000);
    return () => window.clearTimeout(t);
  }, [stopError]);

  useEffect(() => {
    invoke<EngineSnapshot>("engine_status")
      .then((s) => {
        setStatus(s.status);
        setPhase(s.phase);
        setHotkey(s.hotkey);
      })
      .catch(() => {});
    invoke<{ hands_free_enabled: boolean }>("get_settings")
      .then((s) => setHandsFreeOn(s.hands_free_enabled))
      .catch(() => {});
    const a = listen<EngineSnapshot>("engine-status", (e) => {
      setStatus(e.payload.status);
      setHotkey(e.payload.hotkey);
    });
    const b = listen<string>("engine-state", (e) => setPhase(e.payload));
    const c = listen<{ hands_free_enabled: boolean }>("settings-changed", (e) =>
      setHandsFreeOn(e.payload.hands_free_enabled),
    );
    return () => {
      a.then((off) => off());
      b.then((off) => off());
      c.then((off) => off());
    };
  }, []);

  const state = status?.state ?? (phase as CoordinatorStatus["state"]);

  // The engine is ready (the backend only shows the window now on Windows).
  if (phase === "idle" && state === "idle") {
    return <IdlePill hotkey={hotkey} handsFree={handsFreeOn} />;
  }

  if (state === "processing") {
    return (
      <div className="pill pill-processing" role="status">
        <span className="pill-shimmer" aria-hidden="true" />
        <span className="pill-note">
          {status?.estimate_secs ? `~${status.estimate_secs}s to process` : "Processing…"}
        </span>
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="pill pill-error" role="alert">
        <svg width="15" height="15" viewBox="0 0 24 24" aria-hidden="true" className="pill-warn-ico">
          <path d="M12 3 2 20h20L12 3Z" fill="currentColor" />
          <path d="M12 9v5M12 17h.01" stroke="var(--bg-elev-solid)" strokeWidth="2" strokeLinecap="round" />
        </svg>
        <span className="pill-error-text">{status?.error ?? "Something went wrong"}</span>
      </div>
    );
  }

  const countdown = status?.countdown ?? null;
  const handsFree = status?.mode === "hands_free";
  const edit = status?.kind === "edit";
  return (
    <div className={`pill pill-recording ${countdown != null ? "counting" : ""}`}>
      <Waveform countingDown={countdown != null} level={level} />
      <div className="pill-controls">
        <span className="pill-spacer" />
        {status?.recovering && <span className="pill-badge" title="Reconnecting the microphone">Reconnecting…</span>}
        {stopError && (
          <span className="pill-badge" role="alert" title={stopError}>
            Couldn't stop
          </span>
        )}
        {edit && (
          <span className="pill-badge" title="Speak how to change the selected text" aria-label="Recording edit instruction">
            EDIT
          </span>
        )}
        {handsFree && (
          <span className="pill-infinity" title="Tap the shortcut, press stop, or pause to finish" aria-label="Hands-free recording">
            ∞
          </span>
        )}
        <span className="pill-time mono">
          {clock(status?.elapsed_secs ?? 0)}
          {countdown != null && (
            <>
              <span className="pill-dot">·</span>
              <span className="pill-countdown">{countdown}s</span>
            </>
          )}
        </span>
        <button
          className="pill-stop"
          onClick={() => {
            setStopError(null);
            invoke("stop_recording").catch((e) => setStopError(String(e)));
          }}
          title={stopError ? `Couldn't stop: ${stopError}` : "Stop dictation"}
          aria-label="Stop dictation"
        >
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <rect x="1" y="1" width="8" height="8" rx="1.5" fill="currentColor" />
          </svg>
        </button>
      </div>
    </div>
  );
}

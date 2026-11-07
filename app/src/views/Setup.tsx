// First-run setup, in its own 600 x 750 window (Mac SetupView): a connected step
// track, an animated scene, one title line, the step's content, and a footer that
// says what's still needed before Continue. The Mac's four steps, with the
// permissions step adapted to what Linux and Windows actually ask for.
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AppearanceSync, BrandMark } from "../App";
import { Button, Icon, Select, ThemeProvider, ToastProvider, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { comboToString, SettingsProvider, useSettings } from "../lib/settings";
import { EngineProvider, formatBytes, startEngine, useDownloads, useEngine, type AudioDevice, type HardwareInfo, type ModelInfo } from "../lib/engine";
import ShortcutRecorder from "../components/ShortcutRecorder";
import SetupArt from "../components/SetupArt";
import { useLevelMonitor } from "./settings/Audio";
import "../styles/setup.css";

type PlatformCheck = {
  platform: string;
  session: string;
  desktop: string | null;
  injector: string;
  clipboard: string;
  hotkeys: string;
  portal_hotkeys: boolean;
  portal_typing: boolean;
  wtype: boolean;
  wl_clipboard: boolean;
  secure_detection: boolean;
  microphones: number;
  notes: string[];
};

type ExpressModel = {
  id: string;
  name: string;
  description: string;
  approx_size_mb: number;
  multilingual: boolean;
  downloaded: boolean;
};

const STEPS = ["Welcome", "Permissions", "Engine", "Ready"];

const COPY = [
  { title: "Welcome to Orttaai", sub: "Hold a key, speak, and Orttaai types it into any app. Privately, on this computer." },
  { title: "Two quick permissions", sub: "The microphone so it can hear you, and a way to type where your cursor is." },
  { title: "Choose your voice engine", sub: "It runs entirely on this computer. Pick one now; you can switch any time." },
  { title: "You're ready", sub: "Try your first dictation right here, then start using Orttaai." },
];

/* ------------------------------------------------------------- step track -- */

function StepTrack({ current }: { current: number }) {
  return (
    <div className="step-track" role="img" aria-label={`Step ${current + 1} of ${STEPS.length}: ${STEPS[current]}`}>
      {STEPS.map((title, i) => (
        <div key={title} className="step-part">
          <div className={`step-node ${i < current ? "done" : i === current ? "current" : ""}`}>
            <span className="step-dot">
              {i === current && <i className="step-pulse" />}
              {i < current ? <Icon name="check" size={12} /> : i + 1}
            </span>
            <span className="step-title">{title}</span>
          </div>
          {i < STEPS.length - 1 && (
            <span className="step-line">
              <i className={i < current ? "filled" : ""} />
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

/* ---------------------------------------------------------------- welcome -- */

function Feature({ icon, title, detail }: { icon: string; title: string; detail: string }) {
  return (
    <div className="setup-row">
      <span className="setup-tile">
        <Icon name={icon} size={18} />
      </span>
      <div className="setup-row-text">
        <b>{title}</b>
        <span>{detail}</span>
      </div>
    </div>
  );
}

function Welcome() {
  const link = (url: string) => () => {
    openUrl(url).catch(() => {});
  };
  return (
    <div className="setup-list">
      <Feature icon="waveform" title="Hold a key, speak, let go" detail="Orttaai types what you said into whatever app you're in." />
      <Feature icon="shieldLock" title="Stays on your computer" detail="Speech and AI run on your own hardware. Nothing is uploaded." />
      <Feature icon="textCheck" title="Learns your words" detail="Names and terms you use get spelled right, and it improves as you fix them." />
      <div className="setup-credit">
        <span className="setup-appicon">
          <BrandMark />
        </span>
        <span>Made by Olanrewaju Oyinbooke</span>
        <span className="setup-credit-fill" />
        <button className="setup-ghost" onClick={link("https://github.com/theoyinbooke")}>
          <Icon name="codeBrackets" size={13} /> GitHub
        </button>
        <button className="setup-ghost" onClick={link("https://github.com/theoyinbooke/orttaai-desktop")}>
          <Icon name="external" size={13} /> Project
        </button>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------ permissions -- */

function PermissionRow({
  icon,
  title,
  detail,
  granted,
  optional,
  action,
}: {
  icon: string;
  title: string;
  detail: ReactNode;
  granted: boolean;
  optional?: boolean;
  action?: { label: string; run: () => void };
}) {
  return (
    <div className={`setup-row ${granted ? "granted" : ""}`} aria-label={`${title}, ${granted ? "Granted" : optional ? "Optional" : "Required before continuing"}`}>
      <span className={`setup-tile ${granted ? "ok" : ""}`}>
        <Icon name={icon} size={18} />
        {granted && (
          <i className="setup-tile-check">
            <Icon name="check" size={9} />
          </i>
        )}
      </span>
      <div className="setup-row-text">
        <b>
          {title}
          {optional && <em className="setup-optional">Optional</em>}
        </b>
        <span>{detail}</span>
      </div>
      {granted ? (
        <span className="setup-allowed">Allowed</span>
      ) : action ? (
        <Button size="sm" variant={optional ? "ghost" : "primary"} onClick={action.run}>
          {action.label}
        </Button>
      ) : null}
    </div>
  );
}

function Permissions({ onChange }: { onChange: (ok: boolean) => void }) {
  const { settings } = useSettings();
  const [check, setCheck] = useState<PlatformCheck | null>(null);
  const [attempt, setAttempt] = useState(0);
  const { metered, error } = useLevelMonitor(settings?.audio_device ?? null, attempt > 0);

  const recheck = useCallback(() => {
    call<PlatformCheck>("platform_check").then(setCheck).catch(() => {});
  }, []);
  // Checked again every few seconds, so plugging in a mic is noticed (the Mac polls too).
  useEffect(() => {
    recheck();
    setAttempt(1);
    const timer = window.setInterval(recheck, 3000);
    return () => window.clearInterval(timer);
  }, [recheck]);

  const micOk = !!metered && !error && (check?.microphones ?? 0) > 0;
  useEffect(() => onChange(micOk), [micOk, onChange]);

  const wayland = check?.session === "wayland";
  const typingOk = !wayland || !!check?.portal_typing || !!check?.wtype;
  const hotkeyOk = !wayland || !!check?.portal_hotkeys;
  const typingDetail = !check
    ? "So it can type into the app you're using."
    : wayland
      ? typingOk
        ? "So it can type into the app you're using. Your desktop asks once; choose Allow."
        : "Typing needs your desktop's remote-input portal, or wtype on Sway and Hyprland."
      : "So it can type into the app you're using.";

  return (
    <div className="setup-list">
      <PermissionRow
        icon="mic"
        title="Microphone"
        detail={micOk ? "So Orttaai can hear you while you hold the key." : error ?? (check && check.microphones === 0 ? "No microphone found. Connect one to continue." : "So Orttaai can hear you while you hold the key.")}
        granted={micOk}
        action={{ label: "Allow", run: () => setAttempt((a) => a + 1) }}
      />
      {check && !micOk && (error || check.microphones === 0) && (
        <div className="setup-trouble" role="status">
          <b>
            <Icon name="alert" size={13} /> Microphone not detected
          </b>
          <span>
            {check.platform === "windows"
              ? "Check Settings › Privacy & security › Microphone, then allow access for desktop apps."
              : "Check that a microphone is connected and not muted in your sound settings."}
          </span>
        </div>
      )}
      <PermissionRow
        icon="cursorRays"
        title="Typing into apps"
        detail={typingDetail}
        granted={typingOk}
        action={{ label: "Check Again", run: recheck }}
      />
      <PermissionRow
        icon="keyboard"
        title="Global shortcut"
        optional
        detail={hotkeyOk ? "Lets the key work in every app." : "If your desktop asks, assign a key to push-to-talk. You can also click to record."}
        granted={hotkeyOk}
        action={{ label: "Check Again", run: recheck }}
      />
      {wayland && (
        <>
          <PermissionRow icon="keyboard" title="wtype" optional granted={!!check?.wtype} detail={check?.wtype ? "Typing works on Sway and Hyprland too." : <>Install with <code>sudo apt install wtype</code></>} />
          <PermissionRow icon="copy" title="wl-clipboard" optional granted={!!check?.wl_clipboard} detail={check?.wl_clipboard ? "Paste and voice edit can use the clipboard." : <>Install with <code>sudo apt install wl-clipboard</code></>} />
        </>
      )}
      <p className="setup-trust">
        <Icon name="lock" size={12} /> Audio is transcribed on this computer and never uploaded.
      </p>
    </div>
  );
}

/* ----------------------------------------------------------------- engine -- */

function EngineChoice({
  icon,
  title,
  badge,
  subtitle,
  detail,
  selected,
  actionTitle,
  disabled,
  onAction,
}: {
  icon: string;
  title: string;
  badge?: string | null;
  subtitle: string;
  detail: string;
  selected: boolean;
  actionTitle: string;
  disabled: boolean;
  onAction: () => void;
}) {
  return (
    <div className={`setup-row engine ${selected ? "granted" : ""}`}>
      <span className={`setup-tile big ${selected ? "ok" : ""}`}>
        <Icon name={selected ? "check" : icon} size={19} />
      </span>
      <div className="setup-row-text">
        <b className="big">
          {title}
          {badge && <em className="setup-badge">{badge}</em>}
        </b>
        <code className="setup-id">{subtitle}</code>
        <span className="one-line" title={detail}>
          {detail}
        </span>
      </div>
      <Button size="sm" variant={selected ? "ghost" : "primary"} disabled={disabled} onClick={onAction}>
        {actionTitle}
      </Button>
    </div>
  );
}

function EngineStep({ onReady }: { onReady: (ready: boolean) => void }) {
  const { settings, patch } = useSettings();
  const { engine } = useEngine();
  const toast = useToast();
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [express, setExpress] = useState<ExpressModel[]>([]);
  const [expressProgress, setExpressProgress] = useState<number | null>(null);
  const [storage, setStorage] = useState<{ dir: string; custom: boolean; free_gb: number } | null>(null);
  const [editingFolder, setEditingFolder] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    call<ModelInfo[]>("list_models").then(setModels).catch(() => {});
    call<ExpressModel[]>("express_models").then(setExpress).catch(() => {});
    call<{ dir: string; custom: boolean; free_gb: number }>("models_storage").then(setStorage).catch(() => {});
  }, []);
  const { downloads, start } = useDownloads(load);
  useEffect(() => {
    load();
    call<HardwareInfo>("hardware_info").then(setHw).catch(() => {});
  }, [load]);

  const expressModel = express.find((m) => m.id === settings?.express_model) ?? express[0];
  const whisper = models.find((m) => m.recommended) ?? models.find((m) => m.id === settings?.model_id);
  const english = (settings?.dictation_language ?? "en").toLowerCase().startsWith("en");
  const expressAvailable = !!expressModel && english;
  const running = engine === "idle" || engine === "recording" || engine === "processing";
  const usingExpress = settings?.engine === "express" && !!expressModel?.downloaded;
  const usingWhisper = settings?.engine !== "express" && !!whisper?.downloaded && settings?.model_id === whisper.id;
  const whisperDl = whisper ? downloads[whisper.id] : undefined;
  const busy = expressProgress !== null || (!!whisperDl && !whisperDl.error);
  const ready = (usingExpress || usingWhisper) && running;

  useEffect(() => onReady(ready), [ready, onReady]);

  // Whatever is chosen, make sure the engine is up with it.
  useEffect(() => {
    if ((usingExpress || usingWhisper) && engine === "off") {
      startEngine().catch((e) => setError(`Couldn't load the engine. ${e}`));
    }
  }, [usingExpress, usingWhisper, engine]);

  const useExpress = useCallback(() => {
    patch({ engine: "express" }).catch((e) => toast(String(e), "error"));
  }, [patch, toast]);
  useEvent<{ id: string; fraction: number }>("express-progress", (e) => setExpressProgress(e.fraction));
  useEvent<{ id: string }>("express-done", () => {
    setExpressProgress(null);
    load();
    useExpress();
  });
  useEvent<{ id: string }>("express-cancelled", () => setExpressProgress(null));
  useEvent<{ id: string; error: string }>("express-error", (e) => {
    setExpressProgress(null);
    setError(`Couldn't download Express. ${e.error}`);
  });

  const pickExpress = () => {
    if (!expressModel) return;
    setError(null);
    if (expressModel.downloaded) return useExpress();
    setExpressProgress(0);
    call("express_download", { id: expressModel.id }).catch((e) => {
      setExpressProgress(null);
      setError(String(e));
    });
  };
  const pickWhisper = () => {
    if (!whisper) return;
    setError(null);
    if (!whisper.downloaded) {
      start(whisper.id);
      return;
    }
    patch({ engine: "whisper", model_id: whisper.id }).catch((e) => toast(String(e), "error"));
  };
  // A finished download selects that model (Mac: choosing downloads, then uses it).
  const pendingWhisper = !!whisperDl && !whisperDl.error;
  const [wasDownloading, setWasDownloading] = useState(false);
  useEffect(() => {
    if (pendingWhisper) setWasDownloading(true);
    else if (wasDownloading && whisper?.downloaded) {
      setWasDownloading(false);
      patch({ engine: "whisper", model_id: whisper.id }).catch(() => {});
    }
  }, [pendingWhisper, whisper?.downloaded]); // eslint-disable-line react-hooks/exhaustive-deps

  const anyInstalled = !!whisper?.downloaded || !!expressModel?.downloaded;
  const saveFolder = () => {
    const path = editingFolder?.trim() || null;
    call("set_models_dir", { path })
      .then(() => {
        setEditingFolder(null);
        load();
      })
      .catch((e) => setError(String(e)));
  };

  return (
    <div className="setup-list">
      {expressModel && (
        <EngineChoice
          icon="bolt"
          title="Express"
          badge={english ? "Recommended" : null}
          subtitle={`${expressModel.name} + Word Boost`}
          detail={english ? `Fastest dictation, best with names. English only, ${formatBytes(expressModel.approx_size_mb * 1024 * 1024)}.` : "English only. Set dictation to English to use it."}
          selected={usingExpress}
          actionTitle={usingExpress ? (running ? "Ready" : "Loading...") : expressProgress !== null ? "Downloading..." : expressModel.downloaded ? "Use Downloaded" : anyInstalled ? "Download Instead" : "Download"}
          disabled={busy || usingExpress || !english}
          onAction={pickExpress}
        />
      )}
      {whisper && (
        <EngineChoice
          icon="waveform"
          title={`Whisper ${whisper.name.split(" · ")[0]}`}
          badge={expressAvailable ? null : "Recommended"}
          subtitle={whisper.id}
          detail={`${whisper.description.replace(/\.$/, "")}, ${formatBytes(whisper.size_bytes)}.`}
          selected={usingWhisper}
          actionTitle={usingWhisper ? (running ? "Ready" : "Loading...") : whisperDl && !whisperDl.error ? "Downloading..." : whisper.downloaded ? "Use Downloaded" : anyInstalled ? "Download Instead" : "Download"}
          disabled={busy || usingWhisper}
          onAction={pickWhisper}
        />
      )}
      {!models.length && <p className="setup-note">Loading the engines…</p>}

      {(expressProgress !== null || (whisperDl && !whisperDl.error)) && (
        <div className="setup-progress" role="status">
          <div className="setup-bar">
            <i style={{ width: `${Math.round((expressProgress ?? whisperDl?.fraction ?? 0) * 100)}%` }} />
          </div>
          <div className="setup-progress-line">
            <span>{expressProgress !== null ? "Downloading Express models..." : "Downloading model files..."}</span>
            <code>{Math.round((expressProgress ?? whisperDl?.fraction ?? 0) * 100)}%</code>
          </div>
          <span className="setup-note">First-time download can take a few minutes.</span>
        </div>
      )}
      {(usingExpress || usingWhisper) && engine === "loading" && (
        <div className="setup-progress" role="status">
          <span>Loading model...</span>
          <span className="setup-note">Running a quick warm-up for faster first dictation.</span>
        </div>
      )}
      {whisperDl?.error && <p className="setup-error">Couldn't download the model. Check your connection and try again. ({whisperDl.error})</p>}
      {error && <p className="setup-error">{error}</p>}

      <div className="setup-storage">
        <span className="setup-tile small">
          <Icon name="drive" size={16} />
        </span>
        <div className="setup-row-text">
          <code className="setup-path" title={storage?.dir}>
            {storage?.dir ?? "…"}
          </code>
          <span className="setup-hw">
            <span>
              <Icon name="cpu" size={12} /> {hw ? hw.cpu_model.trim() || "This computer" : "Checking…"}
            </span>
            {hw && (
              <span>
                <Icon name="memoryChip" size={12} /> {hw.ram_gb} GB memory
              </span>
            )}
          </span>
        </div>
        <button className="setup-ghost" disabled={busy} onClick={() => setEditingFolder(storage?.dir ?? "")} title="Where Orttaai keeps speech models">
          Change…
        </button>
      </div>
      {editingFolder !== null && (
        <div className="setup-folder">
          <input className="text-input" value={editingFolder} onChange={(e) => setEditingFolder(e.target.value)} aria-label="Models folder" />
          <Button size="sm" variant="primary" onClick={saveFolder}>
            Save
          </Button>
          {storage?.custom && (
            <Button
              size="sm"
              onClick={() => {
                setEditingFolder("");
                call("set_models_dir", { path: null }).then(() => {
                  setEditingFolder(null);
                  load();
                });
              }}
            >
              Use Default Location
            </Button>
          )}
          <Button size="sm" onClick={() => setEditingFolder(null)}>
            Cancel
          </Button>
        </div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ ready -- */

const DEFAULT_DEVICE = "__default__";

function Ready({ onStart }: { onStart: () => void }) {
  const { settings, patch } = useSettings();
  const { engine, level, monitorLevel, snapshot } = useEngine();
  const toast = useToast();
  const { metered, error } = useLevelMonitor(settings?.audio_device ?? null, engine !== "recording" && engine !== "processing");
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [text, setText] = useState("");
  const [failed, setFailed] = useState(false);
  const [heard, setHeard] = useState(false);
  useEvent<string>("engine-error", () => setFailed(true));
  useEffect(() => {
    call<AudioDevice[]>("list_audio_devices").then(setDevices).catch(() => {});
  }, []);
  useEffect(() => {
    if (engine === "recording") {
      setFailed(false);
      setHeard(true);
    }
  }, [engine]);
  const options = useMemo(
    () => [{ value: DEFAULT_DEVICE, label: "System Default" }, ...devices.map((d) => ({ value: d.name, label: d.name }))],
    [devices],
  );
  if (!settings) return null;
  const hotkey = comboToString(settings.push_to_talk).split("+").join(" + ");
  const recording = engine === "recording";
  const processing = engine === "processing";
  const shown = recording ? level : processing ? 0 : monitorLevel;
  const percent = Math.round(Math.min(1, shown) * 100);
  const bad = failed || engine === "error" || !!error;
  const state = recording ? "Live" : processing ? "Paused" : bad ? "Check mic" : "Monitoring";
  const stateTone = recording ? "ok" : processing ? "warn" : bad ? "bad" : "accent";
  const message = recording
    ? "Meter is now following the live dictation capture."
    : processing
      ? "Input monitoring pauses while Orttaai transcribes and pastes."
      : bad
        ? "If this stays flat, confirm microphone access and the selected input device."
        : "Speak normally to confirm the bar reacts before you start the quick test.";
  const status = recording
    ? "Listening... Speak now."
    : processing
      ? "Transcribing..."
      : bad
        ? "Dictation failed. Try again."
        : engine === "off" || engine === "loading"
          ? "Starting the speech engine…"
          : heard
            ? "Done. Hold the shortcut to try again."
            : `Waiting for hotkey. Hold ${hotkey}.`;
  const statusIcon = recording ? "mic" : processing ? "waveform" : bad ? "alert" : "keyboard";
  const canClick = !!snapshot?.hotkey_error || !!snapshot?.wayland;

  return (
    <div className="setup-ready">
      <div className="setup-press">
        Press <kbd>{hotkey}</kbd> to start dictating.
      </div>
      {snapshot?.hotkey_error && (
        <div className="setup-trouble">
          <b>
            <Icon name="alert" size={13} /> That shortcut isn't available
          </b>
          <span>Pick another one, or use Click to record below.</span>
          <ShortcutRecorder
            value={settings.push_to_talk}
            other={settings.edit_commands_enabled ? settings.edit_shortcut : null}
            onChange={(c) => patch({ push_to_talk: c }).catch((e) => toast(String(e), "error"))}
          />
        </div>
      )}

      <div className="setup-mic">
        <div className="setup-mic-head">
          <b>Microphone Check</b>
          <span className="setup-credit-fill" />
          <div className="setup-mic-select">
            <Select
              value={settings.audio_device ?? DEFAULT_DEVICE}
              options={options}
              onChange={(v: string) => patch({ audio_device: v === DEFAULT_DEVICE ? null : v }).catch((e) => toast(String(e), "error"))}
            />
          </div>
          <span className={`setup-capsule ${stateTone}`}>{state}</span>
        </div>
        <div className="setup-mic-level">
          <span className="setup-mic-name">
            <Icon name="mic" size={13} /> {metered ?? settings.audio_device ?? "System Default"}
          </span>
          <span className="setup-meter" role="meter" aria-label="Microphone input level" aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}>
            <i style={{ width: `${percent}%` }} />
          </span>
          <span className="setup-credit-fill" />
          <code>{percent}%</code>
        </div>
        <span className={error ? "setup-error small" : "setup-note"}>{error ?? message}</span>
      </div>

      <div className="setup-test">
        <b>Quick Test</b>
        <span className="setup-test-hint">Select the field, then hold {hotkey} and speak.</span>
        <textarea value={text} onChange={(e) => setText(e.target.value)} placeholder="Try saying: Hello from Orttaai" aria-label="Quick test field" />
        <div className="setup-test-foot">
          <span className={`setup-status ${bad ? "bad" : recording ? "live" : ""}`} role="status">
            <Icon name={statusIcon} size={12} /> {status}
          </span>
          {canClick && (
            <button className="setup-ghost" onClick={() => call("toggle_recording").catch((e) => toast(String(e), "error"))}>
              {recording ? "Stop & insert" : "Click to record"}
            </button>
          )}
        </div>
      </div>

      <Button variant="primary" onClick={onStart}>
        Start Using Orttaai
      </Button>
    </div>
  );
}

/* ----------------------------------------------------------------- window -- */

function SetupFlow() {
  const { patch } = useSettings();
  const toast = useToast();
  const [step, setStep] = useState(0);
  const [forward, setForward] = useState(true);
  const [permissionsOk, setPermissionsOk] = useState(false);
  const [engineReady, setEngineReady] = useState(false);

  const go = (next: number) => {
    setForward(next > step);
    setStep(next);
  };
  const finish = () =>
    patch({ has_completed_setup: true })
      .then(() => call("finish_setup"))
      .catch((e) => toast(String(e), "error"));

  const canContinue = step === 1 ? permissionsOk : step === 2 ? engineReady : true;
  const hint = canContinue ? null : step === 1 ? "Allow the microphone to continue" : "Choose an engine to continue";
  const complete = step === 1 ? permissionsOk : step === 2 ? engineReady : false;

  return (
    <div className="setup-window">
      <AppearanceSync />
      <div className="setup-top">
        <StepTrack current={step} />
      </div>
      <div className="setup-stage">
        <SetupArt step={step} complete={complete} />
      </div>
      <div className={`setup-pane ${forward ? "forward" : "back"}`} key={step}>
        <div className="setup-heading">
          <h1>{COPY[step].title}</h1>
          <p>{COPY[step].sub}</p>
        </div>
        <div className="setup-scroll">
          {step === 0 && <Welcome />}
          {step === 1 && <Permissions onChange={setPermissionsOk} />}
          {step === 2 && <EngineStep onReady={setEngineReady} />}
          {step === 3 && <Ready onStart={finish} />}
        </div>
      </div>
      <footer className="setup-footer">
        {step > 0 && (
          <Button variant="ghost" icon="chevronLeft" onClick={() => go(step - 1)}>
            Back
          </Button>
        )}
        <span className="setup-credit-fill" />
        {hint && <span className="setup-hint">{hint}</span>}
        {step < STEPS.length - 1 && (
          <Button variant="primary" disabled={!canContinue} onClick={() => go(step + 1)}>
            {step === 0 ? "Get Started" : "Continue"} <Icon name="arrowRight" size={13} />
          </Button>
        )}
      </footer>
    </div>
  );
}

export default function SetupWindow() {
  return (
    <ThemeProvider>
      <ToastProvider>
        <SettingsProvider>
          <EngineProvider>
            <SetupFlow />
          </EngineProvider>
        </SettingsProvider>
      </ToastProvider>
    </ThemeProvider>
  );
}

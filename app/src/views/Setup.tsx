// First-run setup wizard (full-window), the Mac's four steps adapted to
// Linux/Windows: Welcome → System check (microphone test, typing + shortcut
// backends, Wayland portal / wtype status, Windows notes) → Speech model
// (hardware + recommended model download with progress) → Ready (mic level,
// shortcut recorder, "Hello from Orttaai" test field). Sets
// `has_completed_setup`. Owned by the engine area.
import { useEffect, useRef, useState } from "react";
import { Badge, Button, Icon, LevelMeter, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { comboToString, useSettings } from "../lib/settings";
import {
  formatBytes,
  startEngine,
  useDownloads,
  useEngine,
  type HardwareInfo,
  type ModelInfo,
} from "../lib/engine";
import ShortcutRecorder from "../components/ShortcutRecorder";
import { HardwareCard } from "./Models";
import { useLevelMonitor } from "./settings/Audio";
import "../styles/engine.css";

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

const STEPS = [
  {
    title: "Welcome to Orttaai",
    sub: "Hold a key, speak, and Orttaai types it into any app. Privately, on this computer.",
  },
  {
    title: "A quick system check",
    sub: "The microphone so it can hear you, and a way to type where your cursor is.",
  },
  {
    title: "Choose your speech model",
    sub: "It runs entirely on this computer. Pick one now; you can switch any time.",
  },
  {
    title: "Try your first dictation",
    sub: "Try your first dictation right here, then start using Orttaai.",
  },
];

function Check({ ok, optional, label, detail }: { ok: boolean | null; optional?: boolean; label: string; detail: string }) {
  return (
    <div className="setup-check">
      <span className={`setup-check-ico ${ok ? "ok" : ok === false ? (optional ? "opt" : "bad") : ""}`}>
        <Icon name={ok ? "check" : ok === false ? "alert" : "info"} size={15} />
      </span>
      <div className="setup-check-text">
        <span className="setup-check-label">
          {label}
          {optional && <span className="badge badge-neutral">Optional</span>}
        </span>
        <span className="small muted">{detail}</span>
      </div>
      {ok && <span className="setup-allowed small">Allowed</span>}
    </div>
  );
}

function Welcome() {
  const tiles = [
    { icon: "mic", title: "Hold a key, speak, let go", desc: "Orttaai types what you said into whatever app you're in." },
    { icon: "shield", title: "Stays on your computer", desc: "Speech and AI run on your own hardware. Nothing is uploaded." },
    {
      icon: "dictionary",
      title: "Learns your words",
      desc: "Names and terms you use get spelled right, and it improves as you fix them.",
    },
  ];
  return (
    <div className="setup-tiles">
      {tiles.map((t) => (
        <div key={t.title} className="setup-tile">
          <span className="setup-tile-ico">
            <Icon name={t.icon} size={20} />
          </span>
          <div>
            <h3>{t.title}</h3>
            <p className="small muted">{t.desc}</p>
          </div>
        </div>
      ))}
      <p className="small setup-credit">Made by Olanrewaju Oyinbooke</p>
    </div>
  );
}

function Permissions({ check, onRecheck, heard }: { check: PlatformCheck | null; onRecheck: () => void; heard: boolean }) {
  const { monitorLevel } = useEngine();
  const { settings } = useSettings();
  const { metered, error } = useLevelMonitor(settings?.audio_device ?? null, true);
  if (!check) return <p className="muted">Checking this computer…</p>;
  const wayland = check.session === "wayland";
  const micOk = check.microphones > 0 && !error;
  return (
    <div className="setup-stack">
      <Check
        ok={micOk ? true : false}
        label="Microphone"
        detail={
          micOk
            ? heard
              ? `Hearing you through ${metered ?? "the default microphone"}.`
              : `Say something to test ${metered ?? "the default microphone"}.`
            : error ?? "No microphone found. Connect one, then check again."
        }
      />
      {micOk && (
        <div className="setup-meter">
          <LevelMeter level={monitorLevel} active />
        </div>
      )}
      <Check
        ok={true}
        label="Typing into apps"
        detail={
          wayland
            ? `Wayland (${check.desktop ?? "desktop"}): ${check.injector}. Your desktop asks once for permission to type — choose Allow.`
            : check.platform === "windows"
              ? `${check.injector}. Password fields are detected and never typed into.`
              : check.injector
        }
      />
      <Check
        ok={true}
        label="Global shortcut"
        detail={
          check.portal_hotkeys
            ? "Global Shortcuts portal. If your desktop asks, assign a key to “push-to-talk”."
            : check.hotkeys
        }
      />
      {wayland && (
        <>
          <Check
            ok={check.wtype}
            optional
            label="wtype"
            detail={check.wtype ? "Installed — typing works on wlroots desktops too." : "sudo apt install wtype — for Sway/Hyprland."}
          />
          <Check
            ok={check.wl_clipboard}
            optional
            label="wl-clipboard"
            detail={check.wl_clipboard ? "Installed — paste mode and voice edit can use the clipboard." : "sudo apt install wl-clipboard"}
          />
        </>
      )}
      {check.notes.length > 0 && (
        <ul className="setup-notes small">
          {check.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      <div className="setup-row-end">
        <span className="small muted">
          <Icon name="shield" size={13} /> Audio is transcribed on this computer and never uploaded.
        </span>
        <Button size="sm" variant="ghost" icon="refresh" onClick={onRecheck}>
          Check again
        </Button>
      </div>
    </div>
  );
}

function EngineStep({ onReady }: { onReady: (ready: boolean) => void }) {
  const { settings, patch } = useSettings();
  const { engine } = useEngine();
  const toast = useToast();
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const load = () => call<ModelInfo[]>("list_models").then(setModels).catch(() => {});
  const { downloads, start, cancel } = useDownloads(() => load());
  useEffect(() => {
    load();
    call<HardwareInfo>("hardware_info").then(setHw).catch(() => {});
  }, []);

  const recommended = models.find((m) => m.recommended);
  const current = chosen ?? (models.find((m) => m.id === settings?.model_id && m.downloaded)?.id || recommended?.id || null);
  const model = models.find((m) => m.id === current);
  const dl = current ? downloads[current] : undefined;
  const ready = !!model?.downloaded && (engine === "idle" || engine === "recording" || engine === "processing");

  useEffect(() => onReady(ready), [ready]); // eslint-disable-line react-hooks/exhaustive-deps

  // Once the chosen model is on disk: select it and (re)start the engine.
  useEffect(() => {
    if (!model?.downloaded || !settings) return;
    const go = async () => {
      try {
        if (settings.model_id !== model.id) await patch({ model_id: model.id });
        if (engine === "off") await startEngine();
        setStartError(null);
      } catch (e) {
        setStartError(String(e));
      }
    };
    void go();
  }, [model?.id, model?.downloaded]); // eslint-disable-line react-hooks/exhaustive-deps

  const others = models.filter((m) => m.id !== current && m.supported).slice(0, 6);
  return (
    <div className="setup-stack">
      <HardwareCard hw={hw} />
      {model && (
        <div className="setup-model card">
          <div className="model-title">
            <span className="model-name">{model.name}</span>
            {model.recommended && <Badge tone="accent">Recommended for this computer</Badge>}
          </div>
          <span className="small muted">
            {formatBytes(model.size_bytes)} • {model.speed} • {model.accuracy} accuracy — {model.description}
          </span>
          {model.downloaded ? (
            <span className="small setup-status">
              {engine === "loading" ? (
                "Loading model… Running a quick warm-up for faster first dictation."
              ) : ready ? (
                <>
                  <Icon name="check" size={13} /> Ready
                </>
              ) : (
                "Preparing…"
              )}
            </span>
          ) : dl && !dl.error ? (
            <>
              <div className="dl-line">
                <div className="dl-track">
                  <div className="dl-bar" style={{ width: `${Math.round(dl.fraction * 100)}%` }} />
                </div>
                <span className="small mono">{Math.round(dl.fraction * 100)}%</span>
              </div>
              <span className="small muted">Downloading model files… First-time download can take a few minutes.</span>
              <div>
                <Button size="sm" variant="ghost" onClick={() => cancel(model.id)}>
                  Cancel
                </Button>
              </div>
            </>
          ) : (
            <div>
              <Button variant="primary" icon="download" onClick={() => start(model.id)}>
                Download ({formatBytes(model.size_bytes)})
              </Button>
              {dl?.error && <p className="small eng-err">Couldn't download model. Check your connection and try again. ({dl.error})</p>}
            </div>
          )}
          {startError && (
            <p className="small eng-err">
              Couldn't load downloaded model. Try again or choose another model. ({startError})
            </p>
          )}
        </div>
      )}
      {others.length > 0 && (
        <details className="setup-others">
          <summary className="small">Choose a different model</summary>
          {others.map((m) => (
            <button
              key={m.id}
              className="setup-other"
              onClick={() => {
                setChosen(m.id);
                if (!m.downloaded) start(m.id);
              }}
            >
              <span>{m.name}</span>
              <span className="small muted">
                {formatBytes(m.size_bytes)} • {m.speed} • {m.accuracy}
                {m.downloaded ? " • downloaded" : ""}
              </span>
            </button>
          ))}
        </details>
      )}
      {!models.length && <p className="muted">Loading the model list…</p>}
      {startError && (
        <Button size="sm" onClick={() => startEngine().then(() => setStartError(null)).catch((e) => toast(String(e), "error"))}>
          Try again
        </Button>
      )}
    </div>
  );
}

function Ready() {
  const { settings, patch } = useSettings();
  const { engine, monitorLevel, snapshot } = useEngine();
  const toast = useToast();
  useLevelMonitor(settings?.audio_device ?? null, true);
  const [failed, setFailed] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  useEvent<string>("engine-error", () => setFailed(true));
  useEffect(() => {
    if (engine === "recording") setFailed(false);
  }, [engine]);
  if (!settings) return null;
  const hotkey = comboToString(settings.push_to_talk);
  const line =
    engine === "recording"
      ? "Listening... Speak now."
      : engine === "processing"
        ? "Transcribing..."
        : failed || engine === "error"
          ? "Dictation failed. Try again."
          : engine === "off" || engine === "loading"
            ? "Starting the speech engine…"
            : `Waiting for hotkey. Hold ${hotkey}.`;
  return (
    <div className="setup-stack">
      <div className="card setup-card">
        <span className="overline">Microphone Check</span>
        <LevelMeter level={monitorLevel} active />
        <span className="small muted">
          {engine === "recording"
            ? "Meter is now following the live dictation capture."
            : "Speak normally to confirm the bar reacts before you start the quick test."}
        </span>
      </div>
      <div className="card setup-card">
        <span className="overline">Shortcut</span>
        <div className="setup-inline">
          <span className="small">Press</span>
          <ShortcutRecorder
            value={settings.push_to_talk}
            other={settings.edit_commands_enabled ? settings.edit_shortcut : null}
            onChange={(c) => patch({ push_to_talk: c }).catch((e) => toast(String(e), "error"))}
          />
          <span className="small">to start dictating.</span>
        </div>
        {snapshot?.hotkey_error && (
          <span className="small eng-err">
            The shortcut isn't available ({snapshot.hotkey_error}). Pick another, or use Click to record.
          </span>
        )}
      </div>
      <div className="card setup-card">
        <span className="overline">Quick Test</span>
        <span className="small">Select the field, then hold {hotkey} and speak.</span>
        <textarea
          ref={field}
          className="text-input setup-test"
          placeholder="Try saying: Hello from Orttaai"
          rows={3}
        />
        <div className="setup-row-end">
          <span className={`small ${failed ? "eng-err" : "muted"}`} role="status">
            {line}
          </span>
          {engine !== "off" && engine !== "loading" && (
            <Button
              size="sm"
              icon={engine === "recording" ? "stop" : "mic"}
              onClick={() => {
                field.current?.focus();
                call("toggle_recording").catch((e) => toast(String(e), "error"));
              }}
            >
              {engine === "recording" ? "Stop & insert" : "Click to record"}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

export default function Setup({ onDone }: { onDone: () => void }) {
  const { patch } = useSettings();
  const toast = useToast();
  const { monitorLevel } = useEngine();
  const [step, setStep] = useState(0);
  const [check, setCheck] = useState<PlatformCheck | null>(null);
  const [heard, setHeard] = useState(false);
  const [engineReady, setEngineReady] = useState(false);

  const recheck = () => {
    setCheck(null);
    call<PlatformCheck>("platform_check")
      .then(setCheck)
      .catch((e) => toast(String(e), "error"));
  };
  useEffect(() => {
    if (step === 1 && !check) recheck();
  }, [step]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (step === 1 && monitorLevel > 0.04) setHeard(true);
  }, [monitorLevel, step]);

  const finish = () =>
    patch({ has_completed_setup: true })
      .then(onDone)
      .catch((e) => toast(String(e), "error"));

  const canContinue = step === 1 ? (check?.microphones ?? 0) > 0 : step === 2 ? engineReady : true;
  const blocked = step === 1 ? "Connect a microphone to continue" : step === 2 ? "Download a model to continue" : "";

  return (
    <div className="setup">
      <div className="setup-inner">
        <header className="setup-head">
          <div className="setup-steps" aria-label={`Step ${step + 1} of ${STEPS.length}: ${STEPS[step].title}`}>
            {STEPS.map((s, i) => (
              <span key={s.title} className={`setup-step ${i === step ? "current" : i < step ? "done" : ""}`}>
                {i < step ? <Icon name="check" size={12} /> : i + 1}
              </span>
            ))}
          </div>
          <h1>{STEPS[step].title}</h1>
          <p className="muted">{STEPS[step].sub}</p>
        </header>

        <main className="setup-body">
          {step === 0 && <Welcome />}
          {step === 1 && <Permissions check={check} onRecheck={recheck} heard={heard} />}
          {step === 2 && <EngineStep onReady={setEngineReady} />}
          {step === 3 && <Ready />}
        </main>

        <footer className="setup-foot">
          {step > 0 ? (
            <Button variant="ghost" onClick={() => setStep(step - 1)}>
              Back
            </Button>
          ) : (
            <Button variant="ghost" onClick={finish}>
              Skip setup
            </Button>
          )}
          <span className="small muted">{!canContinue ? blocked : ""}</span>
          {step < STEPS.length - 1 ? (
            <Button variant="primary" onClick={() => setStep(step + 1)} disabled={!canContinue}>
              Continue
            </Button>
          ) : (
            <Button variant="primary" onClick={finish}>
              Start Using Orttaai
            </Button>
          )}
        </footer>
      </div>
    </div>
  );
}

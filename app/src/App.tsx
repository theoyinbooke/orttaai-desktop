import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type AppInfo = { name: string; version: string; platform: string };
type Settings = {
  model_id: string;
  push_to_talk: string;
  preserve_clipboard: boolean;
  low_latency: boolean;
  ollama_endpoint: string;
};
type HistoryItem = {
  id: number;
  text: string;
  app: string | null;
  word_count: number;
  created_at: number;
};

type Tab = "status" | "history" | "settings";
type EngineState = "off" | "idle" | "recording" | "processing";

function App() {
  const [tab, setTab] = useState<Tab>("status");
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [error, setError] = useState<string | null>(null);

  const [engine, setEngine] = useState<EngineState>("off");
  const [modelPath, setModelPath] = useState("");
  const [lastTranscript, setLastTranscript] = useState("");

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo).catch((e) => setError(String(e)));
    invoke<Settings>("get_settings")
      .then(setSettings)
      .catch((e) => setError(String(e)));

    const unlisten = [
      listen<string>("engine-state", (e) => setEngine(e.payload as EngineState)),
      listen<string>("transcript", (e) => setLastTranscript(e.payload)),
      listen<string>("engine-error", (e) => setError(e.payload)),
    ];
    return () => {
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, []);

  useEffect(() => {
    if (tab !== "history") return;
    invoke<HistoryItem[]>("recent_history", { limit: 50 })
      .then(setHistory)
      .catch((e) => setError(String(e)));
  }, [tab, engine]);

  async function start() {
    setError(null);
    try {
      await invoke("start_dictation", { modelPath });
    } catch (e) {
      setError(String(e));
    }
  }
  async function stop() {
    try {
      await invoke("stop_dictation");
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className={`dot ${engine}`} />
          <span className="brand-name">{info?.name ?? "Orttaai"}</span>
        </div>
        <span className="meta">
          v{info?.version ?? "—"} · {info?.platform ?? "—"}
        </span>
      </header>

      <nav className="tabs">
        {(["status", "history", "settings"] as Tab[]).map((t) => (
          <button
            key={t}
            className={t === tab ? "tab active" : "tab"}
            onClick={() => setTab(t)}
          >
            {t[0].toUpperCase() + t.slice(1)}
          </button>
        ))}
      </nav>

      {error && <div className="error">{error}</div>}

      <main className="content">
        {tab === "status" && (
          <StatusView
            settings={settings}
            engine={engine}
            modelPath={modelPath}
            setModelPath={setModelPath}
            lastTranscript={lastTranscript}
            onStart={start}
            onStop={stop}
          />
        )}
        {tab === "history" && <HistoryView items={history} />}
        {tab === "settings" && <SettingsView settings={settings} />}
      </main>
    </div>
  );
}

const STATE_LABEL: Record<EngineState, string> = {
  off: "Off",
  idle: "Listening",
  recording: "Recording",
  processing: "Transcribing",
};

function StatusView(props: {
  settings: Settings | null;
  engine: EngineState;
  modelPath: string;
  setModelPath: (v: string) => void;
  lastTranscript: string;
  onStart: () => void;
  onStop: () => void;
}) {
  const { settings, engine, modelPath, setModelPath, lastTranscript } = props;
  const running = engine !== "off";
  const fileRef = useRef<HTMLInputElement>(null);

  return (
    <section className="panel">
      <div className="status-hero">
        <div className={`status-badge ${engine}`}>{STATE_LABEL[engine]}</div>
        <p className="status-hint">
          Hold <kbd>{settings?.push_to_talk ?? "Ctrl+Shift+Space"}</kbd> and
          speak. Release to transcribe and inject.
        </p>
      </div>

      <div className="controls">
        <input
          ref={fileRef}
          className="model-input"
          placeholder="Path to a whisper .bin / .gguf model…"
          value={modelPath}
          disabled={running}
          onChange={(e) => setModelPath(e.currentTarget.value)}
        />
        {running ? (
          <button className="btn stop" onClick={props.onStop}>
            Stop
          </button>
        ) : (
          <button
            className="btn start"
            onClick={props.onStart}
            disabled={!modelPath.trim()}
          >
            Start
          </button>
        )}
      </div>

      {lastTranscript && (
        <div className="last-transcript">
          <span className="label">Last</span>
          <p>{lastTranscript}</p>
        </div>
      )}

      <p className="note">
        Linux/Windows only for live dictation (mic + global hotkey need OS
        permissions). The transcript is typed into whatever window is focused.
      </p>
    </section>
  );
}

function HistoryView({ items }: { items: HistoryItem[] }) {
  if (items.length === 0) {
    return (
      <section className="panel empty">
        <p>No transcriptions yet.</p>
        <p className="note">Dictate something and it will show up here.</p>
      </section>
    );
  }
  return (
    <section className="panel">
      <ul className="history">
        {items.map((item) => (
          <li key={item.id} className="history-item">
            <p className="history-text">{item.text}</p>
            <div className="history-meta">
              <span>{item.app ?? "unknown app"}</span>
              <span>{item.word_count} words</span>
              <span>{new Date(item.created_at * 1000).toLocaleString()}</span>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function SettingsView({ settings }: { settings: Settings | null }) {
  if (!settings) return <section className="panel">Loading…</section>;
  const rows: [string, string][] = [
    ["Model", settings.model_id],
    ["Push-to-talk", settings.push_to_talk],
    ["Preserve clipboard", settings.preserve_clipboard ? "On" : "Off"],
    ["Low-latency mode", settings.low_latency ? "On" : "Off"],
    ["Ollama endpoint", settings.ollama_endpoint],
  ];
  return (
    <section className="panel">
      <dl className="kv wide">
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      <p className="note">
        Editing settings from the UI lands in a later increment.
      </p>
    </section>
  );
}

export default App;

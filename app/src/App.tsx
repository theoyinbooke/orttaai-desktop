import { useEffect, useState } from "react";
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
type ModelInfo = {
  id: string;
  name: string;
  approx_size_mb: number;
  multilingual: boolean;
  url: string;
  downloaded: boolean;
  path: string;
};

type Tab = "status" | "models" | "history" | "chat" | "settings";
type EngineState = "off" | "idle" | "recording" | "processing";

const STATE_LABEL: Record<EngineState, string> = {
  off: "Off",
  idle: "Listening",
  recording: "Recording",
  processing: "Transcribing",
};

function App() {
  const [tab, setTab] = useState<Tab>("status");
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);

  const [engine, setEngine] = useState<EngineState>("off");
  const [lastTranscript, setLastTranscript] = useState("");

  const refreshSettings = () =>
    invoke<Settings>("get_settings").then(setSettings).catch((e) => setError(String(e)));
  const refreshModels = () =>
    invoke<ModelInfo[]>("list_models").then(setModels).catch((e) => setError(String(e)));

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo).catch((e) => setError(String(e)));
    refreshSettings();
    refreshModels();

    const unlisten = [
      listen<string>("engine-state", (e) => setEngine(e.payload as EngineState)),
      listen<string>("transcript", (e) => setLastTranscript(e.payload)),
      listen<string>("engine-error", (e) => setError(e.payload)),
      listen<{ id: string; fraction: number }>("model-progress", (e) =>
        setProgress((p) => ({ ...p, [e.payload.id]: e.payload.fraction })),
      ),
      listen<{ id: string }>("model-done", (e) => {
        setProgress((p) => {
          const next = { ...p };
          delete next[e.payload.id];
          return next;
        });
        refreshModels();
      }),
      listen<{ error: string }>("model-error", (e) => setError(e.payload.error)),
    ];
    return () => {
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, []);

  useEffect(() => {
    if (tab === "history") {
      invoke<HistoryItem[]>("recent_history", { limit: 50 })
        .then(setHistory)
        .catch((e) => setError(String(e)));
    }
  }, [tab, engine]);

  const activeModel = models.find((m) => m.id === settings?.model_id);
  const canStart = !!activeModel?.downloaded;

  async function start() {
    setError(null);
    try {
      await invoke("start_dictation", { modelPath: "" });
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
        {(["status", "models", "history", "chat", "settings"] as Tab[]).map((t) => (
          <button
            key={t}
            className={t === tab ? "tab active" : "tab"}
            onClick={() => setTab(t)}
          >
            {t[0].toUpperCase() + t.slice(1)}
          </button>
        ))}
      </nav>

      {error && (
        <div className="error" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      <main className="content">
        {tab === "status" && (
          <StatusView
            settings={settings}
            engine={engine}
            activeModel={activeModel}
            canStart={canStart}
            lastTranscript={lastTranscript}
            onStart={start}
            onStop={stop}
            onPickModel={() => setTab("models")}
          />
        )}
        {tab === "models" && (
          <ModelsView
            models={models}
            progress={progress}
            activeId={settings?.model_id}
            onDownload={(id) => invoke("download_model", { id })}
            onUse={async (id) => {
              if (!settings) return;
              await invoke("set_settings", {
                input: { ...settings, model_id: id },
              }).catch((e) => setError(String(e)));
              refreshSettings();
            }}
          />
        )}
        {tab === "history" && <HistoryView items={history} />}
        {tab === "chat" && <ChatView />}
        {tab === "settings" && (
          <SettingsView settings={settings} onSaved={refreshSettings} />
        )}
      </main>
    </div>
  );
}

function StatusView(props: {
  settings: Settings | null;
  engine: EngineState;
  activeModel: ModelInfo | undefined;
  canStart: boolean;
  lastTranscript: string;
  onStart: () => void;
  onStop: () => void;
  onPickModel: () => void;
}) {
  const { settings, engine, activeModel, canStart, lastTranscript } = props;
  const running = engine !== "off";

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
        <div className="active-model">
          <span className="label">Model</span>
          <span>
            {activeModel ? activeModel.name : settings?.model_id ?? "—"}
            {activeModel && !activeModel.downloaded && " (not downloaded)"}
          </span>
        </div>
        {running ? (
          <button className="btn stop" onClick={props.onStop}>
            Stop
          </button>
        ) : canStart ? (
          <button className="btn start" onClick={props.onStart}>
            Start
          </button>
        ) : (
          <button className="btn" onClick={props.onPickModel}>
            Get a model
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

function ModelsView(props: {
  models: ModelInfo[];
  progress: Record<string, number>;
  activeId: string | undefined;
  onDownload: (id: string) => void;
  onUse: (id: string) => void;
}) {
  return (
    <section className="panel">
      <ul className="models">
        {props.models.map((m) => {
          const frac = props.progress[m.id];
          const downloading = frac !== undefined;
          return (
            <li key={m.id} className="model-row">
              <div className="model-info">
                <span className="model-name">{m.name}</span>
                <span className="model-sub">
                  ~{m.approx_size_mb} MB · {m.multilingual ? "multilingual" : "English"}
                </span>
              </div>
              {downloading ? (
                <div className="progress">
                  <div className="bar" style={{ width: `${Math.round(frac * 100)}%` }} />
                  <span>{Math.round(frac * 100)}%</span>
                </div>
              ) : m.downloaded ? (
                props.activeId === m.id ? (
                  <span className="badge active-badge">Active</span>
                ) : (
                  <button className="btn small" onClick={() => props.onUse(m.id)}>
                    Use
                  </button>
                )
              ) : (
                <button className="btn small ghost" onClick={() => props.onDownload(m.id)}>
                  Download
                </button>
              )}
            </li>
          );
        })}
      </ul>
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

function ChatView() {
  const [messages, setMessages] = useState<{ role: "user" | "ai"; text: string }[]>([]);
  const [input, setInput] = useState("");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    invoke<string[]>("ollama_models")
      .then((m) => {
        setModels(m);
        if (m[0]) setModel(m[0]);
      })
      .catch((e) => setErr(String(e)));
  }, []);

  async function send() {
    const prompt = input.trim();
    if (!prompt || busy) return;
    setInput("");
    setMessages((m) => [...m, { role: "user", text: prompt }]);
    setBusy(true);
    setErr(null);
    try {
      const reply = await invoke<string>("ollama_chat", { prompt, model });
      setMessages((m) => [...m, { role: "ai", text: reply }]);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel chat">
      <div className="chat-head">
        <select value={model} onChange={(e) => setModel(e.currentTarget.value)}>
          {models.length === 0 && <option value="">no models</option>}
          {models.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <span className="note inline">Local Ollama — start it and pull a model.</span>
      </div>
      <div className="chat-log">
        {messages.length === 0 && !busy && (
          <p className="note">Ask anything. Responses come from your local Ollama.</p>
        )}
        {messages.map((m, i) => (
          <div key={i} className={`msg ${m.role}`}>
            {m.text}
          </div>
        ))}
        {busy && <div className="msg ai thinking">…</div>}
      </div>
      {err && <div className="error inline">{err}</div>}
      <div className="chat-input">
        <input
          value={input}
          placeholder="Message…"
          onChange={(e) => setInput(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") send();
          }}
        />
        <button className="btn start" onClick={send} disabled={busy}>
          Send
        </button>
      </div>
    </section>
  );
}

function SettingsView(props: { settings: Settings | null; onSaved: () => void }) {
  const [form, setForm] = useState<Settings | null>(props.settings);
  const [saved, setSaved] = useState(false);

  useEffect(() => setForm(props.settings), [props.settings]);

  if (!form) return <section className="panel">Loading…</section>;

  const update = (patch: Partial<Settings>) => {
    setForm({ ...form, ...patch });
    setSaved(false);
  };

  async function save() {
    await invoke("set_settings", { input: form });
    setSaved(true);
    props.onSaved();
  }

  return (
    <section className="panel">
      <div className="form">
        <label>
          <span>Push-to-talk</span>
          <input
            value={form.push_to_talk}
            onChange={(e) => update({ push_to_talk: e.currentTarget.value })}
          />
        </label>
        <label>
          <span>Ollama endpoint</span>
          <input
            value={form.ollama_endpoint}
            onChange={(e) => update({ ollama_endpoint: e.currentTarget.value })}
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={form.preserve_clipboard}
            onChange={(e) => update({ preserve_clipboard: e.currentTarget.checked })}
          />
          <span>Preserve clipboard</span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={form.low_latency}
            onChange={(e) => update({ low_latency: e.currentTarget.checked })}
          />
          <span>Low-latency mode</span>
        </label>
        <div className="form-actions">
          <button className="btn start" onClick={save}>
            Save
          </button>
          {saved && <span className="saved">Saved ✓</span>}
        </div>
      </div>
      <p className="note">Model is chosen in the Models tab.</p>
    </section>
  );
}

export default App;

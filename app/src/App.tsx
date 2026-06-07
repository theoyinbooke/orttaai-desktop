import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

function App() {
  const [tab, setTab] = useState<Tab>("status");
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo).catch((e) => setError(String(e)));
    invoke<Settings>("get_settings")
      .then(setSettings)
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (tab !== "history") return;
    invoke<HistoryItem[]>("recent_history", { limit: 50 })
      .then(setHistory)
      .catch((e) => setError(String(e)));
  }, [tab]);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="dot" />
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
        {tab === "status" && <StatusView settings={settings} />}
        {tab === "history" && <HistoryView items={history} />}
        {tab === "settings" && <SettingsView settings={settings} />}
      </main>
    </div>
  );
}

function StatusView({ settings }: { settings: Settings | null }) {
  return (
    <section className="panel">
      <div className="status-hero">
        <div className="status-badge idle">Idle</div>
        <p className="status-hint">
          Hold <kbd>{settings?.push_to_talk ?? "Ctrl+Shift+Space"}</kbd> and
          speak. Release to transcribe and inject.
        </p>
      </div>
      <dl className="kv">
        <div>
          <dt>Model</dt>
          <dd>{settings?.model_id ?? "—"}</dd>
        </div>
        <div>
          <dt>Push-to-talk</dt>
          <dd>{settings?.push_to_talk ?? "—"}</dd>
        </div>
      </dl>
      <p className="note">
        The dictation engine runs from the core; wiring the live controls into
        this window is the next step.
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

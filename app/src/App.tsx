import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { runUpdate } from "./lib/updater";
import "./App.css";
import {
  Icon,
  ThemeProvider,
  ToastProvider,
  useTheme,
  useToast,
} from "./ui";
import {
  STATE_LABEL,
  type AppInfo,
  type EngineState,
  type HistoryItem,
  type ModelInfo,
  type Settings,
  type Tab,
} from "./types";
import Dictate from "./views/Dictate";
import History from "./views/History";
import Insights from "./views/Insights";
import Dictionary from "./views/Dictionary";
import Models from "./views/Models";
import Assistant from "./views/Assistant";
import SettingsView from "./views/Settings";
import About from "./views/About";

const NAV: { group: string; items: { tab: Tab; label: string; icon: string }[] }[] = [
  {
    group: "Workspace",
    items: [
      { tab: "dictate", label: "Dictate", icon: "dictate" },
      { tab: "history", label: "History", icon: "history" },
      { tab: "insights", label: "Insights", icon: "insights" },
    ],
  },
  {
    group: "Library",
    items: [
      { tab: "dictionary", label: "Dictionary", icon: "dictionary" },
      { tab: "models", label: "Models", icon: "models" },
      { tab: "assistant", label: "Assistant", icon: "assistant" },
    ],
  },
];

function BrandMark() {
  return (
    <svg className="brand-mark" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="9" width="2.6" height="6" rx="1.3" />
      <rect x="8" y="5" width="2.6" height="14" rx="1.3" />
      <rect x="13.4" y="2.5" width="2.6" height="19" rx="1.3" />
      <rect x="18.8" y="8" width="2.6" height="8" rx="1.3" />
    </svg>
  );
}

function AppShell() {
  const [tab, setTab] = useState<Tab>("dictate");
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem("orttaai-rail") === "1",
  );
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [engine, setEngine] = useState<EngineState>("off");
  const [lastTranscript, setLastTranscript] = useState("");
  const [historyVersion, setHistoryVersion] = useState(0);
  const [level, setLevel] = useState(0);
  const { choice, setChoice } = useTheme();
  const toast = useToast();

  const refreshSettings = () =>
    invoke<Settings>("get_settings").then(setSettings).catch((e) => toast(String(e), "error"));
  const refreshModels = () =>
    invoke<ModelInfo[]>("list_models").then(setModels).catch((e) => toast(String(e), "error"));

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo).catch((e) => toast(String(e), "error"));
    refreshSettings();
    refreshModels();
    const unlisten = [
      listen<string>("engine-state", (e) => setEngine(e.payload as EngineState)),
      listen<string>("transcript", (e) => setLastTranscript(e.payload)),
      listen<string>("engine-error", (e) => toast(e.payload, "error")),
      listen<string>("engine-warning", (e) => toast(e.payload, "warn")),
      listen("history-changed", () => setHistoryVersion((v) => v + 1)),
      listen<number>("audio-level", (e) => setLevel(e.payload)),
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
      listen<{ error: string }>("model-error", (e) => toast(e.payload.error, "error")),
    ];

    // Auto-check for updates shortly after launch. Silent: stays quiet when
    // already up to date (or in a dev build with no updater), and only surfaces
    // UI once an update is actually found. Safe to install + relaunch here — the
    // engine is always "off" at startup, so nothing is mid-dictation.
    const updateTimer = setTimeout(() => {
      runUpdate(
        (s) => {
          if (s.kind === "available") toast(`Update ${s.version} found — installing…`, "info");
          else if (s.kind === "installed") toast(`Updated to ${s.version} — restarting…`, "success");
          else if (s.kind === "error") toast(`Update failed: ${s.message}`, "warn");
        },
        { silent: true },
      );
    }, 3000);

    return () => {
      clearTimeout(updateTimer);
      unlisten.forEach((p) => p.then((off) => off()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (tab === "history") {
      // Fetch the full set (backend clamps at 500) so the History page can
      // paginate through everything, 20 per page — not just the recent few.
      invoke<HistoryItem[]>("recent_history", { limit: 500 })
        .then(setHistory)
        .catch((e) => toast(String(e), "error"));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, historyVersion]);

  const activeModel = models.find((m) => m.id === settings?.model_id);
  const canStart = !!activeModel?.downloaded;

  const toggleRail = () => {
    setCollapsed((c) => {
      localStorage.setItem("orttaai-rail", c ? "0" : "1");
      return !c;
    });
  };

  const cycleTheme = () =>
    setChoice(choice === "dark" ? "light" : choice === "light" ? "system" : "dark");
  const themeIcon = choice === "dark" ? "moon" : choice === "light" ? "sun" : "monitor";

  async function checkUpdates() {
    await runUpdate((s) => {
      switch (s.kind) {
        case "checking":
          return toast("Checking for updates…", "info");
        case "uptodate":
          return toast("You're on the latest version.", "success");
        case "downloading":
          return toast(`Downloading ${s.version}…`, "info");
        case "installed":
          return toast("Update installed — restarting…", "success");
        case "error":
          return toast(`Update check failed: ${s.message}`, "error");
      }
    });
  }

  function openFeedback() {
    // Open a prefilled "new issue" on GitHub, stamped with the build so reports
    // are easy to triage. opener:default already permits https URLs.
    const body = `<!-- Describe your feedback, bug, or feature request below. -->\n\n\n---\nOrttaai v${info?.version ?? "?"} · ${info?.platform ?? "?"}`;
    const url = `https://github.com/theoyinbooke/orttaai-desktop/issues/new?body=${encodeURIComponent(body)}`;
    openUrl(url).catch((e) => toast(String(e), "error"));
  }

  return (
    <div className={`app ${collapsed ? "rail-collapsed" : ""}`}>
      <aside className="sidebar">
        <div className="brand">
          <button
            className="brand-badge"
            onClick={collapsed ? toggleRail : undefined}
            title={collapsed ? "Expand sidebar" : undefined}
            aria-label={collapsed ? "Expand sidebar" : "Orttaai"}
          >
            <BrandMark />
          </button>
          <span className="brand-name">Orttaai</span>
          <button
            className="icon-btn rail-toggle"
            onClick={toggleRail}
            aria-label="Collapse sidebar"
            title="Collapse sidebar"
          >
            <Icon name="chevronLeft" size={18} />
          </button>
        </div>

        <div className={`engine-pill engine-${engine}`} title={`Engine: ${STATE_LABEL[engine]}`}>
          <span className="engine-dot" />
          <span className="engine-text">{STATE_LABEL[engine]}</span>
        </div>

        <nav className="nav">
          {NAV.map((section) => (
            <div key={section.group} className="nav-group">
              <span className="nav-group-label">{section.group}</span>
              {section.items.map((it) => (
                <button
                  key={it.tab}
                  className={`nav-item ${tab === it.tab ? "active" : ""}`}
                  onClick={() => setTab(it.tab)}
                  title={collapsed ? it.label : undefined}
                >
                  <Icon name={it.icon} size={19} />
                  <span className="nav-label">{it.label}</span>
                </button>
              ))}
            </div>
          ))}
        </nav>

        <div className="sidebar-foot">
          <button
            className="nav-item"
            onClick={openFeedback}
            title={collapsed ? "Send feedback" : undefined}
          >
            <Icon name="send" size={19} />
            <span className="nav-label">Feedback</span>
          </button>
          <button
            className={`nav-item ${tab === "about" ? "active" : ""}`}
            onClick={() => setTab("about")}
            title={collapsed ? "About" : undefined}
          >
            <Icon name="info" size={19} />
            <span className="nav-label">About</span>
          </button>
          <button
            className={`nav-item ${tab === "settings" ? "active" : ""}`}
            onClick={() => setTab("settings")}
            title={collapsed ? "Settings" : undefined}
          >
            <Icon name="settings" size={19} />
            <span className="nav-label">Settings</span>
          </button>
          <div className="foot-tools">
            <button
              className="icon-btn"
              onClick={checkUpdates}
              title="Check for updates"
              aria-label="Check for updates"
            >
              <Icon name="refresh" size={17} />
            </button>
            <button className="icon-btn" onClick={cycleTheme} title={`Theme: ${choice}`} aria-label="Toggle theme">
              <Icon name={themeIcon} size={17} />
            </button>
          </div>
        </div>
      </aside>

      <main className="content">
        {tab === "dictate" && (
          <Dictate
            engine={engine}
            settings={settings}
            activeModel={activeModel}
            canStart={canStart}
            level={level}
            lastTranscript={lastTranscript}
            historyVersion={historyVersion}
            onStart={() => invoke("start_dictation", { modelPath: "" }).catch((e) => toast(String(e), "error"))}
            onStop={() => invoke("stop_dictation").catch((e) => toast(String(e), "error"))}
            onToggleRecord={() => invoke("toggle_recording").catch((e) => toast(String(e), "error"))}
            onNavigate={(t) => setTab(t)}
          />
        )}
        {tab === "history" && <History items={history} />}
        {tab === "insights" && <Insights />}
        {tab === "dictionary" && <Dictionary />}
        {tab === "models" && (
          <Models
            models={models}
            progress={progress}
            activeId={settings?.model_id}
            onDownload={(id) => invoke("download_model", { id })}
            onUse={async (id) => {
              if (!settings) return;
              await invoke("set_settings", { input: { ...settings, model_id: id } }).catch((e) =>
                toast(String(e), "error"),
              );
              refreshSettings();
            }}
            onDelete={(id) =>
              invoke("delete_model", { id })
                .then(() => {
                  refreshModels();
                  toast("Model deleted", "success");
                })
                .catch((e) => toast(String(e), "error"))
            }
          />
        )}
        {tab === "assistant" && <Assistant />}
        {tab === "settings" && (
          <SettingsView settings={settings} info={info} onSaved={refreshSettings} />
        )}
        {tab === "about" && <About info={info} onFeedback={openFeedback} />}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <ToastProvider>
        <AppShell />
      </ToastProvider>
    </ThemeProvider>
  );
}

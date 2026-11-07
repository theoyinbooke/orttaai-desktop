import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";
import { Icon, ThemeProvider, ToastProvider, useTheme, useToast } from "./ui";
import { TABS, type AppInfo, type Tab } from "./types";
import { call, setPendingChatPrefill, useEvent } from "./lib/api";
import { SettingsProvider, useSettings } from "./lib/settings";
import { EngineProvider, STATE_LABEL, useEngine } from "./lib/engine";
import { phaseLabel, useUpdater } from "./lib/updater";
import Home from "./views/Home";
import ChatAI from "./views/ChatAI";
import MemoryGraph from "./views/MemoryGraph";
import Memory from "./views/Memory";
import Analytics from "./views/Analytics";
import Milestones from "./views/Milestones";
import Coach from "./views/Coach";
import Models from "./views/Models";
import SettingsView from "./views/Settings";
import About from "./views/About";
import Setup from "./views/Setup";
import WhatsNew from "./components/WhatsNew";
import Celebrations from "./components/Celebrations";

const NAV: { group: string; items: { tab: Tab; label: string; icon: string }[] }[] = [
  {
    group: "Workspace",
    items: [
      { tab: "home", label: "Home", icon: "home" },
      { tab: "chat", label: "Chat AI", icon: "assistant" },
      { tab: "graph", label: "Memory Graph", icon: "graph" },
      { tab: "memory", label: "Memory", icon: "dictionary" },
    ],
  },
  {
    group: "Progress",
    items: [
      { tab: "analytics", label: "Analytics", icon: "insights" },
      { tab: "milestones", label: "Milestones", icon: "trophy" },
      { tab: "coach", label: "Coach", icon: "coach" },
    ],
  },
  {
    group: "Library",
    items: [{ tab: "models", label: "Models", icon: "models" }],
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

/** Keep the CSS theme in sync with `settings.appearance` (the source of truth). */
function AppearanceSync() {
  const { settings } = useSettings();
  const { choice, setChoice } = useTheme();
  useEffect(() => {
    if (settings && settings.appearance !== choice) setChoice(settings.appearance);
  }, [settings?.appearance]); // eslint-disable-line react-hooks/exhaustive-deps
  return null;
}

function UpdateButton({ collapsed }: { collapsed: boolean }) {
  const { settings } = useSettings();
  const { phase, act, checkNow } = useUpdater(settings?.auto_download_updates ?? true);
  const toast = useToast();
  useEvent<string>("navigate", (r) => {
    if (r === "check-updates") checkNow(false);
  });
  useEffect(() => {
    if (phase.kind === "uptodate") toast("You're on the latest version.", "success");
    if (phase.kind === "failed") toast(`Update check failed: ${phase.message}`, "warn");
  }, [phase.kind]); // eslint-disable-line react-hooks/exhaustive-deps
  const highlight = phase.kind === "available" || phase.kind === "ready";
  const label = phaseLabel(phase);
  return (
    <button
      className={`nav-item update-btn ${highlight ? "update-ready" : ""}`}
      onClick={act}
      title={collapsed ? label : undefined}
      disabled={phase.kind === "checking" || phase.kind === "installing"}
    >
      <Icon name={phase.kind === "ready" ? "restart" : "refresh"} size={19} />
      <span className="nav-label">{label}</span>
      {phase.kind === "downloading" && (
        <span className="update-progress" style={{ width: `${Math.round(phase.fraction * 100)}%` }} />
      )}
    </button>
  );
}

function AppShell() {
  const [tab, setTab] = useState<Tab>("home");
  const [setupOpen, setSetupOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem("orttaai-rail") === "1";
    } catch {
      return false;
    }
  });
  const [info, setInfo] = useState<AppInfo | null>(null);
  const { settings, patch } = useSettings();
  const { engine } = useEngine();
  const toast = useToast();

  useEffect(() => {
    call<AppInfo>("app_info").then(setInfo).catch((e) => toast(String(e), "error"));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // First run → the setup wizard.
  useEffect(() => {
    if (settings && !settings.has_completed_setup) setSetupOpen(true);
  }, [settings?.has_completed_setup]); // eslint-disable-line react-hooks/exhaustive-deps

  useEvent<string>("chat-prefill", (q) => setPendingChatPrefill(q));
  useEvent<string>("engine-error", (m) => toast(m, "error"));
  useEvent<string>("engine-warning", (m) => toast(m, "warn"));
  useEvent<string>("navigate", (route) => {
    if (route === "setup") setSetupOpen(true);
    else if ((TABS as readonly string[]).includes(route)) setTab(route as Tab);
  });

  const toggleRail = () =>
    setCollapsed((c) => {
      try {
        localStorage.setItem("orttaai-rail", c ? "0" : "1");
      } catch {
        /* storage unavailable */
      }
      return !c;
    });

  const appearance = settings?.appearance ?? "system";
  const cycleTheme = () =>
    patch({ appearance: appearance === "dark" ? "light" : appearance === "light" ? "system" : "dark" }).catch(
      (e) => toast(String(e), "error"),
    );
  const themeIcon = appearance === "dark" ? "moon" : appearance === "light" ? "sun" : "monitor";

  function openFeedback() {
    const body = `<!-- Describe your feedback, bug, or feature request below. -->\n\n\n---\nOrttaai v${info?.version ?? "?"} · ${info?.platform ?? "?"} · ${info?.session ?? "?"} · ${info?.gpu_backend ?? "?"}`;
    const url = `https://github.com/theoyinbooke/orttaai-desktop/issues/new?body=${encodeURIComponent(body)}`;
    openUrl(url).catch((e) => toast(String(e), "error"));
  }

  if (setupOpen) {
    return (
      <Setup
        onDone={() => {
          setSetupOpen(false);
          setTab("home");
        }}
      />
    );
  }

  return (
    <div className={`app ${collapsed ? "rail-collapsed" : ""}`}>
      <AppearanceSync />
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
          <button className="icon-btn rail-toggle" onClick={toggleRail} aria-label="Collapse sidebar" title="Collapse sidebar">
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
          <UpdateButton collapsed={collapsed} />
          <button className="nav-item" onClick={openFeedback} title={collapsed ? "Send feedback" : undefined}>
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
            <button className="icon-btn" onClick={cycleTheme} title={`Appearance: ${appearance}`} aria-label="Toggle appearance">
              <Icon name={themeIcon} size={17} />
            </button>
          </div>
        </div>
      </aside>

      <main className="content">
        {tab === "home" && <Home onNavigate={setTab} />}
        {tab === "chat" && <ChatAI />}
        {tab === "graph" && <MemoryGraph />}
        {tab === "memory" && <Memory />}
        {tab === "analytics" && <Analytics />}
        {tab === "milestones" && <Milestones />}
        {tab === "coach" && <Coach />}
        {tab === "models" && <Models />}
        {tab === "settings" && <SettingsView info={info} onRunSetup={() => setSetupOpen(true)} />}
        {tab === "about" && <About info={info} onFeedback={openFeedback} />}
      </main>
      <WhatsNew />
      <Celebrations />
    </div>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <ToastProvider>
        <SettingsProvider>
          <EngineProvider>
            <AppShell />
          </EngineProvider>
        </SettingsProvider>
      </ToastProvider>
    </ThemeProvider>
  );
}

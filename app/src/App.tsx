import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";
import { Button, Icon, ShellContext, ThemeProvider, ToastProvider, useTheme, useToast } from "./ui";
import { TABS, type AppInfo, type Tab } from "./types";
import { call, setPendingChatPrefill, useEvent } from "./lib/api";
import { SettingsProvider, useSettings } from "./lib/settings";
import { EngineProvider } from "./lib/engine";
import { phaseLabel, UpdaterProvider, useAppUpdater } from "./lib/updater";
import { AI_SETTINGS_ROUTE, requestAISection } from "./lib/navigation";
import { useRightCtrlTap } from "./lib/rightCtrl";
import Home from "./views/Home";
import ChatAI from "./views/ChatAI";
import MemoryGraph from "./views/MemoryGraph";
import Memory from "./views/Memory";
import Devices from "./views/Devices";
import Analytics from "./views/Analytics";
import Milestones from "./views/Milestones";
import Coach from "./views/Coach";
import Models from "./views/Models";
import SettingsView from "./views/Settings";
import About from "./views/About";
import WhatsNew from "./components/WhatsNew";
import Celebrations from "./components/Celebrations";

type Section = { tab: Tab; label: string; icon: string; tag?: string };

// Mac HomeSidebarView: the same two groups, in the same order.
const WORKSPACE: Section[] = [
  { tab: "home", label: "Overview", icon: "house" },
  { tab: "chat", label: "ChatAI", icon: "chatBubbles" },
  { tab: "graph", label: "Graph", icon: "connected" },
  { tab: "analytics", label: "Analytics", icon: "barsAxis" },
  { tab: "milestones", label: "Milestones", icon: "rosette" },
  { tab: "coach", label: "Coach", icon: "scope", tag: "Beta" },
];
const MANAGE: Section[] = [
  { tab: "memory", label: "Memory", icon: "book" },
  { tab: "devices", label: "Devices", icon: "devices" },
  { tab: "models", label: "Model", icon: "cpu" },
  { tab: "settings", label: "Settings", icon: "gear" },
  { tab: "about", label: "About", icon: "infoCircle" },
];

/** The Mac's signal-cursor glyph: three rising bars and a text cursor. */
export function BrandMark({ className }: { className?: string }) {
  return (
    <svg className={`brand-mark ${className ?? ""}`} viewBox="0 0 100 84" aria-hidden="true">
      <rect x="0" y="44" width="18" height="36" rx="9" />
      <rect x="25" y="28" width="18" height="52" rx="9" />
      <rect x="50" y="12" width="18" height="68" rx="9" />
      <rect x="85" y="3" width="6" height="78" rx="3" />
      <rect x="77" y="0" width="22" height="6" rx="3" />
      <rect x="77" y="78" width="22" height="6" rx="3" />
    </svg>
  );
}

/** Keep the CSS theme in sync with `settings.appearance` (the source of truth). */
export function AppearanceSync() {
  const { settings } = useSettings();
  const { choice, setChoice } = useTheme();
  useEffect(() => {
    if (settings && settings.appearance !== choice) setChoice(settings.appearance);
  }, [settings?.appearance]); // eslint-disable-line react-hooks/exhaustive-deps
  return null;
}

/** Mac AppUpdateSidebarButton: hidden while there is nothing to say. */
function UpdateRow() {
  const { phase, act } = useAppUpdater();
  if (phase.kind !== "available" && phase.kind !== "downloading" && phase.kind !== "ready" && phase.kind !== "installing") {
    return null;
  }
  const emphasized = phase.kind === "available" || phase.kind === "ready";
  return (
    <button className={`update-row ${emphasized ? "emphasized" : ""}`} onClick={act} disabled={phase.kind === "installing"}>
      <Icon name={phase.kind === "ready" ? "restart" : "download"} size={16} />
      <span className="update-row-text">
        <b>{phaseLabel(phase)}</b>
        <span>Orttaai {phase.version}</span>
      </span>
      {phase.kind === "downloading" && <i className="update-row-bar" style={{ width: `${Math.round(phase.fraction * 100)}%` }} />}
    </button>
  );
}

function WindowControls() {
  const win = getCurrentWindow();
  return (
    <div className="window-controls">
      <button onClick={() => win.minimize()} aria-label="Minimize" title="Minimize">
        <Icon name="minimize" size={15} />
      </button>
      <button onClick={() => win.toggleMaximize()} aria-label="Maximize" title="Maximize">
        <Icon name="maximize" size={15} />
      </button>
      <button className="close" onClick={() => win.close()} aria-label="Close" title="Close">
        <Icon name="close" size={15} />
      </button>
    </div>
  );
}

function SidebarRow({ section, tab, count, onSelect }: { section: Section; tab: Tab; count?: number; onSelect: (t: Tab) => void }) {
  const selected = tab === section.tab;
  return (
    <button
      className={`side-row ${selected ? "selected" : ""}`}
      onClick={() => onSelect(section.tab)}
      aria-label={section.tag ? `${section.label}, ${section.tag}` : section.label}
      aria-current={selected ? "page" : undefined}
    >
      <Icon name={section.icon} size={18} />
      <span className="side-row-label">{section.label}</span>
      {section.tag && <span className="side-row-tag">{section.tag}</span>}
      {!!count && <span className="side-row-count" aria-label={`${count} suggestions waiting`}>{count}</span>}
    </button>
  );
}

/** Mac SectionRail: the slim column shown when the sidebar is collapsed and beside Coach. */
function SectionRail({ tab, coach, onSelect, onExpand }: { tab: Tab; coach: boolean; onSelect: (t: Tab) => void; onExpand?: () => void }) {
  const rail = (s: Section) => (
    <button
      key={s.tab}
      className={`rail-btn ${tab === s.tab ? "selected" : ""}`}
      onClick={() => onSelect(s.tab)}
      title={s.tag ? `${s.label} (beta)` : s.label}
      aria-label={s.label}
    >
      <Icon name={s.icon} size={18} />
    </button>
  );
  return (
    <aside className={`section-rail ${coach ? "coach" : ""}`} data-tauri-drag-region>
      <BrandMark className="rail-brand" />
      {onExpand && (
        <button className="rail-expand" onClick={onExpand} title="Show the sidebar" aria-label="Show the sidebar">
          <Icon name="sidebarLeft" size={16} />
        </button>
      )}
      {WORKSPACE.map(rail)}
      <span className="rail-divider" aria-hidden="true" />
      {MANAGE.slice(0, 2).map(rail)}
      <span className="rail-fill" data-tauri-drag-region />
      {rail(MANAGE[2])}
    </aside>
  );
}

function AppShell() {
  useRightCtrlTap();
  const [tab, setTab] = useState<Tab>("home");
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem("orttaai-rail") === "1";
    } catch {
      return false;
    }
  });
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [pending, setPending] = useState(0);
  const { settings, error: settingsError, patch, reload: reloadSettings } = useSettings();
  const toast = useToast();

  useEffect(() => {
    call<AppInfo>("app_info").then(setInfo).catch((e) => toast(String(e), "error"));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const refreshPending = () =>
    call<unknown[]>("list_suggestions").then((s) => setPending(s.length)).catch(() => {});
  useEffect(() => {
    refreshPending();
  }, []);
  useEvent("memory-changed", refreshPending);

  const openSetup = () => call("open_setup_window").catch((e) => toast(String(e), "error"));

  useEvent<string>("chat-prefill", (q) => setPendingChatPrefill(q));
  useEvent<string>("engine-error", (m) => toast(m, "error"));
  useEvent<string>("engine-warning", (m) => toast(m, "warn"));
  useEvent<string>("navigate", (route) => {
    if (route === "setup") openSetup();
    else if (route === AI_SETTINGS_ROUTE) {
      requestAISection();
      setTab("models");
    } else if ((TABS as readonly string[]).includes(route)) setTab(route as Tab);
  });

  // Dev rig only (stripped from release builds): keys the screenshot script drives.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const onKey = (e: KeyboardEvent) => {
      const n = /^F(\d+)$/.exec(e.key)?.[1];
      if (!n) return;
      const i = Number(n);
      if (i >= 1 && i <= TABS.length) {
        e.preventDefault();
        setTab(TABS[i - 1]);
      } else if (i === 11 || i === 12) {
        e.preventDefault();
        patch({ appearance: i === 11 ? "dark" : "light" }).catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [patch]);

  const toggleSidebar = () =>
    setCollapsed((c) => {
      try {
        localStorage.setItem("orttaai-rail", c ? "0" : "1");
      } catch {
        /* storage unavailable */
      }
      return !c;
    });

  // Coach brings its own slim rail; the sidebar steps aside.
  const coach = tab === "coach";
  const showRail = collapsed || coach;

  return (
    <ShellContext.Provider value={{ toggleSidebar: coach ? undefined : toggleSidebar }}>
      <div className={`app ${showRail ? "rail" : ""}`}>
        <AppearanceSync />
        {showRail ? (
          <SectionRail tab={tab} coach={coach} onSelect={setTab} onExpand={coach ? undefined : toggleSidebar} />
        ) : (
          <aside className="sidebar">
            <div className="sidebar-brand" data-tauri-drag-region>
              <BrandMark />
              <span data-tauri-drag-region>Orttaai</span>
            </div>
            <nav className="sidebar-nav">
              <span className="side-group">Workspace</span>
              {WORKSPACE.map((s) => (
                <SidebarRow key={s.tab} section={s} tab={tab} onSelect={setTab} />
              ))}
              <span className="side-group manage">Manage</span>
              {MANAGE.map((s) => (
                <SidebarRow key={s.tab} section={s} tab={tab} count={s.tab === "memory" ? pending : 0} onSelect={setTab} />
              ))}
            </nav>
            <div className="sidebar-foot">
              <UpdateRow />
              <button className="run-setup" onClick={openSetup} title="Run setup again">
                <Icon name="sliders" size={17} />
                <span>Run Setup</span>
              </button>
            </div>
          </aside>
        )}

        <main className={`content section-${tab}`}>
          <WindowControls />
          {!settings && settingsError && (
            <div className="empty-state" role="alert" style={{ margin: 24 }}>
              <h3>Couldn't load your settings</h3>
              <p>{settingsError}</p>
              <Button className="empty-action" size="sm" variant="primary" onClick={reloadSettings}>
                Try Again
              </Button>
            </div>
          )}
          {tab === "home" && <Home onNavigate={setTab} />}
          {tab === "chat" && <ChatAI />}
          {tab === "graph" && <MemoryGraph />}
          {tab === "memory" && <Memory />}
          {tab === "devices" && <Devices />}
          {tab === "analytics" && <Analytics />}
          {tab === "milestones" && <Milestones />}
          {tab === "coach" && <Coach />}
          {tab === "models" && <Models />}
          {tab === "settings" && <SettingsView info={info} onRunSetup={openSetup} />}
          {tab === "about" && <About info={info} />}
        </main>
        <WhatsNew />
        <Celebrations />
      </div>
    </ShellContext.Provider>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <ToastProvider>
        <SettingsProvider>
          <EngineProvider>
            <UpdaterProvider>
              <AppShell />
            </UpdaterProvider>
          </EngineProvider>
        </SettingsProvider>
      </ToastProvider>
    </ThemeProvider>
  );
}

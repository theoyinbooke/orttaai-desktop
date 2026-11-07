// Settings shell — sections mirror the macOS Settings window:
// General · Dictation · Audio · Text · AI Features · Coach · About.
import { useState } from "react";
import { Button, ConfirmDialog, Icon, PageHeader, Segmented, useToast } from "../ui";
import type { AppInfo } from "../types";
import { call } from "../lib/api";
import { useSettings } from "../lib/settings";
import { phaseLabel, useUpdater } from "../lib/updater";
import { Row, Section, ToggleRow, usePatch } from "../components/SettingsRows";
import DictationSettings from "./settings/Dictation";
import AudioSettings from "./settings/Audio";
import TextSettings from "./settings/Text";
import AISettings from "./settings/AI";
import CoachSettings from "./settings/Coach";

type Pane = "general" | "dictation" | "audio" | "text" | "ai" | "coach" | "about";

const PANES: { id: Pane; label: string; icon: string }[] = [
  { id: "general", label: "General", icon: "settings" },
  { id: "dictation", label: "Dictation", icon: "keyboard" },
  { id: "audio", label: "Audio", icon: "mic" },
  { id: "text", label: "Text", icon: "edit" },
  { id: "ai", label: "AI Features", icon: "sparkle" },
  { id: "coach", label: "Coach", icon: "coach" },
  { id: "about", label: "About", icon: "info" },
];

export default function Settings(props: { info: AppInfo | null; onRunSetup: () => void }) {
  const [pane, setPane] = useState<Pane>("general");
  return (
    <div className="view">
      <PageHeader title="Settings" desc="Every change saves instantly." />
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {PANES.map((p) => (
            <button key={p.id} className={pane === p.id ? "active" : ""} onClick={() => setPane(p.id)}>
              <Icon name={p.icon} size={16} />
              {p.label}
            </button>
          ))}
        </nav>
        <div className="settings-body">
          {pane === "general" && <General onRunSetup={props.onRunSetup} />}
          {pane === "dictation" && <DictationSettings />}
          {pane === "audio" && <AudioSettings />}
          {pane === "text" && <TextSettings />}
          {pane === "ai" && <AISettings />}
          {pane === "coach" && <CoachSettings />}
          {pane === "about" && <AboutPane info={props.info} />}
        </div>
      </div>
    </div>
  );
}

function General({ onRunSetup }: { onRunSetup: () => void }) {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [confirm, setConfirm] = useState<null | "history" | "reset">(null);
  if (!settings) return null;
  return (
    <>
      <Section title="Appearance">
        <Row label="Look" hint="Follow the system, or pick light or dark.">
          <Segmented
            value={settings.appearance}
            options={[
              { value: "system", label: "System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
            onChange={(v) => save({ appearance: v })}
          />
        </Row>
      </Section>
      <Section title="Startup">
        <ToggleRow k="launch_at_login" label="Launch at login" hint="Start Orttaai in the tray when you sign in." />
        <ToggleRow
          k="start_engine_on_launch"
          label="Start dictation on launch"
          hint="Load the speech model as soon as Orttaai opens, so the shortcut works right away."
        />
      </Section>
      <Section title="Feedback">
        <ToggleRow
          k="show_processing_estimate"
          label="Show processing estimate"
          hint="The recording pill shows how long transcription should take."
        />
        <ToggleRow k="celebrate_milestones" label="Celebrate milestones" hint="A little celebration when you unlock a badge." />
        <ToggleRow k="sound_cues" label="Sound cues" hint="Soft sounds when recording starts, stops, or fails." />
      </Section>
      <Section title="Setup">
        <Row label="Run setup again" hint="Walk through permissions, model and shortcut again.">
          <Button onClick={onRunSetup}>Run Setup…</Button>
        </Row>
      </Section>
      <Section title="Data">
        <Row label="Clear history" hint="Delete every dictation and everything derived from them.">
          <Button variant="danger" onClick={() => setConfirm("history")}>
            Clear History…
          </Button>
        </Row>
        <Row label="Reset app data" hint="Delete history, dictionary, snippets, chats and badges. Settings are kept.">
          <Button variant="danger" onClick={() => setConfirm("reset")}>
            Reset App Data…
          </Button>
        </Row>
      </Section>
      <ConfirmDialog
        open={confirm !== null}
        title={confirm === "history" ? "Clear all history?" : "Reset all app data?"}
        body={
          confirm === "history"
            ? "Every dictation, its insights and memory index will be deleted. This can't be undone."
            : "History, dictionary, snippets, conversations, badges and Coach progress will be deleted. This can't be undone."
        }
        confirmLabel={confirm === "history" ? "Clear History" : "Reset"}

        onCancel={() => setConfirm(null)}
        onConfirm={() => {
          const cmd = confirm === "history" ? "clear_history" : "reset_app_data";
          setConfirm(null);
          call(cmd)
            .then(() => toast(cmd === "clear_history" ? "History cleared" : "App data reset", "success"))
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </>
  );
}

function AboutPane({ info }: { info: AppInfo | null }) {
  const { settings } = useSettings();
  const { phase, act } = useUpdater(settings?.auto_download_updates ?? true);
  return (
    <>
      <Section title="Orttaai">
        <Row label="Version" hint={`${info?.platform ?? "—"} · ${info?.session ?? "—"} · ${info?.gpu_backend ?? "cpu"} build`}>
          <span className="mono">v{info?.version ?? "—"}</span>
        </Row>
      </Section>
      <Section title="Updates">
        <Row label="Software update" hint="Signed updates from GitHub Releases. Orttaai never restarts without your click.">
          <Button onClick={act} disabled={phase.kind === "checking" || phase.kind === "installing"}>
            {phaseLabel(phase)}
          </Button>
        </Row>
        <ToggleRow k="auto_download_updates" label="Automatically download updates" hint="Download in the background; install when you choose." />
      </Section>
    </>
  );
}

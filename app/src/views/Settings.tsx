// Settings (Mac HomeSettingsWorkspaceView): title, tab bar and one centred column.
// Dictation · Text · Audio · Coach · General.
import { useEffect, useState, type ReactNode } from "react";
import { Button, ConfirmDialog, Icon, PageHeader, Segmented, useToast } from "../ui";
import type { AppInfo } from "../types";
import { call } from "../lib/api";
import { useSettings } from "../lib/settings";
import { Glyph, Row, Section, TabBar, ToggleRow, usePatch } from "../components/SettingsRows";
import DictationSettings from "./settings/Dictation";
import AudioSettings from "./settings/Audio";
import TextSettings from "./settings/Text";
import CoachSettings from "./settings/Coach";

type Pane = "dictation" | "text" | "audio" | "coach" | "general";

const PANES: { value: Pane; label: string; icon: ReactNode }[] = [
  { value: "dictation", label: "Dictation", icon: <Icon name="waveform" size={13} /> },
  { value: "text", label: "Text", icon: <Glyph name="alignLeft" /> },
  { value: "audio", label: "Audio", icon: <Icon name="mic" size={13} /> },
  { value: "coach", label: "Coach", icon: <Icon name="scope" size={13} /> },
  { value: "general", label: "General", icon: <Icon name="sliders" size={13} /> },
];

export default function Settings({ info }: { info: AppInfo | null; onRunSetup: () => void }) {
  const [pane, setPane] = useState<Pane>("dictation");
  // Each section opens at its top (the Mac rebuilds the scroll view per tab).
  useEffect(() => {
    document.querySelector(".content")?.scrollTo(0, 0);
  }, [pane]);
  return (
    <div className="view">
      <PageHeader title="Settings" leading={<TabBar tabs={PANES} value={pane} onChange={setPane} label="Settings sections" />} />
      <div className="st-column">
        {pane === "dictation" && <DictationSettings />}
        {pane === "text" && <TextSettings />}
        {pane === "audio" && <AudioSettings />}
        {pane === "coach" && <CoachSettings />}
        {pane === "general" && <General linux={info?.platform === "linux"} />}
      </div>
    </div>
  );
}

/** App-level behaviour and the destructive resets (Mac GeneralSettingsView). */
function General({ linux }: { linux: boolean }) {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [confirm, setConfirm] = useState<null | "history" | "reset">(null);
  if (!settings) return null;
  return (
    <>
      <Section title="App">
        <Row label="Appearance" hint="Light, dark, or whichever this computer is set to. The recording pill stays dark either way.">
          <Segmented
            value={settings.appearance}
            options={[
              { value: "system", label: "Match System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
            onChange={(v) => save({ appearance: v })}
          />
        </Row>
        <ToggleRow
          k="launch_at_login"
          label="Launch at Login"
          hint="Starts Orttaai when you log in to this computer so dictation is always ready."
        />
        <ToggleRow
          k="start_engine_on_launch"
          label="Start Dictation on Launch"
          hint="Loads the speech model as soon as Orttaai opens, so the shortcut works right away."
        />
        {/* Linux hides the pill while it processes, so the estimate would never show. */}
        {!linux && (
          <ToggleRow
            k="show_processing_estimate"
            label="Show Processing Estimate"
            hint="Shows roughly how long transcription will take in the pill while it finishes a recording."
          />
        )}
        <ToggleRow
          k="celebrate_milestones"
          label="Celebrate Milestones"
          hint="Shows a small card in the corner when a dictation earns a badge, and a celebration the next time you open Orttaai. Badges are still earned when this is off; see them in Milestones."
        />
        <ToggleRow k="sound_cues" label="Sound Cues" hint="Plays soft sounds when recording starts, stops, or fails." />
      </Section>
      <Section title="Danger Zone">
        <Row
          label="Clear History"
          hint="Permanently deletes every saved transcription and what was derived from it. Settings, Personal Memory, and models are kept."
        >
          <Button size="sm" className="st-btn st-danger" onClick={() => setConfirm("history")}>
            Clear History…
          </Button>
        </Row>
        <Row
          label="Reset App Data"
          hint="Clears history, Personal Memory, insights, conversations, badges and Coach progress. Settings and downloaded models are kept."
        >
          <Button size="sm" className="st-btn st-danger" onClick={() => setConfirm("reset")}>
            Reset App Data…
          </Button>
        </Row>
      </Section>
      <ConfirmDialog
        open={confirm !== null}
        title={confirm === "history" ? "Clear All History?" : "Reset App Data?"}
        body={
          confirm === "history"
            ? "This will permanently delete all transcriptions. This cannot be undone."
            : "This clears history, Personal Memory, insights, conversations, badges and Coach progress. This cannot be undone."
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

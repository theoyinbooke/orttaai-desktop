import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { runUpdate } from "../lib/updater";
import {
  Button,
  Card,
  Field,
  Icon,
  Kbd,
  PageHeader,
  Select,
  Toggle,
  useTheme,
  useToast,
  type ThemeChoice,
} from "../ui";
import type { AppInfo, Settings as S } from "../types";

export default function Settings(props: {
  settings: S | null;
  info: AppInfo | null;
  onSaved: () => void;
}) {
  const [form, setForm] = useState<S | null>(props.settings);
  const [dirty, setDirty] = useState(false);
  const [updateMsg, setUpdateMsg] = useState<string | null>(null);
  const { choice, setChoice } = useTheme();
  const toast = useToast();

  useEffect(() => setForm(props.settings), [props.settings]);

  if (!form)
    return (
      <div className="view">
        <PageHeader title="Settings" />
        <p className="muted">Loading…</p>
      </div>
    );

  const update = (patch: Partial<S>) => {
    setForm({ ...form, ...patch });
    setDirty(true);
  };

  async function save() {
    if (!form) return;
    try {
      await invoke("set_settings", { input: form });
      setDirty(false);
      props.onSaved();
      toast("Settings saved", "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function checkUpdates() {
    await runUpdate((s) => {
      switch (s.kind) {
        case "checking":
          return setUpdateMsg("Checking…");
        case "uptodate":
          return setUpdateMsg("You're up to date.");
        case "downloading":
          return setUpdateMsg(`Downloading ${s.version}…`);
        case "installed":
          return setUpdateMsg("Installed — restarting…");
        case "error":
          return setUpdateMsg(`Update check failed: ${s.message}`);
      }
    });
  }

  const themes: { value: ThemeChoice; label: string; icon: string }[] = [
    { value: "system", label: "System", icon: "monitor" },
    { value: "light", label: "Light", icon: "sun" },
    { value: "dark", label: "Dark", icon: "moon" },
  ];

  return (
    <div className="view settings stagger">
      <PageHeader
        title="Settings"
        actions={
          <Button variant="primary" icon="check" onClick={save} disabled={!dirty}>
            {dirty ? "Save changes" : "Saved"}
          </Button>
        }
      />

      <Card className="settings-section">
        <h2 className="section-h">Dictation</h2>
        <Field
          label="Push-to-talk shortcut"
          hint={
            <>
              Current: <Kbd combo={form.push_to_talk} />. On GNOME/Wayland you may also need to bind
              it once in Settings → Keyboard.
            </>
          }
        >
          <input
            className="text-input"
            value={form.push_to_talk}
            onChange={(e) => update({ push_to_talk: e.currentTarget.value })}
          />
        </Field>
        <div className="toggle-stack">
          <Toggle
            checked={form.preserve_clipboard}
            onChange={(v) => update({ preserve_clipboard: v })}
            label="Preserve clipboard"
            hint="Restore your clipboard after pasting a transcript on fallback."
          />
          <Toggle
            checked={form.strict_secure}
            onChange={(v) => update({ strict_secure: v })}
            label="Never type into password fields"
            hint="Refuses to insert unless the focused field is confirmed safe. On Linux field type can't be detected, so this blocks typing until you paste manually — leave off unless you dictate near password boxes."
          />
        </div>
      </Card>

      <Card className="settings-section">
        <h2 className="section-h">Performance</h2>
        <div className="field">
          <span className="field-label">Speed vs accuracy</span>
          <div className="segmented" role="radiogroup" aria-label="Decode preset">
            {(["fast", "balanced", "accuracy"] as const).map((p) => (
              <button
                key={p}
                role="radio"
                aria-checked={form.preset === p}
                className={`seg-btn ${form.preset === p ? "active" : ""}`}
                onClick={() => update({ preset: p })}
              >
                {p[0].toUpperCase() + p.slice(1)}
              </button>
            ))}
          </div>
          <span className="field-hint">
            Fast &amp; Balanced decode in a single deterministic pass (lowest latency). Accuracy
            re-tries hard audio at higher temperature — slower worst-case, more robust.
          </span>
        </div>
        <div className="field">
          <span className="field-label">Decode threads</span>
          <Select
            value={String(form.n_threads)}
            onChange={(v) => update({ n_threads: Number(v) })}
            ariaLabel="Decode threads"
            options={[
              { value: "0", label: "Auto (recommended)" },
              { value: "4", label: "4 threads" },
              { value: "6", label: "6 threads" },
              { value: "8", label: "8 threads" },
              { value: "12", label: "12 threads" },
              { value: "16", label: "16 threads" },
            ]}
          />
          <span className="field-hint">
            More isn't always faster on hybrid CPUs (performance + efficiency cores). Auto picks a
            sensible default. Takes effect next time you start the engine.
          </span>
        </div>
      </Card>

      <Card className="settings-section">
        <h2 className="section-h">Assistant</h2>
        <Field label="Ollama endpoint" hint="Where your local Ollama server is listening.">
          <input
            className="text-input"
            value={form.ollama_endpoint}
            onChange={(e) => update({ ollama_endpoint: e.currentTarget.value })}
          />
        </Field>
      </Card>

      <Card className="settings-section">
        <h2 className="section-h">Appearance</h2>
        <div className="segmented" role="radiogroup" aria-label="Theme">
          {themes.map((t) => (
            <button
              key={t.value}
              role="radio"
              aria-checked={choice === t.value}
              className={`seg-btn ${choice === t.value ? "active" : ""}`}
              onClick={() => setChoice(t.value)}
            >
              <Icon name={t.icon} size={16} />
              {t.label}
            </button>
          ))}
        </div>
      </Card>

      <Card className="settings-section">
        <h2 className="section-h">Updates</h2>
        <div className="row-between">
          <Button variant="ghost" icon="refresh" onClick={checkUpdates}>
            Check for updates
          </Button>
          {updateMsg && <span className="muted small">{updateMsg}</span>}
        </div>
        {props.info && (
          <p className="app-meta mono">
            {props.info.name} v{props.info.version} · {props.info.platform}
          </p>
        )}
      </Card>
    </div>
  );
}

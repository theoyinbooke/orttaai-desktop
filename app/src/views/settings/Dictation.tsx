// Settings → Dictation (Mac DictationSettingsView): how recording starts and stops,
// the spoken language, plus the desktop's typing options. Owned by the engine area.
import { useEffect, useState } from "react";
import { Segmented, Select } from "../../ui";
import { call } from "../../lib/api";
import { useSettings } from "../../lib/settings";
import { Notice, Row, Section, ToggleRow, usePatch } from "../../components/SettingsRows";
import ShortcutRecorder from "../../components/ShortcutRecorder";
import { useEngine, useExpressInUse, type ModelInfo } from "../../lib/engine";
import "../../styles/engine.css";

export const SUPPORTED_LANGUAGES: { code: string; name: string }[] = [
  { code: "en", name: "English" },
  { code: "es", name: "Spanish" },
  { code: "fr", name: "French" },
  { code: "de", name: "German" },
  { code: "ja", name: "Japanese" },
  { code: "zh", name: "Chinese" },
  { code: "ko", name: "Korean" },
  { code: "pt", name: "Portuguese" },
  { code: "it", name: "Italian" },
  { code: "nl", name: "Dutch" },
  { code: "ru", name: "Russian" },
  { code: "ar", name: "Arabic" },
  { code: "hi", name: "Hindi" },
  { code: "auto", name: "Auto-detect" },
];

const secondsLabel = (s: number) => (s >= 120 && s % 60 === 0 ? `${s / 60} min` : `${s}s`);
const minutesLabel = (s: number) => (s % 60 === 0 ? `${s / 60} min` : `${(s / 60).toFixed(1)} min`);
const silenceLabel = (s: number) => (Math.round(s) === s ? `${s}s` : `${s.toFixed(1)}s`);

/** Preset choices plus the stored value when it is a custom one (Mac `presetOptions`). */
function presets(values: number[], current: number, min: number, max: number, label: (n: number) => string) {
  const v = [...values];
  if (current >= min && current <= max && !v.includes(current)) {
    v.push(current);
    v.sort((a, b) => a - b);
  }
  return v.map((n) => ({ value: String(n), label: label(n) }));
}

export default function DictationSettings() {
  const { settings } = useSettings();
  const { snapshot } = useEngine();
  const save = usePatch();
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [platform, setPlatform] = useState<{ platform: string; session: string } | null>(null);
  const [expressModels, setExpressModels] = useState<{ id: string; multilingual: boolean }[]>([]);
  const expressInUse = useExpressInUse();
  useEffect(() => {
    call<ModelInfo[]>("list_models").then(setModels).catch(() => {});
    call<{ id: string; multilingual: boolean }[]>("express_models").then(setExpressModels).catch(() => {});
    call<{ platform: string; session: string }>("app_info").then(setPlatform).catch(() => {});
  }, []);
  if (!settings) return null;

  const hf = settings.hands_free_enabled;
  // "Tap Right Ctrl" recordings are always hands-free, so they use the same limits.
  const handsFreeLimits = hf || settings.right_ctrl_dictation_enabled;
  // A custom stored value is only listed while silence stop is on (Mac).
  const silenceOptions = [
    { value: "off", label: "Off" },
    ...presets(
      [2, 4, 6, 8, 10],
      settings.hands_free_silence_stop ? settings.hands_free_silence_secs : 4,
      1,
      10,
      silenceLabel,
    ),
  ];
  const silenceValue = settings.hands_free_silence_stop ? String(settings.hands_free_silence_secs) : "off";
  const activeModel = models.find((m) => m.id === settings.model_id);
  // Only matters when Whisper does the work.
  const englishOnlyMismatch = !expressInUse && activeModel?.english_only && settings.dictation_language !== "en";
  const linux = platform?.platform === "linux";
  // Express takes no language: v2 is English only (other languages fall back to
  // Whisper), the multilingual model detects the language itself.
  const expressChosen = settings.engine === "express";
  const expressModel = expressModels.find((m) => m.id === settings.express_model) ?? expressModels[0];
  const languageHint = expressChosen && expressModel
    ? expressModel.multilingual
      ? "Express detects the language you speak by itself. This choice applies when Whisper is in use."
      : expressInUse
        ? "Express is English only. Pick another language and dictation uses your Whisper model instead."
        : "Express is English only, so dictation in this language uses your Whisper model."
    : "The language you speak. Auto-detect handles any language but is slower.";
  const rightCtrl = settings.right_ctrl_dictation_enabled;
  const rightCtrlError = snapshot?.right_ctrl_error ?? null;

  return (
    <>
      <Section title="Recording">
        <Row
          label="Push to Talk Shortcut"
          hint={
            hf
              ? "Hold to dictate and release to insert. A quick tap starts a hands-free recording instead."
              : "Hold to dictate and release to insert."
          }
        >
          <ShortcutRecorder
            plain
            value={settings.push_to_talk}
            other={settings.edit_commands_enabled ? settings.edit_shortcut : null}
            onChange={(c) => save({ push_to_talk: c })}
          />
        </Row>
        <ToggleRow
          k="hands_free_enabled"
          label="Hands-Free Dictation"
          hint="Tap the shortcut to start, and tap again to stop, instead of holding it. The pill's stop button and Click to record work hands-free too."
        />
        <ToggleRow
          k="right_ctrl_dictation_enabled"
          label="Tap Right Ctrl to Dictate"
          hint={
            !rightCtrl && rightCtrlError
              ? rightCtrlError
              : "Tap the Right Ctrl key once to start talking, and tap it again to insert the text. Ctrl shortcuts still work as usual."
          }
          disabled={!rightCtrl && !!rightCtrlError}
        />
        {rightCtrl && rightCtrlError && <Notice kind="warning">{rightCtrlError}</Notice>}
        {handsFreeLimits && (
          <>
            <Row
              label="Stop After Silence"
              hint="Ends a hands-free recording once you stop talking for this long. It only starts counting after it hears you speak. Off means it runs until you stop it."
            >
              <Segmented
                value={silenceValue}
                options={silenceOptions}
                onChange={(v) =>
                  v === "off"
                    ? save({ hands_free_silence_stop: false })
                    : save({ hands_free_silence_stop: true, hands_free_silence_secs: Number(v) })
                }
              />
            </Row>
            <Row
              label="Hands-Free Limit"
              hint="The longest a hands-free recording can run. The pill counts down the final 20 seconds."
            >
              <Segmented
                value={String(settings.hands_free_max_secs)}
                options={presets([300, 600, 900, 1800], settings.hands_free_max_secs, 120, 1800, minutesLabel)}
                onChange={(v) => save({ hands_free_max_secs: Number(v) })}
              />
            </Row>
          </>
        )}
        <Row
          label="Push-to-Talk Limit"
          hint="The longest you can hold to talk in one go. The pill counts down the final 20 seconds."
        >
          <Segmented
            value={String(settings.max_recording_secs)}
            options={presets([30, 60, 90, 120, 300], settings.max_recording_secs, 10, 300, secondsLabel)}
            onChange={(v) => save({ max_recording_secs: Number(v) })}
          />
        </Row>
      </Section>

      <Section title="Language">
        <Row
          label="Dictation Language"
          hint={languageHint}
        >
          <Select
            value={settings.dictation_language}
            options={SUPPORTED_LANGUAGES.map((l) => ({ value: l.code, label: l.name }))}
            onChange={(v) => save({ dictation_language: v })}
            ariaLabel="Dictation language"
            className="eng-select"
          />
        </Row>
        {englishOnlyMismatch && (
          <Notice kind="warning">
            {activeModel?.name} only understands English. Pick a multilingual model in Model to dictate in{" "}
            {SUPPORTED_LANGUAGES.find((l) => l.code === settings.dictation_language)?.name ?? "this language"}.
          </Notice>
        )}
      </Section>

      {/* Desktop only: the Mac always types through Accessibility, Linux and Windows offer a choice. */}
      <Section title="Typing">
        <Row
          label="Insert Text By"
          hint="Auto pastes longer text (fast, keeps line breaks) and types short phrases. Type works everywhere; Paste is fastest."
        >
          <Segmented
            value={settings.injection_mode}
            options={[
              { value: "auto", label: "Auto" },
              { value: "type", label: "Typing" },
              { value: "paste", label: "Pasting" },
            ]}
            onChange={(v) => save({ injection_mode: v })}
          />
        </Row>
        <ToggleRow
          k="preserve_clipboard"
          label="Restore Clipboard After Pasting"
          hint="Puts back what you had copied once the dictation is pasted. Plain text only."
        />
        <ToggleRow
          k="strict_secure"
          label="Never Type Into Password Fields"
          hint={
            linux
              ? "Linux can't tell when a password field is focused, so this stops all typing; transcripts go to the clipboard instead."
              : "Orttaai never types into fields it knows are passwords. With this on it also skips fields it can't check."
          }
        />
      </Section>
    </>
  );
}

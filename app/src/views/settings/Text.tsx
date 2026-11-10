// Settings → Text (Mac TextSettingsView): what happens to the words after they
// are transcribed, plus the desktop's writing style and live before/after
// preview. Owned by the text-pipeline area.
import { useEffect, useRef, useState } from "react";
import { Segmented, Spinner } from "../../ui";
import { Notice, Row, Section, ToggleRow, usePatch } from "../../components/SettingsRows";
import ShortcutRecorder from "../../components/ShortcutRecorder";
import { useEvent } from "../../lib/api";
import { useSettings, type Settings } from "../../lib/settings";
import { useEngine } from "../../lib/engine";
import { memoryApi, type PipelinePreview } from "../../lib/memory";
import "../../styles/memory.css";

type StyleMode = Settings["style_mode"];

const STYLE_HINT: Record<StyleMode, string> = {
  raw: "Exactly what was heard, after your dictionary and snippets. No cleanup or formatting.",
  clean: "Removes fillers and stutters and applies spoken formatting and paragraphs.",
  formal: "Clean, then Polish rewrites it in a formal, professional register.",
  casual: "Clean, then Polish keeps it relaxed and conversational.",
};

const SAMPLES: { label: string; text: string }[] = [
  { label: "Fillers", text: "um so i think we we should ship the the update today, uh, before lunch" },
  { label: "List", text: "here is the plan new line number one review the budget number two send the report" },
  { label: "Breaks", text: "thanks for your help new paragraph best regards" },
];

export default function TextSettings() {
  const { settings } = useSettings();
  const { snapshot } = useEngine();
  const save = usePatch();
  const rightKeysError = snapshot?.right_shift_error ?? null;
  if (!settings) return null;
  const rightShift = settings.right_shift_edit_enabled;
  const raw = settings.style_mode === "raw";
  const styled = settings.style_mode === "formal" || settings.style_mode === "casual";
  const needsPolish = styled && !settings.polish_enabled;
  const styledByLocalModel = styled && settings.polish_enabled && settings.polish_engine === "s1_mini";
  const rawHint = "Off in the Raw style.";

  return (
    <>
      <Section title="Cleanup">
        <ToggleRow
          k="spoken_formatting"
          disabled={raw}
          label="Spoken Formatting"
          hint={
            raw
              ? rawHint
              : "Turns spoken commands like “new line” and “new paragraph” into real breaks, and “first, … second, …”, “number one” or “bullet point” into lists."
          }
        />
        <ToggleRow
          k="auto_paragraphs"
          disabled={raw}
          label="Automatic Paragraphs"
          hint={
            raw
              ? rawHint
              : "Splits a long dictation into paragraphs where you move to a new point. Never in terminals and other command-line apps."
          }
        />
        <ToggleRow
          k="fuzzy_dictionary"
          disabled={!settings.dictionary_enabled}
          label="Fuzzy Dictionary Matching"
          hint={
            settings.dictionary_enabled
              ? "Corrects near-miss spellings of your dictionary words, like Tematope for Temitope. English only."
              : "Needs Dictionary Replacements."
          }
        />
        <ToggleRow
          k="disfluency_cleanup"
          disabled={raw}
          label="Remove Fillers and Stutters"
          hint={
            raw
              ? rawHint
              : "Removes um and uh, collapses repeats like “the the” and “we don’t we don’t”, and capitalizes a lowercase i. English only."
          }
        />
      </Section>

      <Section title="Voice Editing">
        <ToggleRow
          k="edit_commands_enabled"
          label="Edit Selection with Voice"
          hint="Select text, press the edit shortcut, and say how to change it, like “make this more formal”. With nothing selected, the edit shortcut works on your last dictation. Uses the polish model set in Model › AI Features."
        />
        {settings.edit_commands_enabled && (
          <>
            <ToggleRow
              k="edit_with_dictation_key"
              label="Edit with the Dictation Shortcut"
              hint="Select text, press your dictation shortcut, and say an instruction like “make this shorter” or “fix the grammar”. Anything else you say replaces the selection, as before. English only, and only in apps that let Orttaai read the selection; elsewhere use the edit shortcut."
            />
            <ToggleRow
              k="right_shift_edit_enabled"
              label="Tap Right Shift to Edit"
              hint={
                !rightShift && rightKeysError
                  ? rightKeysError
                  : "Tap the Right Shift key, say how to change the selected text (or your last dictation, when nothing is selected), and tap it again to apply. Shift still works as usual."
              }
              disabled={!rightShift && !!rightKeysError}
            />
            {rightShift && rightKeysError && <Notice kind="warning">{rightKeysError}</Notice>}
            <Row label="Edit Shortcut">
              <ShortcutRecorder
                plain
                value={settings.edit_shortcut}
                other={settings.push_to_talk}
                onChange={(c) => save({ edit_shortcut: c })}
              />
            </Row>
          </>
        )}
      </Section>

      {/* Desktop only: the Mac has no writing style or preview. */}
      <Section title="Style" desc="How much Orttaai tidies what you say before it types it.">
        <Row
          label="Writing Style"
          hint={
            needsPolish
              ? `${STYLE_HINT[settings.style_mode]} Turn on Polish in Model › AI Features to apply it; until then dictation reads as Clean.`
              : styledByLocalModel
                ? `${STYLE_HINT[settings.style_mode]} S1-mini only cleans up, so this style is written by your polish model (${settings.polish_model || "gemma4:e2b"}).`
                : STYLE_HINT[settings.style_mode]
          }
        >
          <Segmented<StyleMode>
            value={settings.style_mode}
            options={[
              { value: "raw", label: "Raw" },
              { value: "clean", label: "Clean" },
              { value: "formal", label: "Formal" },
              { value: "casual", label: "Casual" },
            ]}
            onChange={(v) => save({ style_mode: v })}
          />
        </Row>
      </Section>

      <Section title="Try It" desc="Type what you might say. The right side shows what Orttaai would type with these settings and your memory.">
        <TryIt settings={settings} />
      </Section>
    </>
  );
}

function TryIt({ settings }: { settings: Settings }) {
  const [input, setInput] = useState(SAMPLES[0].text);
  const [preview, setPreview] = useState<PipelinePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [memoryVersion, setMemoryVersion] = useState(0);
  const seq = useRef(0);

  useEvent("memory-changed", () => setMemoryVersion((v) => v + 1));

  useEffect(() => {
    const id = ++seq.current;
    if (!input.trim()) {
      setPreview(null);
      setError(null);
      setBusy(false);
      return;
    }
    setBusy(true);
    const timer = window.setTimeout(() => {
      memoryApi
        .preview(input)
        .then((p) => {
          if (id !== seq.current) return;
          setPreview(p);
          setError(null);
        })
        .catch((e) => id === seq.current && setError(String(e)))
        .finally(() => id === seq.current && setBusy(false));
    }, 220);
    return () => window.clearTimeout(timer);
  }, [input, settings, memoryVersion]);

  const output = preview?.text ?? "";
  const same = output.trim() === input.trim();

  return (
    <>
      <div className="tp-try">
        <div className="tp-samples">
          <span>Try:</span>
          {SAMPLES.map((s) => (
            <button key={s.label} className="ghost-link" onClick={() => setInput(s.text)}>
              {s.label}
            </button>
          ))}
        </div>
        <label className="tp-pane">
          <span className="tp-pane-label">You say</span>
          <textarea className="text-input" value={input} onChange={(e) => setInput(e.currentTarget.value)} spellCheck={false} />
        </label>
        <div className="tp-pane">
          <span className="tp-pane-label">
            Orttaai types {busy && <Spinner size={11} />}
          </span>
          <div className={`tp-output ${same ? "is-same" : ""}`} aria-live="polite">
            {error ? <span className="muted">{error}</span> : output || <span className="muted">Nothing to type.</span>}
          </div>
        </div>
        {preview && preview.changes.length > 0 && (
          <div className="tp-changes" aria-label="What changed">
            {preview.changes.map((c, i) => (
              <span key={i} className="tp-change">
                {c.replace(/^(Spoken formatting|Disfluency|Formatting): /, "")}
              </span>
            ))}
          </div>
        )}
      </div>
      {preview?.style_instruction && (
        <div className="tp-note">
          {settings.polish_enabled
            ? "Polish then rewrites this in the chosen style; the preview shows the text before Polish."
            : "Formal and Casual are applied by Polish, which is off. Turn it on in Model › AI Features."}
        </div>
      )}
    </>
  );
}

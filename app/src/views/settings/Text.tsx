// Settings → Text (Mac TextSettingsView): what happens to the words after
// they are transcribed, with a live before/after preview. Owned by the
// text-pipeline area.
import { useEffect, useRef, useState } from "react";
import { Badge, Segmented, Spinner } from "../../ui";
import { Row, Section, ToggleRow, usePatch } from "../../components/SettingsRows";
import { useEvent } from "../../lib/api";
import { useSettings, type Settings } from "../../lib/settings";
import { memoryApi, usePlatform, type PipelinePreview } from "../../lib/memory";
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
  const save = usePatch();
  const platform = usePlatform();
  if (!settings) return null;
  const raw = settings.style_mode === "raw";
  const needsPolish = (settings.style_mode === "formal" || settings.style_mode === "casual") && !settings.polish_enabled;
  const rawHint = "Off in the Raw style.";
  const editsSupported = platform === "windows";

  return (
    <>
      <Section title="Style" desc="How much Orttaai tidies what you say before it types it.">
        <Row
          label="Writing style"
          hint={
            needsPolish
              ? `${STYLE_HINT[settings.style_mode]} Turn on Polish in AI Features to apply it; until then dictation reads as Clean.`
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

      <Section title="Personal Memory" desc="Add and edit entries on the Memory page.">
        <ToggleRow k="dictionary_enabled" label="Dictionary Replacements" hint="Replaces what you said with how you want it written, as you dictate." />
        <ToggleRow k="snippets_enabled" label="Snippet Expansions" hint="Say a trigger phrase and Orttaai types its full text." />
        <ToggleRow
          k="ai_suggestions"
          label="Prefer Local AI for Suggestions"
          hint="Analyze History asks your local model for dictionary words and snippets, and falls back to the built-in analyzer."
        />
        <ToggleRow
          k="learn_from_edits"
          disabled={!editsSupported}
          label={
            <>
              Learn from My Edits <Badge tone="neutral">Windows only</Badge>
            </>
          }
          hint={
            editsSupported
              ? "When you fix a misheard name right after dictating it, Orttaai suggests it as a dictionary word. Only the word pair is kept; password fields are never read."
              : "Needs Windows UI Automation to read the text box you dictated into."
          }
        />
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
            : "Formal and Casual are applied by Polish, which is off. Turn it on in AI Features."}
        </div>
      )}
    </>
  );
}

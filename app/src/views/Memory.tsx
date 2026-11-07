// Memory (Mac MemoryView): Dictionary · Snippets · Suggestions. Owned by the
// text-pipeline area.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Badge, Button, ConfirmDialog, CopyButton, Icon, IconButton, PageHeader, Spinner, Switch, useToast } from "../ui";
import { Glyph, Row, Section, TabBar, ToggleRow } from "../components/SettingsRows";
import { useSettings } from "../lib/settings";
import {
  countLabel,
  matches,
  memoryApi,
  relativeTime,
  useMemoryData,
  usePlatform,
  type DictionaryEntry,
  type EditCheck,
  type LearningSuggestion,
  type SnippetEntry,
} from "../lib/memory";
import "../styles/memory.css";

type Sub = "dictionary" | "snippets" | "suggestions";

const TABS: { value: Sub; label: string; icon: ReactNode }[] = [
  { value: "dictionary", label: "Dictionary", icon: <Icon name="book" size={13} /> },
  { value: "snippets", label: "Snippets", icon: <Glyph name="textPlus" /> },
  { value: "suggestions", label: "Suggestions", icon: <Glyph name="bulb" /> },
];

const EMPTY: Record<Sub, { title: string; message: string; icon: ReactNode }> = {
  dictionary: {
    title: "No dictionary entries",
    message: "Add common terms, names, and preferred spellings.",
    icon: <Icon name="textCheck" size={13} />,
  },
  snippets: {
    title: "No snippets yet",
    message: "Save short triggers for long text you repeat often.",
    icon: <Glyph name="textInsert" />,
  },
  suggestions: {
    title: "No pending suggestions",
    message: "Fix a misheard name right after dictating it and it appears here, or run Analyze History.",
    icon: <Glyph name="brain" />,
  },
};

type Flash = { tone: "info" | "error"; text: string; action?: { label: string; run: () => void } };

export default function Memory() {
  const data = useMemoryData();
  const toast = useToast();
  const [sub, setSub] = useState<Sub>("dictionary");
  const [query, setQuery] = useState("");
  const [flash, setFlash] = useState<Flash | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => setQuery(""), [sub]);

  const fail = (e: unknown) => setFlash({ tone: "error", text: String(e) });

  const exportMemory = () => {
    memoryApi
      .exportMemory()
      .then((path) =>
        setFlash({
          tone: "info",
          text: `Exported ${data.dictionary.length} dictionary ${data.dictionary.length === 1 ? "entry" : "entries"} and ${data.snippets.length} snippet${data.snippets.length === 1 ? "" : "s"} to ${path}`,
          action: { label: "Show in Folder", run: () => revealItemInDir(path).catch(fail) },
        }),
      )
      .catch(fail);
  };

  const importFile = async (file: File | undefined) => {
    if (!file) return;
    try {
      const s = await memoryApi.importMemory(await file.text());
      const parts = [
        s.dictionary_added && `${s.dictionary_added} new dictionary ${s.dictionary_added === 1 ? "entry" : "entries"}`,
        s.dictionary_updated && `${s.dictionary_updated} updated`,
        s.snippets_added && `${s.snippets_added} new snippet${s.snippets_added === 1 ? "" : "s"}`,
        s.snippets_updated && `${s.snippets_updated} snippet${s.snippets_updated === 1 ? "" : "s"} updated`,
        s.skipped && `${s.skipped} skipped`,
      ].filter(Boolean);
      toast(parts.length ? `Imported ${parts.join(", ")}.` : "Nothing to import in that file.", "success");
      setFlash(null);
    } catch (e) {
      fail(e);
    }
  };

  return (
    <div className="view memory-view">
      <PageHeader
        title="Memory"
        leading={<TabBar tabs={TABS} value={sub} onChange={setSub} label="Memory sections" />}
        actions={
          <label className="mem-search">
            <Icon name="search" size={13} />
            <input
              value={query}
              onChange={(e) => setQuery(e.currentTarget.value)}
              placeholder="Search entries"
              aria-label="Search entries"
            />
            {query && (
              <button className="mem-search-clear" aria-label="Clear search" onClick={() => setQuery("")}>
                <Glyph name="clearFill" size={13} />
              </button>
            )}
          </label>
        }
      />

      {(flash || data.error) && (
        <div className={`mem-flash ${flash?.tone === "error" || (!flash && data.error) ? "is-error" : ""}`} role="status">
          <span>{flash?.text ?? data.error}</span>
          {flash?.action && (
            <button className="ghost-link" onClick={flash.action.run}>
              {flash.action.label}
            </button>
          )}
          {flash && (
            <button className="mem-flash-x" aria-label="Dismiss" onClick={() => setFlash(null)}>
              <Icon name="x" size={12} />
            </button>
          )}
        </div>
      )}

      {data.loading ? (
        <div className="mem-loading">
          <Spinner size={13} /> <span>Loading memory entries…</span>
        </div>
      ) : sub === "dictionary" ? (
        <DictionaryTab entries={data.dictionary} query={query} onError={fail} />
      ) : sub === "snippets" ? (
        <SnippetsTab entries={data.snippets} query={query} onError={fail} />
      ) : (
        <SuggestionsTab
          suggestions={data.suggestions}
          checks={data.editChecks}
          query={query}
          onError={fail}
          onReport={(text) => setFlash({ tone: "info", text })}
        />
      )}

      {/* Desktop only: moving Personal Memory between computers, which the Mac does over iCloud. */}
      {!data.loading && sub !== "suggestions" && (
        <Section>
          <Row label="Import and Export" hint="Moves your dictionary and snippets between computers as a JSON file.">
            <Button size="sm" className="st-btn" onClick={() => fileRef.current?.click()}>
              Import…
            </Button>
            <Button size="sm" className="st-btn" onClick={exportMemory}>
              Export
            </Button>
            <input
              ref={fileRef}
              type="file"
              accept="application/json,.json"
              hidden
              onChange={(e) => {
                importFile(e.currentTarget.files?.[0]);
                e.currentTarget.value = "";
              }}
            />
          </Row>
        </Section>
      )}
    </div>
  );
}

/* ------------------------------------------------------------ shared bits -- */

function Empty({ sub, searching }: { sub: Sub; searching: boolean }) {
  const e = EMPTY[sub];
  const noun = sub === "dictionary" ? "dictionary" : sub === "snippets" ? "snippet" : "suggestion";
  return (
    <div className="mem-empty">
      <div className="mem-empty-title">
        {e.icon}
        <strong>{searching ? `No ${noun} matches` : e.title}</strong>
      </div>
      <p>{searching ? "Try a different search query." : e.message}</p>
    </div>
  );
}

function Used({ n }: { n: number }) {
  return <span className="mem-used">{n}×</span>;
}

/** Enter submits, Escape cancels (single-line inputs). */
function keys(onEnter: () => void, onEscape?: () => void) {
  return (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onEnter();
    } else if (e.key === "Escape" && onEscape) {
      e.preventDefault();
      onEscape();
    }
  };
}

function CaseToggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      className={`mem-case ${on ? "on" : ""}`}
      aria-pressed={on}
      aria-label="Case sensitive"
      title={on ? "Matches exact capitalization" : "Matches any capitalization"}
      onClick={() => onChange(!on)}
    >
      Aa
    </button>
  );
}

/** The add line doubles as the edit line, so bring it into view when editing starts. */
function focusEditor(input: HTMLElement | null) {
  input?.closest(".set-section")?.scrollIntoView({ block: "nearest" });
  input?.focus();
}

/* ------------------------------------------------------------- dictionary -- */

function DictionaryTab({ entries, query, onError }: { entries: DictionaryEntry[]; query: string; onError: (e: unknown) => void }) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [editing, setEditing] = useState<DictionaryEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState<DictionaryEntry | null>(null);
  const firstField = useRef<HTMLInputElement>(null);
  const shown = useMemo(() => entries.filter((e) => matches(query, e.source, e.target)), [entries, query]);
  const valid = source.trim() !== "" && target.trim() !== "";

  const reset = () => {
    setSource("");
    setTarget("");
    setCaseSensitive(false);
    setEditing(null);
  };
  const begin = (e: DictionaryEntry) => {
    setEditing(e);
    setSource(e.source);
    setTarget(e.target);
    setCaseSensitive(e.is_case_sensitive);
    focusEditor(firstField.current);
  };
  const save = () => {
    if (!valid || busy) return;
    setBusy(true);
    memoryApi
      .saveDictionary(
        editing
          ? { id: editing.id, source, target, caseSensitive, active: editing.is_active }
          : { source, target, caseSensitive },
      )
      .then(reset)
      .catch(onError)
      .finally(() => setBusy(false));
  };
  const onKeys = keys(save, editing ? reset : undefined);

  return (
    <>
      <Section>
        <ToggleRow
          k="dictionary_enabled"
          label="Dictionary Replacements"
          hint="Replaces what you said with how you want it written, as you dictate."
        />
        <ToggleRow
          k="vocabulary_bias"
          label="Bias Recognition to My Words"
          hint="Nudges speech recognition toward the words in your dictionary, so names are heard right the first time."
        />
      </Section>

      <Section
        title="Dictionary"
        desc="What speech recognition hears, and how Orttaai should write it."
        accessory={countLabel(entries.filter((e) => e.is_active).length, entries.length)}
      >
        <div className="mem-editor">
          <input ref={firstField} className="text-input" placeholder="Heard as" value={source} onChange={(e) => setSource(e.currentTarget.value)} onKeyDown={onKeys} aria-label="Heard as" />
          <span className="mem-editor-arrow" aria-hidden="true">
            <Icon name="arrowRight" size={11} />
          </span>
          <input className="text-input" placeholder="Written as" value={target} onChange={(e) => setTarget(e.currentTarget.value)} onKeyDown={onKeys} aria-label="Written as" />
          <CaseToggle on={caseSensitive} onChange={setCaseSensitive} />
          <Button variant="primary" size="sm" className="st-btn" onClick={save} disabled={!valid} loading={busy}>
            {editing ? "Update" : "Add"}
          </Button>
          {editing && (
            <Button size="sm" className="st-btn" onClick={reset}>
              Cancel
            </Button>
          )}
        </div>

        {shown.length === 0 ? (
          <Empty sub="dictionary" searching={query.trim() !== ""} />
        ) : (
          <div className="mem-table" role="table" aria-label="Dictionary entries">
            <div className="mem-row mem-head mem-dict" role="row">
              <span>Heard as</span>
              <span>Written as</span>
              <span>Used</span>
              <span>Active</span>
              <span />
            </div>
            {shown.map((e) => (
              <DictionaryRow key={e.id} entry={e} onError={onError} onEdit={() => begin(e)} onDelete={() => setDeleting(e)} />
            ))}
          </div>
        )}
      </Section>

      <ConfirmDialog
        open={deleting !== null}
        title="Delete Dictionary Entry?"
        body="This dictionary rule will no longer apply during dictation."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const id = deleting?.id;
          setDeleting(null);
          if (id === editing?.id) reset();
          if (id !== undefined) memoryApi.deleteDictionary(id).catch(onError);
        }}
      />
    </>
  );
}

function DictionaryRow({
  entry,
  onError,
  onEdit,
  onDelete,
}: {
  entry: DictionaryEntry;
  onError: (e: unknown) => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`mem-row mem-dict ${entry.is_active ? "" : "is-off"}`} role="row">
      <span className="mem-cell" title={entry.source}>
        {entry.source}
        {entry.is_case_sensitive && (
          <span className="mem-aa" title="Case sensitive">
            Aa
          </span>
        )}
      </span>
      <span className="mem-cell mem-target" title={entry.target}>
        {entry.target}
      </span>
      <Used n={entry.usage_count} />
      <span className="mem-switch">
        <Switch checked={entry.is_active} label={`${entry.source} active`} onChange={(v) => memoryApi.setDictionaryActive(entry.id, v).catch(onError)} />
      </span>
      <span className="mem-actions">
        <IconButton name="edit" label={`Edit ${entry.source}`} onClick={onEdit} />
        <IconButton name="trash" label={`Delete ${entry.source}`} className="mem-danger" onClick={onDelete} />
      </span>
    </div>
  );
}

/* --------------------------------------------------------------- snippets -- */

function SnippetsTab({ entries, query, onError }: { entries: SnippetEntry[]; query: string; onError: (e: unknown) => void }) {
  const [trigger, setTrigger] = useState("");
  const [expansion, setExpansion] = useState("");
  const [editing, setEditing] = useState<SnippetEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState<SnippetEntry | null>(null);
  const firstField = useRef<HTMLInputElement>(null);
  const shown = useMemo(() => entries.filter((e) => matches(query, e.trigger, e.expansion)), [entries, query]);
  const valid = trigger.trim() !== "" && expansion.trim() !== "";

  const reset = () => {
    setTrigger("");
    setExpansion("");
    setEditing(null);
  };
  const begin = (e: SnippetEntry) => {
    setEditing(e);
    setTrigger(e.trigger);
    setExpansion(e.expansion);
    focusEditor(firstField.current);
  };
  const save = () => {
    if (!valid || busy) return;
    setBusy(true);
    memoryApi
      .saveSnippet(editing ? { id: editing.id, trigger, expansion, active: editing.is_active } : { trigger, expansion })
      .then(reset)
      .catch(onError)
      .finally(() => setBusy(false));
  };

  return (
    <>
      <Section>
        <ToggleRow k="snippets_enabled" label="Snippet Expansions" hint="Say a trigger phrase and Orttaai types its full text." />
      </Section>

      <Section
        title="Snippets"
        desc="Short phrases that expand into text you repeat often."
        accessory={countLabel(entries.filter((e) => e.is_active).length, entries.length)}
      >
        <div className="mem-editor mem-editor-snippet">
          <input
            ref={firstField}
            className="text-input mem-trigger-field"
            placeholder="Trigger phrase"
            value={trigger}
            onChange={(e) => setTrigger(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && editing) reset();
            }}
            aria-label="Trigger phrase"
          />
          <textarea
            className="text-input"
            placeholder="Text it types"
            value={expansion}
            rows={Math.min(5, Math.max(1, expansion.split("\n").length))}
            onChange={(e) => setExpansion(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && editing) reset();
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                save();
              }
            }}
            aria-label="Text it types"
          />
          <Button variant="primary" size="sm" className="st-btn" onClick={save} disabled={!valid} loading={busy}>
            {editing ? "Update" : "Add"}
          </Button>
          {editing && (
            <Button size="sm" className="st-btn" onClick={reset}>
              Cancel
            </Button>
          )}
        </div>

        {shown.length === 0 ? (
          <Empty sub="snippets" searching={query.trim() !== ""} />
        ) : (
          <div className="mem-table" role="table" aria-label="Snippets">
            <div className="mem-row mem-head mem-snip" role="row">
              <span>Say</span>
              <span>Types</span>
              <span>Used</span>
              <span>Active</span>
              <span />
            </div>
            {shown.map((e) => (
              <SnippetRow key={e.id} entry={e} onError={onError} onEdit={() => begin(e)} onDelete={() => setDeleting(e)} />
            ))}
          </div>
        )}
      </Section>

      <ConfirmDialog
        open={deleting !== null}
        title="Delete Snippet?"
        body="This snippet trigger will no longer expand."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const id = deleting?.id;
          setDeleting(null);
          if (id === editing?.id) reset();
          if (id !== undefined) memoryApi.deleteSnippet(id).catch(onError);
        }}
      />
    </>
  );
}

function SnippetRow({
  entry,
  onError,
  onEdit,
  onDelete,
}: {
  entry: SnippetEntry;
  onError: (e: unknown) => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`mem-row mem-snip ${entry.is_active ? "" : "is-off"}`} role="row">
      <span className="mem-cell mem-target" title={entry.trigger}>
        {entry.trigger}
      </span>
      <span className="mem-cell" title={entry.expansion}>
        {entry.expansion.replace(/\n/g, " ")}
      </span>
      <Used n={entry.usage_count} />
      <span className="mem-switch">
        <Switch checked={entry.is_active} label={`${entry.trigger} active`} onChange={(v) => memoryApi.setSnippetActive(entry.id, v).catch(onError)} />
      </span>
      <span className="mem-actions">
        <CopyButton text={entry.expansion} label="Copy text" compact />
        <IconButton name="edit" label={`Edit ${entry.trigger}`} onClick={onEdit} />
        <IconButton name="trash" label={`Delete ${entry.trigger}`} className="mem-danger" onClick={onDelete} />
      </span>
    </div>
  );
}

/* ------------------------------------------------------------ suggestions -- */

/** Mac `MemoryView.symbol(for:)`. */
const CHECK_ICON: Record<EditCheck["outcome"], ReactNode> = {
  learned: <Icon name="book" size={13} />,
  already_known: <Icon name="check" size={13} />,
  edited_no_name: <Icon name="edit" size={13} />,
  no_change: <Icon name="check" size={13} />,
  text_not_found: <Glyph name="eyeSlash" />,
  unreadable: <Glyph name="eyeSlash" />,
  secure_field: <Icon name="lock" size={13} />,
  not_english: <Icon name="globe" size={13} />,
};

function SuggestionsTab({
  suggestions,
  checks,
  query,
  onError,
  onReport,
}: {
  suggestions: LearningSuggestion[];
  checks: EditCheck[];
  query: string;
  onError: (e: unknown) => void;
  onReport: (text: string) => void;
}) {
  const { settings } = useSettings();
  const platform = usePlatform();
  const [analyzing, setAnalyzing] = useState(false);
  const shown = useMemo(
    () => suggestions.filter((s) => matches(query, s.candidate_source, s.candidate_target)),
    [suggestions, query],
  );
  const editsSupported = platform === "windows";

  const analyze = () => {
    setAnalyzing(true);
    memoryApi
      .analyzeHistory()
      .then((r) => onReport(r.ai_error ? `${r.message} Local AI wasn't reachable: ${r.ai_error}` : r.message))
      .catch(onError)
      .finally(() => setAnalyzing(false));
  };

  return (
    <>
      <Section>
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
              ? "When you fix a name or term right after dictating it, Orttaai suggests it here as a dictionary word. Only the word pair is kept, never the text around it, and password fields are never read."
              : "Needs Windows UI Automation to read the text box you dictated into. Linux desktops don't offer an equivalent, so suggestions come from Analyze History here."
          }
        />
        <ToggleRow
          k="ai_suggestions"
          label="Prefer Local AI for Suggestions"
          hint="Uses your local model (Ollama or LM Studio, set in Model › AI Features) to propose dictionary words and snippets from your history. Nothing leaves this computer."
        />
      </Section>

      {settings?.learn_from_edits && editsSupported && (
        <Section
          title="Learning from your edits"
          desc="After each dictation, Orttaai watches the same text box for a few minutes. If you fix a misheard name or term in what you dictated, the fix appears below as a suggestion. Rewording and ordinary typos don't count. Only the outcome is kept here, never the text."
        >
          {checks.length === 0 ? (
            <p className="mem-pad">
              Nothing checked yet. Dictate into any app; if you then fix a misheard name in the same box, Orttaai will notice.
            </p>
          ) : (
            <ul className="mem-checks">
              {checks.map((c) => (
                <li key={c.id} className={c.outcome === "learned" ? "is-learned" : ""}>
                  <span className="mem-check-icon">{CHECK_ICON[c.outcome] ?? <Icon name="info" size={13} />}</span>
                  <span className="mem-check-text">{c.description}</span>
                  {c.app_name && <span className="mem-check-meta">{c.app_name}</span>}
                  <span className="mem-check-meta mem-check-when">{relativeTime(c.checked_at)}</span>
                </li>
              ))}
            </ul>
          )}
        </Section>
      )}

      <Section
        title="Suggestions"
        desc="Words and phrases found in your history that could become dictionary entries or snippets."
        accessory={
          <Button size="sm" className="st-btn" onClick={analyze} disabled={analyzing}>
            {analyzing ? "Analyzing…" : "Analyze History"}
          </Button>
        }
      >
        {shown.length === 0 ? (
          <Empty sub="suggestions" searching={query.trim() !== ""} />
        ) : (
          <div className="mem-table" role="table" aria-label="Suggestions">
            <div className="mem-row mem-head mem-sugg" role="row">
              <span>Type</span>
              <span>Heard as</span>
              <span>Suggested</span>
              <span>Confidence</span>
              <span />
            </div>
            {shown.map((s) => (
              <div key={s.id} className="mem-row mem-sugg" role="row">
                <span className="mem-type">
                  <span>{s.kind === "dictionary" ? "Dictionary" : "Snippet"}</span>
                  <span className="mem-origin">{s.origin === "edit" ? "Your edit" : "History"}</span>
                </span>
                <span className="mem-cell" title={s.candidate_source}>
                  {s.candidate_source}
                </span>
                <span className="mem-cell mem-target" title={s.evidence ?? s.candidate_target}>
                  {s.candidate_target.replace(/\n/g, " ")}
                </span>
                <span className="mem-used">{Math.round(s.confidence * 100)}%</span>
                <span className="mem-actions">
                  <Button size="sm" variant="primary" className="st-btn" onClick={() => memoryApi.acceptSuggestion(s.id).catch(onError)}>
                    Accept
                  </Button>
                  <Button size="sm" className="st-btn st-danger" onClick={() => memoryApi.dismissSuggestion(s.id).catch(onError)}>
                    Reject
                  </Button>
                </span>
              </div>
            ))}
          </div>
        )}
      </Section>
    </>
  );
}

// Memory (Mac MemoryView): Dictionary · Snippets · Suggestions. Owned by the
// text-pipeline area.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Badge, Button, ConfirmDialog, CopyButton, Icon, IconButton, PageHeader, Spinner, Switch, Tabs, useToast } from "../ui";
import { ToggleRow } from "../components/SettingsRows";
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

const EMPTY: Record<Sub, { title: string; message: string; icon: string }> = {
  dictionary: {
    title: "No dictionary entries",
    message: "Add common terms, names, and preferred spellings.",
    icon: "dictionary",
  },
  snippets: {
    title: "No snippets yet",
    message: "Save short triggers for long text you repeat often.",
    icon: "edit",
  },
  suggestions: {
    title: "No pending suggestions",
    message: "Fix a misheard name right after dictating it and it appears here, or run Analyze History.",
    icon: "sparkle",
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

  const pending = data.suggestions.length;

  return (
    <div className="view memory-view">
      <PageHeader
        title="Memory"
        desc="What speech recognition hears, how Orttaai should write it, and what it has learned from you."
        actions={
          <>
            <Button size="sm" icon="download" onClick={() => fileRef.current?.click()}>
              Import…
            </Button>
            <Button size="sm" icon="share" onClick={exportMemory}>
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
          </>
        }
      />

      <div className="mem-toolbar">
        <Tabs<Sub>
          value={sub}
          onChange={setSub}
          tabs={[
            { value: "dictionary", label: `Dictionary · ${data.dictionary.length}` },
            { value: "snippets", label: `Snippets · ${data.snippets.length}` },
            { value: "suggestions", label: pending ? `Suggestions · ${pending}` : "Suggestions" },
          ]}
        />
        <label className="mem-search">
          <Icon name="search" size={15} />
          <input
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder="Search entries"
            aria-label="Search entries"
          />
          {query && (
            <button className="mem-search-clear" aria-label="Clear search" onClick={() => setQuery("")}>
              <Icon name="x" size={13} />
            </button>
          )}
        </label>
      </div>

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
              <Icon name="x" size={13} />
            </button>
          )}
        </div>
      )}

      {data.loading ? (
        <div className="card mem-loading">
          <Spinner /> <span className="muted">Loading memory entries…</span>
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
    </div>
  );
}

/* ------------------------------------------------------------ shared bits -- */

function MemCard({ title, info, meta, children }: { title: string; info: string; meta?: ReactNode; children: ReactNode }) {
  return (
    <section className="mem-card">
      <header className="mem-card-head">
        <div>
          <h2>{title}</h2>
          <p>{info}</p>
        </div>
        {meta && <div className="mem-card-meta">{meta}</div>}
      </header>
      {children}
    </section>
  );
}

function Empty({ sub, searching }: { sub: Sub; searching: boolean }) {
  const e = EMPTY[sub];
  const noun = sub === "dictionary" ? "dictionary" : sub === "snippets" ? "snippet" : "suggestion";
  return (
    <div className="mem-empty">
      <Icon name={searching ? "search" : e.icon} size={16} />
      <div>
        <strong>{searching ? `No ${noun} matches` : e.title}</strong>
        <p>{searching ? "Try a different search query." : e.message}</p>
      </div>
    </div>
  );
}

function Used({ n }: { n: number }) {
  return <span className="mem-used mono">{n}×</span>;
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

/* ------------------------------------------------------------- dictionary -- */

function DictionaryTab({ entries, query, onError }: { entries: DictionaryEntry[]; query: string; onError: (e: unknown) => void }) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState<DictionaryEntry | null>(null);
  const shown = useMemo(() => entries.filter((e) => matches(query, e.source, e.target)), [entries, query]);
  const valid = source.trim() !== "" && target.trim() !== "";

  const add = () => {
    if (!valid || busy) return;
    setBusy(true);
    memoryApi
      .saveDictionary({ source, target, caseSensitive })
      .then(() => {
        setSource("");
        setTarget("");
        setCaseSensitive(false);
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };

  return (
    <>
      <div className="set-rows">
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
      </div>

      <MemCard
        title="Dictionary"
        info="What speech recognition hears, and how Orttaai should write it."
        meta={<span className="small muted">{countLabel(entries.filter((e) => e.is_active).length, entries.length)}</span>}
      >
        <div className="mem-editor">
          <input className="text-input" placeholder="Heard as" value={source} onChange={(e) => setSource(e.currentTarget.value)} onKeyDown={keys(add)} aria-label="Heard as" />
          <span className="mem-editor-arrow" aria-hidden="true">
            →
          </span>
          <input className="text-input" placeholder="Written as" value={target} onChange={(e) => setTarget(e.currentTarget.value)} onKeyDown={keys(add)} aria-label="Written as" />
          <CaseToggle on={caseSensitive} onChange={setCaseSensitive} />
          <Button variant="primary" size="sm" icon="plus" onClick={add} disabled={!valid} loading={busy}>
            Add
          </Button>
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
              <DictionaryRow key={e.id} entry={e} onError={onError} onDelete={() => setDeleting(e)} />
            ))}
          </div>
        )}
      </MemCard>

      <ConfirmDialog
        open={deleting !== null}
        title="Delete Dictionary Entry?"
        body="This dictionary rule will no longer apply during dictation."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const id = deleting?.id;
          setDeleting(null);
          if (id !== undefined) memoryApi.deleteDictionary(id).catch(onError);
        }}
      />
    </>
  );
}

function DictionaryRow({ entry, onError, onDelete }: { entry: DictionaryEntry; onError: (e: unknown) => void; onDelete: () => void }) {
  const [editing, setEditing] = useState(false);
  const [source, setSource] = useState(entry.source);
  const [target, setTarget] = useState(entry.target);
  const [cs, setCs] = useState(entry.is_case_sensitive);

  const begin = () => {
    setSource(entry.source);
    setTarget(entry.target);
    setCs(entry.is_case_sensitive);
    setEditing(true);
  };
  const save = () => {
    if (!source.trim() || !target.trim()) return;
    memoryApi
      .saveDictionary({ id: entry.id, source, target, caseSensitive: cs, active: entry.is_active })
      .then(() => setEditing(false))
      .catch(onError);
  };

  if (editing) {
    return (
      <div className="mem-row mem-dict is-editing" role="row">
        <input className="text-input" autoFocus value={source} onChange={(e) => setSource(e.currentTarget.value)} onKeyDown={keys(save, () => setEditing(false))} aria-label="Heard as" />
        <input className="text-input" value={target} onChange={(e) => setTarget(e.currentTarget.value)} onKeyDown={keys(save, () => setEditing(false))} aria-label="Written as" />
        <CaseToggle on={cs} onChange={setCs} />
        <span className="mem-actions">
          <Button size="sm" variant="primary" onClick={save} disabled={!source.trim() || !target.trim()}>
            Update
          </Button>
          <Button size="sm" onClick={() => setEditing(false)}>
            Cancel
          </Button>
        </span>
      </div>
    );
  }

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
      <span>
        <Switch checked={entry.is_active} label={`${entry.source} active`} onChange={(v) => memoryApi.setDictionaryActive(entry.id, v).catch(onError)} />
      </span>
      <span className="mem-actions">
        <IconButton name="edit" label={`Edit ${entry.source}`} onClick={begin} />
        <IconButton name="trash" label={`Delete ${entry.source}`} className="mem-danger" onClick={onDelete} />
      </span>
    </div>
  );
}

/* --------------------------------------------------------------- snippets -- */

function SnippetsTab({ entries, query, onError }: { entries: SnippetEntry[]; query: string; onError: (e: unknown) => void }) {
  const [trigger, setTrigger] = useState("");
  const [expansion, setExpansion] = useState("");
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState<SnippetEntry | null>(null);
  const shown = useMemo(() => entries.filter((e) => matches(query, e.trigger, e.expansion)), [entries, query]);
  const valid = trigger.trim() !== "" && expansion.trim() !== "";

  const add = () => {
    if (!valid || busy) return;
    setBusy(true);
    memoryApi
      .saveSnippet({ trigger, expansion })
      .then(() => {
        setTrigger("");
        setExpansion("");
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };

  return (
    <>
      <div className="set-rows">
        <ToggleRow k="snippets_enabled" label="Snippet Expansions" hint="Say a trigger phrase and Orttaai types its full text." />
      </div>

      <MemCard
        title="Snippets"
        info="Short phrases that expand into text you repeat often. Say the trigger on its own, or after “insert”."
        meta={<span className="small muted">{countLabel(entries.filter((e) => e.is_active).length, entries.length)}</span>}
      >
        <div className="mem-editor mem-editor-snippet">
          <input className="text-input mem-trigger-field" placeholder="Trigger phrase" value={trigger} onChange={(e) => setTrigger(e.currentTarget.value)} aria-label="Trigger phrase" />
          <textarea
            className="text-input"
            placeholder="Text it types"
            value={expansion}
            rows={Math.min(5, Math.max(1, expansion.split("\n").length))}
            onChange={(e) => setExpansion(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                add();
              }
            }}
            aria-label="Text it types"
          />
          <Button variant="primary" size="sm" icon="plus" onClick={add} disabled={!valid} loading={busy}>
            Add
          </Button>
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
              <SnippetRow key={e.id} entry={e} onError={onError} onDelete={() => setDeleting(e)} />
            ))}
          </div>
        )}
      </MemCard>

      <ConfirmDialog
        open={deleting !== null}
        title="Delete Snippet?"
        body="This snippet trigger will no longer expand."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const id = deleting?.id;
          setDeleting(null);
          if (id !== undefined) memoryApi.deleteSnippet(id).catch(onError);
        }}
      />
    </>
  );
}

function SnippetRow({ entry, onError, onDelete }: { entry: SnippetEntry; onError: (e: unknown) => void; onDelete: () => void }) {
  const [editing, setEditing] = useState(false);
  const [trigger, setTrigger] = useState(entry.trigger);
  const [expansion, setExpansion] = useState(entry.expansion);

  const begin = () => {
    setTrigger(entry.trigger);
    setExpansion(entry.expansion);
    setEditing(true);
  };
  const save = () => {
    if (!trigger.trim() || !expansion.trim()) return;
    memoryApi
      .saveSnippet({ id: entry.id, trigger, expansion, active: entry.is_active })
      .then(() => setEditing(false))
      .catch(onError);
  };

  if (editing) {
    return (
      <div className="mem-row mem-snip is-editing" role="row">
        <input className="text-input" autoFocus value={trigger} onChange={(e) => setTrigger(e.currentTarget.value)} onKeyDown={keys(save, () => setEditing(false))} aria-label="Trigger phrase" />
        <textarea
          className="text-input"
          value={expansion}
          rows={Math.min(5, Math.max(2, expansion.split("\n").length))}
          onChange={(e) => setExpansion(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setEditing(false);
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) save();
          }}
          aria-label="Text it types"
        />
        <span className="mem-actions">
          <Button size="sm" variant="primary" onClick={save} disabled={!trigger.trim() || !expansion.trim()}>
            Update
          </Button>
          <Button size="sm" onClick={() => setEditing(false)}>
            Cancel
          </Button>
        </span>
      </div>
    );
  }

  return (
    <div className={`mem-row mem-snip ${entry.is_active ? "" : "is-off"}`} role="row">
      <span className="mem-cell mem-target" title={entry.trigger}>
        {entry.trigger}
      </span>
      <span className="mem-cell" title={entry.expansion}>
        {entry.expansion.replace(/\n/g, " ")}
      </span>
      <Used n={entry.usage_count} />
      <span>
        <Switch checked={entry.is_active} label={`${entry.trigger} active`} onChange={(v) => memoryApi.setSnippetActive(entry.id, v).catch(onError)} />
      </span>
      <span className="mem-actions">
        <CopyButton text={entry.expansion} label="Copy text" compact />
        <IconButton name="edit" label={`Edit ${entry.trigger}`} onClick={begin} />
        <IconButton name="trash" label={`Delete ${entry.trigger}`} className="mem-danger" onClick={onDelete} />
      </span>
    </div>
  );
}

/* ------------------------------------------------------------ suggestions -- */

const CHECK_ICON: Record<EditCheck["outcome"], string> = {
  learned: "dictionary",
  already_known: "check",
  edited_no_name: "edit",
  no_change: "check",
  text_not_found: "search",
  unreadable: "x",
  secure_field: "shield",
  not_english: "globe",
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
      <div className="set-rows">
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
          hint="Uses your local model (Ollama or LM Studio, from AI Features) to propose dictionary words and snippets from your history. Nothing leaves this computer."
        />
      </div>

      {settings?.learn_from_edits && editsSupported && (
        <MemCard
          title="Learning from your edits"
          info="After each dictation, Orttaai watches the same text box for a few minutes. If you fix a misheard name or term in what you dictated, the fix appears below as a suggestion. Rewording and ordinary typos don't count. Only the outcome is kept here, never the text."
        >
          {checks.length === 0 ? (
            <p className="muted small mem-pad">
              Nothing checked yet. Dictate into any app; if you then fix a misheard name in the same box, Orttaai will notice.
            </p>
          ) : (
            <ul className="mem-checks">
              {checks.map((c) => (
                <li key={c.id} className={c.outcome === "learned" ? "is-learned" : ""}>
                  <Icon name={CHECK_ICON[c.outcome] ?? "info"} size={14} />
                  <span className="mem-check-text">{c.description}</span>
                  {c.app_name && <span className="muted small">{c.app_name}</span>}
                  <span className="muted small mem-check-when">{relativeTime(c.checked_at)}</span>
                </li>
              ))}
            </ul>
          )}
        </MemCard>
      )}

      <MemCard
        title="Suggestions"
        info="Words and phrases found in your history that could become dictionary entries or snippets."
        meta={
          <Button size="sm" icon="sparkle" onClick={analyze} loading={analyzing}>
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
                  <span className="faint">{s.origin === "edit" ? "Your edit" : "History"}</span>
                </span>
                <span className="mem-cell" title={s.candidate_source}>
                  {s.candidate_source}
                </span>
                <span className="mem-cell mem-target" title={s.evidence ?? s.candidate_target}>
                  {s.candidate_target.replace(/\n/g, " ")}
                </span>
                <span className="mem-conf">
                  <span className="mem-conf-bar">
                    <span style={{ width: `${Math.round(s.confidence * 100)}%` }} />
                  </span>
                  <span className="mono small">{Math.round(s.confidence * 100)}%</span>
                </span>
                <span className="mem-actions">
                  <Button size="sm" variant="primary" onClick={() => memoryApi.acceptSuggestion(s.id).catch(onError)}>
                    Accept
                  </Button>
                  <Button size="sm" className="mem-reject" onClick={() => memoryApi.dismissSuggestion(s.id).catch(onError)}>
                    Dismiss
                  </Button>
                </span>
              </div>
            ))}
          </div>
        )}
      </MemCard>
    </>
  );
}

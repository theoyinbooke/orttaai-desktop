// History (Mac HistoryView): search, app and kind filters, pages of 50, a
// detail view with the final text beside what was heard, every timing and how
// it was inserted; copy, use as a snippet, delete. Analytics/home area.
import { useCallback, useEffect, useState } from "react";
import "../styles/analytics.css";
import { Button, ConfirmDialog, EmptyState, Icon, Modal, Select, Spinner, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import {
  clockDuration,
  copyText,
  fmt,
  modelDisplayName,
  relativeTime,
  type HistoryPage,
  type TranscriptionRecord,
} from "../lib/analytics";

const PAGE = 50;
type Kind = "" | "dictation" | "edit";
const KINDS: { value: Kind; label: string }[] = [
  { value: "", label: "All kinds" },
  { value: "dictation", label: "Dictations" },
  { value: "edit", label: "Voice edits" },
];
const INJECTION: Record<string, string> = {
  typed: "Typed",
  paste: "Pasted",
  portal: "Typed (Wayland portal)",
  clipboard: "Left on the clipboard",
  failed: "Insert failed",
  blocked: "Blocked (secure field)",
};

const preview = (text: string, limit = 140) => {
  const single = text.replace(/[\r\n]+/g, " ").trim();
  return single.length > limit ? `${single.slice(0, limit)}...` : single;
};
const appName = (r: TranscriptionRecord) => (r.app && r.app.trim()) || "Unknown App";
const rawDiffers = (r: TranscriptionRecord) => !!r.raw_text && r.raw_text.trim() !== "" && r.raw_text.trim() !== r.text.trim();

export default function History() {
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [app, setApp] = useState("");
  const [kind, setKind] = useState<Kind>("");
  const [page, setPage] = useState(0);
  const [data, setData] = useState<HistoryPage | null>(null);
  const [allTotal, setAllTotal] = useState(0);
  const [apps, setApps] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [detail, setDetail] = useState<TranscriptionRecord | null>(null);
  const [pendingDelete, setPendingDelete] = useState<TranscriptionRecord | null>(null);
  const [snippet, setSnippet] = useState<TranscriptionRecord | null>(null);
  const [copied, setCopied] = useState<number | null>(null);

  useEffect(() => {
    const t = window.setTimeout(() => {
      setQuery(search.trim());
      setPage(0);
    }, 200);
    return () => window.clearTimeout(t);
  }, [search]);

  const load = useCallback(() => {
    call<HistoryPage>("history_page", {
      query: { search: query || null, app: app || null, entry_kind: kind || null, limit: PAGE, offset: page * PAGE },
    })
      .then((p) => {
        // Deleting the last row of the last page steps back a page.
        if (p.items.length === 0 && page > 0 && p.total > 0) setPage(Math.max(0, Math.ceil(p.total / PAGE) - 1));
        else setData(p);
        setError(null);
      })
      .catch((e) => setError(`Couldn't load history. ${e}`));
    call<HistoryPage>("history_page", { query: { limit: 1, offset: 0 } })
      .then((p) => setAllTotal(p.total))
      .catch(() => {});
    call<string[]>("history_apps").then(setApps).catch(() => {});
  }, [query, app, kind, page]);
  useEffect(load, [load]);
  useEvent("history-changed", load);

  const copy = async (r: TranscriptionRecord) => {
    try {
      await copyText(r.text, r.id);
      setCopied(r.id);
      window.setTimeout(() => setCopied((c) => (c === r.id ? null : c)), 1100);
    } catch (e) {
      toast(String(e), "error");
    }
  };

  const remove = async (r: TranscriptionRecord) => {
    setPendingDelete(null);
    if (r.id == null) return;
    try {
      await call("delete_transcription", { id: r.id });
      if (detail?.id === r.id) setDetail(null);
      load();
    } catch (e) {
      toast(`Couldn't delete dictation. ${e}`, "error");
    }
  };

  const filtered = !!(query || app || kind);
  const pages = data ? Math.max(1, Math.ceil(data.total / PAGE)) : 1;

  return (
    <div className="hist">
      <div className="hist-head">
        <div className="hist-search">
          <Icon name="search" size={15} />
          <input
            className="hist-search-input"
            placeholder="Search transcripts"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            aria-label="Search transcripts"
          />
          {search && (
            <button className="hist-clear" onClick={() => setSearch("")} aria-label="Clear search" title="Clear search">
              <Icon name="x" size={13} />
            </button>
          )}
        </div>
        <Select
          className="hist-filter"
          value={app}
          onChange={(v) => {
            setApp(v);
            setPage(0);
          }}
          options={[{ value: "", label: "All apps" }, ...apps.map((a) => ({ value: a, label: a }))]}
          ariaLabel="Filter by app"
        />
        <Select
          className="hist-filter"
          value={kind}
          onChange={(v) => {
            setKind(v);
            setPage(0);
          }}
          options={KINDS}
          ariaLabel="Filter by kind"
        />
        <span className="hist-count mono-num">
          {fmt(data?.total ?? 0)} / {fmt(allTotal)}
        </span>
      </div>

      {error && <div className="an-error">{error}</div>}

      {!data ? (
        <div className="ms-loading">
          <Spinner />
        </div>
      ) : allTotal === 0 ? (
        <EmptyState icon="mic" title="No transcriptions yet." desc="Hold your dictation shortcut and speak to get started." />
      ) : data.items.length === 0 ? (
        <EmptyState
          icon="search"
          title={query ? `No results for “${query}”` : "No dictations match these filters"}
          action={
            <Button
              variant="ghost"
              onClick={() => {
                setSearch("");
                setApp("");
                setKind("");
              }}
            >
              Clear filters
            </Button>
          }
        />
      ) : (
        <div className="card hist-table" role="table" aria-label="Dictation history">
          <div className="hist-row hist-header" role="row">
            <span className="c-time">Time</span>
            <span className="c-app">App</span>
            <span className="c-text">Transcript</span>
            <span className="c-dur">Duration</span>
            <span className="c-words">Words</span>
            <span className="c-lat">Latency</span>
            <span className="c-model">Model</span>
            <span className="c-act">Actions</span>
          </div>
          {data.items.map((r) => (
            <div
              key={r.id ?? r.created_at}
              className="hist-row"
              role="row"
              tabIndex={0}
              onClick={() => setDetail(r)}
              onKeyDown={(e) => e.key === "Enter" && setDetail(r)}
              aria-label={`${appName(r)}, ${preview(r.text, 80)}, ${r.word_count} words`}
            >
              <span className="c-time" title={new Date(r.created_at * 1000).toLocaleString()}>
                {relativeTime(r.created_at)}
              </span>
              <span className="c-app">{appName(r)}</span>
              <span className="c-text">
                {r.entry_kind === "edit" && (
                  <span className="edit-chip" title={r.edit_instruction ? `Voice edit: ${r.edit_instruction}` : "Voice edit"}>
                    <Icon name="edit" size={10} /> EDIT
                  </span>
                )}
                <span className="c-text-body">{preview(r.text)}</span>
              </span>
              <span className="c-dur mono-num">{clockDuration(r.duration_ms)}</span>
              <span className="c-words mono-num">{r.word_count}</span>
              <span className="c-lat mono-num">{fmt(Math.max(0, r.processing_ms))} ms</span>
              <span className="c-model mono" title={r.model_id}>
                {r.model_id.trim() || "Not set"}
              </span>
              <span className="c-act" onClick={(e) => e.stopPropagation()}>
                <button className="row-icon" onClick={() => setSnippet(r)} title="Create Snippet" aria-label="Create Snippet">
                  <Icon name="plus" size={14} />
                </button>
                <button className={`row-icon ${copied === r.id ? "ok" : ""}`} onClick={() => copy(r)} title={copied === r.id ? "Copied" : "Copy"} aria-label="Copy">
                  <Icon name={copied === r.id ? "check" : "copy"} size={14} />
                </button>
                <button className="row-icon danger" onClick={() => setPendingDelete(r)} title="Delete" aria-label="Delete">
                  <Icon name="trash" size={14} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}

      {data && data.total > PAGE && (
        <div className="pager">
          <Button variant="ghost" size="sm" icon="chevronLeft" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
            Newer
          </Button>
          <span className="pager-info">
            Page {page + 1} of {pages} · {fmt(page * PAGE + 1)}–{fmt(Math.min(data.total, (page + 1) * PAGE))} of {fmt(data.total)}
            {filtered ? " matching" : ""}
          </span>
          <Button variant="ghost" size="sm" disabled={page >= pages - 1} onClick={() => setPage((p) => p + 1)}>
            Older <Icon name="chevronRight" size={15} />
          </Button>
        </div>
      )}

      <Modal open={!!detail} onClose={() => setDetail(null)} labelledBy="hist-detail-title">
        {detail && (
          <HistoryDetail
            record={detail}
            copied={copied === detail.id}
            onCopy={() => copy(detail)}
            onSnippet={() => {
              setSnippet(detail);
              setDetail(null);
            }}
            onDelete={() => setPendingDelete(detail)}
            onClose={() => setDetail(null)}
          />
        )}
      </Modal>

      <ConfirmDialog
        open={!!pendingDelete}
        title="Delete Dictation?"
        body="This removes the transcript from local history."
        confirmLabel="Delete"
        onConfirm={() => pendingDelete && remove(pendingDelete)}
        onCancel={() => setPendingDelete(null)}
      />

      {snippet && <SnippetSheet record={snippet} onClose={() => setSnippet(null)} />}
    </div>
  );
}

export function HistoryDetail({
  record: r,
  copied,
  onCopy,
  onSnippet,
  onDelete,
  onClose,
}: {
  record: TranscriptionRecord;
  copied: boolean;
  onCopy: () => void;
  onSnippet?: () => void;
  onDelete?: () => void;
  onClose: () => void;
}) {
  const timings: [string, number | null][] = [
    ["Spoke for", r.duration_ms],
    ["Release → text", r.processing_ms],
    ["Transcribe", r.transcription_ms],
    ["Rules & cleanup", r.text_processing_ms],
    ["Polish", r.polish_ms],
    ["Insert", r.injection_ms],
  ];
  return (
    <div className="hist-detail">
      <header className="hist-detail-top">
        <div>
          <h2 id="hist-detail-title">Transcript</h2>
          <p className="muted">
            {appName(r)} • {new Date(r.created_at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}
          </p>
        </div>
        <Button variant="ghost" onClick={onClose}>
          Done
        </Button>
      </header>

      {r.entry_kind === "edit" && r.edit_instruction && (
        <div className="hist-edit-note">
          <Icon name="edit" size={13} /> Voice edit: “{r.edit_instruction}”
        </div>
      )}

      <div className="hist-detail-scroll">
        <div className="hist-final">
          <span className="overline">{r.polished ? "Final text · polished" : "Final text"}</span>
          <p className="hist-final-text">{r.text}</p>
        </div>
        {rawDiffers(r) && (
          <div className="hist-raw">
            <span className="overline">You said</span>
            <p>{r.raw_text}</p>
          </div>
        )}
        <div className="hist-timings">
          {timings.map(([label, ms]) => (
            <div key={label} className="hist-timing">
              <span className="hist-timing-label">{label}</span>
              <span className="hist-timing-value mono-num">{ms == null ? "–" : label === "Spoke for" ? clockDuration(ms) : `${fmt(ms)} ms`}</span>
            </div>
          ))}
        </div>
        <dl className="hist-facts">
          <div>
            <dt>Inserted</dt>
            <dd>{r.injection_method ? INJECTION[r.injection_method] ?? r.injection_method : "–"}</dd>
          </div>
          <div>
            <dt>Model</dt>
            <dd title={r.model_id}>{r.model_id ? modelDisplayName(r.model_id) : "Not set"}</dd>
          </div>
          <div>
            <dt>Microphone</dt>
            <dd>{r.audio_device || "Default"}</dd>
          </div>
          <div>
            <dt>Words</dt>
            <dd>{fmt(r.word_count)}</dd>
          </div>
        </dl>
      </div>

      <footer className="hist-detail-actions">
        {onDelete && (
          <Button variant="ghost" icon="trash" onClick={onDelete}>
            Delete
          </Button>
        )}
        <span className="celebration-spacer" />
        {onSnippet && (
          <Button variant="ghost" icon="plus" onClick={onSnippet}>
            Use as Snippet
          </Button>
        )}
        <Button variant="primary" icon={copied ? "check" : "copy"} onClick={onCopy}>
          {copied ? "Copied" : "Copy"}
        </Button>
      </footer>
    </div>
  );
}

function SnippetSheet({ record, onClose }: { record: TranscriptionRecord; onClose: () => void }) {
  const toast = useToast();
  const suggested = record.text.trim().split(/\s+/).slice(0, 4).join(" ").toLowerCase();
  const [trigger, setTrigger] = useState(suggested);
  const [saving, setSaving] = useState(false);
  const save = async () => {
    setSaving(true);
    try {
      await call("save_snippet", { id: null, trigger: trigger.trim(), expansion: record.text, active: true });
      toast(`Saved snippet “${trigger.trim()}” to Memory.`, "success");
      onClose();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setSaving(false);
    }
  };
  return (
    <Modal open onClose={onClose} labelledBy="snippet-title">
      <div className="hist-detail snippet-sheet">
        <header className="hist-detail-top">
          <div>
            <h2 id="snippet-title">Create Snippet</h2>
            <p className="muted">Save as a reusable snippet.</p>
          </div>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
        </header>
        <label className="field">
          <span className="field-label">Trigger phrase</span>
          <input
            className="text-input"
            value={trigger}
            autoFocus
            placeholder="Type a short phrase like meeting recap"
            onChange={(e) => setTrigger(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && trigger.trim() && save()}
          />
        </label>
        <div className="field">
          <span className="field-label">Expansion preview</span>
          <p className="hist-final-text snippet-preview">{record.text}</p>
        </div>
        <footer className="hist-detail-actions">
          <span className="celebration-spacer" />
          <Button variant="primary" onClick={save} loading={saving} disabled={!trigger.trim()}>
            Save Snippet
          </Button>
        </footer>
      </div>
    </Modal>
  );
}

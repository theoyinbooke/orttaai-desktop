// History (Mac HistoryView): a search over transcript, app and model, then the
// whole archive in one scrolling table. A row opens the transcript; copy, use
// as a snippet, delete. Analytics/home area.
import { useCallback, useEffect, useMemo, useRef, useState, type UIEvent } from "react";
import "../styles/analytics.css";
import { Button, ConfirmDialog, Icon, Modal, Select, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { comboToString, useSettings } from "../lib/settings";
import { clockDuration, copyText, fmt, modelDisplayName, relativeTime, type HistoryPage, type TranscriptionRecord } from "../lib/analytics";

/** The most the backend hands over at once. */
const FETCH = 1000;
/** Rows drawn at a time; more as the table nears its end (the Mac's LazyVStack). */
const DRAW = 80;

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

const flatten = (text: string) => text.replace(/[\r\n]+/g, " ").trim();
const preview = (text: string, limit = 140) => {
  const single = flatten(text);
  return single.length > limit ? `${single.slice(0, limit)}...` : single;
};
const appName = (r: TranscriptionRecord) => (r.app && r.app.trim()) || "Unknown App";

// Orttaai Link: dictations from other devices (through a linked Mac) say where
// they were said. No device recorded means this PC.
const DEVICE_KIND: Record<string, string> = {
  mac: "Mac",
  iphone: "iPhone",
  ipad: "iPad",
  windows: "Windows",
  linux: "Linux",
  android: "Android",
};
type DeviceNames = Record<string, string>;
const fromElsewhere = (r: TranscriptionRecord, me: string | null) =>
  !!r.source_device_id && r.source_device_id !== me;
const deviceLabel = (r: TranscriptionRecord, names: DeviceNames) =>
  (r.source_device_id && names[r.source_device_id]) || DEVICE_KIND[r.source_device_kind ?? ""] || "Another device";
const deviceIcon = (kind: string | null | undefined) =>
  kind === "mac" ? "laptop" : kind === "iphone" || kind === "ipad" || kind === "android" ? "phone" : "pc";
const modelName = (r: TranscriptionRecord) => r.model_id.trim() || "Not set";
const rawDiffers = (r: TranscriptionRecord) => !!r.raw_text && r.raw_text.trim() !== "" && r.raw_text.trim() !== r.text.trim();

/** The Mac's `laptopcomputer`, marking the device a dictation came from. */
function LaptopGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="4.5" y="5" width="15" height="11" rx="1.5" />
      <path d="M2 19h20" />
    </svg>
  );
}

/** The Mac's `text.badge.plus`. */
function SnippetGlyph() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3 6h12M3 12h9M3 18h7M18 11v8M14 15h8" />
    </svg>
  );
}

export default function History() {
  const toast = useToast();
  const { settings } = useSettings();
  const [search, setSearch] = useState("");
  const [app, setApp] = useState("");
  const [kind, setKind] = useState<Kind>("");
  const [device, setDevice] = useState("");
  const [devices, setDevices] = useState<{ deviceId: string; name: string; kind: string; thisDevice: boolean }[]>([]);
  const me = devices.find((d) => d.thisDevice)?.deviceId ?? null;
  const deviceNames = useMemo(() => Object.fromEntries(devices.map((d) => [d.deviceId, d.name])), [devices]);
  const [apps, setApps] = useState<string[]>([]);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filtersRef = useRef<HTMLDivElement>(null);
  const [records, setRecords] = useState<TranscriptionRecord[] | null>(null);
  const [drawn, setDrawn] = useState(DRAW);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [detail, setDetail] = useState<TranscriptionRecord | null>(null);
  const [pendingDelete, setPendingDelete] = useState<TranscriptionRecord | null>(null);
  const [snippet, setSnippet] = useState<TranscriptionRecord | null>(null);
  const [copied, setCopied] = useState<number | null>(null);
  const gen = useRef(0);

  const load = useCallback(async () => {
    const g = ++gen.current;
    try {
      const all: TranscriptionRecord[] = [];
      for (;;) {
        const page = await call<HistoryPage>("history_page", { query: { limit: FETCH, offset: all.length } });
        all.push(...page.items);
        if (page.items.length === 0 || all.length >= page.total) break;
      }
      // An older load never replaces a newer one's rows.
      if (g !== gen.current) return;
      setRecords(all);
      setError(null);
    } catch {
      if (g === gen.current) setError("Couldn't load history.");
    }
    call<string[]>("history_apps").then(setApps).catch(() => {});
    call<typeof devices>("link_devices").then(setDevices).catch(() => {});
  }, []);
  useEffect(() => {
    load();
  }, [load]);
  useEvent("history-changed", load);

  useEffect(() => {
    if (!filtersOpen) return;
    const away = (e: MouseEvent) => {
      if (!filtersRef.current?.contains(e.target as Node)) setFiltersOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [filtersOpen]);

  const query = search.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!records) return [];
    return records.filter(
      (r) =>
        (!app || (r.app ?? "").trim() === app) &&
        (!kind || r.entry_kind === kind) &&
        (!device || (device === "this" ? !fromElsewhere(r, me) : r.source_device_id === device)) &&
        (!query || flatten(r.text).toLowerCase().includes(query) || appName(r).toLowerCase().includes(query) || modelName(r).toLowerCase().includes(query)),
    );
  }, [records, query, app, kind, device, me]);
  useEffect(() => setDrawn(DRAW), [query, app, kind, device]);
  const filtering = !!(app || kind || device);
  const clearAll = () => {
    setSearch("");
    setApp("");
    setKind("");
    setDevice("");
  };
  // Only devices that have dictations here, and only once there's more than one.
  const deviceOptions = useMemo(() => {
    if (!records) return [];
    const present = new Set(records.filter((r) => fromElsewhere(r, me)).map((r) => r.source_device_id as string));
    if (present.size === 0) return [];
    return [
      { value: "", label: "All devices" },
      { value: "this", label: "This PC" },
      ...devices.filter((d) => present.has(d.deviceId)).map((d) => ({ value: d.deviceId, label: d.name })),
    ];
  }, [records, devices, me]);

  const onScroll = (e: UIEvent<HTMLDivElement>) => {
    const t = e.currentTarget;
    if (t.scrollHeight - t.scrollTop - t.clientHeight < 400) setDrawn((d) => (d < filtered.length ? d + DRAW : d));
  };

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
      setError(null);
      if (detail?.id === r.id) setDetail(null);
      load();
    } catch {
      setError("Couldn't delete dictation.");
    }
  };

  return (
    <div className="hist">
      <div className="hist-head">
        <div className="hist-title">
          <h2>History</h2>
          <span className="hist-count mono-num">
            {filtered.length} / {records?.length ?? 0}
          </span>
        </div>
        <div className="hist-search">
          <Icon name="search" size={13} />
          <input
            className="hist-search-input"
            placeholder="Search transcript, app, or model"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            aria-label="Search transcript, app, or model"
          />
          {search && (
            <button className="hist-clear" onClick={() => setSearch("")} aria-label="Clear search" title="Clear search">
              <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true">
                <circle cx="12" cy="12" r="10" fill="currentColor" />
                <path d="m8.5 8.5 7 7m0-7-7 7" fill="none" stroke="var(--bg)" strokeWidth="2.2" strokeLinecap="round" />
              </svg>
            </button>
          )}
          <div className="hist-filters" ref={filtersRef}>
            <button
              className={`hist-filter-btn ${filtering ? "on" : ""}`}
              onClick={() => setFiltersOpen((o) => !o)}
              aria-expanded={filtersOpen}
              aria-label="Filter by app or kind"
              title="Filter by app or kind"
            >
              <Icon name="filter" size={13} />
            </button>
            {filtersOpen && (
              <div className="hist-filter-pop">
                <Select value={app} onChange={setApp} options={[{ value: "", label: "All apps" }, ...apps.map((a) => ({ value: a, label: a }))]} ariaLabel="Filter by app" />
                <Select value={kind} onChange={setKind} options={KINDS} ariaLabel="Filter by kind" />
                {deviceOptions.length > 0 && (
                  <Select value={device} onChange={setDevice} options={deviceOptions} ariaLabel="Filter by device" />
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {error && <div className="an-error">{error}</div>}
      {info && <div className="hist-info">{info}</div>}

      {records === null ? null : records.length === 0 ? (
        <div className="hist-state">
          <svg className="hist-state-mark" viewBox="0 0 100 84" aria-hidden="true">
            <rect x="0" y="44" width="18" height="36" rx="9" />
            <rect x="25" y="28" width="18" height="52" rx="9" />
            <rect x="50" y="12" width="18" height="68" rx="9" />
            <rect x="85" y="3" width="6" height="78" rx="3" />
            <rect x="77" y="0" width="22" height="6" rx="3" />
            <rect x="77" y="78" width="22" height="6" rx="3" />
          </svg>
          <p className="hist-state-title">No transcriptions yet.</p>
          <p className="hist-state-hint">Press {comboToString(settings?.push_to_talk)} to start dictating.</p>
        </div>
      ) : filtered.length === 0 ? (
        <div className="hist-state tight">
          <p className="hist-none">{query ? `No results for "${search}"` : "No dictations match these filters"}</p>
          <Button variant="ghost" onClick={clearAll}>
            {filtering ? "Clear Filters" : "Clear Search"}
          </Button>
        </div>
      ) : (
        <div className="card hist-table" role="table" aria-label="Dictation history" onScroll={onScroll}>
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
          {filtered.slice(0, drawn).map((r) => (
            <div
              key={r.id ?? r.created_at}
              className="hist-row"
              role="row"
              tabIndex={0}
              onClick={() => setDetail(r)}
              onKeyDown={(e) => e.key === "Enter" && setDetail(r)}
              aria-label={`${appName(r)}${fromElsewhere(r, me) ? ` on ${deviceLabel(r, deviceNames)}` : ""}, ${preview(r.text, 80)}, duration ${clockDuration(r.duration_ms)}, ${r.word_count} words, ${Math.max(0, r.processing_ms)} milliseconds.`}
            >
              <span className="c-time" title={new Date(r.created_at * 1000).toLocaleString()}>
                {relativeTime(r.created_at)}
              </span>
              <span className="c-app" title={fromElsewhere(r, me) ? `${appName(r)} · ${deviceLabel(r, deviceNames)}` : undefined}>
                <span className="c-app-icon">
                  {fromElsewhere(r, me) ? <Icon name={deviceIcon(r.source_device_kind)} size={14} /> : <LaptopGlyph />}
                </span>
                <span className="c-app-name">
                  {appName(r)}
                  {fromElsewhere(r, me) && <span className="c-app-device">{deviceLabel(r, deviceNames)}</span>}
                </span>
              </span>
              <span className="c-text">
                {r.entry_kind === "edit" && (
                  <span className="edit-chip" title={r.edit_instruction ? `Voice edit: ${r.edit_instruction}` : "Voice edit"} aria-label="Voice edit command">
                    <Icon name="edit" size={9} /> EDIT
                  </span>
                )}
                <span className="c-text-body">{preview(r.text)}</span>
              </span>
              <span className="c-dur mono-num">{clockDuration(r.duration_ms)}</span>
              <span className="c-words mono-num">{r.word_count}</span>
              <span className="c-lat mono-num">{Math.max(0, r.processing_ms)} ms</span>
              <span className="c-model mono" title={r.model_id}>
                {modelName(r)}
              </span>
              <span className="c-act" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                <button className="row-icon" onClick={() => setSnippet(r)} title="Create Snippet" aria-label="Create Snippet">
                  <SnippetGlyph />
                </button>
                <button className={`row-icon ${copied === r.id ? "ok" : ""}`} onClick={() => copy(r)} title={copied === r.id ? "Copied" : "Copy"} aria-label={copied === r.id ? "Copied" : "Copy"}>
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

      {snippet && (
        <SnippetSheet
          record={snippet}
          onClose={() => setSnippet(null)}
          onSaved={(trigger) => {
            setInfo(`Saved snippet "${trigger}" to Memory.`);
            setError(null);
          }}
          onFailed={(message) => {
            setInfo(null);
            setError(message);
          }}
        />
      )}
    </div>
  );
}

export function HistoryDetail({
  record: r,
  copied,
  onCopy,
  onSnippet,
  onClose,
}: {
  record: TranscriptionRecord;
  copied: boolean;
  onCopy: () => void;
  onSnippet?: () => void;
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
          <p>
            {appName(r)} • {new Date(r.created_at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}
          </p>
        </div>
        <Button variant="ghost" onClick={onClose}>
          Done
        </Button>
      </header>

      {r.entry_kind === "edit" && r.edit_instruction && (
        <div className="hist-edit-note">
          <Icon name="edit" size={11} /> Voice edit: “{r.edit_instruction}”
        </div>
      )}

      <div className="hist-detail-scroll">
        <p className="hist-final-text">{r.text}</p>
        {rawDiffers(r) && (
          <div className="hist-raw">
            <span>You said</span>
            <p>{r.raw_text}</p>
          </div>
        )}
        <details className="hist-more">
          <summary>Timings and details</summary>
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
              <dt>Text</dt>
              <dd>{r.polished ? "Polished" : "As transcribed"}</dd>
            </div>
          </dl>
        </details>
      </div>

      <footer className="hist-detail-actions">
        <span className="hist-fact">
          <Icon name="dictionary" size={13} /> {fmt(r.word_count)} words
        </span>
        <span className="celebration-spacer" />
        <span className="hist-fact faint">
          <Icon name="clock" size={13} /> {clockDuration(r.duration_ms)}
        </span>
        <span className="hist-fact faint">
          <Icon name="speedometer" size={13} /> {Math.max(0, r.processing_ms)} ms
        </span>
        {copied ? (
          <Button variant="ghost" className="hist-copied" icon="check" onClick={onCopy}>
            Copied
          </Button>
        ) : (
          <Button variant="primary" icon="copy" onClick={onCopy}>
            Copy
          </Button>
        )}
        {onSnippet && (
          <Button variant="ghost" icon="plus" onClick={onSnippet}>
            Use as Snippet
          </Button>
        )}
      </footer>
    </div>
  );
}

function SnippetSheet({
  record,
  onClose,
  onSaved,
  onFailed,
}: {
  record: TranscriptionRecord;
  onClose: () => void;
  onSaved: (trigger: string) => void;
  onFailed: (message: string) => void;
}) {
  const suggested = record.text.trim().split(/\s+/).slice(0, 4).join(" ").toLowerCase();
  const [trigger, setTrigger] = useState(suggested);
  const [saving, setSaving] = useState(false);
  const save = async () => {
    const phrase = trigger.trim();
    setSaving(true);
    try {
      await call("save_snippet", { id: null, trigger: phrase, expansion: record.text, active: true });
      onSaved(phrase);
    } catch (e) {
      onFailed(String(e));
    } finally {
      setSaving(false);
      onClose();
    }
  };
  return (
    <Modal open onClose={onClose} labelledBy="snippet-title">
      <div className="hist-detail snippet-sheet">
        <header className="hist-detail-top">
          <div>
            <h2 id="snippet-title">Create Snippet</h2>
            <p>Save as a reusable snippet.</p>
          </div>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
        </header>
        <label className="snippet-field">
          <span>Trigger phrase</span>
          <input
            className="hist-input"
            value={trigger}
            autoFocus
            placeholder="Type a short phrase like meeting recap"
            onChange={(e) => setTrigger(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && trigger.trim() && save()}
          />
        </label>
        <div className="snippet-field grow">
          <span>Expansion preview</span>
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

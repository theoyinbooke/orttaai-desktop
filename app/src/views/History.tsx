import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, ConfirmDialog, CopyButton, EmptyState, Icon, Modal, PageHeader, useToast } from "../ui";
import type { HistoryItem } from "../types";

const PER_PAGE = 20;

function relativeTime(unix: number): string {
  const diff = Date.now() / 1000 - unix;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return new Date(unix * 1000).toLocaleDateString();
}

export default function History({ items }: { items: HistoryItem[] }) {
  const [selected, setSelected] = useState<HistoryItem | null>(null);
  const [pendingDelete, setPendingDelete] = useState<HistoryItem | null>(null);
  const [page, setPage] = useState(1);
  const toast = useToast();

  const pageCount = Math.max(1, Math.ceil(items.length / PER_PAGE));
  // Keep the page valid if items shrink (e.g. after deleting the last row on a page).
  useEffect(() => {
    if (page > pageCount) setPage(pageCount);
  }, [page, pageCount]);
  const pageItems = items.slice((page - 1) * PER_PAGE, page * PER_PAGE);

  async function confirmDelete() {
    const item = pendingDelete;
    setPendingDelete(null);
    if (!item) return;
    if (selected?.id === item.id) setSelected(null);
    try {
      await invoke("delete_transcription", { id: item.id });
    } catch (e) {
      toast(String(e), "error");
    }
  }

  if (items.length === 0) {
    return (
      <div className="view">
        <PageHeader title="History" desc="Everything you've dictated, newest first." />
        <EmptyState
          icon="history"
          title="No transcriptions yet"
          desc="Dictate something and it'll appear here — click any row to read the full text."
        />
      </div>
    );
  }

  return (
    <div className="view stagger">
      <PageHeader
        title="History"
        desc={`${items.length} transcription${items.length === 1 ? "" : "s"} — click a row to read the full text.`}
      />

      <div className="htable-wrap">
        <table className="htable">
          <thead>
            <tr>
              <th className="col-text">Transcript</th>
              <th className="col-app">App</th>
              <th className="col-words">Words</th>
              <th className="col-when">When</th>
              <th className="col-act" aria-label="Actions" />
            </tr>
          </thead>
          <tbody>
            {pageItems.map((item) => (
              <tr key={item.id} className="htable-row" onClick={() => setSelected(item)}>
                <td className="col-text">
                  <span className="cell-text">{item.text}</span>
                </td>
                <td className="col-app muted">{item.app ?? "unknown"}</td>
                <td className="col-words mono">{item.word_count}</td>
                <td className="col-when mono muted">{relativeTime(item.created_at)}</td>
                <td className="col-act">
                  <div className="row-actions">
                    <CopyButton text={item.text} compact />
                    <button
                      className="row-del"
                      aria-label="Delete"
                      title="Delete"
                      onClick={(e) => {
                        e.stopPropagation();
                        setPendingDelete(item);
                      }}
                    >
                      <Icon name="trash" size={15} />
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {pageCount > 1 && (
        <div className="pager">
          <Button
            variant="ghost"
            size="sm"
            icon="chevronLeft"
            disabled={page <= 1}
            onClick={() => setPage((p) => Math.max(1, p - 1))}
          >
            Prev
          </Button>
          <span className="pager-info muted small">
            Page {page} of {pageCount}
          </span>
          <Button
            variant="ghost"
            size="sm"
            icon="chevronRight"
            disabled={page >= pageCount}
            onClick={() => setPage((p) => Math.min(pageCount, p + 1))}
          >
            Next
          </Button>
        </div>
      )}

      <Modal open={selected !== null} onClose={() => setSelected(null)} labelledBy="hist-title">
        {selected && (
          <div className="hist-detail">
            <div className="hist-detail-head">
              <span className="overline" id="hist-title">
                Transcript
              </span>
              <button className="icon-btn" aria-label="Close" onClick={() => setSelected(null)}>
                <Icon name="x" size={18} />
              </button>
            </div>
            <p className="hist-detail-body">{selected.text}</p>
            <div className="hist-detail-foot">
              <span className="hist-meta">
                <span className="meta-app">{selected.app ?? "unknown app"}</span>
                <span className="dot-sep">·</span>
                <span className="mono">{selected.word_count} words</span>
                <span className="dot-sep">·</span>
                <span className="mono">{new Date(selected.created_at * 1000).toLocaleString()}</span>
              </span>
              <div className="hist-detail-actions">
                <Button variant="ghost" size="sm" icon="trash" onClick={() => setPendingDelete(selected)}>
                  Delete
                </Button>
                <CopyButton text={selected.text} />
              </div>
            </div>
          </div>
        )}
      </Modal>

      <ConfirmDialog
        open={pendingDelete !== null}
        title="Delete this transcription?"
        body="It will be permanently removed from your history. This can't be undone."
        confirmLabel="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}

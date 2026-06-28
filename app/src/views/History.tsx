import { useState } from "react";
import { CopyButton, EmptyState, Icon, Modal, PageHeader } from "../ui";
import type { HistoryItem } from "../types";

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
            {items.map((item) => (
              <tr key={item.id} className="htable-row" onClick={() => setSelected(item)}>
                <td className="col-text">
                  <span className="cell-text">{item.text}</span>
                </td>
                <td className="col-app muted">{item.app ?? "unknown"}</td>
                <td className="col-words mono">{item.word_count}</td>
                <td className="col-when mono muted">{relativeTime(item.created_at)}</td>
                <td className="col-act">
                  <CopyButton text={item.text} compact />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

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
              <CopyButton text={selected.text} />
            </div>
          </div>
        )}
      </Modal>
    </div>
  );
}

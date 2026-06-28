import { useState } from "react";
import { Badge, Button, ConfirmDialog, Icon, PageHeader } from "../ui";
import type { ModelInfo } from "../types";

export default function Models(props: {
  models: ModelInfo[];
  progress: Record<string, number>;
  activeId: string | undefined;
  onDownload: (id: string) => void;
  onUse: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const [pendingDelete, setPendingDelete] = useState<ModelInfo | null>(null);

  return (
    <div className="view stagger">
      <PageHeader
        title="Models"
        desc="On-device whisper models. Larger models are more accurate but slower."
      />

      <table className="model-table">
        <thead>
          <tr>
            <th>Model</th>
            <th>Size</th>
            <th>Language</th>
            <th className="ta-right">Status</th>
            <th aria-label="Remove" />
          </tr>
        </thead>
        <tbody>
          {props.models.map((m) => {
            const frac = props.progress[m.id];
            const downloading = frac !== undefined;
            const active = props.activeId === m.id;
            return (
              <tr key={m.id} className={active ? "is-active" : ""}>
                <td className="mt-name">
                  <span className="model-name">{m.name}</span>
                  {active && <Badge tone="accent">Active</Badge>}
                </td>
                <td className="mono small">{m.approx_size_mb} MB</td>
                <td className="small muted">{m.multilingual ? "Multilingual" : "English"}</td>
                <td className="ta-right">
                  {downloading ? (
                    <div className="dl-progress">
                      <div className="dl-track">
                        <div className="dl-bar" style={{ width: `${Math.round(frac * 100)}%` }} />
                      </div>
                      <span className="mono small">{Math.round(frac * 100)}%</span>
                    </div>
                  ) : m.downloaded ? (
                    active ? (
                      <span className="active-pill">
                        <Icon name="check" size={15} /> In use
                      </span>
                    ) : (
                      <Button variant="primary" size="sm" onClick={() => props.onUse(m.id)}>
                        Use
                      </Button>
                    )
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      icon="download"
                      onClick={() => props.onDownload(m.id)}
                    >
                      Download
                    </Button>
                  )}
                </td>
                <td className="ta-right">
                  {m.downloaded && !downloading && (
                    <button
                      className="row-del"
                      aria-label={`Delete ${m.name} download`}
                      title="Delete download"
                      onClick={() => setPendingDelete(m)}
                    >
                      <Icon name="trash" size={15} />
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      <ConfirmDialog
        open={pendingDelete !== null}
        title={`Delete ${pendingDelete?.name ?? "this model"}?`}
        body={`This removes the ${pendingDelete?.approx_size_mb ?? ""} MB download from disk. You can re-download it anytime.`}
        confirmLabel="Delete"
        onConfirm={() => {
          if (pendingDelete) props.onDelete(pendingDelete.id);
          setPendingDelete(null);
        }}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}

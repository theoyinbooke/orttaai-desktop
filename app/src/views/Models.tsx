import { Badge, Button, Card, Icon, PageHeader } from "../ui";
import type { ModelInfo } from "../types";

export default function Models(props: {
  models: ModelInfo[];
  progress: Record<string, number>;
  activeId: string | undefined;
  onDownload: (id: string) => void;
  onUse: (id: string) => void;
}) {
  return (
    <div className="view stagger">
      <PageHeader
        title="Models"
        desc="On-device whisper models. Larger models are more accurate but slower."
      />
      <ul className="model-list">
        {props.models.map((m) => {
          const frac = props.progress[m.id];
          const downloading = frac !== undefined;
          const active = props.activeId === m.id;
          return (
            <Card key={m.id} className={`model-row ${active ? "is-active" : ""}`}>
              <div className="model-info">
                <div className="model-name-row">
                  <span className="model-name">{m.name}</span>
                  {active && <Badge tone="accent">Active</Badge>}
                </div>
                <div className="model-sub">
                  <span className="mono">{m.approx_size_mb} MB</span>
                  <span className="dot-sep">·</span>
                  <span>{m.multilingual ? "Multilingual" : "English"}</span>
                  {m.downloaded && !active && (
                    <>
                      <span className="dot-sep">·</span>
                      <span className="ok-text">
                        <Icon name="check" size={13} /> Downloaded
                      </span>
                    </>
                  )}
                </div>
              </div>

              <div className="model-action">
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
                  <Button variant="ghost" size="sm" icon="download" onClick={() => props.onDownload(m.id)}>
                    Download
                  </Button>
                )}
              </div>
            </Card>
          );
        })}
      </ul>
    </div>
  );
}

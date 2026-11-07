// Express engine card (Mac ExpressEngineCard): switch between Whisper and
// Express (NVIDIA Parakeet via ONNX Runtime — the fastest engine on CPU) and
// manage the Express models.
import { useCallback, useEffect, useState } from "react";
import { Badge, Button, ConfirmDialog, Segmented, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import { formatBytes } from "../lib/engine";
import { Row, Section, ToggleRow, usePatch } from "./SettingsRows";

type ExpressModel = {
  id: string;
  name: string;
  description: string;
  approx_size_mb: number;
  multilingual: boolean;
  downloaded: boolean;
  path: string;
};

export default function ExpressCard() {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [models, setModels] = useState<ExpressModel[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [confirm, setConfirm] = useState<ExpressModel | null>(null);

  const refresh = useCallback(() => {
    call<ExpressModel[]>("express_models").then(setModels).catch((e) => toast(String(e), "error"));
  }, [toast]);
  useEffect(refresh, [refresh]);

  const clear = (id: string) =>
    setProgress((p) => {
      const n = { ...p };
      delete n[id];
      return n;
    });
  useEvent<{ id: string; fraction: number }>("express-progress", (e) =>
    setProgress((p) => ({ ...p, [e.id]: e.fraction })),
  );
  useEvent<{ id: string }>("express-done", (e) => {
    clear(e.id);
    refresh();
    toast("Express model ready.", "success");
  });
  useEvent<{ id: string }>("express-cancelled", (e) => clear(e.id));
  useEvent<{ id: string; error: string }>("express-error", (e) => {
    clear(e.id);
    toast(`Couldn't download the Express model: ${e.error}`, "error");
  });

  if (!settings) return null;
  const active = models.find((m) => m.id === settings.express_model);
  const usingExpress = settings.engine === "express";

  const setEngine = (engine: "whisper" | "express") => {
    if (engine === "express" && !active?.downloaded) {
      toast("Download an Express model first.", "warn");
      return;
    }
    save({ engine });
  };

  return (
    <Section
      title="Express Engine"
      desc="NVIDIA Parakeet, run on your CPU through ONNX Runtime — typically several times faster than Whisper without a GPU. Punctuation and casing included."
    >
      <Row label="Speech engine" hint={usingExpress ? "Dictation uses Express." : "Dictation uses Whisper (the models below)."}>
        <Segmented
          value={settings.engine}
          options={[
            { value: "whisper", label: "Whisper" },
            { value: "express", label: "Express" },
          ]}
          onChange={setEngine}
        />
      </Row>
      {models.map((m) => {
        const pct = progress[m.id];
        const inUse = usingExpress && m.id === settings.express_model;
        return (
          <Row
            key={m.id}
            label={
              <span className="model-title">
                <span className="model-name">{m.name}</span>
                {inUse && <Badge tone="accent">In use</Badge>}
                {!m.multilingual && <Badge>English</Badge>}
              </span>
            }
            hint={
              pct !== undefined
                ? `Downloading… ${Math.round(pct * 100)}%`
                : `${formatBytes(m.approx_size_mb * 1024 * 1024)} — ${m.description}`
            }
          >
            {pct !== undefined ? (
              <Button size="sm" variant="ghost" onClick={() => call("express_cancel", { id: m.id })}>
                Cancel
              </Button>
            ) : m.downloaded ? (
              <>
                {m.id !== settings.express_model || !usingExpress ? (
                  <Button size="sm" variant="primary" onClick={() => save({ engine: "express", express_model: m.id })}>
                    Use
                  </Button>
                ) : null}
                <Button size="sm" variant="ghost" icon="trash" aria-label={`Delete ${m.name}`} onClick={() => setConfirm(m)} />
              </>
            ) : (
              <Button
                size="sm"
                icon="download"
                onClick={() => {
                  setProgress((p) => ({ ...p, [m.id]: 0 }));
                  call("express_download", { id: m.id }).catch((e) => {
                    clear(m.id);
                    toast(String(e), "error");
                  });
                }}
              >
                Download
              </Button>
            )}
          </Row>
        );
      })}
      <ToggleRow
        k="express_word_boost"
        label="Word Boost"
        hint="Nudges Express toward your dictionary's names and terms."
        disabled={!usingExpress}
      />
      <ConfirmDialog
        open={!!confirm}
        title={`Delete ${confirm?.name ?? "model"}?`}
        body={
          usingExpress && confirm?.id === settings.express_model
            ? "It's the Express model in use — dictation switches back to Whisper."
            : "You can download it again any time."
        }
        confirmLabel="Delete"
        onCancel={() => setConfirm(null)}
        onConfirm={() => {
          const m = confirm;
          setConfirm(null);
          if (!m) return;
          const wasActive = usingExpress && m.id === settings.express_model;
          call("express_delete", { id: m.id })
            .then(() => {
              refresh();
              if (wasActive) save({ engine: "whisper" });
            })
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </Section>
  );
}

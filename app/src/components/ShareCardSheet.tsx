// Share sheet (Mac MilestoneShareSheet): pick a card for a badge (or the
// numbers so far), see it at size, save the PNG or copy it. Cards show
// numbers only. Analytics/home area.
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { Button, Icon, Select, useToast } from "../ui";
import { call } from "../lib/api";
import {
  CARD_SIZES,
  CardContent,
  TEMPLATE_TITLE,
  cardKind,
  fmt,
  templatesFor,
  type BadgeView,
  type CardSize,
  type CardTemplate,
  type MilestonesOverview,
} from "../lib/analytics";
import { ShareCardCanvas, cardBlob, cardPng } from "./ShareCardRender";

const VISIBLE = 5;
const THUMB_W = 78;

export default function ShareCardSheet({
  overview,
  subjectId,
  onClose,
}: {
  overview: MilestonesOverview;
  /** A badge id, or `"progress"` for the numbers so far. */
  subjectId: string;
  onClose: () => void;
}) {
  const toast = useToast();
  const unlocked = useMemo(() => overview.badges.filter((b) => b.state.kind === "unlocked"), [overview]);
  const [selection, setSelection] = useState(subjectId);
  const badge: BadgeView | null = unlocked.find((b) => b.id === selection) ?? null;
  const templates = templatesFor(cardKind(badge));
  const [template, setTemplate] = useState<CardTemplate>(templates[0]);
  const [size, setSize] = useState<CardSize>("portrait");
  const [stripStart, setStripStart] = useState(0);
  const [busy, setBusy] = useState<"save" | "copy" | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const content = useMemo(() => new CardContent(overview.stats, badge, unlocked), [overview.stats, badge, unlocked]);

  useEffect(() => {
    // Each milestone opens on its own picture.
    setTemplate(templatesFor(cardKind(badge))[0]);
    setStripStart(0);
    setSaved(null);
  }, [selection]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const options = [
    { value: "progress", label: `${fmt(overview.stats.total_words)} words · my stats so far` },
    ...unlocked.map((b) => ({
      value: b.id,
      label: b.unlocked_at
        ? `${b.name} · ${new Date(b.unlocked_at * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}`
        : b.name,
    })),
  ];
  const fileName = `Orttaai-${badge?.id ?? "progress"}-${template}-${size}.png`;
  const maxStart = Math.max(0, templates.length - VISIBLE);
  const start = Math.min(stripStart, maxStart);
  const dims = CARD_SIZES.find((s) => s.value === size) ?? CARD_SIZES[0];
  const previewW = Math.min(470, (600 * dims.w) / dims.h);

  async function save() {
    setBusy("save");
    try {
      const png = await cardPng(template, content, size);
      const path = await call<string>("save_share_card", { fileName, pngBase64: png });
      setSaved(path);
      toast(`Saved ${path.split(/[\\/]/).pop()}`, "success");
    } catch (e) {
      toast(`Couldn't save: ${String(e)}`, "error");
    } finally {
      setBusy(null);
    }
  }

  async function copy() {
    setBusy("copy");
    try {
      const blob = await cardBlob(template, content, size);
      try {
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      } catch {
        await call("copy_share_card", { pngBase64: await cardPng(template, content, size) });
      }
      toast("Copied. Paste it into a post or a message.", "success");
    } catch (e) {
      toast(`Couldn't copy: ${String(e)}`, "error");
    } finally {
      setBusy(null);
    }
  }

  return createPortal(
    <div className="modal-overlay" onMouseDown={onClose}>
      <div
        className="share-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="share-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="share-preview">
          <ShareCardCanvas template={template} content={content} size={size} displayWidth={previewW} className="share-preview-card" />
        </div>
        <div className="share-controls">
          <div className="share-head">
            <div>
              <h2 id="share-title">Share your milestone</h2>
              <p className="muted">Cards show your numbers only, never anything you dictated.</p>
            </div>
            <button className="icon-btn share-close" onClick={onClose} aria-label="Close" title="Close">
              <Icon name="x" size={16} />
            </button>
          </div>

          <div className="share-field">
            <span className="share-label">Milestone</span>
            <Select value={selection} onChange={setSelection} options={options} ariaLabel="Milestone" />
          </div>

          <div className="share-field">
            <span className="share-label">Design · {templates.length} for this milestone</span>
            <div className="share-strip">
              <div className="share-strip-track" style={{ transform: `translateX(${-start * (THUMB_W + 12)}px)` }}>
                {templates.map((t) => (
                  <button
                    key={t}
                    className={`share-thumb ${t === template ? "selected" : ""}`}
                    onClick={() => setTemplate(t)}
                    aria-pressed={t === template}
                    aria-label={`${TEMPLATE_TITLE[t]} template`}
                  >
                    <span className="share-thumb-frame">
                      <ShareCardCanvas template={t} content={content} size="portrait" displayWidth={THUMB_W} />
                    </span>
                    <span className="share-thumb-name">{TEMPLATE_TITLE[t]}</span>
                  </button>
                ))}
              </div>
              {start > 0 && (
                <button className="share-arrow left" onClick={() => setStripStart(Math.max(0, start - 2))} aria-label="Show earlier templates">
                  <Icon name="chevronLeft" size={16} />
                </button>
              )}
              {start < maxStart && (
                <button className="share-arrow right" onClick={() => setStripStart(Math.min(maxStart, start + 2))} aria-label="Show more templates">
                  <Icon name="chevronRight" size={16} />
                </button>
              )}
            </div>
          </div>

          <div className="share-field">
            <span className="share-label">Size</span>
            <div className="chip-row">
              {CARD_SIZES.map((s) => (
                <button key={s.value} className={`chip ${size === s.value ? "selected" : ""}`} onClick={() => setSize(s.value)} aria-pressed={size === s.value}>
                  {s.label}
                </button>
              ))}
            </div>
          </div>

          <div className="share-spacer" />
          {saved && (
            <button className="ghost-link share-saved" onClick={() => call("reveal_share_card", { path: saved }).catch((e) => toast(String(e), "error"))}>
              <Icon name="check" size={14} /> Saved to {saved} · Show in folder
            </button>
          )}
          <div className="share-actions">
            <Button variant="ghost" icon="copy" onClick={copy} loading={busy === "copy"} disabled={busy !== null}>
              Copy
            </Button>
            <Button variant="primary" icon="download" onClick={save} loading={busy === "save"} disabled={busy !== null}>
              Save image
            </Button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

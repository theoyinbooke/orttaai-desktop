// Share sheet (Mac MilestoneShareSheet): pick a card for a badge (or the
// numbers so far), see it at size, then save, copy or share the image. Dark in
// both looks, as on the Mac. Cards show numbers only. Analytics/home area.
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, Select, Spinner } from "../ui";
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
  const unlocked = useMemo(() => overview.badges.filter((b) => b.state.kind === "unlocked"), [overview]);
  const [selection, setSelection] = useState(subjectId);
  const badge: BadgeView | null = unlocked.find((b) => b.id === selection) ?? null;
  const templates = templatesFor(cardKind(badge));
  const [template, setTemplate] = useState<CardTemplate>(templates[0]);
  const [size, setSize] = useState<CardSize>("portrait");
  const [stripStart, setStripStart] = useState(0);
  const [busy, setBusy] = useState<"save" | "copy" | "share" | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const feedbackTimer = useRef(0);
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

  const show = (message: string) => {
    setFeedback(message);
    window.clearTimeout(feedbackTimer.current);
    feedbackTimer.current = window.setTimeout(() => setFeedback(null), 3000);
  };
  useEffect(() => () => window.clearTimeout(feedbackTimer.current), []);

  const saveCard = async () => call<string>("save_share_card", { fileName, pngBase64: await cardPng(template, content, size) });

  async function save() {
    setBusy("save");
    try {
      setSaved(await saveCard());
    } catch (e) {
      show(`Couldn't save: ${String(e)}`);
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
      show("Copied. Paste it into a post or a message.");
    } catch (e) {
      show(`Couldn't copy: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  }

  // Linux and Windows have no share picker to open, so the card is saved and
  // shown in its folder, ready to attach to a post or a message.
  async function share() {
    setBusy("share");
    try {
      const path = await saveCard();
      setSaved(path);
      await call("reveal_share_card", { path });
    } catch (e) {
      show(`Couldn't prepare the card: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  }

  return createPortal(
    <div className="modal-overlay share-overlay" onMouseDown={onClose}>
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
              <p>Cards show your numbers only, never anything you dictated.</p>
            </div>
            <button className="share-close" onClick={onClose} aria-label="Close" title="Close">
              <Icon name="x" size={13} />
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
                <span className="share-fade left">
                  <button className="share-arrow" onClick={() => setStripStart(Math.max(0, start - 2))} aria-label="Show earlier templates">
                    <Icon name="chevronLeft" size={15} />
                  </button>
                </span>
              )}
              {start < maxStart && (
                <span className="share-fade right">
                  <button className="share-arrow" onClick={() => setStripStart(Math.min(maxStart, start + 2))} aria-label="Show more templates">
                    <Icon name="chevronRight" size={15} />
                  </button>
                </span>
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
          {feedback && (
            <p className="share-feedback" role="status">
              {feedback}
            </p>
          )}
          {saved && (
            <button className="ghost-link share-saved" onClick={() => call("reveal_share_card", { path: saved }).catch((e) => show(String(e)))}>
              <Icon name="check" size={14} /> Saved to {saved} · Show in folder
            </button>
          )}
          <div className="share-actions">
            <button className="pill-btn" onClick={save} disabled={busy !== null}>
              {busy === "save" ? <Spinner /> : <Icon name="download" size={15} />} Save image
            </button>
            <button className="pill-btn" onClick={copy} disabled={busy !== null}>
              {busy === "copy" ? <Spinner /> : <Icon name="copy" size={15} />} Copy
            </button>
            <button className="pill-btn primary" onClick={share} disabled={busy !== null}>
              {busy === "share" ? <Spinner /> : <Icon name="share" size={15} />} Share…
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

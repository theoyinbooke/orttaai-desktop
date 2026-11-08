// Express Engine card (Mac ExpressEngineCard) and the settings parts the Model
// page shares with its AI Features section (Mac SettingsComponents): card, row,
// info button, stepper, slider, notice, status chip and small buttons.
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { ConfirmDialog, Select, Spinner, Switch, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings, type Settings, type SettingsPatch } from "../lib/settings";
import { memoryApi } from "../lib/memory";
import { ai } from "../lib/ai";
import { formatBytes } from "../lib/engine";
import { usePatch } from "./SettingsRows";
import "../styles/engine.css";

/* ------------------------------------------------------------- glyphs --- */

const stroke = (d: string) => <path d={d} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />;
const GLYPHS: Record<string, ReactNode> = {
  waveform: stroke("M3.5 10.5v3M7.5 7.5v9M11.5 3.5v17M15.5 7v10M19.5 10v4"),
  brain: stroke(
    "M12 4.8c-1-1.8-4-1.8-5 .2-2-.1-3.5 1.8-2.8 3.7-1.9.9-2.3 3.6-.6 4.9-.7 2.1.9 4 2.9 3.7.5 2.1 3.6 3 5.5 1.5V4.8Zm0 0c1-1.8 4-1.8 5 .2 2-.1 3.5 1.8 2.8 3.7 1.9.9 2.3 3.6.6 4.9.7 2.1-.9 4-2.9 3.7-.5 2.1-3.6 3-5.5 1.5M7.6 9.6c1.4 0 2.4.9 2.4 2.2M16.4 9.6c-1.4 0-2.4.9-2.4 2.2",
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9.5" fill="none" stroke="currentColor" strokeWidth="2" />
      <path d="M12 11v6" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
      <circle cx="12" cy="7.5" r="1.4" fill="currentColor" />
    </>
  ),
  circle: <circle cx="12" cy="12" r="9.5" fill="none" stroke="currentColor" strokeWidth="2" />,
  checkCircleFill: (
    <>
      <circle cx="12" cy="12" r="11" fill="currentColor" />
      <path d="M7 12.5l3.4 3.4L17 8.8" fill="none" stroke="var(--surface)" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  checkCircle: (
    <>
      <circle cx="12" cy="12" r="9.5" fill="none" stroke="currentColor" strokeWidth="2" />
      <path d="M7.5 12.5l3 3 6-6.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  alertCircleFill: (
    <>
      <circle cx="12" cy="12" r="11" fill="currentColor" />
      <path d="M12 6.5v7" stroke="var(--surface)" strokeWidth="2.6" strokeLinecap="round" />
      <circle cx="12" cy="17.5" r="1.6" fill="var(--surface)" />
    </>
  ),
  alertTriangleFill: (
    <>
      <path d="M10.3 3.2 1.6 18.5A2 2 0 0 0 3.3 21.5h17.4a2 2 0 0 0 1.7-3L13.7 3.2a2 2 0 0 0-3.4 0Z" fill="currentColor" />
      <path d="M12 8.5v6" stroke="var(--surface)" strokeWidth="2.4" strokeLinecap="round" />
      <circle cx="12" cy="18" r="1.4" fill="var(--surface)" />
    </>
  ),
  x: stroke("M6 6l12 12M18 6 6 18"),
  minus: stroke("M5 12h14"),
  plus: stroke("M12 5v14M5 12h14"),
  chevronUp: stroke("M6 15l6-6 6 6"),
  chevronDown: stroke("M6 9l6 6 6-6"),
  refresh: stroke("M20.5 12a8.5 8.5 0 1 1-2.5-6M20.5 3v5h-5"),
  trash: stroke("M4 6.5h16M9.5 6.5V4h5v2.5M6.5 6.5l1 14h9l1-14M10 10.5v6M14 10.5v6"),
  bolt: <path d="M13.5 2 4 14h7l-1 8 10-12.5h-7.5l.5-7.5Z" fill="currentColor" />,
  downloadCircle: (
    <>
      <circle cx="12" cy="12" r="9.5" fill="none" stroke="currentColor" strokeWidth="2" />
      <path d="M12 7v9m0 0 3.6-3.6M12 16l-3.6-3.6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  downRightCircle: (
    <>
      <circle cx="12" cy="12" r="9.5" fill="none" stroke="currentColor" strokeWidth="2" />
      <path d="M8.5 8.5l7 7m0-5.5v5.5H10" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  drive: (
    <>
      <path d="M3 14 5.5 6.5A2 2 0 0 1 7.4 5h9.2a2 2 0 0 1 1.9 1.5L21 14v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-3Zm0 0h18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
      <circle cx="17" cy="16.5" r="1.2" fill="currentColor" />
    </>
  ),
  folder: stroke("M3 7a2 2 0 0 1 2-2h4.5l2 2.5H19a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"),
  folderPlus: stroke("M3 7a2 2 0 0 1 2-2h4.5l2 2.5H19a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7ZM12 11v5.5M9.2 13.7h5.6"),
  sync: stroke("M4 11a8 8 0 0 1 13.7-4.6L20 8.5M20 4v4.5h-4.5M20 13a8 8 0 0 1-13.7 4.6L4 15.5M4 20v-4.5h4.5"),
};

export function Glyph({ name, size = 12 }: { name: keyof typeof GLYPHS | string; size?: number }) {
  return (
    <svg className="mset-glyph" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      {GLYPHS[name]}
    </svg>
  );
}

/* --------------------------------------------------------------- info --- */

/** Small info icon beside a label. Hover shows a tooltip; a click opens a popover. */
export function InfoButton({ title, text }: { title: string; text: string }) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const btn = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const r = btn.current?.getBoundingClientRect();
    const h = pop.current?.offsetHeight ?? 80;
    if (!r) return;
    const width = 284;
    const left = Math.min(window.innerWidth - width - 8, Math.max(8, r.left + r.width / 2 - width / 2));
    const below = r.bottom + 6;
    setPos({ left, top: below + h > window.innerHeight - 8 ? Math.max(8, r.top - h - 6) : below });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!btn.current?.contains(t) && !pop.current?.contains(t)) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", close, true);
    };
  }, [open]);

  return (
    <>
      <button
        ref={btn}
        type="button"
        className={`mset-info ${open ? "open" : ""}`}
        title={open ? undefined : text}
        aria-label={`About ${title}`}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <Glyph name="info" size={11} />
      </button>
      {open &&
        createPortal(
          <div ref={pop} className="mset-pop" role="tooltip" style={pos ?? { top: 0, left: 0, visibility: "hidden" }}>
            {text}
          </div>,
          document.body,
        )}
    </>
  );
}

/* ------------------------------------------------------- cards and rows -- */

export function MLabel({ title, info, heading }: { title: string; info?: string; heading?: boolean }) {
  return (
    <span className={`mset-label ${heading ? "is-heading" : ""}`}>
      {heading ? <h2>{title}</h2> : <span>{title}</span>}
      {info && <InfoButton title={title} text={info} />}
    </span>
  );
}

/** A titled group of rows; the accessory sits at the trailing edge of the title line. */
export function MCard({
  title,
  info,
  accessory,
  children,
}: {
  title: string;
  info?: string;
  accessory?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="mset-card">
      <header className="mset-card-head">
        <MLabel title={title} info={info} heading />
        <span className="mset-spacer" />
        {accessory}
      </header>
      {children}
    </section>
  );
}

export function MRow({
  title,
  info,
  children,
  disabled,
}: {
  title: string;
  info?: string;
  children?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`mset-row ${disabled ? "is-disabled" : ""}`}>
      <MLabel title={title} info={info} />
      <span className="mset-spacer" />
      {children}
    </div>
  );
}

export const MDivider = () => <div className="mset-divider" />;
export const MFootnote = ({ children }: { children: ReactNode }) => <p className="mset-foot">{children}</p>;

type BoolKey = { [K in keyof Settings]: Settings[K] extends boolean ? K : never }[keyof Settings];

/** A switch bound to a boolean setting. */
export function MSwitch({ k, title, disabled }: { k: BoolKey; title: string; disabled?: boolean }) {
  const { settings } = useSettings();
  const save = usePatch();
  if (!settings) return null;
  return (
    <Switch
      checked={settings[k] as boolean}
      label={title}
      disabled={disabled}
      onChange={(v: boolean) => save({ [k]: v } as SettingsPatch)}
    />
  );
}

export function MToggleRow({ k, title, info, disabled }: { k: BoolKey; title: string; info?: string; disabled?: boolean }) {
  return (
    <MRow title={title} info={info} disabled={disabled}>
      <MSwitch k={k} title={title} disabled={disabled} />
    </MRow>
  );
}

/** A slider with its current value, on one compact line. Saves when released. */
export function MSliderRow({
  title,
  info,
  value,
  min,
  max,
  step,
  format,
  onCommit,
  disabled,
}: {
  title: string;
  info?: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
  onCommit: (v: number) => void;
  disabled?: boolean;
}) {
  const clamped = Math.min(max, Math.max(min, value));
  const [draft, setDraft] = useState(clamped);
  useEffect(() => setDraft(clamped), [clamped]);
  const commit = () => draft !== clamped && onCommit(draft);
  return (
    <MRow title={title} info={info} disabled={disabled}>
      <span className="mset-slider">
        <input
          type="range"
          className="mset-range"
          min={min}
          max={max}
          step={step}
          value={draft}
          disabled={disabled}
          aria-label={title}
          aria-valuetext={format(draft)}
          style={{ "--pct": `${((draft - min) / (max - min)) * 100}%` } as React.CSSProperties}
          onChange={(e) => setDraft(Number(e.currentTarget.value))}
          onPointerUp={commit}
          onKeyUp={commit}
        />
        <output className="mset-slider-value">{format(draft)}</output>
      </span>
    </MRow>
  );
}

/** Compact minus / value / plus control. */
export function MStepperRow({
  title,
  info,
  value,
  min,
  max,
  step = 1,
  format = (v) => String(v),
  onChange,
  disabled,
}: {
  title: string;
  info?: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <MRow title={title} info={info} disabled={disabled}>
      <span className="mset-stepper" role="group" aria-label={title}>
        <button type="button" disabled={disabled || value <= min} aria-label={`Decrease ${title}`} onClick={() => onChange(Math.max(min, value - step))}>
          <Glyph name="minus" size={10} />
        </button>
        <output>{format(value)}</output>
        <button type="button" disabled={disabled || value >= max} aria-label={`Increase ${title}`} onClick={() => onChange(Math.min(max, value + step))}>
          <Glyph name="plus" size={10} />
        </button>
      </span>
    </MRow>
  );
}

/** Inline status message shown next to the action that produced it. */
export function MNotice({
  kind,
  children,
  onDismiss,
}: {
  kind: "error" | "warning" | "success" | "info";
  children: ReactNode;
  onDismiss?: () => void;
}) {
  const icon = { error: "alertTriangleFill", warning: "alertCircleFill", success: "checkCircleFill", info: "info" }[kind];
  return (
    <div className={`mset-notice ${kind}`} role={kind === "error" ? "alert" : "status"}>
      <Glyph name={icon} size={12} />
      <span className="mset-notice-text">{children}</span>
      {onDismiss && (
        <button type="button" onClick={onDismiss} aria-label="Dismiss">
          <Glyph name="x" size={9} />
        </button>
      )}
    </div>
  );
}

export function StatusChip({ title, glyph, tone }: { title: string; glyph: string; tone: "ok" | "warn" | "danger" | "accent" | "plain" }) {
  return (
    <span className={`mset-chip ${tone}`}>
      <Glyph name={glyph} size={11} />
      {title}
    </span>
  );
}

/** OrttaaiButtonStyle at its small size: secondary, primary or destructive. */
export function MButton({
  kind = "secondary",
  glyph,
  loading,
  children,
  className = "",
  ...rest
}: {
  kind?: "secondary" | "primary" | "danger";
  glyph?: string;
  loading?: boolean;
  children?: ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" className={`mset-btn ${kind} ${className}`} {...rest} disabled={loading || rest.disabled}>
      {loading ? <Spinner size={12} /> : glyph ? <Glyph name={glyph} size={13} /> : null}
      {children}
    </button>
  );
}

/** Determinate bar under a caption, for downloads. */
export function MProgress({ label, fraction, trailing }: { label: ReactNode; fraction: number | null; trailing?: ReactNode }) {
  return (
    <div className="mset-progress">
      <div className="mset-progress-head">
        <span>{label}</span>
        {trailing}
      </div>
      <div className={`mset-progress-track ${fraction == null ? "indeterminate" : ""}`}>
        <span style={fraction == null ? undefined : { width: `${Math.round(fraction * 100)}%` }} />
      </div>
    </div>
  );
}

/* ------------------------------------------------------------ the card --- */

type ExpressModel = {
  id: string;
  name: string;
  description: string;
  approx_size_mb: number;
  multilingual: boolean;
  downloaded: boolean;
  path: string;
};

/** The whole Express chain in the order it runs: engine, Word Boost, then Polish. */
export default function ExpressCard({ onShowPolishSettings }: { onShowPolishSettings?: () => void }) {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [models, setModels] = useState<ExpressModel[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [failure, setFailure] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ExpressModel | null>(null);
  const [boosted, setBoosted] = useState<number | null>(null);
  const [boostModel, setBoostModel] = useState<{ installed: boolean; size_mb: number; downloading: boolean } | null>(null);
  const [boostProgress, setBoostProgress] = useState<number | null>(null);
  const refreshBoostModel = useCallback(() => {
    call<{ installed: boolean; size_mb: number; downloading: boolean }>("word_boost_status")
      .then(setBoostModel)
      .catch(() => setBoostModel(null));
  }, []);
  useEffect(refreshBoostModel, [refreshBoostModel]);
  useEvent<{ fraction: number }>("word-boost-progress", (e) => setBoostProgress(e.fraction));
  useEvent("word-boost-done", () => {
    setBoostProgress(null);
    refreshBoostModel();
  });
  useEvent<{ error: string }>("word-boost-error", (e) => {
    setBoostProgress(null);
    toast(`Couldn't download Word Boost: ${e.error}`, "error");
  });
  useEvent("express-done", refreshBoostModel);
  // Turning Express on before its model is here: download first, then switch.
  const enableAfterDownload = useRef<string | null>(null);

  const refresh = useCallback(() => {
    call<ExpressModel[]>("express_models").then(setModels).catch((e) => toast(String(e), "error"));
  }, [toast]);
  useEffect(refresh, [refresh]);

  const usingExpress = settings?.engine === "express";
  useEffect(() => {
    if (!usingExpress) return;
    memoryApi
      .listDictionary()
      .then((d) => setBoosted(d.filter((e) => e.is_active).length))
      .catch(() => setBoosted(null));
  }, [usingExpress]);

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
    if (enableAfterDownload.current === e.id) {
      enableAfterDownload.current = null;
      save({ engine: "express", express_model: e.id });
    }
  });
  useEvent<{ id: string }>("express-cancelled", (e) => {
    clear(e.id);
    enableAfterDownload.current = null;
  });
  useEvent<{ id: string; error: string }>("express-error", (e) => {
    clear(e.id);
    enableAfterDownload.current = null;
    setFailure(e.error);
  });

  // Polish status names the engine, so keep the provider's name current.
  const [providerName, setProviderName] = useState("your local AI server");
  useEffect(() => {
    ai.status()
      .then((s) => setProviderName(s.provider_name))
      .catch(() => {});
  }, [settings?.llm_provider]);

  if (!settings) return null;
  const selected = models.find((m) => m.id === settings.express_model) ?? models[0];
  const downloadingId = Object.keys(progress)[0];
  const pct = downloadingId ? progress[downloadingId] : undefined;
  const downloading = pct !== undefined;

  const download = (id: string) => {
    setFailure(null);
    setProgress((p) => ({ ...p, [id]: 0 }));
    call("express_download", { id }).catch((e) => {
      clear(id);
      enableAfterDownload.current = null;
      setFailure(String(e));
    });
  };

  const setEnabled = (on: boolean) => {
    if (!on) {
      enableAfterDownload.current = null;
      if (downloadingId) call("express_cancel", { id: downloadingId });
      save({ engine: "whisper" });
      return;
    }
    if (!selected) return;
    if (selected.downloaded) {
      save({ engine: "express", express_model: selected.id });
    } else if (!downloading) {
      enableAfterDownload.current = selected.id;
      download(selected.id);
    }
  };

  const chooseModel = (id: string) => {
    const m = models.find((x) => x.id === id);
    if (!m) return;
    if (usingExpress && !m.downloaded) {
      enableAfterDownload.current = id;
      if (progress[id] === undefined) download(id);
      return;
    }
    save({ express_model: id });
  };

  const needsBoostModel = !!boostModel && !boostModel.installed;
  const wordBoostStatus =
    boostProgress !== null
      ? `Downloading the Word Boost model… ${Math.round(boostProgress * 100)}%`
      : usingExpress && settings.express_word_boost && needsBoostModel
        ? `Needs its model, a one-time ${boostModel?.size_mb ?? 132} MB download.`
        : !usingExpress || !settings.express_word_boost || boosted === null
      ? null
      : boosted === 0
        ? "Add words in Memory > Dictionary and Word Boost will listen for them."
        : boosted === 1
          ? "Boosting 1 dictionary word."
          : `Boosting ${boosted} dictionary words.`;

  // Word Boost rescoring needs the whole recording, so live segments are skipped
  // while it has dictionary words to boost (unknown count: assume it does).
  const liveSegmentsBlocked = usingExpress && settings.express_word_boost && boosted !== 0;

  const polishModel =
    settings.polish_engine === "s1_mini" ? `S1-mini on ${providerName}` : `${settings.polish_model} on ${providerName}`;
  const polishStatus = settings.polish_enabled ? `Polishes with ${polishModel}.` : `Off. When on, polishes with ${polishModel}.`;

  const chip = downloading ? (
    <StatusChip title="Working" glyph="downloadCircle" tone="accent" />
  ) : failure ? (
    <StatusChip title="Unavailable" glyph="alertTriangleFill" tone="danger" />
  ) : usingExpress ? (
    <StatusChip title="Active" glyph="bolt" tone="ok" />
  ) : selected?.downloaded ? (
    <StatusChip title="Downloaded" glyph="checkCircle" tone="plain" />
  ) : (
    <StatusChip title="Not downloaded" glyph="downloadCircle" tone="plain" />
  );

  const size = selected ? formatBytes(selected.approx_size_mb * 1024 * 1024) : "";
  return (
    <MCard
      title="Express Engine"
      info="NVIDIA Parakeet, a compact speech model that runs on your CPU through ONNX Runtime. It's typically several times faster than Whisper without a GPU. Your Whisper model stays selected for when Express is off."
      accessory={chip}
    >
      <DetailRow
        title="Use Express for dictation"
        detail={
          selected
            ? `${selected.multilingual ? "25 European languages" : "English only"}. Downloads ${size} once.`
            : "Checking the Express models…"
        }
      >
        <Switch
          checked={usingExpress || enableAfterDownload.current !== null}
          label="Use Express for dictation"
          onChange={setEnabled}
        />
      </DetailRow>
      <MDivider />
      {models.length > 1 && selected && (
        <>
          <MRow title="Express Model" info={selected.description}>
            <Select
              value={selected.id}
              onChange={chooseModel}
              options={models.map((m) => ({ value: m.id, label: m.name.replace(/^Express · /, "") }))}
              ariaLabel="Express model"
              className="mset-dd"
            />
          </MRow>
          <MDivider />
        </>
      )}
      <DetailRow
        title="Word Boost"
        info="Checks the words in your Memory dictionary against what Express heard, so names and terms come out spelled your way."
        detail={wordBoostStatus}
        action={
          usingExpress && settings.express_word_boost && needsBoostModel && boostProgress === null
            ? {
                label: "Download",
                run: () => {
                  setBoostProgress(0);
                  call("word_boost_download").catch((e) => {
                    setBoostProgress(null);
                    toast(String(e), "error");
                  });
                },
              }
            : undefined
        }
      >
        <MSwitch k="express_word_boost" title="Word Boost" disabled={!usingExpress} />
      </DetailRow>
      <MDivider />
      <DetailRow
        title="Decode while I speak"
        info="Each stretch of speech is decoded at your pauses while you're still talking, so a long dictation finishes about as fast as a short one. Off by default while it's being tested."
        detail={liveSegmentsBlocked ? "Off while Word Boost is on: Word Boost needs the whole recording." : "Long dictations finish sooner."}
      >
        <MSwitch k="express_live_segments" title="Decode while I speak" disabled={!usingExpress || liveSegmentsBlocked} />
      </DetailRow>
      <MDivider />
      <DetailRow
        title="Polish"
        info="Cleans up punctuation, grammar and filler words with a local language model before the text is inserted. This is the same switch as Text Polish in AI Features, so it applies to Whisper dictations too."
        detail={polishStatus}
        action={onShowPolishSettings && { label: "Choose Model…", run: onShowPolishSettings }}
      >
        <MSwitch k="polish_enabled" title="Polish" />
      </DetailRow>
      <div className="mset-stack">
        {downloading && (
          <MProgress
            label={`Downloading Express… ${Math.round((pct ?? 0) * 100)}%`}
            fraction={pct ?? 0}
            trailing={
              <button type="button" className="mset-link" onClick={() => call("express_cancel", { id: downloadingId })}>
                Cancel
              </button>
            }
          />
        )}
        {usingExpress && selected && !selected.multilingual && settings.dictation_language !== "en" && (
          <MNotice kind="info">This Express model is English only, so dictation uses Whisper until the dictation language is English.</MNotice>
        )}
        {failure && (
          <MNotice kind="error" onDismiss={() => setFailure(null)}>
            Express couldn't start: {failure}
          </MNotice>
        )}
        {!usingExpress && selected?.downloaded && !downloading && (
          <div>
            <MButton glyph="trash" onClick={() => setConfirm(selected)}>
              Remove Express Model
            </MButton>
          </div>
        )}
      </div>
      <p className="mset-foot mset-foot-top">NVIDIA Parakeet TDT 0.6B (CC BY 4.0), run through ONNX Runtime with sherpa-onnx.</p>
      <ConfirmDialog
        open={!!confirm}
        title={`Delete ${confirm?.name ?? "model"}?`}
        body="You can download it again any time."
        confirmLabel="Delete"
        onCancel={() => setConfirm(null)}
        onConfirm={() => {
          const m = confirm;
          setConfirm(null);
          if (!m) return;
          call("express_delete", { id: m.id })
            .then(refresh)
            .catch((e) => toast(String(e), "error"));
        }}
      />
    </MCard>
  );
}

/** A setting with its explanation directly under the title. */
function DetailRow({
  title,
  info,
  detail,
  action,
  children,
}: {
  title: string;
  info?: string;
  detail?: string | null;
  action?: { label: string; run: () => void };
  children: ReactNode;
}) {
  return (
    <div className="mset-detail-row">
      <div className="mset-detail-text">
        <MLabel title={title} info={info} />
        {detail && (
          <span className="mset-detail">
            {detail}
            {action && (
              <button type="button" className="mset-link" onClick={action.run}>
                {action.label}
              </button>
            )}
          </span>
        )}
      </div>
      <span className="mset-spacer" />
      {children}
    </div>
  );
}

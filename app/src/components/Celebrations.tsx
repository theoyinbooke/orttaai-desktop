// Milestone toasts and celebrations (Mac MilestoneToastController +
// MilestoneCelebrationHost). A dictation that earns a badge raises a corner
// toast while Orttaai isn't in front; the next time the window is, every badge
// earned since is celebrated one at a time. Both respect Settings › Celebrate
// milestones and wait for What's New to close. Owned by the analytics/home area.
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { emit } from "@tauri-apps/api/event";
import "../styles/analytics.css";
import { Icon, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import { useWhatsNewOpen, type BadgeView, type MilestonesOverview } from "../lib/analytics";
import { MilestoneBadge } from "./ShareCardBadge";
import ShareCardSheet from "./ShareCardSheet";

const TOAST_SECONDS = 6;

function useWindowActive() {
  const [active, setActive] = useState(() => document.hasFocus() && document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setActive(document.hasFocus() && document.visibilityState === "visible");
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return active;
}

export default function Celebrations() {
  const { settings } = useSettings();
  const celebrate = settings?.celebrate_milestones ?? true;
  const whatsNew = useWhatsNewOpen();
  const active = useWindowActive();
  const toastErr = useToast();
  const [queue, setQueue] = useState<BadgeView[]>([]);
  const [runTotal, setRunTotal] = useState(0);
  const [toast, setToast] = useState<BadgeView[] | null>(null);
  const [share, setShare] = useState<{ overview: MilestonesOverview; id: string } | null>(null);

  const refresh = useCallback(() => {
    call<BadgeView[]>("milestones_pending")
      .then(setQueue)
      .catch(() => {});
  }, []);
  useEffect(refresh, [refresh]);
  useEvent("milestones-changed", refresh);
  // A dictionary word can earn Wordsmith.
  useEvent("memory-changed", () => {
    call("milestones_evaluate").catch(() => {});
  });
  useEvent<BadgeView[]>("milestone-unlocked", (fresh) => {
    refresh();
    if (!document.hasFocus() && fresh.length > 0) setToast(fresh);
  });

  useEffect(() => {
    if (queue.length === 0) setRunTotal(0);
    else if (queue.length > runTotal) setRunTotal(queue.length);
  }, [queue.length]); // eslint-disable-line react-hooks/exhaustive-deps

  const mark = (badges: BadgeView[]) => {
    const ids = badges.map((b) => b.id);
    setQueue((q) => q.filter((b) => !ids.includes(b.id)));
    call("milestones_mark_celebrated", { ids }).catch((e) => toastErr(String(e), "error"));
  };

  const openShare = async (id: string) => {
    try {
      const overview = await call<MilestonesOverview>("milestones_overview");
      setShare({ overview, id });
    } catch (e) {
      toastErr(String(e), "error");
    }
  };

  const dismissToast = useCallback(() => setToast(null), []);
  const current = queue[0];
  const showCelebration = celebrate && active && !whatsNew && !share && !!current;

  return (
    <>
      {showCelebration &&
        createPortal(
          <div className="celebration-overlay" role="presentation">
            <CelebrationCard
              key={current.id}
              badge={current}
              position={Math.max(1, runTotal - queue.length + 1)}
              total={Math.max(runTotal, queue.length)}
              onLater={() => mark(queue)}
              onNext={() => mark([current])}
              onShare={() => {
                mark([current]);
                openShare(current.id);
              }}
              onSeeAll={() => {
                mark(queue);
                emit("navigate", "milestones").catch(() => {});
              }}
            />
          </div>,
          document.body,
        )}
      {toast && celebrate && (
        <MilestoneToast
          badges={toast}
          onDismiss={dismissToast}
          onShare={() => {
            const b = toast[0];
            setToast(null);
            mark([b]);
            openShare(b.id);
          }}
        />
      )}
      {share && <ShareCardSheet overview={share.overview} subjectId={share.id} onClose={() => setShare(null)} />}
    </>
  );
}

function CelebrationCard({
  badge,
  position,
  total,
  onLater,
  onShare,
  onNext,
  onSeeAll,
}: {
  badge: BadgeView;
  position: number;
  total: number;
  onLater: () => void;
  onShare: () => void;
  onNext: () => void;
  onSeeAll: () => void;
}) {
  const cardRef = useRef<HTMLDivElement>(null);
  const keys = useRef({ onLater, onShare });
  keys.current = { onLater, onShare };
  useEffect(() => {
    cardRef.current?.focus();
    // Escape and Return act as the Mac's cancel and default actions, unless a
    // button was tabbed to, which then handles Return itself.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") keys.current.onLater();
      else if (e.key === "Enter" && !(document.activeElement instanceof HTMLButtonElement)) keys.current.onShare();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className="celebration-card" ref={cardRef} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="celebration-title">
      <Burst badge={badge} />
      <span className="celebration-eyebrow">MILESTONE UNLOCKED</span>
      <h2 id="celebration-title">{badge.name}</h2>
      <p className="celebration-line">{badge.celebration_line}</p>
      <div className="celebration-actions">
        <button className="pill-btn" onClick={onLater}>
          Not now
        </button>
        <button className="pill-btn primary" onClick={onShare}>
          <Icon name="share" size={15} /> Share card
        </button>
      </div>
      <div className="celebration-foot">
        <button className="ghost-link celebration-all" onClick={onSeeAll}>
          See all milestones
        </button>
        <span className="celebration-spacer" />
        {total > 1 && (
          <>
            <span className="celebration-count">
              {position} of {total} new
            </span>
            <span className="celebration-dots" aria-hidden="true">
              {Array.from({ length: Math.min(total, 8) }, (_, i) => (
                <span key={i} className={i === position - 1 ? "on" : ""} />
              ))}
            </span>
            <button className="celebration-next" onClick={onNext} aria-label="Next milestone">
              <Icon name="chevronRight" size={14} />
            </button>
          </>
        )}
      </div>
    </div>
  );
}

const PIECES: [number, number, number, number, string, number][] = [
  [-110, -70, 8, 18, "#D4952A", 0.25], [115, -60, 6, 12, "#F5F3F0", 0.35],
  [-95, 90, 8, 14, "#E2B04A", 0.45], [100, 95, 6, 16, "#D4952A", 0.3],
  [10, -125, 6, 10, "#F5F3F0", 0.55], [-130, 10, 8, 12, "#E2B04A", 0.65],
  [130, 15, 6, 14, "#D4952A", 0.5],
];

/** The badge popping in with turning rays, a pulse ring, the brand's rising
 *  bars and pieces thrown outward. Still under reduced motion. */
function Burst({ badge }: { badge: BadgeView }) {
  return (
    <div className="burst" aria-hidden="true">
      <div className="burst-rays">
        {Array.from({ length: 16 }, (_, i) => (
          <span key={i} className={i % 2 === 0 ? "long" : ""} style={{ transform: `rotate(${i * 22.5}deg) translateY(-118px)` }} />
        ))}
      </div>
      <span className="burst-pulse" />
      {PIECES.map(([dx, dy, w, h, color, delay], i) => (
        <span
          key={i}
          className="burst-piece"
          style={
            {
              width: w,
              height: h,
              background: color,
              animationDelay: `${delay}s`,
              "--dx": `${dx}px`,
              "--dy": `${dy}px`,
              "--rot": `${dx}deg`,
            } as CSSProperties
          }
        />
      ))}
      <span className="burst-badge">
        <MilestoneBadge badge={badge} size={156} forceUnlocked />
      </span>
      <span className="burst-bars">
        {[20, 30, 40, 42].map((h, i) => (
          <span key={i} style={{ height: h, width: i === 3 ? 4 : 10, animationDelay: `${0.35 + i * 0.1}s` }} />
        ))}
      </span>
    </div>
  );
}

function MilestoneToast({ badges, onShare, onDismiss }: { badges: BadgeView[]; onShare: () => void; onDismiss: () => void }) {
  const [hover, setHover] = useState(false);
  const timer = useRef<number>(0);
  useEffect(() => {
    if (hover) return;
    timer.current = window.setTimeout(onDismiss, TOAST_SECONDS * 1000);
    return () => window.clearTimeout(timer.current);
  }, [hover, onDismiss, badges]);
  const b = badges[0];
  return createPortal(
    <div
      className={`ms-toast ${hover ? "paused" : ""}`}
      role="status"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <div className="ms-toast-body">
        <span className="ms-toast-badge">
          <MilestoneBadge badge={b} size={48} forceUnlocked />
        </span>
        <div className="ms-toast-text">
          <span className="ms-toast-eyebrow">{badges.length > 1 ? `${badges.length} MILESTONES UNLOCKED` : "MILESTONE UNLOCKED"}</span>
          <span className="ms-toast-name">{b.name}</span>
          <span className="ms-toast-line">{b.celebration_line}</span>
        </div>
        <button className="pill-btn primary small" onClick={onShare}>
          Share
        </button>
        <button className="ms-toast-x" onClick={onDismiss} aria-label="Dismiss">
          <Icon name="x" size={13} />
        </button>
      </div>
      <span className="ms-toast-timer" key={hover ? "h" : "r"} style={{ animationDuration: `${TOAST_SECONDS}s` }} />
    </div>,
    document.body,
  );
}

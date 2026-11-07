// What's New (Mac WhatsNewCard): after an update, the first time the window
// opens, up to three highlights one at a time. Next moves on; Skip or the
// close button puts them away. Seen only once closed, so quitting first keeps
// it for next time. Owned by the analytics/home area.
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import "../styles/analytics.css";
import { Icon } from "../ui";
import { call } from "../lib/api";
import { whatsNewOpen, type WhatsNewArt, type WhatsNewRelease } from "../lib/analytics";
import { MilestoneBadge } from "./ShareCardBadge";

export default function WhatsNew() {
  const [release, setRelease] = useState<WhatsNewRelease | null>(null);
  const [index, setIndex] = useState(0);
  const nextRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    call<WhatsNewRelease | null>("whats_new_pending")
      .then((r) => {
        if (r && r.items.length > 0) {
          setRelease(r);
          whatsNewOpen.set(true);
        }
      })
      .catch(() => {});
  }, []);

  const close = () => {
    if (!release) return;
    call("whats_new_mark_seen", { version: release.version }).catch(() => {});
    setRelease(null);
    whatsNewOpen.set(false);
  };
  const closeRef = useRef(close);
  closeRef.current = close;

  useEffect(() => {
    if (!release) return;
    nextRef.current?.focus();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && closeRef.current();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [release]);

  if (!release) return null;
  const items = release.items.slice(0, 3);
  const item = items[Math.min(index, items.length - 1)];
  const isLast = index >= items.length - 1;

  return createPortal(
    <div className="celebration-overlay wn-overlay" role="presentation">
      <div className="wn-card" role="dialog" aria-modal="true" aria-label={`What's new in Orttaai ${release.version}`}>
        <div className="wn-art">
          <span className="wn-glow" />
          <span className="wn-dots" />
          <div className="wn-scene" key={item.id}>
            <Scene art={item.art} />
          </div>
          <button className="wn-close" onClick={close} aria-label="Close What's New" title="Close">
            <Icon name="x" size={13} />
          </button>
        </div>
        <div className="wn-body">
          <span className="wn-eyebrow">WHAT'S NEW IN {release.version}</span>
          <div className="wn-text" key={item.id}>
            <h2>{item.title}</h2>
            <p>{item.detail}</p>
          </div>
          <div className="wn-foot">
            {items.length > 1 && (
              <span className="celebration-dots" aria-label={`${index + 1} of ${items.length}`}>
                {items.map((it, i) => (
                  <span key={it.id} className={i === index ? "on" : ""} />
                ))}
              </span>
            )}
            <span className="celebration-spacer" />
            {!isLast && (
              <button className="ghost-link wn-skip" onClick={close} title="Close What's New">
                Skip
              </button>
            )}
            <button ref={nextRef} className="pill-btn primary small" onClick={() => (isLast ? close() : setIndex((i) => i + 1))}>
              {isLast ? "Got it" : "Next"}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function Scene({ art }: { art: WhatsNewArt }) {
  switch (art.kind) {
    case "milestones":
      return (
        <div className="wn-milestones">
          {Array.from({ length: 16 }, (_, i) => (
            <span key={i} className={`wn-ray ${i % 2 === 0 ? "long" : ""}`} style={{ transform: `rotate(${i * 22.5}deg) translateY(-76px)` }} />
          ))}
          {(
            [
              [-120, -50, 6, 14, 30, "#D4952A"], [118, -58, 5, 11, -25, "#F5F3F0"],
              [-104, 52, 6, 12, -40, "#E2B04A"], [112, 48, 5, 13, 55, "#D4952A"],
              [-150, 2, 4, 9, 70, "#F5F3F0"], [150, -6, 6, 12, -60, "#E2B04A"],
            ] as [number, number, number, number, number, string][]
          ).map(([x, y, w, h, a, c], i) => (
            <span key={i} className="wn-confetti" style={{ width: w, height: h, background: c, transform: `translate(${x}px, ${y}px) rotate(${a}deg)` }} />
          ))}
          <MilestoneBadge
            badge={{ tier: "silver", symbol: "text.alignleft", name: "5,000 Words", fraction: 1, state: { kind: "unlocked", at: 0 } }}
            size={108}
          />
        </div>
      );
    case "learning":
      return (
        <div className="wn-learning">
          <div className="wn-words">
            <span className="wn-chip struck">Orta</span>
            <Icon name="arrowRight" size={14} />
            <span className="wn-chip">Orttaai</span>
          </div>
          <span className="wn-learned">
            <Icon name="check" size={13} /> Learned a word <b>Review</b>
          </span>
        </div>
      );
    case "updates":
      return (
        <div className="wn-updates">
          <svg width="92" height="92" viewBox="0 0 92 92" aria-hidden="true">
            <circle cx="46" cy="46" r="40" fill="none" stroke="var(--line)" strokeWidth="7" />
            <circle
              cx="46"
              cy="46"
              r="40"
              fill="none"
              stroke="var(--accent)"
              strokeWidth="7"
              strokeLinecap="round"
              strokeDasharray={`${2 * Math.PI * 40 * 0.72} 999`}
              transform="rotate(-90 46 46)"
            />
          </svg>
          <span className="wn-updates-icon">
            <Icon name="download" size={26} />
          </span>
          <div className="wn-update-rows">
            <span className="wn-update-row">
              <Icon name="download" size={12} /> Downloading 72%
            </span>
            <span className="wn-update-row filled">
              <Icon name="restart" size={12} /> Restart to update
            </span>
          </div>
        </div>
      );
    case "symbol":
      return (
        <div className="wn-symbol">
          {[150, 116, 84].map((s) => (
            <span key={s} className={`wn-ring ${s === 84 ? "inner" : ""}`} style={{ width: s, height: s }} />
          ))}
          <span className="wn-symbol-core">
            <Icon name={art.symbol} size={26} />
          </span>
        </div>
      );
  }
}

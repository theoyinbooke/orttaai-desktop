// The card that offers the speed test once: after setup, or after updating
// from a version without it. Run Speed Test starts it in the background and
// closes the card; a toast says what it chose when it finishes, wherever the
// test was started (here or Settings › Model › Performance). Not Now, the
// close button or Escape put the card away for good; the test stays in
// Settings. It waits for What's New to close, and milestone celebrations
// wait for it.
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import "../styles/analytics.css";
import { Icon, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import { speedTestCardOpen, useWhatsNewChecked, useWhatsNewOpen } from "../lib/analytics";
import { doneMessage, type SpeedTestDone, type SpeedTestStatus } from "../lib/speedtest";

export default function SpeedTestCard() {
  const { settings } = useSettings();
  const whatsNewChecked = useWhatsNewChecked();
  const whatsNewOpen = useWhatsNewOpen();
  const toast = useToast();
  const [offer, setOffer] = useState(false);
  const cardRef = useRef<HTMLDivElement>(null);

  // Asked again when setup finishes or the speech model changes, since a
  // first-time user only has a model to time once setup is done.
  const ready = settings?.has_completed_setup ?? false;
  useEffect(() => {
    if (!whatsNewChecked || !ready) return;
    call<SpeedTestStatus>("speed_test_status")
      .then((s) => setOffer(s.offer))
      .catch(() => {});
  }, [whatsNewChecked, ready, settings?.engine, settings?.express_model, settings?.model_id, settings?.dictation_language]);

  useEvent<SpeedTestDone>("speed-test-done", (e) => {
    if (e.result) toast(doneMessage(e.result), "success");
    else toast(`Couldn't finish the speed test: ${e.error ?? "unknown error"}`, "error");
  });

  const dismiss = () => {
    setOffer(false);
    call("speed_test_offer_dismiss").catch(() => {});
  };
  const run = () => {
    setOffer(false);
    call("start_speed_test")
      .then(() => toast("The speed test is running in the background. Keep dictating; you'll see the result here in about a minute.", "info"))
      .catch((e) => toast(`Couldn't start the speed test: ${e}`, "error"));
  };
  const keys = useRef({ dismiss, run });
  keys.current = { dismiss, run };

  const shown = offer && !whatsNewOpen;
  useEffect(() => {
    speedTestCardOpen.set(shown);
    return () => speedTestCardOpen.set(false);
  }, [shown]);
  useEffect(() => {
    if (!shown) return;
    cardRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") keys.current.dismiss();
      else if (e.key === "Enter" && !(document.activeElement instanceof HTMLButtonElement)) keys.current.run();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [shown]);

  if (!shown) return null;
  return createPortal(
    <div className="celebration-overlay wn-overlay" role="presentation">
      <div className="wn-card" ref={cardRef} tabIndex={-1} role="dialog" aria-modal="true" aria-label="Speed up dictation">
        <div className="wn-art">
          <span className="wn-glow" />
          <span className="wn-dots" />
          <div className="wn-scene">
            <div className="wn-symbol">
              {[150, 116, 84].map((s) => (
                <span key={s} className={`wn-ring ${s === 84 ? "inner" : ""}`} style={{ width: s, height: s }} />
              ))}
              <span className="wn-symbol-core">
                <Icon name="speedometer" size={26} />
              </span>
            </div>
          </div>
          <button className="wn-close" onClick={dismiss} aria-label="Close" title="Close">
            <Icon name="x" size={13} />
          </button>
        </div>
        <div className="wn-body">
          <span className="wn-eyebrow">FASTER DICTATION</span>
          <div className="wn-text">
            <h2>Tune Orttaai to this computer</h2>
            <p>
              A one-minute test times your speech model on this computer's cores and keeps the fastest setting. It runs in
              the background, so you can keep dictating.
            </p>
          </div>
          <div className="wn-foot">
            <span className="celebration-spacer" />
            <button className="ghost-link wn-skip" onClick={dismiss} title="You can run it later in Settings › Model › Performance">
              Not Now
            </button>
            <button className="pill-btn primary wn-next" onClick={run}>
              Run Speed Test
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

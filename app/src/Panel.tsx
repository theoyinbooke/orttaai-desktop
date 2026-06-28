import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

/// The floating recording overlay (a separate borderless Tauri window).
export default function Panel() {
  const [state, setState] = useState("recording");

  useEffect(() => {
    const un = listen<string>("engine-state", (e) => setState(e.payload));
    return () => {
      un.then((off) => off());
    };
  }, []);

  const recording = state === "recording";

  return (
    <div className="panel-pill" data-state={state}>
      <span className="panel-bars" aria-hidden="true">
        <i />
        <i />
        <i />
        <i />
      </span>
      <span className="panel-label">{recording ? "Recording" : "Transcribing…"}</span>
    </div>
  );
}

// Tap Right Ctrl inside Orttaai's own window. The system-wide listener sees
// taps in every other app, but on Windows it can miss the ones made while this
// window has focus, so the window reports them too; the engine counts each tap
// once whichever report comes first (engine.rs `right_ctrl_window_tap`).
import { useEffect } from "react";
import { call } from "./api";

/** Same as the engine's tap: down and up within half a second, nothing else in between. */
const TAP_MAX_MS = 500;

export function useRightCtrlTap() {
  useEffect(() => {
    let start: number | null = null;
    let spoiled = false;
    const onDown = (e: KeyboardEvent) => {
      if (e.code === "ControlRight") {
        if (e.repeat) return;
        start = performance.now();
        spoiled = e.altKey || e.shiftKey || e.metaKey;
      } else if (start !== null) {
        spoiled = true;
      }
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code !== "ControlRight" || start === null) return;
      const clean = !spoiled && performance.now() - start < TAP_MAX_MS;
      start = null;
      if (clean) call("right_ctrl_window_tap").catch(() => {});
    };
    const onPointer = () => {
      if (start !== null) spoiled = true;
    };
    const onBlur = () => {
      start = null;
    };
    window.addEventListener("keydown", onDown, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("mousedown", onPointer, true);
    window.addEventListener("wheel", onPointer, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onDown, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("mousedown", onPointer, true);
      window.removeEventListener("wheel", onPointer, true);
      window.removeEventListener("blur", onBlur);
    };
  }, []);
}

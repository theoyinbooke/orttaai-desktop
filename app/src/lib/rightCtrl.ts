// Tap Right Ctrl (dictate) or Right Shift (edit) inside Orttaai's own window.
// The system-wide listener sees taps in every other app, but on Windows it can
// miss the ones made while this window has focus, so the window reports them
// too; the engine counts each tap once whichever report comes first (engine.rs
// `right_ctrl_window_tap`, `right_shift_window_tap`).
import { useEffect } from "react";
import { call } from "./api";

/** Same as the engine's tap: down and up within half a second, nothing else in between. */
const TAP_MAX_MS = 500;

const KEYS: Record<string, string> = {
  ControlRight: "right_ctrl_window_tap",
  ShiftRight: "right_shift_window_tap",
};

export function useRightCtrlTap() {
  useEffect(() => {
    let key: string | null = null;
    let start: number | null = null;
    let spoiled = false;
    const onDown = (e: KeyboardEvent) => {
      if (e.code in KEYS && (key === null || key === e.code)) {
        if (e.repeat) return;
        key = e.code;
        start = performance.now();
        spoiled =
          e.altKey || e.metaKey || (e.code === "ControlRight" ? e.shiftKey : e.ctrlKey);
      } else if (start !== null) {
        spoiled = true;
      }
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code !== key || start === null) return;
      const clean = !spoiled && performance.now() - start < TAP_MAX_MS;
      start = null;
      key = null;
      if (clean) call(KEYS[e.code]).catch(() => {});
    };
    const onPointer = () => {
      if (start !== null) spoiled = true;
    };
    const onBlur = () => {
      start = null;
      key = null;
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

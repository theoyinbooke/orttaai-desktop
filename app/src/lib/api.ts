// Thin typed wrappers over Tauri IPC shared by every view.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

/** Invoke a backend command; rejects with the backend's error string. */
export function call<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(cmd, args);
}

/** Subscribe to a backend event for the component's lifetime. */
export function useEvent<T>(name: string, handler: (payload: T) => void) {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    let off: UnlistenFn | undefined;
    let dead = false;
    listen<T>(name, (e) => ref.current(e.payload)).then((u) => {
      if (dead) u();
      else off = u;
    });
    return () => {
      dead = true;
      off?.();
    };
  }, [name]);
}

/** Unix seconds → "Today 14:02" / "Mon 09:10" / "12 Mar 2025". */
export function formatWhen(unix: number): string {
  const d = new Date(unix * 1000);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return `Today ${time}`;
  const diffDays = (now.getTime() - d.getTime()) / 86_400_000;
  if (diffDays < 6) return `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
  return d.toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" });
}

export const fmtInt = (n: number) => Math.round(n).toLocaleString();

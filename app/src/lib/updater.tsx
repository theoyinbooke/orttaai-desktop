// App update flow (mirrors the Mac's Sparkle-driven sidebar button):
// check → available → downloading % → ready ("Restart to update") → installing.
// Never restarts without a click. Checks on launch (after 15s) and every 3h;
// downloads automatically when `auto_download_updates` is on.
//
// One updater for the whole main window: `UpdaterProvider` sits at the App root
// (always mounted), so the sidebar row, About and the tray's "Check for
// Updates…" all share the same state and never download twice.

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { useEvent } from "./api";
import { useSettings } from "./settings";
import { useToast } from "../ui";

export type UpdatePhase =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "uptodate" }
  | { kind: "available"; version: string }
  | { kind: "downloading"; version: string; fraction: number }
  | { kind: "ready"; version: string }
  | { kind: "installing"; version: string }
  | { kind: "failed"; message: string };

const CHECK_EVERY_MS = 3 * 60 * 60 * 1000;

function useUpdater(autoDownload: boolean) {
  const [phase, setPhase] = useState<UpdatePhase>({ kind: "idle" });
  const updateRef = useRef<Update | null>(null);
  const busy = useRef(false);
  const autoRef = useRef(autoDownload);
  autoRef.current = autoDownload;

  const download = useCallback(async () => {
    const u = updateRef.current;
    if (!u || busy.current) return;
    busy.current = true;
    let total = 0;
    let got = 0;
    setPhase({ kind: "downloading", version: u.version, fraction: 0 });
    try {
      await u.download((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") {
          got += ev.data.chunkLength;
          setPhase({ kind: "downloading", version: u.version, fraction: total ? got / total : 0 });
        }
      });
      setPhase({ kind: "ready", version: u.version });
    } catch (e) {
      setPhase({ kind: "failed", message: String(e) });
    } finally {
      busy.current = false;
    }
  }, []);

  /** `silent`: swallow errors (startup / background checks, dev builds). */
  const checkNow = useCallback(
    async (silent = false) => {
      if (busy.current) return;
      busy.current = true;
      if (!silent) setPhase({ kind: "checking" });
      try {
        const u = await check();
        busy.current = false;
        if (!u) {
          if (!silent) setPhase({ kind: "uptodate" });
          return;
        }
        updateRef.current = u;
        setPhase({ kind: "available", version: u.version });
        if (autoRef.current) await download();
      } catch (e) {
        busy.current = false;
        if (!silent) setPhase({ kind: "failed", message: String(e) });
      }
    },
    [download],
  );

  const install = useCallback(async () => {
    const u = updateRef.current;
    if (!u) return;
    setPhase({ kind: "installing", version: u.version });
    try {
      await u.install();
      await relaunch();
    } catch (e) {
      setPhase({ kind: "failed", message: String(e) });
    }
  }, []);

  useEffect(() => {
    const first = setTimeout(() => checkNow(true), 15_000);
    const every = setInterval(() => checkNow(true), CHECK_EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, [checkNow]);

  /** The sidebar button's single action for the current phase. */
  const act = useCallback(() => {
    switch (phase.kind) {
      case "available":
        return download();
      case "ready":
        return install();
      case "downloading":
      case "installing":
      case "checking":
        return;
      default:
        return checkNow(false);
    }
  }, [phase, download, install, checkNow]);

  return { phase, checkNow, download, install, act };
}

type Updater = ReturnType<typeof useUpdater>;

const UpdaterCtx = createContext<Updater | null>(null);

/** The app's one updater. Also answers the tray's "Check for Updates…" (navigate "check-updates"). */
export function UpdaterProvider({ children }: { children: ReactNode }) {
  const { settings } = useSettings();
  const updater = useUpdater(settings?.auto_download_updates ?? true);
  const toast = useToast();
  const { phase, checkNow } = updater;
  // Only a check the tray asked for reports "up to date"; failures of a check it
  // asked for, or of a download or install, are always reported.
  const announce = useRef(false);
  const previous = useRef(phase.kind);
  useEvent<string>("navigate", (route) => {
    if (route !== "check-updates") return;
    announce.current = true;
    checkNow(false);
  });
  useEffect(() => {
    const was = previous.current;
    previous.current = phase.kind;
    if (phase.kind === "uptodate" && announce.current) toast("You're on the latest version.", "success");
    if (phase.kind === "failed" && (announce.current || was === "downloading" || was === "installing")) {
      toast(`Update check failed: ${phase.message}`, "warn");
    }
    if (phase.kind !== "checking") announce.current = false;
  }, [phase]); // eslint-disable-line react-hooks/exhaustive-deps
  return <UpdaterCtx.Provider value={updater}>{children}</UpdaterCtx.Provider>;
}

/** The shared updater: `{ phase, checkNow, download, install, act }`. */
export function useAppUpdater(): Updater {
  const u = useContext(UpdaterCtx);
  if (!u) throw new Error("useAppUpdater needs an UpdaterProvider");
  return u;
}

export function phaseLabel(p: UpdatePhase): string {
  switch (p.kind) {
    case "idle":
      return "Check for updates";
    case "checking":
      return "Checking…";
    case "uptodate":
      return "Up to date";
    case "available":
      return `Update ${p.version} available`;
    case "downloading":
      return `Downloading ${Math.round(p.fraction * 100)}%`;
    case "ready":
      return "Restart to update";
    case "installing":
      return "Installing…";
    case "failed":
      return "Update failed — retry";
  }
}

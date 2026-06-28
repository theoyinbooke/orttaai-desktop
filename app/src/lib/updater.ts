// Shared auto-update flow — used by both the manual "Check for updates" buttons
// (App header + Settings) and the silent check that runs on startup.
//
// The whole pipeline (signed GitHub release → latest.json → download → install →
// relaunch) is handled by the Tauri updater plugin; this just drives it and
// reports progress so each caller can render it its own way.

import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStage =
  | { kind: "checking" }
  | { kind: "uptodate" }
  | { kind: "available"; version: string }
  | { kind: "downloading"; version: string }
  | { kind: "installed"; version: string }
  | { kind: "error"; message: string };

/**
 * Check for an update; if one exists, download + install it and relaunch.
 *
 * `onStage` reports each step. With `{ silent: true }` a failing `check()` is
 * swallowed (returns false, no `error` stage) — this is the startup path, where
 * a `tauri dev` build (no updater) or a not-yet-published endpoint would
 * otherwise throw a scary error the user never asked to see. The manual buttons
 * pass `silent: false` so the user gets a result either way.
 *
 * Resolves `true` only when an update was installed (the app is relaunching).
 */
export async function runUpdate(
  onStage: (stage: UpdateStage) => void,
  opts: { silent?: boolean } = {},
): Promise<boolean> {
  onStage({ kind: "checking" });

  let update;
  try {
    update = await check();
  } catch (e) {
    if (opts.silent) return false;
    onStage({ kind: "error", message: String(e) });
    return false;
  }

  if (!update) {
    onStage({ kind: "uptodate" });
    return false;
  }

  onStage({ kind: "available", version: update.version });
  try {
    onStage({ kind: "downloading", version: update.version });
    await update.downloadAndInstall();
    onStage({ kind: "installed", version: update.version });
    await relaunch();
    return true;
  } catch (e) {
    onStage({ kind: "error", message: String(e) });
    return false;
  }
}

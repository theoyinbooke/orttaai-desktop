// Shared shell types. Feature areas keep their own DTO types next to their views
// (or in src/lib/<area>.ts).

export type AppInfo = {
  name: string;
  version: string;
  platform: string;
  session: string;
  gpu_backend: string;
};

/** Top-level pages, in sidebar order (mirrors the macOS HomeNavigation). */
export type Tab =
  | "home"
  | "chat"
  | "graph"
  | "memory"
  | "devices"
  | "analytics"
  | "milestones"
  | "coach"
  | "models"
  | "settings"
  | "about";

export const TABS: readonly Tab[] = [
  "home",
  "chat",
  "graph",
  "memory",
  "analytics",
  "milestones",
  "coach",
  "models",
  "settings",
  "about",
  // Last, so the number shortcuts for the other pages stay as they were.
  "devices",
];

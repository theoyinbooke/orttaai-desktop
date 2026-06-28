// Shared domain types — mirror the Rust DTOs exposed over Tauri IPC.

export type AppInfo = { name: string; version: string; platform: string };

export type Settings = {
  model_id: string;
  push_to_talk: string;
  preserve_clipboard: boolean;
  ollama_endpoint: string;
  strict_secure: boolean;
  preset: string;
  n_threads: number;
};

export type HistoryItem = {
  id: number;
  text: string;
  app: string | null;
  word_count: number;
  created_at: number;
};

export type ModelInfo = {
  id: string;
  name: string;
  approx_size_mb: number;
  multilingual: boolean;
  url: string;
  downloaded: boolean;
  path: string;
};

export type DashboardStats = {
  total: number;
  total_words: number;
  avg_words: number;
  avg_wpm: number;
  last7_days: { day: string; count: number }[];
  top_apps: { app: string; count: number }[];
};

export type MemoryEntry = {
  id: number | null;
  kind: string;
  trigger: string;
  replacement: string;
};

export type Tab =
  | "dictate"
  | "history"
  | "insights"
  | "dictionary"
  | "models"
  | "assistant"
  | "settings";

export type EngineState = "off" | "loading" | "idle" | "recording" | "processing";

export const STATE_LABEL: Record<EngineState, string> = {
  off: "Off",
  loading: "Starting…",
  idle: "Listening",
  recording: "Recording",
  processing: "Transcribing",
};

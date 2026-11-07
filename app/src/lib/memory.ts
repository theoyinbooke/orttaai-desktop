// Personal Memory: DTOs and typed wrappers over the text-pipeline commands
// (app/src-tauri/src/memory.rs). Every change emits `memory-changed`.
import { useCallback, useEffect, useState } from "react";
import { call, useEvent } from "./api";
import type { AppInfo } from "../types";

export type DictionaryEntry = {
  id: number;
  source: string;
  target: string;
  is_case_sensitive: boolean;
  is_active: boolean;
  usage_count: number;
  created_at: number;
  updated_at: number;
};

export type SnippetEntry = {
  id: number;
  trigger: string;
  expansion: string;
  is_active: boolean;
  usage_count: number;
  created_at: number;
  updated_at: number;
};

export type LearningSuggestion = {
  id: number;
  kind: "dictionary" | "snippet";
  candidate_source: string;
  candidate_target: string;
  normalized_source: string;
  confidence: number;
  status: "pending" | "accepted" | "dismissed";
  evidence: string | null;
  /** `edit` = learned from the user's own fix; `history` = found by analysis. */
  origin: "edit" | "history";
  created_at: number;
  updated_at: number;
};

export type EditCheck = {
  id: number;
  checked_at: number;
  app_name: string | null;
  outcome:
    | "learned"
    | "already_known"
    | "edited_no_name"
    | "no_change"
    | "text_not_found"
    | "unreadable"
    | "secure_field"
    | "not_english";
  learned_count: number;
  description: string;
};

export type AnalysisReport = {
  analyzer_name: string;
  inserted_count: number;
  history_sample_count: number;
  used_fallback: boolean;
  ai_error: string | null;
  message: string;
};

export type ImportSummary = {
  dictionary_added: number;
  dictionary_updated: number;
  snippets_added: number;
  snippets_updated: number;
  skipped: number;
};

export type PipelinePreview = {
  text: string;
  changes: string[];
  dictionary_hits: string[];
  snippet_hits: string[];
  style_instruction: string | null;
};

export const memoryApi = {
  listDictionary: () => call<DictionaryEntry[]>("list_dictionary"),
  saveDictionary: (e: { id?: number; source: string; target: string; caseSensitive: boolean; active?: boolean }) =>
    call("save_dictionary", { id: e.id ?? null, source: e.source, target: e.target, caseSensitive: e.caseSensitive, active: e.active ?? null }),
  setDictionaryActive: (id: number, active: boolean) => call("set_dictionary_active", { id, active }),
  deleteDictionary: (id: number) => call("delete_dictionary", { id }),

  listSnippets: () => call<SnippetEntry[]>("list_snippets"),
  saveSnippet: (e: { id?: number; trigger: string; expansion: string; active?: boolean }) =>
    call("save_snippet", { id: e.id ?? null, trigger: e.trigger, expansion: e.expansion, active: e.active ?? null }),
  setSnippetActive: (id: number, active: boolean) => call("set_snippet_active", { id, active }),
  deleteSnippet: (id: number) => call("delete_snippet", { id }),

  listSuggestions: () => call<LearningSuggestion[]>("list_suggestions"),
  acceptSuggestion: (id: number) => call("accept_suggestion", { id }),
  dismissSuggestion: (id: number) => call("dismiss_suggestion", { id }),
  listEditChecks: () => call<EditCheck[]>("list_edit_checks"),
  analyzeHistory: () => call<AnalysisReport>("analyze_history"),

  exportMemory: () => call<string>("export_memory"),
  importMemory: (json: string) => call<ImportSummary>("import_memory", { json }),

  preview: (text: string, appName?: string) =>
    call<PipelinePreview>("preview_text_pipeline", { text, appName: appName ?? null }),
};

/** Everything the Memory page shows, reloaded on `memory-changed`. */
export function useMemoryData() {
  const [dictionary, setDictionary] = useState<DictionaryEntry[]>([]);
  const [snippets, setSnippets] = useState<SnippetEntry[]>([]);
  const [suggestions, setSuggestions] = useState<LearningSuggestion[]>([]);
  const [editChecks, setEditChecks] = useState<EditCheck[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    Promise.all([
      memoryApi.listDictionary(),
      memoryApi.listSnippets(),
      memoryApi.listSuggestions(),
      memoryApi.listEditChecks(),
    ])
      .then(([d, s, g, c]) => {
        setDictionary(d);
        setSnippets(s);
        setSuggestions(g);
        setEditChecks(c);
        setError(null);
      })
      .catch(() => setError("Couldn't load memory data."))
      .finally(() => setLoading(false));
  }, []);

  useEffect(reload, [reload]);
  useEvent("memory-changed", reload);
  useEvent("edit-checks-changed", reload);
  return { dictionary, snippets, suggestions, editChecks, loading, error, reload };
}

/** The OS the app runs on (`windows`, `linux`, …), once known. */
export function usePlatform(): string | null {
  const [platform, setPlatform] = useState<string | null>(null);
  useEffect(() => {
    call<AppInfo>("app_info")
      .then((i) => setPlatform(i.platform))
      .catch(() => setPlatform(null));
  }, []);
  return platform;
}

/** Case-insensitive "contains" over any of the given fields. */
export function matches(query: string, ...fields: string[]): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return fields.some((f) => f.toLowerCase().includes(q));
}

/** "3 entries" or "2 of 3 active" (Mac count chip). */
export function countLabel(active: number, total: number): string {
  if (active === total) return `${total} ${total === 1 ? "entry" : "entries"}`;
  return `${active} of ${total} active`;
}

/** Relative time for edit checks ("just now", "5 min ago", "yesterday"). */
export function relativeTime(unix: number): string {
  const diff = Math.max(0, Date.now() / 1000 - unix);
  if (diff < 45) return "just now";
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`;
  if (diff < 86_400) {
    const h = Math.round(diff / 3600);
    return `${h} hour${h === 1 ? "" : "s"} ago`;
  }
  const d = Math.round(diff / 86_400);
  if (d === 1) return "yesterday";
  if (d < 7) return `${d} days ago`;
  return new Date(unix * 1000).toLocaleDateString([], { day: "numeric", month: "short" });
}

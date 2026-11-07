// App-wide settings context. The backend owns the `Settings` JSON (see
// core/src/settings.rs); the UI reads it whole and writes partial patches.
import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { call, useEvent } from "./api";

export type Modifier = "ctrl" | "shift" | "alt" | "meta";
export type HotkeyCombo = { key: string; modifiers: Modifier[] };

export type Settings = {
  model_id: string;
  engine: "whisper" | "express";
  decode: { language: string | null; preset: "fast" | "balanced" | "accuracy"; temperature: number; n_threads: number };
  advanced_decoding: {
    enabled: boolean;
    temperature: number;
    best_of: number;
    temperature_fallback_count: number;
    compression_ratio_threshold: number;
    logprob_threshold: number;
    no_speech_threshold: number;
  };
  vocabulary_bias: boolean;
  express_word_boost: boolean;
  low_latency: boolean;
  warm_models: boolean;

  push_to_talk: HotkeyCombo;
  hands_free_enabled: boolean;
  hands_free_silence_stop: boolean;
  hands_free_silence_secs: number;
  hands_free_max_secs: number;
  max_recording_secs: number;
  dictation_language: string;
  audio_device: string | null;
  sound_cues: boolean;

  edit_commands_enabled: boolean;
  edit_shortcut: HotkeyCombo;
  edit_timeout_ms: number;
  edit_max_chars: number;

  style_mode: "raw" | "clean" | "formal" | "casual";
  spoken_formatting: boolean;
  auto_paragraphs: boolean;
  dictionary_enabled: boolean;
  fuzzy_dictionary: boolean;
  snippets_enabled: boolean;
  disfluency_cleanup: boolean;
  ai_suggestions: boolean;
  learn_from_edits: boolean;

  injection_mode: "auto" | "type" | "paste";
  preserve_clipboard: boolean;
  wayland_restore_token: string | null;
  strict_secure: boolean;

  llm_provider: "ollama" | "lm_studio";
  ollama_endpoint: string;
  lmstudio_endpoint: string;
  polish_enabled: boolean;
  polish_engine: "s1_mini" | "local_llm";
  polish_model: string;
  polish_timeout_ms: number;
  polish_max_chars: number;
  chat_model: string;
  chat_thinking: "low" | "medium" | "high";
  insights_enabled: boolean;
  insights_model: string;
  insights_context_tokens: number;
  insights_thinking: boolean;
  semantic_enabled: boolean;
  semantic_auto_index: boolean;
  semantic_keyword_fallback: boolean;
  semantic_embedding_model: string;
  semantic_summary_enabled: boolean;
  semantic_summary_model: string;

  coach_enabled: boolean;
  coach_first_run_done: boolean;
  coach_persona: string;
  coach_app_overrides: Record<string, string>;
  coach_partner_model: string;
  coach_judge_model: string;
  // Coach: note writer ("" = automatic), coach per way of talking, read-aloud,
  // hands-free scenes, setup card seen, preferred speech voice ("" = automatic).
  coach_model: string;
  coach_personas: Record<string, string>;
  coach_read_aloud: boolean;
  coach_hands_free: boolean;
  coach_model_setup_seen: boolean;
  coach_voice: string;

  appearance: "system" | "light" | "dark";
  launch_at_login: boolean;
  start_engine_on_launch: boolean;
  show_processing_estimate: boolean;
  celebrate_milestones: boolean;
  has_completed_setup: boolean;
  auto_download_updates: boolean;
  analytics_time_range: string;
  last_whats_new_version: string | null;
};

type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> | T[K] : T[K] };
export type SettingsPatch = DeepPartial<Settings>;

const Ctx = createContext<{
  settings: Settings | null;
  patch: (p: SettingsPatch) => Promise<Settings | null>;
  reload: () => void;
}>({ settings: null, patch: async () => null, reload: () => {} });

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const reload = useCallback(() => {
    call<Settings>("get_settings").then(setSettings).catch(() => {});
  }, []);
  useEffect(reload, [reload]);
  useEvent<Settings>("settings-changed", setSettings);
  const patch = useCallback(async (p: SettingsPatch) => {
    const next = await call<Settings>("update_settings", { patch: p });
    setSettings(next);
    return next;
  }, []);
  return <Ctx.Provider value={{ settings, patch, reload }}>{children}</Ctx.Provider>;
}

/** `{ settings, patch, reload }` — `patch` saves immediately and rejects with the error. */
export const useSettings = () => useContext(Ctx);

export function comboToString(c: HotkeyCombo | undefined | null): string {
  if (!c) return "";
  const names: Record<Modifier, string> = { ctrl: "Ctrl", shift: "Shift", alt: "Alt", meta: "Super" };
  return [...c.modifiers.map((m) => names[m]), c.key].join("+");
}

# macOS → Linux/Windows parity map

Reference: the native Mac app at v1.18.1. Each row lists the Mac source and where the
desktop port lives. "Adapted" means the Apple framework was replaced with a portable
equivalent; behaviour, thresholds and copy follow the Mac.

| Mac feature (source) | Desktop (core / app / UI) | Notes |
|---|---|---|
| Dictation state machine, hold / tap→hands-free (`DictationCoordinator`, `HotkeyGestureInterpreter`) | `core/src/coordinator.rs`, `core/src/gesture.rs`, `app/src-tauri/src/engine.rs` | Same tap threshold, caps, countdown |
| Hands-free silence stop (`HandsFreeAutoStop`, `SileroVoiceDetector`) | `core/src/vad.rs` | Adapted: Silero via whisper.cpp VAD + adaptive-RMS fallback |
| Audio capture + Bluetooth recovery (`AudioCaptureService`, `AudioDeviceManager`) | `core/src/audio/` | cpal; Mac retry schedule; keeps audio heard |
| Whisper (WhisperKit/CoreML) | `core/src/transcription/whisper.rs` | Adapted: whisper.cpp (CPU / CUDA) |
| Express engine (Parakeet, CoreML) | `core/src/transcription/express.rs` | Adapted: Parakeet TDT 0.6B int8 via sherpa-onnx |
| Model manager, tiers, hardware detection | `core/src/models/`, `core/src/hardware.rs`, `views/Models.tsx` | Tiers redefined for CPU / RAM / NVIDIA |
| Text pipeline (`TextProcessor`, `DisfluencyCleaner`, `FuzzyDictionaryMatcher`, number formatter) | `core/src/text/`, `core/src/memory.rs` | Same step order; bundled word list |
| Learning suggestions + learn from edits (`MemoryLearningService`, `EditLearningWatcher`) | `core/src/store/learning.rs`, `core/src/injection/edit_watch.rs` | Field reading on Windows (UIA); Linux reports "unreadable" |
| Text injection, clipboard, selection capture (`TextInjectionService`, AX) | `core/src/injection/`, `core/src/clipboard.rs` | SendInput / XTest / portal / wtype; paste with restore |
| Polish: S1-mini + local LLM (`S1MiniPolish`, `LocalLLMTextProcessor`, `PolishEngine`) | `core/src/polish.rs`, `app/src-tauri/src/pipeline.rs` | S1-mini served by Ollama/LM Studio (`hf.co/superwhisper/s1-mini-GGUF`) |
| Voice edit (`EditCommandProcessor`) | `core/src/edit.rs` | Ctrl+C capture, never invents text |
| Ollama / LM Studio clients, local-only policy | `core/src/llm/` | Streaming, JSON schema, embeddings, pulls |
| Chat AI (`ChatAIView`) | `core/src/chat.rs`, `views/ChatAI.tsx` | Saved conversations, memory context, reasoning |
| Writing Insights, Tone of Voice | `core/src/writing.rs`, `views/analytics/*` | Deterministic fallback without an LLM |
| Semantic memory, concept graph, signals, findings, work activity | `core/src/semantic/`, `views/MemoryGraph.tsx` | Adapted: portable analyzer replaces NaturalLanguage |
| Home overview, Analytics dashboard, History | `core/src/analytics.rs`, `views/Home.tsx`, `views/Analytics.tsx`, `views/History.tsx` | Hand-rolled SVG charts |
| Milestones + share cards | `core/src/milestones.rs`, `views/Milestones.tsx`, `components/ShareCard*` | 22 badges, PNG export |
| What's New | `core/src/whatsnew.rs`, `components/WhatsNew.tsx` | Desktop release notes |
| Coach (CoachCore + Coach UI) | `core/src/coach/`, `views/Coach.tsx` | Voices via WebView `speechSynthesis` |
| Setup wizard, menu bar, floating panel | `views/Setup.tsx`, tray in `lib.rs`, `Panel.tsx` | Tray replaces the menu bar |
| Settings (General, Dictation, Audio, Text, AI, Coach, About) | `core/src/settings.rs`, `views/Settings.tsx` + `views/settings/*` | Every Mac key has a field |
| Sparkle updates (sidebar button) | `lib/updater.ts` + Tauri updater | Same phases; never restarts without a click |
| Database (GRDB migrations v1–v24) | `core/src/store/schema.rs` | Same tables (minus CloudKit sync tables) |
| Design system (`Orttaai/Design`) | `app/src/App.css` tokens | Mac colours and radii; Inter for SF Pro |

## Not ported (Apple-only)

- **iCloud / CloudKit sync** with the iPhone keyboard (Orttakey). The Mac stores record
  payloads in CloudKit *encrypted* fields, which only Apple devices can decrypt.
  Options: a Mac relay over the LAN, or a dual-written app-encrypted field read through
  CloudKit Web Services. See the discussion in the project notes.
- **Apple Intelligence polish** (Foundation Models) — use S1-mini or a local model.
- **CoreML coach voices** (Pocket TTS, Phonon) — the WebView's system voices are used.

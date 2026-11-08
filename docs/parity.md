# macOS → Linux/Windows parity map

Reference: the native Mac app at v1.18.1. Each row lists the Mac source and where the
desktop port lives. "Adapted" means the Apple framework was replaced with a portable
equivalent; behaviour, thresholds and copy follow the Mac.

| Mac feature (source) | Desktop (core / app / UI) | Notes |
|---|---|---|
| Dictation state machine, hold / tap→hands-free (`DictationCoordinator`, `HotkeyGestureInterpreter`) | `core/src/coordinator.rs`, `core/src/gesture.rs`, `app/src-tauri/src/engine.rs` | Same tap threshold, caps, countdown |
| Tap Right Option to dictate (`RightOptionTapDetector`, `RightOptionTapMonitor`) | `core/src/hotkey/tap.rs`, `core/src/hotkey/right_ctrl.rs` | Adapted: Right Ctrl (Right Alt is AltGr on many keyboards); Windows low-level hooks, X11 XInput2 raw events; unavailable on Wayland |
| Hands-free silence stop (`HandsFreeAutoStop`, `SileroVoiceDetector`) | `core/src/vad.rs` | Adapted: Silero via whisper.cpp VAD + adaptive-RMS fallback |
| Audio capture + Bluetooth recovery (`AudioCaptureService`, `AudioDeviceManager`) | `core/src/audio/` | cpal; Mac retry schedule; keeps audio heard |
| Whisper (WhisperKit/CoreML) | `core/src/transcription/whisper.rs` | Adapted: whisper.cpp (CPU / CUDA) |
| Background decoding while speaking (`TranscriptionService` live session, Express "Decode while I speak") | `core/src/live.rs`, coordinator `poll_live` / `decode_recording` | Partial: same clip, pause and segment cut points, and live clips run unprompted as on the Mac. Missing: the Mac's speculative tail decode (Whisper) and Express early decode at a 0.5 s pause (see Paused below) |
| Express Word Boost (`ExpressTranscriber` Word Boost, FluidAudio `VocabularyRescorer`, `WordBoostGuard`, `WordBoostChunks`) | `core/src/wordboost/` | Parakeet CTC 110M as sherpa-onnx ONNX on sherpa's ONNX Runtime; NeMo mel front end in Rust; rescorer ported from FluidAudio (Apache-2.0) |
| Express fallback (`isExpressEngineActive`, `ensureTranscriptionModelLoaded`) | `Settings::express_active`, `load_transcriber` in `engine.rs` | Express only for its languages; a failed Express load dictates with Whisper and says so |
| Return to the target app before typing (`waitForReturn`) | `core/src/focus.rs` `WindowFocus`, coordinator `deliver_to_target` | X11 `_NET_ACTIVE_WINDOW`, Windows `SetForegroundWindow`; 8 s wait, else clipboard. Not on Wayland |
| Express engine (Parakeet, CoreML) | `core/src/transcription/express.rs` | Adapted: Parakeet TDT 0.6B int8 via sherpa-onnx, with the Mac's quiet-tail lift (`lift_quiet_tail`) and 0.4 s trailing pad |
| Model manager, tiers, hardware detection | `core/src/models/`, `core/src/hardware.rs`, `views/Models.tsx` | Tiers redefined for CPU / RAM / NVIDIA |
| Text pipeline (`TextProcessor`, `DisfluencyCleaner`, `FuzzyDictionaryMatcher`, number formatter) | `core/src/text/`, `core/src/memory.rs` | Same step order; bundled word list |
| Learning suggestions + learn from edits (`MemoryLearningService`, `EditLearningWatcher`) | `core/src/store/learning.rs`, `core/src/injection/edit_watch.rs` | Field reading through UI Automation (Windows) and AT-SPI (Linux) |
| Text injection, clipboard, selection capture (`TextInjectionService`, AX) | `core/src/injection/`, `core/src/clipboard.rs` | SendInput / XTest / portal / wtype; paste with restore |
| Polish: S1-mini + local LLM (`S1MiniPolish`, `LocalLLMTextProcessor`, `PolishEngine`) | `core/src/polish.rs`, `app/src-tauri/src/pipeline.rs` | Adapted: S1-mini needs Ollama or LM Studio (`hf.co/superwhisper/s1-mini-GGUF`); the Mac runs it inside the app |
| Voice edit (`EditCommandProcessor`) | `core/src/edit.rs` | Ctrl+C capture, never invents text |
| Ollama / LM Studio clients, local-only policy | `core/src/llm/` | Streaming, JSON schema, embeddings, pulls |
| Chat AI (`ChatAIView`) | `core/src/chat.rs`, `views/ChatAI.tsx` | Saved conversations, memory context, reasoning |
| Writing Insights, Tone of Voice | `core/src/writing.rs`, `views/analytics/*` | Deterministic fallback without an LLM |
| Semantic memory, concept graph, signals, findings, work activity | `core/src/semantic/`, `views/MemoryGraph.tsx` | Adapted: portable analyzer replaces NaturalLanguage |
| Home overview, Analytics dashboard, History | `core/src/analytics.rs`, `views/Home.tsx`, `views/Analytics.tsx`, `views/History.tsx` | Hand-rolled SVG charts |
| Milestones + share cards | `core/src/milestones.rs`, `views/Milestones.tsx`, `components/ShareCard*` | 22 badges, PNG export |
| What's New | `core/src/whatsnew.rs`, `components/WhatsNew.tsx` | Desktop release notes |
| Coach (CoachCore + Coach UI) | `core/src/coach/`, `views/Coach.tsx` | Voices via WebView `speechSynthesis` |
| Setup window (`SetupView`, `SetupArtwork`, Permission/Download/Ready steps) | `views/Setup.tsx`, `components/SetupArt.tsx`, `open_setup_window` / `finish_setup` in `lib.rs` | Its own fixed 600 x 750 window, shown alone on first launch; same four steps, step track and animated scenes. Permissions = microphone, typing, shortcut |
| Window shell (`HomeShellView`, `HomeNavigation`, `SectionRail`, `WorkspaceHeader`) | `App.tsx`, `App.css` (shell section), `PageHeader` in `ui.tsx` | Sidebar dark in both looks, same sections and order; integrated title strip with the page title, tabs and actions; window controls drawn by the app (undecorated window) |
| Menu bar, floating panel | tray in `lib.rs`, `Panel.tsx` | The tray replaces the menu bar: same items with a Polish toggle, and its icon shows the state. On Windows a click on the icon starts and stops dictation, and is the idle handle instead of a pill (a floating pill over the taskbar fought the taskbar). The pill shows only while dictating, just above the taskbar on Windows |
| Settings (Dictation, Text, Audio, Coach, General) and About | `core/src/settings.rs`, `views/Settings.tsx` + `views/settings/*`, `views/About.tsx` | AI Features lives on the Model page, as on the Mac. Difference: no Low Latency Mode (removed; Express covers speed) |
| Model page (`ModelSettingsView`: Speech and AI Features) | `views/Models.tsx`, `components/ExpressCard.tsx`, `views/settings/AI.tsx` | Same |
| Sparkle updates (sidebar button) | `lib/updater.ts` + Tauri updater | Same phases; never restarts without a click |
| Database (GRDB migrations v1–v24) | `core/src/store/schema.rs` | Same tables (minus CloudKit sync tables) |
| Design system (`Orttaai/Design`) | `app/src/App.css` tokens | Mac colours and radii; Inter for SF Pro |

## Paused (decided 2026-11-08)

Built since: decode before release, the tray Polish toggle and state icon, full Reset
App Data, and images kept on the clipboard. The idle pill was built and then replaced on
Windows by click-to-dictate on the tray icon. Still left for later:

- **S1-mini inside the app** (no Ollama/LM Studio needed): llama.cpp and whisper.cpp
  both bundle ggml, so linking both needs work first.
- **Per-dictation finalize trace** (the optional `ORTTAAI_LOG` log covers errors only).

## Not ported (Apple-only)

- **iCloud / CloudKit sync** with the iPhone keyboard (Orttakey). The Mac stores record
  payloads in CloudKit *encrypted* fields, which only Apple devices can decrypt.
  Options: a Mac relay over the LAN, or a dual-written app-encrypted field read through
  CloudKit Web Services. See the discussion in the project notes.
- **Apple Intelligence polish** (Foundation Models) — use S1-mini or a local model.
- **CoreML coach voices** (Pocket TTS, Phonon) — the WebView's system voices are used.

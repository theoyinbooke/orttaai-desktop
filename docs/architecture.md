# Architecture

## The one rule

> **Build the core once; quarantine every OS-specific behavior behind a trait.**

Durable logic lives in `core/` (the `orttaai-core` crate), which has **no GUI dependencies**. The UI (`app/`, a Tauri shell, added in Phase 2) and the headless `cli/` are thin consumers of the core. When an OS changes (a new Wayland protocol, a new model engine), you edit **one trait implementation** — never the coordinator, UI, data, or settings.

## Layers

```
app/ (Tauri)  ──┐
cli/          ──┼──►  orttaai-core
                │       ├─ coordinator      hold/tap → record → transcribe → pipeline → deliver
                │       ├─ traits (OS-isolated):
                │       │    Transcriber   → whisper.cpp (`whisper`) · Fluid Speech Mini on ONNX Runtime · Parakeet/sherpa-onnx (`express`)
                │       │    AudioCapture  → cpal + resample (`cpal-audio`)
                │       │    TextInjector  → SendInput / XTest / RemoteDesktop portal / wtype
                │       │    HotkeyManager → RegisterHotKey / XGrabKey / GlobalShortcuts portal
                │       │    Clipboard     → arboard (+ wl-copy)
                │       │    VoiceDetector → Silero (whisper.cpp VAD) / adaptive RMS
                │       │    LlmClient     → Ollama · LM Studio (`ollama`), MockLlm in tests
                │       └─ portable services:
                │            Store (rusqlite, versioned migrations) · Settings ·
                │            text pipeline · polish · edit · chat · writing ·
                │            semantic memory · analytics · milestones · coach · whatsnew
```

The Tauri shell is split by area (`engine`, `pipeline`, `memory`, `ai`, `semantic`,
`insights`, `coach`, `express_cmds`, `settings_cmds`); every completed dictation goes
through `persist::save_record`, whose `after_save` hook fans out to semantic indexing,
milestones and Coach metrics.

## The dictation loop

1. Hotkey **down** → start capture. A short tap promotes the session to hands-free.
2. Release (or tap / silence / cap in hands-free) → stop, keep every sample heard.
3. `raw = Transcriber.transcribe(samples)` (vocabulary-bias prompt for Whisper).
4. `TranscriptPipeline.process(raw)` → dictionary → fuzzy → snippets → filler cleanup →
   spoken formatting → paragraphs → optional Polish (local LLM, never fails).
5. An armed Coach practice rep takes the text instead of typing it; otherwise check the
   secure field, deliver (type or paste + clipboard restore), and record the method.
6. Persist the full `TranscriptionRecord` (per-stage timings) and emit UI events.

## Backend selection

- **Mock backends** (always compiled) make the core build and test green on any OS, including the macOS dev machine.
- **Real backends** are enabled per platform via cargo features and `#[cfg(target_os)]`. On macOS the platform impls are stubs (this is a Linux/Windows product; macOS is only a dev host).

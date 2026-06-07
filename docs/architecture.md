# Architecture

## The one rule

> **Build the core once; quarantine every OS-specific behavior behind a trait.**

Durable logic lives in `core/` (the `orttaai-core` crate), which has **no GUI dependencies**. The UI (`app/`, a Tauri shell, added in Phase 2) and the headless `cli/` are thin consumers of the core. When an OS changes (a new Wayland protocol, a new model engine), you edit **one trait implementation** — never the coordinator, UI, data, or settings.

## Layers

```
app/ (Tauri)  ──┐
cli/          ──┼──►  orttaai-core
                │       ├─ coordinator      the press→speak→inject state machine
                │       ├─ traits (OS-isolated):
                │       │    Transcriber   → whisper.cpp (feature `whisper`)
                │       │    AudioCapture  → cpal + resample (feature `cpal-audio`)
                │       │    TextInjector  → SendInput / xdotool / wtype
                │       │    HotkeyManager → RegisterHotKey / XGrabKey / portal
                │       │    Clipboard     → arboard
                │       └─ portable services:
                │            Store (rusqlite) · Settings (serde+directories) ·
                │            MemoryService · LlmClient (Ollama) · Analytics
```

## The dictation loop

1. Hotkey **down** → `AudioCapture.start()` (background thread fills a ring buffer; `level()` drives the meter).
2. Hotkey **up** → `samples = AudioCapture.stop()`.
3. `text = Transcriber.transcribe(samples, opts)`.
4. `text = MemoryService.apply(text)` (dictionary replacements + snippet expansion).
5. `Clipboard.save()` → set transcript → `TextInjector.inject(text)` → `Clipboard.restore()`.
6. Persist a `TranscriptionRecord`; emit an event to the UI.

## Backend selection

- **Mock backends** (always compiled) make the core build and test green on any OS, including the macOS dev machine.
- **Real backends** are enabled per platform via cargo features and `#[cfg(target_os)]`. On macOS the platform impls are stubs (this is a Linux/Windows product; macOS is only a dev host).

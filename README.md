# Orttaai for Linux & Windows

**Cross-platform voice keyboard. Press a hotkey, speak, and your words appear at the cursor — in any app. 100% on-device.**

This is a standalone Rust + Tauri reimplementation of [Orttaai](https://github.com/theoyinbooke/orttaai) (the native macOS app) for **Linux and Windows**. The macOS app is a separate project and is not affected by this one.

## Architecture

A **write-once Rust core** + a **disposable Tauri UI shell**. Every OS-specific behavior (audio, text injection, global hotkey) lives behind a trait, so a change in one platform never forces a rewrite elsewhere.

```
app/   Tauri 2.x shell (web UI)        ← added in Phase 2
core/  orttaai-core (Rust, no GUI deps) ← the durable engine
cli/   headless dictation tool          ← Phase 0/1 deliverable
```

See [`docs/architecture.md`](docs/architecture.md) and the full build plan for details.

## Status

**Phase 1 — backends landing.** The OS-agnostic core (traits, coordinator, memory, settings, store) is green. **Real backends working: whisper.cpp transcription** (verified on a WAV), **cpal microphone capture** with rubato resampling to 16 kHz (resampling unit-tested), and **text injection** (`enigo` for Windows/macOS/X11, `wtype` for Wayland). The only remaining backend is the **global hotkey**; once it lands the full `DictationCoordinator` loop runs on real hardware. Live capture/injection are verified on the target machine — see the platform notes below.

## Build

```bash
cargo build           # OS-agnostic core + CLI (mock backends)
cargo test            # unit tests
cargo run -p orttaai-cli -- demo   # run the dictation loop with mock backends
```

### Real transcription (whisper.cpp)

```bash
# tiny English model (~75 MB); models/ is git-ignored
mkdir -p models
curl -L -o models/ggml-tiny.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin

# transcribe a 16 kHz mono WAV
cargo run -p orttaai-cli --features whisper -- \
  transcribe models/ggml-tiny.en.bin path/to/audio.wav
```

> Building `--features whisper` compiles whisper.cpp via cmake (needs a C/C++ toolchain).
> GPU acceleration is a build flag, not a code change: enable whisper-rs's `cuda` /
> `vulkan` / `metal` / `hipblas` features per target.
>
> Known cosmetic follow-up: whisper.cpp logs to stderr; the transcript itself prints
> cleanly to stdout. We'll route those logs through `tracing` when the app shell lands.

### Microphone capture (cpal)

```bash
# list input devices
cargo run -p orttaai-cli --features audio -- devices

# record 5s from the mic and transcribe it (full mic → whisper pipeline)
cargo run -p orttaai-cli --features "audio whisper" -- \
  record 5 models/ggml-tiny.en.bin
```

> Building `--features audio` needs ALSA headers on Linux (`libasound2-dev`); macOS
> and Windows need nothing extra.
>
> **macOS note:** capturing (and even *enumerating*) the microphone is gated behind
> the system privacy prompt. Grant your terminal microphone access in
> *System Settings → Privacy & Security → Microphone* before running `record`/`devices`,
> or the call will block waiting for permission.

### Text injection (enigo / wtype)

```bash
# Linux build dep: libxdo-dev (enigo's X11 backend); Wayland uses `wtype` at runtime.
cargo run -p orttaai-cli --features injection -- inject "hello from orttaai"
```

> `inject` waits 2 s so you can focus a target window, then types the text.
> Backends: Windows `SendInput`, macOS `CGEvent`, Linux X11 via `enigo`, Linux
> Wayland via `wtype` (install it). On macOS, grant **Accessibility** permission to
> your terminal (*System Settings → Privacy & Security → Accessibility*).
>
> Secure/password-field detection is not reliable on Linux, so it is reported as
> `Unknown` and injection is **not** blocked there — see [`docs/gaps.md`](docs/gaps.md).

## Platform support targets

| | Audio | Injection | Global hotkey | Notes |
|---|---|---|---|---|
| **Windows** | WASAPI (`cpal`) | `SendInput` (`enigo`) | `RegisterHotKey` | Easiest target — all solid |
| **Linux X11** | ALSA/Pipe (`cpal`) | `xdotool` | `XGrabKey` | Reliable |
| **Linux Wayland** | PipeWire (`cpal`) | `wtype` | XDG portal | Brittle hotkey; see gaps doc |

## Known OS-level gaps

Secure/password-field detection is unreliable on Linux, Wayland global hotkeys are brittle, and there is no Apple-Neural-Engine-class acceleration. See [`docs/gaps.md`](docs/gaps.md).

## License

[MIT](LICENSE)

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

**Phase 0 — foundation scaffold.** The core engine, traits, coordinator, memory, settings, and store compile and test green; real platform backends (whisper.cpp, cpal audio, wtype/SendInput injection, global hotkeys) are wired behind cargo features and `#[cfg(target_os)]` and are filled in per the roadmap.

## Build

```bash
cargo build           # OS-agnostic core + CLI (mock backends)
cargo test            # unit tests
cargo run -p orttaai-cli -- demo   # run the dictation loop with mock backends
```

Real backends (Linux/Windows) are enabled with cargo features, e.g. `--features whisper,cpal-audio`.

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

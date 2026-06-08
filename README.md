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

**Phase 1 — all real backends landed.** The OS-agnostic core (traits, coordinator, memory, settings, store) is green, and **all four platform backends are implemented**: whisper.cpp transcription, cpal microphone capture (rubato resampling to 16 kHz), text injection (`enigo` + `wtype`), and the global push-to-talk hotkey (`global-hotkey`, with a win32 message pump on Windows). The **full `dictate` loop is wired end-to-end** — hold hotkey → speak → release → transcribe → inject. Everything compiles and lints clean across every target; live capture/injection/hotkey are verified on the target machine (they need OS permissions a CI/sandbox can't grant — see the platform notes).

**Phase 2 — the Tauri desktop app is feature-complete.** `app/` is a Tauri 2 + React app over the core: a **system tray**, a **floating recording panel** (separate borderless window), and tabs for **Home** (7-day activity, top apps), **Status** (live Start/Stop + state badge), **Models** (download GGUF models with a progress bar, pick the active one), **Memory** (dictionary/snippet CRUD, applied during dictation), **History**, **Chat AI** (local Ollama), and editable **Settings**. The engine runs the real dictation loop and emits live `engine-state`/`transcript` events. Everything builds + lints clean; live dictation runs on Linux/Windows.

## Install

Grab the latest installer from the
[**Releases page**](https://github.com/theoyinbooke/orttaai-desktop/releases/latest),
or download **v0.1.0** directly:

| Platform | Download |
|---|---|
| **Linux** — AppImage (portable) | [`Orttaai_0.1.0_amd64.AppImage`](https://github.com/theoyinbooke/orttaai-desktop/releases/download/v0.1.0/Orttaai_0.1.0_amd64.AppImage) |
| **Linux** — Debian / Ubuntu | [`Orttaai_0.1.0_amd64.deb`](https://github.com/theoyinbooke/orttaai-desktop/releases/download/v0.1.0/Orttaai_0.1.0_amd64.deb) |
| **Linux** — Fedora / RHEL | [`Orttaai-0.1.0-1.x86_64.rpm`](https://github.com/theoyinbooke/orttaai-desktop/releases/download/v0.1.0/Orttaai-0.1.0-1.x86_64.rpm) |
| **Windows** — installer (.exe) | [`Orttaai_0.1.0_x64-setup.exe`](https://github.com/theoyinbooke/orttaai-desktop/releases/download/v0.1.0/Orttaai_0.1.0_x64-setup.exe) |
| **Windows** — MSI | [`Orttaai_0.1.0_x64_en-US.msi`](https://github.com/theoyinbooke/orttaai-desktop/releases/download/v0.1.0/Orttaai_0.1.0_x64_en-US.msi) |

```bash
# Linux — AppImage (no install needed)
chmod +x Orttaai_0.1.0_amd64.AppImage && ./Orttaai_0.1.0_amd64.AppImage

# Linux — Debian/Ubuntu
sudo apt install ./Orttaai_0.1.0_amd64.deb

# Linux — Fedora/RHEL
sudo dnf install ./Orttaai-0.1.0-1.x86_64.rpm
```

On **Windows**, run the `.exe` (or `.msi`) and follow the installer.

Then: open the **Models** tab → download a Whisper model → **Status → Start**, and
hold the push-to-talk chord (default **Ctrl+Shift+Space**) to dictate into any app.
The app self-updates via *Settings → Check for updates*.

> **Linux runtime notes:** dictation captures the mic (PipeWire/ALSA) and types via
> `wtype` (Wayland) or `xdotool` (X11) — install `wtype` on Wayland. Global hotkeys
> use X11/XWayland. See [`docs/gaps.md`](docs/gaps.md).

## Build

```bash
cargo build           # OS-agnostic core + CLI (mock backends)
cargo test            # unit tests
cargo run -p orttaai-cli -- demo   # run the dictation loop with mock backends
```

### Desktop app (Tauri)

```bash
cd app
npm install
npm run tauri dev     # launch the desktop window (tray + Status/History/Settings)
```

> Linux build deps for Tauri: `libwebkit2gtk-4.1-dev libgtk-3-dev
> libayatana-appindicator3-dev librsvg2-dev libxdo-dev`. `app/src-tauri` is its own
> Cargo workspace, so the heavy webview build stays out of the core/cli checks.

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

### Full dictation loop (the Phase-0 spike)

The real end-to-end loop, ready to run on Linux & Windows:

```bash
# needs ALSA + libxdo on Linux; a model on disk; mic + accessibility permissions
cargo run -p orttaai-cli --features full -- dictate models/ggml-tiny.en.bin
```

Hold the push-to-talk chord (default **Ctrl+Shift+Space**), speak, and release — the
transcript is typed into the focused window. This is the Phase-0 spike gate from the
build plan: *hold hotkey → speak → text appears*.

> **Wayland:** native global shortcuts go through the XDG portal and aren't wired yet;
> the hotkey path uses X11 (works under XWayland). Injection still uses `wtype` on
> Wayland. **macOS** (dev host only): the hotkey needs a main-thread run loop, so
> `dictate` is intended for the Linux/Windows targets.

## Platform support targets

| | Audio | Injection | Global hotkey | Notes |
|---|---|---|---|---|
| **Windows** | WASAPI (`cpal`) | `SendInput` (`enigo`) | `RegisterHotKey` | Easiest target — all solid |
| **Linux X11** | ALSA/Pipe (`cpal`) | `xdotool` | `XGrabKey` | Reliable |
| **Linux Wayland** | PipeWire (`cpal`) | `wtype` | XDG portal | Brittle hotkey; see gaps doc |

## Known OS-level gaps

Secure/password-field detection is unreliable on Linux, Wayland global hotkeys are brittle, and there is no Apple-Neural-Engine-class acceleration. See [`docs/gaps.md`](docs/gaps.md).

## Releases, installers & auto-updates

`.github/workflows/release.yml` builds signed installers for every platform via
[`tauri-action`](https://github.com/tauri-apps/tauri-action):

- **Linux:** `.AppImage`, `.deb`, `.rpm`
- **Windows:** `.exe` (NSIS) and `.msi`

**Cut a release:**

```bash
# 1. bump "version" in app/src-tauri/tauri.conf.json
# 2. tag and push
git tag v0.1.0 && git push origin v0.1.0
```

CI builds each platform, signs the updater artifacts, and opens a **draft** GitHub
Release with the installers + a `latest.json` manifest. Publish it to go live.

**One-time secret setup** (the updater signing key was generated with
`tauri signer generate`; the public key is committed in `tauri.conf.json`):

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | contents of the private key file (kept out of the repo) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the key password (empty if none) |

**Auto-updates:** the app's *Settings → Check for updates* uses
`@tauri-apps/plugin-updater`, fetching
`releases/latest/download/latest.json` and installing the signed bundle for the
current platform, then relaunching. The repository's Releases must be **public**
for the default endpoint to be reachable.

## License

[MIT](LICENSE)

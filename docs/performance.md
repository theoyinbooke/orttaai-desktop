# Transcription performance

Whisper runs on-device. Two levers: the **decode pipeline** (always on) and the
**compute backend** (build-time).

## Pipeline (shipped, CPU + GPU)

Tuned for push-to-talk dictation (short, independent utterances):

- **No temperature fallback** on the Fast/Balanced presets — whisper.cpp's default
  re-decodes a "hard" clip up to ~6×, silently multiplying latency. Accuracy preset
  keeps the fallback for tough audio.
- **`no_context` + `single_segment` + `no_timestamps` + `suppress_nst`** — skip work
  that's irrelevant to plain-text dictation.
- **Thread cap** (~performance-core count, default `min(8, logical)`): on hybrid CPUs
  (P + E cores) over-threading lets the slowest core gate every layer. Override in
  Settings → Performance.
- **Quantized (`q5`) models** in the catalog — ~2.4–2.9× smaller and faster than f16
  with minor accuracy loss. `large-v3-turbo` is the GPU sweet spot.

## GPU backends (opt-in build)

whisper-rs statically links one backend per binary (no runtime backend switching),
so GPU is a build choice:

| Build | Command (in `app/`) | Runtime |
|-------|---------|---------|
| CPU (default) | `cargo build` | runs everywhere |
| **CUDA** (NVIDIA) | `npx tauri build --features cuda` | needs the NVIDIA driver; peak throughput, ~5–15× on medium/large |
| Vulkan (cross-vendor) | _blocked — see below_ | NVIDIA + Intel + AMD, one binary |

Build prerequisites: all need CMake + a C/C++ toolchain + libclang. **CUDA** needs the
CUDA Toolkit 12.x (`nvcc`) at build and the NVIDIA driver at runtime.

> **Vulkan is currently blocked upstream.** whisper-rs 0.16.0 (the latest release) ships a
> `src/vulkan.rs` that imports ggml device-enumeration symbols (`ggml_backend_vk_get_device_*`)
> which the bundled whisper.cpp removed when ggml moved to the generic `ggml_backend_dev_*`
> API — so `--features vulkan` fails to compile. Re-enable it once whisper-rs ships a fix (or
> bumps its bundled whisper.cpp). Until then, **CUDA is the GPU path** (which fits the NVIDIA
> target hardware). The `vulkan` cargo feature is left defined for when that lands.

Distribution: ship the **CPU** build as the cross-platform default (it's tuned + quantized and
runs everywhere), and offer a separate **CUDA "NVIDIA"** download for max throughput.

> Note: GPU builds can't be validated in a headless/virtualized environment without a working
> GPU driver — verify on real hardware with `whisper-cli` / bench.

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

| Build | Command | Runtime |
|-------|---------|---------|
| CPU (default) | `cargo build` | runs everywhere |
| **Vulkan** (recommended) | `cargo build --features vulkan` | NVIDIA + Intel + AMD from one binary; falls back to CPU if no device |
| CUDA (NVIDIA only) | `cargo build --features cuda` | needs the NVIDIA driver at runtime; peak throughput |

Build prerequisites: all need CMake + a C/C++ toolchain + libclang. **Vulkan** needs
`libvulkan-dev` + shader tools (`glslc`/`shaderc`) on Linux, and the LunarG Vulkan
SDK (`VULKAN_SDK` set) on Windows. **CUDA** needs the CUDA Toolkit 12.x (`nvcc`).

Recommended distribution: ship **Vulkan** as the default for Linux + Windows, and
offer a separate **CUDA "NVIDIA"** download for users who want max throughput on
medium/large/turbo models.

> Note: GPU builds can't be validated in a headless/virtualized environment without
> a working GPU driver — verify on real hardware with `whisper-cli` / bench.

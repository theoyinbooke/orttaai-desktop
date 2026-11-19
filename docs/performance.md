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

> **Windows CUDA installer ships its runtime DLLs.** whisper.cpp's CUDA backend imports
> `cudart64_*`, `cublas64_*` and `cublasLt64_*` at load time, so on Windows those DLLs must sit
> next to the exe or the app won't launch on a machine without the CUDA Toolkit installed. The
> `-cuda.exe` (NSIS) produced by `release-cuda.yml` bundles them as resources and a post-install
> hook (`src-tauri/cuda-nsis-hooks.nsh`, wired via `src-tauri/tauri.cuda.conf.json`) copies them
> beside the exe. Only the NSIS `-cuda.exe` is shipped for GPU — the MSI has no equivalent hook.
> A local `npx tauri build --features cuda` does **not** bundle the DLLs; add
> `-c src-tauri/tauri.cuda.conf.json` (after staging the DLLs into `src-tauri/cuda-runtime/`) to
> get a redistributable Windows GPU installer.

> Note: GPU builds can't be validated in a headless/virtualized environment without a working
> GPU driver — verify on real hardware with `whisper-cli` / bench.

## Portable release builds (`GGML_NATIVE=OFF`)

ggml defaults to `GGML_NATIVE=ON`, which compiles whisper.cpp with `-march=native` —
optimizing for **the build machine's exact CPU**. Ideal for a local build, **fatal for a
distributed one**: GitHub's CI runners are AVX-512-capable Intel Xeons, so a default CI build
bakes in AVX-512 instructions. On the many consumer CPUs without AVX-512 (e.g. 12th/13th-gen
Intel Core, AMD Zen < 4 mobile) the binary crashes with an **illegal instruction (SIGILL)** the
moment whisper runs — even though a *locally*-built binary on the same machine works fine.
(See whisper.cpp issue #2928.)

The release workflow therefore sets `GGML_NATIVE: 'OFF'` (and, redundantly, `GGML_AVX512: 'OFF'`).
On a normal build, `GGML_NATIVE=OFF` enables an **AVX2 + FMA + F16C + BMI2** baseline and leaves
AVX-512 off — fast, and portable to every x86-64 CPU since ~2013 (Haswell / Zen 1).
`whisper-rs-sys`' build script forwards any `GGML_*` / `CMAKE_*` env var to CMake, so this is a
one-line env change in `.github/workflows/release.yml` — no code change.

> **Rule:** any CI/distribution build of whisper.cpp/ggml/llama.cpp must set `GGML_NATIVE=OFF`.
> Local dev builds can keep the default (native, slightly faster, targets your own CPU).

## CPU threads and the speed test (0.7)

`core/src/cpu.rs` sizes Whisper and Express to the physical performance cores
(Windows efficiency classes, Linux `/sys/devices/cpu_core`, macOS
`hw.perflevel0`), at least four where the CPU has four physical cores and at
most eight; Word Boost gets about a third of that, since it scores alongside
Express. On Windows the app opts out of EcoQoS (`ProcessPowerThrottling`) at
launch: dictation always decodes while another app is in front.

No rule fits every PC, so Settings › Model › Performance › **Speed Test**
(`core/src/speedtest.rs`, `app/src-tauri/src/speedtest.rs`) times 8 s of
deterministic sample speech with the model in use at 2, 4, 6 and 8 threads
(plus the fast and physical core counts), keeps the fastest (ties within 5 %
go to fewer threads) and saves it as Decode Threads. Express loads once per
count, since ONNX Runtime fixes a session's threads when it loads; Whisper
takes the count per decode.

## Graphics chips without CUDA (DirectML): not yet

Running Express on Intel and AMD graphics chips through DirectML was looked at
for 0.7 and set aside, for reasons that a code change can't fix:

- sherpa-onnx has DirectML only as a build-from-source option
  (`SHERPA_ONNX_ENABLE_DIRECTML`, `cmake/onnxruntime-win-x64-directml.cmake`);
  no release ships it, and its recipe pins ONNX Runtime DirectML **1.14.1**,
  which predates the `MatMulNBits` operator Fluid Speech Mini (2/4-bit) and
  Fluid Speech Pro (int8) are built from. Those models can't load on it.
- A current ONNX Runtime DirectML build would have to be built or fetched and
  shipped as DLLs (onnxruntime + DirectML, on the order of 100–200 MB), with
  sherpa-onnx built against it and Word Boost and Fluid Speech Mini (which
  call the ONNX Runtime C API directly) moved to the shared runtime.
- DirectML runs only some quantised `MatMulNBits` widths on the GPU; layers it
  can't run fall back to the CPU and copy data back and forth, which can be
  slower than the CPU alone. A real gain likely needs fp16 builds of the
  models (Fluid Speech Pro's fp32 ONNX is 2.67 GB; fp16 would be about half),
  a separate download.
- It can't be tested here: DirectML needs Windows with a DirectX 12 GPU, which
  neither Docker nor Wine provides.

The way forward, when there's an Intel PC to measure on: build a throwaway
Windows test binary that loads Fluid Speech Pro's fp32 encoder with ONNX
Runtime DirectML (current release, from NuGet) and times it against the CPU
int8 path on the same 8 s sample the speed test uses. Only if the GPU is
clearly faster is it worth the build and download work above; the speed test
would then also compare CPU and GPU and keep the faster.

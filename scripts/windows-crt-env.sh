# Source before building for Windows (MSVC or cargo-xwin): compile whisper.cpp
# with the static C runtime (/MT) so it links with sherpa-onnx (Express engine)
# and the +crt-static Rust code (.cargo/config.toml).
export CMAKE_C_FLAGS_RELEASE="/MT /O2 /Ob2 /DNDEBUG"
export CMAKE_CXX_FLAGS_RELEASE="/MT /O2 /Ob2 /DNDEBUG"
export CMAKE_C_FLAGS_RELWITHDEBINFO="/MT /O2 /Ob1 /DNDEBUG"
export CMAKE_CXX_FLAGS_RELWITHDEBINFO="/MT /O2 /Ob1 /DNDEBUG"
export CMAKE_C_FLAGS_DEBUG="/MT /Od"
export CMAKE_CXX_FLAGS_DEBUG="/MT /Od"
# CUDA builds: nvcc's host compiler must use /MT too.
export CMAKE_CUDA_FLAGS_RELEASE="-O3 -DNDEBUG -Xcompiler=/MT"
export CMAKE_CUDA_FLAGS_RELWITHDEBINFO="-O2 -DNDEBUG -Xcompiler=/MT"
# Cross builds from Linux or macOS (cargo-xwin) also need the static Universal
# CRT named, or expf/logf stay unresolved. Not for native MSVC builds: there it
# clashes with ucrt.lib. These replace .cargo/config.toml's rustflags, so they
# repeat +crt-static.
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -C link-arg=libucrt.lib"
export CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -C link-arg=libucrt.lib"

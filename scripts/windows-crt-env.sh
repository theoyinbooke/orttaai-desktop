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

# Dot-source before building on Windows: . .\scripts\windows-crt-env.ps1
# Compiles whisper.cpp with the static C runtime (/MT) so it links with
# sherpa-onnx (Express engine) and the +crt-static Rust code.
$env:CMAKE_C_FLAGS_RELEASE = "/MT /O2 /Ob2 /DNDEBUG"
$env:CMAKE_CXX_FLAGS_RELEASE = "/MT /O2 /Ob2 /DNDEBUG"
$env:CMAKE_C_FLAGS_RELWITHDEBINFO = "/MT /O2 /Ob1 /DNDEBUG"
$env:CMAKE_CXX_FLAGS_RELWITHDEBINFO = "/MT /O2 /Ob1 /DNDEBUG"
$env:CMAKE_C_FLAGS_DEBUG = "/MT /Od"
$env:CMAKE_CXX_FLAGS_DEBUG = "/MT /Od"

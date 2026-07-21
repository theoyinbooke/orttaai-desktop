; NSIS installer hooks for the CUDA (NVIDIA) build.
;
; The CUDA runtime DLLs (cudart64_*, cublas64_*, cublasLt64_*) are bundled as
; Tauri resources, which the installer places in $INSTDIR\resources\cuda-runtime.
; whisper.cpp's CUDA backend links these DLLs at load time, so Windows must find
; them in the executable's own directory ($INSTDIR). Copy them up there on
; install, and remove them on uninstall.

!macro NSIS_HOOK_POSTINSTALL
  CopyFiles /SILENT "$INSTDIR\resources\cuda-runtime\*.dll" "$INSTDIR"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\cudart64_*.dll"
  Delete "$INSTDIR\cublas64_*.dll"
  Delete "$INSTDIR\cublasLt64_*.dll"
!macroend

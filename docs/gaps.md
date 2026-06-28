# Known OS-level gaps

These are realities of the underlying operating systems, isolated behind the `TextInjector` / `HotkeyManager` traits. They are communicated in-product, not "fixed."

## 1. Secure / password-field detection — solved on Windows, not on Linux
**Windows:** detected via UI Automation (`IUIAutomationElement::CurrentIsPassword` on the focused element), so `is_secure_field_focused()` returns `Secure`/`NotSecure` and the coordinator blocks password fields automatically.
**Linux:** no reliable system-level equivalent (AT-SPI2 `ROLE_PASSWORD` is unreliable and unavailable to sandboxed apps; nothing at all on Wayland), so the trait returns `SecureFieldStatus::Unknown`.
**Product handling:** the `strict_secure` setting ("Never type into password fields") makes the coordinator refuse to type whenever the field can't be confirmed safe. It's off by default (on Linux every field is `Unknown`, so enabling it blocks all typing until you paste manually); on Windows it only affects the rare `Unknown` case since detection normally returns a definite answer.

## 2. Wayland global hotkey — brittle
The XDG GlobalShortcuts portal is patchy; some compositors (Sway, custom Hyprland) may not implement it. Windows (`RegisterHotKey`) and X11 (`XGrabKey`) are solid.
**Product handling:** detect registration failure → offer a manual trigger (tray/click) or "configure in your WM settings."

## 3. No Neural-Engine-class acceleration
macOS uses the Apple Neural Engine via CoreML. CPU-only transcription is ~2–4× slower; only NVIDIA CUDA (or maturing AMD ROCm) nears that speed.
**Product handling:** hardware detection sets expectations and recommends a model tier.

## Lesser deltas
- Rich clipboard loss on Wayland (text/plain only).
- Reduced floating-panel fidelity (no NSVisualEffectView blur / Spaces-awareness).
- Tray needs a user-installed extension on GNOME.
- No model migration from macOS (`.mlmodelc` → GGUF re-download; model IDs stay stable).
- Some X11 apps reject synthetic events (`XKeyEvent.synthetic`).

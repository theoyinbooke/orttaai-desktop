# Known OS-level gaps

These are realities of the underlying operating systems, isolated behind the `TextInjector` / `HotkeyManager` / `Clipboard` / `AudioCapture` traits. They are communicated in-product (Setup › System check, Settings hints, the overlay and toasts), not "fixed."

## 1. Secure / password-field detection — solved on Windows, not on Linux
**Windows:** detected via UI Automation (`IUIAutomationElement::CurrentIsPassword` on the focused element), so `is_secure_field_focused()` returns `Secure`/`NotSecure`; the coordinator blocks password fields automatically, and voice edit never reads a password field's selection.
**Linux:** no reliable system-level equivalent (AT-SPI2 `ROLE_PASSWORD` is unreliable and unavailable to sandboxed apps; nothing at all on Wayland), so the trait returns `SecureFieldStatus::Unknown`.
**Product handling:** "Never type into password fields" (`strict_secure`) makes the coordinator refuse to type — and voice edit refuse to read a selection — whenever the field can't be confirmed safe. It's off by default (on Linux every field is `Unknown`, so enabling it blocks all typing; those dictations are still saved to History and left on the clipboard). A field known to be a password keeps nothing, where the Mac still saves it to History.

## 2. Wayland global shortcuts — brittle
The XDG GlobalShortcuts portal is patchy; some compositors (Sway, custom Hyprland) don't implement it, and GNOME often ignores the preferred trigger so the user assigns the key in Settings › Keyboard. Both Orttaai shortcuts (push-to-talk and edit-selection) are bound in one portal session; changing a shortcut re-binds, which may show the desktop's dialog again. Windows (`RegisterHotKey`) and X11 (`XGrabKey`) are solid.
**Product handling:** registration failure never stops the engine — "Click to record" on Home, the tray item and the overlay's stop button drive the same hands-free recording. The portal's release events are backed by a watchdog so a dropped release can't strand a recording.

## 3. Floating overlay and focus on Linux
GNOME/Mutter does not reliably keep an always-on-top `focusable:false` window out of the focus chain, so a visible overlay could capture synthesized keystrokes. On Linux the pill is therefore hidden the moment recording ends — before processing/injection — so the "~Ns to process" estimate is shown only on Windows (where a `WS_EX_NOACTIVATE` window is never activated). Error pills (shown after delivery) appear on every platform. No NSVisualEffectView blur or Spaces awareness.

## 4. Typing, pasting and the clipboard
- **Wayland typing** goes through the RemoteDesktop portal (GNOME/KDE; the user approves once, a restore token avoids re-prompts) or `wtype` (wlroots). GNOME has no virtual-keyboard protocol, so `wtype` alone can't type there.
- **Paste mode** stages the text, sends Ctrl+V (Shift+Insert for terminals, where Ctrl+V is "literal next"; on Linux, Shift+Insert pastes the PRIMARY selection, so that is staged and restored too), then restores the clipboard after 250 ms. The Mac verifies the paste through Accessibility before restoring after 90 ms; desktop platforms have no such read-back, so the delay is longer to let apps that fetch the selection asynchronously read it.
- **Clipboard after a paste:** on Windows everything that was on the clipboard comes back (text, images, rich text, copied files). On Linux one kind can be restored: text when there was any, otherwise an image (a screenshot copy). On native Wayland the clipboard goes through `wl-copy`/`wl-paste` (`wl-clipboard`); without it, XWayland's clipboard is used, which native Wayland apps may not share.
- Some X11 apps reject synthetic events (`XKeyEvent.synthetic`); apps running as Administrator on Windows ignore input from non-elevated apps.

## 5. Voice edit selection capture
The Mac reads the selection through Accessibility first and falls back to Cmd+C. Linux/Windows have no portable selected-text API, so the selection is always captured with the copy chord into a saved-and-restored clipboard. It is read when the edit shortcut is **released** (after a short settle), not when it's pressed — a synthetic Ctrl+C sent while the user still holds Shift would become Ctrl+Shift+C (devtools in browsers). "Select text first" therefore appears at release rather than at press.

## 6. Learn from edits
Reading the focused field back after insertion uses UI Automation on Windows (`ValuePattern`, then `TextPattern`) and AT-SPI on Linux (X11 and Wayland): Orttaai asks the accessibility registry for focus events, remembers the editable object that last took focus, and reads it through its `Text` interface (password fields by role are never read). GTK, Qt, LibreOffice and Firefox fields are readable; apps that don't expose accessibility (some Electron and Chromium builds unless accessibility is switched on) report `unreadable`. Field identity is approximated by text overlap (a re-read that shares under half its words with the watched text counts as "moved to another field").

## 7. Audio
Devices are selected by their persistent name; a device that's gone falls back to the system default (Mac behaviour). Stream errors, stalls (1.5 s without audio) and streams that never start (800 ms) are restarted on the Mac's retry schedule, keeping every captured sample. There's no OS sleep/wake notification wired in, so after a wake the user's "Reset Audio" is what selects the longer "stale audio stack" start-up schedule. Coverage checks allow 300 ms of stream start-up latency (desktop audio stacks deliver the first buffer later than Core Audio).

## 8. No Neural-Engine-class acceleration
macOS uses the Apple Neural Engine via Core ML. CPU-only transcription is ~2–4× slower; only NVIDIA CUDA (the separate NVIDIA build) nears that speed.
**Product handling:** hardware detection (CPU, threads, RAM, `nvidia-smi`, free disk) sets a tier and recommends a model; a CPU build that finds an NVIDIA GPU suggests the CUDA download.

## 9. Voice activity detection
The Mac's Silero runs as a 256 ms streaming Core ML model. Here Silero v5 runs through whisper.cpp (`ggml-silero-v5.1.2.bin`, downloaded on first use and checksum-verified), which scores 32 ms windows and resets its state per call; Orttaai feeds ~1 s of context and calls a 256 ms chunk speech when ≥ 64 ms of it is voiced. Until the model is present (or if it fails mid-recording), hands-free auto-stop uses the Mac's adaptive-loudness policy.

## Window chrome
The main window draws its own title strip, as the Mac's does (an undecorated window with minimize, maximize and close at the top right; the strip and sidebar header drag the window). Setup uses a normal decorated window, like the Mac's "Orttaai Setup" window.

## Word Boost
Ported in full. As on the Mac, a second model (Parakeet CTC 110M, from `nvidia/parakeet-tdt_ctc-110m`) checks the Memory dictionary's words against the audio, and FluidAudio's rescorer plus the Mac's guard decide which replacements to keep. The model is the sherpa-onnx int8 ONNX export of the same checkpoint. It runs on the ONNX Runtime that sherpa-onnx already links, with NeMo's mel front end ported to Rust, and it downloads (104 MB) with every Express model. Long recordings are scored in the Mac's 15 s chunks while you speak. Word Boost is English only, like the Mac's.

## Formal and Casual styles
The Mac has no writing-style setting; this is a desktop addition. Formal and Casual are written by Polish, so they apply only while Polish is on. S1-mini only cleans up, so with S1-mini these styles are written by the general polish model, with S1-mini as the fallback.

## Target window on Wayland
If focus moves while a dictation is processed, Orttaai brings the original window back (X11, Windows) or waits up to 8 s for it, and otherwise leaves the text on the clipboard. Wayland tells clients nothing about windows, so there the text goes wherever focus is.

## Lesser deltas
- Tray needs a user-installed extension on GNOME.
- No model migration from macOS (`.mlmodelc` → GGUF re-download; model IDs stay stable).
- The custom models folder is entered as a path (no native folder picker is bundled).

## Not portable from macOS
- **iCloud sync** with the iPhone keyboard: CloudKit encrypted fields can't be read off
  Apple devices. Sync would need a Mac relay or an app-encrypted CloudKit Web Services field.
- **Apple Intelligence polish** and **CoreML coach voices** — replaced by S1-mini / local
  models and the WebView's speech voices.

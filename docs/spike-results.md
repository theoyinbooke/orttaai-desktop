# Phase 0 spike results

Record pass/fail of the end-to-end *hold hotkey → speak → text lands in focused field* test per environment. **Exit gate:** works on Windows **and** ≥3 of 4 Linux environments.

| Environment | Transcription (whisper-rs) | Audio (cpal) | Injection | Hotkey | E2E loop | Notes |
|---|---|---|---|---|---|---|
| Windows 11 | ⬜ | ⬜ | ⬜ SendInput | ⬜ RegisterHotKey | ⬜ | |
| GNOME 45+ (Wayland) | ⬜ | ⬜ | ⬜ wtype | ⬜ portal | ⬜ | |
| KDE Plasma 6.5+ (Wayland) | ⬜ | ⬜ | ⬜ wtype | ⬜ portal | ⬜ | |
| Sway / Hyprland | ⬜ | ⬜ | ⬜ wtype | ⬜ portal | ⬜ | hotkey risk |
| X11 (any DE) | ⬜ | ⬜ | ⬜ xdotool | ⬜ XGrabKey | ⬜ | |

**Decision:** ⬜ GO  ⬜ NO-GO  ⬜ scope to Windows + X11 first

_Legend: ✅ pass · ⚠️ partial · ❌ fail · ⬜ untested_

// Global-shortcut recorder (Mac ShortcutRecorderView). Click, press the chord,
// done. While listening, the app's global shortcuts are suspended so pressing
// the current shortcut records it instead of starting a dictation. Owned by the
// engine area.
import { useEffect, useRef, useState } from "react";
import { Button, Kbd } from "../ui";
import { call } from "../lib/api";
import { comboToString, type HotkeyCombo, type Modifier } from "../lib/settings";

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

/** `KeyboardEvent.code` → the key name the backend parses. */
function keyName(code: string): string {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  return code; // Space, F5, ArrowUp, Backquote, Enter, Tab…
}

export function comboFromEvent(e: KeyboardEvent): HotkeyCombo | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const modifiers: Modifier[] = [];
  if (e.ctrlKey) modifiers.push("ctrl");
  if (e.altKey) modifiers.push("alt");
  if (e.shiftKey) modifiers.push("shift");
  if (e.metaKey) modifiers.push("meta");
  return { key: keyName(e.code), modifiers };
}

export default function ShortcutRecorder({
  value,
  onChange,
  other,
  disabled,
  plain,
}: {
  value: HotkeyCombo;
  onChange: (c: HotkeyCombo) => void;
  /** The app's other shortcut, which this one must not collide with. */
  other?: HotkeyCombo | null;
  disabled?: boolean;
  /** The compact settings-row field: the chord as text, Escape or a click away cancels. */
  plain?: boolean;
}) {
  const [listening, setListening] = useState(false);
  const [live, setLive] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!listening) return;
    call("suspend_hotkeys", { suspended: true }).catch(() => {});
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
        setListening(false);
        return;
      }
      const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(
        Boolean,
      ) as string[];
      const combo = comboFromEvent(e);
      if (!combo) {
        setLive(mods.join("+"));
        return;
      }
      call("validate_shortcut", { combo, other: other ?? null })
        .then(() => {
          setError(null);
          setListening(false);
          onChange(combo);
        })
        .catch((err) => {
          setError(String(err));
          setLive("");
        });
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      call("suspend_hotkeys", { suspended: false }).catch(() => {});
      setLive("");
    };
  }, [listening]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="shortcut-recorder">
      <button
        ref={ref}
        type="button"
        className={`sr-field ${plain ? "sr-plain" : ""} ${listening ? "listening" : ""}`}
        onClick={() => {
          setError(null);
          setListening((l) => !l);
        }}
        onBlur={() => setListening(false)}
        disabled={disabled}
        aria-label={listening ? "Press the new shortcut" : `Shortcut ${comboToString(value)}. Click to change.`}
      >
        {listening ? (
          <span className="sr-prompt">{live ? `${live}+…` : plain ? "Press Shortcut" : "Press a shortcut…"}</span>
        ) : plain ? (
          comboToString(value)
        ) : (
          <Kbd combo={comboToString(value)} />
        )}
      </button>
      {listening && !plain && (
        <Button size="sm" variant="ghost" onMouseDown={(e) => e.preventDefault()} onClick={() => setListening(false)}>
          Cancel
        </Button>
      )}
      {error && <span className="sr-error small">{error}</span>}
    </div>
  );
}

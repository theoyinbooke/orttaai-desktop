// A dictation into one of Orttaai's own windows (Setup's Quick Test, Chat AI,
// Memory) arrives as text to insert at the caret, not as synthesized keys,
// which Windows doesn't reliably deliver to the app that sends them
// (engine.rs `OwnWindowFocus`).
import { listen } from "@tauri-apps/api/event";

/** Inserts `text` where the caret is, the way typing would, so React sees it. */
export function insertAtCaret(text: string): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && /^(text|search|url|email|)$/.test(el.type))) {
    if (el.readOnly || el.disabled) return false;
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    el.setRangeText(text, start, end, "end");
    // React tracks the value it last set; a native input event makes it
    // notice the new one.
    el.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  if (el.isContentEditable) {
    return document.execCommand("insertText", false, text);
  }
  return false;
}

export function listenForInsertedText() {
  listen<string>("insert-text", (e) => {
    insertAtCaret(e.payload);
  }).catch(() => {});
}

// Speaking a chat message (Mac ChatAIViewModel voice input). The engine records and
// transcribes; arming it hands the words back here instead of typing them, the same
// hand-off Coach uses for practice.
import { useCallback, useEffect, useRef, useState } from "react";
import { useEvent } from "../../lib/api";
import { coachApi, newToken, waitForCapture } from "../../lib/coach";
import { startEngine, useEngine } from "../../lib/engine";

export type VoicePhase = "off" | "starting" | "recording" | "processing";

const SHORTEST_RECORDING_MS = 500;
const ENGINE_START_LIMIT_MS = 60_000;
/** After the engine settles, how long a transcript may still take to arrive. */
const CAPTURE_GRACE_MS = 2500;

export function useChatVoice(onText: (text: string) => void, onError: (message: string) => void) {
  const { engine } = useEngine();
  const [phase, setPhase] = useState<VoicePhase>("off");
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const engineRef = useRef(engine);
  engineRef.current = engine;
  const phaseRef = useRef<VoicePhase>("off");
  phaseRef.current = phase;
  const capture = useRef<{ token: number; cancel: () => void } | null>(null);
  const handlers = useRef({ onText, onError });
  handlers.current = { onText, onError };

  const clearCapture = useCallback(() => {
    capture.current?.cancel();
    capture.current = null;
  }, []);

  const finishWith = useCallback(
    (message: string) => {
      clearCapture();
      coachApi.disarm().catch(() => {});
      setStartedAt(null);
      setPhase("off");
      handlers.current.onError(message);
    },
    [clearCapture],
  );

  /** Closes the microphone and drops what it heard. */
  const drop = useCallback(() => {
    clearCapture();
    coachApi
      .discard()
      .then(() => (engineRef.current === "recording" ? coachApi.toggleRecording() : coachApi.disarm()))
      .catch(() => {});
  }, [clearCapture]);

  const begin = useCallback(async () => {
    try {
      const state = () => engineRef.current;
      if (state() !== "idle") {
        setPhase("starting");
        if (state() === "off" || state() === "error") await startEngine();
        const deadline = Date.now() + ENGINE_START_LIMIT_MS;
        while (state() !== "idle") {
          if (Date.now() > deadline) throw new Error("the dictation engine isn't ready");
          await new Promise((r) => window.setTimeout(r, 150));
          if (phaseRef.current !== "starting") return;
        }
      }
      const token = newToken();
      const wait = waitForCapture(token);
      capture.current = { token, cancel: wait.cancel };
      wait.promise.then((c) => {
        if (capture.current?.token !== token) return;
        capture.current = null;
        setStartedAt(null);
        setPhase("off");
        const text = c.text.trim();
        if (text) handlers.current.onText(text);
        else handlers.current.onError("No speech was detected.");
      });
      await coachApi.arm(token, false);
      await coachApi.toggleRecording();
      setStartedAt(Date.now());
      setPhase("recording");
    } catch (e) {
      finishWith(`Microphone unavailable: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [finishWith]);

  const finish = useCallback(() => {
    if (phaseRef.current !== "recording") return;
    if (startedAt == null || Date.now() - startedAt < SHORTEST_RECORDING_MS) {
      drop();
      setStartedAt(null);
      setPhase("off");
      handlers.current.onError("Recording was too short.");
      return;
    }
    setStartedAt(null);
    setPhase("processing");
    coachApi.toggleRecording().catch((e) => finishWith(`Could not transcribe voice: ${String(e)}`));
  }, [drop, finishWith, startedAt]);

  const toggle = useCallback(() => {
    if (phaseRef.current === "recording") finish();
    else if (phaseRef.current === "off") begin();
  }, [begin, finish]);

  const cancel = useCallback(() => {
    if (phaseRef.current !== "recording") return;
    drop();
    setStartedAt(null);
    setPhase("off");
  }, [drop]);

  // Stopped with the dictation shortcut instead of here.
  useEffect(() => {
    if (phase === "recording" && engine === "processing") {
      setStartedAt(null);
      setPhase("processing");
    }
  }, [engine, phase]);

  // The engine finished without handing anything back.
  useEffect(() => {
    if (phase !== "processing" || engine !== "idle") return;
    const t = window.setTimeout(() => finishWith("No speech was detected."), CAPTURE_GRACE_MS);
    return () => window.clearTimeout(t);
  }, [engine, finishWith, phase]);

  useEvent<string>("engine-error", (message) => {
    if (phaseRef.current !== "off") finishWith(`Microphone unavailable: ${message}`);
  });

  // Leaving the page while recording drops the recording.
  useEffect(
    () => () => {
      if (phaseRef.current === "recording") drop();
      else if (capture.current) clearCapture();
      phaseRef.current = "off";
    },
    [clearCapture, drop],
  );

  return { phase, startedAt, toggle, finish, cancel };
}

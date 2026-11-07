// Live engine state shared by the shell, Home and the overlay.
import { createContext, useContext, useState, type ReactNode } from "react";
import { useEvent } from "./api";

export type EngineState = "off" | "loading" | "idle" | "recording" | "processing";

export const STATE_LABEL: Record<EngineState, string> = {
  off: "Off",
  loading: "Starting…",
  idle: "Ready",
  recording: "Recording",
  processing: "Transcribing",
};

const Ctx = createContext<{ engine: EngineState; level: number; lastTranscript: string }>({
  engine: "off",
  level: 0,
  lastTranscript: "",
});

export function EngineProvider({ children }: { children: ReactNode }) {
  const [engine, setEngine] = useState<EngineState>("off");
  const [level, setLevel] = useState(0);
  const [lastTranscript, setLast] = useState("");
  useEvent<string>("engine-state", (s) => setEngine(s as EngineState));
  useEvent<number>("audio-level", setLevel);
  useEvent<string>("transcript", setLast);
  return <Ctx.Provider value={{ engine, level, lastTranscript }}>{children}</Ctx.Provider>;
}

export const useEngine = () => useContext(Ctx);

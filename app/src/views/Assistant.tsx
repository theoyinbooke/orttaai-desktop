import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, EmptyState, PageHeader, Select, Spinner, useToast } from "../ui";

type Msg = { role: "user" | "ai"; text: string };

export default function Assistant() {
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<string[]>("ollama_models")
      .then((m) => {
        setModels(m);
        if (m[0]) setModel(m[0]);
      })
      .catch((e) => toast(String(e), "error"));
  }, [toast]);

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight, behavior: "smooth" });
  }, [messages, busy]);

  async function send() {
    const prompt = input.trim();
    if (!prompt || busy) return;
    setInput("");
    setMessages((m) => [...m, { role: "user", text: prompt }]);
    setBusy(true);
    try {
      const reply = await invoke<string>("ollama_chat", { prompt, model });
      setMessages((m) => [...m, { role: "ai", text: reply }]);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="view assistant">
      <PageHeader
        title="Assistant"
        desc="Chat with a local model via Ollama — nothing leaves your machine."
        actions={
          <Select
            value={model}
            onChange={setModel}
            placeholder="No models"
            ariaLabel="Ollama model"
            options={models.map((m) => ({ value: m, label: m }))}
          />
        }
      />

      <div className="chat-log" ref={logRef}>
        {messages.length === 0 && !busy ? (
          <EmptyState
            icon="assistant"
            title="Ask anything"
            desc="Responses come from your local Ollama. Start Ollama and pull a model to begin."
          />
        ) : (
          <>
            {messages.map((m, i) => (
              <div key={i} className={`msg msg-${m.role}`}>
                {m.text}
              </div>
            ))}
            {busy && (
              <div className="msg msg-ai msg-thinking">
                <Spinner size={14} /> thinking…
              </div>
            )}
          </>
        )}
      </div>

      <div className="chat-input">
        <input
          className="text-input"
          value={input}
          placeholder="Message your local model…"
          onChange={(e) => setInput(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && send()}
        />
        <Button variant="primary" icon="send" onClick={send} loading={busy}>
          Send
        </Button>
      </div>
    </div>
  );
}

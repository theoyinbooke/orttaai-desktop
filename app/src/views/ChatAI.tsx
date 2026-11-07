// Chat AI (Mac ChatAIView). Owned by the ai area.
// Saved conversations grouped by recency, a composer with model / reply depth /
// thinking / mode, streamed replies rendered as safe markdown, collapsible
// reasoning, sources from your own dictation, stop, and offline guidance.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button, ConfirmDialog, CopyButton, Icon, IconButton, Modal, Spinner, useToast } from "../ui";
import { useEvent } from "../lib/api";
import { useSettings } from "../lib/settings";
import {
  chat,
  latestThought,
  nameSupportsThinking,
  replyStatusLine,
  shortTimestamp,
  type ChatConversation,
  type ChatDeltaEvent,
  type ChatDocument,
  type ChatErrorEvent,
  type ChatGroup,
  type ChatInfo,
  type ChatMessage,
  type ChatMode,
  type ModelInfo,
  type ThinkingDepth,
  type TurnResult,
} from "../lib/ai";
import "../styles/ai.css";

// ============================================================ markdown ===

type Block =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "list"; ordered: boolean; items: { marker: string; text: string; depth: number }[] }
  | { kind: "quote"; text: string }
  | { kind: "code"; lang: string; code: string }
  | { kind: "table"; header: string[]; align: ("left" | "center" | "right")[]; rows: string[][] }
  | { kind: "rule" };

const LIST_RE = /^(\s*)([-*+•]|\d{1,3}[.)])\s+(.*)$/;
const splitRow = (line: string) =>
  line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
const isTableRule = (line: string) => /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(line);

/** Markdown as local models write it, split into blocks. Works on half-finished
 *  text too, so a streaming reply renders as it arrives. */
export function parseMarkdown(md: string): Block[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const flush = () => {
    const text = para.join("\n").trim();
    if (text) blocks.push({ kind: "paragraph", text });
    para = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const t = line.trim();
    if (t.startsWith("```") || t.startsWith("~~~")) {
      flush();
      const fence = t.slice(0, 3);
      const lang = t.slice(3).trim();
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence)) code.push(lines[i++]);
      i++;
      blocks.push({ kind: "code", lang, code: code.join("\n") });
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(t);
    if (heading) {
      flush();
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2].replace(/\s*#+$/, "") });
      i++;
      continue;
    }
    if (/^(\*\s*){3,}$|^(-\s*){3,}$|^(_\s*){3,}$/.test(t)) {
      flush();
      blocks.push({ kind: "rule" });
      i++;
      continue;
    }
    if (t.startsWith(">")) {
      flush();
      const q: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) q.push(lines[i++].trim().replace(/^>\s?/, ""));
      blocks.push({ kind: "quote", text: q.join("\n") });
      continue;
    }
    if (t.includes("|") && i + 1 < lines.length && isTableRule(lines[i + 1])) {
      flush();
      const header = splitRow(line);
      const align = splitRow(lines[i + 1]).map((c) =>
        c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : "left",
      ) as ("left" | "center" | "right")[];
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim()) rows.push(splitRow(lines[i++]));
      blocks.push({ kind: "table", header, align, rows });
      continue;
    }
    const li = LIST_RE.exec(line);
    if (li) {
      flush();
      const ordered = /\d/.test(li[2]);
      const items: { marker: string; text: string; depth: number }[] = [];
      while (i < lines.length) {
        const m = LIST_RE.exec(lines[i]);
        if (m) {
          items.push({ marker: m[2], text: m[3], depth: Math.min(4, Math.floor(m[1].replace(/\t/g, "  ").length / 2)) });
          i++;
        } else if (lines[i].trim() && /^\s{2,}/.test(lines[i]) && items.length) {
          items[items.length - 1].text += " " + lines[i].trim();
          i++;
        } else break;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    if (!t) {
      flush();
      i++;
      continue;
    }
    para.push(line);
    i++;
  }
  flush();
  return blocks;
}

const INLINE_RE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*|__[^_\n]+__)|(\*[^*\s][^*\n]*\*|_[^_\s][^_\n]*_)|(\[[^\]\n]+\]\([^)\s]+\))|(https?:\/\/[^\s<>()]+[^\s<>().,;:!?])/g;

function safeLink(url: string): string | null {
  return /^https?:\/\//i.test(url) ? url : null;
}

/** Inline styling: code, bold, italic, links (http/https only). Text only, never HTML. */
function inline(text: string, keyBase = ""): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let k = 0;
  for (const m of text.matchAll(INLINE_RE)) {
    const idx = m.index ?? 0;
    if (idx > last) out.push(text.slice(last, idx));
    const tok = m[0];
    const key = `${keyBase}-${k++}`;
    if (m[1]) out.push(<code key={key}>{tok.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={key}>{inline(tok.slice(2, -2), key)}</strong>);
    else if (m[3]) out.push(<em key={key}>{inline(tok.slice(1, -1), key)}</em>);
    else if (m[4]) {
      const close = tok.indexOf("](");
      const label = tok.slice(1, close);
      const href = safeLink(tok.slice(close + 2, -1));
      out.push(href ? <ExtLink key={key} href={href}>{inline(label, key)}</ExtLink> : label);
    } else if (m[5]) {
      out.push(<ExtLink key={key} href={tok}>{tok}</ExtLink>);
    }
    last = idx + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function ExtLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        openUrl(href).catch(() => {});
      }}
    >
      {children}
    </a>
  );
}

function withBreaks(text: string, key: string): ReactNode[] {
  return text.split("\n").flatMap((l, i) => (i === 0 ? inline(l, `${key}-${i}`) : [<br key={`${key}-br${i}`} />, ...inline(l, `${key}-${i}`)]));
}

export function Markdown({ text }: { text: string }) {
  const blocks = useMemo(() => parseMarkdown(text), [text]);
  return (
    <div className="md">
      {blocks.map((b, i) => {
        const key = `b${i}`;
        switch (b.kind) {
          case "heading": {
            const H = (`h${Math.min(6, b.level + 2)}`) as "h3" | "h4" | "h5" | "h6";
            return <H key={key}>{inline(b.text, key)}</H>;
          }
          case "paragraph":
            return <p key={key}>{withBreaks(b.text, key)}</p>;
          case "quote":
            return <blockquote key={key}>{withBreaks(b.text, key)}</blockquote>;
          case "rule":
            return <hr key={key} />;
          case "code":
            return (
              <div key={key} className="md-code">
                <div className="md-code-head">
                  <span className="mono small">{b.lang || "text"}</span>
                  <CopyButton text={b.code} compact />
                </div>
                <pre>
                  <code>{b.code}</code>
                </pre>
              </div>
            );
          case "table":
            return (
              <div key={key} className="md-table">
                <table>
                  <thead>
                    <tr>
                      {b.header.map((h, j) => (
                        <th key={j} style={{ textAlign: b.align[j] ?? "left" }}>
                          {inline(h, `${key}h${j}`)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {b.rows.map((r, ri) => (
                      <tr key={ri}>
                        {r.map((c, j) => (
                          <td key={j} style={{ textAlign: b.align[j] ?? "left" }}>
                            {inline(c, `${key}r${ri}c${j}`)}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          case "list":
            return (
              <ul key={key} className={`md-list ${b.ordered ? "ordered" : ""}`}>
                {b.items.map((it, j) => (
                  <li key={j} style={{ marginLeft: it.depth * 18 }}>
                    <span className="md-marker">{b.ordered ? it.marker.replace(")", ".") : "•"}</span>
                    <span>{inline(it.text, `${key}i${j}`)}</span>
                  </li>
                ))}
              </ul>
            );
        }
        return null;
      })}
    </div>
  );
}

// ============================================================== glyphs ===

function Glyph({ name, size = 15 }: { name: "chevron" | "clip" | "sidebar" | "brain" | "doc" | "more"; size?: number }) {
  const paths: Record<string, ReactNode> = {
    chevron: <path d="m6 9 6 6 6-6" />,
    clip: <path d="m21 11-8.6 8.6a5 5 0 0 1-7-7l8.5-8.6a3.3 3.3 0 0 1 4.7 4.7l-8.6 8.6a1.7 1.7 0 0 1-2.3-2.3l7.9-8" />,
    sidebar: (
      <>
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M9 4v16" />
      </>
    ),
    brain: (
      <path d="M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 6 1V5a2 2 0 0 0-3-1Zm6 0a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-6 1" />
    ),
    doc: <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5ZM14 3v5h5M9 13h6M9 17h4" />,
    more: <path d="M5 12h.01M12 12h.01M19 12h.01" strokeWidth={3} />,
  };
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.75} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

/** The Orttaai mark, beating gently while a reply is being written. */
function WaitingMark({ beating }: { beating: boolean }) {
  return (
    <svg className={`chat-mark ${beating ? "beating" : ""}`} viewBox="0 0 24 24" width={16} height={16} aria-hidden="true">
      <rect x="3" y="9" width="2.6" height="6" rx="1.3" />
      <rect x="8" y="5" width="2.6" height="14" rx="1.3" />
      <rect x="13.4" y="2.5" width="2.6" height="19" rx="1.3" />
      <rect x="18.8" y="8" width="2.6" height="8" rx="1.3" />
    </svg>
  );
}

// ============================================================ helpers ===

const DEPTHS: { value: ThinkingDepth; label: string; hint: string }[] = [
  { value: "low", label: "Low", hint: "Quick, concise answers" },
  { value: "medium", label: "Medium", hint: "Reasoned, polished answers" },
  { value: "high", label: "High", hint: "Deep structure, tone and audience" },
];

function greeting(): string {
  const h = new Date().getHours();
  if (h >= 5 && h < 12) return "Good morning";
  if (h >= 12 && h < 17) return "Good afternoon";
  if (h >= 17 && h < 22) return "Good evening";
  return "Late night";
}

type Starter = { title: string; detail: string; prompt: string; myTone?: boolean; sendNow?: boolean; icon: string };

function starters(hasTone: boolean): Starter[] {
  return [
    { title: "Polish a reply", detail: "Turn a rough idea into a clear message", prompt: "Draft a polished, concise reply for this situation:", icon: "edit" },
    {
      title: "Rewrite in my tone",
      detail: hasTone ? "Sounds like you, not a template" : "Builds on Tone of Voice in Analytics",
      prompt: "Rewrite this in my tone while keeping it clear and natural:",
      myTone: true,
      icon: "sparkle",
    },
    { title: "Search my dictation", detail: "Ask what you said about anything", prompt: "What have I said about ", icon: "search" },
    {
      title: "How do I write?",
      detail: "Patterns from your recent dictation",
      prompt: "Look at how I've been writing lately. What patterns, habits and favourite phrases stand out, and what could I tighten?",
      sendNow: true,
      icon: "insights",
    },
  ];
}

const lsGet = (k: string, d: string) => {
  try {
    return localStorage.getItem(k) ?? d;
  } catch {
    return d;
  }
};
const lsSet = (k: string, v: string) => {
  try {
    localStorage.setItem(k, v);
  } catch {
    /* storage unavailable */
  }
};

type Streaming = { conversationId: number; messageId: number; content: string; reasoning: string; startedAt: number };

// ================================================================ view ===

export default function ChatAI() {
  const toast = useToast();
  const { settings, patch } = useSettings();
  const [groups, setGroups] = useState<ChatGroup[]>([]);
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [historyVisible, setHistoryVisible] = useState(() => lsGet("chatAIHistoryVisible", "1") === "1");
  const [selected, setSelected] = useState<number | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [model, setModel] = useState("");
  const [depth, setDepth] = useState<ThinkingDepth>("medium");
  const [think, setThink] = useState(false);
  const [mode, setMode] = useState<ChatMode>("regular");
  const [documents, setDocuments] = useState<ChatDocument[]>([]);
  const [info, setInfo] = useState<ChatInfo | null>(null);
  const [offline, setOffline] = useState<string | null>(null);
  const [streaming, setStreaming] = useState<Streaming | null>(null);
  const [errorBanner, setErrorBanner] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<ChatConversation | null>(null);
  const [deleting, setDeleting] = useState<ChatConversation | null>(null);
  const [pasteOpen, setPasteOpen] = useState(false);
  const [menu, setMenu] = useState<null | "engine" | "mode">(null);
  const [now, setNow] = useState(Date.now());
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const selectedRef = useRef<number | null>(null);
  selectedRef.current = selected;

  const allConversations = useMemo(() => groups.flatMap((g) => g.conversations), [groups]);
  const current = allConversations.find((c) => c.id === selected) ?? null;

  const refreshList = useCallback(() => {
    chat.conversations(search).then(setGroups).catch(() => {});
  }, [search]);

  const refreshModels = useCallback(() => {
    chat
      .models()
      .then((m) => {
        setModels(m);
        setOffline(null);
        setModel((cur) => {
          if (cur && m.some((x) => x.name === cur)) return cur;
          const saved = settings?.chat_model?.trim();
          if (saved && m.some((x) => x.name === saved)) return saved;
          return m[0]?.name ?? cur;
        });
      })
      .catch((e) => {
        setModels([]);
        setOffline(String(e));
      });
  }, [settings?.chat_model]);

  useEffect(() => {
    chat
      .info()
      .then((i) => {
        setInfo(i);
        setModel((cur) => cur || i.default_model);
      })
      .catch(() => {});
    refreshModels();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings?.llm_provider, settings?.ollama_endpoint, settings?.lmstudio_endpoint]);

  useEffect(() => {
    if (settings?.chat_thinking) setDepth(settings.chat_thinking);
  }, [settings?.chat_thinking]);

  useEffect(() => {
    const t = window.setTimeout(refreshList, search ? 180 : 0);
    return () => window.clearTimeout(t);
  }, [refreshList, search]);

  useEffect(() => {
    if (selected == null) {
      setMessages([]);
      return;
    }
    chat.messages(selected).then(setMessages).catch(() => setMessages([]));
  }, [selected]);

  useEffect(() => {
    if (!streaming) return;
    const t = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(t);
  }, [streaming]);

  // Keep the newest words in view.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, streaming?.content, streaming?.reasoning]);

  useEvent<ChatDeltaEvent>("chat-delta", (d) => {
    setStreaming((s) =>
      s && s.messageId === d.message_id ? { ...s, content: d.content, reasoning: d.reasoning } : s,
    );
  });
  useEvent<TurnResult>("chat-done", (r) => {
    setStreaming((s) => (s && s.messageId === r.assistant_message_id ? null : s));
    if (r.error) setErrorBanner(r.error);
    if (selectedRef.current === r.conversation_id) {
      chat.messages(r.conversation_id).then(setMessages).catch(() => {});
    }
    refreshList();
    chat.info().then(setInfo).catch(() => {});
  });
  useEvent<ChatErrorEvent>("chat-error", (e) => {
    setStreaming((s) => (s && s.messageId === e.message_id ? null : s));
    setErrorBanner(e.error);
    refreshList();
  });
  useEvent("ai-models-changed", () => refreshModels());

  const modelInfo = models?.find((m) => m.name === model);
  const isOllama = settings?.llm_provider !== "lm_studio";
  const supportsThinking = isOllama && (modelInfo?.thinking ?? nameSupportsThinking(model));
  const thinkingOn = supportsThinking && think;
  const busy = streaming != null;
  const hasTone = info?.has_tone_profile ?? false;
  const providerName = info?.provider_name ?? (isOllama ? "Ollama" : "LM Studio");

  const toggleHistory = () => {
    setHistoryVisible((v) => {
      lsSet("chatAIHistoryVisible", v ? "0" : "1");
      return !v;
    });
  };

  const newChat = () => {
    setSelected(null);
    setDraft("");
    setErrorBanner(null);
    setDocuments([]);
    inputRef.current?.focus();
  };

  const send = async (text?: string, forceMode?: ChatMode) => {
    const prompt = (text ?? draft).trim();
    if (!prompt || busy) return;
    setDraft("");
    setErrorBanner(null);
    const useMode = forceMode ?? mode;
    try {
      const turn = await chat.send({
        conversation_id: selected,
        prompt,
        model,
        depth,
        think: thinkingOn,
        mode: useMode,
        documents,
      });
      setStreaming({
        conversationId: turn.conversation_id,
        messageId: turn.assistant_message_id,
        content: "",
        reasoning: "",
        startedAt: Date.now(),
      });
      if (selected !== turn.conversation_id) {
        setSelected(turn.conversation_id);
      }
      setMessages((ms) => (selected === turn.conversation_id ? [...ms, turn.user_message] : [turn.user_message]));
      refreshList();
      if (model && model !== settings?.chat_model) patch({ chat_model: model }).catch(() => {});
    } catch (e) {
      setDraft(prompt);
      setErrorBanner(String(e));
    }
  };

  const stop = () => {
    if (streaming) chat.stop(streaming.conversationId).catch(() => {});
  };

  const pickStarter = (s: Starter) => {
    if (s.myTone) setMode("my_tone");
    if (s.sendNow) {
      send(s.prompt, s.myTone ? "my_tone" : undefined);
      return;
    }
    setDraft(s.prompt);
    window.setTimeout(() => {
      const el = inputRef.current;
      if (el) {
        el.focus();
        el.setSelectionRange(el.value.length, el.value.length);
      }
    }, 0);
  };

  const attachFiles = async (files: FileList | null) => {
    if (!files) return;
    const added: ChatDocument[] = [];
    for (const f of Array.from(files)) {
      try {
        const text = (await f.text()).trim();
        if (text) added.push({ name: f.name, content: text.slice(0, 120_000) });
      } catch {
        /* unreadable */
      }
    }
    if (!added.length) {
      toast("No readable text was found in the selected file.", "warn");
      return;
    }
    setDocuments((d) => {
      const next = [...d, ...added];
      toast(`${next.length} document${next.length === 1 ? "" : "s"} attached.`, "success");
      return next;
    });
  };

  // Auto-grow the composer.
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(220, el.scrollHeight)}px`;
  }, [draft]);

  const streamingHere = streaming && streaming.conversationId === selected ? streaming : null;
  const showEmpty = selected == null || (messages.length === 0 && !streamingHere);
  const recents = allConversations.filter((c) => c.id !== selected).slice(0, 3);

  return (
    <div className={`chat ${historyVisible ? "with-history" : ""}`}>
      {historyVisible && (
        <aside className="chat-history" aria-label="Chat history">
          <div className="chat-history-head">
            <span className="chat-history-title">Chats</span>
            <div className="chat-history-actions">
              <IconButton name="search" label={searchOpen ? "Close search" : "Search chats"} onClick={() => {
                setSearchOpen((o) => !o);
                if (searchOpen) setSearch("");
              }} />
              <IconButton name="plus" label="New chat" onClick={newChat} />
            </div>
          </div>
          {searchOpen && (
            <div className="chat-search">
              <input
                className="text-input"
                placeholder="Search chats"
                value={search}
                autoFocus
                onChange={(e) => setSearch(e.target.value)}
                onKeyDown={(e) => e.key === "Escape" && (setSearch(""), setSearchOpen(false))}
              />
            </div>
          )}
          <div className="chat-history-list">
            {groups.length === 0 && (
              <p className="muted small chat-history-empty">
                {search ? `No chats match “${search}”` : "No chats yet"}
              </p>
            )}
            {groups.map((g) => (
              <div key={g.title} className="chat-group">
                <div className="overline chat-group-title">{g.title}</div>
                {g.conversations.map((c) => (
                  <HistoryRow
                    key={c.id}
                    c={c}
                    active={c.id === selected}
                    live={info?.inflight.includes(c.id) || streaming?.conversationId === c.id}
                    onOpen={() => {
                      setSelected(c.id);
                      setErrorBanner(null);
                    }}
                    onRename={() => setRenaming(c)}
                    onDelete={() => setDeleting(c)}
                  />
                ))}
              </div>
            ))}
          </div>
        </aside>
      )}

      <section className="chat-main">
        <header className="chat-head">
          <button className="icon-btn" onClick={toggleHistory} aria-label={historyVisible ? "Hide chat history" : "Show chat history"} title={historyVisible ? "Hide chat history" : "Show chat history"}>
            <Glyph name="sidebar" size={18} />
          </button>
          <h1 className="chat-title">{current?.title ?? "ChatAI"}</h1>
          <IconButton name="plus" label="New chat" onClick={newChat} />
        </header>

        <div className="chat-scroll" ref={scrollRef}>
          {offline && (
            <div className="chat-offline card">
              <Icon name="alert" size={18} />
              <div>
                <strong>{providerName} isn't reachable{info ? ` at ${info.endpoint}` : ""}.</strong>
                <p className="muted small">
                  Start {providerName}, then refresh models.{" "}
                  {isOllama
                    ? "Install Ollama from ollama.com and run `ollama serve`, then pull a chat model such as gemma4:e2b."
                    : "Start the server from LM Studio's Developer tab, or run `lms server start`, then download a chat model."}
                </p>
                <div className="chat-offline-actions">
                  <Button size="sm" icon="refresh" onClick={refreshModels}>
                    Refresh Models
                  </Button>
                  <Button size="sm" icon="settings" onClick={() => emit("navigate", "settings")}>
                    AI Settings…
                  </Button>
                </div>
              </div>
            </div>
          )}
          {!offline && models && models.length === 0 && (
            <div className="chat-offline card">
              <Icon name="info" size={18} />
              <div>
                <strong>{providerName} is reachable, but no local chat models were found.</strong>
                <p className="muted small">Download one in Settings → AI Features (gemma4:e2b is a good start).</p>
                <div className="chat-offline-actions">
                  <Button size="sm" icon="settings" onClick={() => emit("navigate", "settings")}>
                    AI Settings…
                  </Button>
                </div>
              </div>
            </div>
          )}

          {showEmpty ? (
            <div className="chat-empty">
              <WaitingMark beating={false} />
              <h2>{greeting()}</h2>
              <p className="muted">
                {hasTone
                  ? "Draft, rewrite in your own voice, or ask how you write. Everything stays on this computer."
                  : "Draft, rewrite, or ask about how you write. Everything stays on this computer."}
              </p>
              <div className="chat-starters">
                {starters(hasTone).map((s) => (
                  <button key={s.title} className="chat-starter" onClick={() => pickStarter(s)} title={s.detail}>
                    <Icon name={s.icon} size={16} />
                    <span className="chat-starter-text">
                      <span className="chat-starter-title">{s.title}</span>
                      <span className="chat-starter-detail">{s.detail}</span>
                    </span>
                  </button>
                ))}
              </div>
              {recents.length > 0 && (
                <div className="chat-recents">
                  <div className="overline">Recent</div>
                  {recents.map((c) => (
                    <button key={c.id} className="chat-recent" onClick={() => setSelected(c.id)}>
                      <span className="chat-recent-title">{c.title}</span>
                      <span className="muted small">{shortTimestamp(c.updated_at)}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ) : (
            <div className="chat-messages">
              {messages.map((m) => (
                <MessageBubble key={m.id} m={m} />
              ))}
              {streamingHere && (
                <div className="chat-msg assistant">
                  <div className="chat-msg-head">
                    <WaitingMark beating />
                    <span className="chat-status" key={streamingHere.content ? "chatai" : "status"}>
                      {streamingHere.content
                        ? "ChatAI"
                        : replyStatusLine((now - streamingHere.startedAt) / 1000, thinkingOn, streamingHere.reasoning || null)}
                    </span>
                  </div>
                  {streamingHere.reasoning && (
                    <Reasoning text={streamingHere.reasoning} live={!streamingHere.content} />
                  )}
                  {streamingHere.content && (
                    <div className="chat-msg-body">
                      <Markdown text={streamingHere.content} />
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="chat-composer-wrap">
          <div className="chat-context">
            <span className="chat-context-item" title="Replies draw on your dictation history and recent work">
              <Icon name="dictionary" size={13} /> Your dictation
            </span>
            <span className="chat-context-item" title="Runs on your own hardware">
              <Icon name="monitor" size={13} /> {providerName} · This computer
            </span>
            {documents.length > 0 && (
              <span className="chat-context-item">
                <Glyph name="clip" size={13} /> {documents.length === 1 ? "1 file" : `${documents.length} files`}
              </span>
            )}
            <span className="chat-context-spacer" />
            <button className="chat-context-btn" onClick={refreshModels} title="Refresh models">
              <Icon name="refresh" size={13} />
            </button>
            <button className="chat-context-btn" onClick={() => emit("navigate", "settings")} title="AI Settings…">
              <Icon name="settings" size={13} />
            </button>
          </div>
          <div className="chat-composer">
            {documents.length > 0 && (
              <div className="chat-docs">
                {documents.map((d, i) => (
                  <span key={`${d.name}-${i}`} className="chat-doc">
                    <Glyph name="doc" size={13} />
                    <span className="chat-doc-name">{d.name}</span>
                    <span className="muted small">{d.content.split(/\s+/).filter(Boolean).length.toLocaleString()} words</span>
                    <button aria-label={`Remove ${d.name}`} onClick={() => setDocuments((ds) => ds.filter((_, j) => j !== i))}>
                      <Icon name="x" size={12} />
                    </button>
                  </span>
                ))}
              </div>
            )}
            <textarea
              ref={inputRef}
              className="chat-input"
              rows={1}
              value={draft}
              placeholder="Ask about your dictation, or ask for a draft"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  send();
                }
              }}
            />
            <div className="chat-composer-bar">
              <div className="chat-composer-left">
                <button className="chat-chip icon-only" title="Attach documents" onClick={() => fileRef.current?.click()}>
                  <Glyph name="clip" />
                </button>
                <button className="chat-chip icon-only" title="Paste a document" onClick={() => setPasteOpen(true)}>
                  <Glyph name="doc" />
                </button>
                <input
                  ref={fileRef}
                  type="file"
                  multiple
                  hidden
                  accept=".txt,.md,.markdown,.csv,.tsv,.json,.log,.html,.xml,.yaml,.yml,.rtf,.tex,.srt,.vtt,text/*"
                  onChange={(e) => {
                    attachFiles(e.target.files);
                    e.target.value = "";
                  }}
                />
                <div className="chat-menu-anchor">
                  <button
                    className="chat-chip"
                    onClick={() => setMenu(menu === "mode" ? null : "mode")}
                    title={hasTone ? "Chat mode" : "Chat mode. My Tone needs a Tone of Voice profile from Analytics."}
                  >
                    <Icon name={mode === "my_tone" ? "sparkle" : "assistant"} size={14} />
                    {mode === "my_tone" ? "My Tone" : "Regular"}
                    <Glyph name="chevron" size={11} />
                  </button>
                  {menu === "mode" && (
                    <Popover onClose={() => setMenu(null)}>
                      {(["regular", "my_tone"] as ChatMode[]).map((m) => (
                        <button key={m} className={`chat-pop-item ${mode === m ? "on" : ""}`} onClick={() => (setMode(m), setMenu(null))}>
                          <span>{m === "regular" ? "Regular" : "My Tone"}</span>
                          <span className="muted small">
                            {m === "regular" ? "A clear assistant voice" : hasTone ? "Writes like you" : "No tone profile yet: run Tone of Voice"}
                          </span>
                        </button>
                      ))}
                    </Popover>
                  )}
                </div>
              </div>
              <div className="chat-composer-right">
                <div className="chat-menu-anchor">
                  <button className="chat-chip" onClick={() => setMenu(menu === "engine" ? null : "engine")} title="Model, reply depth and thinking">
                    {thinkingOn ? <Glyph name="brain" size={14} /> : <Icon name="bolt" size={14} />}
                    <span className="chat-chip-model">{model || "Choose model"}</span>
                    <span className="muted">{DEPTHS.find((d) => d.value === depth)?.label}</span>
                    <Glyph name="chevron" size={11} />
                  </button>
                  {menu === "engine" && (
                    <Popover onClose={() => setMenu(null)} align="right">
                      <div className="chat-pop-section">Model</div>
                      {models && models.length === 0 && <div className="chat-pop-note muted small">No chat models found</div>}
                      {models === null && <div className="chat-pop-note"><Spinner /></div>}
                      {models?.map((m) => (
                        <button key={m.name} className={`chat-pop-item ${m.name === model ? "on" : ""}`} onClick={() => setModel(m.name)}>
                          <span>{m.name}</span>
                          {m.name === model && <Icon name="check" size={14} />}
                        </button>
                      ))}
                      <div className="chat-pop-section">Reply depth</div>
                      {DEPTHS.map((d) => (
                        <button key={d.value} className={`chat-pop-item ${d.value === depth ? "on" : ""}`} onClick={() => setDepth(d.value)}>
                          <span>{d.label}</span>
                          <span className="muted small">{d.hint}</span>
                        </button>
                      ))}
                      {supportsThinking && (
                        <>
                          <div className="chat-pop-divider" />
                          <label className="chat-pop-toggle">
                            <span>Thinking</span>
                            <input type="checkbox" checked={think} onChange={(e) => setThink(e.target.checked)} />
                          </label>
                        </>
                      )}
                    </Popover>
                  )}
                </div>
                {busy ? (
                  <button className="chat-send stop" onClick={stop} aria-label="Stop" title="Stop generating">
                    <Icon name="stop" size={16} />
                  </button>
                ) : (
                  <button className="chat-send" disabled={!draft.trim()} onClick={() => send()} aria-label="Send" title="Send (Enter). Shift-Enter adds a new line.">
                    <Icon name="send" size={16} />
                  </button>
                )}
              </div>
            </div>
          </div>
          {errorBanner && (
            <div className="chat-error" role="alert">
              <Icon name="alert" size={14} />
              <span>{errorBanner}</span>
              <button aria-label="Dismiss" onClick={() => setErrorBanner(null)}>
                <Icon name="x" size={13} />
              </button>
            </div>
          )}
        </div>
      </section>

      <RenameModal
        conversation={renaming}
        onClose={() => setRenaming(null)}
        onSave={(title) => {
          if (!renaming) return;
          chat
            .rename(renaming.id, title)
            .then(refreshList)
            .catch((e) => toast(String(e), "error"));
          setRenaming(null);
        }}
      />
      <ConfirmDialog
        open={deleting != null}
        title="Delete this chat?"
        body={`“${deleting?.title ?? ""}” and its messages will be removed. This can't be undone.`}
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          const id = deleting?.id;
          setDeleting(null);
          if (id == null) return;
          chat
            .remove(id)
            .then(() => {
              if (selected === id) newChat();
              refreshList();
            })
            .catch((e) => toast(String(e), "error"));
        }}
      />
      <PasteModal
        open={pasteOpen}
        onClose={() => setPasteOpen(false)}
        onAdd={(doc) => {
          setDocuments((d) => [...d, doc]);
          setPasteOpen(false);
        }}
      />
    </div>
  );
}

// ========================================================= components ===

function HistoryRow({
  c,
  active,
  live,
  onOpen,
  onRename,
  onDelete,
}: {
  c: ChatConversation;
  active: boolean;
  live?: boolean;
  onOpen: () => void;
  onRename: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`chat-row ${active ? "active" : ""}`}>
      <button className="chat-row-main" onClick={onOpen} title={c.title}>
        <span className="chat-row-title">{c.title}</span>
        <span className="chat-row-time">{live ? <Spinner size={11} /> : shortTimestamp(c.updated_at)}</span>
      </button>
      <span className="chat-row-actions">
        <button aria-label="Rename chat" title="Rename" onClick={onRename}>
          <Icon name="edit" size={13} />
        </button>
        <button aria-label="Delete chat" title="Delete" onClick={onDelete}>
          <Icon name="trash" size={13} />
        </button>
      </span>
    </div>
  );
}

function MessageBubble({ m }: { m: ChatMessage }) {
  if (m.role === "user") {
    return (
      <div className="chat-msg user">
        <div className="chat-bubble">{m.content}</div>
      </div>
    );
  }
  return (
    <div className="chat-msg assistant">
      <div className="chat-msg-head">
        <WaitingMark beating={false} />
        <span className="chat-status">ChatAI</span>
      </div>
      {m.reasoning && <Reasoning text={m.reasoning} live={false} />}
      <div className="chat-msg-body">
        <Markdown text={m.content} />
      </div>
      <div className="chat-msg-foot">
        <CopyButton text={m.content} compact />
        {m.sources.length > 0 && <Sources sources={m.sources} />}
      </div>
    </div>
  );
}

function Reasoning({ text, live }: { text: string; live: boolean }) {
  const [open, setOpen] = useState(false);
  const thought = latestThought(text);
  return (
    <div className={`chat-reasoning ${open ? "open" : ""}`}>
      <button className="chat-reasoning-toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Glyph name="brain" size={13} />
        <span>{live ? thought ?? "Thinking" : "Reasoning"}</span>
        <span className="chat-reasoning-chev">
          <Glyph name="chevron" size={11} />
        </span>
      </button>
      {open && <div className="chat-reasoning-body">{text.trim()}</div>}
    </div>
  );
}

function Sources({ sources }: { sources: ChatMessage["sources"] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="chat-sources">
      <button className="chat-sources-toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Icon name="dictionary" size={13} />
        {sources.length} from your dictation
        <Glyph name="chevron" size={11} />
      </button>
      {open && (
        <ol className="chat-sources-list">
          {sources.map((s, i) => (
            <li key={`${s.transcription_id}-${i}`}>
              <div className="chat-source-meta">
                <span>{s.app || "Unknown App"}</span>
                <span>·</span>
                <span>{new Date(s.created_at * 1000).toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" })}</span>
                <span>·</span>
                <span>{Math.round(s.score * 100)}% match</span>
              </div>
              <p>{s.text.length > 320 ? s.text.slice(0, 319) + "…" : s.text}</p>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

function Popover({ children, onClose, align = "left" }: { children: ReactNode; onClose: () => void; align?: "left" | "right" }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  return (
    <div ref={ref} className={`chat-pop ${align}`} role="menu">
      {children}
    </div>
  );
}

function RenameModal({
  conversation,
  onClose,
  onSave,
}: {
  conversation: ChatConversation | null;
  onClose: () => void;
  onSave: (title: string) => void;
}) {
  const [title, setTitle] = useState("");
  useEffect(() => setTitle(conversation?.title ?? ""), [conversation]);
  return (
    <Modal open={conversation != null} onClose={onClose} labelledBy="rename-chat">
      <form
        className="ai-modal"
        onSubmit={(e) => {
          e.preventDefault();
          if (title.trim()) onSave(title.trim());
        }}
      >
        <h2 id="rename-chat">Rename chat</h2>
        <input className="text-input" value={title} autoFocus placeholder="Title" onChange={(e) => setTitle(e.target.value)} />
        <div className="ai-modal-actions">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!title.trim()}>
            Rename
          </Button>
        </div>
      </form>
    </Modal>
  );
}

function PasteModal({ open, onClose, onAdd }: { open: boolean; onClose: () => void; onAdd: (d: ChatDocument) => void }) {
  const [name, setName] = useState("");
  const [content, setContent] = useState("");
  useEffect(() => {
    if (open) {
      setName("");
      setContent("");
    }
  }, [open]);
  return (
    <Modal open={open} onClose={onClose} labelledBy="paste-doc">
      <form
        className="ai-modal wide"
        onSubmit={(e) => {
          e.preventDefault();
          if (content.trim()) onAdd({ name: name.trim() || "Pasted text", content: content.trim().slice(0, 120_000) });
        }}
      >
        <h2 id="paste-doc">Add a document</h2>
        <p className="muted small">ChatAI reads the parts that match your question. It stays on this computer.</p>
        <input className="text-input" value={name} placeholder="Name (optional)" onChange={(e) => setName(e.target.value)} />
        <textarea className="text-input ai-paste" value={content} autoFocus placeholder="Paste text here" onChange={(e) => setContent(e.target.value)} />
        <div className="ai-modal-actions">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!content.trim()}>
            Add
          </Button>
        </div>
      </form>
    </Modal>
  );
}


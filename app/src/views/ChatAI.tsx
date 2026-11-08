// Chat AI (Mac ChatAIView, ChatAIMarkdown, ChatAIReplyStatus). Owned by the ai area.
// A floating history panel, a start page with the composer and starter pills, and a
// conversation that scrolls on under the composer. Replies render as safe markdown.
// Kept beyond the Mac: Stop, folded reasoning, the dictation a reply drew on, Paste a
// Document (in the chat settings menu), and a spinner on a chat still being answered.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button, ConfirmDialog, Modal, PageHeader, Spinner, useToast } from "../ui";
import { Glyph, SignalMark, type GlyphName } from "../components/chat/Glyph";
import { useChatVoice, type VoicePhase } from "../components/chat/useChatVoice";
import { takePendingChatPrefill, useEvent } from "../lib/api";
import { useEngine } from "../lib/engine";
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
        const depth = m ? Math.min(3, Math.floor(m[1].replace(/\t/g, "    ").length / 2)) : 0;
        if (m && (/\d/.test(m[2]) === ordered || depth > 0)) {
          items.push({ marker: m[2], text: m[3], depth });
          i++;
        } else if (m) break;
        else if (lines[i].trim() && /^\s{2,}/.test(lines[i]) && items.length) {
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

/** Language and Copy sit on a code block's header line. */
function CopyCode({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (
    <button
      className={`md-code-copy ${copied ? "copied" : ""}`}
      onClick={() => {
        navigator.clipboard?.writeText(code);
        setCopied(true);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => setCopied(false), 1500);
      }}
    >
      <Glyph name={copied ? "check" : "copy"} size={12} />
      {copied ? "Copied" : "Copy"}
    </button>
  );
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
            return (
              <H key={key} className={`md-h${Math.min(4, b.level)}`}>
                {inline(b.text, key)}
              </H>
            );
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
                  <span>{b.lang || "code"}</span>
                  <CopyCode code={b.code} />
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
              <ul key={key} className="md-list">
                {b.items.map((it, j) => (
                  <li key={j} style={{ paddingLeft: it.depth * 18 }}>
                    {/\d/.test(it.marker) ? (
                      <span className="md-marker number">{it.marker.replace(")", ".")}</span>
                    ) : (
                      <span className="md-marker">{it.depth === 0 ? "•" : "◦"}</span>
                    )}
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

// ============================================================== marks ===

/** The Orttaai mark, beating gently while a reply is being written. */
function WaitingMark({ beating }: { beating: boolean }) {
  return <SignalMark height={15} className={`chat-mark ${beating ? "beating" : ""}`} />;
}

// ============================================================ helpers ===

const DEPTHS: { value: ThinkingDepth; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
];

function greeting(): string {
  const h = new Date().getHours();
  if (h >= 5 && h < 12) return "Good morning";
  if (h >= 12 && h < 17) return "Good afternoon";
  if (h >= 17 && h < 22) return "Good evening";
  return "Late night";
}

type Starter = { title: string; detail: string; prompt: string; myTone?: boolean; sendNow?: boolean; icon: GlyphName };

function starters(hasTone: boolean): Starter[] {
  return [
    { title: "Polish a reply", detail: "Turn a rough idea into a clear message", prompt: "Draft a polished, concise reply for this situation:", icon: "pencilLine" },
    {
      title: "Rewrite in my tone",
      detail: hasTone ? "Sounds like you, not a template" : "Builds on Tone of Voice in Analytics",
      prompt: "Rewrite this in my tone while keeping it clear and natural:",
      myTone: true,
      icon: "personWave",
    },
    { title: "Search my dictation", detail: "Ask what you said about anything", prompt: "What have I said about ", icon: "search" },
    {
      title: "How do I write?",
      detail: "Patterns from your recent dictation",
      prompt: "Look at how I've been writing lately. What patterns, habits and favourite phrases stand out, and what could I tighten?",
      sendNow: true,
      icon: "textSearch",
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

/** "2 hours ago", "yesterday" (Foundation's named relative format). */
function relativeTime(unix: number): string {
  const diff = unix - Date.now() / 1000;
  const abs = Math.abs(diff);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31_536_000],
    ["month", 2_592_000],
    ["week", 604_800],
    ["day", 86_400],
    ["hour", 3600],
    ["minute", 60],
  ];
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  for (const [unit, secs] of units) {
    if (abs >= secs) return rtf.format(Math.trunc(diff / secs), unit);
  }
  return rtf.format(0, "second");
}

/** Keeps both ends of a long model name, like `.truncationMode(.middle)` at 150pt. */
function middleTruncate(name: string, max = 24): string {
  if (name.length <= max) return name;
  const keep = (max - 1) / 2;
  return `${name.slice(0, Math.ceil(keep))}…${name.slice(-Math.floor(keep))}`;
}

const THOUGHT_HOLD_MS = 1200;
const LINE_HEIGHT = 16;
const CHAT_MODES: { value: ChatMode; label: string; icon: GlyphName }[] = [
  { value: "regular", label: "Regular", icon: "textBubble" },
  { value: "my_tone", label: "My Tone", icon: "personWave" },
];

type Streaming = {
  conversationId: number;
  messageId: number;
  content: string;
  reasoning: string;
  thought: string | null;
  thoughtAt: number;
  startedAt: number;
};

// ================================================================ view ===

export default function ChatAI() {
  const toast = useToast();
  const { settings, patch } = useSettings();
  const { level } = useEngine();
  const [groups, setGroups] = useState<ChatGroup[]>([]);
  const [everyChat, setEveryChat] = useState<ChatConversation[]>([]);
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [historyVisible, setHistoryVisible] = useState(() => lsGet("chatAIHistoryVisible", "0") === "1");
  const [selected, setSelected] = useState<number | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [loadingModels, setLoadingModels] = useState(false);
  const [model, setModel] = useState("");
  // Reply depth is the saved `chat_thinking` setting, so it survives restarts.
  const depth: ThinkingDepth = settings?.chat_thinking ?? "low";
  const setDepth = (d: ThinkingDepth) => {
    if (d !== settings?.chat_thinking) patch({ chat_thinking: d }).catch((e) => toast(String(e), "error"));
  };
  const [think, setThink] = useState(false);
  const [mode, setMode] = useState<ChatMode>("regular");
  const [documents, setDocuments] = useState<ChatDocument[]>([]);
  const [info, setInfo] = useState<ChatInfo | null>(null);
  const [offline, setOffline] = useState(false);
  const [streaming, setStreaming] = useState<Streaming | null>(null);
  const [errorBanner, setErrorBanner] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<ChatConversation | null>(null);
  const [deleting, setDeleting] = useState<ChatConversation | null>(null);
  const [menu, setMenu] = useState<null | "engine" | "mode" | "settings">(null);
  const [pasteOpen, setPasteOpen] = useState(false);
  const [now, setNow] = useState(Date.now());
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const selectedRef = useRef<number | null>(null);
  selectedRef.current = selected;
  // The page opens on your latest chat, unless another page handed over a question.
  const openedLatest = useRef(false);

  const voice = useChatVoice(
    (text) => {
      setErrorBanner(null);
      setDraft((d) => (!d.trim() ? text : /[ \n]$/.test(d) ? d + text : `${d} ${text}`));
    },
    (message) => setErrorBanner(message),
  );
  const voiceActive = voice.phase !== "off";

  const refreshList = useCallback(() => {
    chat
      .conversations(search)
      .then((g) => {
        setGroups(g);
        if (search.trim()) return;
        const all = g.flatMap((x) => x.conversations);
        setEveryChat(all);
        if (!openedLatest.current) {
          openedLatest.current = true;
          if (selectedRef.current == null && all[0]) setSelected(all[0].id);
        }
      })
      .catch(() => {});
  }, [search]);

  const refreshModels = useCallback(() => {
    setLoadingModels(true);
    chat
      .models()
      .then((m) => {
        setModels(m);
        setOffline(false);
        setModel((cur) => {
          if (cur && m.some((x) => x.name === cur)) return cur;
          const saved = settings?.chat_model?.trim();
          if (saved && m.some((x) => x.name === saved)) return saved;
          return m[0]?.name ?? cur;
        });
      })
      .catch(() => {
        setModels([]);
        setOffline(true);
      })
      .finally(() => setLoadingModels(false));
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
  }, [messages, streaming?.content, streaming?.thought]);

  useEvent<ChatDeltaEvent>("chat-delta", (d) => {
    setStreaming((s) => {
      if (!s || s.messageId !== d.message_id) return s;
      // A fast model finishes a sentence many times a second; a new one replaces
      // the status line only after the last has been readable.
      const at = Date.now();
      const fresh = d.reasoning && latestThought(d.reasoning) && latestThought(d.reasoning) !== latestThought(s.thought);
      const show = fresh && at - s.thoughtAt >= THOUGHT_HOLD_MS;
      return { ...s, content: d.content, reasoning: d.reasoning, thought: show ? d.reasoning : s.thought, thoughtAt: show ? at : s.thoughtAt };
    });
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
  // Questions handed over from other pages (Memory Graph "Ask Chat AI").
  const applyPrefill = (q: string | null) => {
    if (!q) return;
    openedLatest.current = true;
    setSelected(null);
    setMessages([]);
    setDraft(q);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  };
  useEffect(() => applyPrefill(takePendingChatPrefill()), []); // eslint-disable-line react-hooks/exhaustive-deps
  useEvent<string>("chat-prefill", (q) => {
    applyPrefill(q);
    // The shell buffered it too; drop that copy once both listeners ran.
    window.setTimeout(() => takePendingChatPrefill(), 0);
  });

  const modelInfo = models?.find((m) => m.name === model);
  const isOllama = settings?.llm_provider !== "lm_studio";
  const supportsThinking = isOllama && (modelInfo?.thinking ?? nameSupportsThinking(model));
  const thinkingOn = supportsThinking && think;
  const busy = streaming != null;
  const hasTone = info?.has_tone_profile ?? false;
  const providerName = info?.provider_name ?? (isOllama ? "Ollama" : "LM Studio");
  const canSend = draft.trim().length > 0 && !busy;
  const canRecord = !busy && voice.phase !== "processing" && voice.phase !== "starting";

  const toggleHistory = (visible: boolean) => {
    lsSet("chatAIHistoryVisible", visible ? "1" : "0");
    setHistoryVisible(visible);
  };

  const toggleSearch = () => {
    if (searchOpen) setSearch("");
    setSearchOpen(!searchOpen);
  };

  const newChat = () => {
    setSelected(null);
    setDraft("");
    setErrorBanner(null);
    inputRef.current?.focus();
  };

  const openChat = (id: number) => {
    setSelected(id);
    setDraft("");
    setErrorBanner(null);
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
        thought: null,
        thoughtAt: 0,
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
    const useTone = s.myTone && hasTone;
    if (useTone) setMode("my_tone");
    if (s.sendNow) {
      send(s.prompt, useTone ? "my_tone" : undefined);
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
    if (!added.length && !documents.length) {
      setErrorBanner("No readable text was found in the selected file.");
      return;
    }
    setErrorBanner(null);
    setDocuments((d) => [...d, ...added]);
  };

  const streamingHere = streaming && streaming.conversationId === selected ? streaming : null;
  const showEmpty = selected == null || (messages.length === 0 && !streamingHere);
  const recents = everyChat.filter((c) => c.id !== selected).slice(0, 3);
  const historyCount = everyChat.length;
  const offlineLine = offline
    ? `${providerName} is not reachable${info ? ` at ${info.endpoint}` : ""}. Start ${providerName}, then refresh models.`
    : null;
  const noModelsLine =
    !offline && models && models.length === 0
      ? `${providerName} is reachable, but no local chat models were found. Download one in Model › AI Features (gemma4:e2b is a good start).`
      : null;
  const errorLine = errorBanner ?? offlineLine ?? noModelsLine;
  const offlineHint = !errorBanner && offline
    ? isOllama
      ? "Install Ollama from ollama.com and run `ollama serve`, then pull a chat model such as gemma4:e2b."
      : "Start the server from LM Studio's Developer tab, or run `lms server start`, then download a chat model."
    : undefined;

  // Two lines in a conversation, four on the start page, growing to eight. Sized
  // before paint, so keeping the newest words in view measures the final layout.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "0px";
    const min = (showEmpty ? 4 : 2) * LINE_HEIGHT;
    el.style.height = `${Math.min(Math.max(el.scrollHeight, min), 8 * LINE_HEIGHT)}px`;
  }, [draft, showEmpty]);

  // The composer moves when a chat starts; keep typing where you were.
  const wasEmpty = useRef(showEmpty);
  useEffect(() => {
    if (wasEmpty.current !== showEmpty) {
      wasEmpty.current = showEmpty;
      inputRef.current?.focus();
    }
  }, [showEmpty]);

  // The text field keeps the focus once a voice message ends.
  const wasVoiceActive = useRef(false);
  useEffect(() => {
    if (wasVoiceActive.current && !voiceActive) inputRef.current?.focus();
    wasVoiceActive.current = voiceActive;
  }, [voiceActive]);

  // Return finishes a recording and Escape drops it, wherever the focus is.
  useEffect(() => {
    if (voice.phase !== "recording") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        voice.finish();
      } else if (e.key === "Escape") {
        e.preventDefault();
        voice.cancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [voice]);

  const composer = (
    <div className="chat-composer">
      <div className="chat-context">
        <span className="chat-context-item" title="Replies draw on your dictation history and recent work">
          <Glyph name="bookClosed" /> Your dictation
        </span>
        <span className="chat-context-item" title="Runs on your own hardware">
          <Glyph name="laptop" /> {providerName} · This computer
        </span>
        {documents.length > 0 && (
          <span className="chat-context-item">
            <Glyph name="clip" /> {documents.length === 1 ? "1 file" : `${documents.length} files`}
          </span>
        )}
        <span className="chat-context-spacer" />
        <div className="chat-menu-anchor">
          <button className="chat-context-btn" onClick={() => setMenu(menu === "settings" ? null : "settings")} title="Chat settings" aria-label="Chat settings">
            <Glyph name="gear" size={14} />
          </button>
          {menu === "settings" && (
            <Popover onClose={() => setMenu(null)} align="right">
              <button
                className="chat-pop-item"
                disabled={loadingModels}
                onClick={() => {
                  setMenu(null);
                  refreshModels();
                }}
              >
                <Glyph name="refresh" size={14} /> Refresh Models
              </button>
              <button
                className="chat-pop-item"
                onClick={() => {
                  setMenu(null);
                  setPasteOpen(true);
                }}
              >
                <Glyph name="doc" size={14} /> Paste a Document…
              </button>
              <div className="chat-pop-divider" />
              <button
                className="chat-pop-item"
                onClick={() => {
                  setMenu(null);
                  emit("navigate", "ai-settings");
                }}
              >
                <Glyph name="sliders" size={14} /> AI Settings…
              </button>
            </Popover>
          )}
        </div>
      </div>
      <div className="chat-input-card">
        <div className="chat-input-wrap">
          <textarea
            ref={inputRef}
            className={`chat-input ${voiceActive ? "hidden" : ""}`}
            rows={1}
            value={draft}
            tabIndex={voiceActive ? -1 : 0}
            spellCheck={false}
            placeholder="Ask about your dictation, or ask for a draft"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                send();
              }
            }}
          />
          {voiceActive && <VoiceStrip phase={voice.phase} startedAt={voice.startedAt} level={level} onCancel={voice.cancel} />}
        </div>
        {documents.length > 0 && (
          <div className="chat-docs">
            {documents.map((d, i) => (
              <span key={`${d.name}-${i}`} className="chat-doc">
                <Glyph name="doc" />
                <span className="chat-doc-name">{d.name}</span>
                <span className="chat-doc-words">{d.content.split(/\s+/).filter(Boolean).length}w</span>
                <button aria-label={`Remove ${d.name}`} onClick={() => setDocuments((ds) => ds.filter((_, j) => j !== i))}>
                  <Glyph name="x" size={11} weight={3} />
                </button>
              </span>
            ))}
          </div>
        )}
        <div className="chat-composer-bar">
          <button className="chat-chip icon-only" title="Attach documents" aria-label="Attach documents" onClick={() => fileRef.current?.click()}>
            <Glyph name="plus" size={18} />
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
              className={`chat-chip menu ${mode === "my_tone" ? "active" : ""}`}
              onClick={() => setMenu(menu === "mode" ? null : "mode")}
              title={hasTone ? "Chat mode" : "Chat mode. My Tone needs a Tone of Voice profile from Analytics."}
            >
              <Glyph name={mode === "my_tone" ? "personWave" : "textBubble"} />
              {mode === "my_tone" ? "My Tone" : "Regular"}
              <Glyph name="chevron" size={9} weight={4} className="chat-chevron" />
            </button>
            {menu === "mode" && (
              <Popover onClose={() => setMenu(null)}>
                {CHAT_MODES.map((m) => (
                  <button
                    key={m.value}
                    className="chat-pop-item"
                    disabled={m.value === "my_tone" && !hasTone}
                    onClick={() => {
                      setMode(m.value);
                      setMenu(null);
                    }}
                  >
                    <Glyph name={m.icon} size={14} /> {m.label}
                  </button>
                ))}
              </Popover>
            )}
          </div>
          <span className="chat-composer-spacer" />
          <div className="chat-menu-anchor">
            <button className="chat-chip engine" onClick={() => setMenu(menu === "engine" ? null : "engine")} title="Model, reply depth and thinking">
              <Glyph name={thinkingOn ? "brain" : "bolt"} className="chat-engine-icon" />
              <span className="chat-chip-model" title={model}>
                {model ? middleTruncate(model) : "Choose model"}
              </span>
              <span className="chat-chip-depth">{DEPTHS.find((d) => d.value === depth)?.label}</span>
              <Glyph name="chevron" size={9} weight={4} className="chat-chevron" />
            </button>
            {menu === "engine" && (
              <Popover onClose={() => setMenu(null)} align="right">
                <div className="chat-pop-section">Model</div>
                {models && models.length === 0 && <div className="chat-pop-note">No chat models found</div>}
                {models === null && (
                  <div className="chat-pop-note">
                    <Spinner />
                  </div>
                )}
                {models?.map((m) => (
                  <button key={m.name} className="chat-pop-item" onClick={() => (setModel(m.name), setMenu(null))}>
                    <span className="chat-pop-check">{m.name === model && <Glyph name="check" size={13} weight={2.6} />}</span>
                    {m.name}
                  </button>
                ))}
                <div className="chat-pop-section">Reply depth</div>
                {DEPTHS.map((d) => (
                  <button key={d.value} className="chat-pop-item" onClick={() => (setDepth(d.value), setMenu(null))}>
                    <span className="chat-pop-check">{d.value === depth && <Glyph name="check" size={13} weight={2.6} />}</span>
                    {d.label}
                  </button>
                ))}
                {supportsThinking && (
                  <>
                    <div className="chat-pop-divider" />
                    <button className="chat-pop-item" onClick={() => (setThink(!think), setMenu(null))}>
                      <span className="chat-pop-check">{think && <Glyph name="check" size={13} weight={2.6} />}</span>
                      Thinking
                    </button>
                  </>
                )}
              </Popover>
            )}
          </div>
          <button
            className={`chat-voice ${voice.phase === "recording" ? "recording" : ""}`}
            disabled={!canRecord}
            onClick={voice.toggle}
            title={voice.phase === "recording" ? "Finish voice input (Enter)" : "Speak your message"}
            aria-label={voice.phase === "recording" ? "Finish voice input" : "Speak your message"}
          >
            {voice.phase === "processing" || voice.phase === "starting" ? (
              <Spinner size={14} />
            ) : (
              <span className="chat-voice-bars">
                <i />
                <i />
                <i />
              </span>
            )}
          </button>
          {busy ? (
            <button className="chat-send ready" onClick={stop} aria-label="Stop" title="Stop generating">
              <Glyph name="stop" size={17} />
            </button>
          ) : (
            <button className={`chat-send ${canSend ? "ready" : ""}`} disabled={!canSend} onClick={() => send()} aria-label="Send" title="Send (Enter). Shift-Enter adds a new line.">
              <Glyph name="arrowUp" size={17} weight={3} />
            </button>
          )}
        </div>
        {errorLine && (
          <p className="chat-error" role="alert" title={offlineHint}>
            {errorLine}
          </p>
        )}
      </div>
    </div>
  );

  return (
    <div className="chat">
      <PageHeader
        title="ChatAI"
        actions={
          <div className="chat-head-actions">
            <HeadButton label={historyVisible ? "Hide chat history" : "Show chat history"} icon="history" active={historyVisible} onClick={() => toggleHistory(!historyVisible)} />
            <HeadButton label="New chat" icon="compose" onClick={newChat} />
          </div>
        }
      />
      <div className="chat-body">
        {historyVisible && (
          <aside className="chat-history" aria-label="Chat history">
            <div className="chat-history-head">
              <span className="chat-history-title">Chats</span>
              {historyCount > 0 && <span className="chat-history-count">{historyCount.toLocaleString()}</span>}
              <span className="chat-history-spacer" />
              <HeadButton label={searchOpen ? "Close search" : "Search chats"} icon="search" active={searchOpen} onClick={toggleSearch} />
              <HeadButton label="New chat" icon="compose" onClick={newChat} />
              <HeadButton label="Close chats" icon="x" onClick={() => toggleHistory(false)} />
            </div>
            {searchOpen && (
              <div className="chat-search">
                <Glyph name="search" />
                <input
                  placeholder="Search chats"
                  value={search}
                  autoFocus
                  spellCheck={false}
                  onChange={(e) => setSearch(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && toggleSearch()}
                />
                {search && (
                  <button title="Clear search" aria-label="Clear search" onClick={() => setSearch("")}>
                    <Glyph name="xCircle" />
                  </button>
                )}
              </div>
            )}
            <div className="chat-history-list">
              {groups.map((g) => (
                <section key={g.title} className="chat-group">
                  <div className="chat-group-title">{g.title}</div>
                  {g.conversations.map((c) => (
                    <HistoryRow
                      key={c.id}
                      c={c}
                      active={c.id === selected}
                      live={info?.inflight.includes(c.id) || streaming?.conversationId === c.id}
                      onOpen={() => openChat(c.id)}
                      onRename={() => setRenaming(c)}
                      onDelete={() => setDeleting(c)}
                    />
                  ))}
                </section>
              ))}
              {groups.length === 0 && (
                <div className="chat-history-empty">
                  <Glyph name={search ? "search" : "bubbles"} size={22} />
                  <p>{search ? `No chats match “${search}”` : "No chats yet"}</p>
                </div>
              )}
            </div>
          </aside>
        )}

        <section className="chat-surface">
          {showEmpty ? (
            <div className="chat-start">
              <div className="chat-empty">
                <div className="chat-empty-head">
                  <SignalMark height={30} className="chat-mark" />
                  <h2>{greeting()}</h2>
                  <p>
                    {hasTone
                      ? "Draft, rewrite in your own voice, or ask how you write. Everything stays on this computer."
                      : "Draft, rewrite, or ask about how you write. Everything stays on this computer."}
                  </p>
                </div>
                {composer}
                <div className="chat-starters">
                  {starters(hasTone).map((s) => (
                    <button key={s.title} className="chat-starter" onClick={() => pickStarter(s)} title={s.detail}>
                      <Glyph name={s.icon} size={14} />
                      {s.title}
                    </button>
                  ))}
                </div>
                {recents.length > 0 && !historyVisible && (
                  <div className="chat-recents">
                    <div className="chat-recents-title">Recent</div>
                    {recents.map((c) => (
                      <button key={c.id} className="chat-recent" onClick={() => openChat(c.id)}>
                        <Glyph name="bubble" className="chat-recent-icon" />
                        <span className="chat-recent-title">{c.title}</span>
                        <span className="chat-recent-time">{relativeTime(c.updated_at)}</span>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="chat-scroll" ref={scrollRef}>
              <div className="chat-messages">
                {messages.map((m) => (
                  <MessageBubble key={m.id} m={m} />
                ))}
                {streamingHere && <ReplyInProgress s={streamingHere} now={now} thinking={thinkingOn} />}
              </div>
              <div className="chat-dock">
                <div className="chat-dock-inner">{composer}</div>
              </div>
            </div>
          )}
        </section>
      </div>

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
              if (selected === id) setSelected(everyChat.find((c) => c.id !== id)?.id ?? null);
              refreshList();
            })
            .catch((e) => toast(String(e), "error"));
        }}
      />
      <PasteModal
        open={pasteOpen}
        onClose={() => setPasteOpen(false)}
        onAdd={(doc) => {
          setErrorBanner(null);
          setDocuments((d) => [...d, doc]);
          setPasteOpen(false);
        }}
      />
    </div>
  );
}

// ========================================================= components ===

function HeadButton({ label, icon, active, onClick }: { label: string; icon: GlyphName; active?: boolean; onClick: () => void }) {
  return (
    <button className={`chat-head-btn ${active ? "active" : ""}`} onClick={onClick} title={label} aria-label={label}>
      <Glyph name={icon} size={17} weight={1.6} />
    </button>
  );
}

/** One chat on one row: title and time. Hovering swaps the time for delete;
 *  right-clicking offers rename and delete. */
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
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  return (
    <div
      className={`chat-row ${active ? "active" : ""}`}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuAt({ x: e.clientX, y: e.clientY });
      }}
    >
      <button className="chat-row-main" onClick={onOpen} aria-label={`${c.title}, ${shortTimestamp(c.updated_at)}`} />
      <span className="chat-row-title">{c.title}</span>
      <span className="chat-row-time">{live ? <Spinner size={11} /> : shortTimestamp(c.updated_at)}</span>
      <button className="chat-row-delete" onClick={onDelete} title="Delete chat" aria-label="Delete chat">
        <Glyph name="trash" size={12} />
      </button>
      {menuAt && (
        <Popover at={menuAt} onClose={() => setMenuAt(null)}>
          <button
            className="chat-pop-item"
            onClick={() => {
              setMenuAt(null);
              onRename();
            }}
          >
            Rename…
          </button>
          <div className="chat-pop-divider" />
          <button
            className="chat-pop-item danger"
            onClick={() => {
              setMenuAt(null);
              onDelete();
            }}
          >
            Delete…
          </button>
        </Popover>
      )}
    </div>
  );
}

/** Copies a whole reply as written (Markdown included). */
function CopyReply({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (
    <button
      className="chat-copy"
      title={copied ? "Copied" : "Copy reply"}
      aria-label="Copy reply"
      onClick={() => {
        navigator.clipboard?.writeText(text);
        setCopied(true);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => setCopied(false), 1500);
      }}
    >
      <Glyph name={copied ? "check" : "copy"} />
    </button>
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
      {m.reasoning && <Reasoning text={m.reasoning} />}
      <Markdown text={m.content} />
      <div className="chat-msg-foot">
        <CopyReply text={m.content} />
        {m.sources.length > 0 && <Sources sources={m.sources} />}
      </div>
    </div>
  );
}

/** The reply being written: the beating mark with what it is doing, then its
 *  words streaming straight onto the background. */
function ReplyInProgress({ s, now, thinking }: { s: Streaming; now: number; thinking: boolean }) {
  const line = s.content ? "ChatAI" : replyStatusLine((now - s.startedAt) / 1000, thinking, s.thought);
  return (
    <div className="chat-msg assistant">
      <div className="chat-msg-head">
        <WaitingMark beating />
        <span className="chat-status" key={line}>
          {line}
        </span>
      </div>
      {s.reasoning && <Reasoning text={s.reasoning} />}
      {s.content && <Markdown text={s.content} />}
    </div>
  );
}

/** What a thinking model reasoned, folded away until asked for. */
function Reasoning({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={`chat-fold ${open ? "open" : ""}`}>
      <button className="chat-fold-toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Glyph name="brain" size={12} />
        Reasoning
        <Glyph name="chevron" size={8} weight={4} className="chat-fold-chevron" />
      </button>
      {open && <div className="chat-reasoning-body">{text.trim()}</div>}
    </div>
  );
}

/** The dictation a reply drew on. */
function Sources({ sources }: { sources: ChatMessage["sources"] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={`chat-fold chat-sources ${open ? "open" : ""}`}>
      <button className="chat-fold-toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Glyph name="bookClosed" size={12} />
        {sources.length} from your dictation
        <Glyph name="chevron" size={8} weight={4} className="chat-fold-chevron" />
      </button>
      {open && (
        <ol className="chat-sources-list">
          {sources.map((s, i) => (
            <li key={`${s.transcription_id}-${i}`}>
              <div className="chat-source-meta">
                {s.app || "Unknown App"} · {new Date(s.created_at * 1000).toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" })} ·{" "}
                {Math.round(s.score * 100)}% match
              </div>
              <p>{s.text.length > 320 ? s.text.slice(0, 319) + "…" : s.text}</p>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

/** Shown in place of the text while a voice message is recorded or transcribed:
 *  a live level meter and the time, with how to finish. */
function VoiceStrip({ phase, startedAt, level, onCancel }: { phase: VoicePhase; startedAt: number | null; level: number; onCancel: () => void }) {
  const [t, setT] = useState(() => Date.now());
  const recording = phase === "recording";
  useEffect(() => {
    if (!recording) return;
    let frame = 0;
    let last = 0;
    const tick = (ms: number) => {
      if (ms - last >= 1000 / 24) {
        last = ms;
        setT(Date.now());
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [recording]);

  if (!recording) {
    return (
      <div className="chat-voice-strip" role="status">
        <Spinner size={14} />
        <span className="chat-voice-working">{phase === "starting" ? "Getting the microphone ready…" : "Transcribing your voice…"}</span>
      </div>
    );
  }
  const seconds = Math.max(0, Math.floor((t - (startedAt ?? t)) / 1000));
  const boosted = Math.min(1, level * 2.5 + 0.08);
  const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  return (
    <div className="chat-voice-strip" role="status" aria-label="Recording your voice">
      <span className="chat-voice-dot" />
      <span className="chat-voice-time">{`${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`}</span>
      <span className="chat-voice-meter">
        {Array.from({ length: 24 }, (_, i) => {
          const wave = still ? 0.6 : 0.45 + 0.55 * Math.abs(Math.sin(i * 0.9 + (t / 1000) * 7));
          return <i key={i} style={{ height: Math.max(3, 20 * boosted * wave) }} />;
        })}
      </span>
      <span className="chat-voice-spacer" />
      <span className="chat-voice-hint">Enter to finish · Esc to cancel</span>
      <button className="chat-chip chat-voice-cancel" onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}

/** A menu below its button (above when there's no room), or at a point. */
function Popover({
  children,
  onClose,
  align = "left",
  at,
}: {
  children: ReactNode;
  onClose: () => void;
  align?: "left" | "right";
  at?: { x: number; y: number };
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const a = at ? { left: at.x, right: at.x, top: at.y, bottom: at.y } : el.parentElement!.getBoundingClientRect();
    const gap = at ? 0 : 4;
    let top = a.bottom + gap;
    if (top + h > window.innerHeight - 8) top = Math.max(8, a.top - h - gap);
    const left = Math.min(Math.max(8, align === "right" ? a.right - w : a.left), window.innerWidth - w - 8);
    setPos({ left, top });
  }, [align, at]);
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
    <div ref={ref} className="chat-pop" role="menu" style={pos ?? { left: 0, top: 0, visibility: "hidden" }}>
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

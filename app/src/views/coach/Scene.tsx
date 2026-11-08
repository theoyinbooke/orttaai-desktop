// A scene, full screen (Mac CoachSceneView + CoachSceneSession): the partner
// with a mood that moves, the goal, the conversation as subtitles (the
// partner's reply appears as it's written and is spoken sentence by
// sentence), and your turn by voice or by typing. Hands-free, the microphone
// opens when the partner stops and your turn ends when you pause; Space
// always cuts in. (Coach area.)
import { useCallback, useEffect, useRef, useState } from "react";
import "./practice.css";
import { Icon } from "../../ui";
import { useEvent } from "../../lib/api";
import { useEngine } from "../../lib/engine";
import { useSettings } from "../../lib/settings";
import {
  SentenceStream,
  TurnDetector,
  cleanedReply,
  coachApi,
  coachVoice,
  newToken,
  personaVoice,
  spokenSentence,
  waitForCapture,
  type Capture,
  type Mood,
  type RepResult,
  type SceneStart,
  type SceneTurn,
} from "../../lib/coach";
import { CoachIcon } from "./icons";
import { Btn, Eyebrow, Spinner, kindLimit, personaOf, useCoach } from "./parts";
import PersonaGlyph from "./PersonaGlyph";
import Reveal from "./Reveal";

type ReplyMode = "voice" | "text" | "both";
const REPLY_MODES: { id: ReplyMode; title: string }[] = [
  { id: "voice", title: "Voice" },
  { id: "text", title: "Text" },
  { id: "both", title: "Both" },
];
const speaksIn = (m: ReplyMode) => m !== "text";

type Phase =
  | { k: "loading" }
  | { k: "starting" }
  | { k: "talking" }
  | { k: "recording"; started: number }
  | { k: "thinking" }
  | { k: "scoring"; message: string }
  | { k: "ended"; result: RepResult }
  | { k: "failed"; message: string };

const MOODS: { id: Mood; title: string; step: number }[] = [
  { id: "pushing", title: "Pushing", step: 0 },
  { id: "weighing", title: "Weighing it", step: 1 },
  { id: "agreed", title: "Agreed", step: 2 },
];

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable;
}

const now = () => performance.now() / 1000;

export default function SceneSession({ sceneId, onScene, onExit }: { sceneId: string; onScene: (id: string) => void; onExit: () => void }) {
  const { catalog } = useCoach();
  const { settings, patch } = useSettings();
  const { engine } = useEngine();

  const [start, setStart] = useState<SceneStart | null>(null);
  const [phase, setPhase] = useState<Phase>({ k: "loading" });
  const [turns, setTurns] = useState<SceneTurn[]>([]);
  const [mood, setMood] = useState<Mood>("pushing");
  const [replyMode, setReplyMode] = useState<ReplyMode>("both");
  const [typed, setTyped] = useState("");
  const [typeInstead, setTypeInstead] = useState(false);
  const [live, setLive] = useState<string | null>(null);
  const [partnerSpeaking, setPartnerSpeaking] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [level, setLevel] = useState(0);
  const handsFree = settings?.coach_hands_free ?? true;

  // Refs the async flow reads, so it always sees the latest.
  const cancelled = useRef(false);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const turnsRef = useRef(turns);
  turnsRef.current = turns;
  const modeRef = useRef(replyMode);
  modeRef.current = replyMode;
  const handsFreeRef = useRef(handsFree);
  handsFreeRef.current = handsFree;
  const typeInsteadRef = useRef(typeInstead);
  typeInsteadRef.current = typeInstead;
  const speakingRef = useRef(false);
  const engineRef = useRef(engine);
  engineRef.current = engine;
  const startRef = useRef<SceneStart | null>(null);

  const capture = useRef<{ token: number; cancel: () => void } | null>(null);
  const listening = useRef(false);
  const recordingByUs = useRef(false);
  const detector = useRef<TurnDetector | null>(null);
  const request = useRef<number | null>(null);
  const liveRef = useRef("");
  const stream = useRef(new SentenceStream());
  const bottom = useRef<HTMLDivElement>(null);
  const partnerTurnRef = useRef<(sofar: SceneTurn[]) => Promise<void>>(async () => {});

  const scene = start?.scene ?? catalog.scenes.find((s) => s.id === sceneId) ?? catalog.scenes[0];
  const name = scene.partner_name;
  const yourTurns = turns.filter((t) => t.speaker === "you").length;
  const canEnd = yourTurns > 0;
  const limit = kindLimit(catalog, "scene");
  const persona = start ? personaOf(catalog, start.persona) : null;

  const setSpeaking = (v: boolean) => {
    speakingRef.current = v;
    setPartnerSpeaking(v);
  };

  const cancelCapture = useCallback(() => {
    capture.current?.cancel();
    capture.current = null;
    listening.current = false;
  }, []);

  /** Stops a recording we started without keeping what it hears. */
  const dropRecording = useCallback(() => {
    cancelCapture();
    if (recordingByUs.current) {
      recordingByUs.current = false;
      coachApi
        .discard()
        .then(() => (engineRef.current === "recording" ? coachApi.toggleRecording() : undefined))
        .catch(() => {});
    } else {
      coachApi.disarm().catch(() => {});
    }
  }, [cancelCapture]);

  // ---- The partner ------------------------------------------------------

  const speakAs = useCallback(() => {
    coachVoice.begin({ speaker: name, rate: 1, pitch: 1 });
  }, [name]);

  const received = useCallback((text: string) => {
    if (request.current == null || text.length <= liveRef.current.length) return;
    liveRef.current = text;
    setLive(text);
    if (speaksIn(modeRef.current)) {
      for (const s of stream.current.feed(text)) {
        const say = spokenSentence(s);
        if (say) coachVoice.enqueue(say);
      }
    }
  }, []);

  useEvent<{ request: number; text: string }>("coach-scene-delta", (p) => {
    if (p.request === request.current) received(p.text);
  });

  // ---- Your turn --------------------------------------------------------

  const startRecording = useCallback(async () => {
    if (phaseRef.current.k !== "talking" || cancelled.current) return;
    if (engineRef.current === "off" || engineRef.current === "loading") {
      setNotice("Dictation isn't ready yet. Start the engine on Home, or type your turn.");
      return;
    }
    coachVoice.stop();
    setSpeaking(false);
    setNotice(null);
    cancelCapture();
    const token = newToken();
    const wait = waitForCapture(token);
    capture.current = { token, cancel: wait.cancel };
    listening.current = true;
    wait.promise.then((c: Capture) => {
      if (capture.current?.token !== token || cancelled.current) return;
      capture.current = null;
      listening.current = false;
      recordingByUs.current = false;
      const turn: SceneTurn = { speaker: "you", text: c.text, heard: c.raw, duration_ms: c.duration_ms };
      const next = [...turnsRef.current, turn];
      setTurns(next);
      turnsRef.current = next;
      partnerTurnRef.current(next);
    });
    try {
      // The partner answers what you said; Polish would only slow it down.
      await coachApi.arm(token, false);
      await coachApi.toggleRecording();
      recordingByUs.current = true;
      detector.current = new TurnDetector();
      setLevel(0);
      setPhase({ k: "recording", started: now() });
    } catch (e) {
      cancelCapture();
      coachApi.disarm().catch(() => {});
      setNotice(`That turn didn't go through: ${String(e)}`);
    }
  }, [cancelCapture]);

  const listenIfHandsFree = useCallback(() => {
    if (!handsFreeRef.current || modeRef.current === "text" || typeInsteadRef.current) return;
    if (phaseRef.current.k !== "talking" || cancelled.current) return;
    startRecording();
  }, [startRecording]);

  const finishRecording = useCallback(() => {
    if (phaseRef.current.k !== "recording") return;
    setLevel(0);
    setPhase({ k: "thinking" });
    coachApi.toggleRecording().catch((e) => {
      cancelCapture();
      setPhase({ k: "talking" });
      setNotice(`That turn didn't go through: ${String(e)}`);
    });
  }, [cancelCapture]);

  const cutIn = useCallback(() => {
    if (speakingRef.current) {
      coachVoice.stop();
      setSpeaking(false);
    }
    if (phaseRef.current.k === "talking") startRecording();
  }, [startRecording]);

  // Hands-free: the level decides when your turn ends.
  useEvent<number>("audio-level", (l) => {
    if (phaseRef.current.k !== "recording") return;
    setLevel(l);
    const d = detector.current;
    if (!d || !handsFreeRef.current) return;
    const event = d.feed(l, now());
    if (event === "ended") {
      finishRecording();
    } else if (event === "gaveUp") {
      // Nobody spoke: close the microphone and wait for a tap.
      dropRecording();
      setLevel(0);
      setPhase({ k: "talking" });
    }
  });

  // The turn's time limit.
  useEffect(() => {
    if (phase.k !== "recording") return;
    const started = phase.started;
    const id = window.setInterval(() => {
      if (now() - started >= limit) finishRecording();
    }, 250);
    return () => window.clearInterval(id);
  }, [phase, limit, finishRecording]);

  // Stopped with the dictation hotkey.
  useEffect(() => {
    if (phase.k === "recording" && engine === "processing") setPhase({ k: "thinking" });
  }, [engine, phase.k]);

  // The engine finished but nothing reached Coach: nothing was heard.
  useEffect(() => {
    if (engine !== "idle" || !listening.current || phase.k !== "thinking") return;
    const id = window.setTimeout(() => {
      if (!listening.current || cancelled.current) return;
      cancelCapture();
      recordingByUs.current = false;
      coachApi.disarm().catch(() => {});
      setPhase({ k: "talking" });
      setNotice("Coach didn't hear that turn.");
      window.setTimeout(listenIfHandsFree, 0);
    }, 2500);
    return () => window.clearTimeout(id);
  }, [engine, phase.k, cancelCapture, listenIfHandsFree]);

  useEvent<string>("engine-error", (message) => {
    if (phaseRef.current.k !== "recording" && !listening.current) return;
    cancelCapture();
    recordingByUs.current = false;
    coachApi.disarm().catch(() => {});
    setPhase({ k: "talking" });
    setNotice(`That turn didn't go through: ${message}`);
  });

  // ---- The end ----------------------------------------------------------

  const end = useCallback(async () => {
    const s = startRef.current;
    if (!s || !turnsRef.current.some((t) => t.speaker === "you")) return;
    cancelled.current = true;
    if (phaseRef.current.k === "recording") dropRecording();
    else cancelCapture();
    request.current = null;
    coachVoice.stop();
    setSpeaking(false);
    setLive(null);
    setPhase({ k: "scoring", message: "Reading the scene back…" });
    try {
      const result = await coachApi.sceneEnd(sceneId, s.skill, s.persona, s.usual, turnsRef.current);
      setPhase({ k: "ended", result });
      if (settings?.coach_read_aloud) coachVoice.speak(result.note, personaVoice(catalog, s.persona, settings));
    } catch (e) {
      setPhase({ k: "failed", message: String(e) });
    }
  }, [sceneId, settings, catalog, dropRecording, cancelCapture]);

  useEvent<{ message: string }>("coach-rep-phase", (p) => {
    if (phaseRef.current.k === "scoring") setPhase({ k: "scoring", message: p.message });
  });

  // ---- The partner's turn ----------------------------------------------

  const partnerTurn = useCallback(
    async (sofar: SceneTurn[]) => {
      if (sofar.filter((t) => t.speaker === "you").length >= catalog.max_scene_turns) {
        await end();
        return;
      }
      setPhase({ k: "thinking" });
      const speaks = speaksIn(modeRef.current);
      const id = newToken();
      request.current = id;
      liveRef.current = "";
      stream.current = new SentenceStream();
      setLive("");
      if (speaks) {
        speakAs();
        setSpeaking(true);
      }
      let reply: string | null = null;
      try {
        reply = await coachApi.sceneReply(sceneId, sofar, id);
      } catch {
        reply = null;
      }
      if (cancelled.current || request.current !== id) return;
      if (reply) {
        received(reply);
        const rest = stream.current.finish(reply);
        if (rest && speaksIn(modeRef.current)) {
          const say = spokenSentence(rest);
          if (say) coachVoice.enqueue(say);
        }
        const withReply = [...sofar, { speaker: "partner" as const, text: cleanedReply(reply) }];
        setTurns(withReply);
        turnsRef.current = withReply;
        coachApi
          .sceneMood(sceneId, withReply)
          .then((m) => {
            if (m && !cancelled.current) setMood(m);
          })
          .catch(() => {});
      } else {
        setNotice(`That turn didn't go through: ${name} didn't answer.`);
      }
      request.current = null;
      liveRef.current = "";
      setLive(null);
      setPhase({ k: "talking" });
      phaseRef.current = { k: "talking" };
      if (speaks) {
        await coachVoice.idle();
        if (cancelled.current) return;
        setSpeaking(false);
      }
      if (reply) listenIfHandsFree();
    },
    [catalog.max_scene_turns, end, sceneId, speakAs, received, name, listenIfHandsFree],
  );
  partnerTurnRef.current = partnerTurn;

  const sendTyped = useCallback(() => {
    const text = typed.trim();
    if (phaseRef.current.k !== "talking" || !text) return;
    coachVoice.stop();
    setSpeaking(false);
    setNotice(null);
    setTyped("");
    const next = [...turnsRef.current, { speaker: "you" as const, text }];
    setTurns(next);
    turnsRef.current = next;
    partnerTurn(next);
  }, [typed, partnerTurn]);

  // ---- Opening ----------------------------------------------------------

  useEffect(() => {
    cancelled.current = false;
    (async () => {
      let s: SceneStart;
      try {
        s = await coachApi.sceneBegin(sceneId);
      } catch (e) {
        setPhase({ k: "failed", message: String(e) });
        return;
      }
      if (cancelled.current) return;
      startRef.current = s;
      setStart(s);
      const opening: SceneTurn[] = [{ speaker: "partner", text: s.scene.opening }];
      setTurns(opening);
      turnsRef.current = opening;
      if (s.problem) {
        setPhase({ k: "failed", message: s.problem });
        return;
      }
      setPhase({ k: "starting" });
      await coachApi.sceneWarm().catch(() => {});
      if (cancelled.current) return;
      setPhase({ k: "talking" });
      phaseRef.current = { k: "talking" };
      if (speaksIn(modeRef.current)) {
        setSpeaking(true);
        coachVoice.begin({ speaker: s.scene.partner_name, rate: 1, pitch: 1 });
        coachVoice.enqueue(s.scene.opening);
        await coachVoice.idle();
        if (cancelled.current) return;
        setSpeaking(false);
      }
      listenIfHandsFree();
    })();
    return () => {
      cancelled.current = true;
      request.current = null;
      coachVoice.stop();
      capture.current?.cancel();
      capture.current = null;
      if (recordingByUs.current && engineRef.current === "recording") {
        coachApi
          .discard()
          .then(() => coachApi.toggleRecording())
          .catch(() => {});
      } else {
        coachApi.disarm().catch(() => {});
      }
    };
    // Runs once per scene; the callbacks it uses read refs.
  }, [sceneId]);

  // Keep the latest line in view.
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end", behavior: "smooth" });
  }, [turns.length, live]);

  const recording = phase.k === "recording";
  const ready = phase.k === "talking";

  const micPress = useCallback(() => {
    if (speakingRef.current) cutIn();
    else if (phaseRef.current.k === "recording") finishRecording();
    else startRecording();
  }, [cutIn, finishRecording, startRecording]);

  // Space talks or cuts in, Escape leaves.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (document.querySelector(".coach-pop")) return;
        e.preventDefault();
        onExit();
        return;
      }
      if (e.key !== " " || e.repeat || isTyping(e.target) || modeRef.current === "text") return;
      const k = phaseRef.current.k;
      if (k === "talking" || k === "recording") {
        e.preventDefault();
        micPress();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [micPress, onExit]);

  const status = (): string => {
    if (recording && partnerSpeaking) return `${name} is speaking. Just talk to cut in.`;
    if (recording) return handsFree ? "Listening. Pause when you're done." : "Listening. Press again when you're done.";
    if (partnerSpeaking) return `${name} is speaking. Press Space to cut in.`;
    if (ready && notice) return notice;
    if (ready) return "Your turn. Press to talk, or the space bar.";
    return `${name} is answering…`;
  };

  const nextScene = () => {
    const all = catalog.scenes;
    const i = all.findIndex((s) => s.id === sceneId);
    return all[(i + 1) % all.length].id;
  };

  const moodStep = MOODS.find((m) => m.id === mood)?.step ?? 0;
  const moodLine = mood === "pushing" ? `${name} is pushing` : mood === "weighing" ? `${name} is weighing it` : `${name} agrees`;
  const showTyping = replyMode === "text" || typeInstead;

  return (
    <div className="coach-focus coach-scene">
      <header className="coach-focus-head">
        <button type="button" className="coach-icon-btn" onClick={onExit} aria-label="Leave the scene">
          <Icon name="x" size={16} />
        </button>
        <div className="middle coach-scene-title">
          <Eyebrow>Scene</Eyebrow>
          <strong>{scene.title}</strong>
        </div>
        <div className="coach-mode-switch" role="radiogroup" aria-label={`${name} replies in`}>
          {REPLY_MODES.map((m) => (
            <button
              key={m.id}
              type="button"
              role="radio"
              aria-checked={replyMode === m.id}
              onClick={() => {
                setReplyMode(m.id);
                if (!speaksIn(m.id)) {
                  coachVoice.stop();
                  setSpeaking(false);
                }
              }}
            >
              {m.title}
            </button>
          ))}
        </div>
      </header>

      {phase.k === "ended" && (
        <Reveal
          result={phase.result}
          nextTitle="Another scene"
          onAgain={() => onScene(sceneId)}
          onNext={() => onScene(nextScene())}
          onDone={onExit}
        />
      )}

      {phase.k === "scoring" && (
        <div className="coach-rep-center" role="status">
          <Spinner />
          <p className="coach-rep-status">{phase.message}</p>
        </div>
      )}

      {phase.k === "failed" && (
        <div className="coach-rep-center">
          <p className="coach-rep-failed">{phase.message}</p>
          <Btn kind="primary" onClick={onExit}>
            Back to Studio
          </Btn>
        </div>
      )}

      {phase.k === "loading" && (
        <div className="coach-rep-center" role="status">
          <Spinner />
        </div>
      )}

      {(phase.k === "starting" || phase.k === "talking" || phase.k === "recording" || phase.k === "thinking") && (
        <div className="coach-scene-stage">
          <aside className="coach-scene-side">
            <div className="coach-scene-partner">
              <div className={`coach-scene-avatar ${phase.k === "thinking" || phase.k === "starting" ? "thinking" : ""}`}>
                <span className="ring outer" aria-hidden="true" />
                <span className="ring inner" aria-hidden="true" />
                <span className="disc coach-display">{name.slice(0, 1)}</span>
              </div>
              <div className="coach-scene-who">
                <strong>{name}</strong>
                <span className="coach-label">{scene.partner_role}</span>
              </div>
              <div className="coach-scene-mood" role="img" aria-label={`Mood: ${MOODS[moodStep].title}`}>
                <span className="coach-scene-mood-line">{moodLine}</span>
                <div className="coach-scene-mood-bars">
                  {MOODS.map((m) => (
                    <div key={m.id} className="coach-scene-mood-step">
                      <span className={`bar ${m.step <= moodStep ? "on" : ""}`} />
                      <span className={`t ${m.id === mood ? "now" : ""}`}>{m.title}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
            <div className="coach-surface deep coach-scene-goal">
              <span className="coach-label">Your goal</span>
              <p>{scene.goal}</p>
              <span className="coach-scene-turn">
                Turn {Math.min(yourTurns + 1, catalog.max_scene_turns)} of {catalog.max_scene_turns}
              </span>
            </div>
            {persona && (
              <div className="coach-row coach-scene-listening">
                <PersonaGlyph persona={persona.id} size={24} />
                <span>{persona.name} is listening. Notes when the scene ends.</span>
              </div>
            )}
          </aside>

          <section className="coach-scene-main">
            <div className="coach-scene-conversation" aria-live="polite">
              {turns.map((turn, i) => {
                const latest = i === turns.length - 1 && live == null;
                return turn.speaker === "partner" ? (
                  <div key={i} className={`coach-scene-line partner ${latest ? "latest" : ""}`}>
                    <span className="who">{name}</span>
                    {(replyMode !== "voice" || latest) && <p>{latest ? `“${turn.text}”` : turn.text}</p>}
                    {speaksIn(replyMode) && (
                      <button
                        type="button"
                        className="coach-scene-hear"
                        disabled={phase.k !== "talking"}
                        onClick={() => {
                          setSpeaking(true);
                          coachVoice.speak(turn.text, { speaker: name, rate: 1, pitch: 1 }).then(() => setSpeaking(false));
                        }}
                      >
                        <CoachIcon name="playFill" size={13} />
                        Hear again
                      </button>
                    )}
                  </div>
                ) : (
                  <div key={i} className="coach-scene-line you">
                    <span className="who">{turn.duration_ms == null ? "You · typed" : "You · spoken"}</span>
                    <p>{turn.text}</p>
                  </div>
                );
              })}
              {live ? (
                <div className="coach-scene-line partner latest">
                  <span className="who">{name}</span>
                  <p>“{live}”</p>
                </div>
              ) : (
                (phase.k === "thinking" || phase.k === "starting") && (
                  <div className="coach-row coach-scene-thinking">
                    <Spinner small />
                    <span className="coach-label">{name} is thinking…</span>
                  </div>
                )
              )}
              <div ref={bottom} />
            </div>

            {showTyping && (
              <div className="coach-scene-bar text">
                <input
                  className="coach-scene-input"
                  value={typed}
                  placeholder={`Reply to ${name}`}
                  onChange={(e) => setTyped(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      sendTyped();
                    }
                  }}
                  disabled={!ready}
                  aria-label={`Reply to ${name}`}
                  autoFocus
                />
                <Btn kind="primary" onClick={sendTyped} disabled={!ready || !typed.trim()}>
                  Send
                </Btn>
                {replyMode === "text" && (
                  <Btn onClick={end} disabled={!canEnd}>
                    End scene
                  </Btn>
                )}
              </div>
            )}

            {replyMode !== "text" && (
              <div className="coach-scene-bar">
                <button
                  type="button"
                  className={`coach-scene-mic ${recording ? "live" : ""}`}
                  onClick={micPress}
                  disabled={!(ready || recording)}
                  aria-label={partnerSpeaking ? "Cut in" : recording ? "Finish your turn" : "Talk"}
                  style={{ ["--level" as string]: recording ? Math.min(1, level) : 0 }}
                >
                  <CoachIcon name={recording ? "stopFill" : "micFill"} size={24} />
                </button>
                <span className={`coach-scene-status ${ready && notice ? "warn" : ""}`} title={notice ?? undefined}>
                  {status()}
                </span>
                <span className="coach-spacer" />
                <button
                  type="button"
                  className="coach-link quiet"
                  onClick={() => {
                    if (!typeInstead && phaseRef.current.k === "recording") {
                      dropRecording();
                      setPhase({ k: "talking" });
                    }
                    setTypeInstead((t) => !t);
                  }}
                >
                  {typeInstead ? "Hide typing" : "Type instead"}
                </button>
                <label className="coach-scene-handsfree" title={`Your turn starts when ${name} stops, and ends when you pause.`}>
                  <span>Hands-free</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={handsFree}
                    className={`switch ${handsFree ? "on" : ""}`}
                    onClick={() => patch({ coach_hands_free: !handsFree })}
                  >
                    <span className="switch-knob" />
                  </button>
                </label>
                <Btn onClick={end} disabled={!canEnd}>
                  End scene
                </Btn>
              </div>
            )}
          </section>
        </div>
      )}
    </div>
  );
}

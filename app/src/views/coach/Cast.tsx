// Your coaches (Mac CoachCastView): one for each way of talking, picked by
// their voice, and the local models that judge, write and play the partners.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, Switch } from "../../ui";
import { useEvent } from "../../lib/api";
import { useSettings, type SettingsPatch } from "../../lib/settings";
import {
  availableVoices,
  coachApi,
  coachVoice,
  personaFor,
  personaVoice,
  percent,
  SHAPED,
  speechSupported,
  modeTitle,
  type Catalog,
  type Comparison,
  type Grade,
  type JudgeState,
  type ModelsInfo,
  type PersonaId,
  type ShapedMode,
} from "../../lib/coach";
import { CoachIcon } from "./icons";
import PersonaGlyph from "./PersonaGlyph";
import { Btn, CoachPage, InfoButton, Spinner, Tag, useCoach } from "./parts";

export default function Cast() {
  const [showingModels, setShowingModels] = useState(false);
  return showingModels ? <ModelsView onBack={() => setShowingModels(false)} /> : <CastView onModels={() => setShowingModels(true)} />;
}

function CastView({ onModels }: { onModels: () => void }) {
  const { snapshot, catalog, settings } = useCoach();
  const [seat, setSeat] = useState<ShapedMode>("ai");
  const [setup, setSetup] = useState<ModelsInfo["setup"] | null>(null);
  useEffect(() => {
    coachApi.models().then((m) => setSetup(m.setup)).catch(() => {});
  }, [settings?.coach_model, settings?.coach_judge_model]);
  const total = SHAPED.reduce((n, m) => n + (snapshot.recent_by_mode[m] ?? 0), 0);
  const chosen = personaFor(settings, seat);
  return (
    <CoachPage
      eyebrow="Cast"
      title="Your coaches"
      accessory={<InfoButton title="Your coaches" text="Coaches change the words, never the score. Pick a seat, then the coach for it." />}
    >
      <div className="coach-grid" style={{ gap: 14, gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))" }}>
        {SHAPED.map((mode) => {
          const persona = personaFor(settings, mode);
          const p = catalog.personas.find((x) => x.id === persona)!;
          return (
            <button
              key={mode}
              type="button"
              className={`coach-surface coach-seat coach-plain ${mode === seat ? "selected" : ""}`}
              aria-pressed={mode === seat}
              onClick={() => setSeat(mode)}
            >
              <PersonaGlyph persona={persona} size={48} />
              <span className="coach-col" style={{ gap: 2, flex: 1, textAlign: "left" }}>
                <span className="coach-label">{modeTitle(mode)}</span>
                <strong style={{ fontSize: 18 }}>{p.name}</strong>
              </span>
              <span className="coach-label">{total ? percent((snapshot.recent_by_mode[mode] ?? 0) / total) : "–"}</span>
            </button>
          );
        })}
      </div>
      <div className="coach-grid" style={{ ["--min" as string]: "190px" }}>
        {catalog.personas.map((p) => (
          <div
            key={p.id}
            role="button"
            tabIndex={0}
            aria-pressed={p.id === chosen}
            aria-label={`${p.name}, ${p.trait}`}
            className={`coach-surface coach-persona ${p.id === chosen ? "selected" : ""}`}
            onClick={() => coachApi.setPersona(p.id, seat)}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), coachApi.setPersona(p.id, seat))}
          >
            <div className="coach-row">
              <PersonaGlyph persona={p.id} size={60} />
              <span className="coach-spacer" />
              {p.id === chosen && (
                <span style={{ color: "var(--accent)" }}>
                  <CoachIcon name="checkmark" size={15} weight={2.6} />
                </span>
              )}
            </div>
            <div className="coach-col" style={{ gap: 2 }}>
              <strong style={{ fontSize: 18 }}>{p.name}</strong>
              <span className="coach-label" style={{ color: "var(--c-warm-ink)" }}>
                {p.trait}
              </span>
            </div>
            <p className="coach-persona-sample">“{p.sample}”</p>
            <button
              type="button"
              className="coach-chip-btn"
              onClick={(e) => {
                e.stopPropagation();
                coachVoice.speak(p.sample, personaVoice(catalog, p.id as PersonaId, settings));
              }}
            >
              <CoachIcon name="playFill" size={12} />
              Hear
            </button>
          </div>
        ))}
      </div>
      <div className="coach-surface coach-cast-models">
        <span className="coach-label">On this computer</span>
        <ModelLabel job="Coach" model={setup?.coach_model} />
        <ModelLabel job="Judge" model={setup?.judge_model} />
        <ModelLabel job="Server" model={setup?.provider_name} />
        <span className="coach-spacer" />
        <button type="button" className="coach-link" style={{ fontSize: 14 }} onClick={onModels}>
          Change models
        </button>
      </div>
    </CoachPage>
  );
}

function ModelLabel({ job, model }: { job: string; model?: string }) {
  return (
    <span className="coach-model-label">
      <span>{job}</span>
      <strong>{model ?? "…"}</strong>
    </span>
  );
}

// -------------------------------------------------------------- models ----

/** Models and grades from the server, refreshed when grades or settings change. */
export function useModelsInfo() {
  const [info, setInfo] = useState<ModelsInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [judge, setJudge] = useState<JudgeState | null>(null);
  const load = useCallback(() => {
    setLoading(true);
    coachApi
      .models()
      .then((m) => {
        setInfo(m);
        setJudge(m.judge_state);
      })
      .catch(() => setInfo(null))
      .finally(() => setLoading(false));
  }, []);
  useEffect(load, [load]);
  useEvent<JudgeState>("coach-judge-state", (s) => {
    setJudge(s);
    if (s.state !== "checking") load();
  });
  useEvent("settings-changed", load);
  return { info, loading, judge, reload: load };
}

const passes = (g: Grade, cat: Catalog | null) => {
  const acc = cat?.calibration.pass_accuracy ?? 0.8;
  const rec = cat?.calibration.pass_recall ?? 0.7;
  return g.total > 0 && g.answered >= g.total * 0.9 && g.accuracy >= acc && g.yes_recall >= rec && g.no_recall >= rec;
};

function gradeDetail(info: ModelsInfo, model: string, catalog: Catalog | null): string | undefined {
  if (!info.server_running) return undefined;
  if (!info.installed.includes(model)) return "Not installed";
  const g = info.grades.find((x) => x.model === model);
  if (g) return `${passes(g, catalog) ? "Passed" : "Failed"} the judge check · ${Math.round(g.accuracy * 100)}%`;
  return model === info.insights_model ? "Not checked · also used by Insights" : "Not checked";
}

type DropdownOption = { value: string; label: string; detail?: string; warns?: boolean };

/** Mac OrttaaiDropdown: a 340pt field whose list drops down as wide as it, with
 *  a search box, a detail line under each option and a check on the chosen one. */
function Dropdown({
  value,
  options,
  onChange,
  ariaLabel,
  triggerLabel,
  searchPrompt,
}: {
  value: string;
  options: DropdownOption[];
  onChange: (v: string) => void;
  ariaLabel: string;
  triggerLabel?: string;
  searchPrompt?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [rect, setRect] = useState<{ top: number; left: number; width: number } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const label = triggerLabel ?? options.find((o) => o.value === value)?.label ?? "Select";
  const text = query.trim().toLowerCase();
  const shown = text ? options.filter((o) => o.label.toLowerCase().includes(text)) : options;

  useLayoutEffect(() => {
    if (!open || !trigger.current) return;
    const r = trigger.current.getBoundingClientRect();
    const height = list.current?.offsetHeight ?? 200;
    const below = r.bottom + 6;
    setRect({ top: below + height > window.innerHeight - 8 ? Math.max(8, r.top - 6 - height) : below, left: r.left, width: r.width });
  }, [open, shown.length]);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!trigger.current?.contains(t) && !list.current?.contains(t)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // Only the list closes, not the page behind it.
      e.stopPropagation();
      setOpen(false);
      trigger.current?.focus();
    };
    const onShift = (e: Event) => {
      if (!list.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    window.addEventListener("resize", onShift);
    window.addEventListener("scroll", onShift, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey, true);
      window.removeEventListener("resize", onShift);
      window.removeEventListener("scroll", onShift, true);
      setQuery("");
    };
  }, [open]);

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className={`coach-dropdown ${open ? "open" : ""}`}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="coach-ellipsis">{label}</span>
        <svg width={10} height={14} viewBox="0 0 10 14" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M2 5l3-3 3 3M2 9l3 3 3-3" />
        </svg>
      </button>
      {open &&
        createPortal(
          <div
            ref={list}
            className="coach-dropdown-list"
            style={{ top: rect?.top ?? 0, left: rect?.left ?? 0, width: rect?.width ?? 340, visibility: rect ? "visible" : "hidden" }}
          >
            {searchPrompt && (
              <label className="coach-dropdown-search">
                <Icon name="search" size={13} />
                <input autoFocus value={query} placeholder={searchPrompt} aria-label={searchPrompt} onChange={(e) => setQuery(e.target.value)} />
              </label>
            )}
            <div className="coach-dropdown-options" role="listbox" aria-label={ariaLabel}>
              {shown.map((o) => (
                <button
                  key={o.value}
                  type="button"
                  role="option"
                  aria-selected={o.value === value}
                  className={o.value === value ? "selected" : ""}
                  onClick={() => {
                    onChange(o.value);
                    setOpen(false);
                  }}
                >
                  <span className="text">
                    <span className="coach-ellipsis">{o.label}</span>
                    {o.detail && <span className={`detail coach-ellipsis ${o.warns ? "warns" : ""}`}>{o.detail}</span>}
                  </span>
                  {o.value === value && <CoachIcon name="checkmark" size={13} weight={2.4} />}
                </button>
              ))}
              {shown.length === 0 && <span className="none">No matches</span>}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

/** The three jobs, each a dropdown of the models on the server (Mac `jobRow`s). */
export function JobPickers({ info, catalog }: { info: ModelsInfo; catalog: Catalog | null }) {
  const { patch } = useSettings();
  const setup = info.setup;
  const jobs: { key: keyof SettingsPatch & string; title: string; icon: string; summary: string; infoText: string; current: string; value: string; automatic: string }[] = [
    {
      key: "coach_model",
      title: "Coach",
      icon: "textBubble",
      summary: "Writes your note after each rep",
      infoText: "Writes the note after each rep, in your coach's voice. Only numbers Coach counted may appear in a note.",
      current: setup.coach_model,
      value: setup.coach_automatic ? "" : setup.coach_model,
      automatic: info.best_passing ? "Best that passed the judge check" : "No model has passed yet",
    },
    {
      key: "coach_judge_model",
      title: "Judge",
      icon: "checkSeal",
      summary: judgeSummary(info, catalog),
      infoText: "Answers one yes or no question per check, with how sure it is. It's only used once it has passed the judge check.",
      current: setup.judge_model,
      value: setup.judge_automatic ? "" : setup.judge_model,
      automatic: info.best_passing ? "Best that passed the judge check" : "No model has passed yet",
    },
    {
      key: "coach_partner_model",
      title: "Scene partner",
      icon: "bubbles",
      summary: "Plays the people in scenes",
      infoText: "Plays the people in scenes and answers while you wait, so a quick model feels most like talking. Its reply is spoken as it's written.",
      current: setup.scene_model,
      value: setup.partner_model,
      automatic: "Same as Coach",
    },
  ];
  return (
    <div className="coach-surface coach-jobs">
      {jobs.map((job) => {
        const models = info.chat_models.includes(job.current) || !job.current ? info.chat_models : [job.current, ...info.chat_models];
        const options: DropdownOption[] = [
          { value: "", label: "Automatic", detail: job.automatic },
          ...models.map((m) => {
            const d = gradeDetail(info, m, catalog);
            return { value: m, label: m, detail: d, warns: d === "Not installed" || d?.startsWith("Failed") };
          }),
        ];
        const warn = job.summary.startsWith("Failed") || job.summary.startsWith("Not checked");
        return (
          <div key={job.key} className="coach-job">
            <span className="icon">
              <CoachIcon name={job.icon} size={20} />
            </span>
            <div className="coach-col" style={{ gap: 2, flex: 1, minWidth: 0 }}>
              <span className="coach-row" style={{ gap: 6 }}>
                <strong style={{ fontSize: 16 }}>{job.title}</strong>
                <InfoButton title={job.title} text={job.infoText} />
              </span>
              <span style={{ fontSize: 13, color: warn ? "var(--c-warm-ink)" : "var(--c-ink-3)" }}>{job.summary}</span>
            </div>
            <Dropdown
              ariaLabel={`${job.title} model`}
              value={job.value}
              options={options}
              triggerLabel={job.value ? job.current : `Automatic · ${job.current}`}
              searchPrompt="Search models"
              onChange={(v) => patch({ [job.key]: v } as SettingsPatch)}
            />
          </div>
        );
      })}
    </div>
  );
}

function judgeSummary(info: ModelsInfo, catalog: Catalog | null): string {
  const g = info.grades.find((x) => x.model === info.setup.judge_model);
  if (!g) return "Not checked yet: run the judge check";
  return passes(g, catalog)
    ? `Scores your reps · passed the judge check at ${Math.round(g.accuracy * 100)}%`
    : "Failed the judge check, so judged checks are off";
}

function ModelsView({ onBack }: { onBack: () => void }) {
  const { catalog, settings, openSetup } = useCoach();
  const { patch } = useSettings();
  const { info, loading, judge } = useModelsInfo();
  const provider = info?.setup.provider_name ?? "Ollama";
  return (
    <CoachPage
      eyebrow="Cast"
      title="Models"
      accessory={
        <>
          <Tag
            text={loading && !info ? `Checking ${provider}…` : info?.server_running ? `${provider} connected` : `${provider} isn't running`}
            color={info?.server_running ? "var(--c-gain)" : "var(--c-warm-ink)"}
          />
          <Btn onClick={() => openSetup(true)}>Set up models</Btn>
          <Btn onClick={onBack}>Back to coaches</Btn>
        </>
      }
    >
      {!loading && info && !info.server_running && (
        <div className="coach-notice" style={{ fontSize: 15 }}>
          Start Ollama or LM Studio to use models. Until then, reps are scored on the counted checks and your coach uses its own lines.
        </div>
      )}
      {info && <JobPickers info={info} catalog={catalog} />}
      {info && <JudgeCheckCard info={info} judge={judge} />}
      {info && <CompareCard info={info} />}
      <div className="coach-surface coach-jobs">
        <div className="coach-job short">
          <div className="coach-col" style={{ gap: 2, flex: 1 }}>
            <span className="coach-row" style={{ gap: 5 }}>
              <span className="coach-settings-label">Read the note aloud</span>
              <InfoButton title="Read the note aloud" text="Uses this computer's own voice, after each rep." />
            </span>
          </div>
          <Switch label="Read the note aloud" checked={!!settings?.coach_read_aloud} onChange={(v) => patch({ coach_read_aloud: v })} />
        </div>
        <VoiceRow />
        <div className="coach-job short">
          <div className="coach-col" style={{ gap: 2, flex: 1 }}>
            <span className="coach-row" style={{ gap: 5 }}>
              <span className="coach-settings-label">Hearing you</span>
              <InfoButton title="Hearing you" text="Practice uses the same speech engine and Polish as your dictation." />
            </span>
          </div>
          <span style={{ fontSize: 14, color: "var(--c-ink-2)" }}>Same as dictation</span>
        </div>
      </div>
    </CoachPage>
  );
}

/** The system voice Coach speaks with (the Mac downloads natural voices; the
 *  desktop uses the voices this computer already has). */
export function VoiceRow() {
  const { settings, patch } = useSettings();
  const [voices, setVoices] = useState(() => availableVoices());
  useEffect(() => {
    const s = speechSupported() ? window.speechSynthesis : null;
    const update = () => setVoices(availableVoices());
    s?.addEventListener?.("voiceschanged", update);
    const t = window.setTimeout(update, 400);
    return () => {
      s?.removeEventListener?.("voiceschanged", update);
      window.clearTimeout(t);
    };
  }, []);
  return (
    <div className="coach-job voice">
      <div className="coach-col" style={{ gap: 2, flex: 1, minWidth: 0 }}>
        <span className="coach-row" style={{ gap: 5 }}>
          <span className="coach-settings-label">Voice</span>
          <InfoButton
            title="Voice"
            text="Coach and the scene partners speak with the voices installed on this computer. Natural or neural voices sound best; each partner keeps a voice of its own."
          />
        </span>
        {!voices.length && (
          <span style={{ fontSize: 13, color: "var(--c-warm-ink)" }}>
            This system has no voices for apps to use, so Coach shows its words instead.
          </span>
        )}
      </div>
      {voices.length > 0 && (
        <>
          <Dropdown
            ariaLabel="Coach voice"
            value={settings?.coach_voice ?? ""}
            options={[{ value: "", label: "Automatic" }, ...voices.map((v) => ({ value: v.name, label: v.name, detail: v.lang }))]}
            searchPrompt="Search voices"
            onChange={(v) => patch({ coach_voice: v })}
          />
          <Btn compact onClick={() => coachVoice.speak("That's the one. Say what done looks like was clear all the way through.", { voice: settings?.coach_voice ?? "" })}>
            <CoachIcon name="playFill" size={12} />
            Hear
          </Btn>
        </>
      )}
    </div>
  );
}

function JudgeCheckCard({ info, judge }: { info: ModelsInfo; judge: JudgeState | null }) {
  const { catalog } = useCoach();
  const { patch } = useSettings();
  const grades = [...info.grades].sort((a, b) => Number(passes(b, catalog)) - Number(passes(a, catalog)) || b.accuracy - a.accuracy);
  const checking = judge?.state === "checking" ? judge : null;
  return (
    <section className="coach-surface coach-pad tight coach-col" style={{ gap: 14 }}>
      <div className="coach-row coach-wrap" style={{ gap: 6 }}>
        <h2 className="coach-h2" style={{ fontSize: 18 }}>
          Judge check
        </h2>
        <InfoButton
          title="Judge check"
          text={`Each model answers ${catalog.calibration.examples} questions about dictations whose answer is plain: half clearly do the thing, half clearly don't. A model passes at ${Math.round(catalog.calibration.pass_accuracy * 100)}% right, and only if it gets both kinds right, so one that always says no can't pass. Only a model that passed is used as judge.`}
        />
        <span className="coach-spacer" />
        {checking ? (
          <span className="coach-row" style={{ gap: 10 }}>
            <span className="coach-bar" style={{ width: 90 }}>
              <span style={{ width: `${(checking.done / Math.max(checking.total, 1)) * 100}%` }} />
            </span>
            <span className="coach-label">
              Checking {checking.model} · {checking.done} of {checking.total}
            </span>
          </span>
        ) : (
          <Btn disabled={info.chat_models.length === 0} onClick={() => coachApi.checkJudges(info.chat_models)}>
            {grades.length ? "Check again" : "Check my models"}
          </Btn>
        )}
      </div>
      {grades.length === 0 && (
        <span className="coach-label">
          {info.chat_models.length ? "Each model takes a few seconds, longer the first time it loads." : "Needs a model on your server."}
        </span>
      )}
      {grades.map((g) => {
        const ok = passes(g, catalog);
        const judging = !info.setup.judge_automatic && info.setup.judge_model === g.model;
        return (
          <div key={g.model} className="coach-grade-row">
            <strong className="coach-ellipsis">{g.model}</strong>
            <span className="coach-spacer" />
            <span style={{ fontSize: 14, color: "var(--c-ink-2)" }}>{Math.round(g.accuracy * 100)}% right</span>
            <span className="ms">{g.ms_per_answer == null ? "–" : g.ms_per_answer < 1000 ? `${g.ms_per_answer} ms` : `${(g.ms_per_answer / 1000).toFixed(1)} s`}</span>
            <span title={ok ? "Right on the yes and the no examples alike." : info.why_failed[g.model]}>
              <Tag text={ok ? "Passed" : "Failed"} color={ok ? "var(--c-gain)" : "var(--c-warm-ink)"} />
            </span>
            <button type="button" className="coach-link" disabled={!ok || judging} onClick={() => patch({ coach_judge_model: g.model })} style={{ width: 90, textAlign: "right" }}>
              {judging ? "Judging" : "Use as judge"}
            </button>
          </div>
        );
      })}
    </section>
  );
}

function CompareCard({ info }: { info: ModelsInfo }) {
  const { catalog } = useCoach();
  const { patch } = useSettings();
  const [comparing, setComparing] = useState<string | null>(null);
  const [result, setResult] = useState<Comparison | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEvent<{ model: string }>("coach-compare-progress", (p) => setComparing((c) => (c ? p.model : c)));
  const run = async () => {
    setComparing(info.chat_models[0] ?? "…");
    setError(null);
    try {
      const r = await coachApi.compare();
      setResult(r);
      if (!r.question) setError("Needs a few recent dictations in your busiest way of talking.");
    } catch (e) {
      setError(String(e));
    } finally {
      setComparing(null);
    }
  };
  const question = result?.question ? catalog.questions.find((q) => q.id === result.question)?.label : null;
  return (
    <section className="coach-surface coach-pad tight coach-col" style={{ gap: 14 }}>
      <div className="coach-row coach-wrap" style={{ gap: 6 }}>
        <h2 className="coach-h2" style={{ fontSize: 18 }}>
          Compare on my dictations
        </h2>
        <InfoButton
          title="Compare on my dictations"
          text="Each model judges the same 20 of your recent dictations on one question. Agreement is how often it matched the majority. Where they agree, pick the faster one. Nothing is kept."
        />
        <span className="coach-spacer" />
        {comparing ? (
          <span className="coach-row" style={{ gap: 8 }}>
            <Spinner small />
            <span className="coach-label">Asking {comparing}…</span>
          </span>
        ) : (
          <Btn disabled={info.chat_models.length < 2} onClick={run}>
            Compare
          </Btn>
        )}
      </div>
      {error && <span className="coach-label">{error}</span>}
      {question && result && result.rows.length > 0 ? (
        <>
          <span className="coach-label">On “{question.toLowerCase()}”</span>
          {result.rows.map((row) => {
            const judging = !info.setup.judge_automatic && info.setup.judge_model === row.model;
            return (
              <div key={row.model} className="coach-grade-row">
                <strong className="coach-ellipsis">{row.model}</strong>
                <span className="coach-spacer" />
                <span style={{ fontSize: 14, color: "var(--c-ink-2)" }}>
                  {row.agreement == null ? "No answers" : `${Math.round(row.agreement * 100)}% agree`}
                </span>
                <span className="ms">{row.ms_per_answer == null ? "–" : `${row.ms_per_answer} ms`}</span>
                <button type="button" className="coach-link" disabled={judging} onClick={() => patch({ coach_judge_model: row.model })}>
                  {judging ? "Judging" : "Use as judge"}
                </button>
              </div>
            );
          })}
        </>
      ) : (
        info.chat_models.length < 2 && <span className="coach-label">Needs at least two models on your server.</span>
      )}
    </section>
  );
}

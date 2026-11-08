// Coach (Mac CoachShellView): a space of its own with a floating tab bar —
// Today, Path, Studio, Proof and Cast — the first-run welcome, focus-mode reps
// and scenes, the month report and the model setup card. (Coach area.)
import { useCallback, useEffect, useMemo, useState } from "react";
import "../styles/coach.css";
import { Icon, useToast } from "../ui";
import { useSettings } from "../lib/settings";
import { coachApi, useCatalog, useCoachData, type RepKind, type RepPlan } from "../lib/coach";
import { CoachIcon } from "./coach/icons";
import { Btn, CoachCtx, Eyebrow, Loading, type CoachContextValue, type CoachTab } from "./coach/parts";
import Welcome from "./coach/Welcome";
import Today from "./coach/Today";
import PathView from "./coach/Path";
import Studio from "./coach/Studio";
import Proof from "./coach/Proof";
import Month from "./coach/Month";
import Cast from "./coach/Cast";
import RepSession from "./coach/Session";
import SceneSession from "./coach/Scene";
import { WeekShareSheet } from "./coach/Cards";
import { SetupHost, useSetupOffer } from "./coach/ModelSetup";

const TABS: { id: CoachTab; title: string; icon: string }[] = [
  { id: "today", title: "Today", icon: "sunMax" },
  { id: "path", title: "Path", icon: "curvePath" },
  { id: "studio", title: "Studio", icon: "mic" },
  { id: "proof", title: "Proof", icon: "chartUp" },
  { id: "cast", title: "Cast", icon: "person2" },
];

type Active = { kind: "rep"; plan: RepPlan } | { kind: "scene"; sceneId: string; key: number } | null;

export default function Coach() {
  const { settings, patch } = useSettings();
  const toast = useToast();
  const catalog = useCatalog();
  const { snapshot, loaded, error, judge, refresh } = useCoachData();
  const [tab, setTab] = useState<CoachTab>("today");
  const [active, setActive] = useState<Active>(null);
  const [showingMonth, setShowingMonth] = useState(false);
  const [sharingWeek, setSharingWeek] = useState(false);
  const [setup, setSetup] = useState<{ open: boolean; atSetup: boolean }>({ open: false, atSetup: false });

  const enabled = settings?.coach_enabled ?? true;
  const firstRunDone = settings?.coach_first_run_done ?? false;

  useEffect(() => setShowingMonth(false), [tab]);
  // Leaving Coach mid-rep: nothing stays armed, judging may resume.
  useEffect(() => () => void coachApi.endSession().catch(() => {}), []);

  const openSetup = useCallback((atSetup = false) => setSetup({ open: true, atSetup }), []);
  useSetupOffer(enabled && firstRunDone && !!settings && !settings.coach_model_setup_seen, () => openSetup(false));

  const beginRep = useCallback(
    (skillId: string, kind: RepKind, spoken: boolean) => {
      coachApi
        .beginRep(skillId, kind, spoken)
        .then((plan) => setActive({ kind: "rep", plan }))
        .catch((e) => toast(String(e), "error"));
    },
    [toast],
  );
  const beginScene = useCallback((sceneId: string) => setActive({ kind: "scene", sceneId, key: Date.now() }), []);
  const endActive = useCallback(() => {
    setActive(null);
    coachApi.endSession().catch(() => {});
    refresh();
  }, [refresh]);

  const value: CoachContextValue | null = useMemo(
    () =>
      snapshot && catalog
        ? { snapshot, catalog, judge, settings, refresh, beginRep, beginScene, setTab, openSetup }
        : null,
    [snapshot, catalog, judge, settings, refresh, beginRep, beginScene, openSetup],
  );

  let body;
  if (!settings || !catalog) {
    body = <Loading />;
  } else if (!enabled) {
    body = (
      <div className="coach-loading">
        <div className="coach-col" style={{ alignItems: "center", gap: 16 }}>
          <Eyebrow>Coach</Eyebrow>
          <h1 className="coach-display" style={{ fontSize: 34, margin: 0 }}>
            Coach is off.
          </h1>
          <Btn kind="primary" onClick={() => patch({ coach_enabled: true })}>
            Turn on Coach
          </Btn>
        </div>
      </div>
    );
  } else if (!loaded || !value) {
    body = error ? (
      <div className="coach-loading">
        <div className="coach-col" style={{ alignItems: "center" }}>
          <p>Coach couldn't read your dictations: {error}</p>
          <Btn onClick={refresh}>Try again</Btn>
        </div>
      </div>
    ) : (
      <Loading />
    );
  } else if (!firstRunDone) {
    body = (
      <CoachCtx.Provider value={value}>
        <Welcome onDone={() => patch({ coach_first_run_done: true })} />
      </CoachCtx.Provider>
    );
  } else if (active?.kind === "rep") {
    body = (
      <CoachCtx.Provider value={value}>
        <RepSession plan={active.plan} onPlan={(plan) => setActive({ kind: "rep", plan })} onExit={endActive} />
      </CoachCtx.Provider>
    );
  } else if (active?.kind === "scene") {
    body = (
      <CoachCtx.Provider value={value}>
        <SceneSession key={active.key} sceneId={active.sceneId} onScene={beginScene} onExit={endActive} />
      </CoachCtx.Provider>
    );
  } else {
    body = (
      <CoachCtx.Provider value={value}>
        <div className="coach-tab-page" key={tab}>
          {tab === "today" && <Today onShowProof={() => setTab("proof")} onShareWeek={() => setSharingWeek(true)} />}
          {tab === "path" && <PathView />}
          {tab === "studio" && <Studio />}
          {tab === "proof" &&
            (showingMonth ? <Month onBack={() => setShowingMonth(false)} /> : <Proof onShowMonth={() => setShowingMonth(true)} />)}
          {tab === "cast" && <Cast />}
        </div>
        <nav className="coach-tabbar" aria-label="Coach">
          <div className="coach-tabbar-inner" role="tablist">
            {TABS.map((t) => (
              <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} onClick={() => setTab(t.id)}>
                {t.icon === "mic" ? <Icon name="mic" size={17} /> : <CoachIcon name={t.icon} size={17} />}
                {t.title}
              </button>
            ))}
          </div>
        </nav>
        {sharingWeek && <WeekShareSheet onClose={() => setSharingWeek(false)} />}
      </CoachCtx.Provider>
    );
  }

  return (
    <div className="coach coach-root">
      <div className="coach-drag" aria-hidden="true">
        <div data-tauri-drag-region />
      </div>
      {body}
      {value && setup.open && (
        <CoachCtx.Provider value={value}>
          <SetupHost
            atSetup={setup.atSetup}
            onClose={() => {
              setSetup({ open: false, atSetup: false });
              if (!settings?.coach_model_setup_seen) patch({ coach_model_setup_seen: true });
              refresh();
            }}
          />
        </CoachCtx.Provider>
      )}
    </div>
  );
}

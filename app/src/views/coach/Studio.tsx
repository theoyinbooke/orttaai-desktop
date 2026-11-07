// Studio (Mac CoachStudioView): today's rep, every scene, and where each
// track left off. Starting one opens it full screen, by voice or by typing.
import { Icon } from "../../ui";
import { lowerFirst, personaFor, type Scene, type TrackId } from "../../lib/coach";
import PersonaGlyph from "./PersonaGlyph";
import { Btn, CoachPage, Eyebrow, kindTitle, skillOf, trackOf, useCoach } from "./parts";

export default function Studio() {
  const { snapshot, catalog, settings, beginRep } = useCoach();
  const skill = skillOf(catalog, snapshot.todays_skill);
  const track = trackOf(catalog, skill.track);
  const persona = personaFor(settings, track.mode ?? snapshot.busiest_mode);
  return (
    <CoachPage eyebrow="Studio" title="Practise">
      <section className="coach-surface warm coach-studio-today">
        <PersonaGlyph persona={persona} size={48} />
        <div className="coach-col" style={{ gap: 4, flex: 1, minWidth: 0 }}>
          <div className="coach-row" style={{ gap: 6 }}>
            <Eyebrow>Today's rep</Eyebrow>
            <span className="coach-warm-label coach-ellipsis">
              · {track.title} · {kindTitle(catalog, snapshot.todays_kind)}
            </span>
          </div>
          <div className="coach-row coach-wrap" style={{ alignItems: "baseline", gap: 12 }}>
            <span className="coach-display" style={{ fontSize: 24 }}>
              {skill.title}
            </span>
            <span style={{ fontSize: 14, color: "var(--c-ink-2)" }}>{skill.why}</span>
          </div>
        </div>
        <div className="coach-row">
          <Btn kind="primary" onClick={() => beginRep(skill.id, snapshot.todays_kind, true)}>
            <Icon name="mic" size={16} />
            Speak it
          </Btn>
          {!skill.speak_only && <Btn onClick={() => beginRep(skill.id, snapshot.todays_kind, false)}>Type it</Btn>}
        </div>
      </section>

      <section className="coach-col">
        <h3 className="coach-h3">Scenes</h3>
        <div className="coach-grid" style={{ ["--min" as string]: "300px" }}>
          {catalog.scenes.map((scene) => (
            <SceneTile key={scene.id} scene={scene} />
          ))}
        </div>
      </section>

      <section className="coach-col">
        <h3 className="coach-h3">Where each track left off</h3>
        <div className="coach-grid" style={{ ["--min" as string]: "250px" }}>
          {catalog.tracks.map((t) => (
            <TrackTile key={t.id} track={t.id} />
          ))}
        </div>
      </section>
    </CoachPage>
  );
}

function SceneTile({ scene }: { scene: Scene }) {
  const { beginScene } = useCoach();
  const role = lowerFirst(scene.partner_role);
  const partner = `${scene.partner_name}, ${role}`;
  const short = `${scene.partner_name}, ${role.split(",")[0]}`;
  return (
    <button
      type="button"
      className="coach-surface coach-scene-tile coach-plain"
      onClick={() => beginScene(scene.id)}
      title={`${scene.title} — ${partner}. ${scene.goal}`}
      aria-label={`Scene: ${scene.title}, with ${partner}. Goal: ${scene.goal}`}
    >
      <span className="initial coach-display">{scene.partner_name[0]}</span>
      <span className="coach-col" style={{ gap: 2, flex: 1, minWidth: 0, textAlign: "left" }}>
        <strong className="coach-ellipsis" style={{ fontSize: 15 }}>
          {scene.title}
        </strong>
        <span className="coach-label coach-ellipsis">{short}</span>
        <span className="coach-ellipsis" style={{ fontSize: 13, color: "var(--c-ink-2)" }}>
          {scene.goal}
        </span>
      </span>
      <span style={{ color: "var(--c-ink-3)" }}>
        <Icon name="chevronRight" size={16} />
      </span>
    </button>
  );
}

function TrackTile({ track }: { track: TrackId }) {
  const { snapshot, catalog, beginRep } = useCoach();
  const t = trackOf(catalog, track);
  const p = snapshot.tracks.find((x) => x.track === track);
  const skill = skillOf(catalog, p?.current_skill ?? t.skills[0]);
  const progress = snapshot.skill_progress[skill.id];
  const strong = progress?.strong_reps ?? 0;
  return (
    <section
      className="coach-surface coach-track-tile"
      aria-label={`${t.title}: ${skill.title}, ${strong} of ${catalog.reps_to_master} strong reps`}
    >
      <span style={{ fontSize: 12, color: "var(--c-ink-3)" }}>{t.title}</span>
      <strong className="coach-ellipsis" title={skill.title} style={{ fontSize: 15 }}>
        {skill.title}
      </strong>
      <div className="coach-row" style={{ gap: 6, marginTop: 4 }}>
        {Array.from({ length: catalog.reps_to_master }, (_, i) => (
          <span key={i} className={`coach-dot ${i < strong ? "on" : ""}`} style={{ width: 7, height: 7, background: i < strong ? undefined : "var(--w-12)" }} />
        ))}
        {progress?.usual != null && <span style={{ fontSize: 12, color: "var(--c-ink-3)" }}>usual {progress.usual.toFixed(1)}</span>}
        <span className="coach-spacer" />
        <Btn compact onClick={() => beginRep(skill.id, "drill", true)}>
          Speak
        </Btn>
        {!skill.speak_only && (
          <Btn compact onClick={() => beginRep(skill.id, "drill", false)}>
            Type
          </Btn>
        )}
      </div>
    </section>
  );
}

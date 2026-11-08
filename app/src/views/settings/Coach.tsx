// Settings → Coach (Mac CoachSettingsView): Coach on or off, setup, and which
// way of talking each app counts as. The coach, its models and its voice live
// in Coach › Cast, as they do on the Mac. Apps can also be added by hand here,
// since Linux and Windows have no bundle IDs to find them by. (Coach area.)
import { useCallback, useEffect, useState } from "react";
import "../../styles/coach.css";
import { Button, Select, useToast } from "../../ui";
import { Row, Section, ToggleRow, usePatch } from "../../components/SettingsRows";
import { useSettings } from "../../lib/settings";
import { useEvent } from "../../lib/api";
import { coachApi, type AppRow, type Rule } from "../../lib/coach";

const RULES: { value: Rule; label: string }[] = [
  { value: "ai", label: "Talking to AI" },
  { value: "people", label: "Talking to people" },
  { value: "writing", label: "Thinking and writing" },
  { value: "elsewhere", label: "Elsewhere" },
  { value: "off", label: "Off" },
];

export default function CoachSettings() {
  const { settings } = useSettings();
  const save = usePatch();
  const toast = useToast();
  const [apps, setApps] = useState<AppRow[] | null>(null);
  const [newApp, setNewApp] = useState("");
  const [newRule, setNewRule] = useState<Rule>("ai");

  const loadApps = useCallback(() => {
    coachApi
      .snapshot()
      .then((s) => setApps(s?.apps.filter((a) => a.key) ?? []))
      .catch(() => setApps([]));
  }, []);
  useEffect(loadApps, [loadApps]);
  useEvent("coach-changed", loadApps);

  if (!settings) return null;
  const enabled = settings.coach_enabled;
  const listedKeys = new Set((apps ?? []).map((a) => a.key));
  // Choices made by hand for apps Coach hasn't seen yet.
  const extra = Object.entries(settings.coach_app_overrides).filter(([key]) => !listedKeys.has(key));

  const addApp = () => {
    const name = newApp.trim();
    if (!name) return;
    coachApi.setAppRule({ key: "", app_id: null, app_name: name }, newRule).then(() => {
      setNewApp("");
      loadApps();
    });
  };

  return (
    <>
      <Section title="Coach">
        <ToggleRow
          k="coach_enabled"
          label="Coach"
          hint="Counts fillers, pace and sentence length in your dictations, on this computer, and checks whether habits you start stick. It stores numbers only."
        />
        <Row label="Setup" hint="Shows Coach's first-run steps again the next time you open it.">
          <Button size="sm" className="st-btn" disabled={!settings.coach_first_run_done} onClick={() => save({ coach_first_run_done: false })}>
            Run Again
          </Button>
        </Row>
      </Section>

      <Section
        title="Apps"
        desc="Coach groups your dictations by the app they went to. Move an app to another way of talking, or switch it off so Coach never reads it."
      >
        {apps !== null && apps.length === 0 && extra.length === 0 && (
          <p className="coach-settings-foot">Apps appear here after your first dictations.</p>
        )}
        {(apps ?? []).map((a) => (
          <Row key={a.key} label={a.display_name} disabled={!enabled}>
            <Select ariaLabel={a.display_name} value={a.rule} options={RULES} onChange={(rule) => coachApi.setAppRule(a, rule).then(loadApps)} />
          </Row>
        ))}
        {extra.map(([key, rule]) => (
          <Row key={key} label={key.startsWith("name:") ? key.slice(5) : key} hint="Added by hand" disabled={!enabled}>
            <Select
              ariaLabel={key}
              value={rule as Rule}
              options={RULES}
              onChange={(r) => coachApi.setAppRule({ key, app_id: null, app_name: key.startsWith("name:") ? key.slice(5) : null }, r)}
            />
            <Button size="sm" variant="ghost" className="st-btn" onClick={() => coachApi.removeAppRule(key).catch((e) => toast(String(e), "error"))}>
              Remove
            </Button>
          </Row>
        ))}
        <Row label="Add an app" hint="Its name or program, such as firefox, slack or Code.exe." disabled={!enabled}>
          <input
            className="text-input"
            value={newApp}
            placeholder="App name"
            aria-label="App name"
            onChange={(e) => setNewApp(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addApp()}
            style={{ width: 160, height: 32, padding: "0 10px" }}
          />
          <Select ariaLabel="Way of talking" value={newRule} options={RULES} onChange={setNewRule} />
          <Button size="sm" className="st-btn" disabled={!newApp.trim()} onClick={addApp}>
            Add
          </Button>
        </Row>
      </Section>
    </>
  );
}

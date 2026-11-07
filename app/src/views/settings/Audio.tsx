// Settings → Audio (Mac AudioSettingsView): input device, live level, reset.
// Owned by the engine area.
import { useCallback, useEffect, useState } from "react";
import { Button, LevelMeter, Select } from "../../ui";
import { call } from "../../lib/api";
import { useSettings } from "../../lib/settings";
import { useEngine, type AudioDevice } from "../../lib/engine";
import { Row, Section, usePatch } from "../../components/SettingsRows";
import "../../styles/engine.css";

const DEFAULT = "__default__";

/** Bluetooth headset mics switch the headphones to call quality (Mac notice). */
const isHeadset = (name: string) => /bluetooth|headset|hands-?free|airpods|bt /i.test(name);

/** The live meter: starts the backend level monitor for `device` while mounted. */
export function useLevelMonitor(device: string | null | undefined, enabled = true) {
  const [metered, setMetered] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    call<string | null>("start_level_monitor", { device: device ?? null })
      .then((name) => {
        if (live) {
          setMetered(name);
          setError(null);
        }
      })
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
      call("stop_level_monitor").catch(() => {});
    };
  }, [device, enabled]);
  return { metered, error };
}

export default function AudioSettings() {
  const { settings } = useSettings();
  const save = usePatch();
  const { monitorLevel } = useEngine();
  const [devices, setDevices] = useState<AudioDevice[] | null>(null);
  const [resetting, setResetting] = useState(false);
  const [resetMessage, setResetMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const loadDevices = useCallback(() => {
    call<AudioDevice[]>("list_audio_devices")
      .then(setDevices)
      .catch(() => setDevices([]));
  }, []);
  useEffect(loadDevices, [loadDevices]);

  const selected = settings?.audio_device ?? null;
  const { metered, error } = useLevelMonitor(selected, !!settings);
  if (!settings) return null;

  const defaultDevice = devices?.find((d) => d.is_default);
  const missing = selected && devices && !devices.some((d) => d.name === selected);
  const options = [
    {
      value: DEFAULT,
      label: defaultDevice ? `System Default (${defaultDevice.name})` : "System Default",
    },
    ...(devices ?? []).map((d) => ({ value: d.name, label: d.name })),
    ...(missing ? [{ value: selected!, label: `${selected} (unavailable)` }] : []),
  ];
  const activeName = metered ?? selected ?? defaultDevice?.name ?? "System Default";

  const reset = () => {
    setResetting(true);
    setResetMessage({ ok: true, text: "Resetting audio pipeline…" });
    call<string>("reset_audio")
      .then((m) => setResetMessage({ ok: true, text: m }))
      .catch((e) => setResetMessage({ ok: false, text: String(e) || "Audio reset failed." }))
      .finally(() => {
        setResetting(false);
        loadDevices();
      });
  };

  return (
    <>
      <Section title="Microphone">
        <Row
          label="Input Device"
          hint="The microphone Orttaai records from. System Default follows the input chosen in your system's sound settings."
        >
          <Select
            value={selected ?? DEFAULT}
            options={options}
            onChange={(v) => save({ audio_device: v === DEFAULT ? null : v })}
            ariaLabel="Input device"
            className="eng-select"
          />
        </Row>
        <Row
          label="Input Level"
          hint="Speak to check the level moves. If it stays flat, check your system's microphone permission or choose another device."
        >
          <div className="eng-meter" aria-label={`Input level for ${activeName}`}>
            <LevelMeter level={monitorLevel} active={!error} />
            <span className="small muted eng-meter-label">{error ? "Meter unavailable" : activeName}</span>
          </div>
        </Row>
        {missing && (
          <div className="eng-notice warn">
            “{selected}” isn't connected. Orttaai records from the system default until it's back.
          </div>
        )}
        {isHeadset(activeName) && (
          <div className="eng-notice">
            This is a Bluetooth headset mic. Recording with it switches the headphones to call quality and can take a
            moment to start. For the clearest dictation, choose your computer's microphone here and keep the headphones
            for listening.
          </div>
        )}
        {devices && devices.length === 0 && (
          <div className="eng-notice warn">
            No audio input devices detected. Reconnect a microphone and reopen this section.
          </div>
        )}
        {error && <div className="eng-notice warn">{error}</div>}
      </Section>

      <Section title="Troubleshooting">
        <Row
          label="Reset Audio"
          hint="Restarts audio capture. Use it if recording stops picking up sound, for example after your computer wakes from sleep or a microphone reconnects."
        >
          <Button onClick={reset} loading={resetting} icon="refresh">
            {resetting ? "Resetting…" : "Reset"}
          </Button>
        </Row>
        {resetMessage && (
          <div className={`eng-notice ${resetMessage.ok ? "" : "warn"}`} role="status">
            {resetMessage.text}
          </div>
        )}
      </Section>
    </>
  );
}

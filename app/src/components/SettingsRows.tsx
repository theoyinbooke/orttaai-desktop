// Shared building blocks for every Settings section (Mac SettingsComponents).
import type { ReactNode } from "react";
import { Switch } from "../ui";
import { useSettings, type Settings, type SettingsPatch } from "../lib/settings";
import { useToast } from "../ui";

/** A titled group of rows. */
export function Section({ title, desc, children }: { title: string; desc?: ReactNode; children: ReactNode }) {
  return (
    <section className="set-section">
      <header className="set-section-head">
        <h2>{title}</h2>
        {desc && <p className="muted small">{desc}</p>}
      </header>
      <div className="set-rows">{children}</div>
    </section>
  );
}

/** Label + hint on the left, control on the right. */
export function Row({
  label,
  hint,
  children,
  disabled,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`set-row ${disabled ? "is-disabled" : ""}`}>
      <div className="set-row-text">
        <span className="set-row-label">{label}</span>
        {hint && <span className="set-row-hint">{hint}</span>}
      </div>
      <div className="set-row-control">{children}</div>
    </div>
  );
}

/** Save a patch immediately, toasting any error. */
export function usePatch() {
  const { patch } = useSettings();
  const toast = useToast();
  return (p: SettingsPatch) => patch(p).catch((e) => toast(String(e), "error"));
}

type BoolKey = { [K in keyof Settings]: Settings[K] extends boolean ? K : never }[keyof Settings];

/** A row bound to a boolean setting. */
export function ToggleRow({
  k,
  label,
  hint,
  disabled,
}: {
  k: BoolKey;
  label: ReactNode;
  hint?: ReactNode;
  disabled?: boolean;
}) {
  const { settings } = useSettings();
  const save = usePatch();
  if (!settings) return null;
  return (
    <Row label={label} hint={hint} disabled={disabled}>
      <Switch
        checked={settings[k] as boolean}
        onChange={(v: boolean) => save({ [k]: v } as SettingsPatch)}
        disabled={disabled}
      />
    </Row>
  );
}

/** Number slider with a live value readout. */
export function SliderRow({
  label,
  hint,
  value,
  min,
  max,
  step = 1,
  format = (v) => String(v),
  onCommit,
  disabled,
}: {
  label: ReactNode;
  hint?: ReactNode;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  onCommit: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <Row label={label} hint={hint} disabled={disabled}>
      <div className="slider">
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          defaultValue={value}
          key={value}
          disabled={disabled}
          onChange={(e) => {
            const out = e.currentTarget.parentElement?.querySelector("output");
            if (out) out.textContent = format(Number(e.currentTarget.value));
          }}
          onPointerUp={(e) => onCommit(Number((e.target as HTMLInputElement).value))}
          onKeyUp={(e) => onCommit(Number((e.target as HTMLInputElement).value))}
        />
        <output className="mono small">{format(value)}</output>
      </div>
    </Row>
  );
}

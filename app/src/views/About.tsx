// About (Mac AboutView): app card with updates, open source, acknowledgments, in
// the settings column. The credits name what this port actually runs on.
import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button, Icon, PageHeader, useToast } from "../ui";
import type { AppInfo } from "../types";
import { useSettings } from "../lib/settings";
import { phaseLabel, useUpdater, type UpdatePhase } from "../lib/updater";
import { Glyph, Notice, Row, Section, ToggleRow } from "../components/SettingsRows";

const REPO = "https://github.com/theoyinbooke/orttaai-desktop";
const GITHUB = "https://github.com/theoyinbooke";
const YOUTUBE = "https://youtube.com/c/theoyinbooke";

const glyph = (name: string) => <Glyph name={name} size={13} weight={1.75} />;
const icon = (name: string) => <Icon name={name} size={13} />;

const CREDITS: { name: string; detail: string; icon: ReactNode }[] = [
  { name: "whisper.cpp", detail: "On-device speech recognition, MIT License", icon: icon("waveform") },
  { name: "Parakeet TDT 0.6B", detail: "Express speech model by NVIDIA, CC BY 4.0", icon: icon("bolt") },
  { name: "sherpa-onnx", detail: "Runs Express on this computer, Apache 2.0", icon: glyph("cpu") },
  { name: "Parakeet CTC 110M", detail: "Word Boost model by NVIDIA, CC BY 4.0", icon: icon("textCheck") },
  { name: "FluidAudio", detail: "Word Boost's rescoring, ported from FluidInference, Apache 2.0", icon: icon("textCheck") },
  { name: "Silero VAD", detail: "Hears when you stop talking in hands-free recordings, MIT License", icon: icon("waveform") },
  {
    name: "S1-mini",
    detail: "Text Polish model by Superwhisper, Apache 2.0, built on Qwen3 0.6B by Alibaba Cloud",
    icon: glyph("alignLeft"),
  },
  { name: "Ollama and LM Studio", detail: "Run S1-mini and your local AI models on this computer", icon: glyph("cpu") },
  { name: "SQLite and rusqlite", detail: "Local database, public domain and MIT License", icon: glyph("cylinder") },
  { name: "Tauri", detail: "App framework and auto-update, MIT and Apache 2.0", icon: glyph("downCircle") },
  { name: "cpal", detail: "Audio capture, Apache 2.0", icon: icon("mic") },
  { name: "global-hotkey", detail: "Shortcut recording, MIT and Apache 2.0", icon: icon("keyboard") },
];

/** The Mac's BrandIconLight / BrandIconDark, drawn: the rounded tile and gold mark. */
function AppIcon() {
  return (
    <svg className="st-about-icon" viewBox="0 0 1024 1024" aria-hidden="true">
      <rect x="56" y="56" width="912" height="912" rx="200" className="st-about-tile" />
      <g fill="#D4952A">
        <rect x="210" y="524" width="109" height="218" rx="54.5" />
        <rect x="361" y="427" width="109" height="315" rx="54.5" />
        <rect x="512" y="330" width="109" height="412" rx="54.5" />
        <rect x="723" y="276" width="37" height="472" />
        <rect x="675" y="258" width="133" height="37" rx="18.5" />
        <rect x="675" y="729" width="133" height="37" rx="18.5" />
      </g>
    </svg>
  );
}

/** The signal-bars mark, oversized and faded behind the header. */
function BackdropMark() {
  return (
    <svg className="st-about-mark" viewBox="0 0 100 84" aria-hidden="true">
      <rect x="0" y="44" width="18" height="36" rx="9" />
      <rect x="25" y="28" width="18" height="52" rx="9" />
      <rect x="50" y="12" width="18" height="68" rx="9" />
      <rect x="85" y="3" width="6" height="78" rx="3" />
      <rect x="77" y="0" width="22" height="6" rx="3" />
      <rect x="77" y="78" width="22" height="6" rx="3" />
    </svg>
  );
}

function updateLabel(phase: UpdatePhase) {
  return phase.kind === "idle" ? "Check for Updates…" : phaseLabel(phase);
}

/** A new GitHub issue prefilled with the version (Mac AppLinks.newIssueURL). */
function issueUrl(kind: "bug" | "support", info: AppInfo | null) {
  const body = [
    `### ${kind === "bug" ? "What happened?" : "What do you need help with?"}`,
    "",
    "### Steps to reproduce",
    "1.",
    "2.",
    "3.",
    "",
    "### Expected behavior",
    "",
    "### Actual behavior",
    "",
    "### Environment",
    `- App version: ${info?.version ?? "?"}`,
    `- System: ${info?.platform ?? "?"} · ${info?.session ?? "?"} · ${info?.gpu_backend ?? "?"}`,
    "- Model:",
  ].join("\n");
  const q = new URLSearchParams({
    title: kind === "bug" ? "[Bug] " : "[Support] ",
    labels: kind,
    body,
  });
  return `${REPO}/issues/new?${q.toString()}`;
}

export default function About({ info }: { info: AppInfo | null; onFeedback: () => void }) {
  const { settings } = useSettings();
  const { phase, act } = useUpdater(settings?.auto_download_updates ?? true);
  const toast = useToast();
  const open = (url: string) => () => {
    openUrl(url).catch((e) => toast(String(e), "error"));
  };
  const version = info?.version ?? "—";
  const environment = `${info?.platform ?? "—"} · ${info?.session ?? "—"} · ${info?.gpu_backend ?? "cpu"} build`;

  return (
    <div className="view">
      <PageHeader title="About" />
      <div className="st-column">
        <div className="st-about-hero-card">
          <div className="st-about-hero">
            <div className="st-about-wash" />
            <BackdropMark />
            <AppIcon />
            <div className="st-about-id">
              <span className="st-about-name">Orttaai</span>
              <span className="st-about-tag">Private, on-device dictation for your computer.</span>
              <span className="st-chip" title={environment}>
                <Glyph name="seal" size={12} weight={1.75} />
                Version {version} ({info?.platform ?? "—"})
              </span>
            </div>
          </div>
          <div className="set-rows">
            <Row
              label="Updates"
              hint="Orttaai checks every few hours, and shortly after it opens. A new version shows at the bottom left of the sidebar."
            >
              <Button
                size="sm"
                className="st-btn"
                onClick={act}
                disabled={phase.kind === "checking" || phase.kind === "downloading" || phase.kind === "installing"}
              >
                {updateLabel(phase)}
              </Button>
            </Row>
            {phase.kind === "failed" && <Notice kind="error">{phase.message}</Notice>}
            <ToggleRow
              k="auto_download_updates"
              label="Automatically download updates"
              hint="Downloads a new version as soon as it's found, with its progress at the bottom left. Click Restart to update there; Orttaai never restarts without your click."
            />
          </div>
        </div>

        <Section
          title="Open Source"
          desc="Orttaai is free and open source under the MIT License. Bug reports and support requests open a GitHub issue prefilled with your version."
        >
          <div className="st-about-maker">
            <div className="st-about-maker-who">
              <small>Made by</small>
              <strong>Olanrewaju Oyinbooke</strong>
            </div>
            <Button size="sm" className="st-link" title="Open github.com/theoyinbooke" onClick={open(GITHUB)}>
              <Glyph name="code" size={12} weight={1.75} />
              GitHub
            </Button>
            <Button size="sm" className="st-link" title="Open youtube.com/c/theoyinbooke" onClick={open(YOUTUBE)}>
              <Glyph name="playRect" size={12} weight={1.75} />
              YouTube
            </Button>
          </div>
          <div className="st-about-divider" />
          <div className="st-about-actions">
            <Button size="sm" className="st-btn" onClick={open(issueUrl("bug", info))}>
              <Glyph name="ant" size={12} weight={1.75} />
              Report a Bug
            </Button>
            <Button size="sm" className="st-btn" onClick={open(issueUrl("support", info))}>
              <Glyph name="lifebuoy" size={12} weight={1.75} />
              Get Support
            </Button>
            <Button size="sm" className="st-btn" onClick={open(REPO)}>
              <Glyph name="externalSquare" size={12} weight={1.75} />
              Contribute on GitHub
            </Button>
            <span className="st-about-fill" />
            {/* Desktop only: the release page and licence, kept from the earlier About page. */}
            <Button size="sm" className="st-link" onClick={open(`${REPO}/releases/latest`)}>
              Latest Release
            </Button>
            <Button size="sm" className="st-link" onClick={open(`${REPO}/blob/main/LICENSE`)}>
              License
            </Button>
          </div>
        </Section>

        <Section title="Acknowledgments">
          <div className="st-credits">
            {CREDITS.map((c) => (
              <div key={c.name} className="st-credit">
                <span className="st-credit-icon">{c.icon}</span>
                <span className="st-credit-text">
                  <strong>{c.name}</strong>
                  <span>{c.detail}</span>
                </span>
              </div>
            ))}
          </div>
        </Section>
      </div>
    </div>
  );
}

import { openUrl } from "@tauri-apps/plugin-opener";
import { Card, Icon, PageHeader } from "../ui";
import type { AppInfo } from "../types";

const REPO = "https://github.com/theoyinbooke/orttaai-desktop";
const CONTRIBUTOR_GH = "https://github.com/theoyinbooke";

export default function About(props: { info: AppInfo | null; onFeedback: () => void }) {
  const open = (url: string) => openUrl(url).catch(() => {});
  const version = props.info?.version ?? "—";
  const platform = props.info?.platform ?? "—";

  return (
    <div className="view about stagger">
      <PageHeader title="About" desc="A 100% on-device voice keyboard for Linux & Windows." />

      <Card className="about-hero">
        <div className="about-id">
          <span className="about-name">Orttaai</span>
          <span className="about-version mono">v{version} · {platform}</span>
        </div>
        <p className="about-tagline">
          Press your shortcut, speak, and your words are typed into whatever app is focused —
          transcribed on-device with whisper.cpp. Your audio never leaves your machine.
        </p>
      </Card>

      <Card className="about-card">
        <span className="overline">Primary contributor</span>
        <div className="about-person">
          <div className="about-person-meta">
            <span className="about-person-name">Olanrewaju Oyinbooke</span>
            <button className="about-handle" onClick={() => open(CONTRIBUTOR_GH)}>
              @theoyinbooke
            </button>
          </div>
        </div>
      </Card>

      <Card className="about-card">
        <span className="overline">Links</span>
        <div className="about-links">
          <button className="about-link" onClick={() => open(REPO)}>
            <Icon name="arrowRight" size={16} /> Source code
          </button>
          <button className="about-link" onClick={props.onFeedback}>
            <Icon name="send" size={16} /> Send feedback / report an issue
          </button>
          <button className="about-link" onClick={() => open(`${REPO}/releases/latest`)}>
            <Icon name="download" size={16} /> Latest release
          </button>
          <button className="about-link" onClick={() => open(`${REPO}/blob/main/LICENSE`)}>
            <Icon name="info" size={16} /> License (MIT)
          </button>
        </div>
      </Card>

      <p className="foot-note">
        <Icon name="info" size={14} /> Built with whisper.cpp &amp; Tauri.
      </p>
    </div>
  );
}

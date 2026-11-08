// Devices: Orttaai Link. This PC syncs with Orttaai on a Mac, and through it
// with iCloud and Orttakey on iPhone, iPad and Android. Link once with the
// code the Mac shows; after that it syncs by itself, at home on Wi-Fi or away
// over Tailscale. The Mac's side is Settings › Devices.
import { useEffect, useState } from "react";
import { Button, ConfirmDialog, Icon, PageHeader, Spinner, useToast } from "../ui";
import { call, fmtInt, useEvent } from "../lib/api";
import { Notice, Row, Section } from "../components/SettingsRows";
import "../styles/devices.css";

type DeviceKind = "mac" | "iphone" | "ipad" | "windows" | "linux" | "android" | "unknown";

type LinkStatus = {
  linked: boolean;
  macName: string | null;
  state: "idle" | "syncing" | "offline" | "unlinked" | "updateMac" | "error";
  message: string | null;
  lastSyncMs: number | null;
  viaTailscale: boolean;
  host: string | null;
  counts: { waiting: number; failed: number; lastFailure: string | null };
  icloud: { state: string; lastSentAt: number | null } | null;
  deviceId: string | null;
  tailscaleName: string | null;
};

type DiscoveredMac = {
  macDeviceId: string;
  name: string;
  fingerprint: string;
  hosts: string[];
  port: number;
  linking: boolean;
};

type DeviceView = {
  deviceId: string;
  name: string;
  kind: DeviceKind;
  app: string;
  thisDevice: boolean;
  totals: { dictations: number; words: number; recordingMs: number };
  lastSeenAt: number;
};

const KIND_NAME: Record<DeviceKind, string> = {
  mac: "Mac",
  iphone: "iPhone",
  ipad: "iPad",
  windows: "Windows",
  linux: "Linux",
  android: "Android",
  unknown: "Device",
};

function kindIcon(kind: DeviceKind) {
  if (kind === "mac") return "laptop";
  if (kind === "iphone" || kind === "android") return "phone";
  if (kind === "ipad") return "phone";
  return "pc";
}

/** "2 minutes ago", "yesterday". */
function ago(ms: number | null | undefined): string {
  if (!ms) return "never";
  const seconds = Math.max(0, (Date.now() - ms) / 1000);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (seconds < 45) return "just now";
  if (seconds < 3600) return rtf.format(-Math.round(seconds / 60), "minute");
  if (seconds < 86_400) return rtf.format(-Math.round(seconds / 3600), "hour");
  return rtf.format(-Math.round(seconds / 86_400), "day");
}

function duration(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes >= 60) return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  if (minutes > 0) return `${minutes}m`;
  return `${Math.round(ms / 1000)}s`;
}

const plural = (n: number, word: string) => `${fmtInt(n)} ${word}${n === 1 ? "" : "s"}`;

export default function Devices() {
  const toast = useToast();
  const [status, setStatus] = useState<LinkStatus | null>(null);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [confirmUnlink, setConfirmUnlink] = useState(false);

  const loadDevices = () => call<DeviceView[]>("link_devices").then(setDevices).catch(() => {});
  useEffect(() => {
    call<LinkStatus>("link_status").then(setStatus).catch((e) => toast(String(e), "error"));
    loadDevices();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useEvent<LinkStatus>("link-status", setStatus);
  useEvent("history-changed", loadDevices);
  // Relative times stay fresh.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  const unlink = async () => {
    setConfirmUnlink(false);
    try {
      await call("link_unlink");
      setStatus(await call<LinkStatus>("link_status"));
      toast("Unlinked. Everything stays on this PC.");
    } catch (e) {
      toast(String(e), "error");
    }
  };

  return (
    <div className="view">
      <PageHeader title="Devices" />
      <div className="st-column">
        {status?.linked ? (
          <LinkedCard
            status={status}
            onSyncNow={() => call("link_sync_now").catch((e) => toast(String(e), "error"))}
            onUnlink={() => setConfirmUnlink(true)}
          />
        ) : (
          <LinkCard
            onLinked={async (name) => {
              setStatus(await call<LinkStatus>("link_status"));
              loadDevices();
              toast(`Linked to ${name}. Syncing now.`, "success");
            }}
          />
        )}

        {status?.linked && devices.length > 0 && (
          <Section
            title="Your Devices"
            desc="Every device in your Orttaai archive, with the dictations it added. Words and time count each device's own dictations."
          >
            {devices.map((d) => (
              <div className="dv-device" key={d.deviceId}>
                <span className="dv-device-icon">
                  <Icon name={kindIcon(d.kind)} size={18} />
                </span>
                <span className="dv-device-text">
                  <span className="dv-device-name">
                    {d.name}
                    <span className="dv-device-kind">{d.thisDevice ? "This PC" : KIND_NAME[d.kind]}</span>
                  </span>
                  <span className="dv-device-detail">
                    {d.totals.dictations > 0
                      ? `${plural(d.totals.dictations, "dictation")} · ${plural(d.totals.words, "word")} · ${duration(d.totals.recordingMs)} spoken`
                      : "No dictations yet"}
                  </span>
                </span>
                <span className="dv-device-seen">{d.thisDevice ? d.app : `seen ${ago(d.lastSeenAt)}`}</span>
              </div>
            ))}
          </Section>
        )}

        <Section title="How Syncing Works">
          <div className="dv-steps">
            <p>
              <b>Your Mac carries everything to iCloud.</b> This PC sends its dictations, dictionary, snippets and
              insights to Orttaai on your Mac. The Mac passes them to iCloud, and on to Orttakey on your iPhone.
              Dictations from your other devices come back here, each labelled with where it was said.
            </p>
            <p>
              <b>At home and away.</b> On the same Wi-Fi, this PC finds the Mac by itself. Away from home it reaches
              the Mac over Tailscale when both run it.
            </p>
            <p>
              <b>Nothing is lost.</b> When the Mac is off or asleep, changes wait here and go the next time it's back.
              Keep Orttaai running on the Mac.
            </p>
          </div>
        </Section>
      </div>

      <ConfirmDialog
        open={confirmUnlink}
        title={`Unlink from ${status?.macName ?? "the Mac"}?`}
        body="This PC stops syncing. Everything stays here and on the Mac. You can link again any time."
        confirmLabel="Unlink"
        onConfirm={unlink}
        onCancel={() => setConfirmUnlink(false)}
      />
    </div>
  );
}

function LinkedCard({
  status,
  onSyncNow,
  onUnlink,
}: {
  status: LinkStatus;
  onSyncNow: () => void;
  onUnlink: () => void;
}) {
  const { state, counts } = status;
  const headline =
    state === "syncing"
      ? counts.waiting > 0
        ? `Sending ${plural(counts.waiting, "change")}`
        : "Syncing"
      : state === "offline"
        ? "Waiting for the Mac"
        : state === "unlinked"
          ? "Removed on the Mac"
          : state === "updateMac"
            ? "The Mac needs an update"
            : state === "error"
              ? "Sync stopped"
              : "Up to date";
  const tone = state === "idle" || state === "syncing" ? "ok" : state === "offline" ? "wait" : "bad";
  const icloud = status.icloud;
  const icloudText = !icloud
    ? null
    : icloud.state === "syncing"
      ? "The Mac syncs with iCloud, so your iPhone gets these too."
      : icloud.state === "off"
        ? "iCloud Sync is off on the Mac, so these stay on the Mac. Turn it on in Orttaai on the Mac to reach your iPhone."
        : icloud.state === "signedOut"
          ? "The Mac is signed out of iCloud. Changes reach the Mac and wait there."
          : "iCloud Sync is paused on the Mac. Changes reach the Mac and wait there.";

  return (
    <Section
      title="Synced With a Mac"
      desc="This PC syncs with Orttaai on your Mac, and through it with iCloud and Orttakey."
      accessory={
        <Button size="sm" variant="subtle" icon="refresh" onClick={onSyncNow}>
          Sync Now
        </Button>
      }
    >
      <div className="dv-linked">
        <span className="dv-linked-icon">
          <Icon name="laptop" size={22} />
        </span>
        <span className="dv-linked-text">
          <span className="dv-linked-name">{status.macName ?? "Mac"}</span>
          <span className={`dv-state dv-state-${tone}`}>
            <i aria-hidden="true" />
            {headline}
            {status.lastSyncMs && <span className="dv-state-when"> · last reached {ago(status.lastSyncMs)}</span>}
          </span>
          {status.host && (
            <span className="dv-linked-route">
              <Icon name={status.viaTailscale ? "globe" : "link"} size={12} />
              {status.viaTailscale ? "Through Tailscale" : "On home Wi-Fi"}
            </span>
          )}
        </span>
      </div>
      {status.message && (
        <div className="dv-pad">
          <Notice kind={state === "offline" ? "info" : "warning"}>{status.message}</Notice>
        </div>
      )}
      {counts.failed > 0 && (
        <div className="dv-pad">
          <Notice kind="warning">
            {plural(counts.failed, "item")} couldn't be sent{counts.lastFailure ? `: ${counts.lastFailure}` : "."}
          </Notice>
        </div>
      )}
      {icloudText && (
        <Row label="iCloud" hint="Only Apple devices can open Orttaai's iCloud data, so the Mac carries this PC's changes there.">
          <span className="dv-muted">{icloudText}</span>
        </Row>
      )}
      <Row label="Unlink" hint="Stops syncing with this Mac. Nothing is deleted on either side.">
        <Button size="sm" variant="ghost" className="dv-unlink" onClick={onUnlink}>
          Unlink…
        </Button>
      </Row>
    </Section>
  );
}

function LinkCard({ onLinked }: { onLinked: (macName: string) => void }) {
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);
  const [macs, setMacs] = useState<DiscoveredMac[]>([]);
  const [chosen, setChosen] = useState<DiscoveredMac | null>(null);
  const [code, setCode] = useState("");
  const [linking, setLinking] = useState(false);
  const [invite, setInvite] = useState("");
  const [showInvite, setShowInvite] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const search = async () => {
    setSearching(true);
    setError(null);
    try {
      const found = await call<DiscoveredMac[]>("link_discover");
      setMacs(found);
      if (found.length === 1) setChosen(found[0]);
    } catch (e) {
      setError(String(e));
    } finally {
      setSearching(false);
      setSearched(true);
    }
  };
  useEffect(() => {
    search();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const linkWithCode = async () => {
    if (!chosen) return;
    setLinking(true);
    setError(null);
    try {
      onLinked(await call<string>("link_with_code", { mac: chosen, code }));
    } catch (e) {
      setError(String(e));
    } finally {
      setLinking(false);
    }
  };

  const linkWithInvite = async () => {
    setLinking(true);
    setError(null);
    try {
      onLinked(await call<string>("link_with_invite", { text: invite.trim() }));
    } catch (e) {
      setError(String(e));
    } finally {
      setLinking(false);
    }
  };

  const digits = code.replace(/\D/g, "");

  return (
    <Section
      title="Link to Your Mac"
      desc="Sync this PC with Orttaai on your Mac, and through it with iCloud and Orttakey on your iPhone."
      accessory={
        <Button size="sm" variant="subtle" icon="refresh" onClick={search} disabled={searching}>
          Look Again
        </Button>
      }
    >
      <div className="dv-link">
        <ol className="dv-link-steps">
          <li>
            On your Mac, open Orttaai › Settings › <b>Devices</b> and choose <b>Link a Device</b>.
          </li>
          <li>Pick your Mac below and type the 6-digit code it shows.</li>
        </ol>

        <div className="dv-macs" role="listbox" aria-label="Macs on this network">
          {searching && macs.length === 0 && (
            <div className="dv-searching">
              <Spinner /> Looking for Orttaai on your network…
            </div>
          )}
          {!searching && searched && macs.length === 0 && (
            <div className="dv-searching">
              No Mac found on this network. Check that Orttaai is open on the Mac and both are on the same Wi-Fi, or
              paste the Mac's link below.
            </div>
          )}
          {macs.map((m) => (
            <button
              key={m.macDeviceId}
              type="button"
              role="option"
              aria-selected={chosen?.macDeviceId === m.macDeviceId}
              className={`dv-mac ${chosen?.macDeviceId === m.macDeviceId ? "selected" : ""}`}
              onClick={() => setChosen(m)}
            >
              <Icon name="laptop" size={18} />
              <span className="dv-mac-text">
                <span className="dv-mac-name">{m.name}</span>
                <span className="dv-mac-hint">{m.linking ? "Ready to link" : "Open Link a Device on this Mac"}</span>
              </span>
              {chosen?.macDeviceId === m.macDeviceId && <Icon name="check" size={16} className="dv-mac-check" />}
            </button>
          ))}
        </div>

        {chosen && (
          <form
            className="dv-code"
            onSubmit={(e) => {
              e.preventDefault();
              if (digits.length === 6) linkWithCode();
            }}
          >
            <label htmlFor="dv-code-input">Code from {chosen.name}</label>
            <div className="dv-code-row">
              <input
                id="dv-code-input"
                className="text-input dv-code-input"
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder="000 000"
                maxLength={7}
                value={digits.length > 3 ? `${digits.slice(0, 3)} ${digits.slice(3, 6)}` : digits}
                onChange={(e) => setCode(e.target.value)}
                autoFocus
              />
              <Button variant="primary" type="submit" loading={linking} disabled={digits.length !== 6}>
                Link
              </Button>
            </div>
          </form>
        )}

        {error && <Notice kind="error">{error}</Notice>}

        <div className="dv-invite">
          {!showInvite ? (
            <button type="button" className="dv-text-btn" onClick={() => setShowInvite(true)}>
              Not on the same Wi-Fi? Paste the link from your Mac
            </button>
          ) : (
            <form
              className="dv-code"
              onSubmit={(e) => {
                e.preventDefault();
                if (invite.trim()) linkWithInvite();
              }}
            >
              <label htmlFor="dv-invite-input">
                On the Mac, choose Link a Device › <b>Copy Link for a PC Away from Home</b>, then paste it here.
              </label>
              <div className="dv-code-row">
                <input
                  id="dv-invite-input"
                  className="text-input"
                  placeholder="orttaai://link?…"
                  value={invite}
                  onChange={(e) => setInvite(e.target.value)}
                />
                <Button variant="primary" type="submit" loading={linking} disabled={!invite.trim().startsWith("orttaai://link")}>
                  Link
                </Button>
              </div>
            </form>
          )}
        </div>
      </div>
    </Section>
  );
}

// Milestones (Mac MilestonesView): the next goal with progress, three summary
// tiles, filters and the badge wall in its five groups. Unlocked badges open
// the share sheet. Owned by the analytics/home area.
import { useCallback, useEffect, useState } from "react";
import "../styles/analytics.css";
import { Button, EmptyState, PageHeader, Spinner, useToast } from "../ui";
import { call, useEvent } from "../lib/api";
import { TIER_TITLE, durationText, type BadgeView, type MilestonesOverview } from "../lib/analytics";
import { MilestoneBadge } from "../components/ShareCardBadge";
import ShareCardSheet from "../components/ShareCardSheet";

type Filter = "all" | "unlocked" | "in_progress" | "locked";
const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "unlocked", label: "Unlocked" },
  { value: "in_progress", label: "In progress" },
  { value: "locked", label: "Locked" },
];

const matches = (b: BadgeView, f: Filter) => f === "all" || b.state.kind === f;

export function useMilestones() {
  const [overview, setOverview] = useState<MilestonesOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    call<MilestonesOverview>("milestones_overview")
      .then((o) => {
        setOverview(o);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(load, [load]);
  useEvent("milestones-changed", load);
  useEvent("history-changed", load);
  useEvent("memory-changed", load);
  return { overview, error, reload: load };
}

export default function Milestones() {
  const { overview, error } = useMilestones();
  const [filter, setFilter] = useState<Filter>("all");
  const [share, setShare] = useState<string | null>(null);
  const toast = useToast();

  useEffect(() => {
    if (error) toast(`Couldn't load milestones: ${error}`, "error");
  }, [error]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!overview) {
    return (
      <div className="view">
        <PageHeader title="Milestones" />
        {error ? <EmptyState icon="alert" title="Couldn't load milestones" desc={error} /> : <div className="ms-loading"><Spinner /></div>}
      </div>
    );
  }

  const { stats } = overview;
  const goal = overview.badges.find((b) => b.id === overview.next_goal) ?? null;
  const shown = overview.badges.filter((b) => matches(b, filter));

  return (
    <div className="view ms-view">
      <PageHeader
        title="Milestones"
        desc="Badges you earn by talking instead of typing. Once earned, they stay yours."
        actions={
          <Button
            variant="ghost"
            size="sm"
            icon="share"
            disabled={stats.dictations === 0}
            onClick={() => setShare("progress")}
            title="Make a card from your numbers so far"
          >
            Share progress
          </Button>
        }
      />

      <div className="ms-hero">
        <div className="ms-card ms-next">
          {goal ? (
            <>
              <MilestoneBadge badge={goal} size={88} />
              <div className="ms-next-text">
                <span className="ms-eyebrow">NEXT UP</span>
                <h2>{goal.name}</h2>
                <div className="ms-bar" role="progressbar" aria-label={`Progress to ${goal.name}`} aria-valuenow={Math.round(goal.fraction * 100)} aria-valuemin={0} aria-valuemax={100}>
                  <span style={{ width: `${goal.fraction * 100}%` }} />
                </div>
                <p className="muted">{overview.next_up_detail}</p>
              </div>
            </>
          ) : (
            <div className="ms-next-text">
              <h2>Every badge earned</h2>
              <p className="muted">All {overview.total} milestones are yours. Share the set.</p>
            </div>
          )}
        </div>
        <div className="ms-tiles">
          <Tile value={`${overview.unlocked_count} / ${overview.total}`} label="Badges unlocked" />
          <Tile value={stats.best_wpm > 0 ? `${stats.best_wpm}` : "–"} label="Best words per minute" />
          <Tile value={durationText(stats.minutes_saved)} label="Typing time saved" />
        </div>
      </div>

      <div className="chip-row ms-filters" role="radiogroup" aria-label="Filter badges">
        {FILTERS.map((f) => (
          <button
            key={f.value}
            role="radio"
            aria-checked={filter === f.value}
            className={`chip ${filter === f.value ? "selected" : ""}`}
            onClick={() => setFilter(f.value)}
          >
            {f.label} {overview.badges.filter((b) => matches(b, f.value)).length}
          </button>
        ))}
      </div>

      {shown.length === 0 ? (
        <p className="muted ms-none">Nothing here yet.</p>
      ) : (
        overview.families.map((fam) => {
          const badges = shown.filter((b) => b.family === fam.id);
          if (badges.length === 0) return null;
          return (
            <section key={fam.id} className="ms-group">
              <header className="ms-group-head">
                <h3>{fam.title}</h3>
                <span className="muted small">{fam.note}</span>
              </header>
              <div className="ms-grid">
                {badges.map((b) => (
                  <BadgeCell key={b.id} badge={b} onShare={() => setShare(b.id)} />
                ))}
              </div>
            </section>
          );
        })
      )}

      {share && <ShareCardSheet overview={overview} subjectId={share} onClose={() => setShare(null)} />}
    </div>
  );
}

function Tile({ value, label }: { value: string; label: string }) {
  return (
    <div className="ms-card ms-tile">
      {/* Long values shrink to fit, as the Mac's minimumScaleFactor. */}
      <span className="ms-tile-value" style={{ fontSize: value.length > 8 ? 19 : value.length > 6 ? 21 : 24 }}>
        {value}
      </span>
      <span className="ms-tile-label">{label}</span>
    </div>
  );
}

function statusText(b: BadgeView): string {
  switch (b.state.kind) {
    case "unlocked":
      return `Unlocked ${new Date(b.state.at * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
    case "in_progress":
      return b.progress_text;
    case "locked":
      return b.rule;
  }
}

function BadgeCell({ badge, onShare }: { badge: BadgeView; onShare: () => void }) {
  const unlocked = badge.state.kind === "unlocked";
  return (
    <button
      className={`ms-cell ms-${badge.state.kind}`}
      onClick={onShare}
      disabled={!unlocked}
      title={unlocked ? `Share a card for ${badge.name}` : `${TIER_TITLE[badge.tier]} · ${badge.rule}`}
      aria-label={`${badge.name}, ${statusText(badge)}`}
    >
      <MilestoneBadge badge={badge} size={72} />
      <span className="ms-cell-name">{badge.name}</span>
      <span className="ms-cell-status">{statusText(badge)}</span>
    </button>
  );
}

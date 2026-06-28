import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Card, EmptyState, PageHeader, Spinner, useToast } from "../ui";
import type { DashboardStats } from "../types";

const RANGES: { label: string; days: number }[] = [
  { label: "7 days", days: 7 },
  { label: "30 days", days: 30 },
  { label: "All time", days: 0 },
];

export default function Insights() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [range, setRange] = useState(7);
  const toast = useToast();

  useEffect(() => {
    setStats(null);
    invoke<DashboardStats>("dashboard_stats", { days: range })
      .then(setStats)
      .catch((e) => toast(String(e), "error"));
  }, [range, toast]);

  const chartLabel = range === 0 ? "Daily activity (last 30 days)" : `Last ${range} days`;

  const filter = (
    <div className="segmented" role="radiogroup" aria-label="Time range">
      {RANGES.map((r) => (
        <button
          key={r.days}
          role="radio"
          aria-checked={range === r.days}
          className={`seg-btn ${range === r.days ? "active" : ""}`}
          onClick={() => setRange(r.days)}
        >
          {r.label}
        </button>
      ))}
    </div>
  );

  return (
    <div className="view stagger">
      <PageHeader title="Insights" desc="Your dictation activity at a glance." actions={filter} />

      {!stats ? (
        <div className="loading-row">
          <Spinner size={18} /> Loading…
        </div>
      ) : (
        <>
          <div className="stat-grid">
            <Stat label="Dictations" value={stats.total} />
            <Stat label="Words typed" value={stats.total_words.toLocaleString()} />
            <Stat label="Words / minute" value={stats.avg_wpm > 0 ? Math.round(stats.avg_wpm) : "—"} />
          </div>

          <Card className="chart-card">
            <span className="overline">{chartLabel}</span>
            {stats.last7_days.length === 0 ? (
              <p className="muted small">No activity in this range.</p>
            ) : (
              <div className="chart">
                {stats.last7_days.map((d) => {
                  const maxDay = Math.max(1, ...stats.last7_days.map((x) => x.count));
                  return (
                    <div key={d.day} className="chart-col" title={`${d.day}: ${d.count}`}>
                      <div className="chart-track">
                        <div
                          className="chart-bar"
                          style={{ height: `${Math.max(4, (d.count / maxDay) * 100)}%` }}
                        />
                      </div>
                      <span className="chart-val mono">{d.count}</span>
                      <span className="chart-label">{d.day.slice(5)}</span>
                    </div>
                  );
                })}
              </div>
            )}
          </Card>

          <Card className="apps-card">
            <span className="overline">Top apps</span>
            {stats.top_apps.length === 0 ? (
              <EmptyState
                icon="models"
                title="No app data yet"
                desc="The app you dictate into is recorded on X11 and Windows. On GNOME/Wayland the focused app can't be detected, so it stays unattributed."
              />
            ) : (
              <ul className="top-apps">
                {stats.top_apps.map((a) => (
                  <li key={a.app}>
                    <span>{a.app}</span>
                    <span className="mono">{a.count}</span>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number | string }) {
  return (
    <Card className="stat">
      <span className="stat-value mono">{value}</span>
      <span className="stat-label">{label}</span>
    </Card>
  );
}

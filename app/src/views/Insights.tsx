import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Card, EmptyState, PageHeader, Spinner, useToast } from "../ui";
import type { DashboardStats } from "../types";

export default function Insights() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const toast = useToast();

  useEffect(() => {
    invoke<DashboardStats>("dashboard_stats")
      .then(setStats)
      .catch((e) => toast(String(e), "error"));
  }, [toast]);

  if (!stats)
    return (
      <div className="view">
        <PageHeader title="Insights" desc="Your dictation activity at a glance." />
        <div className="loading-row">
          <Spinner size={18} /> Loading…
        </div>
      </div>
    );

  const maxDay = Math.max(1, ...stats.last7_days.map((d) => d.count));

  return (
    <div className="view stagger">
      <PageHeader title="Insights" desc="Your dictation activity at a glance." />

      <div className="stat-grid">
        <Stat label="Dictations" value={stats.total} />
        <Stat label="Words typed" value={stats.total_words.toLocaleString()} />
        <Stat label="Avg / dictation" value={stats.avg_words.toFixed(1)} />
      </div>

      <Card className="chart-card">
        <span className="overline">Last 7 days</span>
        {stats.last7_days.length === 0 ? (
          <p className="muted small">No activity yet.</p>
        ) : (
          <div className="chart">
            {stats.last7_days.map((d) => (
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
            ))}
          </div>
        )}
      </Card>

      <Card className="apps-card">
        <span className="overline">Top apps</span>
        {stats.top_apps.length === 0 ? (
          <EmptyState
            icon="models"
            title="App attribution coming soon"
            desc="Once Orttaai can detect the focused app, your most-dictated-into apps will rank here."
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

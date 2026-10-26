import { useEffect, useState, type ReactElement } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";

export type AppUsage = { app_name: string; dictations: number; percent: number };

export type StreakInfo = {
  current_days: number;
  longest_days: number;
  daily_counts: Record<string, number>;
};

export type AdoptionItem = { key: string; label: string; done: boolean; suggestion: string };

export type AdoptionInfo = { score_percent: number; items: AdoptionItem[] };

export type InsightsPayload = {
  total_dictations: number;
  total_words: number;
  smart_formatting_count: number;
  avg_words_per_minute: number | null;
  personal_best_wpm: number | null;
  latency: { median_ms: number; p90_ms: number; samples: number } | null;
  app_usage: AppUsage[];
  streak: StreakInfo;
  adoption: AdoptionInfo;
};

export function formatSeconds(ms: number): string {
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}

export function StatCard({
  value,
  label,
  children,
}: {
  value: ReactElement | string;
  label: string;
  children?: ReactElement;
}) {
  return (
    <div className="flex flex-col rounded-lg border border-base-content/10 bg-base-100 p-3">
      <div className="text-[26px] font-semibold leading-none">{value}</div>
      <div className="mt-1.5 text-[10px] font-semibold uppercase tracking-wide opacity-50">{label}</div>
      {children && <div className="mt-2.5 border-t border-base-content/10 pt-2.5">{children}</div>}
    </div>
  );
}

/// Semicircle gauge showing average pace as a fraction of this device's
/// personal-best WPM — deliberately not a percentile against other users
/// (Flow's "Top 0.1%"), since a local single-user app has no population
/// to rank against. Arc length is proportional to angle for a fixed
/// radius, so `circumference * ratio` is an exact (not approximated)
/// fill for the swept angle.
export function WpmGauge({ ratio }: { ratio: number }) {
  const clamped = Math.max(0, Math.min(1, ratio));
  const radius = 42;
  const circumference = Math.PI * radius;
  return (
    <svg width="100" height="56" viewBox="0 0 100 56">
      <path
        d="M8,50 A42,42 0 0 1 92,50"
        fill="none"
        stroke="currentColor"
        strokeOpacity="0.12"
        strokeWidth="8"
        strokeLinecap="round"
      />
      <path
        d="M8,50 A42,42 0 0 1 92,50"
        fill="none"
        stroke="#2f9e6e"
        strokeWidth="8"
        strokeLinecap="round"
        strokeDasharray={`${circumference * clamped} ${circumference}`}
      />
    </svg>
  );
}

export const HEATMAP_WEEKS = 18;

export const WEEKDAY_LABELS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export const MONTH_LABELS = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

export function isoDateUTC(d: Date): string {
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export type HeatmapCell = { key: string; count: number; isFuture: boolean; monthLabel: string | null };

// Trailing `HEATMAP_WEEKS` weeks (Sun-start columns) ending at the current
// week, matching the backend's UTC-calendar-day bucketing (see
// `insights::civil_from_days`) rather than local time, so a cell's date
// key always lines up with what the backend counted it under.
export function buildHeatmapWeeks(dailyCounts: Record<string, number>): HeatmapCell[][] {
  const now = new Date();
  const todayUTC = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const todayKey = isoDateUTC(todayUTC);
  const todayDow = todayUTC.getUTCDay();

  const gridStart = new Date(todayUTC);
  gridStart.setUTCDate(gridStart.getUTCDate() - todayDow - (HEATMAP_WEEKS - 1) * 7);

  const weeks: HeatmapCell[][] = [];
  let lastMonth = -1;
  const cursor = new Date(gridStart);
  for (let w = 0; w < HEATMAP_WEEKS; w++) {
    const week: HeatmapCell[] = [];
    for (let d = 0; d < 7; d++) {
      const key = isoDateUTC(cursor);
      const month = cursor.getUTCMonth();
      let monthLabel: string | null = null;
      if (d === 0 && month !== lastMonth) {
        monthLabel = MONTH_LABELS[month];
        lastMonth = month;
      }
      week.push({ key, count: dailyCounts[key] ?? 0, isFuture: key > todayKey, monthLabel });
      cursor.setUTCDate(cursor.getUTCDate() + 1);
    }
    weeks.push(week);
  }
  return weeks;
}

export function heatmapLevel(count: number): number {
  if (count <= 0) return 0;
  if (count === 1) return 1;
  if (count <= 3) return 2;
  return 3;
}

export const HEATMAP_LEVEL_BG = [
  "var(--dw-border, #2a2d32)",
  "#1c4f3a",
  "#237050",
  "#2f9e6e",
];

export function InsightsSection() {
  const [data, setData] = useState<InsightsPayload | null>(null);
  const [loading, setLoading] = useState(true);

  function refresh() {
    invoke<InsightsPayload>("get_insights")
      .then(setData)
      .catch((err) => notifyError("get_insights", err))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    refresh();
    // New dictations (and journal summaries) fire this same event History
    // already listens to — Insights is just another read-only view over
    // the same underlying data.
    const unlisten = listen("history-updated", refresh);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  if (loading) {
    return (
      <div className="mb-4">
        <span className="loading loading-spinner loading-xs" />
      </div>
    );
  }
  if (!data) return null;

  const weeks = buildHeatmapWeeks(data.streak.daily_counts);
  const wpmRatio =
    data.avg_words_per_minute && data.personal_best_wpm ? data.avg_words_per_minute / data.personal_best_wpm : 0;

  return (
    <div className="mb-4 flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <StatCard value={data.total_words.toLocaleString()} label="Total words dictated" />
        <StatCard value={String(data.smart_formatting_count)} label="Smart formatting applied">
          <p className="text-xs opacity-70">
            Punctuation commands, lists, casing, snippets, and Backtrack corrections.
          </p>
        </StatCard>
        {data.avg_words_per_minute ? (
          <StatCard value={`${Math.round(data.avg_words_per_minute)}`} label="Words per minute">
            <div className="flex items-center justify-between gap-2">
              <WpmGauge ratio={wpmRatio} />
              <p className="text-right text-xs opacity-70">
                Personal best
                <br />
                <span className="font-semibold opacity-100">
                  {data.personal_best_wpm ? Math.round(data.personal_best_wpm) : "—"} WPM
                </span>
              </p>
            </div>
          </StatCard>
        ) : (
          <StatCard value="—" label="Words per minute">
            <p className="text-xs opacity-70">Not enough data yet — this fills in after a few more dictations.</p>
          </StatCard>
        )}
        <StatCard value={data.latency ? formatSeconds(data.latency.median_ms) : "—"} label="Time to paste">
          <p className="text-xs opacity-70">
            {data.latency
              ? `Typical wait after you stop talking. Slowest 1 in 10: ${formatSeconds(data.latency.p90_ms)}.`
              : "Fills in after your next dictation."}
          </p>
        </StatCard>
      </div>

      <div className="rounded-lg border border-base-content/10 bg-base-100 p-3">
        <div className="mb-2 flex items-center justify-between">
          <label className="text-xs font-medium opacity-70">App usage</label>
          <span className="text-[10px] font-medium uppercase tracking-wide opacity-50">
            {data.app_usage.length} app{data.app_usage.length === 1 ? "" : "s"} used
          </span>
        </div>
        {data.app_usage.length === 0 ? (
          <p className="text-xs opacity-60">No dictations logged yet.</p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {data.app_usage.map((app) => (
              <div key={app.app_name} className="flex items-center gap-2">
                <span className="w-28 shrink-0 truncate text-xs">{app.app_name}</span>
                <div className="h-2 flex-1 overflow-hidden rounded-full bg-base-300">
                  <div
                    className="h-full rounded-full bg-primary"
                    style={{ width: `${Math.max(app.percent, 2)}%` }}
                  />
                </div>
                <span className="w-16 shrink-0 text-right text-xs tabular-nums opacity-70">
                  {app.dictations} · {Math.round(app.percent)}%
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="rounded-lg border border-base-content/10 bg-base-100 p-3">
        <div className="mb-2 flex items-center justify-between">
          <label className="text-xs font-medium opacity-70">
            {data.streak.current_days} day{data.streak.current_days === 1 ? "" : "s"} streak
          </label>
          <span className="text-[10px] font-medium uppercase tracking-wide opacity-50">
            Longest streak · {data.streak.longest_days} day{data.streak.longest_days === 1 ? "" : "s"}
          </span>
        </div>
        <div className="overflow-x-auto">
          <div className="inline-flex flex-col gap-1">
            <div className="flex gap-[3px] pl-8">
              {weeks.map((week, i) => (
                <div key={i} className="w-[13px] shrink-0 text-[9px] opacity-50">
                  {week[0].monthLabel ?? ""}
                </div>
              ))}
            </div>
            {WEEKDAY_LABELS.map((label, dow) => (
              <div key={label} className="flex items-center gap-[3px]">
                <span className="w-6 shrink-0 text-[9px] opacity-50">{dow % 2 === 1 ? label : ""}</span>
                {weeks.map((week, i) => {
                  const cell = week[dow];
                  return (
                    <div
                      key={i}
                      title={cell.isFuture ? undefined : `${cell.key}: ${cell.count}`}
                      className="h-[13px] w-[13px] shrink-0 rounded-[3px]"
                      style={{
                        background: cell.isFuture ? "transparent" : HEATMAP_LEVEL_BG[heatmapLevel(cell.count)],
                      }}
                    />
                  );
                })}
              </div>
            ))}
          </div>
        </div>
        <div className="mt-2 flex items-center justify-end gap-1 text-[9px] opacity-50">
          <span>Less</span>
          {HEATMAP_LEVEL_BG.map((bg, i) => (
            <div key={i} className="h-[10px] w-[10px] rounded-[2px]" style={{ background: bg }} />
          ))}
          <span>More</span>
        </div>
      </div>

      <div className="rounded-lg border border-base-content/10 bg-base-100 p-3">
        <div className="mb-2 flex items-center justify-between">
          <label className="text-xs font-medium opacity-70">Getting the most out of Mouthpiece</label>
          <span className="text-[10px] font-medium uppercase tracking-wide opacity-50">
            {data.adoption.score_percent}% of features used
          </span>
        </div>
        <div className="flex flex-col gap-2">
          {data.adoption.items.map((item) => (
            <div key={item.key} className="flex items-start gap-2">
              <span
                className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[10px] ${
                  item.done ? "bg-primary text-primary-content" : "bg-base-300 opacity-60"
                }`}
              >
                {item.done ? "✓" : ""}
              </span>
              <div>
                <div className="text-xs font-medium">{item.label}</div>
                {!item.done && <p className="text-xs opacity-60">{item.suggestion}</p>}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

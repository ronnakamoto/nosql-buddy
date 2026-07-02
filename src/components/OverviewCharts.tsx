import { useMemo, useState } from "react";
import type { CollectionSummary, DatabaseSummary } from "../ipc/commands";

const ACCENT = "var(--accent-500)";

// Distribution of accent-tinted colors for the storage share donut.
// Hue steps through 256 with chroma ramping down so each slice stays distinct
// against the surface background but doesn't break the restrained color
// strategy: all are variations on the system accent.
const SHARE_PALETTE = [
  "oklch(0.62 0.175 256)", // accent-500
  "oklch(0.55 0.18 220)",  // teal
  "oklch(0.6  0.16 295)",  // violet
  "oklch(0.65 0.15 165)",  // green
  "oklch(0.7  0.14 60)",   // amber
  "oklch(0.6  0.17 195)",  // cyan
  "oklch(0.6  0.16 330)",  // magenta
];

// Maximum number of named slices. Anything beyond this is rolled into a single
// "Other" bucket so the chart stays readable with 20+ databases while still
// preserving the cumulative share of the long tail.
const MAX_SLICES = 7;
// Below this percentage a slice is too thin to render as a distinct arc on
// the ring; we drop it from the donut but keep it in the "Other" total.
const MIN_VISIBLE_PCT = 1.5;
const OTHER_COLOR = "oklch(0.55 0.008 240)"; // neutral gray-ish, matches surface-2 family

interface BarDatum {
  label: string;
  value: number;
  displayValue: string;
}

function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function safeNum(v: unknown): number {
  return isFiniteNumber(v) ? v : 0;
}

function formatBytes(bytes: number): string {
  if (!isFiniteNumber(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatCount(n: number): string {
  if (!isFiniteNumber(n) || n <= 0) return "0";
  if (n < 1000) return String(Math.floor(n));
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

function ChartEmpty() {
  return <div className="overview__chart-empty">No data available</div>;
}

function BarList({ data, scale = "linear" }: { data: BarDatum[]; scale?: "linear" | "log" }) {
  if (!data || data.length === 0) return <ChartEmpty />;
  const max = data[0].value;
  if (max <= 0) return <ChartEmpty />;
  // Log scaling: log1p compresses the high end and expands the low end so a
  // 57.7k row and a 3 row both remain readable side by side. The log floor
  // is 1 to avoid log(0); values are mapped to [log(1+1), log(1+max)].
  const useLog = scale === "log";
  const minLog = Math.log1p(1);
  const maxLog = Math.log1p(max);
  const logRange = maxLog - minLog || 1;
  return (
    <div className="overview__chart-bars">
      {data.map((d, i) => {
        const pct = useLog
          ? ((Math.log1p(d.value) - minLog) / logRange) * 100
          : max > 0 ? (d.value / max) * 100 : 0;
        return (
          <div className="overview__bar-row" key={`${d.label}-${i}`}>
            <span className="overview__bar-label" title={d.label}>{d.label}</span>
            <span className="overview__bar-track">
              <span className="overview__bar-fill" style={{ width: `${Math.max(pct, 0.5)}%` }} />
            </span>
            <span className="overview__bar-value">{d.displayValue}</span>
          </div>
        );
      })}
    </div>
  );
}

/* ── Documents by database ────────────────────────────────────── */

// Aggregate per-collection document counts when dbStats.objects is missing
// (system DBs or older MongoDB versions). The Collections record is fetched
// alongside DatabaseSummary on connect, so this is a free fallback.
function dbDocumentCount(
  db: DatabaseSummary,
  collections: Record<string, CollectionSummary[]> | undefined,
): number {
  const fromStats = safeNum(db?.documentCount);
  if (fromStats > 0) return fromStats;
  const colls = collections?.[db?.name ?? ""];
  if (!Array.isArray(colls)) return 0;
  return colls.reduce((n, c) => n + safeNum(c?.documentCount), 0);
}

export function DocumentsByDatabaseChart({
  databases,
  collections,
}: {
  databases: DatabaseSummary[];
  collections: Record<string, CollectionSummary[]>;
}) {
  const [scale, setScale] = useState<"linear" | "log">("log");
  const data = useMemo(() => {
    if (!Array.isArray(databases)) return [];
    return databases
      .map((d) => ({ db: d, count: dbDocumentCount(d, collections ?? {}) }))
      .filter((e) => e.count > 0 && e.db?.name)
      .sort((a, b) => b.count - a.count)
      .slice(0, 8)
      .map((e) => ({
        label: e.db.name,
        value: e.count,
        displayValue: formatCount(e.count),
      }));
  }, [databases, collections]);

  // Show the scale toggle only when the data spans more than an order of
  // magnitude (e.g. 57.7k vs 3). On uniform data the toggle adds noise.
  const showToggle = data.length >= 2 && data[0].value >= data[data.length - 1].value * 10;

  return (
    <>
      {showToggle && (
        <div className="overview__scale-toggle" role="group" aria-label="Bar scale">
          <button
            className={"overview__scale-btn" + (scale === "log" ? " is-active" : "")}
            onClick={() => setScale("log")}
            aria-pressed={scale === "log"}
            title="Log scale — emphasizes relative differences between small and large values"
          >
            Log
          </button>
          <button
            className={"overview__scale-btn" + (scale === "linear" ? " is-active" : "")}
            onClick={() => setScale("linear")}
            aria-pressed={scale === "linear"}
            title="Linear scale — proportional bar lengths"
          >
            Linear
          </button>
        </div>
      )}
      <BarList data={data} scale={scale} />
    </>
  );
}

/* ── Storage share by database (donut chart) ──────────────────── */

/**
 * Donut chart showing each database's share of total sizeOnDisk across all
 * databases. Uses native listDatabases data (reliable, single round trip) plus
 * per-collection size aggregation as a fallback. Complementary to the Storage
 * bar chart: bar shows absolute ranking, donut shows proportional share.
 */
export function StorageShareDonutChart({ databases }: { databases: DatabaseSummary[] }) {
  const slices = useMemo(() => {
    if (!Array.isArray(databases)) return { segments: [], total: 0, hiddenCount: 0 };
    // Sort all databases by size, then split into visible + other.
    const all = databases
      .filter((d) => d && typeof d === "object")
      .map((d) => ({ name: d.name ?? "unknown", value: safeNum(d?.sizeOnDisk) }))
      .filter((e) => e.value > 0)
      .sort((a, b) => b.value - a.value);
    const total = all.reduce((n, e) => n + e.value, 0);
    if (total === 0) return { segments: [], total: 0, hiddenCount: 0 };

    // Keep top MAX_SLICES, but only those whose share is at least MIN_VISIBLE_PCT
    // of the total. Anything thinner gets bucketed into "Other" so the chart
    // stays readable with 20+ databases while preserving the long-tail share.
    const pctOf = (v: number) => (v / total) * 100;
    const top = all.slice(0, MAX_SLICES);
    const visible = top.filter((e) => pctOf(e.value) >= MIN_VISIBLE_PCT);
    const overflow = all.slice(visible.length);
    const otherValue = overflow.reduce((n, e) => n + e.value, 0);

    const segments: { name: string; value: number; color: string; pct: number }[] = visible.map((e, i) => ({
      ...e,
      color: SHARE_PALETTE[i % SHARE_PALETTE.length],
      pct: pctOf(e.value),
    }));

    if (otherValue > 0) {
      segments.push({
        name: `Other (${overflow.length})`,
        value: otherValue,
        color: OTHER_COLOR,
        pct: pctOf(otherValue),
      });
    }

    return { segments, total, hiddenCount: overflow.length };
  }, [databases]);

  if (!slices.total || slices.segments.length === 0) return <ChartEmpty />;

  // Build a conic-gradient from the segments so each segment takes its share
  // of the circle. startDeg tracks the running angle.
  const stops: string[] = [];
  let startDeg = 0;
  for (const s of slices.segments) {
    const angle = (s.value / slices.total) * 360;
    const endDeg = startDeg + angle;
    stops.push(`${s.color} ${startDeg}deg ${endDeg}deg`);
    startDeg = endDeg;
  }

  return (
    <div className="overview__donut">
      <div
        className="overview__donut-ring"
        style={{ background: `conic-gradient(${stops.join(", ")})` }}
        role="img"
        aria-label="Storage share by database"
      >
        <div className="overview__donut-center">
          <span className="overview__donut-center-value">{formatBytes(slices.total)}</span>
          <span className="overview__donut-center-label">Total</span>
        </div>
      </div>
      <div className="overview__donut-legend">
        {slices.segments.map((s) => (
          <span key={s.name} className="overview__donut-legend-item">
            <span
              className="overview__type-dot"
              style={{ background: s.color }}
              aria-hidden="true"
            />
            <span className="overview__donut-legend-label" title={s.name}>{s.name}</span>
            <span className="overview__donut-legend-pct">{s.pct.toFixed(1)}%</span>
            <span className="overview__donut-legend-value">{formatBytes(s.value)}</span>
          </span>
        ))}
      </div>
    </div>
  );
}

/* ── Top collections by document count ────────────────────────── */

export function TopCollectionsChart({
  databases,
  collections,
}: {
  databases: DatabaseSummary[];
  collections: Record<string, CollectionSummary[]>;
}) {
  const data = useMemo(() => {
    if (!Array.isArray(databases)) return [];
    const collMap = collections ?? {};
    const all: BarDatum[] = [];
    for (const db of databases) {
      if (!db?.name) continue;
      const colls = collMap[db.name];
      if (!Array.isArray(colls)) continue;
      for (const c of colls) {
        if (!c?.name) continue;
        const count = safeNum(c.documentCount);
        if (count > 0) {
          all.push({
            label: `${db.name}.${c.name}`,
            value: count,
            displayValue: formatCount(count),
          });
        }
      }
    }
    return all.sort((a, b) => b.value - a.value).slice(0, 8);
  }, [databases, collections]);

  return <BarList data={data} />;
}

/* ── Collection type breakdown (stacked proportion bar) ────────── */

const KIND_LABELS: Record<string, string> = {
  collection: "Collections",
  view: "Views",
  "time-series": "Time-Series",
  sharded: "Sharded",
  bucketed: "Bucketed",
};

const KIND_COLORS: Record<string, string> = {
  collection: ACCENT,
  view: "oklch(0.65 0.12 300)",
  "time-series": "oklch(0.68 0.13 190)",
  sharded: "oklch(0.7 0.14 60)",
  bucketed: "oklch(0.65 0.12 30)",
};

export function CollectionTypeChart({
  databases,
  collections,
}: {
  databases: DatabaseSummary[];
  collections: Record<string, CollectionSummary[]>;
}) {
  const counts = useMemo(() => {
    if (!Array.isArray(databases)) return [];
    const collMap = collections ?? {};
    const map = new Map<string, number>();
    for (const db of databases) {
      if (!db?.name) continue;
      const colls = collMap[db.name];
      if (!Array.isArray(colls)) continue;
      for (const c of colls) {
        if (!c?.type) continue;
        map.set(c.type, (map.get(c.type) ?? 0) + 1);
      }
    }
    return [...map.entries()].filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1]);
  }, [databases, collections]);

  if (counts.length === 0) return <ChartEmpty />;

  const total = counts.reduce((n, [, c]) => n + c, 0);
  if (total <= 0) return <ChartEmpty />;

  return (
    <div className="overview__type-breakdown">
      <div className="overview__type-bar" aria-hidden="true">
        {counts.map(([kind, n]) => (
          <span
            key={kind}
            className="overview__type-seg"
            style={{ width: `${(n / total) * 100}%`, background: KIND_COLORS[kind] ?? ACCENT }}
          />
        ))}
      </div>
      <div className="overview__type-legend">
        {counts.map(([kind, n]) => (
          <span key={kind} className="overview__type-legend-item">
            <span
              className="overview__type-dot"
              style={{ background: KIND_COLORS[kind] ?? ACCENT }}
              aria-hidden="true"
            />
            <span className="overview__type-label">{KIND_LABELS[kind] ?? kind}</span>
            <span className="overview__type-count">{n}</span>
          </span>
        ))}
      </div>
    </div>
  );
}

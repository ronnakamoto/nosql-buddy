import { useMemo } from "react";
import type { CollectionSummary, DatabaseSummary } from "../ipc/commands";

const ACCENT = "var(--accent-500)";
const DATA_COLOR = "var(--accent-500)";
const INDEX_COLOR = "oklch(0.65 0.12 300)";

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

function BarList({ data }: { data: BarDatum[] }) {
  if (!data || data.length === 0) return <ChartEmpty />;
  const max = data[0].value;
  if (max <= 0) return <ChartEmpty />;
  return (
    <div className="overview__chart-bars">
      {data.map((d, i) => {
        const pct = max > 0 ? (d.value / max) * 100 : 0;
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

/* ── Storage by database ──────────────────────────────────────── */

export function StorageByDatabaseChart({ databases }: { databases: DatabaseSummary[] }) {
  const data = useMemo(() => {
    if (!Array.isArray(databases)) return [];
    return databases
      .filter((d) => safeNum(d?.sizeOnDisk) > 0)
      .sort((a, b) => safeNum(b?.sizeOnDisk) - safeNum(a?.sizeOnDisk))
      .slice(0, 8)
      .map((d) => ({
        label: d.name ?? "unknown",
        value: safeNum(d.sizeOnDisk),
        displayValue: formatBytes(safeNum(d.sizeOnDisk)),
      }));
  }, [databases]);

  return <BarList data={data} />;
}

/* ── Documents by database ────────────────────────────────────── */

export function DocumentsByDatabaseChart({ databases }: { databases: DatabaseSummary[] }) {
  const data = useMemo(() => {
    if (!Array.isArray(databases)) return [];
    return databases
      .filter((d) => safeNum(d?.documentCount) > 0)
      .sort((a, b) => safeNum(b?.documentCount) - safeNum(a?.documentCount))
      .slice(0, 8)
      .map((d) => ({
        label: d.name ?? "unknown",
        value: safeNum(d.documentCount),
        displayValue: formatCount(safeNum(d.documentCount)),
      }));
  }, [databases]);

  return <BarList data={data} />;
}

/* ── Data vs Index storage (stacked horizontal bars) ──────────── */

export function DataVsIndexChart({ databases }: { databases: DatabaseSummary[] }) {
  const data = useMemo(() => {
    if (!Array.isArray(databases)) return [];
    return databases
      .filter((d) => safeNum(d?.storageSizeBytes) > 0 || safeNum(d?.indexSizeBytes) > 0)
      .sort(
        (a, b) =>
          safeNum(b?.storageSizeBytes) + safeNum(b?.indexSizeBytes) -
          (safeNum(a?.storageSizeBytes) + safeNum(a?.indexSizeBytes)),
      )
      .slice(0, 8);
  }, [databases]);

  if (data.length === 0) return <ChartEmpty />;

  const totals = data.map((d) => safeNum(d.storageSizeBytes) + safeNum(d.indexSizeBytes));
  const maxTotal = Math.max(...totals, 1);

  return (
    <div className="overview__chart-bars">
      {data.map((d, i) => {
        const dataBytes = safeNum(d.storageSizeBytes);
        const indexBytes = safeNum(d.indexSizeBytes);
        const total = dataBytes + indexBytes;
        const dataPct = maxTotal > 0 ? (dataBytes / maxTotal) * 100 : 0;
        const indexPct = maxTotal > 0 ? (indexBytes / maxTotal) * 100 : 0;
        return (
          <div className="overview__bar-row overview__bar-row--stacked" key={`${d.name}-${i}`}>
            <span className="overview__bar-label" title={d.name}>{d.name}</span>
            <span className="overview__bar-track overview__bar-track--stacked">
              <span
                className="overview__bar-fill overview__bar-fill--data"
                style={{ width: `${Math.max(dataPct, 0.5)}%` }}
              />
              <span
                className="overview__bar-fill overview__bar-fill--index"
                style={{ width: `${Math.max(indexPct, 0.5)}%` }}
              />
            </span>
            <span className="overview__bar-value">{formatBytes(total)}</span>
          </div>
        );
      })}
      <div className="overview__stacked-legend">
        <span className="overview__stacked-legend-item">
          <span className="overview__type-dot" style={{ background: DATA_COLOR }} aria-hidden="true" />
          Data
        </span>
        <span className="overview__stacked-legend-item">
          <span className="overview__type-dot" style={{ background: INDEX_COLOR }} aria-hidden="true" />
          Indexes
        </span>
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

//! Schema tab chart primitives built on visx. Uses OKLCH theme tokens via CSS
//! variables so charts adapt to light/dark automatically. visx gives us tested
//! scales, axes, and tooltips without the overhead of a full chart framework.

import { useMemo } from "react";
import { Bar } from "@visx/shape";
import { AxisBottom } from "@visx/axis";
import { Group } from "@visx/group";
import { scaleBand, scaleLinear } from "@visx/scale";
import { ParentSize } from "@visx/responsive";
import type { ScaleBand } from "d3-scale";

import type {
  SchemaDateStats,
  SchemaNumericStats,
  SchemaValueCount,
} from "../ipc/commands";

const ACCENT = "var(--accent-500)";
const INK_MUTED = "var(--ink-muted)";
const BORDER = "var(--border-strong)";

/** Horizontal bar chart for top values of a low-cardinality field. */
export function TopValuesChart({ values }: { values: SchemaValueCount[] }) {
  if (values.length === 0) return null;
  const max = Math.max(...values.map((v) => v.count));
  return (
    <div className="schema-top-values">
      {values.map((v, i) => {
        const pct = max > 0 ? (v.count / max) * 100 : 0;
        return (
          <div className="schema-top-values__row" key={`${v.value}-${i}`}>
            <span className="schema-top-values__label" title={v.value}>
              {v.value}
            </span>
            <span className="schema-top-values__track">
              <span
                className="schema-top-values__bar"
                style={{ transform: `scaleX(${Math.max(pct, 0.001) / 100})` }}
              />
            </span>
            <span className="schema-top-values__count">{v.count}</span>
          </div>
        );
      })}
    </div>
  );
}

interface HistogramProps {
  buckets: Array<{ lo: number; hi: number; count: number }>;
  loLabel: string;
  hiLabel: string;
  fmt: (v: number) => string;
}

/** Responsive vertical-bar histogram using visx scales + Bar. */
export function HistogramChart({
  buckets,
  loLabel,
  hiLabel,
  fmt,
}: HistogramProps) {
  if (buckets.length === 0) return null;

  return (
    <ParentSize>
      {({ width }) => (
        <HistogramInner
          width={width || 320}
          buckets={buckets}
          loLabel={loLabel}
          hiLabel={hiLabel}
          fmt={fmt}
        />
      )}
    </ParentSize>
  );
}

function HistogramInner({
  width,
  buckets,
  loLabel,
  hiLabel,
  fmt,
}: HistogramProps & { width: number }) {
  const height = 130;
  const padL = 6;
  const padR = 6;
  const padB = 26;
  const padT = 6;
  const plotW = Math.max(width - padL - padR, 10);
  const plotH = height - padT - padB;

  const xScale = useMemo(
    () =>
      scaleBand<string>({
        domain: buckets.map((_, i) => String(i)),
        range: [0, plotW],
        padding: 0.15,
      }),
    [buckets, plotW],
  );

  const maxCount = Math.max(...buckets.map((b) => b.count), 1);
  const yScale = useMemo(
    () =>
      scaleLinear<number>({
        domain: [0, maxCount],
        range: [plotH, 0],
        nice: true,
      }),
    [maxCount, plotH],
  );

  const tickValues = [0, Math.ceil(maxCount / 2), maxCount];

  return (
    <svg width={width} height={height} role="img" aria-label="value distribution histogram">
      <Group top={padT} left={padL}>
        {buckets.map((b, i) => {
          const barWidth = xScale.bandwidth();
          const x = (xScale as ScaleBand<string>)(String(i)) ?? 0;
          const y = yScale(b.count);
          const h = plotH - y;
          return (
            <Bar
              key={i}
              x={x}
              y={y}
              width={barWidth}
              height={h}
              fill={ACCENT}
              rx={1}
            >
              <title>{`${fmt(b.lo)} – ${fmt(b.hi)}: ${b.count}`}</title>
            </Bar>
          );
        })}
        <AxisBottom
          top={plotH}
          scale={xScale}
          tickFormat={() => ""}
          stroke={BORDER}
          tickStroke={BORDER}
          hideTicks
        />
        {/* Y tick labels (count) */}
        {tickValues.map((tv) => (
          <text
            key={tv}
            x={-2}
            y={yScale(tv)}
            dy="0.32em"
            textAnchor="end"
            className="schema-histogram__label"
            fill={INK_MUTED}
          >
            {tv}
          </text>
        ))}
      </Group>
      {/* range labels under the axis */}
      <text x={padL} y={height - 6} className="schema-histogram__label" fill={INK_MUTED}>
        {loLabel}
      </text>
      <text
        x={width - padR}
        y={height - 6}
        className="schema-histogram__label"
        fill={INK_MUTED}
        textAnchor="end"
      >
        {hiLabel}
      </text>
    </svg>
  );
}

export function NumericHistogramChart({ stats }: { stats: SchemaNumericStats }) {
  const fmt = (v: number) =>
    Number.isInteger(v) ? v.toString() : v.toFixed(2);
  return (
    <HistogramChart
      buckets={stats.buckets}
      loLabel={fmt(stats.min)}
      hiLabel={fmt(stats.max)}
      fmt={fmt}
    />
  );
}

export function DateHistogramChart({ stats }: { stats: SchemaDateStats }) {
  const fmt = (ms: number) => new Date(ms).toISOString().slice(0, 10);
  return (
    <HistogramChart
      buckets={stats.buckets.map((b) => ({ lo: b.loMs, hi: b.hiMs, count: b.count }))}
      loLabel={fmt(stats.minMs)}
      hiLabel={fmt(stats.maxMs)}
      fmt={fmt}
    />
  );
}

/** Stacked horizontal bar showing present / null / missing proportions. */
export function RatioBar({
  total,
  nullCount,
  missingCount,
}: {
  total: number;
  nullCount: number;
  missingCount: number;
}) {
  if (total === 0) return null;
  const present = total - nullCount - missingCount;
  const pct = (n: number) => (n / total) * 100;
  return (
    <div
      className="schema-ratio"
      title={`present ${present} · null ${nullCount} · missing ${missingCount}`}
    >
      <span
        className="schema-ratio__seg schema-ratio__seg--present"
        style={{ width: `${pct(present)}%` }}
      />
      <span
        className="schema-ratio__seg schema-ratio__seg--null"
        style={{ width: `${pct(nullCount)}%` }}
      />
      <span
        className="schema-ratio__seg schema-ratio__seg--missing"
        style={{ width: `${pct(missingCount)}%` }}
      />
    </div>
  );
}

/** Small numeric stat line: min / mean / max. */
export function NumericStatLine({ stats }: { stats: SchemaNumericStats }) {
  const fmt = (v: number) =>
    Number.isInteger(v) ? v.toString() : v.toFixed(2);
  return (
    <div className="schema-statline">
      <span>
        <b>min</b> {fmt(stats.min)}
      </span>
      <span>
        <b>mean</b> {fmt(stats.mean)}
      </span>
      <span>
        <b>max</b> {fmt(stats.max)}
      </span>
    </div>
  );
}

/** Date range line: min → max. */
export function DateStatLine({ stats }: { stats: SchemaDateStats }) {
  const fmt = (ms: number) => new Date(ms).toISOString().slice(0, 10);
  return (
    <div className="schema-statline">
      <span>
        <b>from</b> {fmt(stats.minMs)}
      </span>
      <span>
        <b>to</b> {fmt(stats.maxMs)}
      </span>
    </div>
  );
}

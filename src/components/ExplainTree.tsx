import { useMemo, useState } from "react";
import {
  ChevronRight,
  ChevronDown,
  CheckCircle,
  AlertTriangle,
  Database,
  Gauge,
  Zap,
  Activity,
  type LucideIcon,
} from "lucide-react";
import { type ExplainNode, type ParsedExplain, parseExplain } from "./explainParser";

export interface ExplainTreeProps {
  raw: { queryPlannerWinningPlan?: unknown; executionStats?: unknown; raw?: unknown } | null;
  /** Initial collapsed depth. `Infinity` = collapse everything; 0 = expand. */
  initialCollapsedDepth?: number;
}

/**
 * Render a MongoDB explain winning-plan tree as nested collapsible
 * rows. A three-cell findings dashboard at the top answers the
 * questions a developer asks in the first two seconds — was an index
 * used, what did the query cost, and which stage is the bottleneck —
 * followed by the full execution-stage tree with per-stage stats.
 */
export function ExplainTree({ raw, initialCollapsedDepth = 1 }: ExplainTreeProps) {
  const parsed = useMemo(() => parseExplain(raw), [raw]);
  // `overrides` is a sparse set of path → boolean indicating the
  // user's explicit expand (true) or collapse (false) choice for
  // that node. The default behaviour is determined by
  // `initialCollapsedDepth`: nodes deeper than that are collapsed
  // unless overridden.
  const [overrides, setOverrides] = useState<Map<string, boolean>>(new Map());

  const indexInfo = useMemo(() => collectIndexInfo(parsed.root), [parsed]);
  const bottleneck = useMemo(() => findBottleneck(parsed.root), [parsed]);

  if (!parsed.root) {
    return (
      <div className="explain-empty">
        <h3>No winning plan</h3>
        <p>The explain response did not include a query planner winning plan.</p>
      </div>
    );
  }

  function toggle(path: string, depth: number) {
    setOverrides((prev) => {
      const next = new Map(prev);
      // If the user toggles, the new state is the inverse of the
      // current default behaviour at that depth.
      const defaultCollapsed = depth > initialCollapsedDepth;
      const current =
        next.has(path) ? next.get(path)! === false : defaultCollapsed;
      next.set(path, !current);
      return next;
    });
  }

  function isCollapsed(path: string, depth: number): boolean {
    if (overrides.has(path)) return !overrides.get(path);
    return depth > initialCollapsedDepth;
  }

  function expandAll() {
    // Truly expand every stage: force every node path to expanded
    // (true), overriding the depth-based default that would otherwise
    // keep deep stages collapsed.
    const all = collectIds(parsed.root);
    const next = new Map<string, boolean>();
    for (const id of all) next.set(id, true);
    setOverrides(next);
  }

  function collapseAll() {
    const all = collectIds(parsed.root);
    const next = new Map<string, boolean>();
    for (const id of all) next.set(id, false);
    setOverrides(next);
  }

  return (
    <div className="explain-tree">
      <ExplainFindings
        summary={parsed.summary}
        indexInfo={indexInfo}
        bottleneck={bottleneck}
      />
      <div className="explain-tree__toolbar">
        <span className="explain-tree__count">{parsed.stageCount} stages</span>
        <div className="explain-tree__actions">
          <button className="btn btn--sm" onClick={expandAll}>Expand all</button>
          <button className="btn btn--sm" onClick={collapseAll}>Collapse all</button>
        </div>
      </div>
      <div className="explain-tree__body">
        <ExplainNodeView
          node={parsed.root}
          depth={0}
          path="0"
          isCollapsed={isCollapsed}
          onToggle={toggle}
          bottleneckPath={bottleneck?.path ?? null}
        />
      </div>
    </div>
  );
}

function collectIds(node: ExplainNode | null): string[] {
  if (!node) return [];
  const out: string[] = [];
  const walk = (n: ExplainNode, path: string) => {
    out.push(path);
    n.children.forEach((c, i) => walk(c, `${path}.${i}`));
  };
  walk(node, "0");
  return out;
}

interface IndexInfo {
  hasIndexScan: boolean;
  hasCollectionScan: boolean;
  indexNames: string[];
}

function collectIndexInfo(root: ExplainNode | null): IndexInfo {
  if (!root) return { hasIndexScan: false, hasCollectionScan: false, indexNames: [] };
  let hasIndexScan = false;
  let hasCollectionScan = false;
  const indexNames = new Set<string>();
  const walk = (n: ExplainNode) => {
    if (n.stage === "IXSCAN" || n.stage === "IDHACK") {
      hasIndexScan = true;
      if (n.indexName) indexNames.add(n.indexName);
      else if (n.stage === "IDHACK") indexNames.add("_id_");
    }
    if (n.stage === "COLLSCAN") hasCollectionScan = true;
    n.children.forEach(walk);
  };
  walk(root);
  return { hasIndexScan, hasCollectionScan, indexNames: [...indexNames] };
}

interface Bottleneck {
  path: string;
  stage: string;
  ms: number;
  docs: number | null;
}

function findBottleneck(root: ExplainNode | null): Bottleneck | null {
  if (!root) return null;
  let best: Bottleneck | null = null;
  const walk = (n: ExplainNode, path: string) => {
    if (n.executionTimeMs !== null && n.executionTimeMs > 0) {
      if (!best || n.executionTimeMs > best.ms) {
        best = { path, stage: n.stage, ms: n.executionTimeMs, docs: n.docsExamined };
      }
    }
    n.children.forEach((c, i) => walk(c, `${path}.${i}`));
  };
  walk(root, "0");
  return best;
}

type FindingTone = "success" | "danger" | "warning" | "neutral";

function ExplainFindings({
  summary,
  indexInfo,
  bottleneck,
}: {
  summary: ParsedExplain["summary"];
  indexInfo: IndexInfo;
  bottleneck: Bottleneck | null;
}) {
  // Index strategy
  let indexTone: FindingTone = "neutral";
  let indexIcon: LucideIcon = Database;
  let indexValue = "No scan";
  let indexDetail = "No IXSCAN or COLLSCAN in plan.";
  if (indexInfo.hasCollectionScan) {
    indexTone = "danger";
    indexIcon = AlertTriangle;
    indexValue = "COLLSCAN";
    indexDetail = "No index used. Add an index to filter efficiently.";
  } else if (indexInfo.hasIndexScan) {
    indexTone = "success";
    indexIcon = CheckCircle;
    indexValue = "IXSCAN";
    indexDetail =
      indexInfo.indexNames.length > 0
        ? indexInfo.indexNames.join(", ")
        : "Indexed access.";
  }

  // Query cost
  const costValue =
    summary.totalExecutionMs !== null ? `${formatNumber(summary.totalExecutionMs)} ms` : "No timing";
  const costParts: string[] = [];
  if (summary.totalDocsExamined !== null)
    costParts.push(`${formatNumber(summary.totalDocsExamined)} examined`);
  if (summary.totalReturned !== null)
    costParts.push(`${formatNumber(summary.totalReturned)} returned`);
  if (summary.totalKeysExamined !== null)
    costParts.push(`${formatNumber(summary.totalKeysExamined)} keys`);
  const costDetail = costParts.length > 0 ? costParts.join(" · ") : "No execution stats.";

  // Bottleneck
  let slowTone: FindingTone = "neutral";
  let slowIcon: LucideIcon = Activity;
  let slowValue = "No bottleneck";
  let slowDetail = "All stages report 0 ms.";
  if (bottleneck) {
    slowTone = "warning";
    slowIcon = Zap;
    slowValue = bottleneck.stage;
    slowDetail =
      `${bottleneck.ms} ms` +
      (bottleneck.docs !== null ? ` · ${formatNumber(bottleneck.docs)} docs` : "");
  }

  return (
    <div className="explain-findings">
      <FindingCell tone={indexTone} icon={indexIcon} label="Index" value={indexValue} detail={indexDetail} />
      <FindingCell tone="neutral" icon={Gauge} label="Cost" value={costValue} detail={costDetail} />
      <FindingCell tone={slowTone} icon={slowIcon} label="Slowest stage" value={slowValue} detail={slowDetail} />
    </div>
  );
}

function FindingCell({
  tone,
  icon: Icon,
  label,
  value,
  detail,
}: {
  tone: FindingTone;
  icon: LucideIcon;
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <div className={`explain-finding explain-finding--${tone}`}>
      <div className="explain-finding__head">
        <Icon className="explain-finding__icon" size={14} aria-hidden />
        <span className="explain-finding__label">{label}</span>
      </div>
      <div className="explain-finding__value">{value}</div>
      <div className="explain-finding__detail">{detail}</div>
    </div>
  );
}

interface ExplainNodeViewProps {
  node: ExplainNode;
  depth: number;
  path: string;
  isCollapsed: (path: string, depth: number) => boolean;
  onToggle: (path: string, depth: number) => void;
  bottleneckPath: string | null;
}

function ExplainNodeView({
  node,
  depth,
  path,
  isCollapsed,
  onToggle,
  bottleneckPath,
}: ExplainNodeViewProps) {
  const collapsed = isCollapsed(path, depth);
  const stageClass = stageBadgeClass(node.stage);
  const hasChildren = node.children.length > 0;
  const isBottleneck = path === bottleneckPath;

  return (
    <div className={`explain-node${isBottleneck ? " explain-node--bottleneck" : ""}`}>
      <div className="explain-node__head">
        {hasChildren ? (
          <button
            className="explain-node__toggle"
            onClick={() => onToggle(path, depth)}
            aria-expanded={!collapsed}
            aria-label={collapsed ? "Expand stage" : "Collapse stage"}
          >
            {collapsed ? <ChevronRight size={14} aria-hidden /> : <ChevronDown size={14} aria-hidden />}
          </button>
        ) : (
          <span className="explain-node__leaf" aria-hidden="true" />
        )}
        <span className={`explain-node__stage ${stageClass}`}>{node.stage}</span>
        {isBottleneck && (
          <span className="explain-node__bottleneck-tag" title="Slowest stage in this plan">
            Slowest
          </span>
        )}
        {node.indexName && (
          <span className="explain-node__index" title={describeIndex(node)}>
            {node.indexName}
          </span>
        )}
        <ExplainStats node={node} />
      </div>
      {!collapsed && hasChildren && (
        <div className="explain-node__children">
          {node.children.map((c, i) => (
            <ExplainNodeView
              key={`${path}.${i}`}
              node={c}
              depth={depth + 1}
              path={`${path}.${i}`}
              isCollapsed={isCollapsed}
              onToggle={onToggle}
              bottleneckPath={bottleneckPath}
            />
          ))}
        </div>
      )}
      {!collapsed && Object.keys(node.extra).length > 0 && (
        <details className="explain-node__details">
          <summary>Extra fields</summary>
          <pre className="json-view">{JSON.stringify(node.extra, null, 2)}</pre>
        </details>
      )}
    </div>
  );
}

function ExplainStats({ node }: { node: ExplainNode }) {
  const parts: string[] = [];
  if (node.docsExamined !== null) parts.push(`docs ${formatNumber(node.docsExamined)}`);
  if (node.nReturned !== null) parts.push(`returned ${formatNumber(node.nReturned)}`);
  if (node.executionTimeMs !== null) parts.push(`${formatNumber(node.executionTimeMs)} ms`);
  if (node.memUsage !== null && node.memUsage > 0) parts.push(`mem ${formatBytes(node.memUsage)}`);
  if (parts.length === 0) return null;
  return <span className="explain-node__stats">{parts.join(" · ")}</span>;
}

function stageBadgeClass(stage: string): string {
  switch (stage) {
    case "IXSCAN":
    case "IDHACK":
      return "explain-stage--index";
    case "COLLSCAN":
      return "explain-stage--scan";
    case "FETCH":
    case "PROJECTION_DEFAULT":
    case "PROJECTION_SIMPLE":
    case "PROJECTION_COVERED":
      return "explain-stage--fetch";
    case "SORT":
    case "SORT_KEY_GENERATOR":
    case "SORT_MERGE":
      return "explain-stage--sort";
    case "LIMIT":
    case "SKIP":
    case "COUNT":
    case "COUNT_SCAN":
      return "explain-stage--limit";
    case "GROUP":
    case "GROUP_BY":
    case "$group":
      return "explain-stage--group";
    default:
      return "explain-stage--other";
  }
}

function describeIndex(node: ExplainNode): string {
  if (!node.keyPattern) return node.indexName ?? "";
  return `Index ${node.indexName} on ${JSON.stringify(node.keyPattern)}`;
}

function formatNumber(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

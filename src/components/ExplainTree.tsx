import { useMemo, useState } from "react";
import { type ExplainNode, type ParsedExplain, parseExplain } from "./explainParser";

export interface ExplainTreeProps {
  raw: { queryPlannerWinningPlan?: unknown; executionStats?: unknown; raw?: unknown } | null;
  /** Initial collapsed depth. `Infinity` = collapse everything; 0 = expand. */
  initialCollapsedDepth?: number;
}

/**
 * Render a MongoDB explain winning-plan tree as nested collapsible
 * cards. Each node shows its stage, optional index name, and per-stage
 * execution stats (docsExamined, nReturned, timeMs, memUsage). At the
 * top: a summary with totals and a count of COLLSCAN/IXSCAN nodes.
 */
export function ExplainTree({ raw, initialCollapsedDepth = 1 }: ExplainTreeProps) {
  const parsed = useMemo(() => parseExplain(raw), [raw]);
  // `overrides` is a sparse set of path → boolean indicating the
  // user's explicit expand (true) or collapse (false) choice for
  // that node. The default behaviour is determined by
  // `initialCollapsedDepth`: nodes deeper than that are collapsed
  // unless overridden.
  const [overrides, setOverrides] = useState<Map<string, boolean>>(new Map());

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
    setOverrides(new Map());
  }

  function collapseAll() {
    const all = collectIds(parsed.root);
    const next = new Map<string, boolean>();
    for (const id of all) next.set(id, false);
    setOverrides(next);
  }

  return (
    <div className="explain-tree">
      <div className="explain-tree__head">
        <ExplainSummary summary={parsed.summary} parsed={parsed} />
        <div className="explain-tree__actions">
          <button className="btn btn--sm" onClick={expandAll}>Expand all</button>
          <button className="btn btn--sm" onClick={collapseAll}>Collapse all</button>
        </div>
      </div>
      <ExplainNodeView
        node={parsed.root}
        depth={0}
        path="0"
        isCollapsed={isCollapsed}
        onToggle={toggle}
      />
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

interface ExplainNodeViewProps {
  node: ExplainNode;
  depth: number;
  path: string;
  isCollapsed: (path: string, depth: number) => boolean;
  onToggle: (path: string, depth: number) => void;
}

function ExplainNodeView({
  node,
  depth,
  path,
  isCollapsed,
  onToggle,
}: ExplainNodeViewProps) {
  const collapsed = isCollapsed(path, depth);
  const stageClass = stageBadgeClass(node.stage);
  const hasChildren = node.children.length > 0;

  return (
    <div className="explain-node" style={{ marginLeft: depth === 0 ? 0 : 14 }}>
      <div className="explain-node__head">
        {hasChildren ? (
          <button
            className="explain-node__toggle"
            onClick={() => onToggle(path, depth)}
            aria-expanded={!collapsed}
            aria-label={collapsed ? "Expand stage" : "Collapse stage"}
          >
            {collapsed ? "▶" : "▼"}
          </button>
        ) : (
          <span className="explain-node__toggle explain-node__toggle--leaf" aria-hidden="true">
            ·
          </span>
        )}
        <span className={`explain-node__stage ${stageClass}`}>{node.stage}</span>
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
  if (node.executionTimeMs !== null) parts.push(`${node.executionTimeMs} ms`);
  if (node.memUsage !== null && node.memUsage > 0) parts.push(`mem ${formatBytes(node.memUsage)}`);
  if (parts.length === 0) return null;
  return <span className="explain-node__stats">{parts.join(" · ")}</span>;
}

function ExplainSummary({
  summary,
  parsed,
}: {
  summary: ParsedExplain["summary"];
  parsed: ParsedExplain;
}) {
  return (
    <div className="explain-tree__summary">
      <span className="explain-tree__pill">{parsed.stageCount} stages</span>
      {parsed.hasCollectionScan && (
        <span className="explain-tree__pill explain-tree__pill--warn" title="A COLLSCAN was used; consider adding an index.">
          COLLSCAN
        </span>
      )}
      {parsed.hasIndexScan && (
        <span className="explain-tree__pill explain-tree__pill--ok">IXSCAN</span>
      )}
      {summary.totalReturned !== null && (
        <span className="explain-tree__pill">
          returned {formatNumber(summary.totalReturned)}
        </span>
      )}
      {summary.totalDocsExamined !== null && (
        <span className="explain-tree__pill">
          examined {formatNumber(summary.totalDocsExamined)}
        </span>
      )}
      {summary.totalKeysExamined !== null && (
        <span className="explain-tree__pill">
          keys {formatNumber(summary.totalKeysExamined)}
        </span>
      )}
      {summary.totalExecutionMs !== null && (
        <span className="explain-tree__pill">{summary.totalExecutionMs} ms</span>
      )}
    </div>
  );
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

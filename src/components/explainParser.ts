/**
 * Walker for MongoDB `explain()` output. Given an `ExplainResult`,
 * walks the `queryPlanner.winningPlan` tree and produces a normalized
 * `ExplainNode` tree that the React UI can render. The walker is
 * defensive about the actual shape (which varies between Mongo
 * versions and between find vs aggregate vs distinct vs count).
 */

export interface ExplainNode {
  /** Stage operator name, e.g. `IXSCAN`, `COLLSCAN`, `FETCH`. */
  stage: string;
  /** Optional index name (for IXSCAN). */
  indexName: string | null;
  /** Optional key pattern (for IXSCAN), preserved as raw JSON. */
  keyPattern: Record<string, unknown> | null;
  /** docsExamined from executionStats (per-stage). */
  docsExamined: number | null;
  /** nReturned from executionStats (per-stage). */
  nReturned: number | null;
  /** executionTimeMillisEstimate from executionStats (per-stage). */
  executionTimeMs: number | null;
  /** Optional memory usage (e.g. for in-memory sorts). */
  memUsage: number | null;
  /** Any extra unknown fields, preserved for the JSON details view. */
  extra: Record<string, unknown>;
  /** Children: single-input stages (`inputStage`) or multi-input stages
   *  (`inputStages[]`, e.g. `$lookup`, `$unionWith`). */
  children: ExplainNode[];
}

export interface ExplainSummary {
  /** nReturned from the top-level executionStats. */
  totalReturned: number | null;
  /** totalDocsExamined from the top-level executionStats. */
  totalDocsExamined: number | null;
  /** totalKeysExamined from the top-level executionStats. */
  totalKeysExamined: number | null;
  /** executionTimeMillis from the top-level executionStats. */
  totalExecutionMs: number | null;
}

export interface ParsedExplain {
  /** The parsed tree (null if the explain JSON has no winning plan). */
  root: ExplainNode | null;
  /** Top-level execution stats summary. */
  summary: ExplainSummary;
  /** Diagnostic: how many stages total. */
  stageCount: number;
  /** True when the plan contains at least one COLLSCAN. */
  hasCollectionScan: boolean;
  /** True when the plan contains at least one IXSCAN. */
  hasIndexScan: boolean;
}

export interface RawExplain {
  queryPlannerWinningPlan?: unknown;
  executionStats?: unknown;
  raw?: unknown;
}

const KNOWN_KEYS = new Set([
  "stage",
  "inputStage",
  "inputStages",
  "indexName",
  "keyPattern",
  "isMultiKey",
  "isUnique",
  "isSparse",
  "direction",
  "indexBounds",
  "docsExamined",
  "nReturned",
  "executionTimeMillisEstimate",
  "memLimit",
  "memUsage",
  "sortPattern",
  "filter",
  "limitAmount",
  "skipAmount",
]);

export function parseExplain(raw: RawExplain | null | undefined): ParsedExplain {
  const winning = unwrapWinningPlan(raw?.queryPlannerWinningPlan ?? null);
  const execution = (raw?.executionStats ?? null) as Record<string, unknown> | null;
  const summary: ExplainSummary = {
    totalReturned: numberOrNull(execution?.nReturned),
    totalDocsExamined: numberOrNull(execution?.totalDocsExamined),
    totalKeysExamined: numberOrNull(execution?.totalKeysExamined),
    totalExecutionMs: numberOrNull(execution?.executionTimeMillis),
  };
  let stageCount = 0;
  let hasCollectionScan = false;
  let hasIndexScan = false;
  const root = winning ? walkStage(winning, {
    onNode() {
      stageCount += 1;
    },
    onStage(name) {
      if (name === "COLLSCAN") hasCollectionScan = true;
      if (name === "IXSCAN") hasIndexScan = true;
    },
  }) : null;
  return { root, summary, stageCount, hasCollectionScan, hasIndexScan };
}

interface WalkCallbacks {
  onNode(): void;
  onStage(stage: string): void;
}

function walkStage(value: unknown, cb: WalkCallbacks): ExplainNode | null {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const obj = value as Record<string, unknown>;
  cb.onNode();
  const stage = typeof obj.stage === "string" ? obj.stage : "(unknown)";
  cb.onStage(stage);

  const extra: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(obj)) {
    if (!KNOWN_KEYS.has(k)) {
      extra[k] = v;
    }
  }

  const children: ExplainNode[] = [];
  const inputStage = obj.inputStage;
  if (inputStage) {
    const child = walkStage(inputStage, cb);
    if (child) children.push(child);
  }
  const inputStages = obj.inputStages;
  if (Array.isArray(inputStages)) {
    for (const child of inputStages) {
      const c = walkStage(child, cb);
      if (c) children.push(c);
    }
  }

  return {
    stage,
    indexName: typeof obj.indexName === "string" ? obj.indexName : null,
    keyPattern:
      obj.keyPattern && typeof obj.keyPattern === "object" && !Array.isArray(obj.keyPattern)
        ? (obj.keyPattern as Record<string, unknown>)
        : null,
    docsExamined: numberOrNull(obj.docsExamined),
    nReturned: numberOrNull(obj.nReturned),
    executionTimeMs: numberOrNull(obj.executionTimeMillisEstimate),
    memUsage: numberOrNull(obj.memUsage),
    extra,
    children,
  };
}

function numberOrNull(v: unknown): number | null {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  return null;
}

/**
 * The Rust `explain_aggregate` path currently returns the full explain
 * document as `queryPlannerWinningPlan`, while `explain_find` returns
 * the inner `queryPlanner.winningPlan`. This helper unwraps either
 * shape so the React walker gets the same input regardless of which
 * IPC was used.
 */
function unwrapWinningPlan(winning: unknown): unknown {
  if (!winning || typeof winning !== "object" || Array.isArray(winning)) return null;
  const obj = winning as Record<string, unknown>;
  if (obj.queryPlanner && typeof obj.queryPlanner === "object") {
    const qp = obj.queryPlanner as Record<string, unknown>;
    if (qp.winningPlan) return qp.winningPlan;
  }
  // Some explain responses put `stages` (aggregate-specific) at the
  // top level instead of nesting it inside `queryPlanner`.
  if (Array.isArray(obj.stages)) return { stage: "SHARDED_PIPELINE", inputStages: obj.stages };
  return obj;
}

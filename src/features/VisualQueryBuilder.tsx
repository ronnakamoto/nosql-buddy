import { useCallback, useEffect, useState } from "react";
import commands, { type VqbCombinator, type VqbNode, type VqbTranslateRequest } from "../ipc/commands";

const OPERATORS: { value: string; label: string }[] = [
  { value: "eq", label: "=" },
  { value: "ne", label: "≠" },
  { value: "gt", label: ">" },
  { value: "gte", label: "≥" },
  { value: "lt", label: "<" },
  { value: "lte", label: "≤" },
  { value: "in", label: "in" },
  { value: "nin", label: "not in" },
  { value: "exists", label: "exists" },
  { value: "regex", label: "regex" },
  { value: "text", label: "text search" },
  { value: "is_null", label: "is null" },
  { value: "is_not_null", label: "is not null" },
];

const VALUELESS_OPERATORS = new Set(["is_null", "is_not_null"]);

export interface VisualQueryBuilderProps {
  filterJson: string;
  onFilterJsonChange: (filterJson: string) => void;
  disabled?: boolean;
}

function defaultRoot(): VqbNode {
  return {
    kind: "group",
    combinator: "and",
    children: [defaultCondition()],
  };
}

function defaultCondition(): VqbNode {
  return {
    kind: "condition",
    field: "",
    operator: "eq",
    value: "",
    enabled: true,
  };
}

async function translateVqb(node: VqbNode): Promise<Record<string, unknown>> {
  const req: VqbTranslateRequest = { node };
  return commands.translateVqb(req) as Promise<Record<string, unknown>>;
}

export function VisualQueryBuilder({
  filterJson,
  onFilterJsonChange,
  disabled = false,
}: VisualQueryBuilderProps) {
  const [root, setRoot] = useState<VqbNode>(defaultRoot);
  const [parseError, setParseError] = useState<string | null>(null);

  // Parse the incoming JSON filter into a VQB tree whenever it changes.
  useEffect(() => {
    let cancelled = false;
    const parse = async () => {
      try {
        const trimmed = filterJson.trim();
        if (!trimmed || trimmed === "{}") {
          if (!cancelled) {
            setRoot(defaultRoot());
            setParseError(null);
          }
          return;
        }
        const parsed = JSON.parse(trimmed) as Record<string, unknown>;
        // Best-effort local parse of the JSON filter back into a VQB tree.
        if (!cancelled) {
          const node = parseFilterToVqb(parsed);
          setRoot(node);
          setParseError(null);
        }
      } catch (e) {
        if (!cancelled) {
          setParseError(e instanceof Error ? e.message : "Invalid filter JSON");
        }
      }
    };
    void parse();
    return () => {
      cancelled = true;
    };
  }, [filterJson]);

  // Translate the current VQB tree to a JSON filter and notify the parent.
  const emitChange = useCallback(async (node: VqbNode) => {
    try {
      const filter = await translateVqb(node);
      onFilterJsonChange(JSON.stringify(filter));
      setParseError(null);
    } catch (e) {
      setParseError(e instanceof Error ? e.message : "Translation failed");
    }
  }, [onFilterJsonChange]);

  const updateRoot = useCallback(
    (updater: (node: VqbNode) => VqbNode) => {
      const next = updater(root);
      setRoot(next);
      void emitChange(next);
    },
    [root, emitChange],
  );

  return (
    <div className={`vqb ${disabled ? "vqb--disabled" : ""}`}>
      {parseError && (
        <div className="vqb__notice vqb__notice--error">{parseError}</div>
      )}
      <GroupEditor
        node={root}
        path={[]}
        onChange={(next) => updateRoot(() => next)}
        level={0}
      />
      <div className="vqb__raw">
        <span className="vqb__raw-label">Generated filter</span>
        <code className="vqb__raw-code">{filterJson}</code>
      </div>
    </div>
  );
}

interface GroupEditorProps {
  node: VqbNode;
  path: number[];
  onChange: (node: VqbNode) => void;
  level: number;
}

function GroupEditor({ node, path, onChange, level }: GroupEditorProps) {
  if (node.kind !== "group") return null;
  const { combinator, children } = node;

  const updateChild = (idx: number, next: VqbNode) => {
    const nextChildren = [...children];
    nextChildren[idx] = next;
    onChange({ ...node, children: nextChildren });
  };

  const addCondition = () => {
    onChange({
      ...node,
      children: [...children, defaultCondition()],
    });
  };

  const addGroup = () => {
    onChange({
      ...node,
      children: [
        ...children,
        { kind: "group", combinator: "and", children: [defaultCondition()] },
      ],
    });
  };

  const removeChild = (idx: number) => {
    const nextChildren = children.filter((_, i) => i !== idx);
    if (nextChildren.length === 0) {
      // Keep at least one condition so the group isn't empty.
      onChange({ ...node, children: [defaultCondition()] });
    } else {
      onChange({ ...node, children: nextChildren });
    }
  };

  return (
    <div className="vqb__group" data-level={level}>
      <div className="vqb__group-header">
        <select
          className="vqb__combinator"
          value={combinator}
          onChange={(e) =>
            onChange({ ...node, combinator: e.target.value as VqbCombinator })
          }
          aria-label="Group combinator"
        >
          <option value="and">Match all (AND)</option>
          <option value="or">Match any (OR)</option>
          <option value="nor">Match none (NOR)</option>
        </select>
        <span className="vqb__group-meta">{children.length} clause(s)</span>
      </div>
      <div className="vqb__children">
        {children.map((child, idx) => {
          const childPath = [...path, idx];
          const key = childPath.join("-");
          if (child.kind === "group") {
            return (
              <div key={key} className="vqb__child vqb__child--nested">
                <GroupEditor
                  node={child}
                  path={childPath}
                  onChange={(next) => updateChild(idx, next)}
                  level={level + 1}
                />
                <button
                  className="btn btn--sm btn--danger vqb__remove"
                  onClick={() => removeChild(idx)}
                  title="Remove group"
                >
                  ×
                </button>
              </div>
            );
          }
          return (
            <div key={key} className="vqb__child">
              <ConditionEditor
                node={child}
                onChange={(next) => updateChild(idx, next)}
              />
              <button
                className="btn btn--sm vqb__clone"
                onClick={() =>
                  onChange({
                    ...node,
                    children: [
                      ...children.slice(0, idx + 1),
                      { ...child },
                      ...children.slice(idx + 1),
                    ],
                  })
                }
                title="Clone clause"
              >
                Clone
              </button>
              <button
                className="btn btn--sm btn--danger vqb__remove"
                onClick={() => removeChild(idx)}
                title="Remove clause"
              >
                ×
              </button>
            </div>
          );
        })}
      </div>
      <div className="vqb__group-actions">
        <button className="btn btn--sm" onClick={addCondition}>
          + Condition
        </button>
        <button className="btn btn--sm" onClick={addGroup}>
          + Group
        </button>
      </div>
    </div>
  );
}

interface ConditionEditorProps {
  node: VqbNode;
  onChange: (node: VqbNode) => void;
}

function ConditionEditor({ node, onChange }: ConditionEditorProps) {
  if (node.kind !== "condition") return null;
  const { field, operator, value, enabled } = node;
  const valueless = VALUELESS_OPERATORS.has(operator);

  return (
    <div className={`vqb__condition ${!enabled ? "vqb__condition--disabled" : ""}`}>
      <input
        type="checkbox"
        className="vqb__toggle"
        checked={enabled}
        onChange={(e) => onChange({ ...node, enabled: e.target.checked })}
        title="Enable clause"
        aria-label="Enable clause"
      />
      <input
        type="text"
        className="vqb__field"
        placeholder="field"
        value={field}
        onChange={(e) => onChange({ ...node, field: e.target.value })}
        aria-label="Field"
      />
      <select
        className="vqb__operator"
        value={operator}
        onChange={(e) =>
          onChange({
            ...node,
            operator: e.target.value,
            value: VALUELESS_OPERATORS.has(e.target.value) ? "" : value,
          })
        }
        aria-label="Operator"
      >
        {OPERATORS.map((op) => (
          <option key={op.value} value={op.value}>
            {op.label}
          </option>
        ))}
      </select>
      {!valueless && (
        <input
          type="text"
          className="vqb__value"
          placeholder={operator === "in" || operator === "nin" ? "a, b, c" : "value"}
          value={value == null ? "" : String(value)}
          onChange={(e) => onChange({ ...node, value: e.target.value })}
          aria-label="Value"
        />
      )}
    </div>
  );
}

/**
 * Best-effort local parser that turns a MongoDB filter JSON into a VQB node tree.
 * Only handles the patterns produced by the backend `to_filter` function.
 */
function parseFilterToVqb(value: unknown): VqbNode {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return defaultRoot();
  }
  const obj = value as Record<string, unknown>;
  if (Object.keys(obj).length === 0) {
    return defaultRoot();
  }

  // Top-level group operators.
  if (obj.$and !== undefined && Array.isArray(obj.$and)) {
    return {
      kind: "group",
      combinator: "and",
      children: obj.$and.map(parseFilterToVqb),
    };
  }
  if (obj.$or !== undefined && Array.isArray(obj.$or)) {
    return {
      kind: "group",
      combinator: "or",
      children: obj.$or.map(parseFilterToVqb),
    };
  }
  if (obj.$nor !== undefined && Array.isArray(obj.$nor)) {
    return {
      kind: "group",
      combinator: "nor",
      children: obj.$nor.map(parseFilterToVqb),
    };
  }

  // Treat each top-level key as an AND condition.
  const children: VqbNode[] = [];
  for (const [field, val] of Object.entries(obj)) {
    if (field === "$text") {
      const search =
        typeof val === "object" && val !== null
          ? (val as Record<string, unknown>).$search
          : undefined;
      children.push({
        kind: "condition",
        field: "$text",
        operator: "text",
        value: search ?? "",
        enabled: true,
      });
      continue;
    }
    const cond = parseFieldValue(field, val);
    if (cond) children.push(cond);
  }
  if (children.length === 0) {
    return defaultRoot();
  }
  if (children.length === 1) {
    return children[0];
  }
  return { kind: "group", combinator: "and", children };
}

function parseFieldValue(field: string, value: unknown): VqbNode | null {
  if (typeof value !== "object" || value === null) {
    return {
      kind: "condition",
      field,
      operator: value === null ? "is_null" : "eq",
      value: value === null ? "" : value,
      enabled: true,
    };
  }
  const obj = value as Record<string, unknown>;
  const entries = Object.entries(obj);
  if (entries.length === 0) {
    return null;
  }
  if (entries.length === 1) {
    const [op, val] = entries[0];
    const operator = mongoToVqbOp(op);
    if (operator) {
      return {
        kind: "condition",
        field,
        operator,
        value: val ?? "",
        enabled: true,
      };
    }
  }
  // Multi-operator object: convert to a group of conditions on the same field.
  const children: VqbNode[] = [];
  for (const [op, val] of entries) {
    const operator = mongoToVqbOp(op);
    if (!operator) continue;
    children.push({
      kind: "condition",
      field,
      operator,
      value: val ?? "",
      enabled: true,
    });
  }
  if (children.length === 0) return null;
  return { kind: "group", combinator: "and", children };
}

function mongoToVqbOp(op: string): string | null {
  switch (op) {
    case "$eq":
      return "eq";
    case "$ne":
      return "ne";
    case "$gt":
      return "gt";
    case "$gte":
      return "gte";
    case "$lt":
      return "lt";
    case "$lte":
      return "lte";
    case "$in":
      return "in";
    case "$nin":
      return "nin";
    case "$exists":
      return "exists";
    case "$regex":
      return "regex";
    case "$type":
      return "type";
    case "$size":
      return "size";
    default:
      return null;
  }
}

import { useCallback, useEffect, useMemo, useState } from "react";
import commands, {
  type VqbCombinator,
  type VqbNode,
  type VqbTranslateRequest,
} from "../ipc/commands";
import { InfoPopover } from "../components/InfoPopover";

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
  { value: "type", label: "type" },
  { value: "size", label: "size" },
  { value: "elem_match", label: "elemMatch" },
  { value: "expr", label: "$expr" },
  { value: "mod", label: "mod" },
  { value: "json_schema", label: "$jsonSchema" },
  { value: "is_null", label: "is null" },
  { value: "is_not_null", label: "is not null" },
];

const VALUELESS_OPERATORS = new Set(["is_null", "is_not_null"]);

/** Top-level operators that don't bind to a user-typed field. */
const TOP_LEVEL_OPERATORS: Record<string, string> = {
  text: "$text",
  expr: "$expr",
  json_schema: "$jsonSchema",
};

/** Operators whose value is a JSON object (rendered as a textarea). */
const JSON_OBJECT_OPERATORS = new Set(["elem_match", "expr", "json_schema"]);

/** BSON type aliases for the $type operator. */
const BSON_TYPES = [
  "double", "string", "object", "array", "binData", "objectId",
  "bool", "date", "null", "regex", "int", "long", "decimal",
  "timestamp", "minKey", "maxKey",
];

export interface VisualQueryBuilderProps {
  filterJson: string;
  onFilterJsonChange: (filterJson: string) => void;
  disabled?: boolean;
  connectionId?: string;
  database?: string;
  collection?: string;
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
  connectionId,
  database,
  collection,
}: VisualQueryBuilderProps) {
  const [root, setRoot] = useState<VqbNode>(defaultRoot);
  const [parseError, setParseError] = useState<string | null>(null);
  const [schemaFields, setSchemaFields] = useState<string[]>([]);

  // Fetch schema for field autocomplete.
  useEffect(() => {
    if (!connectionId || !database || !collection) {
      setSchemaFields([]);
      return;
    }
    let cancelled = false;
    const fetchSchema = async () => {
      try {
        const report = await commands.sampleSchema(connectionId, database, collection);
        if (!cancelled) {
          setSchemaFields(report.fields.map((f) => f.name));
        }
      } catch {
        // Schema sampling can fail (permissions, empty collection); silently degrade.
        if (!cancelled) setSchemaFields([]);
      }
    };
    void fetchSchema();
    return () => { cancelled = true; };
  }, [connectionId, database, collection]);

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
    return () => { cancelled = true; };
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

  const datalistId = useMemo(
    () => `vqb-fields-${Math.random().toString(36).slice(2, 8)}`,
    [],
  );

  return (
    <div className={`vqb ${disabled ? "vqb--disabled" : ""}`}>
      {parseError && (
        <div className="vqb__notice vqb__notice--error">{parseError}</div>
      )}
      {/* Hidden datalist for field autocomplete, shared by all condition editors. */}
      {schemaFields.length > 0 && (
        <datalist id={datalistId}>
          {schemaFields.map((f) => (
            <option key={f} value={f} />
          ))}
        </datalist>
      )}
      <GroupEditor
        node={root}
        path={[]}
        onChange={(next) => updateRoot(() => next)}
        level={0}
        datalistId={datalistId}
        hasSchema={schemaFields.length > 0}
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
  datalistId: string;
  hasSchema: boolean;
}

function GroupEditor({ node, path, onChange, level, datalistId, hasSchema }: GroupEditorProps) {
  const [draggingIdx, setDraggingIdx] = useState<number | null>(null);

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
      onChange({ ...node, children: [defaultCondition()] });
    } else {
      onChange({ ...node, children: nextChildren });
    }
  };

  const moveChild = (from: number, to: number) => {
    if (from === to) return;
    const nextChildren = [...children];
    const [moved] = nextChildren.splice(from, 1);
    nextChildren.splice(to, 0, moved);
    onChange({ ...node, children: nextChildren });
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
        <InfoPopover label="Group combinator help" title="Group combinator">
        <p><strong>AND</strong>: all conditions must match.</p>
        <p><strong>OR</strong>: at least one condition must match.</p>
        <p><strong>NOR</strong>: none of the conditions may match (inverse of OR).</p>
      </InfoPopover>
        <span className="vqb__group-meta">{children.length} clause(s)</span>
      </div>
      <div className="vqb__children">
        {children.map((child, idx) => {
          const childPath = [...path, idx];
          const key = childPath.join("-");
          if (child.kind === "group") {
            return (
              <div
                key={key}
                className={`vqb__child vqb__child--nested ${draggingIdx === idx ? "vqb__child--dragging" : ""}`}
                draggable
                onDragStart={() => setDraggingIdx(idx)}
                onDragOver={(e) => e.preventDefault()}
                onDragEnd={() => setDraggingIdx(null)}
                onDrop={(e) => {
                  e.preventDefault();
                  if (draggingIdx !== null && draggingIdx !== idx) {
                    moveChild(draggingIdx, idx);
                  }
                  setDraggingIdx(null);
                }}
              >
                <span className="vqb__drag-handle" title="Drag to reorder">⋮⋮</span>
                <GroupEditor
                  node={child}
                  path={childPath}
                  onChange={(next) => updateChild(idx, next)}
                  level={level + 1}
                  datalistId={datalistId}
                  hasSchema={hasSchema}
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
            <div
              key={key}
              className={`vqb__child ${draggingIdx === idx ? "vqb__child--dragging" : ""}`}
              draggable
              onDragStart={() => setDraggingIdx(idx)}
              onDragOver={(e) => e.preventDefault()}
              onDragEnd={() => setDraggingIdx(null)}
              onDrop={(e) => {
                e.preventDefault();
                if (draggingIdx !== null && draggingIdx !== idx) {
                  moveChild(draggingIdx, idx);
                }
                setDraggingIdx(null);
              }}
            >
              <span className="vqb__drag-handle" title="Drag to reorder">⋮⋮</span>
              <ConditionEditor
                node={child}
                onChange={(next) => updateChild(idx, next)}
                datalistId={datalistId}
                hasSchema={hasSchema}
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
  datalistId: string;
  hasSchema: boolean;
}

function ConditionEditor({ node, onChange, datalistId, hasSchema }: ConditionEditorProps) {
  if (node.kind !== "condition") return null;
  const { field, operator, value, enabled } = node;
  const valueless = VALUELESS_OPERATORS.has(operator);
  const topLevelOp = TOP_LEVEL_OPERATORS[operator];
  const isJsonObject = JSON_OBJECT_OPERATORS.has(operator);
  const isMod = operator === "mod";
  const isType = operator === "type";
  const isSize = operator === "size";

  const handleOperatorChange = (newOp: string) => {
    const newTopLevel = TOP_LEVEL_OPERATORS[newOp];
    const newIsMod = newOp === "mod";
    const newIsJsonObject = JSON_OBJECT_OPERATORS.has(newOp);
    const newIsValueless = VALUELESS_OPERATORS.has(newOp);
    // Reset value when switching to an operator with a different value shape.
    const needsReset =
      newIsValueless ||
      newIsMod ||
      newIsJsonObject ||
      isMod ||
      isJsonObject;
    onChange({
      ...node,
      operator: newOp,
      // Auto-set field for top-level operators; clear if leaving a top-level op.
      field: newTopLevel ?? (topLevelOp ? "" : field),
      value: newIsValueless
        ? ""
        : newIsMod
          ? "0, 0"
          : needsReset
            ? ""
            : value,
    });
  };

  // For $mod: render two number inputs.
  const modParts = typeof value === "string" ? value.split(",").map((s) => s.trim()) : ["0", "0"];
  const modDivisor = modParts[0] ?? "0";
  const modRemainder = modParts[1] ?? "0";

  const setModValue = (divisor: string, remainder: string) => {
    onChange({ ...node, value: `${divisor}, ${remainder}` });
  };

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
      {!topLevelOp && (
        <input
          type="text"
          className="vqb__field"
          placeholder="field"
          value={field}
          list={hasSchema ? datalistId : undefined}
          onChange={(e) => onChange({ ...node, field: e.target.value })}
          aria-label="Field"
        />
      )}
      {topLevelOp && (
        <span className="vqb__field vqb__field--fixed">{topLevelOp}</span>
      )}
      <select
        className="vqb__operator"
        value={operator}
        onChange={(e) => handleOperatorChange(e.target.value)}
        aria-label="Operator"
      >
        {OPERATORS.map((op) => (
          <option key={op.value} value={op.value}>
            {op.label}
          </option>
        ))}
      </select>
      {!valueless && !isJsonObject && !isMod && !isType && !isSize && (
        <input
          type="text"
          className="vqb__value"
          placeholder={operator === "in" || operator === "nin" ? "a, b, c" : "value"}
          value={value == null ? "" : String(value)}
          onChange={(e) => onChange({ ...node, value: e.target.value })}
          aria-label="Value"
        />
      )}
      {isType && (
        <select
          className="vqb__value vqb__value--select"
          value={value == null ? "" : String(value)}
          onChange={(e) => onChange({ ...node, value: e.target.value })}
          aria-label="BSON type"
        >
          {BSON_TYPES.map((t) => (
            <option key={t} value={t}>{t}</option>
          ))}
        </select>
      )}
      {isSize && (
        <input
          type="number"
          className="vqb__value"
          placeholder="array length"
          value={value == null ? "" : String(value)}
          onChange={(e) => onChange({ ...node, value: e.target.value })}
          aria-label="Array length"
        />
      )}
      {isMod && (
        <div className="vqb__mod-inputs">
          <input
            type="number"
            className="vqb__value vqb__value--mod"
            placeholder="divisor"
            value={modDivisor}
            onChange={(e) => setModValue(e.target.value, modRemainder)}
            aria-label="Divisor"
          />
          <span className="vqb__mod-sep">mod</span>
          <input
            type="number"
            className="vqb__value vqb__value--mod"
            placeholder="remainder"
            value={modRemainder}
            onChange={(e) => setModValue(modDivisor, e.target.value)}
            aria-label="Remainder"
          />
        </div>
      )}
      {isJsonObject && (
        <textarea
          className="vqb__value vqb__value--json"
          placeholder={
            operator === "elem_match"
              ? '{ "qty": { "$gt": 5 } }'
              : operator === "expr"
                ? '{ "$gt": ["$price", "$cost"] }'
                : '{ "required": ["name"] }'
          }
          value={value == null ? "" : String(value)}
          onChange={(e) => onChange({ ...node, value: e.target.value })}
          spellCheck={false}
          rows={3}
          aria-label="JSON value"
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
    if (field === "$expr") {
      children.push({
        kind: "condition",
        field: "$expr",
        operator: "expr",
        value: val == null ? "" : JSON.stringify(val),
        enabled: true,
      });
      continue;
    }
    if (field === "$jsonSchema") {
      children.push({
        kind: "condition",
        field: "$jsonSchema",
        operator: "json_schema",
        value: val == null ? "" : JSON.stringify(val),
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
      // For $mod, serialize the array as "d, r".
      if (operator === "mod" && Array.isArray(val) && val.length === 2) {
        return {
          kind: "condition",
          field,
          operator,
          value: `${val[0]}, ${val[1]}`,
          enabled: true,
        };
      }
      // For $elemMatch, serialize the sub-filter as JSON string.
      if (operator === "elem_match") {
        return {
          kind: "condition",
          field,
          operator,
          value: JSON.stringify(val),
          enabled: true,
        };
      }
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
    if (operator === "mod" && Array.isArray(val) && val.length === 2) {
      children.push({
        kind: "condition",
        field,
        operator,
        value: `${val[0]}, ${val[1]}`,
        enabled: true,
      });
    } else if (operator === "elem_match") {
      children.push({
        kind: "condition",
        field,
        operator,
        value: JSON.stringify(val),
        enabled: true,
      });
    } else {
      children.push({
        kind: "condition",
        field,
        operator,
        value: val ?? "",
        enabled: true,
      });
    }
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
    case "$elemMatch":
      return "elem_match";
    case "$mod":
      return "mod";
    default:
      return null;
  }
}

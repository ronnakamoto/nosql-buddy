import { useMemo } from "react";
import type { FieldMappingEntry, TypeOverride } from "../../ipc/commands";

/**
 * Field-mapping table for the import/export wizards. Lets the user rename,
 * skip, flatten (dotted source paths), and apply type overrides per field.
 *
 * Contract (matches `mapping.rs::FieldMappingTransform`): the table is the
 * complete output schema — undeclared fields are dropped. The wizard derives
 * the initial rows from a schema sample (export) or import preview (import),
 * so the user starts from "everything included, identity-renamed" and edits
 * down.
 */

export interface DiscoveredField {
  /** Dotted source path, e.g. `address.city`. */
  path: string;
  /** Inferred BSON type label, e.g. `string`, `int32`, `object`, `unknown`. */
  bsonType: string;
  /** When true, the field is a nested object the user can expand into leaves. */
  isObject: boolean;
  /** Sample values for display (truncated). */
  samples: string[];
}

export interface FieldMappingTableProps {
  fields: DiscoveredField[];
  /** Current mapping entries, ordered as the user has arranged them. */
  entries: FieldMappingEntry[];
  onChange: (entries: FieldMappingEntry[]) => void;
  /** Disable all editing while a job is running. */
  disabled?: boolean;
}

const TYPE_OPTIONS: Array<{ value: TypeOverride | null; label: string }> = [
  { value: null, label: "auto" },
  { value: "string", label: "string" },
  { value: "int32", label: "int32" },
  { value: "int64", label: "int64" },
  { value: "double", label: "double" },
  { value: "boolean", label: "boolean" },
  { value: "date", label: "date" },
  { value: "objectId", label: "objectId" },
];

/** Build the initial identity mapping from a set of discovered fields. */
export function identityMapping(fields: DiscoveredField[]): FieldMappingEntry[] {
  return fields
    .filter((f) => !f.isObject) // nested objects start collapsed; leaves map
    .map((f) => ({
      source: f.path,
      target: f.path,
      skip: false,
      typeOverride: null,
    }));
}

/** Derive the discovered-fields list from an import preview's `fields`. */
export function discoveredFieldsFromInference(
  inferred: Array<{
    name: string;
    bsonType: string;
    samples: string[];
  }>,
): DiscoveredField[] {
  // The import preview flattens nothing — top-level keys only. A bsonType of
  // "object" marks an expandable nested field.
  return inferred.map((f) => ({
    path: f.name,
    bsonType: f.bsonType,
    isObject: f.bsonType === "object",
    samples: f.samples,
  }));
}

/** Expand a nested object field into its leaf dotted paths. Returns the new
 * entries to splice in place of the collapsed row. */
export function expandField(
  entries: FieldMappingEntry[],
  collapsedSource: string,
  leaves: string[],
): FieldMappingEntry[] {
  const idx = entries.findIndex((e) => e.source === collapsedSource);
  if (idx === -1) return entries;
  const replacement: FieldMappingEntry[] = leaves.map((leaf) => ({
    source: `${collapsedSource}.${leaf}`,
    target: `${collapsedSource}.${leaf}`,
    skip: false,
    typeOverride: null,
  }));
  return [...entries.slice(0, idx), ...replacement, ...entries.slice(idx + 1)];
}

export function FieldMappingTable({
  fields,
  entries,
  onChange,
  disabled = false,
}: FieldMappingTableProps) {
  // Pre-compute the leaf paths for each expandable object field, so the
  // "expand" action knows what to splice in. The wizard supplies `fields` with
  // dotted paths already; for objects we look at nested paths sharing the
  // prefix.
  const leafMap = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const f of fields) {
      if (f.isObject) {
        const prefix = `${f.path}.`;
        const leaves = fields
          .filter((x) => x.path.startsWith(prefix) && !x.isObject)
          .map((x) => x.path.slice(prefix.length));
        map.set(f.path, leaves);
      }
    }
    return map;
  }, [fields]);

  const fieldByPath = useMemo(() => {
    const m = new Map<string, DiscoveredField>();
    for (const f of fields) m.set(f.path, f);
    return m;
  }, [fields]);

  const update = (idx: number, patch: Partial<FieldMappingEntry>) => {
    const next = entries.map((e, i) => (i === idx ? { ...e, ...patch } : e));
    onChange(next);
  };

  const remove = (idx: number) => {
    onChange(entries.filter((_, i) => i !== idx));
  };

  const expand = (source: string) => {
    const leaves = leafMap.get(source) ?? [];
    if (leaves.length === 0) return;
    onChange(expandField(entries, source, leaves));
  };

  const includedCount = entries.filter((e) => !e.skip).length;

  if (entries.length === 0) {
    return (
      <div className="toast toast--warning" style={{ position: "static", margin: 0 }}>
        No fields discovered yet. Run a preview or provide a source with data.
      </div>
    );
  }

  return (
    <div className="field">
      <span className="field__label">
        Field mapping · {includedCount}/{entries.length} included
      </span>
      <div
        style={{
          maxHeight: 240,
          overflow: "auto",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-md)",
        }}
      >
        <table className="results-grid" style={{ width: "100%" }}>
          <thead>
            <tr>
              <th style={{ width: 40 }}>In</th>
              <th>Source path</th>
              <th>Target name</th>
              <th style={{ width: 110 }}>Type</th>
              <th style={{ width: 40 }}>·</th>
            </tr>
          </thead>
          <tbody>
            {entries.map((entry, idx) => {
              const field = fieldByPath.get(entry.source);
              const isObject = field?.isObject ?? false;
              const canExpand = isObject && leafMap.get(entry.source)?.length;
              const bsonType = field?.bsonType ?? "unknown";
              return (
                <tr key={`${entry.source}-${idx}`}>
                  <td style={{ textAlign: "center" }}>
                    <input
                      type="checkbox"
                      checked={!entry.skip}
                      onChange={(e) => update(idx, { skip: !e.target.checked })}
                      disabled={disabled}
                      aria-label={`Include ${entry.source}`}
                    />
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: "var(--space-1)" }}>
                      <code style={{ fontSize: 12 }}>{entry.source}</code>
                      <span
                        style={{
                          fontSize: 11,
                          color: "var(--ink-faint)",
                          textTransform: "lowercase",
                        }}
                      >
                        {bsonType}
                      </span>
                      {canExpand && (
                        <button
                          className="btn btn--sm"
                          style={{ padding: "0 6px", fontSize: 11 }}
                          onClick={() => expand(entry.source)}
                          disabled={disabled}
                          title="Expand nested object into leaf fields"
                        >
                          expand
                        </button>
                      )}
                    </div>
                  </td>
                  <td>
                    <input
                      className="field__input"
                      value={entry.target}
                      onChange={(e) => update(idx, { target: e.target.value })}
                      disabled={disabled || entry.skip}
                      style={{
                        padding: "var(--space-1) var(--space-2)",
                        fontSize: 12,
                        fontFamily: "var(--font-mono)",
                      }}
                      aria-label={`Target name for ${entry.source}`}
                    />
                  </td>
                  <td>
                    <select
                      className="field__select"
                      value={entry.typeOverride ?? ""}
                      onChange={(e) =>
                        update(idx, {
                          typeOverride: (e.target.value || null) as TypeOverride | null,
                        })
                      }
                      disabled={disabled || entry.skip}
                      style={{
                        padding: "var(--space-1) var(--space-2)",
                        fontSize: 12,
                        width: "100%",
                      }}
                      aria-label={`Type override for ${entry.source}`}
                    >
                      {TYPE_OPTIONS.map((opt) => (
                        <option key={opt.label} value={opt.value ?? ""}>
                          {opt.label}
                        </option>
                      ))}
                    </select>
                  </td>
                  <td style={{ textAlign: "center" }}>
                    <button
                      className="btn btn--sm btn--ghost"
                      style={{ padding: "0 6px", fontSize: 11, color: "var(--danger-500)" }}
                      onClick={() => remove(idx)}
                      disabled={disabled}
                      title="Remove this field from the mapping"
                      aria-label={`Remove ${entry.source} from mapping`}
                    >
                      ×
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <span className="field__hint">
        Undeclared fields are dropped. Use a dotted source path to flatten a
        nested object (e.g. <code>address.city</code>).
      </span>
    </div>
  );
}

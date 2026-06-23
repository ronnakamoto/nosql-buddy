import { useMemo, useState } from "react";
import { EditableCell } from "./EditableCell";
import { detectKind, displayValue, getByPath, kindClassName } from "./resultsDisplay";

export type ResultsViewMode = "table" | "tree" | "json";

export interface ResultsTableProps {
  documents: Array<Record<string, unknown>>;
  connectionId: string;
  database: string;
  collection: string;
  pageSize?: number;
  view?: ResultsViewMode;
  /** When false, cells render as static text. Default true. */
  editable?: boolean;
  /** Called when a cell save succeeds so the parent can refresh. */
  onCellSaved?: (rowIdx: number, fieldPath: string, newValue: unknown) => void;
  /** Called when a cell save fails so the parent can toast the error. */
  onCellError?: (rowIdx: number, fieldPath: string, message: string) => void;
  /** Called when the user clicks the row delete button. */
  onDeleteRow?: (rowIdx: number, doc: Record<string, unknown>) => void;
}

export function ResultsTable({
  documents,
  connectionId,
  database,
  collection,
  pageSize = 200,
  view = "table",
  editable = true,
  onCellSaved,
  onCellError,
  onDeleteRow,
}: ResultsTableProps) {
  const rows = useMemo(() => documents.slice(0, pageSize), [documents, pageSize]);

  if (rows.length === 0) {
    return (
      <div className="empty-state">
        <h2>No documents</h2>
        <p>Try removing the filter, or insert a document to get started.</p>
      </div>
    );
  }

  if (view === "json") {
    return (
      <pre className="json-view" aria-label="JSON results">
        {JSON.stringify(rows, null, 2)}
      </pre>
    );
  }

  if (view === "tree") {
    return (
      <div className="results-tree" aria-label="Tree results">
        {rows.map((row, i) => (
          <div key={i} className="results-tree__doc">
            <div className="results-tree__doc-header">
              <span className="results-tree__doc-label">doc {i}</span>
              {onDeleteRow && (
                <button
                  className="btn btn--sm btn--danger"
                  onClick={() => onDeleteRow(i, row)}
                  title="Delete this document"
                >
                  Delete
                </button>
              )}
            </div>
            <TreeNode value={row} />
          </div>
        ))}
      </div>
    );
  }

  return (
    <TableView
      rows={rows}
      connectionId={connectionId}
      database={database}
      collection={collection}
      editable={editable}
      onCellSaved={onCellSaved}
      onCellError={onCellError}
      onDeleteRow={onDeleteRow}
    />
  );
}

function TableView({
  rows,
  connectionId,
  database,
  collection,
  editable,
  onCellSaved,
  onCellError,
  onDeleteRow,
}: {
  rows: Array<Record<string, unknown>>;
  connectionId: string;
  database: string;
  collection: string;
  editable: boolean;
  onCellSaved?: (rowIdx: number, fieldPath: string, newValue: unknown) => void;
  onCellError?: (rowIdx: number, fieldPath: string, message: string) => void;
  onDeleteRow?: (rowIdx: number, doc: Record<string, unknown>) => void;
}) {
  const columns = useMemo(() => {
    const seen = new Set<string>();
    const order: string[] = [];
    for (const row of rows) {
      for (const k of Object.keys(row)) {
        if (!seen.has(k)) {
          seen.add(k);
          order.push(k);
        }
      }
      if (order.length >= 24) break;
    }
    return order;
  }, [rows]);

  const isNumeric = (kind: string): boolean =>
    ["int", "long", "double", "decimal"].includes(kind);

  return (
    <table className="results-grid" role="grid" aria-label="Query results">
      <thead>
        <tr>
          {columns.map((c) => (
            <th key={c} scope="col">
              {c}
            </th>
          ))}
          {onDeleteRow && (
            <th className="results-grid__actions" scope="col" aria-label="Row actions" />
          )}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, i) => (
          <tr key={i}>
            {columns.map((c) => {
              const value = getByPath(row, c);
              if (editable && onCellSaved && onCellError) {
                return (
                  <td key={c}>
                    <EditableCell
                      row={row}
                      fieldPath={c}
                      value={value}
                      connectionId={connectionId}
                      database={database}
                      collection={collection}
                      onSaved={(newValue) => onCellSaved(i, c, newValue)}
                      onError={(message) => onCellError(i, c, message)}
                    />
                  </td>
                );
              }
              const kind = detectKind(value);
              const valueText = displayValue(value);
              const complex = ["array", "object"].includes(kind);
              return (
                <td key={c}>
                  <span
                    className={`results-grid__cell ${isNumeric(kind) ? "results-grid__cell--numeric" : ""}`}
                    title={valueText}
                  >
                    <span className={`kind-badge ${kindClassName(kind)}`}>{kind}</span>
                    <span
                      className={`results-grid__value ${complex ? "results-grid__value--wrap" : ""}`}
                    >
                      {valueText}
                    </span>
                  </span>
                </td>
              );
            })}
            {onDeleteRow && (
              <td className="results-grid__actions" key="__actions">
                <button
                  className="btn btn--sm btn--danger results-grid-delete"
                  onClick={() => onDeleteRow(i, row)}
                  title="Delete this document"
                >
                  Delete
                </button>
              </td>
            )}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function TreeNode({ value, path = "" }: { value: unknown; path?: string }) {
  const [expanded, setExpanded] = useState(true);
  const kind = detectKind(value);
  const complex = kind === "object" || kind === "array";

  if (!complex) {
    return (
      <div className="results-tree__leaf">
        {path && <span className="results-tree__key">{path}:</span>}
        <span className={`kind-badge ${kindClassName(kind)}`}>{kind}</span>
        <span className="results-tree__value" title={displayValue(value)}>
          {displayValue(value)}
        </span>
      </div>
    );
  }

  const children = kind === "array" ? (value as unknown[]) : Object.entries(value as Record<string, unknown>);
  const isEmpty = kind === "array" ? children.length === 0 : Object.keys(value as Record<string, unknown>).length === 0;

  return (
    <div className="results-tree__branch">
      <button
        className="results-tree__toggle"
        onClick={() => setExpanded((e) => !e)}
        aria-expanded={expanded}
        disabled={isEmpty}
      >
        <span className="results-tree__chevron" aria-hidden="true">
          {isEmpty ? "·" : expanded ? "▼" : "▶"}
        </span>
        {path && <span className="results-tree__key">{path}:</span>}
        <span className={`kind-badge ${kindClassName(kind)}`}>{kind}</span>
        <span className="results-tree__meta">
          {kind === "array" ? `${children.length} items` : `${Object.keys(value as Record<string, unknown>).length} fields`}
        </span>
      </button>
      {expanded && !isEmpty && (
        <div className="results-tree__children">
          {kind === "array"
            ? (value as unknown[]).map((v, i) => (
                <TreeNode key={i} value={v} path={`[${i}]`} />
              ))
            : Object.entries(value as Record<string, unknown>).map(([k, v]) => (
                <TreeNode key={k} value={v} path={k} />
              ))}
        </div>
      )}
    </div>
  );
}

/* Re-export helpers so existing callers can keep importing from ResultsTable. */
export { detectKind, displayValue, getByPath };

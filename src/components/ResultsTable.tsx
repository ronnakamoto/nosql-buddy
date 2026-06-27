import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { EditableCell } from "./EditableCell";
import { detectKind, displayValue, getByPath, kindClassName } from "./resultsDisplay";

export type ResultsViewMode = "table" | "tree" | "json";

// ─── Virtualization ───────────────────────────────────────────────────
// The results grid can hold thousands of rows once paging is in use, and
// rendering them all to the DOM freezes the UI. We window the `<tbody>`:
// only the rows in the current scroll viewport (plus an overscan) are
// mounted, with spacer rows above and below so the scrollbar reflects the
// true row count.
//
// Rows are single-line (`white-space: nowrap` in `.results-grid td`), so a
// fixed row height is accurate. The window is computed against the nearest
// scrollable ancestor (`.pane__body` in the query tab), so no shared layout
// needs restructuring. Small result sets bypass windowing entirely.

/** Estimated rendered row height in px. Matches the mono `font-size-xs` cell
 *  with `--space-2` vertical padding. Used for spacer math; minor error is
 *  absorbed by `OVERSCAN`. */
const ROW_HEIGHT = 34;
/** Extra rows rendered above/below the viewport to hide spacer-height error
 *  and keep scrolling smooth without reflow flashes. */
const OVERSCAN = 6;
/** Render all rows without windowing below this count — small tables (the
 *  common aggregation/shell case) skip the observer overhead entirely. */
const VIRT_THRESHOLD = 80;

/** Walk up the DOM to the first ancestor that actually scrolls (overflow-y
 *  auto/scroll with scrollable content). Falls back to the document
 *  scroller. Used to attach the scroll listener without knowing the host
 *  layout. */
function getScrollParent(el: HTMLElement | null): HTMLElement | null {
  let node = el?.parentElement ?? null;
  while (node && node !== document.body) {
    const oy = getComputedStyle(node).overflowY;
    if (oy === "auto" || oy === "scroll") {
      return node;
    }
    node = node.parentElement;
  }
  return null;
}

interface VirtualWindow {
  startIndex: number;
  endIndex: number;
  padTop: number;
  padBottom: number;
  virtualizing: boolean;
}

/** Compute the visible row window for `count` rows against the scroll parent
 *  of the returned `wrapRef` element. Re-evaluates on scroll (rAF-throttled),
 *  resize, and when `count`/`enabled` change. When `count` is small or
 *  `enabled` is false, returns the full range with no spacers. */
function useVirtualWindow(count: number, enabled: boolean) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [win, setWin] = useState<VirtualWindow>({
    startIndex: 0,
    endIndex: Math.min(count, VIRT_THRESHOLD),
    padTop: 0,
    padBottom: 0,
    virtualizing: false,
  });

  useEffect(() => {
    const wrap = wrapRef.current;
    if (!enabled || count <= VIRT_THRESHOLD || !wrap) {
      setWin({ startIndex: 0, endIndex: count, padTop: 0, padBottom: 0, virtualizing: false });
      return;
    }
    const scroller = getScrollParent(wrap) ?? (document.scrollingElement as HTMLElement | null);
    if (!scroller) return;

    let raf = 0;
    const recompute = () => {
      raf = 0;
      const wrapRect = wrap.getBoundingClientRect();
      const scrollerRect = scroller.getBoundingClientRect();
      // Top of the table in scroller *content* coordinates.
      const tableTop = wrapRect.top - scrollerRect.top + scroller.scrollTop;
      const viewportTop = Math.max(0, scroller.scrollTop - tableTop);
      const viewportBottom = scroller.scrollTop + scroller.clientHeight - tableTop;
      let start = Math.floor(viewportTop / ROW_HEIGHT) - OVERSCAN;
      let end = Math.ceil(viewportBottom / ROW_HEIGHT) + OVERSCAN;
      start = Math.max(0, Math.min(start, count));
      end = Math.max(start, Math.min(end, count));
      setWin({
        startIndex: start,
        endIndex: end,
        padTop: start * ROW_HEIGHT,
        padBottom: (count - end) * ROW_HEIGHT,
        virtualizing: true,
      });
    };
    const schedule = () => {
      if (raf) return;
      raf = requestAnimationFrame(recompute);
    };
    recompute();
    scroller.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    // Observe the wrap itself so layout shifts (toolbar show/hide, error
    // banner, pane resize) recompute the window, not just scroll/resize.
    const ro = new ResizeObserver(schedule);
    ro.observe(wrap);
    return () => {
      if (raf) cancelAnimationFrame(raf);
      scroller.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      ro.disconnect();
    };
  }, [count, enabled]);

  return { wrapRef, ...win };
}

/** Discover columns from the full document set (not the visible window) so
 *  column headers stay stable as the user scrolls. Scans a bounded prefix to
 *  bound the cost on wide collections and caps the column count for layout. */
function discoverColumns(documents: Array<Record<string, unknown>>): string[] {
  const seen = new Set<string>();
  const order: string[] = [];
  const scanCap = Math.min(documents.length, 60);
  for (let i = 0; i < scanCap; i += 1) {
    for (const k of Object.keys(documents[i])) {
      if (!seen.has(k)) {
        seen.add(k);
        order.push(k);
      }
    }
    if (order.length >= 24) break;
  }
  return order;
}

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
  /** When true, render a checkbox column for selecting rows. Default false. */
  selectable?: boolean;
  /** The set of currently selected row ids (from `getRowId`). */
  selectedRowIds?: Set<string>;
  /** Called with the next selection set when the user toggles rows. */
  onSelectionChange?: (next: Set<string>) => void;
  /** Stable identity for a row; defaults to the row index as a string. */
  getRowId?: (row: Record<string, unknown>, index: number) => string;
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
  selectable = false,
  selectedRowIds,
  onSelectionChange,
  getRowId,
}: ResultsTableProps) {
  // Tree/JSON views keep a capped render slice (variable-height blocks,
  // not the perf-critical path); the table view is virtualized below.
  const cappedRows = useMemo(() => documents.slice(0, pageSize), [documents, pageSize]);
  const columns = useMemo(() => discoverColumns(documents), [documents]);
  // Window only the table view; tree/json use the capped slice above.
  const { wrapRef, startIndex, endIndex, padTop, padBottom, virtualizing } =
    useVirtualWindow(documents.length, view === "table");
  const tableRows = useMemo(
    () => documents.slice(startIndex, endIndex),
    [documents, startIndex, endIndex],
  );

  const rowId = useCallback(
    (row: Record<string, unknown>, index: number) =>
      getRowId ? getRowId(row, index) : String(index),
    [getRowId],
  );

  const selectionActive = selectable && !!selectedRowIds && !!onSelectionChange;

  const toggleRow = useCallback(
    (row: Record<string, unknown>, index: number) => {
      if (!selectedRowIds || !onSelectionChange) return;
      const id = rowId(row, index);
      const next = new Set(selectedRowIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      onSelectionChange(next);
    },
    [selectedRowIds, onSelectionChange, rowId],
  );

  // "Select all" operates on the visible window only; the parent's
  // `selectedDocuments` derives from `page.documents` by `_id`, so a
  // windowed toggle is consistent with selection-by-identity.
  const allSelected =
    selectionActive && tableRows.length > 0
      ? tableRows.every((row, i) => selectedRowIds!.has(rowId(row, startIndex + i)))
      : false;

  const toggleAll = useCallback(() => {
    if (!selectedRowIds || !onSelectionChange) return;
    const next = new Set(selectedRowIds);
    if (tableRows.every((row, i) => next.has(rowId(row, startIndex + i)))) {
      tableRows.forEach((row, i) => next.delete(rowId(row, startIndex + i)));
    } else {
      tableRows.forEach((row, i) => next.add(rowId(row, startIndex + i)));
    }
    onSelectionChange(next);
  }, [tableRows, startIndex, selectedRowIds, onSelectionChange, rowId]);

  if (documents.length === 0) {
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
        {JSON.stringify(cappedRows, null, 2)}
      </pre>
    );
  }

  if (view === "tree") {
    return (
      <div className="results-tree" aria-label="Tree results">
        {cappedRows.map((row, i) => (
          <div key={i} className="results-tree__doc">
            <div className="results-tree__doc-header">
              {selectionActive && (
                <input
                  type="checkbox"
                  aria-label={`Select doc ${i}`}
                  checked={selectedRowIds!.has(rowId(row, i))}
                  onChange={() => toggleRow(row, i)}
                />
              )}
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
    <div ref={wrapRef}>
      <TableView
        rows={tableRows}
        startIndex={startIndex}
        columns={columns}
        padTop={virtualizing ? padTop : 0}
        padBottom={virtualizing ? padBottom : 0}
        connectionId={connectionId}
        database={database}
        collection={collection}
        editable={editable}
        onCellSaved={onCellSaved}
        onCellError={onCellError}
        onDeleteRow={onDeleteRow}
        selectionActive={selectionActive}
        selectedRowIds={selectedRowIds}
        rowId={rowId}
        toggleRow={toggleRow}
        toggleAll={toggleAll}
        allSelected={allSelected}
      />
    </div>
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
  selectionActive,
  selectedRowIds,
  rowId,
  toggleRow,
  toggleAll,
  allSelected,
  startIndex,
  columns,
  padTop,
  padBottom,
}: {
  rows: Array<Record<string, unknown>>;
  connectionId: string;
  database: string;
  collection: string;
  editable: boolean;
  onCellSaved?: (rowIdx: number, fieldPath: string, newValue: unknown) => void;
  onCellError?: (rowIdx: number, fieldPath: string, message: string) => void;
  onDeleteRow?: (rowIdx: number, doc: Record<string, unknown>) => void;
  selectionActive: boolean;
  selectedRowIds?: Set<string>;
  rowId: (row: Record<string, unknown>, index: number) => string;
  toggleRow: (row: Record<string, unknown>, index: number) => void;
  toggleAll: () => void;
  allSelected: boolean;
  /** Absolute index of `rows[0]` within the full document array. All
   *  index-based callbacks (`onCellSaved`, `onDeleteRow`, `rowId`) receive
   *  `startIndex + i` so edits/deletes target the right document regardless
   *  of the current scroll window. */
  startIndex: number;
  columns: string[];
  /** Spacer heights (px) for virtualization; 0 when not virtualizing. */
  padTop: number;
  padBottom: number;
}) {
  const isNumeric = (kind: string): boolean =>
    ["int", "long", "double", "decimal"].includes(kind);

  // colspan for the spacer rows = columns + optional selection/actions
  // columns, so a single empty <td> can span the full grid width without
  // perturbing the auto column widths.
  const spacerSpan = columns.length + (selectionActive ? 1 : 0) + (onDeleteRow ? 1 : 0);

  return (
    <table className="results-grid" role="grid" aria-label="Query results">
      <thead>
        <tr>
          {selectionActive && (
            <th className="results-grid__select" scope="col">
              <input
                type="checkbox"
                aria-label="Select all rows"
                checked={allSelected}
                onChange={toggleAll}
              />
            </th>
          )}
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
        {padTop > 0 && (
          <tr aria-hidden="true" style={{ height: padTop }}>
            <td colSpan={spacerSpan} style={{ padding: 0, border: 0, height: padTop }} />
          </tr>
        )}
        {rows.map((row, i) => {
          // Absolute index into the full document array — what the parent's
          // edit/delete/selection handlers expect.
          const abs = startIndex + i;
          return (
          <tr key={abs}>
            {selectionActive && (
              <td className="results-grid__select">
                <input
                  type="checkbox"
                  aria-label={`Select row ${abs}`}
                  checked={selectedRowIds!.has(rowId(row, abs))}
                  onChange={() => toggleRow(row, abs)}
                />
              </td>
            )}
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
                      onSaved={(newValue) => onCellSaved(abs, c, newValue)}
                      onError={(message) => onCellError(abs, c, message)}
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
                  onClick={() => onDeleteRow(abs, row)}
                  title="Delete this document"
                >
                  Delete
                </button>
              </td>
            )}
          </tr>
          );
        })}
        {padBottom > 0 && (
          <tr aria-hidden="true" style={{ height: padBottom }}>
            <td colSpan={spacerSpan} style={{ padding: 0, border: 0, height: padBottom }} />
          </tr>
        )}
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

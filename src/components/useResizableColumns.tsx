import { useCallback, useLayoutEffect, useRef, useState } from "react";

/**
 * Shared column-resize behavior for `.results-grid` tables (query results,
 * field mapping, import preview, ...). Renders each column at its natural
 * (content-driven) width on first paint, then lets the user drag a handle on
 * the trailing edge of any header cell to set an explicit pixel width.
 *
 * Usage: call the hook with the ordered column keys, spread
 * `headerCellProps(col)` onto each resizable `<th>`, render
 * `<ColumnResizeHandle .../>` inside it, and apply `columnStyle(col)` /
 * `tableStyle` to the `<table>`. Columns not passed in (e.g. a leading
 * checkbox column or trailing actions column) keep their normal CSS width.
 */

const MIN_WIDTH = 60;
const MAX_WIDTH = 720;
const DEFAULT_WIDTH = 160;
const KEYBOARD_STEP = 16;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function useResizableColumns(columns: string[]) {
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [ready, setReady] = useState(false);
  const widthsRef = useRef(widths);
  widthsRef.current = widths;
  const headerRefs = useRef(new Map<string, HTMLTableCellElement>());

  // Measure natural (auto-layout) widths once on first mount so switching to
  // fixed-width columns doesn't jump the layout. Columns discovered later
  // (e.g. a wider scroll window turns up a new field) fall back to a
  // sensible default instead of re-measuring, which would reset any widths
  // the user already dragged.
  useLayoutEffect(() => {
    if (ready) return;
    if (columns.length === 0) return;
    const next: Record<string, number> = {};
    for (const col of columns) {
      const el = headerRefs.current.get(col);
      next[col] = clamp(
        el ? Math.round(el.getBoundingClientRect().width) : DEFAULT_WIDTH,
        MIN_WIDTH,
        MAX_WIDTH,
      );
    }
    setWidths(next);
    setReady(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [columns.length, ready]);

  const registerHeaderRef = useCallback(
    (col: string) => (el: HTMLTableCellElement | null) => {
      if (el) headerRefs.current.set(col, el);
      else headerRefs.current.delete(col);
    },
    [],
  );

  const setWidth = useCallback((col: string, width: number) => {
    setWidths((prev) => ({ ...prev, [col]: clamp(width, MIN_WIDTH, MAX_WIDTH) }));
  }, []);

  const startResize = useCallback(
    (col: string) => (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      e.stopPropagation();
      const handle = e.currentTarget;
      const startX = e.clientX;
      const startWidth = widthsRef.current[col] ?? DEFAULT_WIDTH;
      handle.setPointerCapture(e.pointerId);
      handle.classList.add("col-resize-handle--active");
      document.body.classList.add("col-resize-active");

      const onMove = (ev: PointerEvent) => {
        setWidth(col, startWidth + (ev.clientX - startX));
      };
      const onUp = (ev: PointerEvent) => {
        handle.releasePointerCapture(ev.pointerId);
        handle.classList.remove("col-resize-handle--active");
        document.body.classList.remove("col-resize-active");
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [setWidth],
  );

  const onKeyDown = useCallback(
    (col: string) => (e: React.KeyboardEvent<HTMLDivElement>) => {
      const current = widthsRef.current[col] ?? DEFAULT_WIDTH;
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        setWidth(col, current - KEYBOARD_STEP);
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        setWidth(col, current + KEYBOARD_STEP);
      }
    },
    [setWidth],
  );

  const columnStyle = useCallback(
    (col: string): React.CSSProperties | undefined =>
      ready ? { width: widths[col], minWidth: widths[col], maxWidth: widths[col] } : undefined,
    [ready, widths],
  );

  const headerCellProps = useCallback(
    (col: string) => ({
      ref: registerHeaderRef(col),
      style: columnStyle(col),
    }),
    [registerHeaderRef, columnStyle],
  );

  return {
    ready,
    widths,
    columnStyle,
    headerCellProps,
    startResize,
    onKeyDown,
  };
}

export interface ColumnResizeHandleProps {
  column: string;
  label: string;
  width: number | undefined;
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => void;
}

/** Grabbable separator rendered on the trailing edge of a resizable header
 *  cell. Draggable with the pointer, adjustable with the arrow keys when
 *  focused. */
export function ColumnResizeHandle({
  column,
  label,
  width,
  onPointerDown,
  onKeyDown,
}: ColumnResizeHandleProps) {
  return (
    <div
      className="col-resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize ${label} column`}
      aria-valuenow={width}
      aria-valuemin={MIN_WIDTH}
      aria-valuemax={MAX_WIDTH}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
      data-column={column}
    />
  );
}

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { Check, ChevronDown, Copy, Search, X } from "lucide-react";

/**
 * LogViewer — the one place the audit surface renders streamed or static
 * log/terminal output. Used for the setup-wizard progress feed (live,
 * narrow), the completed-setup transcript (static, copyable), and the
 * dev-stack "View Logs" modal (static, searchable). Consolidating these
 * gives every log-like surface the same severity coloring, copy affordance,
 * and auto-follow behavior instead of three slightly different `<pre>`s.
 */

export type LogLevel = "error" | "warn" | "success" | "default";

export interface LogViewerStats {
  /** Total number of lines, independent of any active filter. */
  total: number;
  /** Number of lines currently visible (post-filter). */
  visible: number;
  errors: number;
  warnings: number;
}

/** Classifies a line for color-coding. Errors and warnings win over success. */
export function classifyLogLine(line: string): LogLevel {
  const l = line.toLowerCase();
  if (/\b(error|err|fatal|panic|traceback|exception|failed|failure)\b/.test(l)) return "error";
  if (/\b(warn|warning|deprecat)\b/.test(l)) return "warn";
  if (/\b(ok|success|succeeded|complete|completed|confirmed|deployed)\b/.test(l)) return "success";
  return "default";
}

const EMPTY_STATS: LogViewerStats = { total: 0, visible: 0, errors: 0, warnings: 0 };

export function LogViewer({
  lines,
  loading = false,
  loadingLabel = "Loading…",
  emptyLabel = "No logs available.",
  live = false,
  searchable = false,
  copyable = false,
  showLineNumbers = true,
  colorize = true,
  minHeight = 120,
  maxHeight = 320,
  onStats,
  className,
  style,
}: {
  /** Raw log text, or lines already split. */
  lines: string[] | string;
  loading?: boolean;
  loadingLabel?: string;
  /** Shown when there are no lines at all (not the "no matches" filter case). */
  emptyLabel?: string;
  /** Pins the viewport to the newest line and shows a live indicator + "Jump to latest" affordance when scrolled away. */
  live?: boolean;
  /** Shows an inline filter bar above the viewport. */
  searchable?: boolean;
  /** Shows a copy-to-clipboard control. */
  copyable?: boolean;
  showLineNumbers?: boolean;
  /** Color-codes lines by detected severity (error / warning / success). */
  colorize?: boolean;
  minHeight?: number | string;
  maxHeight?: number | string;
  onStats?: (stats: LogViewerStats) => void;
  className?: string;
  style?: CSSProperties;
}) {
  const [query, setQuery] = useState("");
  const [copied, setCopied] = useState(false);
  const [pinned, setPinned] = useState(true);
  const [overflowing, setOverflowing] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);

  const rawLines = useMemo(
    () => (Array.isArray(lines) ? lines : lines ? lines.split("\n") : []),
    [lines],
  );
  const indexed = useMemo(() => rawLines.map((line, index) => ({ line, index })), [rawLines]);
  const filtered = useMemo(() => {
    if (!searchable || !query.trim()) return indexed;
    const q = query.toLowerCase();
    return indexed.filter((entry) => entry.line.toLowerCase().includes(q));
  }, [indexed, query, searchable]);

  const stats = useMemo<LogViewerStats>(() => {
    if (!colorize) return { ...EMPTY_STATS, total: rawLines.length, visible: filtered.length };
    let errors = 0;
    let warnings = 0;
    for (const line of rawLines) {
      const level = classifyLogLine(line);
      if (level === "error") errors++;
      else if (level === "warn") warnings++;
    }
    return { total: rawLines.length, visible: filtered.length, errors, warnings };
  }, [rawLines, filtered.length, colorize]);

  useEffect(() => {
    onStats?.(stats);
    // Reporting stats up shouldn't re-run just because the parent passed a
    // fresh callback identity; only react to the stats actually changing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stats.total, stats.visible, stats.errors, stats.warnings]);

  // Follow the tail while pinned. Scrolling away un-pins; "Jump to latest"
  // (or scrolling back to the bottom) re-pins.
  useEffect(() => {
    if (!live || !pinned) return;
    const el = viewportRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [live, pinned, filtered]);

  const handleScroll = useCallback(() => {
    const el = viewportRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setPinned(distanceFromBottom < 32);
  }, []);

  const jumpToLatest = useCallback(() => {
    const el = viewportRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    setPinned(true);
  }, []);

  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const check = () => setOverflowing(el.scrollHeight > el.clientHeight + 1);
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [filtered.length]);

  const handleCopy = useCallback(() => {
    const text = filtered.map((entry) => entry.line).join("\n");
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [filtered]);

  const showToolbar = searchable || copyable;
  const isEmpty = !loading && rawLines.length === 0;
  const noMatches = !loading && !isEmpty && filtered.length === 0;

  return (
    <div className={className ? `audit-logviewer ${className}` : "audit-logviewer"} style={style}>
      {showToolbar && (
        <div className="audit-logviewer__toolbar">
          {searchable ? (
            <>
              <Search size={14} className="audit-logviewer__search-icon" aria-hidden="true" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter logs…"
                className="audit-logviewer__search-input"
              />
              {query && (
                <button
                  className="audit-logviewer__clear"
                  onClick={() => setQuery("")}
                  aria-label="Clear filter"
                >
                  <X size={12} />
                </button>
              )}
            </>
          ) : (
            <span className="audit-logviewer__count">
              {rawLines.length} line{rawLines.length === 1 ? "" : "s"}
            </span>
          )}
          {live && !loading && rawLines.length > 0 && (
            <span className="audit-logviewer__live">
              <span className="audit-status-dot audit-status-dot--live" />
              Live
            </span>
          )}
          {copyable && (
            <button
              className={copied ? "audit-logviewer__copy audit-logviewer__copy--done" : "audit-logviewer__copy"}
              onClick={handleCopy}
              disabled={rawLines.length === 0}
              title="Copy log output"
            >
              {copied ? <Check size={12} /> : <Copy size={12} />}
              {copied ? "Copied" : "Copy"}
            </button>
          )}
        </div>
      )}

      <div className="audit-logviewer__frame">
        <div
          ref={viewportRef}
          onScroll={handleScroll}
          className={
            overflowing ? "audit-logviewer__viewport audit-logviewer__viewport--fade" : "audit-logviewer__viewport"
          }
          style={{ minHeight, maxHeight }}
        >
          {loading ? (
            <div className="audit-logviewer__status">
              <span className="audit-spinner" style={{ width: 16, height: 16 }} />
              {loadingLabel}
            </div>
          ) : isEmpty ? (
            <div className="audit-logviewer__status">{emptyLabel}</div>
          ) : noMatches ? (
            <div className="audit-logviewer__status">No lines match your filter.</div>
          ) : (
            filtered.map(({ line, index }, i) => {
              const level = colorize ? classifyLogLine(line) : "default";
              const isCurrent = live && !query && i === filtered.length - 1;
              return (
                <div
                  key={index}
                  className={`audit-logviewer__line audit-logviewer__line--${level}${
                    isCurrent ? " audit-logviewer__line--current" : ""
                  }${showLineNumbers ? "" : " audit-logviewer__line--no-numbers"}`}
                >
                  {showLineNumbers && <span className="audit-logviewer__num">{index + 1}</span>}
                  <span className="audit-logviewer__text">
                    {line || " "}
                    {isCurrent && <span className="audit-logviewer__caret" aria-hidden="true" />}
                  </span>
                </div>
              );
            })
          )}
        </div>
        {live && !pinned && rawLines.length > 0 && (
          <button className="audit-logviewer__jump" onClick={jumpToLatest}>
            <ChevronDown size={12} />
            Jump to latest
          </button>
        )}
      </div>
    </div>
  );
}

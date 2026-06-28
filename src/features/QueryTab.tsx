import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import commands, { type DocumentPage, type SqlLanguage } from "../ipc/commands";
import { ResultsTable, type ResultsViewMode } from "../components/ResultsTable";
import { toFilterId } from "../components/resultsDisplay";
import { InsertDocumentModal } from "../components/InsertDocumentModal";
import { QueryHistoryPanel } from "../components/QueryHistoryPanel";
import { AggregationEditor } from "./AggregationEditor";
import { VisualQueryBuilder } from "./VisualQueryBuilder";
import { pushHistory, type QueryMode, fileExtension, fileFilter } from "./queryHistory";
import { ExportWizard } from "./importExport/ExportWizard";
import { ImportWizard } from "./importExport/ImportWizard";
import type { ExportSourceDto } from "../ipc/commands";
import { save, open } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import Prism from "prismjs";
import "prismjs/components/prism-javascript";
import "prismjs/components/prism-json";
import "prismjs/components/prism-sql";
import "prismjs/components/prism-python";
import "prismjs/components/prism-java";
import "prismjs/components/prism-csharp";
import "prismjs/components/prism-ruby";
import "prismjs/components/prism-bash";
import { CodeEditor } from "../components/CodeEditor";
import { useCollectionSchema } from "../hooks/useCollectionSchema";
import { InfoPopover } from "../components/InfoPopover";
import { Alert } from "../components/Alert";
import { useToast } from "../context/ToastContext";
import { Minus, Maximize2, X } from "lucide-react";

/** Prism grammar name for each driver-code language. Mirrors the map
 *  in DriverCodePanel so the SQL tab's read-only code block shares the
 *  same highlighting vocabulary. */
const PRISM_LANG: Record<SqlLanguage, string> = {
  "node-js": "javascript",
  python: "python",
  java: "java",
  "c-sharp": "csharp",
  ruby: "ruby",
  shell: "bash",
};

function highlightCode(code: string, grammarName: string): string {
  const grammar = Prism.languages[grammarName];
  if (!grammar) return code;
  return Prism.highlight(code, grammar, grammarName);
}

export interface QueryTabProps {
  connectionId: string;
  database: string;
  collection: string;
  /** Profile metadata for the active connection. Used by the
   *  driver-code panel inside the AggregationEditor to embed the
   *  user's real Mongo URI. Optional because legacy code paths
   *  may not have it on hand (e.g. unit tests). */
  profile?: { id: string; name: string; authMechanism: string } | null;
  onClose: () => void;
  onResult?: (page: DocumentPage | null) => void;
  /**
   * Called when the user clicks "Open in Aggregation Editor". The
   * pipeline is the translated aggregation pipeline (an array of
   * stage objects). The host (App.tsx) is expected to open a new tab
   * in aggregation mode and pre-populate it with this pipeline.
   */
  onOpenInAggregationEditor?: (pipeline: unknown[]) => void;
  /** Called after a successful import to refresh external views (e.g. sidebar). */
  onImported?: () => void;
}

type Mode = "find" | "aggregate" | "sql" | "update" | "insert";
type ResultsPanelState = "expanded" | "minimized" | "closed";

const SQL_LANGUAGES: SqlLanguage[] = [
  "node-js",
  "python",
  "java",
  "c-sharp",
  "ruby",
  "shell",
];
const SQL_LANGUAGE_LABELS: Record<SqlLanguage, string> = {
  "node-js": "JavaScript (Node.js)",
  python: "Python",
  java: "Java",
  "c-sharp": "C#",
  ruby: "Ruby",
  shell: "mongo shell",
};

const DEFAULT_FILTER = "{}";
const DEFAULT_PIPELINE = "[\n  { \"$match\": {} },\n  { \"$limit\": 50 }\n]";

/**
 * Set `value` at `path` (dotted) on a copy of `target`. Creates
 * intermediate objects when missing. Used to mirror a saved edit
 * into the local results cache so the user sees the new value
 * without a full re-run.
 */
function applyPath(target: Record<string, unknown>, path: string, value: unknown): void {
  const parts = path.split(".");
  if (parts.length === 1) {
    target[path] = value;
    return;
  }
  let cursor: Record<string, unknown> = target;
  for (let i = 0; i < parts.length - 1; i += 1) {
    const key = parts[i];
    const next = cursor[key];
    if (next === null || typeof next !== "object" || Array.isArray(next)) {
      const fresh: Record<string, unknown> = {};
      cursor[key] = fresh;
      cursor = fresh;
    } else {
      const cloned: Record<string, unknown> = { ...(next as Record<string, unknown>) };
      cursor[key] = cloned;
      cursor = cloned;
    }
  }
  cursor[parts[parts.length - 1]] = value;
}

/**
 * Parse a user-typed pipeline JSON string into an array of stages for
 * the AggregationEditor. Returns an empty array on parse failure so
 * the editor falls back to its default stages.
 */
function parsePipeline(text: string): unknown[] {
  const trimmed = text.trim();
  if (!trimmed) return [];
  try {
    const parsed = JSON.parse(trimmed);
    if (Array.isArray(parsed)) return parsed;
    return [];
  } catch {
    return [];
  }
}

/** Status-bar text for the current page. Renders the loaded count against
 *  the (possibly approximate) total so users on large collections see how
 *  much of the result set is on this page. */
function formatResultSummary(page: DocumentPage, loadingPage: boolean): string {
  const loaded = page.documents.length;
  const ms = page.executionMs ?? 0;
  const tail = loadingPage ? " · loading…" : "";
  const total = page.totalCount;
  if (total == null) return `${loaded} shown · ${ms} ms${tail}`;
  const approx = page.totalCountApprox ? "≈" : "";
  return `${loaded} of ${approx}${total.toLocaleString()} shown · ${ms} ms${tail}`;
}

const PAGE_SIZE_OPTIONS = [25, 50, 100, 200, 500];

/** Footer below the results grid that drives page-based navigation.
 *
 * First/Prev/Page N/Next/Last controls using skip/limit paging. The page-size
 * selector bounds memory per page (default 50, max 1000); changing it
 * triggers a fresh page-1 fetch so the new size takes effect immediately.
 */
function PagingBar({
  total,
  totalApprox,
  hasMore,
  loadingPage,
  pageSize,
  currentPage,
  onPageSize,
  onJump,
}: {
  total: number | null;
  totalApprox: boolean;
  hasMore: boolean;
  loadingPage: boolean;
  pageSize: number;
  currentPage: number;
  onPageSize: (n: number) => void;
  onJump: (n: number) => void;
}) {
  const totalPages = total != null ? Math.max(1, Math.ceil(total / pageSize)) : null;
  const approx = totalApprox ? "≈" : "";
  const atStart = currentPage <= 1;
  const atEnd =
    (totalPages != null && currentPage >= totalPages) || !hasMore;

  return (
    <div className="paging-bar" role="navigation" aria-label="Result paging">
      <label className="paging-bar__size" title="Rows per page (re-runs from page 1)">
        <span>Page size <InfoPopover label="Page size help" title="Page size"><p>Number of documents to load per page. Larger sizes load more data at once but use more memory. Changing this re-runs the query from page 1.</p></InfoPopover></span>
        <select
          value={pageSize}
          onChange={(e) => onPageSize(Number(e.target.value))}
          disabled={loadingPage}
        >
          {PAGE_SIZE_OPTIONS.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>

      <div className="paging-bar__spacer" />

      <div className="paging-bar__pages">
        {total != null && (
          <span className="paging-bar__count">
            {approx}{total.toLocaleString()} total
          </span>
        )}
        <button
          className="btn btn--sm"
          onClick={() => onJump(1)}
          disabled={atStart || loadingPage}
          title="First page"
        >
          «
        </button>
        <button
          className="btn btn--sm"
          onClick={() => onJump(currentPage - 1)}
          disabled={atStart || loadingPage}
          title="Previous page"
        >
          ‹
        </button>
        <span className="paging-bar__page">
          {totalPages != null
            ? `Page ${currentPage} of ${totalPages}`
            : `Page ${currentPage}`}
        </span>
        <button
          className="btn btn--sm"
          onClick={() => onJump(currentPage + 1)}
          disabled={atEnd || loadingPage}
          title="Next page"
        >
          ›
        </button>
        <button
          className="btn btn--sm"
          onClick={() => onJump(totalPages ?? currentPage + 1)}
          disabled={atEnd || loadingPage}
          title="Last page"
        >
          »
        </button>
      </div>
    </div>
  );
}

/** A compact dropdown menu for secondary pane actions. The dropdown is
 *  rendered with `position: fixed` so it can never be clipped by an
 *  overflow container. */
function PaneActionsMenu({
  items,
}: {
  items: {
    id: string;
    label: string;
    hint?: string;
    onClick: () => void;
    disabled?: boolean;
  }[];
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number | null; right: number | null }>({ top: 0, left: 0, right: null });
  const menuRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const updatePosition = useCallback(() => {
    const btn = triggerRef.current;
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    const dropdownWidth = 160;
    const gap = 4;
    const top = rect.bottom + gap;
    if (rect.left + dropdownWidth > window.innerWidth - 8) {
      setPos({ top, left: null, right: window.innerWidth - rect.right });
    } else {
      setPos({ top, left: rect.left, right: null });
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    updatePosition();
    const onClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onResize = () => updatePosition();
    document.addEventListener("mousedown", onClick);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onClick);
      window.removeEventListener("resize", onResize);
    };
  }, [open, updatePosition]);

  return (
    <div className="pane-actions-menu" ref={menuRef}>
      <button
        className="btn btn--sm pane-actions-menu__trigger"
        ref={triggerRef}
        onClick={() => {
          if (!open) updatePosition();
          setOpen((o) => !o);
        }}
        aria-label="More actions"
        aria-expanded={open}
        title="More actions"
      >
        More
      </button>
      {open && (
        <div
          className="pane-actions-menu__dropdown"
          role="menu"
          style={{
            position: "fixed",
            top: pos.top,
            left: pos.left ?? undefined,
            right: pos.right ?? undefined,
          }}
        >
          {items.map((item) => (
            <button
              key={item.id}
              className="pane-actions-menu__item"
              role="menuitem"
              disabled={item.disabled}
              onClick={() => {
                setOpen(false);
                item.onClick();
              }}
            >
              <span className="pane-actions-menu__label">{item.label}</span>
              {item.hint && <span className="pane-actions-menu__hint">{item.hint}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function QueryTab({
  connectionId,
  database,
  collection,
  profile,
  onClose,
  onResult,
  onOpenInAggregationEditor,
  onImported,
}: QueryTabProps) {
  const [mode, setMode] = useState<Mode>("find");
  const [filterText, setFilterText] = useState(DEFAULT_FILTER);
  const [filterEditor, setFilterEditor] = useState<"json" | "visual">("json");
  const [projectionText, setProjectionText] = useState("");
  const [sortText, setSortText] = useState("");
  const [pipelineText, setPipelineText] = useState(DEFAULT_PIPELINE);
  const [pipelineKey, setPipelineKey] = useState(0);
  const [sqlText, setSqlText] = useState(
    `SELECT * FROM ${collection} ORDER BY _id LIMIT 50`,
  );
  const [sqlResult, setSqlResult] = useState<{
    pipeline: unknown[];
    find: Record<string, unknown> | null;
    warnings: string[];
    code: Record<string, string>;
    operation?: import("../ipc/commands").SqlOperation;
  } | null>(null);
  const [sqlLanguage, setSqlLanguage] = useState<SqlLanguage>("node-js");
  const [sqlNotice, setSqlNotice] = useState<string | null>(null);
  const [copyNotice, setCopyNotice] = useState<{ text: string; tone: "success" | "warning" } | null>(null);
  const toast = useToast();
  const [page, setPage] = useState<DocumentPage | null>(null);
  const [running, setRunning] = useState(false);
  const [insertOpen, setInsertOpen] = useState(false);

  // ─── Update mode state ──────────────────────────────────────────────
  const [updateFilterText, setUpdateFilterText] = useState("{}");
  const [updateText, setUpdateText] = useState('{}');
  const [updateMulti, setUpdateMulti] = useState(true);
  const [updateUpsert, setUpdateUpsert] = useState(false);
  const [updatePreviewCount, setUpdatePreviewCount] = useState<number | null>(null);
  const [updatePreviewLoading, setUpdatePreviewLoading] = useState(false);

  // ─── Insert mode state ──────────────────────────────────────────────
  const [insertBody, setInsertBody] = useState('');
  const [insertMany, setInsertMany] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [exportSource, setExportSource] = useState<ExportSourceDto | null>(null);
  const [pendingDelete, setPendingDelete] = useState<{ idx: number; docId: unknown } | null>(null);
  const [viewMode, setViewMode] = useState<ResultsViewMode>("table");
  const [resultsPanelState, setResultsPanelState] = useState<ResultsPanelState>("expanded");
  const [selectedRowIds, setSelectedRowIds] = useState<Set<string>>(() => new Set());

  // ─── Schema for autocomplete ────────────────────────────────────────
  const schema = useCollectionSchema(connectionId, database, collection);

  // ─── Paging state (find mode) ───────────────────────────────────────
  // `page: DocumentPage` remains the single source of truth for the visible
  // rows: keyset "load more" *appends* to `page.documents`, while skip/limit
  // page-jumping *replaces* it. This keeps the existing row-index-based edit,
  // delete, and selection flows working without change. `nextAfterId` on the
  // page is the keyset cursor; `hasMore` is the "more likely exist" signal.
  const [pageSize, setPageSize] = useState(50);
  // Mirror `pageSize` into a ref so a page-size change can re-run immediately
  // without waiting a tick for state to propagate (the new fetch must use the
  // new size, not the value closed over by `run`).
  const pageSizeRef = useRef(pageSize);
  pageSizeRef.current = pageSize;
  const [loadingMore, setLoadingMore] = useState(false);
  const [currentPage, setCurrentPage] = useState(1);
  /** Gates history capture so page jumps don't log a new entry per jump —
   *  only a fresh Run does. Set true at the start of `run()`, set false
   *  before any page jump that is not a fresh Run. */
  const captureHistoryRef = useRef(true);

  const valid = useMemo(() => {
    try {
      if (mode === "find") {
        if (filterText.trim()) JSON.parse(filterText);
        if (projectionText.trim()) JSON.parse(projectionText);
        if (sortText.trim()) JSON.parse(sortText);
      } else if (mode === "aggregate") {
        if (pipelineText.trim()) JSON.parse(pipelineText);
      } else if (mode === "update") {
        if (updateFilterText.trim()) JSON.parse(updateFilterText);
        if (updateText.trim()) JSON.parse(updateText);
      } else if (mode === "insert") {
        if (insertBody.trim()) {
          const parsed = JSON.parse(insertBody);
          if (insertMany && !Array.isArray(parsed)) return false;
          if (!insertMany && (typeof parsed !== "object" || parsed === null || Array.isArray(parsed))) return false;
        }
      }
      return true;
    } catch {
      return false;
    }
  }, [mode, filterText, projectionText, sortText, pipelineText, updateFilterText, updateText, insertBody, insertMany]);

  const sqlPipelineHtml = useMemo(() => {
    if (!sqlResult) return "";
    return highlightCode(JSON.stringify(sqlResult.pipeline, null, 2), "json");
  }, [sqlResult]);

  const sqlCodeHtml = useMemo(() => {
    if (!sqlResult) return "";
    const code = sqlResult.code[sqlLanguage] ?? "";
    return highlightCode(code, PRISM_LANG[sqlLanguage]);
  }, [sqlResult, sqlLanguage]);

  async function run() {
    if (!valid) {
      toast.push("Fix the JSON syntax first.", "error");
      return;
    }
    setRunning(true);
    captureHistoryRef.current = true;
    setCurrentPage(1);
    setSelectedRowIds(new Set());
    try {
      if (mode === "find") {
        const result = await commands.findPage({
          connectionId,
          database,
          collection,
          filterJson: filterText,
          projectionJson: projectionText || null,
          sortJson: sortText || null,
          pageSize: pageSizeRef.current,
          countMode: "estimated",
        });
        setPage(result);
        setResultsPanelState("expanded");
      } else if (mode === "aggregate") {
        const result = await commands.aggregatePage({
          connectionId,
          database,
          collection,
          pipelineJson: pipelineText,
          pageSize: pageSizeRef.current,
          countMode: "none",
        });
        setPage(result);
        setResultsPanelState("expanded");
      } else if (mode === "update") {
        const result = await commands.updateDocuments({
          connectionId,
          database,
          collection,
          filterJson: updateFilterText,
          updateJson: updateText,
          multi: updateMulti,
          upsert: updateUpsert,
        });
        toast.push(
          `Updated ${result.modifiedCount} document(s) (matched ${result.matchedCount})`,
          result.modifiedCount > 0 ? "success" : "warning",
        );
      } else if (mode === "insert") {
        if (insertMany) {
          const result = await commands.insertManyDocuments({
            connectionId,
            database,
            collection,
            documentsJson: insertBody,
          });
          toast.push(`Inserted ${result.insertedCount} document(s).`, "success");
        } else {
          const result = await commands.insertDocument({
            connectionId,
            database,
            collection,
            documentJson: insertBody,
          });
          toast.push(`Inserted document with _id ${result}.`, "success");
        }
      } else {
        const translated = await commands.translateSql(database, sqlText);
        setSqlResult({
          pipeline: translated.pipeline,
          find: translated.find,
          warnings: translated.warnings,
          code: translated.code,
          operation: translated.operation,
        });
        await runSqlTranslation(translated);
      }
    } catch (e) {
      toast.push(describeError(e), "error");
    } finally {
      setRunning(false);
    }
  }

  async function runSqlTranslation(translated: import("../ipc/commands").SqlTranslation) {
    if (!translated.operation) return;
    const op = translated.operation;
    switch (op.kind) {
      case "find":
      case "aggregate": {
        const result = await commands.aggregatePage({
          connectionId,
          database,
          collection,
          pipelineJson: JSON.stringify(translated.pipeline),
          pageSize: pageSizeRef.current,
          countMode: "none",
        });
        setPage(result);
        setResultsPanelState("expanded");
        break;
      }
      case "update": {
        const result = await commands.updateDocuments({
          connectionId,
          database,
          collection,
          filterJson: JSON.stringify(op.filter),
          updateJson: JSON.stringify(op.update),
          multi: op.multi,
          upsert: op.upsert,
        });
        toast.push(
          `Updated ${result.modifiedCount} document(s) (matched ${result.matchedCount})`,
          result.modifiedCount > 0 ? "success" : "warning",
        );
        break;
      }
      case "insert": {
        const result = await commands.insertManyDocuments({
          connectionId,
          database,
          collection,
          documentsJson: JSON.stringify(op.documents),
        });
        toast.push(`Inserted ${result.insertedCount} document(s).`, "success");
        break;
      }
      case "delete": {
        const count = await commands.deleteDocuments(
          connectionId,
          database,
          collection,
          JSON.stringify(op.filter),
        );
        toast.push(`Deleted ${count} document(s).`, "success");
        break;
      }
      case "replace": {
        const result = await commands.replaceDocument({
          connectionId,
          database,
          collection,
          filterJson: JSON.stringify(op.filter),
          replacementJson: JSON.stringify(op.replacement),
          upsert: op.upsert,
        });
        toast.push(
          `Replaced ${result.modifiedCount} document(s) (matched ${result.matchedCount})`,
          result.modifiedCount > 0 ? "success" : "warning",
        );
        break;
      }
    }
  }

  /** Skip/limit page jump. Replaces the visible rows with the requested
   *  page. `n` is 1-based. Dispatches by mode: find uses `findPage`
   *  skip/limit; aggregate/SQL use `aggregatePage` (`$skip`/`$limit`). */
  async function jumpToPage(n: number) {
    if (n < 1 || loadingMore) return;
    setLoadingMore(true);
    captureHistoryRef.current = false;
    try {
      let result: DocumentPage;
      if (mode === "find") {
        result = await commands.findPage({
          connectionId,
          database,
          collection,
          filterJson: filterText,
          projectionJson: projectionText || null,
          sortJson: sortText || null,
          page: n,
          pageSize: pageSizeRef.current,
          countMode: "none",
        });
      } else if (mode === "aggregate") {
        result = await commands.aggregatePage({
          connectionId,
          database,
          collection,
          pipelineJson: pipelineText,
          page: n,
          pageSize: pageSizeRef.current,
          countMode: "none",
        });
      } else {
        // SQL: page the already-translated pipeline. If translation is stale
        // or missing, fall back to re-running page 1 via `run()`.
        const pipeline = sqlResult?.pipeline;
        if (!pipeline) {
          void run();
          return;
        }
        result = await commands.aggregatePage({
          connectionId,
          database,
          collection,
          pipelineJson: JSON.stringify(pipeline),
          page: n,
          pageSize: pageSizeRef.current,
          countMode: "none",
        });
      }
      setCurrentPage(n);
      setSelectedRowIds(new Set());
      // Preserve the first page's total across jumps so the footer count
      // stays stable and we don't re-run a count per jump.
      setPage((prev) =>
        prev
          ? {
              ...prev,
              documents: result.documents,
              hasMore: result.hasMore,
              limit: result.limit,
              skip: result.skip,
            }
          : result,
      );
    } catch (e) {
      toast.push(describeError(e), "error");
    } finally {
      setLoadingMore(false);
    }
  }

  function handlePageSize(n: number) {
    if (n === pageSize) return;
    // Update the ref synchronously so the immediate `run()` fetches with the
    // new size rather than the value closed over by `run`.
    pageSizeRef.current = n;
    setPageSize(n);
    void run();
  }

  useEffect(() => {
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const importHandlerRef = useRef(() => setImportOpen(true));
  importHandlerRef.current = () => setImportOpen(true);
  useEffect(() => {
    const fn = () => importHandlerRef.current();
    window.addEventListener("nosqlbuddy:import-data", fn);
    return () => window.removeEventListener("nosqlbuddy:import-data", fn);
  }, []);

  useEffect(() => {
    if (onResult) onResult(page);
  }, [page, onResult]);

  // History capture: a successful *Run* (not a load-more append or page
  // jump) appends to the per-collection, per-mode history list (capped at
  // HISTORY_CAPACITY). `captureHistoryRef` is set true only at the start of
  // `run()` and consumed here, so paging actions that mutate `page` don't
  // spam the history with one entry per batch.
  useEffect(() => {
    if (!page || running) return;
    if (!captureHistoryRef.current) return;
    captureHistoryRef.current = false;
    const inputText = currentTextForMode();
    pushHistory(connectionId, database, collection, mode as QueryMode, {
      ts: Date.now(),
      text: inputText,
      durationMs: page.executionMs,
      docCount: page.documents.length,
      errored: false,
    });
    // Re-run only when the page changes; mode and inputs are
    // captured via the closure of currentTextForMode at run time.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);

  function currentTextForMode(): string {
    if (mode === "find") {
      return JSON.stringify({
        filter: safeParse(filterText, {}),
        projection: safeParse(projectionText, undefined),
        sort: safeParse(sortText, undefined),
      });
    }
    if (mode === "aggregate") return pipelineText;
    if (mode === "update") {
      return JSON.stringify({
        filter: safeParse(updateFilterText, {}),
        update: safeParse(updateText, {}),
        multi: updateMulti,
        upsert: updateUpsert,
      });
    }
    if (mode === "insert") return insertBody;
    return sqlText;
  }

  function loadTextForMode(text: string): void {
    if (mode === "find") {
      try {
        const parsed = JSON.parse(text);
        if (parsed && typeof parsed === "object") {
          // Always reset the three textareas to a known shape so
          // JSON.stringify never receives `undefined` (which would
          // emit a non-string and break the state setter contract).
          setFilterText(
            parsed.filter !== undefined
              ? JSON.stringify(parsed.filter, null, 2)
              : "{}",
          );
          setProjectionText(
            parsed.projection !== undefined
              ? JSON.stringify(parsed.projection, null, 2)
              : "",
          );
          setSortText(
            parsed.sort !== undefined
              ? JSON.stringify(parsed.sort, null, 2)
              : "",
          );
        }
      } catch {
        // Legacy SQL-only bookmarks stored raw text; fall back to filter.
        setFilterText(text);
      }
      return;
    }
    if (mode === "aggregate") {
      setPipelineText(text);
      return;
    }
    if (mode === "update") {
      try {
        const parsed = JSON.parse(text);
        if (parsed && typeof parsed === "object") {
          setUpdateFilterText(
            parsed.filter !== undefined
              ? JSON.stringify(parsed.filter, null, 2)
              : "{}",
          );
          setUpdateText(
            parsed.update !== undefined
              ? JSON.stringify(parsed.update, null, 2)
              : "{}",
          );
          if (typeof parsed.multi === "boolean") setUpdateMulti(parsed.multi);
          if (typeof parsed.upsert === "boolean") setUpdateUpsert(parsed.upsert);
        }
      } catch {
        setUpdateFilterText(text);
      }
      return;
    }
    if (mode === "insert") {
      setInsertBody(text);
      return;
    }
    setSqlText(text);
  }

  function makeEntireQuerySource(): ExportSourceDto | null {
    if (mode === "find") {
      return {
        mode: "find",
        filterJson: filterText,
        projectionJson: projectionText || null,
        sortJson: sortText || null,
        pipelineJson: null,
        documentsJson: null,
      };
    }
    if (mode === "aggregate") {
      return {
        mode: "aggregate",
        filterJson: null,
        projectionJson: null,
        sortJson: null,
        pipelineJson: pipelineText,
        documentsJson: null,
      };
    }
    if (!sqlResult) {
      toast.push("Run the SQL query first to translate it before exporting.", "error");
      return null;
    }
    return {
      mode: "aggregate",
      filterJson: null,
      projectionJson: null,
      sortJson: null,
      pipelineJson: JSON.stringify(sqlResult.pipeline),
      documentsJson: null,
    };
  }

  function makeDocumentsSource(docs: Array<Record<string, unknown>>): ExportSourceDto {
    return {
      mode: "documents",
      filterJson: null,
      projectionJson: null,
      sortJson: null,
      pipelineJson: null,
      documentsJson: JSON.stringify(docs.map(restoreDisplayBson)),
    };
  }

  function handleExportQuery() {
    const source = makeEntireQuerySource();
    if (!source) return;
    setExportSource(source);
    setExportOpen(true);
  }

  const getRowId = useCallback((row: Record<string, unknown>, index: number) => {
    const id = toFilterId(row);
    if (id !== undefined) return `id:${JSON.stringify(id)}`;
    return `idx:${index}`;
  }, []);

  const selectedDocuments = useMemo(() => {
    const docs = (page?.documents ?? []) as Array<Record<string, unknown>>;
    return docs.filter((doc, index) => selectedRowIds.has(getRowId(doc, index)));
  }, [page, selectedRowIds, getRowId]);

  const selectedSource = useMemo(
    () => (selectedDocuments.length > 0 ? makeDocumentsSource(selectedDocuments) : null),
    [selectedDocuments],
  );

  async function handleCopySelected() {
    if (!selectedSource || selectedDocuments.length === 0) {
      toast.push("Select one or more visible rows first.", "warning");
      return;
    }
    try {
      const result = await commands.exportDocuments({
        connectionId,
        database,
        collection,
        jobId: crypto.randomUUID(),
        source: selectedSource,
        format: "json",
        destination: { kind: "clipboard", path: null },
        options: {
          jsonShape: "array",
          canonical: false,
          csvDelimiter: null,
          csvHeaders: true,
          csvColumns: null,
          compression: "none",
          csvArrayMode: null,
        },
      });
      if (result.clipboardText != null) {
        await navigator.clipboard.writeText(result.clipboardText);
      }
      toast.push(`Copied ${result.processed} selected document(s).`, "success");
    } catch (e) {
      toast.push(`Copy selected failed: ${describeError(e)}`, "error");
    }
  }

  // Respond to the native File > Export Results… menu action, dispatched
  // by App.tsx as a window event. Only the active tab is mounted, so only
  // one handler ever fires. A ref keeps the latest query context in scope.
  const exportHandlerRef = useRef(handleExportQuery);
  exportHandlerRef.current = handleExportQuery;
  useEffect(() => {
    const fn = () => exportHandlerRef.current();
    window.addEventListener("nosqlbuddy:export-results", fn);
    return () => window.removeEventListener("nosqlbuddy:export-results", fn);
  }, []);

  async function handleSaveToFile() {
    const ext = fileExtension(mode as QueryMode);
    try {
      const suggested = `${collection}.${ext}`;
      const path = await save({
        defaultPath: suggested,
        filters: fileFilter(mode as QueryMode),
      });
      if (!path) {
        toast.push("Save cancelled.", "info");
        return;
      }
      await writeTextFile(path, currentTextForMode());
      toast.push(`Saved to ${path}.`, "success");
    } catch (e) {
      const msg = describeError(e);
      toast.push(`Save failed: ${msg}`, "error");
    }
  }

  async function handleLoadFromFile() {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: fileFilter(mode as QueryMode),
      });
      if (!path || typeof path !== "string") {
        toast.push("Load cancelled.", "info");
        return;
      }
      const text = await readTextFile(path);
      loadTextForMode(text);
      if (mode === "aggregate") {
        // Force the visual AggregationEditor to re-mount with the
        // newly loaded pipeline.
        setPipelineKey((k) => k + 1);
      }
      toast.push(`Loaded from ${path}.`, "success");
    } catch (e) {
      const msg = describeError(e);
      toast.push(`Load failed: ${msg}`, "error");
    }
  }

  async function handleCopyCode() {
    if (!sqlResult) return;
    const code = sqlResult.code[sqlLanguage] ?? "";
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      setCopyNotice({ text: "Copied to clipboard.", tone: "success" });
      setTimeout(() => setCopyNotice(null), 1500);
    } catch {
      setCopyNotice({ text: "Clipboard copy failed.", tone: "warning" });
    }
  }

  function handleOpenInAggregationEditor() {
    if (!sqlResult) return;
    // Hand off the translated pipeline to the parent so it can decide
    // whether to open a new tab or hand control back. Fall back to
    // switching this tab into aggregation mode if no handler is wired.
    if (onOpenInAggregationEditor) {
      onOpenInAggregationEditor(sqlResult.pipeline);
      setSqlNotice("Opened in Aggregation Editor.");
      return;
    }
    setPipelineText(JSON.stringify(sqlResult.pipeline, null, 2));
    setMode("aggregate");
    setSqlNotice("Switched to Aggregation with translated pipeline.");
  }

  async function handleCellSaved(rowIdx: number, fieldPath: string, newValue: unknown) {
    // Update the in-memory page so the cell shows the new value
    // without a full re-run.
    setPage((prev) => {
      if (!prev) return prev;
      const docs = prev.documents.slice();
      const target = docs[rowIdx];
      if (!target) return prev;
      const updated: Record<string, unknown> = { ...target };
      applyPath(updated, fieldPath, newValue);
      docs[rowIdx] = updated;
      return { ...prev, documents: docs };
    });
    toast.push(`Saved ${fieldPath}.`, "success");
  }

  function handleCellError(_rowIdx: number, fieldPath: string, message: string) {
    toast.push(`${fieldPath}: ${message}`, "error");
  }

  function handleDeleteRow(rowIdx: number, doc: Record<string, unknown>) {
    const docId = toFilterId(doc);
    if (docId === undefined) {
      toast.push("Cannot delete a document without an `_id`.", "error");
      return;
    }
    setPendingDelete({ idx: rowIdx, docId });
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    const { idx, docId } = pendingDelete;
    setPendingDelete(null);
    try {
      const count = await commands.deleteDocuments(
        connectionId,
        database,
        collection,
        JSON.stringify({ _id: docId }),
      );
      if (count === 0) {
        toast.push(
          "Delete matched 0 documents — the `_id` did not round-trip. Nothing was removed.",
          "error",
        );
        return;
      }
      setPage((prev) => {
        if (!prev) return prev;
        const docs = prev.documents.slice();
        if (idx >= 0 && idx < docs.length) {
          docs.splice(idx, 1);
        }
        return { ...prev, documents: docs, totalCount: Math.max(0, (prev.totalCount ?? 0) - count) };
      });
      toast.push(`Deleted ${count} document(s).`, "success");
    } catch (e) {
      const msg = describeError(e);
      toast.push(`Delete failed: ${msg}`, "error");
    }
  }

  function cancelDelete() {
    setPendingDelete(null);
  }

  async function handleInserted(id: string) {
    toast.push(`Inserted document (id=${id || "auto"}).`, "success");
    // Refresh results so the new row appears.
    await run();
  }

  function handleInsertError(message: string) {
    toast.push(`Insert failed: ${message}`, "error");
  }

  async function handleBulkDeleteSelected() {
    if (selectedDocuments.length === 0) {
      toast.push("Select one or more rows first.", "warning");
      return;
    }
    const ids = selectedDocuments
      .map((doc) => toFilterId(doc))
      .filter((id) => id !== undefined);
    if (ids.length === 0) {
      toast.push("Selected rows lack `_id` fields.", "error");
      return;
    }
    if (!window.confirm(`Delete ${ids.length} selected document(s)? This cannot be undone.`)) {
      return;
    }
    try {
      const filter = ids.length === 1 ? { _id: ids[0] } : { _id: { $in: ids } };
      const count = await commands.deleteDocuments(
        connectionId,
        database,
        collection,
        JSON.stringify(filter),
      );
      toast.push(`Deleted ${count} document(s).`, "success");
      setSelectedRowIds(new Set());
      await run();
    } catch (e) {
      toast.push(`Bulk delete failed: ${describeError(e)}`, "error");
    }
  }

  async function handleBulkUpdateSelected() {
    if (selectedDocuments.length === 0) {
      toast.push("Select one or more rows first.", "warning");
      return;
    }
    const ids = selectedDocuments
      .map((doc) => toFilterId(doc))
      .filter((id) => id !== undefined);
    if (ids.length === 0) {
      toast.push("Selected rows lack `_id` fields.", "error");
      return;
    }
    const updateDoc = window.prompt("Enter $set update JSON (e.g. {\"$set\":{\"status\":\"archived\"}}):", '{"$set":{}}');
    if (!updateDoc) return;
    try {
      JSON.parse(updateDoc);
    } catch {
      toast.push("Invalid update JSON.", "error");
      return;
    }
    try {
      const filter = ids.length === 1 ? { _id: ids[0] } : { _id: { $in: ids } };
      const result = await commands.updateDocuments({
        connectionId,
        database,
        collection,
        filterJson: JSON.stringify(filter),
        updateJson: updateDoc,
        multi: true,
        upsert: false,
      });
      toast.push(
        `Updated ${result.modifiedCount} document(s) (matched ${result.matchedCount}).`,
        result.modifiedCount > 0 ? "success" : "warning",
      );
      setSelectedRowIds(new Set());
      await run();
    } catch (e) {
      toast.push(`Bulk update failed: ${describeError(e)}`, "error");
    }
  }

  const resultsSummary = page ? formatResultSummary(page, loadingMore) : "No results yet";

  return (
    <div className="pane">
      <div className="pane__header">
        <h2 className="pane__title">
          {database}.{collection}
        </h2>
        <div className="tabs-secondary" role="tablist" style={{ borderBottom: 0 }}>
          <div
            className={`tabs-secondary__item ${mode === "find" ? "is-active" : ""}`}
            role="tab"
            tabIndex={0}
            aria-selected={mode === "find"}
            onClick={() => setMode("find")}
            onKeyDown={(e) => e.key === "Enter" && setMode("find")}
          >
            Find
          </div>
          <div
            className={`tabs-secondary__item ${mode === "aggregate" ? "is-active" : ""}`}
            role="tab"
            tabIndex={0}
            aria-selected={mode === "aggregate"}
            onClick={() => setMode("aggregate")}
            onKeyDown={(e) => e.key === "Enter" && setMode("aggregate")}
          >
            Aggregation
          </div>
          <div
            className={`tabs-secondary__item ${mode === "sql" ? "is-active" : ""}`}
            role="tab"
            tabIndex={0}
            aria-selected={mode === "sql"}
            onClick={() => setMode("sql")}
            onKeyDown={(e) => e.key === "Enter" && setMode("sql")}
          >
            SQL
          </div>
          <div
            className={`tabs-secondary__item ${mode === "update" ? "is-active" : ""}`}
            role="tab"
            tabIndex={0}
            aria-selected={mode === "update"}
            onClick={() => setMode("update")}
            onKeyDown={(e) => e.key === "Enter" && setMode("update")}
          >
            Update
          </div>
          <div
            className={`tabs-secondary__item ${mode === "insert" ? "is-active" : ""}`}
            role="tab"
            tabIndex={0}
            aria-selected={mode === "insert"}
            onClick={() => setMode("insert")}
            onKeyDown={(e) => e.key === "Enter" && setMode("insert")}
          >
            Insert
          </div>
        </div>
        <div className="pane__sub">
          {running
            ? "Running…"
            : page
              ? formatResultSummary(page, loadingMore)
              : "Idle"}
        </div>
        <div className="pane__actions">
          <QueryHistoryPanel
            connectionId={connectionId}
            database={database}
            collection={collection}
            mode={mode as QueryMode}
            currentText={currentTextForMode()}
            onLoad={(text) => loadTextForMode(text)}
            onError={(message) => toast.push(message, "error")}
            onNotice={(message) => toast.push(message, "success")}
          />
          <button
            className="btn btn--sm"
            onClick={() => setInsertOpen(true)}
            title="Insert a new document into this collection"
          >
            Insert
          </button>
          <button
            className="btn btn--sm"
            onClick={() => setImportOpen(true)}
            title="Import JSON or CSV documents into this collection"
          >
            Import…
          </button>
          <button
            className="btn btn--sm"
            onClick={handleExportQuery}
            disabled={!page}
            title="Export this query's results to a file or the clipboard"
          >
            Export…
          </button>
          {resultsPanelState === "closed" && page && (
            <button
              className="btn btn--sm"
              onClick={() => setResultsPanelState("expanded")}
              title="Show the current query results"
            >
              Show results
            </button>
          )}
          <PaneActionsMenu
            items={[
              {
                id: "save",
                label: `Save ${fileExtension(mode as QueryMode).toUpperCase()}`,
                hint: `Save current ${mode} input`,
                onClick: () => void handleSaveToFile(),
              },
              {
                id: "load",
                label: `Load ${fileExtension(mode as QueryMode).toUpperCase()}`,
                hint: `Load ${mode} input from file`,
                onClick: () => void handleLoadFromFile(),
              },
              {
                id: "close",
                label: "Close tab",
                hint: "Close this query tab",
                onClick: onClose,
              },
            ]}
          />
          {mode !== "aggregate" && (
            <button
              className="btn btn--primary btn--sm"
              onClick={run}
              disabled={!valid || running}
            >
              {running ? "Running…" : "Run"}
            </button>
          )}
        </div>
      </div>
      <div className={`split split--results-${resultsPanelState}`}>
        <div className="editor">
          {mode === "find" && (
            <div className="pane__body" style={{ display: "grid", gridTemplateRows: "1fr 1fr 1fr" }}>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>Filter</span>
                  <div className="editor__toolbar-tabs">
                    <button
                      className={`editor__toolbar-tab ${filterEditor === "json" ? "is-active" : ""}`}
                      onClick={() => setFilterEditor("json")}
                      aria-pressed={filterEditor === "json"}
                    >
                      JSON
                    </button>
                    <button
                      className={`editor__toolbar-tab ${filterEditor === "visual" ? "is-active" : ""}`}
                      onClick={() => setFilterEditor("visual")}
                      aria-pressed={filterEditor === "visual"}
                    >
                      Visual
                    </button>
                  </div>
                </div>
                {filterEditor === "json" ? (
                  <CodeEditor
                    className="editor__textarea"
                    value={filterText}
                    onChange={setFilterText}
                    context="filter"
                    schema={schema.loading ? undefined : schema}
                  />
                ) : (
                  <div className="editor__textarea" style={{ overflow: "auto" }}>
                    <VisualQueryBuilder
                      filterJson={filterText}
                      onFilterJsonChange={(next) => {
                        setFilterText(next);
                      }}
                      connectionId={connectionId}
                      database={database}
                      collection={collection}
                    />
                  </div>
                )}
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>Projection <InfoPopover label="Projection help" title="Projection"><p>Specify which fields to return. Use <code>{'{ field: 1 }'}</code> to include fields or <code>{'{ field: 0 }'}</code> to exclude them. Leave empty to return all fields.</p></InfoPopover></span>
                  <span className="kbd">{`{ field: 1 }`}</span>
                </div>
                <CodeEditor
                  className="editor__textarea"
                  value={projectionText}
                  onChange={setProjectionText}
                  context="filter"
                  schema={schema.loading ? undefined : schema}
                  placeholder="Optional"
                />
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>Sort <InfoPopover label="Sort help" title="Sort"><p>Sort results by one or more fields. Use <code>{'{ field: 1 }'}</code> for ascending or <code>{'{ field: -1 }'}</code> for descending.</p></InfoPopover></span>
                  <span className="kbd">{`{ field: 1 }`}</span>
                </div>
                <CodeEditor
                  className="editor__textarea"
                  value={sortText}
                  onChange={setSortText}
                  context="filter"
                  schema={schema.loading ? undefined : schema}
                  placeholder="Optional"
                />
              </div>
            </div>
          )}
          {mode === "aggregate" && (
            <div className="pane__body">
              <AggregationEditor
                key={pipelineKey}
                connectionId={connectionId}
                database={database}
                collection={collection}
                profile={profile ?? null}
                onResult={onResult}
                onPipelineChange={(pipeline) =>
                  setPipelineText(JSON.stringify(pipeline, null, 2))
                }
                initialPipeline={parsePipeline(pipelineText)}
              />
            </div>
          )}
          {mode === "sql" && (
            <div className="pane__body" style={{ display: "grid", gridTemplateRows: "auto minmax(160px, 1fr) minmax(120px, 1fr) minmax(120px, 1fr)" }}>
              <div className="sql-toolbar" style={{ display: "flex", gap: 8, alignItems: "center", padding: "var(--space-2) var(--space-3)", flexWrap: "wrap" }}>
                <button
                  className="btn btn--sm"
                  onClick={handleOpenInAggregationEditor}
                  disabled={!sqlResult}
                  title="Open the translated pipeline in the Aggregation tab"
                >
                  Open in Agg Editor
                </button>
                {sqlNotice && (
                  <Alert tone="success" compact>{sqlNotice}</Alert>
                )}
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>SQL</span>
                  <span className="kbd">Translated on Run</span>
                </div>
                <CodeEditor
                  className="editor__textarea"
                  value={sqlText}
                  onChange={setSqlText}
                  context="sql"
                  ariaLabel="SQL query"
                />
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>
                    Generated pipeline
                  </span>
                </div>
                <pre className="json-view" style={{ background: "var(--surface)", flex: "1 1 0", overflow: "auto", margin: 0 }}>
                  {sqlResult ? (
                    <code
                      className="language-json"
                      dangerouslySetInnerHTML={{ __html: sqlPipelineHtml }}
                    />
                  ) : (
                    "Run to translate."
                  )}
                </pre>
                {sqlResult && sqlResult.warnings.length > 0 && (
                  <div style={{ padding: "0 var(--space-3)", display: "grid", gap: "var(--space-2)" }}>
                    {sqlResult.warnings.map((w, i) => (
                      <Alert key={i} tone="warning">{w}</Alert>
                    ))}
                  </div>
                )}
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar" style={{ display: "flex", gap: 8, alignItems: "center" }}>
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>
                    Query code
                  </span>
                  <select
                    className="input input--sm"
                    value={sqlLanguage}
                    onChange={(e) => setSqlLanguage(e.target.value as SqlLanguage)}
                    style={{ marginLeft: "auto" }}
                  >
                    {SQL_LANGUAGES.map((l) => (
                      <option key={l} value={l}>
                        {SQL_LANGUAGE_LABELS[l]}
                      </option>
                    ))}
                  </select>
                  <button
                    className="btn btn--sm"
                    onClick={handleCopyCode}
                    disabled={!sqlResult}
                  >
                    Copy
                  </button>
                  {copyNotice && (
                    <Alert tone={copyNotice.tone} compact>{copyNotice.text}</Alert>
                  )}
                </div>
                <pre className="json-view" style={{ background: "var(--surface)", flex: "1 1 0", overflow: "auto", margin: 0 }}>
                  {sqlResult ? (
                    <code
                      className={`language-${PRISM_LANG[sqlLanguage]}`}
                      dangerouslySetInnerHTML={{ __html: sqlCodeHtml }}
                    />
                  ) : (
                    "Run to generate."
                  )}
                </pre>
              </div>
            </div>
          )}
          {mode === "update" && (
            <div className="pane__body" style={{ display: "grid", gridTemplateRows: "1fr 1fr auto" }}>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>Filter <InfoPopover label="Filter help" title="Update filter"><p>Documents matching this filter will be updated. Use <code>{'{ field: value }'}</code> syntax.</p></InfoPopover></span>
                  <span className="kbd">{`{ field: value }`}</span>
                </div>
                <CodeEditor
                  className="editor__textarea"
                  value={updateFilterText}
                  onChange={setUpdateFilterText}
                  context="filter"
                  schema={schema.loading ? undefined : schema}
                />
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>Update <InfoPopover label="Update help" title="Update document"><p>MongoDB update operators such as <code>$set</code>, <code>$inc</code>, <code>$push</code>, etc.</p></InfoPopover></span>
                  <span className="kbd">{`{ $set: { ... } }`}</span>
                </div>
                <CodeEditor
                  className="editor__textarea"
                  value={updateText}
                  onChange={setUpdateText}
                  context="update"
                  schema={schema.loading ? undefined : schema}
                />
              </div>
              <div style={{ display: "flex", gap: 12, alignItems: "center", padding: "var(--space-2) var(--space-3)", flexWrap: "wrap" }}>
                <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, cursor: "pointer" }}>
                  <input
                    type="checkbox"
                    checked={updateMulti}
                    onChange={(e) => setUpdateMulti(e.target.checked)}
                  />
                  Update multiple
                </label>
                <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, cursor: "pointer" }}>
                  <input
                    type="checkbox"
                    checked={updateUpsert}
                    onChange={(e) => setUpdateUpsert(e.target.checked)}
                  />
                  Upsert
                </label>
                <button
                  className="btn btn--sm"
                  onClick={async () => {
                    if (!valid) {
                      toast.push("Fix the JSON syntax first.", "error");
                      return;
                    }
                    setUpdatePreviewLoading(true);
                    try {
                      const count = await commands.previewUpdate({
                        connectionId,
                        database,
                        collection,
                        filterJson: updateFilterText || null,
                        updateJson: updateText,
                      });
                      setUpdatePreviewCount(count);
                    } catch (e) {
                      toast.push(describeError(e), "error");
                    } finally {
                      setUpdatePreviewLoading(false);
                    }
                  }}
                  disabled={running || updatePreviewLoading}
                >
                  {updatePreviewLoading ? "Previewing…" : "Preview"}
                </button>
                {updatePreviewCount !== null && (
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>
                    {updatePreviewCount.toLocaleString()} document(s) will match
                  </span>
                )}
              </div>
            </div>
          )}
          {mode === "insert" && (
            <div className="pane__body" style={{ display: "grid", gridTemplateRows: "auto 1fr" }}>
              <div style={{ display: "flex", gap: 12, alignItems: "center", padding: "var(--space-2) var(--space-3)", flexWrap: "wrap" }}>
                <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, cursor: "pointer" }}>
                  <input
                    type="checkbox"
                    checked={insertMany}
                    onChange={(e) => setInsertMany(e.target.checked)}
                  />
                  Insert many (JSON array)
                </label>
              </div>
              <div className="editor__pane">
                <div className="editor__toolbar">
                  <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>
                    Document{insertMany ? "s" : ""} JSON
                    <InfoPopover label="Insert help" title="Insert document"><p>Paste a JSON {insertMany ? "array of objects" : "object"}. The <code>_id</code> field is optional.</p></InfoPopover>
                  </span>
                </div>
                <CodeEditor
                  className="editor__textarea"
                  value={insertBody}
                  onChange={setInsertBody}
                  context="insert"
                  schema={schema.loading ? undefined : schema}
                  placeholder={insertMany ? "[\n  { ... },\n  { ... }\n]" : "{\n  ...\n}"}
                />
              </div>
            </div>
          )}
        </div>
        {resultsPanelState !== "closed" && <div className="split__handle" aria-hidden="true" />}
        {resultsPanelState !== "closed" && (
          <div className="pane__body results-pane">
            {resultsPanelState === "minimized" ? (
              <div className="results-minibar" role="region" aria-label="Minimized results panel">
                <div className="results-minibar__text">
                  <strong>Results minimized</strong>
                  <span>{resultsSummary}</span>
                </div>
                <div className="results-minibar__actions">
                  <button
                    className="btn btn--sm btn--icon"
                    onClick={() => setResultsPanelState("expanded")}
                    title="Show results"
                    aria-label="Show results"
                  >
                    <Maximize2 size={14} aria-hidden />
                  </button>
                  <button
                    className="btn btn--sm btn--icon"
                    onClick={() => setResultsPanelState("closed")}
                    title="Close results"
                    aria-label="Close results"
                  >
                    <X size={14} aria-hidden />
                  </button>
                </div>
              </div>
            ) : page ? (
              <div className="results-view">
                <div className="results-view-toolbar">
                  {(["table", "tree", "json"] as ResultsViewMode[]).map((m) => (
                    <button
                      key={m}
                      className={`btn btn--sm ${viewMode === m ? "is-active" : ""}`}
                      onClick={() => setViewMode(m)}
                      aria-pressed={viewMode === m}
                    >
                      {m[0].toUpperCase() + m.slice(1)}
                    </button>
                  ))}
                  <span style={{ fontSize: 12, color: "var(--ink-muted)", marginLeft: "auto" }}>
                    {selectedDocuments.length} selected
                  </span>
                  <button
                    className="btn btn--sm"
                    disabled={selectedDocuments.length === 0}
                    onClick={() => void handleCopySelected()}
                    title="Copy selected documents as JSON"
                  >
                    Copy selected
                  </button>
                  <button
                    className="btn btn--sm btn--danger"
                    disabled={selectedDocuments.length === 0}
                    onClick={() => void handleBulkDeleteSelected()}
                    title="Delete all selected documents"
                  >
                    Delete selected
                  </button>
                  <button
                    className="btn btn--sm"
                    disabled={selectedDocuments.length === 0}
                    onClick={() => void handleBulkUpdateSelected()}
                    title="Update all selected documents with a $set operation"
                  >
                    Update selected
                  </button>
                  <button
                    className="btn btn--sm"
                    disabled={selectedDocuments.length === 0}
                    onClick={() => setSelectedRowIds(new Set())}
                  >
                    Clear
                  </button>
                  <span className="results-view-toolbar__divider" aria-hidden="true" />
                  <button
                    className="btn btn--sm btn--icon"
                    onClick={() => setResultsPanelState("minimized")}
                    title="Minimize results and give the query editor more space"
                    aria-label="Minimize results"
                  >
                    <Minus size={14} aria-hidden />
                  </button>
                  <button
                    className="btn btn--sm btn--icon"
                    onClick={() => setResultsPanelState("closed")}
                    title="Close results and give the query editor all available space"
                    aria-label="Close results"
                  >
                    <X size={14} aria-hidden />
                  </button>
                </div>
                <div className="results-view__table">
                  <ResultsTable
                    documents={page.documents as Array<Record<string, unknown>>}
                    connectionId={connectionId}
                    database={database}
                    collection={collection}
                    view={viewMode}
                    editable
                    onCellSaved={handleCellSaved}
                    onCellError={handleCellError}
                    onDeleteRow={handleDeleteRow}
                    selectable
                    selectedRowIds={selectedRowIds}
                    onSelectionChange={setSelectedRowIds}
                    getRowId={getRowId}
                  />
                </div>
                <PagingBar
                  total={page.totalCount}
                  totalApprox={!!page.totalCountApprox}
                  hasMore={page.hasMore}
                  loadingPage={loadingMore}
                  pageSize={pageSize}
                  currentPage={currentPage}
                  onPageSize={handlePageSize}
                  onJump={jumpToPage}
                />
              </div>
            ) : (
              <div className="empty-state">
                <h2>Run to see results</h2>
                <p>The query, projection, sort, and limit are all JSON.</p>
              </div>
            )}
          </div>
        )}
      </div>
      <InsertDocumentModal
        open={insertOpen}
        connectionId={connectionId}
        database={database}
        collection={collection}
        onClose={() => setInsertOpen(false)}
        onInserted={handleInserted}
        onError={handleInsertError}
      />
      {exportOpen && exportSource && (
        <ExportWizard
          connectionId={connectionId}
          database={database}
          collection={collection}
          profileName={profile?.name}
          source={exportSource}
          selectedSource={selectedSource}
          selectedCount={selectedDocuments.length}
          onClose={() => setExportOpen(false)}
        />
      )}
      {importOpen && (
        <ImportWizard
          connectionId={connectionId}
          database={database}
          collection={collection}
          onClose={() => setImportOpen(false)}
          onImported={() => {
            void run();
            onImported?.();
          }}
        />
      )}
      {pendingDelete && (
        <div className="modal-backdrop" role="dialog" aria-modal="true">
          <div className="modal" style={{ width: "min(420px, 92vw)" }}>
            <div className="modal__header">
              <h2 className="modal__title">Delete document?</h2>
            </div>
            <div className="modal__body">
              <p>This will permanently delete the document from {database}.{collection}.</p>
            </div>
            <div className="modal__footer" style={{ display: "flex", gap: 8, justifyContent: "flex-end", padding: "var(--space-3)" }}>
              <button className="btn btn--sm" onClick={cancelDelete}>Cancel</button>
              <button className="btn btn--primary btn--sm" onClick={() => void confirmDelete()}>
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function restoreDisplayBson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(restoreDisplayBson);
  if (value === null || typeof value !== "object") return value;

  const obj = value as Record<string, unknown>;
  if (typeof obj._idDisplay === "string") return { $oid: obj._idDisplay };
  if (typeof obj._dateDisplay === "string") {
    const raw = obj._dateDisplay;
    return /^\d+$/.test(raw)
      ? { $date: { $numberLong: raw } }
      : { $date: raw };
  }
  if (obj._decimalDisplay !== undefined) {
    return { $numberDecimal: String(obj._decimalDisplay) };
  }
  if (typeof obj._binaryDisplay === "string") {
    return { $binary: { base64: obj._binaryDisplay, subType: "00" } };
  }

  return Object.fromEntries(
    Object.entries(obj).map(([key, child]) => [key, restoreDisplayBson(child)]),
  );
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return "Unexpected error";
}

function safeParse<T>(text: string, fallback: T): T {
  const trimmed = text.trim();
  if (!trimmed) return fallback;
  try {
    return JSON.parse(trimmed) as T;
  } catch {
    return fallback;
  }
}

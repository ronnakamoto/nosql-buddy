import { useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "./ConfirmDialog";
import {
  type BookmarkEntry,
  type BookmarkSummary,
  type HistoryEntry,
  type QueryMode,
  clearHistory,
  deleteBookmark,
  deleteHistoryEntry,
  getBookmark,
  listBookmarks,
  listHistory,
  modeLabel,
  saveBookmark,
} from "../features/queryHistory";

export interface QueryHistoryPanelProps {
  connectionId: string;
  database: string;
  collection: string;
  mode: QueryMode;
  /** Called when the user picks a history entry or bookmark. */
  onLoad: (text: string) => void;
  /** Called on errors so the parent can show a toast. */
  onError: (message: string) => void;
  /** Called when a bookmark is saved. */
  onNotice: (message: string) => void;
  /** Current input text — pre-filled in the Save Bookmark modal. */
  currentText: string;
}

export function QueryHistoryPanel({
  connectionId,
  database,
  collection,
  mode,
  onLoad,
  onError,
  onNotice,
  currentText,
}: QueryHistoryPanelProps) {
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [bookmarks, setBookmarks] = useState<BookmarkSummary[]>([]);
  const [showSaveBookmark, setShowSaveBookmark] = useState(false);
  const [newBookmarkName, setNewBookmarkName] = useState("");
  const [confirmClearOpen, setConfirmClearOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement | null>(null);

  // Refresh the lists when the panel opens or any dependency changes.
  useEffect(() => {
    if (!open) return;
    refresh();
  }, [open, connectionId, database, collection, mode]);

  // Click outside closes the panel.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (!panelRef.current) return;
      if (!panelRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [open]);

  function refresh() {
    try {
      setHistory(listHistory(connectionId, database, collection, mode));
      setBookmarks(listBookmarks(connectionId, database, collection, mode));
    } catch (e) {
      onError(`History read failed: ${describeError(e)}`);
    }
  }

  function handleClearHistory() {
    setConfirmClearOpen(true);
  }

  function doClearHistory() {
    setConfirmClearOpen(false);
    try {
      clearHistory(connectionId, database, collection, mode);
      refresh();
      onNotice("History cleared.");
    } catch (e) {
      onError(`Clear failed: ${describeError(e)}`);
    }
  }

  function handleDeleteHistory(ts: number) {
    try {
      deleteHistoryEntry(connectionId, database, collection, mode, ts);
      refresh();
    } catch (e) {
      onError(`Delete failed: ${describeError(e)}`);
    }
  }

  function handleDeleteBookmark(name: string) {
    try {
      deleteBookmark(connectionId, database, collection, mode, name);
      refresh();
      onNotice(`Deleted bookmark "${name}".`);
    } catch (e) {
      onError(`Delete failed: ${describeError(e)}`);
    }
  }

  function handleLoadBookmark(name: string) {
    const entry = getBookmark(connectionId, database, collection, mode, name);
    if (!entry) {
      onError(`Bookmark "${name}" not found.`);
      return;
    }
    onLoad(entry.text);
    setOpen(false);
  }

  function handleLoadHistory(text: string) {
    onLoad(text);
    setOpen(false);
  }

  function handleSaveBookmark() {
    const name = newBookmarkName.trim();
    if (!name) {
      onError("Bookmark name cannot be empty.");
      return;
    }
    try {
      const entry: BookmarkEntry = saveBookmark(
        connectionId,
        database,
        collection,
        mode,
        name,
        currentText,
      );
      refresh();
      setShowSaveBookmark(false);
      setNewBookmarkName("");
      onNotice(`Saved bookmark "${entry.name}".`);
    } catch (e) {
      onError(`Save failed: ${describeError(e)}`);
    }
  }

  return (
    <span className="history-panel" ref={panelRef}>
      <button
        className="btn btn--sm"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        title={`History + bookmarks for ${modeLabel(mode)} mode`}
      >
        History
      </button>
      {open && (
        <div className="history-panel__popover" role="dialog" aria-label="Query history">
          <div className="history-panel__head">
            <strong>{modeLabel(mode)} · {database}.{collection}</strong>
            <button
              className="btn btn--sm history-panel__close"
              onClick={() => setOpen(false)}
              aria-label="Close history panel"
            >
              ×
            </button>
          </div>
          <div className="history-panel__section">
            <div className="history-panel__section-head">
              <span>Bookmarks ({bookmarks.length})</span>
              <button
                className="btn btn--sm"
                onClick={() => setShowSaveBookmark(true)}
              >
                Save current as…
              </button>
            </div>
            {bookmarks.length === 0 ? (
              <div className="history-panel__empty">
                No bookmarks yet. Use "Save current as…" to add one.
              </div>
            ) : (
              <ul className="history-panel__list">
                {bookmarks.map((b) => (
                  <li key={b.name} className="history-panel__item">
                    <button
                      className="history-panel__load"
                      onClick={() => handleLoadBookmark(b.name)}
                      title={`Load bookmark "${b.name}"`}
                    >
                      {b.name}
                    </button>
                    <span className="history-panel__meta">
                      {new Date(b.updated).toLocaleString()}
                    </span>
                    <button
                      className="btn btn--sm history-panel__delete"
                      onClick={() => handleDeleteBookmark(b.name)}
                      title="Delete bookmark"
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="history-panel__section">
            <div className="history-panel__section-head">
              <span>Recent runs ({history.length})</span>
              {history.length > 0 && (
                <button
                  className="btn btn--sm"
                  onClick={handleClearHistory}
                >
                  Clear
                </button>
              )}
            </div>
            {history.length === 0 ? (
              <div className="history-panel__empty">
                No history yet. Run a query to start capturing it.
              </div>
            ) : (
              <ul className="history-panel__list">
                {history.map((h) => (
                  <li key={h.ts} className="history-panel__item">
                    <button
                      className="history-panel__load"
                      onClick={() => handleLoadHistory(h.text)}
                      title="Load this run"
                    >
                      {summarizeRun(h)}
                    </button>
                    <span className="history-panel__meta">
                      {h.errored
                        ? "error"
                        : `${h.docCount ?? 0} docs${h.durationMs !== null ? ` · ${h.durationMs} ms` : ""}`}
                    </span>
                    <button
                      className="btn btn--sm history-panel__delete"
                      onClick={() => handleDeleteHistory(h.ts)}
                      title="Remove from history"
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {showSaveBookmark && (
            <div className="history-panel__save">
              <input
                className="input input--sm"
                placeholder="Bookmark name"
                value={newBookmarkName}
                onChange={(e) => setNewBookmarkName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleSaveBookmark();
                  if (e.key === "Escape") {
                    setShowSaveBookmark(false);
                    setNewBookmarkName("");
                  }
                }}
                autoFocus
              />
              <button
                className="btn btn--primary btn--sm"
                onClick={handleSaveBookmark}
              >
                Save
              </button>
              <button
                className="btn btn--sm"
                onClick={() => {
                  setShowSaveBookmark(false);
                  setNewBookmarkName("");
                }}
              >
                Cancel
              </button>
            </div>
          )}
        </div>
      )}
      <ConfirmDialog
        open={confirmClearOpen}
        title="Clear query history?"
        description="All recent runs for this collection and mode will be permanently removed. Saved bookmarks will not be affected."
        confirmLabel="Clear history"
        onConfirm={doClearHistory}
        onCancel={() => setConfirmClearOpen(false)}
      />
    </span>
  );
}

function summarizeRun(h: HistoryEntry): string {
  const text = h.text.replace(/\s+/g, " ").trim();
  if (text.length === 0) return "(empty)";
  if (text.length <= 60) return text;
  return text.slice(0, 57) + "…";
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

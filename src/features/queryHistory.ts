/**
 * Unified localStorage-backed store for "saved queries":
 *  - recent runs (a rolling history of up to `HISTORY_CAPACITY` entries
 *    per `database.collection::mode`), captured automatically when the
 *    user runs a query
 *  - named bookmarks (user-curated, persisted indefinitely until deleted)
 *  - raw-text bookmarks for the SQL sub-mode (legacy from the original
 *    implementation; preserved as a thin wrapper)
 *
 * Storage layout
 * --------------
 * key:   `query-history::${connectionId}::${database}.${collection}::${mode}`
 *        value: JSON array of HistoryEntry, most recent first.
 * key:   `query-bookmark::${connectionId}::${database}.${collection}::${mode}::${name}`
 *        value: JSON serialized BookmarkEntry.
 *
 * `connectionId` is part of the key so queries made against different
 * servers never collide in the history list. The exposed API is
 * connection-scoped, so callers don't need to think about that.
 */

export type QueryMode = "find" | "aggregate" | "sql" | "update" | "insert";

export interface HistoryEntry {
  /** Monotonically increasing timestamp (ms since epoch). */
  ts: number;
  /** The user-typed input text (JSON for find/aggregate, SQL for sql). */
  text: string;
  /** Wall-clock duration of the run in ms, if the run completed. */
  durationMs: number | null;
  /** Number of documents returned, if the run completed. */
  docCount: number | null;
  /** True if the run errored out. */
  errored: boolean;
}

export interface BookmarkEntry {
  name: string;
  text: string;
  /** ISO-8601 creation timestamp. */
  created: string;
  /** ISO-8601 last-modified timestamp. */
  updated: string;
}

export const HISTORY_CAPACITY = 20;

function historyKey(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
): string {
  return `query-history::${connectionId}::${database}.${collection}::${mode}`;
}

function bookmarkKey(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
  name: string,
): string {
  return `query-bookmark::${connectionId}::${database}.${collection}::${mode}::${name}`;
}

function safeParse<T>(raw: string | null, fallback: T): T {
  if (raw === null) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

// ---------- History ----------

export function listHistory(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
): HistoryEntry[] {
  const raw = window.localStorage.getItem(
    historyKey(connectionId, database, collection, mode),
  );
  const arr = safeParse<unknown[]>(raw, []);
  if (!Array.isArray(arr)) return [];
  return arr.filter(isHistoryEntry);
}

export function pushHistory(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
  entry: HistoryEntry,
): HistoryEntry[] {
  const existing = listHistory(connectionId, database, collection, mode);
  // Dedupe consecutive identical runs by replacing the latest one
  // with the new entry (so spamming Run on the same query doesn't
  // fill history with duplicates).
  const deduped =
    existing.length > 0 && existing[0].text === entry.text
      ? [entry, ...existing.slice(1)]
      : [entry, ...existing];
  const trimmed = deduped.slice(0, HISTORY_CAPACITY);
  window.localStorage.setItem(
    historyKey(connectionId, database, collection, mode),
    JSON.stringify(trimmed),
  );
  return trimmed;
}

export function deleteHistoryEntry(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
  ts: number,
): void {
  const filtered = listHistory(connectionId, database, collection, mode).filter(
    (e) => e.ts !== ts,
  );
  window.localStorage.setItem(
    historyKey(connectionId, database, collection, mode),
    JSON.stringify(filtered),
  );
}

export function clearHistory(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
): void {
  window.localStorage.removeItem(
    historyKey(connectionId, database, collection, mode),
  );
}

// ---------- Bookmarks ----------

export interface BookmarkSummary {
  name: string;
  created: string;
  updated: string;
}

export function listBookmarks(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
): BookmarkSummary[] {
  const prefix = `query-bookmark::${connectionId}::${database}.${collection}::${mode}::`;
  const out: BookmarkSummary[] = [];
  for (let i = 0; i < window.localStorage.length; i += 1) {
    const k = window.localStorage.key(i);
    if (!k || !k.startsWith(prefix)) continue;
    const name = k.slice(prefix.length);
    const raw = window.localStorage.getItem(k);
    const entry = safeParse<BookmarkEntry | null>(raw, null);
    if (!entry || entry.name !== name) continue;
    out.push({
      name: entry.name,
      created: entry.created,
      updated: entry.updated,
    });
  }
  return out.sort((a, b) => a.name.localeCompare(b.name));
}

export function getBookmark(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
  name: string,
): BookmarkEntry | null {
  const raw = window.localStorage.getItem(
    bookmarkKey(connectionId, database, collection, mode, name),
  );
  const entry = safeParse<BookmarkEntry | null>(raw, null);
  return entry && entry.name === name ? entry : null;
}

export function saveBookmark(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
  name: string,
  text: string,
): BookmarkEntry {
  const existing = getBookmark(connectionId, database, collection, mode, name);
  const now = new Date().toISOString();
  const entry: BookmarkEntry = {
    name,
    text,
    created: existing?.created ?? now,
    updated: now,
  };
  window.localStorage.setItem(
    bookmarkKey(connectionId, database, collection, mode, name),
    JSON.stringify(entry),
  );
  return entry;
}

export function deleteBookmark(
  connectionId: string,
  database: string,
  collection: string,
  mode: QueryMode,
  name: string,
): void {
  window.localStorage.removeItem(
    bookmarkKey(connectionId, database, collection, mode, name),
  );
}

// ---------- Type guards ----------

function isHistoryEntry(v: unknown): v is HistoryEntry {
  if (v === null || typeof v !== "object") return false;
  const e = v as Record<string, unknown>;
  return (
    typeof e.ts === "number" &&
    typeof e.text === "string" &&
    (e.durationMs === null || typeof e.durationMs === "number") &&
    (e.docCount === null || typeof e.docCount === "number") &&
    typeof e.errored === "boolean"
  );
}

// ---------- Mode-aware helpers ----------

export function modeLabel(mode: QueryMode): string {
  switch (mode) {
    case "find":
      return "Find";
    case "aggregate":
      return "Aggregation";
    case "sql":
      return "SQL";
    case "update":
      return "Update";
    case "insert":
      return "Insert";
  }
}

export function fileExtension(mode: QueryMode): "json" | "sql" {
  return mode === "sql" ? "sql" : "json";
}

export function fileFilter(mode: QueryMode): Array<{ name: string; extensions: string[] }> {
  return mode === "sql"
    ? [{ name: "SQL", extensions: ["sql"] }]
    : [{ name: "JSON", extensions: ["json"] }];
}

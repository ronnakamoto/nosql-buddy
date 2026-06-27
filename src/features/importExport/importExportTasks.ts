/**
 * localStorage-backed store for reusable import/export "tasks".
 *
 * A task is a named, persisted snapshot of an export or import configuration
 * (everything the user picked in the wizard: format, destination, options,
 * field mapping) minus the volatile `jobId`, so the same job can be re-run
 * later with one click. Mirrors the storage pattern in `queryHistory.ts`:
 * namespaced keys, safe JSON parse, type guards, sorted summaries.
 *
 * Storage layout
 * --------------
 * key:   `import-export-task::${connectionId}::${kind}::${name}`
 *        value: JSON serialized ImportExportTaskEntry.
 *
 * `connectionId` is part of the key so tasks never leak across servers. The
 * exposed API is connection-scoped. `kind` ("export" | "import") partitions
 * the two wizards so a name can exist in both without colliding.
 */

import type {
  ExportFormat,
  ExportOptions,
  ExportSourceDto,
  ImportFormat,
  ImportOptions,
  ImportSourceDto,
} from "../../ipc/commands";

export type ImportExportTaskKind = "export" | "import";

export interface ExportTaskPayload {
  kind: "export";
  database: string;
  collection: string;
  source: ExportSourceDto;
  format: ExportFormat;
  /** Destination is omitted: the user re-picks a path (or clipboard) at run
   * time, since placeholders resolve against the live context and a saved
   * absolute path would be stale across machines. */
  destinationKind: "file" | "clipboard" | "collection";
  targetDatabase?: string;
  targetCollection?: string;
  options: ExportOptions;
}

export interface ImportTaskPayload {
  kind: "import";
  database: string;
  collection: string;
  source: ImportSourceDto;
  format: ImportFormat;
  options: ImportOptions;
}

export type ImportExportTaskPayload = ExportTaskPayload | ImportTaskPayload;

export interface ImportExportTaskEntry {
  name: string;
  kind: ImportExportTaskKind;
  payload: ImportExportTaskPayload;
  /** ISO-8601 creation timestamp. */
  created: string;
  /** ISO-8601 last-modified timestamp. */
  updated: string;
}

export interface ImportExportTaskSummary {
  name: string;
  kind: ImportExportTaskKind;
  created: string;
  updated: string;
}

function taskKey(
  connectionId: string,
  kind: ImportExportTaskKind,
  name: string,
): string {
  return `import-export-task::${connectionId}::${kind}::${name}`;
}

function safeParse<T>(raw: string | null, fallback: T): T {
  if (raw === null) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

// ---------- Task CRUD ----------

export function listTasks(
  connectionId: string,
  kind?: ImportExportTaskKind,
): ImportExportTaskSummary[] {
  const prefix = kind
    ? `import-export-task::${connectionId}::${kind}::`
    : `import-export-task::${connectionId}::`;
  const out: ImportExportTaskSummary[] = [];
  for (let i = 0; i < window.localStorage.length; i += 1) {
    const k = window.localStorage.key(i);
    if (!k || !k.startsWith(prefix)) continue;
    const raw = window.localStorage.getItem(k);
    const entry = safeParse<ImportExportTaskEntry | null>(raw, null);
    if (!entry || !isTaskEntry(entry)) continue;
    // When `kind` is omitted the prefix is looser, so double-check the kind
    // matches one of the two valid values.
    if (
      entry.kind !== "export" &&
      entry.kind !== "import"
    ) {
      continue;
    }
    if (kind && entry.kind !== kind) continue;
    out.push({
      name: entry.name,
      kind: entry.kind,
      created: entry.created,
      updated: entry.updated,
    });
  }
  // Most recently updated first, then by name for stable ordering.
  out.sort((a, b) => {
    if (a.updated !== b.updated) return a.updated < b.updated ? 1 : -1;
    return a.name.localeCompare(b.name);
  });
  return out;
}

export function getTask(
  connectionId: string,
  kind: ImportExportTaskKind,
  name: string,
): ImportExportTaskEntry | null {
  const raw = window.localStorage.getItem(taskKey(connectionId, kind, name));
  const entry = safeParse<ImportExportTaskEntry | null>(raw, null);
  return entry && entry.name === name && entry.kind === kind ? entry : null;
}

export function saveTask(
  connectionId: string,
  kind: ImportExportTaskKind,
  name: string,
  payload: ImportExportTaskPayload,
): ImportExportTaskEntry {
  const trimmed = name.trim();
  if (!trimmed) {
    throw new Error("Task name cannot be empty.");
  }
  const existing = getTask(connectionId, kind, trimmed);
  const now = new Date().toISOString();
  const entry: ImportExportTaskEntry = {
    name: trimmed,
    kind,
    payload,
    created: existing?.created ?? now,
    updated: now,
  };
  window.localStorage.setItem(
    taskKey(connectionId, kind, trimmed),
    JSON.stringify(entry),
  );
  return entry;
}

export function deleteTask(
  connectionId: string,
  kind: ImportExportTaskKind,
  name: string,
): void {
  window.localStorage.removeItem(taskKey(connectionId, kind, name));
}

// ---------- Type guards ----------

function isTaskEntry(v: unknown): v is ImportExportTaskEntry {
  if (v === null || typeof v !== "object") return false;
  const e = v as Record<string, unknown>;
  return (
    typeof e.name === "string" &&
    (e.kind === "export" || e.kind === "import") &&
    typeof e.created === "string" &&
    typeof e.updated === "string" &&
    e.payload !== null &&
    typeof e.payload === "object"
  );
}

// ---------- Helpers ----------

export function kindLabel(kind: ImportExportTaskKind): string {
  return kind === "export" ? "Export" : "Import";
}

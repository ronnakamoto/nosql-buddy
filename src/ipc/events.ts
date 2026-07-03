// Typed event listeners for the frontend.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ConnectionOpenedPayload {
  connectionId: string;
  profileId: string;
  name: string;
}

export interface ConnectionClosedPayload {
  connectionId: string;
  profileId: string;
  at: string;
}

export async function onConnectionOpened(
  handler: (payload: ConnectionOpenedPayload) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionOpenedPayload>("connection-opened", (event) =>
    handler(event.payload),
  );
}

export async function onConnectionClosed(
  handler: (payload: ConnectionClosedPayload) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionClosedPayload>("connection-closed", (event) =>
    handler(event.payload),
  );
}

export interface ConnectionProgressPayload {
  /** Stable phase id: `resolve`, `authenticate`, `metadata`, `discover`. */
  phase: string;
  /** Human-readable label for the stepper. */
  label: string;
  /** `"active"` when the phase begins, `"done"` when it completes. */
  status: "active" | "done";
}

/**
 * Subscribe to `connection-progress` events emitted during `open_connection`.
 * Lets the UI show a stepper instead of a blank workspace while the driver
 * resolves the URI, performs the TLS + SCRAM handshake, reads deployment
 * metadata, and lists databases. Atlas connections can take several seconds.
 */
export async function onConnectionProgress(
  handler: (payload: ConnectionProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionProgressPayload>("connection-progress", (event) =>
    handler(event.payload),
  );
}

export async function onMenuAction(
  handler: (action: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("menu-action", (event) => handler(event.payload));
}

export interface AuditSetupProgressPayload {
  line: string;
}

/** Subscribe to live audit setup wizard progress lines (secret-redacted). */
export async function onAuditSetupProgress(
  handler: (line: string) => void,
): Promise<UnlistenFn> {
  return listen<AuditSetupProgressPayload>("audit-setup-progress", (event) =>
    handler(event.payload.line),
  );
}

export interface AuditStackProgressPayload {
  line: string;
}

/**
 * Subscribe to live progress lines while the dev audit stack starts
 * (`docker compose up`, including the source-build compile output on a cold
 * cache). Lets the UI show what's happening instead of a bare spinner for
 * however long that takes.
 */
export async function onAuditStackProgress(
  handler: (line: string) => void,
): Promise<UnlistenFn> {
  return listen<AuditStackProgressPayload>("audit-stack-progress", (event) =>
    handler(event.payload.line),
  );
}

export interface ImportExportProgressPayload {
  jobId: string;
  phase: string;
  processed: number;
  total: number | null;
  message: string;
}

/** Subscribe to live import/export job progress (throttled in the backend). */
export async function onImportExportProgress(
  handler: (payload: ImportExportProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<ImportExportProgressPayload>("import-export-progress", (event) =>
    handler(event.payload),
  );
}

export interface JobStatusChangedPayload {
  jobId: string;
  status: string;
  message: string;
  finishedAt: string | null;
}

export async function onJobStatusChanged(
  handler: (payload: JobStatusChangedPayload) => void,
): Promise<UnlistenFn> {
  return listen<JobStatusChangedPayload>("job-status-changed", (event) =>
    handler(event.payload),
  );
}

export interface JobLogEntryPayload {
  jobId: string;
  timestamp: string;
  level: string;
  message: string;
}

export async function onJobLogEntry(
  handler: (payload: JobLogEntryPayload) => void,
): Promise<UnlistenFn> {
  return listen<JobLogEntryPayload>("job-log-entry", (event) =>
    handler(event.payload),
  );
}

export interface DataModelProgressPayload {
  database: string;
  collection: string;
  done: number;
  total: number;
  error?: string | null;
}

export async function onDataModelProgress(
  handler: (payload: DataModelProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<DataModelProgressPayload>("data-model-progress", (event) =>
    handler(event.payload),
  );
}

export interface DataModelUpdatedPayload {
  database: string;
}

/**
 * Subscribe to `data-model-updated` — emitted when a scan completes or an edge
 * override is applied. The handler receives the database name so it can reload
 * the cached graph via `getDataModel`.
 */
export async function onDataModelUpdated(
  handler: (payload: DataModelUpdatedPayload) => void,
): Promise<UnlistenFn> {
  return listen<DataModelUpdatedPayload>("data-model-updated", (event) =>
    handler(event.payload),
  );
}

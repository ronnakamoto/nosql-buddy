import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import commands, {
  formatError,
  PLACEHOLDER_TOKENS,
  type ExportDestinationKind,
  type ExportFormat,
  type ExportOptions,
  type ExportSourceDto,
  type FieldMappingEntry,
  type JsonShape,
  type SchemaField,
  type SchemaReport,
} from "../../ipc/commands";
import { onImportExportProgress } from "../../ipc/events";
import { Alert } from "../../components/Alert";
import { useToast } from "../../context/ToastContext";
import {
  FieldMappingTable,
  type DiscoveredField,
  identityMapping,
} from "./FieldMappingTable";
import { resolvePathPreview } from "./placeholders";
import {
  deleteTask,
  getTask,
  kindLabel,
  listTasks,
  saveTask,
  type ExportTaskPayload,
  type ImportExportTaskSummary,
} from "./importExportTasks";

export interface ExportWizardProps {
  connectionId: string;
  database: string;
  collection: string;
  /** Profile display name, used to resolve `${profile}` in path placeholders. */
  profileName?: string;
  /** The current query context to export (find / aggregate / documents). */
  source: ExportSourceDto;
  /** The currently selected visible rows, if any, as an explicit documents source. */
  selectedSource?: ExportSourceDto | null;
  selectedCount?: number;
  onClose: () => void;
}

type Phase = "config" | "running" | "done" | "error";
type ExportDestinationChoice = ExportDestinationKind | "collection";
type ExportScope = "entire" | "selected";

function fileExtensionFor(format: ExportFormat, shape: JsonShape): string {
  if (format === "csv") return "csv";
  return shape === "ndjson" ? "ndjson" : "json";
}

/** Convert a schema report into the discovered-field list the mapping table
 * consumes. Picks the most frequent non-null type as the field's bsonType and
 * marks object-typed fields as expandable. */
function discoveredFieldsFromSchema(report: SchemaReport | null): DiscoveredField[] {
  if (!report) return [];
  return report.fields.map((f) => schemaFieldToDiscovered(f));
}

function schemaFieldToDiscovered(f: SchemaField): DiscoveredField {
  const types = f.types;
  // Drop "null" from the type ranking so a mostly-null field still shows its
  // real type.
  const ranked = Object.entries(types)
    .filter(([k]) => k !== "null")
    .sort((a, b) => b[1] - a[1]);
  const top = ranked[0]?.[0] ?? "unknown";
  const isObject = "object" in types;
  const samples = (f.topValues ?? []).map((v) => v.value).slice(0, 3);
  return {
    path: f.name,
    bsonType: top,
    isObject,
    samples,
  };
}

export function ExportWizard({
  connectionId,
  database,
  collection,
  profileName = "",
  source,
  selectedSource = null,
  selectedCount = 0,
  onClose,
}: ExportWizardProps) {
  const [format, setFormat] = useState<ExportFormat>("json");
  const [destination, setDestination] = useState<ExportDestinationChoice>("file");
  const [scope, setScope] = useState<ExportScope>("entire");
  const [jsonShape, setJsonShape] = useState<JsonShape>("array");
  const [canonical, setCanonical] = useState(false);
  const [csvDelimiter, setCsvDelimiter] = useState(",");
  const [csvHeaders, setCsvHeaders] = useState(true);
  const [compression, setCompression] = useState<import("../../ipc/commands").CompressionFormat>("none");
  const [csvArrayMode, setCsvArrayMode] = useState<import("../../ipc/commands").CsvArrayMode | null>(null);
  const [targetDatabase, setTargetDatabase] = useState(database);
  const [targetCollection, setTargetCollection] = useState(`${collection}_copy`);

  // Field mapping: discovered fields (from a schema sample) + the user's edits.
  const [showMapping, setShowMapping] = useState(false);
  const [discoveredFields, setDiscoveredFields] = useState<DiscoveredField[]>([]);
  const [mappingEntries, setMappingEntries] = useState<FieldMappingEntry[]>([]);
  const [mappingLoading, setMappingLoading] = useState(false);

  // Saved tasks (localStorage).
  const [tasks, setTasks] = useState<ImportExportTaskSummary[]>([]);
  const [taskName, setTaskName] = useState("");
  const [taskMessage, setTaskMessage] = useState<string | null>(null);

  const toast = useToast();
  const [phase, setPhase] = useState<Phase>("config");
  const [processed, setProcessed] = useState(0);
  const [total, setTotal] = useState<number | null>(null);

  const jobIdRef = useRef<string | null>(null);

  // Refresh the saved-task list whenever the wizard opens or a save/delete
  // happens. Cheap: localStorage scan.
  const refreshTasks = useCallback(() => {
    setTasks(listTasks(connectionId, "export"));
  }, [connectionId]);
  useEffect(() => {
    refreshTasks();
  }, [refreshTasks]);

  // Subscribe to progress for the active job.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onImportExportProgress((p) => {
      if (p.jobId !== jobIdRef.current) return;
      setProcessed(p.processed);
      setTotal(p.total);
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // Placeholder preview context + a sample template, shown so the user can see
  // how tokens expand before they type one into the native save dialog.
  const placeholderCtx = useMemo(
    () => ({ database, collection, profile: profileName }),
    [database, collection, profileName],
  );
  const sampleTemplate = `${collection}_${"${db}"}_${"${date}"}.json`;
  const sampleResolved = useMemo(
    () => resolvePathPreview(sampleTemplate, placeholderCtx),
    [sampleTemplate, placeholderCtx],
  );

  // Discover fields for the mapping table on demand. Uses the collection-wide
  // schema sample (find/documents modes); for aggregate, the discovered fields
  // are a best-effort starting point the user can edit.
  const discoverFields = useCallback(async () => {
    setMappingLoading(true);
    try {
      const report = await commands.sampleSchema(connectionId, database, collection);
      const fields = discoveredFieldsFromSchema(report);
      setDiscoveredFields(fields);
      setMappingEntries(identityMapping(fields));
      setShowMapping(true);
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setMappingLoading(false);
    }
  }, [connectionId, database, collection, toast]);

  const runExport = useCallback(async () => {
    const jobId = crypto.randomUUID();
    jobIdRef.current = jobId;
    const activeSource = scope === "selected" && selectedSource ? selectedSource : source;

    let path: string | null = null;
    if (destination === "file") {
      const ext = fileExtensionFor(format, jsonShape);
      const chosen = await save({
        defaultPath: `${collection}.${ext}`,
        filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
      });
      if (!chosen) return; // user cancelled the dialog
      path = chosen;
    }

    setPhase("running");
    setProcessed(0);
    setTotal(null);
    try {
      if (destination === "collection") {
        if (!targetDatabase.trim() || !targetCollection.trim()) {
          throw new Error("Target database and collection are required.");
        }
        const result = await commands.copyDocuments({
          connectionId,
          database,
          collection,
          jobId,
          source: activeSource,
          target: {
            database: targetDatabase.trim(),
            collection: targetCollection.trim(),
          },
          batchSize: 1000,
        });
        if (result.cancelled) {
          setPhase("config");
          toast.push("Collection copy cancelled.", "info");
          return;
        }
        toast.push(
          `Copied ${result.inserted} document(s) to ${targetDatabase.trim()}.${targetCollection.trim()}.`,
          "success",
        );
        setProcessed(result.processed);
        setPhase("done");
        return;
      }

      // Only attach the mapping when the user has opened the table and there
      // are entries; an empty/absent mapping means "use original fields".
      const fieldMapping =
        showMapping && mappingEntries.length > 0 ? mappingEntries : null;

      const options: ExportOptions = {
        jsonShape,
        canonical,
        csvDelimiter: format === "csv" ? csvDelimiter : null,
        csvHeaders,
        csvColumns: null,
        compression,
        csvArrayMode: format === "csv" ? csvArrayMode : null,
        fieldMapping,
      };

      const result = await commands.exportDocuments({
        connectionId,
        database,
        collection,
        jobId,
        source: activeSource,
        format,
        destination: { kind: destination, path },
        options,
      });

      if (result.cancelled) {
        setPhase("config");
        toast.push("Export cancelled.", "info");
        return;
      }

      if (destination === "clipboard" && result.clipboardText != null) {
        await navigator.clipboard.writeText(result.clipboardText);
        toast.push(`Copied ${result.processed} documents to the clipboard.`, "success");
      } else {
        toast.push(
          `Exported ${result.processed} documents${result.path ? ` to ${result.path}` : ""}.`,
          "success",
        );
      }
      setProcessed(result.processed);
      setPhase("done");
    } catch (e) {
      toast.push(formatError(e), "error");
      setPhase("error");
    } finally {
      jobIdRef.current = null;
    }
  }, [
    connectionId,
    database,
    collection,
    source,
    selectedSource,
    scope,
    format,
    destination,
    jsonShape,
    canonical,
    csvDelimiter,
    csvHeaders,
    compression,
    csvArrayMode,
    targetDatabase,
    targetCollection,
    showMapping,
    mappingEntries,
    toast,
  ]);

  const cancel = useCallback(async () => {
    if (jobIdRef.current) {
      try {
        await commands.cancelImportExport(jobIdRef.current);
      } catch {
        // ignore, the job will finish or already finished
      }
    }
  }, []);

  const saveCurrentTask = useCallback(async () => {
    setTaskMessage(null);
    const name = taskName.trim();
    if (!name) {
      setTaskMessage("Enter a task name first.");
      return;
    }
    const payload: ExportTaskPayload = {
      kind: "export",
      database,
      collection,
      source,
      format,
      destinationKind: destination,
      targetDatabase: destination === "collection" ? targetDatabase : undefined,
      targetCollection: destination === "collection" ? targetCollection : undefined,
      options: {
        jsonShape,
        canonical,
        csvDelimiter: format === "csv" ? csvDelimiter : null,
        csvHeaders,
        csvColumns: null,
        compression,
        csvArrayMode: format === "csv" ? csvArrayMode : null,
        fieldMapping: showMapping && mappingEntries.length > 0 ? mappingEntries : null,
      },
    };
    try {
      saveTask(connectionId, "export", name, payload);
      setTaskMessage(`Saved task "${name}".`);
      setTaskName("");
      refreshTasks();
    } catch (e) {
      setTaskMessage(formatError(e));
    }
  }, [
    connectionId,
    database,
    collection,
    source,
    format,
    destination,
    targetDatabase,
    targetCollection,
    jsonShape,
    canonical,
    csvDelimiter,
    csvHeaders,
    compression,
    csvArrayMode,
    showMapping,
    mappingEntries,
    taskName,
    refreshTasks,
  ]);

  const loadTask = useCallback(
    (name: string) => {
      setTaskMessage(null);
      const entry = getTask(connectionId, "export", name);
      if (!entry || entry.payload.kind !== "export") {
        setTaskMessage(`Task "${name}" not found.`);
        return;
      }
      const p = entry.payload;
      setFormat(p.format);
      setDestination(p.destinationKind);
      if (p.targetDatabase) setTargetDatabase(p.targetDatabase);
      if (p.targetCollection) setTargetCollection(p.targetCollection);
      setJsonShape(p.options.jsonShape);
      setCanonical(p.options.canonical);
      if (p.options.csvDelimiter != null) setCsvDelimiter(p.options.csvDelimiter);
      setCsvHeaders(p.options.csvHeaders);
      const fm = p.options.fieldMapping ?? null;
      if (fm && fm.length > 0) {
        setShowMapping(true);
        setMappingEntries(fm);
        // Discovered fields aren't persisted; derive a minimal list from the
        // mapping entries so the table can still show types/expand actions.
        setDiscoveredFields(
          fm.map((e) => ({
            path: e.source,
            bsonType: "unknown",
            isObject: e.source.includes("."),
            samples: [],
          })),
        );
      } else {
        setShowMapping(false);
        setMappingEntries([]);
      }
      setTaskMessage(`Loaded task "${name}".`);
    },
    [connectionId],
  );

  const removeTask = useCallback(
    (name: string) => {
      deleteTask(connectionId, "export", name);
      refreshTasks();
    },
    [connectionId, refreshTasks],
  );

  const pct =
    total && total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : null;

  const mappingActive = showMapping && mappingEntries.length > 0;

  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="Export documents"
      onClick={(e) => {
        if (e.target === e.currentTarget && phase !== "running") onClose();
      }}
    >
      <div className="modal" style={{ width: "min(620px, 94vw)" }}>
        <div className="modal__header">
          <div className="modal__heading">
            <h2 className="modal__title">Export</h2>
            <span className="modal__subtitle">
              {database}.{collection}
            </span>
          </div>
          <button
            className="modal__close"
            onClick={onClose}
            disabled={phase === "running"}
            aria-label="Close"
          >
            ×
          </button>
        </div>

        <div className="modal__body" style={{ display: "grid", gap: "var(--space-4)" }}>
          {selectedSource && selectedCount > 0 && (
            <div className="field">
              <span className="field__label">Scope</span>
              <div className="row" style={{ gap: "var(--space-2)" }}>
                <button
                  className={`btn btn--sm ${scope === "entire" ? "is-active" : ""}`}
                  onClick={() => setScope("entire")}
                  disabled={phase === "running"}
                  aria-pressed={scope === "entire"}
                >
                  Entire query result
                </button>
                <button
                  className={`btn btn--sm ${scope === "selected" ? "is-active" : ""}`}
                  onClick={() => setScope("selected")}
                  disabled={phase === "running"}
                  aria-pressed={scope === "selected"}
                >
                  Selected documents ({selectedCount})
                </button>
              </div>
            </div>
          )}

          {destination !== "collection" && (
            <div className="field">
            <span className="field__label">Format</span>
            <div className="row" style={{ gap: "var(--space-2)" }}>
              {(["json", "csv", "bson"] as ExportFormat[]).map((f) => (
                <button
                  key={f}
                  className={`btn btn--sm ${format === f ? "is-active" : ""}`}
                  onClick={() => setFormat(f)}
                  disabled={phase === "running"}
                  aria-pressed={format === f}
                >
                  {f.toUpperCase()}
                </button>
              ))}
            </div>
            </div>
          )}

          {destination === "file" && (
            <div className="field">
              <label className="field__label">Compression</label>
              <select
                className="field__select"
                value={compression}
                onChange={(e) => setCompression(e.target.value as import("../../ipc/commands").CompressionFormat)}
                disabled={phase === "running"}
              >
                <option value="none">None</option>
                <option value="gzip">Gzip</option>
                <option value="zstd">Zstd</option>
              </select>
            </div>
          )}

          <div className="field">
            <span className="field__label">Destination</span>
            <div className="row" style={{ gap: "var(--space-2)" }}>
              {(["file", "clipboard", "collection"] as ExportDestinationChoice[]).map((d) => (
                <button
                  key={d}
                  className={`btn btn--sm ${destination === d ? "is-active" : ""}`}
                  onClick={() => setDestination(d)}
                  disabled={phase === "running"}
                  aria-pressed={destination === d}
                >
                  {d === "file" ? "File" : d === "clipboard" ? "Clipboard" : "Collection"}
                </button>
              ))}
            </div>
          </div>

          {destination === "collection" && (
            <div className="row" style={{ gap: "var(--space-3)", alignItems: "end" }}>
              <label className="field" style={{ flex: 1 }}>
                <span className="field__label">Target database</span>
                <input
                  className="field__input"
                  value={targetDatabase}
                  onChange={(e) => setTargetDatabase(e.target.value)}
                  disabled={phase === "running"}
                />
              </label>
              <label className="field" style={{ flex: 1 }}>
                <span className="field__label">Target collection</span>
                <input
                  className="field__input"
                  value={targetCollection}
                  onChange={(e) => setTargetCollection(e.target.value)}
                  disabled={phase === "running"}
                />
              </label>
            </div>
          )}

          {destination === "file" && (
            <div className="field">
              <span className="field__label">Path placeholders (optional)</span>
              <span className="field__hint">
                Use tokens in the filename you pick: {PLACEHOLDER_TOKENS.join(" ")}. They
                expand at export time. Example:{" "}
                <code>{sampleTemplate}</code> → <code>{sampleResolved}</code>.
              </span>
            </div>
          )}

          {destination !== "collection" && format === "json" && (
            <>
              <div className="field">
                <span className="field__label">JSON shape</span>
                <div className="row" style={{ gap: "var(--space-2)" }}>
                  {(["array", "ndjson"] as JsonShape[]).map((s) => (
                    <button
                      key={s}
                      className={`btn btn--sm ${jsonShape === s ? "is-active" : ""}`}
                      onClick={() => setJsonShape(s)}
                      disabled={phase === "running"}
                      aria-pressed={jsonShape === s}
                    >
                      {s === "array" ? "JSON array" : "NDJSON (one per line)"}
                    </button>
                  ))}
                </div>
              </div>
              <label className="row" style={{ gap: "var(--space-2)", alignItems: "center" }}>
                <input
                  type="checkbox"
                  checked={canonical}
                  onChange={(e) => setCanonical(e.target.checked)}
                  disabled={phase === "running"}
                />
                <span style={{ fontSize: 13 }}>
                  Canonical Extended JSON (exact BSON types, more verbose)
                </span>
              </label>
            </>
          )}

          {destination !== "collection" && format === "csv" && (
            <>
              <div className="field">
                <span className="field__label">Delimiter</span>
                <input
                  className="field__input"
                  value={csvDelimiter}
                  maxLength={1}
                  onChange={(e) => setCsvDelimiter(e.target.value || ",")}
                  disabled={phase === "running"}
                  style={{ width: 64 }}
                />
              </div>
              <label className="row" style={{ gap: "var(--space-2)", alignItems: "center" }}>
                <input
                  type="checkbox"
                  checked={csvHeaders}
                  onChange={(e) => setCsvHeaders(e.target.checked)}
                  disabled={phase === "running"}
                />
                <span style={{ fontSize: 13 }}>Include header row</span>
              </label>
              <div className="field">
                <span className="field__label">Array handling</span>
                <select
                  className="field__select"
                  value={csvArrayMode ?? "jsonString"}
                  onChange={(e) => setCsvArrayMode(e.target.value as import("../../ipc/commands").CsvArrayMode)}
                  disabled={phase === "running"}
                >
                  <option value="jsonString">Serialize as JSON string</option>
                  <option value="flatten">Flatten: dotted keys (tags.0, tags.1)</option>
                </select>
              </div>
              <Alert tone="warning" style={{ margin: 0 }}>
                CSV cannot represent all BSON types. Nested objects, arrays, and
                binary become JSON strings; ObjectId and dates become their
                display form. Use JSON for a lossless export.
              </Alert>
            </>
          )}

          {destination !== "collection" && (
            <div className="field">
              <div className="row" style={{ gap: "var(--space-2)", alignItems: "center" }}>
                <span className="field__label" style={{ margin: 0 }}>Field mapping</span>
                {!showMapping && (
                  <button
                    className="btn btn--sm"
                    onClick={discoverFields}
                    disabled={phase === "running" || mappingLoading}
                    aria-label="Discover fields and open the mapping table"
                  >
                    {mappingLoading ? "Sampling…" : "Map fields…"}
                  </button>
                )}
                {showMapping && (
                  <button
                    className="btn btn--sm btn--ghost"
                    onClick={() => setShowMapping(false)}
                    disabled={phase === "running"}
                  >
                    Hide
                  </button>
                )}
              </div>
              {showMapping && (
                <FieldMappingTable
                  fields={discoveredFields}
                  entries={mappingEntries}
                  onChange={setMappingEntries}
                  disabled={phase === "running"}
                />
              )}
              {!showMapping && (
                <span className="field__hint">
                  Optional: rename, skip, or flatten fields before export. The
                  mapping table becomes the output schema; undeclared fields are
                  dropped.
                </span>
              )}
            </div>
          )}

          {/* Saved tasks */}
          <div className="field">
            <div className="row" style={{ gap: "var(--space-2)", alignItems: "end" }}>
              <label className="field" style={{ flex: 1 }}>
                <span className="field__label">Save as task</span>
                <input
                  className="field__input"
                  value={taskName}
                  onChange={(e) => setTaskName(e.target.value)}
                  placeholder="task name"
                  disabled={phase === "running"}
                />
              </label>
              <button
                className="btn btn--sm"
                onClick={saveCurrentTask}
                disabled={phase === "running"}
              >
                Save
              </button>
            </div>
            {tasks.length > 0 && (
              <div className="row" style={{ gap: "var(--space-2)", flexWrap: "wrap", marginTop: "var(--space-1)" }}>
                {tasks.map((t) => (
                  <span
                    key={t.name}
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: "var(--space-1)",
                      padding: "2px var(--space-2)",
                      border: "1px solid var(--border)",
                      borderRadius: "var(--radius-sm)",
                      fontSize: 12,
                    }}
                  >
                    <button
                      className="btn btn--sm btn--ghost"
                      style={{ padding: "0 4px", fontSize: 12 }}
                      onClick={() => loadTask(t.name)}
                      disabled={phase === "running"}
                      title={`Load ${kindLabel(t.kind)} task "${t.name}"`}
                    >
                      {t.name}
                    </button>
                    <button
                      className="btn btn--sm btn--ghost"
                      style={{ padding: "0 4px", fontSize: 12, color: "var(--danger-500)" }}
                      onClick={() => removeTask(t.name)}
                      disabled={phase === "running"}
                      title="Delete this task"
                      aria-label={`Delete task ${t.name}`}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
            )}
            {taskMessage && (
              <span className="field__hint">{taskMessage}</span>
            )}
          </div>

          {phase === "running" && (
            <div className="field">
              <span className="field__label">
                Exporting… {processed.toLocaleString()}
                {total != null ? ` / ${total.toLocaleString()}` : ""}
                {pct != null ? ` (${pct}%)` : ""}
              </span>
              <div
                style={{
                  height: 6,
                  borderRadius: "var(--radius-sm)",
                  background: "var(--surface-3)",
                  overflow: "hidden",
                }}
              >
                <div
                  style={{
                    height: "100%",
                    width: pct != null ? `${pct}%` : "100%",
                    background: "var(--accent-500)",
                    transition: "width 200ms ease-out",
                  }}
                />
              </div>
            </div>
          )}

        </div>

        <div className="modal__footer row row--end" style={{ gap: "var(--space-2)" }}>
          {phase === "running" ? (
            <button className="btn btn--danger" onClick={cancel}>
              Cancel export
            </button>
          ) : phase === "done" ? (
            <button className="btn btn--primary" onClick={onClose}>
              Done
            </button>
          ) : (
            <>
              <button className="btn btn--ghost" onClick={onClose}>
                Close
              </button>
              <button className="btn btn--primary" onClick={runExport}>
                Export{mappingActive ? " (with mapping)" : ""}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

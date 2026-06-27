import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import commands, {
  formatError,
  type FieldInference,
  type FieldMappingEntry,
  type ImportFormat,
  type ImportOptions,
  type ImportRequest,
  type ImportResult,
  type ImportSourceKind,
  type JsonImportShape,
  type PreviewImportResult,
} from "../../ipc/commands";
import { onImportExportProgress } from "../../ipc/events";
import {
  FieldMappingTable,
  discoveredFieldsFromInference,
  identityMapping,
  type DiscoveredField,
} from "./FieldMappingTable";
import {
  deleteTask,
  getTask,
  kindLabel,
  listTasks,
  saveTask,
  type ImportExportTaskSummary,
  type ImportTaskPayload,
} from "./importExportTasks";

export interface ImportWizardProps {
  connectionId: string;
  database: string;
  collection: string;
  onClose: () => void;
  onImported?: () => void;
}

type Phase = "config" | "preview" | "running" | "done" | "error";

export function ImportWizard({
  connectionId,
  database,
  collection,
  onClose,
  onImported,
}: ImportWizardProps) {
  const [format, setFormat] = useState<ImportFormat>("json");
  const [sourceKind, setSourceKind] = useState<ImportSourceKind>("file");
  const [path, setPath] = useState<string | null>(null);
  const [jsonShape, setJsonShape] = useState<JsonImportShape>("array");
  const [csvDelimiter, setCsvDelimiter] = useState(",");
  const [csvHeaders, setCsvHeaders] = useState(true);
  const [batchSize, setBatchSize] = useState(1000);

  // Field mapping: derived from the preview's inferred fields, then edited.
  const [showMapping, setShowMapping] = useState(false);
  const [discoveredFields, setDiscoveredFields] = useState<DiscoveredField[]>([]);
  const [mappingEntries, setMappingEntries] = useState<FieldMappingEntry[]>([]);

  // Saved tasks (localStorage).
  const [tasks, setTasks] = useState<ImportExportTaskSummary[]>([]);
  const [taskName, setTaskName] = useState("");
  const [taskMessage, setTaskMessage] = useState<string | null>(null);

  const [phase, setPhase] = useState<Phase>("config");
  const [processed, setProcessed] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const [preview, setPreview] = useState<PreviewImportResult | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const jobIdRef = useRef<string | null>(null);

  const refreshTasks = useCallback(() => {
    setTasks(listTasks(connectionId, "import"));
  }, [connectionId]);
  useEffect(() => {
    refreshTasks();
  }, [refreshTasks]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onImportExportProgress((p) => {
      if (p.jobId !== jobIdRef.current) return;
      setProcessed(p.processed);
      setTotal(p.total);
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  const chooseFile = useCallback(async () => {
    const extensions = format === "json" ? ["json", "ndjson"] : ["csv", "tsv"];
    const chosen = await open({
      multiple: false,
      directory: false,
      filters: [{ name: format.toUpperCase(), extensions }],
    });
    if (typeof chosen === "string") setPath(chosen);
  }, [format]);

  const buildRequest = useCallback(async (): Promise<ImportRequest> => {
    let clipboardText: string | null = null;
    if (sourceKind === "clipboard") {
      clipboardText = await navigator.clipboard.readText();
      if (!clipboardText.trim()) throw new Error("Clipboard is empty.");
    } else if (!path) {
      throw new Error("Choose a source file first.");
    }

    const fieldMapping =
      showMapping && mappingEntries.length > 0 ? mappingEntries : null;

    const options: ImportOptions = {
      jsonShape,
      csvDelimiter: format === "csv" ? csvDelimiter : null,
      csvHeaders,
      batchSize,
      previewRows: 20,
      fieldMapping,
    };

    return {
      connectionId,
      database,
      collection,
      jobId: jobIdRef.current ?? crypto.randomUUID(),
      source: {
        kind: sourceKind,
        path: sourceKind === "file" ? path : null,
        clipboardText,
      },
      format,
      options,
    };
  }, [
    batchSize,
    collection,
    connectionId,
    csvDelimiter,
    csvHeaders,
    database,
    format,
    jsonShape,
    path,
    sourceKind,
    showMapping,
    mappingEntries,
  ]);

  const runPreview = useCallback(async () => {
    setError(null);
    setMessage(null);
    try {
      // Preview never applies the field mapping — the user needs to see the
      // raw source fields first so they can build the mapping. The mapping is
      // applied only at run time.
      const request = await buildRequest();
      request.options.fieldMapping = null;
      const next = await commands.previewImport(request);
      setPreview(next);
      setPhase("preview");
      // Seed the mapping table from the inferred fields.
      const fields = discoveredFieldsFromInference(next.fields as FieldInference[]);
      setDiscoveredFields(fields);
      setMappingEntries(identityMapping(fields));
      setShowMapping(fields.length > 0);
    } catch (e) {
      setError(formatError(e));
      setPhase("error");
    }
  }, [buildRequest]);

  const runImport = useCallback(async () => {
    setError(null);
    setMessage(null);
    const jobId = crypto.randomUUID();
    jobIdRef.current = jobId;
    setProcessed(0);
    setTotal(null);
    setPhase("running");
    try {
      const request = await buildRequest();
      request.jobId = jobId;
      const next = await commands.runImport(request);
      setResult(next);
      if (next.cancelled) {
        setPhase("config");
        setMessage("Import cancelled.");
      } else {
        setPhase("done");
        setMessage(
          `Inserted ${next.inserted.toLocaleString()} document(s). ${next.errors.toLocaleString()} row error(s).`,
        );
        onImported?.();
      }
    } catch (e) {
      setError(formatError(e));
      setPhase("error");
    } finally {
      jobIdRef.current = null;
    }
  }, [buildRequest, onImported]);

  const cancel = useCallback(async () => {
    if (jobIdRef.current) {
      try {
        await commands.cancelImportExport(jobIdRef.current);
      } catch {
        // ignore, the backend job will finish or has already finished
      }
    }
  }, []);

  const saveCurrentTask = useCallback(() => {
    setTaskMessage(null);
    const name = taskName.trim();
    if (!name) {
      setTaskMessage("Enter a task name first.");
      return;
    }
    // Clipboard text isn't persisted; the task re-reads the clipboard at run.
    const payload: ImportTaskPayload = {
      kind: "import",
      database,
      collection,
      source: {
        kind: sourceKind,
        path: sourceKind === "file" ? path : null,
        clipboardText: null,
      },
      format,
      options: {
        jsonShape,
        csvDelimiter: format === "csv" ? csvDelimiter : null,
        csvHeaders,
        batchSize,
        previewRows: 20,
        fieldMapping: showMapping && mappingEntries.length > 0 ? mappingEntries : null,
      },
    };
    try {
      saveTask(connectionId, "import", name, payload);
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
    sourceKind,
    path,
    format,
    jsonShape,
    csvDelimiter,
    csvHeaders,
    batchSize,
    showMapping,
    mappingEntries,
    taskName,
    refreshTasks,
  ]);

  const loadTask = useCallback(
    (name: string) => {
      setTaskMessage(null);
      const entry = getTask(connectionId, "import", name);
      if (!entry || entry.payload.kind !== "import") {
        setTaskMessage(`Task "${name}" not found.`);
        return;
      }
      const p = entry.payload;
      setFormat(p.format);
      setSourceKind(p.source.kind);
      if (p.source.kind === "file" && p.source.path) setPath(p.source.path);
      setJsonShape(p.options.jsonShape);
      if (p.options.csvDelimiter != null) setCsvDelimiter(p.options.csvDelimiter);
      setCsvHeaders(p.options.csvHeaders);
      if (p.options.batchSize != null) setBatchSize(p.options.batchSize);
      const fm = p.options.fieldMapping ?? null;
      if (fm && fm.length > 0) {
        setShowMapping(true);
        setMappingEntries(fm);
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
      setPreview(null);
      setPhase("config");
      setTaskMessage(`Loaded task "${name}".`);
    },
    [connectionId],
  );

  const removeTask = useCallback(
    (name: string) => {
      deleteTask(connectionId, "import", name);
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
      aria-label="Import documents"
      onClick={(e) => {
        if (e.target === e.currentTarget && phase !== "running") onClose();
      }}
    >
      <div className="modal" style={{ width: "min(720px, 94vw)" }}>
        <div className="modal__header">
          <div className="modal__heading">
            <h2 className="modal__title">Import</h2>
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
          <div className="field">
            <span className="field__label">Format</span>
            <div className="row" style={{ gap: "var(--space-2)" }}>
              {(["json", "csv"] as ImportFormat[]).map((f) => (
                <button
                  key={f}
                  className={`btn btn--sm ${format === f ? "is-active" : ""}`}
                  onClick={() => {
                    setFormat(f);
                    setPreview(null);
                    setShowMapping(false);
                  }}
                  disabled={phase === "running"}
                  aria-pressed={format === f}
                >
                  {f.toUpperCase()}
                </button>
              ))}
            </div>
          </div>

          <div className="field">
            <span className="field__label">Source</span>
            <div className="row" style={{ gap: "var(--space-2)", flexWrap: "wrap" }}>
              {(["file", "clipboard"] as ImportSourceKind[]).map((kind) => (
                <button
                  key={kind}
                  className={`btn btn--sm ${sourceKind === kind ? "is-active" : ""}`}
                  onClick={() => setSourceKind(kind)}
                  disabled={phase === "running"}
                  aria-pressed={sourceKind === kind}
                >
                  {kind === "file" ? "File" : "Clipboard"}
                </button>
              ))}
              {sourceKind === "file" && (
                <button className="btn btn--sm" onClick={chooseFile} disabled={phase === "running"}>
                  Choose file…
                </button>
              )}
            </div>
            {sourceKind === "file" && path && (
              <span style={{ fontSize: 12, color: "var(--ink-muted)" }}>{path}</span>
            )}
          </div>

          {format === "json" ? (
            <div className="field">
              <span className="field__label">JSON shape</span>
              <div className="row" style={{ gap: "var(--space-2)", flexWrap: "wrap" }}>
                {(["object", "array", "ndjson"] as JsonImportShape[]).map((shape) => (
                  <button
                    key={shape}
                    className={`btn btn--sm ${jsonShape === shape ? "is-active" : ""}`}
                    onClick={() => setJsonShape(shape)}
                    disabled={phase === "running"}
                    aria-pressed={jsonShape === shape}
                  >
                    {shape === "object"
                      ? "Single object"
                      : shape === "array"
                        ? "JSON array"
                        : "NDJSON"}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <>
              <div className="row" style={{ gap: "var(--space-4)", alignItems: "end" }}>
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
                  <span style={{ fontSize: 13 }}>First row has headers</span>
                </label>
              </div>
            </>
          )}

          <div className="field">
            <span className="field__label">Batch size</span>
            <input
              className="field__input"
              type="number"
              min={1}
              max={10000}
              value={batchSize}
              onChange={(e) => setBatchSize(Number(e.target.value) || 1000)}
              disabled={phase === "running"}
              style={{ width: 120 }}
            />
          </div>

          {preview && (
            <div className="field">
              <span className="field__label">
                Preview: {preview.rows.length} row(s), {preview.errors.length} error(s)
              </span>
              <div style={{ maxHeight: 180, overflow: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius-md)" }}>
                <table className="results-grid" style={{ width: "100%" }}>
                  <thead>
                    <tr>
                      <th>Field</th>
                      <th>Type</th>
                      <th>Nullable</th>
                      <th>Samples</th>
                    </tr>
                  </thead>
                  <tbody>
                    {preview.fields.map((field) => (
                      <tr key={field.name}>
                        <td>{field.name}</td>
                        <td>{field.bsonType}</td>
                        <td>{field.nullable ? "Yes" : "No"}</td>
                        <td>{field.samples.join(", ")}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {preview.errors.length > 0 && (
                <div className="toast toast--warning" style={{ position: "static", margin: "var(--space-2) 0 0" }}>
                  First error: row {preview.errors[0].row ?? "?"}: {preview.errors[0].message}
                </div>
              )}
            </div>
          )}

          {preview && (
            <div className="field">
              <div className="row" style={{ gap: "var(--space-2)", alignItems: "center" }}>
                <span className="field__label" style={{ margin: 0 }}>Field mapping</span>
                {showMapping && (
                  <button
                    className="btn btn--sm btn--ghost"
                    onClick={() => setShowMapping(false)}
                    disabled={phase === "running"}
                  >
                    Hide
                  </button>
                )}
                {!showMapping && discoveredFields.length > 0 && (
                  <button
                    className="btn btn--sm"
                    onClick={() => setShowMapping(true)}
                    disabled={phase === "running"}
                  >
                    Show
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
                  Optional: rename, skip, or coerce fields before insert. The
                  mapping table becomes the inserted document schema; undeclared
                  fields are dropped.
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
                Importing… {processed.toLocaleString()}
                {total != null ? ` / ${total.toLocaleString()}` : ""}
                {pct != null ? ` (${pct}%)` : ""}
              </span>
              <div style={{ height: 6, borderRadius: "var(--radius-sm)", background: "var(--surface-3)", overflow: "hidden" }}>
                <div style={{ height: "100%", width: pct != null ? `${pct}%` : "100%", background: "var(--accent-500)", transition: "width 200ms ease-out" }} />
              </div>
            </div>
          )}

          {message && phase !== "running" && (
            <div className="toast toast--success" style={{ position: "static", margin: 0 }}>
              {message}
            </div>
          )}
          {result && result.rowErrors.length > 0 && phase === "done" && (
            <div className="toast toast--warning" style={{ position: "static", margin: 0 }}>
              First row error: row {result.rowErrors[0].row ?? "?"}: {result.rowErrors[0].message}
            </div>
          )}
          {error && (
            <div className="toast toast--error" style={{ position: "static", margin: 0 }}>
              {error}
            </div>
          )}
        </div>

        <div className="modal__footer row row--end" style={{ gap: "var(--space-2)" }}>
          {phase === "running" ? (
            <button className="btn btn--danger" onClick={cancel}>
              Cancel import
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
              <button className="btn" onClick={runPreview}>
                Preview
              </button>
              <button className="btn btn--primary" onClick={runImport}>
                Import{mappingActive ? " (with mapping)" : ""}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import commands, { formatError, type CompressionFormat, type DumpResult, type DatabaseSummary, type ScheduleConfig } from "../../ipc/commands";
import { Alert } from "../../components/Alert";
import { useToast } from "../../context/ToastContext";
import { CollectionCheckList, type CollectionItem } from "./CollectionCheckList";
import { SchedulePanel } from "./SchedulePanel";

export interface DumpWizardProps {
  connectionId: string;
  database?: string;
  collections?: CollectionItem[];
  onClose: () => void;
  onDumped?: () => void;
}

type Phase = "config" | "running" | "done" | "error";
type DumpFormat = "bson" | "json";

export function DumpWizard({ connectionId, database: initialDatabase = "", collections: initialCollections = [], onClose, onDumped }: DumpWizardProps) {
  const [databases, setDatabases] = useState<DatabaseSummary[]>([]);
  const [database, setDatabase] = useState(initialDatabase);
  const [collections, setCollections] = useState<CollectionItem[]>(initialCollections);
  const [selected, setSelected] = useState<string[]>(initialCollections.map((c) => c.name));
  const [destination, setDestination] = useState<string | null>(null);
  const [format, setFormat] = useState<DumpFormat>("bson");
  const [compression, setCompression] = useState<CompressionFormat>("gzip");
  const [pathTemplate, setPathTemplate] = useState("${collection}");
  const [schedule, setSchedule] = useState<ScheduleConfig | null>(null);
  const [phase, setPhase] = useState<Phase>("config");
  const [result, setResult] = useState<DumpResult | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [loadingDb, setLoadingDb] = useState(!initialDatabase);
  const [loadingColl, setLoadingColl] = useState(!!initialDatabase && initialCollections.length === 0);
  const toast = useToast();

  // If no database provided, fetch the list.
  useEffect(() => {
    if (initialDatabase) return;
    let cancelled = false;
    commands.listDatabases(connectionId)
      .then((dbs) => { if (!cancelled) setDatabases(dbs); })
      .catch((e) => { if (!cancelled) setErrorMsg(formatError(e)); })
      .finally(() => { if (!cancelled) setLoadingDb(false); });
    return () => { cancelled = true; };
  }, [connectionId, initialDatabase]);

  // When database is known but collections weren't provided, fetch them.
  useEffect(() => {
    if (!database || initialCollections.length > 0) return;
    let cancelled = false;
    setLoadingColl(true);
    commands.listCollections(connectionId, database)
      .then((cols) => {
        if (cancelled) return;
        const items: CollectionItem[] = cols.map((c) => ({
          name: c.name,
          documentCount: c.documentCount,
          sizeBytes: c.sizeBytes,
        }));
        setCollections(items);
        setSelected(items.map((c) => c.name));
      })
      .catch((e) => { if (!cancelled) setErrorMsg(formatError(e)); })
      .finally(() => { if (!cancelled) setLoadingColl(false); });
    return () => { cancelled = true; };
  }, [connectionId, database, initialCollections.length]);

  const pickDir = useCallback(async () => {
    const chosen = await open({ directory: true });
    if (typeof chosen === "string") {
      setDestination(chosen);
    }
  }, []);

  const runDump = useCallback(async () => {
    if (!destination) {
      setErrorMsg("Choose a destination directory first.");
      return;
    }
    if (selected.length === 0) {
      setErrorMsg("Select at least one collection.");
      return;
    }
    setErrorMsg(null);
    setPhase("running");
    const jobId = crypto.randomUUID();
    try {
      const res = await commands.dumpDatabase({
        connectionId,
        database,
        collections: selected,
        destinationDir: destination,
        pathTemplate,
        format,
        compression,
        jobId,
      });
      if (schedule) {
        const tmpl = await commands.updateSchedule({
          jobId: res.jobId,
          cron: schedule.cron,
          enabled: schedule.enabled,
          retentionCount: schedule.retentionCount,
        });
        if (tmpl.schedule?.enabled && tmpl.schedule.nextRunAt) {
          toast.push(
            `Schedule saved — next run ${new Date(tmpl.schedule.nextRunAt).toLocaleString()}.`,
            "success",
          );
        }
      }
      setResult(res);
      setPhase("done");
      toast.push(
        `Dumped ${res.processed.toLocaleString()} document(s) to ${destination}.`,
        "success",
      );
      onDumped?.();
    } catch (e) {
      const msg = formatError(e);
      setErrorMsg(msg);
      setPhase("error");
      toast.push(msg, "error");
    }
  }, [connectionId, database, selected, destination, pathTemplate, format, compression, schedule, toast]);

  const isReady = database && !loadingColl && collections.length > 0;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" style={{ width: 560 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          <h3 className="modal__title">Dump database</h3>
          <button className="modal__close" onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>

        <div className="modal__body" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          {phase === "config" && (
            <>
              {errorMsg && <Alert tone="danger">{errorMsg}</Alert>}

              {!initialDatabase && (
                <div className="field">
                  <label className="field__label">Database</label>
                  {loadingDb ? (
                    <p className="field__hint">Loading databases…</p>
                  ) : databases.length === 0 ? (
                    <Alert tone="warning">No databases found on this connection.</Alert>
                  ) : (
                    <select
                      className="field__select"
                      value={database}
                      onChange={(e) => {
                        setDatabase(e.target.value);
                        setCollections([]);
                        setSelected([]);
                      }}
                    >
                      <option value="">Choose a database…</option>
                      {databases.map((db) => (
                        <option key={db.name} value={db.name}>{db.name}</option>
                      ))}
                    </select>
                  )}
                </div>
              )}

              {initialDatabase && (
                <div className="field">
                  <label className="field__label">Target database</label>
                  <input className="field__input" value={database} disabled />
                </div>
              )}

              <div className="field">
                <label className="field__label">Destination directory</label>
                <div style={{ display: "flex", gap: 8 }}>
                  <input
                    className="field__input"
                    value={destination ?? ""}
                    placeholder="Choose a directory…"
                    readOnly
                    style={{ flex: 1 }}
                  />
                  <button className="btn btn--ghost" onClick={pickDir}>
                    Browse…
                  </button>
                </div>
              </div>

              <div className="field">
                <label className="field__label">Format</label>
                <div className="row" style={{ gap: "var(--space-2)" }}>
                  {(["bson", "json"] as DumpFormat[]).map((f) => (
                    <button
                      key={f}
                      className={`btn btn--sm ${format === f ? "is-active" : ""}`}
                      onClick={() => setFormat(f)}
                      aria-pressed={format === f}
                    >
                      {f.toUpperCase()}
                    </button>
                  ))}
                </div>
              </div>

              <div className="field">
                <label className="field__label">Compression</label>
                <select
                  className="field__select"
                  value={compression}
                  onChange={(e) => setCompression(e.target.value as CompressionFormat)}
                >
                  <option value="none">None</option>
                  <option value="gzip">Gzip</option>
                  <option value="zstd">Zstd</option>
                </select>
              </div>

              <div className="field">
                <label className="field__label">Filename template</label>
                <input
                  className="field__input"
                  value={pathTemplate}
                  onChange={(e) => setPathTemplate(e.target.value)}
                  placeholder="${collection}"
                />
                <p className="field__hint">
                  Tokens: {"${db}"}, {"${collection}"}, {"${date}"}, {"${time}"}, {"${profile}"}
                </p>
              </div>

              <SchedulePanel value={schedule} onChange={setSchedule} />

              {database && (
                <div>
                  <label className="field__label">Collections</label>
                  {loadingColl ? (
                    <p className="field__hint">Loading collections…</p>
                  ) : collections.length === 0 ? (
                    <Alert tone="warning">No collections found in {database}.</Alert>
                  ) : (
                    <CollectionCheckList
                      items={collections}
                      selected={selected}
                      onChange={setSelected}
                    />
                  )}
                </div>
              )}
            </>
          )}

          {phase === "running" && (
            <div style={{ textAlign: "center", padding: "32px 0" }}>
              <div className="job-progress-track" style={{ marginBottom: 12 }}>
                <span
                  className="job-progress-fill"
                  style={{ width: "100%", animation: "pulse 1.5s infinite" }}
                />
              </div>
              <p>Dumping {selected.length} collection(s) from {database}…</p>
              <p className="field__hint">This may take a while for large collections.</p>
            </div>
          )}

          {phase === "done" && result && (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <Alert tone="success">
                Dumped {result.processed.toLocaleString()} document(s) from {selected.length}{" "}
                collection(s).
              </Alert>
              {result.files.length > 0 && (
                <div>
                  <label className="field__label">Files created</label>
                  <ul className="job-meta-list">
                    {result.files.map((f) => (
                      <li key={f}>
                        <code className="job-meta-code">{f}</code>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          )}

          {phase === "error" && errorMsg && <Alert tone="danger">{errorMsg}</Alert>}
        </div>

        <div className="modal__footer">
          {phase === "config" && (
            <>
              <button className="btn btn--ghost" onClick={onClose}>
                Cancel
              </button>
              <button
                className="btn btn--primary"
                onClick={runDump}
                disabled={!isReady || !destination || selected.length === 0}
              >
                Dump {selected.length} collection(s)
              </button>
            </>
          )}
          {(phase === "done" || phase === "error") && (
            <button className="btn btn--primary" onClick={onClose}>
              Close
            </button>
          )}
          {phase === "running" && (
            <button className="btn btn--ghost" disabled>
              Running…
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

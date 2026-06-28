import { useCallback, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import commands, { formatError, type RestoreResult } from "../../ipc/commands";
import { Alert } from "../../components/Alert";
import { Modal } from "../../components/Modal";
import { useToast } from "../../context/ToastContext";
import { CollectionMappingTable } from "./CollectionMappingTable";
import { InfoPopover } from "../../components/InfoPopover";

export interface RestoreWizardProps {
  connectionId: string;
  onClose: () => void;
  onRestored?: () => void;
}

type Phase = "config" | "preview" | "running" | "done" | "error";

export function RestoreWizard({ connectionId, onClose, onRestored }: RestoreWizardProps) {
  const [sourceDir, setSourceDir] = useState<string | null>(null);
  const [targetDatabase, setTargetDatabase] = useState("");
  const [createDatabase, setCreateDatabase] = useState(true);
  const [conflictStrategy, setConflictStrategy] = useState<"drop" | "skip" | "upsert">("drop");
  const [mappings, setMappings] = useState<{ source: string; target: string; enabled: boolean }[]>([]);
  const [phase, setPhase] = useState<Phase>("config");
  const [result, setResult] = useState<RestoreResult | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const toast = useToast();

  const pickDir = useCallback(async () => {
    const chosen = await open({ directory: true });
    if (typeof chosen === "string") {
      setSourceDir(chosen);
      setErrorMsg(null);
    }
  }, []);

  const runPreview = useCallback(async () => {
    if (!sourceDir) {
      setErrorMsg("Choose a source directory first.");
      return;
    }
    setErrorMsg(null);
    try {
      const preview = await commands.previewArchive(sourceDir);
      setMappings(
        preview.map((p) => ({
          source: p.sourceName,
          target: p.targetName,
          enabled: true,
        })),
      );
      setPhase("preview");
    } catch (e) {
      setErrorMsg(formatError(e));
    }
  }, [sourceDir]);

  const runRestore = useCallback(async () => {
    if (!sourceDir || !targetDatabase.trim()) {
      setErrorMsg("Source directory and target database are required.");
      return;
    }
    const enabled = mappings.filter((m) => m.enabled);
    if (enabled.length === 0) {
      setErrorMsg("Select at least one collection to restore.");
      return;
    }
    setErrorMsg(null);
    setPhase("running");
    const jobId = crypto.randomUUID();
    try {
      const res = await commands.restoreDatabase({
        connectionId,
        sourceDir,
        targetDatabase: targetDatabase.trim(),
        createDatabase,
        collectionMap: enabled,
        conflictStrategy,
        jobId,
      });
      setResult(res);
      setPhase("done");
      toast.push(
        `Restored ${res.inserted.toLocaleString()} document(s) into ${targetDatabase.trim()}.`,
        "success",
      );
      onRestored?.();
    } catch (e) {
      const msg = formatError(e);
      setErrorMsg(msg);
      setPhase("error");
      toast.push(msg, "error");
    }
  }, [connectionId, sourceDir, targetDatabase, createDatabase, mappings, conflictStrategy, toast]);

  const footer = (
    <>
      {phase === "config" && (
        <>
          <button className="btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn--primary" onClick={runPreview} disabled={!sourceDir}>
            Preview archive
          </button>
        </>
      )}
      {phase === "preview" && (
        <>
          <button className="btn btn--ghost" onClick={() => setPhase("config")}>
            Back
          </button>
          <button
            className="btn btn--primary"
            onClick={runRestore}
            disabled={!targetDatabase.trim() || mappings.filter((m) => m.enabled).length === 0}
          >
            Restore {mappings.filter((m) => m.enabled).length} collection(s)
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
    </>
  );

  return (
    <Modal open title="Restore database" onClose={onClose} footer={footer} width={600}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {errorMsg && <Alert tone="danger">{errorMsg}</Alert>}

        {phase === "config" && (
          <>
            <div className="field">
              <label className="field__label">Source directory</label>
              <div style={{ display: "flex", gap: 8 }}>
                <input
                  className="field__input"
                  value={sourceDir ?? ""}
                  placeholder="Choose a directory containing .bson files..."
                  readOnly
                  style={{ flex: 1 }}
                />
                <button className="btn btn--ghost" onClick={pickDir}>
                  Browse…
                </button>
              </div>
            </div>
            <div className="field">
              <label className="field__label">Target database</label>
              <input
                className="field__input"
                value={targetDatabase}
                onChange={(e) => setTargetDatabase(e.target.value)}
                placeholder="database_name"
              />
            </div>
            <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
              <input
                type="checkbox"
                checked={createDatabase}
                onChange={(e) => setCreateDatabase(e.target.checked)}
              />
              <span className="field__label" style={{ margin: 0 }}>
                Create database if it does not exist
              </span>
            </label>
          </>
        )}

        {phase === "preview" && (
          <>
            <div className="field">
              <label className="field__label">Conflict strategy <InfoPopover label="Conflict strategy help" title="Conflict strategy">
              <p><strong>Drop</strong>: deletes existing collections before restoring.</p>
              <p><strong>Skip</strong>: keeps existing data, only restores missing collections.</p>
              <p><strong>Upsert</strong>: merges data by updating existing and inserting new documents.</p>
            </InfoPopover></label>
              <select
                className="field__select"
                value={conflictStrategy}
                onChange={(e) => setConflictStrategy(e.target.value as "drop" | "skip" | "upsert")}
              >
                <option value="drop">Drop and replace existing collections</option>
                <option value="skip">Skip existing collections</option>
                <option value="upsert">Upsert (drop and replace for now)</option>
              </select>
            </div>
            <div>
              <label className="field__label">Collection mapping</label>
              <CollectionMappingTable mappings={mappings} onChange={setMappings} />
            </div>
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
            <p>Restoring into {targetDatabase}…</p>
            <p className="field__hint">This may take a while for large collections.</p>
          </div>
        )}

        {phase === "done" && result && (
          <Alert tone="success">
            Restored {result.inserted.toLocaleString()} document(s) into {targetDatabase}.
            {result.errors > 0 && ` ${result.errors.toLocaleString()} error(s) occurred.`}
          </Alert>
        )}

        {phase === "error" && errorMsg && <Alert tone="danger">{errorMsg}</Alert>}
      </div>
    </Modal>
  );
}

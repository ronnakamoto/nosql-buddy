import { useEffect, useRef, useState } from "react";
import { Modal } from "./Modal";
import { useToast } from "../context/ToastContext";
import commands from "../ipc/commands";

export interface InsertDocumentModalProps {
  open: boolean;
  connectionId: string;
  database: string;
  collection: string;
  onClose: () => void;
  /** Called after a successful insert. The argument is the new `_id`. */
  onInserted: (insertedId: string) => void;
  /** Called on a failure so the parent can surface the error. */
  onError: (message: string) => void;
}

const DEFAULT_BODY = '{\n  "name": "Untitled"\n}';

/**
 * Modal that lets the user paste a JSON document and insert it into
 * the active collection via the `insert_document` IPC command. The
 * textarea contents are parsed and the modal stays open with an
 * inline error if parsing fails, so the user can correct the JSON.
 */
export function InsertDocumentModal({
  open,
  connectionId,
  database,
  collection,
  onClose,
  onInserted,
  onError,
}: InsertDocumentModalProps) {
  const [body, setBody] = useState(DEFAULT_BODY);
  const [submitting, setSubmitting] = useState(false);
  const toast = useToast();
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Reset when the modal is opened so the previous draft does not leak.
  useEffect(() => {
    if (open) {
      setBody(DEFAULT_BODY);
      setSubmitting(false);
    }
  }, [open]);

  if (!open) return null;

  async function handleInsert() {
    let parsed: unknown;
    try {
      parsed = JSON.parse(body);
    } catch (e) {
      toast.push(`Invalid JSON: ${describeError(e)}`, "error");
      return;
    }
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      toast.push("Document must be a JSON object.", "error");
      return;
    }
    setSubmitting(true);
    try {
      const id = await commands.insertDocument({
        connectionId,
        database,
        collection,
        documentJson: JSON.stringify(parsed),
      });
      onInserted(id);
      onClose();
    } catch (e) {
      const msg = describeError(e);
      toast.push(msg, "error");
      onError(msg);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Modal
      open={open}
      title={`Insert document into ${database}.${collection}`}
      onClose={submitting ? () => undefined : onClose}
    >
      <div style={{ display: "grid", gap: "var(--space-3)" }}>
        <p style={{ color: "var(--ink-muted)", fontSize: 13, margin: 0 }}>
          Paste a JSON object. The <code>_id</code> field is optional — Mongo
          will assign one if omitted.
        </p>
        <textarea
          ref={textareaRef}
          value={body}
          onChange={(e) => setBody(e.target.value)}
          spellCheck={false}
          rows={12}
          aria-label="Document JSON"
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: 12,
            padding: "var(--space-2)",
            border: "1px solid var(--border)",
            borderRadius: 4,
            background: "var(--surface-2)",
            color: "var(--ink)",
            resize: "vertical",
          }}
        />
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button
            className="btn btn--sm"
            onClick={onClose}
            disabled={submitting}
          >
            Cancel
          </button>
          <button
            className="btn btn--primary btn--sm"
            onClick={() => void handleInsert()}
            disabled={submitting}
          >
            {submitting ? "Inserting…" : "Insert"}
          </button>
        </div>
      </div>
    </Modal>
  );
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

import { useEffect, useRef, useState } from "react";
import { Modal } from "./Modal";
import { useToast } from "../context/ToastContext";
import commands, { formatError } from "../ipc/commands";

export interface InsertDocumentModalProps {
  open: boolean;
  connectionId: string;
  database: string;
  collection: string;
  /** When true, accept a JSON array of documents and call insertMany. */
  many?: boolean;
  onClose: () => void;
  /** Called after a successful insert. The argument is the new `_id` or comma-separated ids. */
  onInserted: (insertedId: string) => void;
  /** Called on a failure so the parent can surface the error. */
  onError: (message: string) => void;
}

const DEFAULT_BODY = '{\n  "name": "Untitled"\n}';
const DEFAULT_BODY_MANY = '[\n  {\n    "name": "Untitled"\n  }\n]';

/**
 * Modal that lets the user paste a JSON document and insert it into
 * the active collection via the `insert_document` or `insert_many_documents`
 * IPC command. The textarea contents are parsed and the modal stays open
 * with an inline error if parsing fails, so the user can correct the JSON.
 */
export function InsertDocumentModal({
  open,
  connectionId,
  database,
  collection,
  many = false,
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
      setBody(many ? DEFAULT_BODY_MANY : DEFAULT_BODY);
      setSubmitting(false);
    }
  }, [open, many]);

  if (!open) return null;

  async function handleInsert() {
    let parsed: unknown;
    try {
      parsed = JSON.parse(body);
    } catch (e) {
      toast.push(`Invalid JSON: ${describeError(e)}`, "error");
      return;
    }
    setSubmitting(true);
    try {
      if (many) {
        if (!Array.isArray(parsed)) {
          toast.push("Insert many requires a JSON array of objects.", "error");
          return;
        }
        if (parsed.length === 0) {
          toast.push("Array must not be empty.", "error");
          return;
        }
        const result = await commands.insertManyDocuments({
          connectionId,
          database,
          collection,
          documentsJson: JSON.stringify(parsed),
        });
        onInserted(result.insertedIds.join(", "));
        onClose();
      } else {
        if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
          toast.push("Document must be a JSON object.", "error");
          return;
        }
        const id = await commands.insertDocument({
          connectionId,
          database,
          collection,
          documentJson: JSON.stringify(parsed),
        });
        onInserted(id);
        onClose();
      }
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
      title={`Insert ${many ? "documents" : "document"} into ${database}.${collection}`}
      onClose={submitting ? () => undefined : onClose}
    >
      <div style={{ display: "grid", gap: "var(--space-3)" }}>
        <p style={{ color: "var(--ink-muted)", fontSize: 13, margin: 0 }}>
          Paste a JSON {many ? "array of objects" : "object"}. The <code>_id</code> field is optional — Mongo
          will assign one if omitted.
        </p>
        <textarea
          ref={textareaRef}
          value={body}
          onChange={(e) => setBody(e.target.value)}
          spellCheck={false}
          rows={12}
          aria-label={many ? "Documents JSON" : "Document JSON"}
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
            {submitting ? "Inserting…" : many ? "Insert Many" : "Insert"}
          </button>
        </div>
      </div>
    </Modal>
  );
}

function describeError(e: unknown): string {
  return formatError(e);
}

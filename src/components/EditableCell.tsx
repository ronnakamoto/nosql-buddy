import { useEffect, useRef, useState } from "react";
import commands from "../ipc/commands";
import { detectKind, displayValue, getByPath, kindClassName, toFilterId } from "./resultsDisplay";

export interface EditableCellProps {
  /** The original row this cell belongs to. Used to derive `_id`. */
  row: Record<string, unknown>;
  /** Column name (top-level or dotted path like `address.city`). */
  fieldPath: string;
  /** Current value to render / edit. */
  value: unknown;
  /** Connection / database / collection used for the update IPC call. */
  connectionId: string;
  database: string;
  collection: string;
  /** Called when a save succeeds so the parent can refresh results. */
  onSaved: (newValue: unknown) => void;
  /** Called when a save fails so the parent can toast the error. */
  onError: (message: string) => void;
}

/**
 * Click-to-edit cell. Clicking turns the value into an editable
 * textarea (JSON for objects/arrays, raw text for scalars). On save
 * (Cmd/Ctrl+Enter or the Save button) the value is parsed and sent
 * as `{ $set: { <fieldPath>: <parsed> } }` with filter `{ _id: <docId> }`
 * to the `update_documents` IPC command.
 *
 * For dotted `fieldPath`, the value is read from / written to the
 * nested object via intermediate keys.
 */
export function EditableCell({
  row,
  fieldPath,
  value,
  connectionId,
  database,
  collection,
  onSaved,
  onError,
}: EditableCellProps) {
  const resolvedValue = value !== undefined ? value : getByPath(row, fieldPath);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<string>(() => defaultDraft(resolvedValue));
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  if (editing) {
    return (
      <span className="editable-cell-edit">
        <textarea
          ref={inputRef}
          className="editable-cell-textarea"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              void handleSave();
            } else if (e.key === "Escape") {
              e.preventDefault();
              setEditing(false);
              setDraft(defaultDraft(resolvedValue));
            }
          }}
          rows={Math.min(8, Math.max(2, draft.split("\n").length))}
          spellCheck={false}
          aria-label={`Edit ${fieldPath}`}
        />
        <span className="editable-cell-actions">
          <button
            className="editable-cell-button editable-cell-button--save"
            onClick={() => void handleSave()}
            disabled={saving}
          >
            {saving ? "Saving…" : "Save"}
          </button>
          <button
            className="editable-cell-button"
            onClick={() => {
              setEditing(false);
              setDraft(defaultDraft(resolvedValue));
            }}
            disabled={saving}
          >
            Cancel
          </button>
        </span>
      </span>
    );
  }

  const kind = detectKind(resolvedValue);
  const isNumeric = ["int", "long", "double", "decimal"].includes(kind);
  const valueText = displayValue(resolvedValue);
  return (
    <button
      type="button"
      className={`editable-cell ${isNumeric ? "editable-cell--numeric" : ""}`}
      onClick={() => {
        setDraft(defaultDraft(resolvedValue));
        setEditing(true);
      }}
      title={`Click to edit ${fieldPath}`}
    >
      <span className={`kind-badge ${kindClassName(kind)}`}>{kind}</span>
      <span className="editable-cell__value" title={valueText}>
        {valueText}
      </span>
    </button>
  );

  async function handleSave() {
    const parsed = parseDraft(draft, value);
    if (parsed.error) {
      onError(parsed.error);
      return;
    }
    const docId = readId(row);
    if (docId === undefined) {
      onError("Cannot edit a document without an `_id`.");
      return;
    }
    setSaving(true);
    try {
      const result = await commands.updateDocuments({
        connectionId,
        database,
        collection,
        filterJson: JSON.stringify({ _id: docId }),
        updateJson: JSON.stringify({ $set: { [fieldPath]: parsed.value } }),
        multi: false,
        upsert: false,
      });
      if (result.matchedCount === 0) {
        // No document matched the filter. The most common cause is the
        // `_id` not round-tripping through display JSON; `toFilterId`
        // handles the known cases, so reaching here means a type we
        // don't reconstruct yet. Surface it loudly rather than silently
        // showing a false "Saved" toast.
        onError(
          "No document matched the filter — the `_id` may not have round-tripped. Nothing was saved.",
        );
        return;
      }
      onSaved(parsed.value);
      setEditing(false);
    } catch (e) {
      onError(describeError(e));
    } finally {
      setSaving(false);
    }
  }
}

function defaultDraft(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "object") return JSON.stringify(value, null, 2);
  return String(value);
}

function parseDraft(
  draft: string,
  original: unknown,
): { value: unknown; error?: undefined } | { value: null; error: string } {
  // For scalar originals we coerce. For object/array originals we
  // require valid JSON.
  const wasObject = typeof original === "object" && original !== null;
  if (wasObject) {
    try {
      return { value: JSON.parse(draft) };
    } catch (e) {
      return { value: null, error: `Invalid JSON: ${describeError(e)}` };
    }
  }
  // Scalar coercion. Empty string is a string; bare `null` becomes null;
  // bare `true`/`false` become booleans; bare numbers become numbers.
  const trimmed = draft.trim();
  if (trimmed === "null") return { value: null };
  if (trimmed === "true") return { value: true };
  if (trimmed === "false") return { value: false };
  if (trimmed !== "" && !Number.isNaN(Number(trimmed)) && /^-?\d+(\.\d+)?$/.test(trimmed)) {
    return { value: Number(trimmed) };
  }
  // Fall back to string (no quoting needed; the textarea content is the string).
  return { value: draft };
}

/**
 * Read the document's `_id` and reconstruct it into MongoDB Extended
 * JSON form so it round-trips through the backend filter parser.
 *
 * `find_documents` returns `_id` in *display* form (e.g.
 * `{ _idDisplay: "hex" }` for ObjectIds) — sending that back as a
 * filter would match nothing. `toFilterId` rebuilds `{ $oid: "hex" }`
 * (and analogous forms for Date/Decimal/Binary) so the update filter
 * actually targets the right document.
 */
function readId(row: Record<string, unknown>): unknown {
  if (row._id === undefined) return undefined;
  return toFilterId(row);
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

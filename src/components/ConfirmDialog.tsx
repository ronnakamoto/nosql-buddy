import { useEffect, useRef } from "react";
import { AlertTriangle } from "lucide-react";

export interface ConfirmDialogProps {
  open: boolean;
  /** Short heading, e.g. "Drop index?" */
  title: string;
  /** One or two sentences describing what will happen and why it cannot be undone. */
  description: string;
  /** Label for the confirm button. Should be verb + object: "Delete document", "Drop index". */
  confirmLabel?: string;
  /** Label for the cancel button. Defaults to "Cancel". */
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  /** Extra detail shown in a monospace block (e.g. index name, document id). Optional. */
  detail?: string;
}

/**
 * A focused confirmation dialog for destructive actions. Uses the app's
 * existing `.modal` / `.modal-backdrop` styles; adds a danger icon, a
 * danger-tinted confirm button, and keyboard focus management so users
 * can cancel with Escape without touching the mouse.
 *
 * Cancel is the default keyboard action (focused on open). Confirm
 * requires an explicit click or Enter on the confirm button.
 */
export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel = "Delete",
  cancelLabel = "Cancel",
  onConfirm,
  onCancel,
  detail,
}: ConfirmDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  // Focus cancel on open; close on Escape.
  useEffect(() => {
    if (!open) return;
    cancelRef.current?.focus();
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onCancel]);

  if (!open) return null;

  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-dialog-title"
      aria-describedby="confirm-dialog-desc"
      onClick={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div className="modal confirm-dialog" style={{ width: "min(400px, 92vw)" }}>
        <div className="confirm-dialog__header">
          <span className="confirm-dialog__icon" aria-hidden="true">
            <AlertTriangle size={18} />
          </span>
          <h2 className="confirm-dialog__title" id="confirm-dialog-title">
            {title}
          </h2>
        </div>
        <div className="confirm-dialog__body">
          <p id="confirm-dialog-desc" className="confirm-dialog__desc">
            {description}
          </p>
          {detail && (
            <code className="confirm-dialog__detail">{detail}</code>
          )}
        </div>
        <div className="confirm-dialog__footer">
          <button
            ref={cancelRef}
            className="btn btn--sm"
            type="button"
            onClick={onCancel}
          >
            {cancelLabel}
          </button>
          <button
            ref={confirmRef}
            className="btn btn--sm btn--danger-filled"
            type="button"
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

import { useEffect, useRef } from "react";

export interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  footer?: React.ReactNode;
  width?: number;
}

export function Modal({ open, title, onClose, children, footer, width }: ModalProps) {
  const backdropRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  // Close when the user clicks directly on the backdrop (not on a child).
  // Use mousedown + a target check so that interactions that start inside the
  // modal (e.g. an input's autocomplete popup landing outside the modal DOM)
  // do not cause a spurious close.
  const handleBackdropMouseDown = (e: React.MouseEvent) => {
    if (e.target === backdropRef.current) {
      e.preventDefault();
      onClose();
    }
  };

  if (!open) return null;
  return (
    <div
      className="modal-backdrop"
      ref={backdropRef}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onMouseDown={handleBackdropMouseDown}
    >
      <div
        className="modal"
        style={width ? { width: `min(${width}px, 92vw)` } : undefined}
      >
        <div className="modal__header">
          <div className="modal__heading">
            <h2 className="modal__title">{title}</h2>
          </div>
          <button
            className="modal__close"
            onClick={onClose}
            aria-label="Close"
            type="button"
          >
            ×
          </button>
        </div>
        <div className="modal__body">{children}</div>
        {footer && <div className="modal__footer">{footer}</div>}
      </div>
    </div>
  );
}

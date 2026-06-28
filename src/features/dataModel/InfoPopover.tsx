import { useCallback, useEffect, useRef, useState } from "react";
import { HelpCircle } from "lucide-react";

export interface InfoPopoverProps {
  label: string;
  title: string;
  children: React.ReactNode;
}

/**
 * A small click-to-open help popover anchored to a help-icon button. Renders
 * with `position: fixed` so it escapes `overflow` clipping in dense toolbars,
 * matching the placement strategy used by `DatabaseRowMenu` and `NewTabMenu`.
 */
export function InfoPopover({ label, title, children }: InfoPopoverProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number | null; right: number | null }>({
    top: 0,
    left: null,
    right: null,
  });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  const place = useCallback(() => {
    const btn = triggerRef.current;
    if (!btn) return;
    const r = btn.getBoundingClientRect();
    const width = 320;
    const gap = 6;
    const top = r.bottom + gap;
    if (r.left + width > window.innerWidth - 8) {
      setPos({ top, left: null, right: window.innerWidth - r.right });
    } else {
      setPos({ top, left: r.left, right: null });
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    place();
    const onDown = (e: MouseEvent) => {
      if (popoverRef.current?.contains(e.target as Node)) return;
      if (triggerRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onResize = () => place();
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onResize);
    };
  }, [open, place]);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="info-popover__trigger"
        aria-label={label}
        aria-expanded={open}
        onClick={() => {
          if (!open) place();
          setOpen((o) => !o);
        }}
      >
        <HelpCircle size={14} />
      </button>
      {open && (
        <div
          ref={popoverRef}
          className="info-popover"
          role="dialog"
          aria-label={title}
          style={{
            position: "fixed",
            top: pos.top,
            left: pos.left ?? undefined,
            right: pos.right ?? undefined,
          }}
        >
          <div className="info-popover__title">{title}</div>
          <div className="info-popover__body">{children}</div>
        </div>
      )}
    </>
  );
}

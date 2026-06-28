import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
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
 *
 * Boundary detection: if the popover would overflow the viewport bottom or right,
 * it is flipped above the trigger or shifted left so it stays fully visible.
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

  // After the popover renders, measure its actual dimensions and flip/shift
  // if it would overflow the viewport.
  useLayoutEffect(() => {
    if (!open) return;
    const raf = requestAnimationFrame(() => {
      const btn = triggerRef.current;
      const popover = popoverRef.current;
      if (!btn || !popover) return;

      const pRect = popover.getBoundingClientRect();
      const bRect = btn.getBoundingClientRect();
      const gap = 6;
      const margin = 8;
      const viewportW = window.innerWidth;
      const viewportH = window.innerHeight;

      let top = pRect.top;
      let left: number | null = pos.left ?? null;
      let right: number | null = pos.right ?? null;

      // Flip vertically if below viewport
      if (pRect.bottom > viewportH - margin) {
        top = bRect.top - pRect.height - gap;
        // Safety: if flipped above is off-screen, clamp to top margin
        top = Math.max(margin, top);
      }

      // Flip horizontally if right of viewport
      if (pRect.right > viewportW - margin) {
        right = margin;
        left = null;
      }
      // Shift right if left is off-screen
      if (pRect.left < margin && left !== null) {
        left = margin;
      }

      setPos({ top, left, right });
    });
    return () => cancelAnimationFrame(raf);
  }, [open, pos.left, pos.right]);

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

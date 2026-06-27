import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { Info, CheckCircle, AlertTriangle, AlertCircle, X, Copy, Check } from "lucide-react";

export type ToastKind = "info" | "success" | "warning" | "error";

export interface ToastMessage {
  id: number;
  text: string;
  kind: ToastKind;
  durationMs: number;
  title?: string;
  leaving?: boolean;
}

const DEFAULT_DURATION = 4000;

/**
 * Toast state controller. Keeps the legacy `push(text, kind, durationMs)`
 * signature for existing callers and adds `pushToast` for richer toasts
 * with a title. `dismiss` marks a toast as leaving (so it can play an exit
 * animation); the purge effect removes it from state once the exit completes.
 */
export function useToasts() {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((current) =>
      current.map((t) => (t.id === id && !t.leaving ? { ...t, leaving: true } : t)),
    );
  }, []);

  const push = useCallback(
    (text: string, kind: ToastKind = "info", durationMs: number = DEFAULT_DURATION) => {
      setToasts((current) => [
        ...current,
        { id: Date.now() + Math.random(), text, kind, durationMs },
      ]);
    },
    [],
  );

  const pushToast = useCallback(
    (opts: { body: string; kind?: ToastKind; title?: string; durationMs?: number }) => {
      setToasts((current) => [
        ...current,
        {
          id: Date.now() + Math.random(),
          text: opts.body,
          kind: opts.kind ?? "info",
          durationMs: opts.durationMs ?? DEFAULT_DURATION,
          title: opts.title,
        },
      ]);
    },
    [],
  );

  // Purge toasts that have finished their exit animation.
  useEffect(() => {
    const leaving = toasts.filter((t) => t.leaving);
    if (leaving.length === 0) return;
    const timers = leaving.map((t) =>
      window.setTimeout(() => setToasts((cur) => cur.filter((x) => x.id !== t.id)), 180),
    );
    return () => {
      for (const id of timers) window.clearTimeout(id);
    };
  }, [toasts]);

  return { toasts, push, pushToast, dismiss };
}

const KIND_ICON = {
  info: Info,
  success: CheckCircle,
  warning: AlertTriangle,
  error: AlertCircle,
} as const;

/**
 * A single toast. Owns its auto-dismiss timer so each toast counts down
 * exactly once (independent of sibling state changes). Hovering pauses the
 * countdown — both the JS timer and the CSS progress bar pause together so
 * they stay in sync. `durationMs <= 0` is sticky (no timer, no progress bar;
 * only the close button dismisses it).
 */
function ToastItem({
  toast,
  onDismiss,
}: {
  toast: ToastMessage;
  onDismiss: (id: number) => void;
}) {
  const timeoutRef = useRef<number | null>(null);
  const deadlineRef = useRef<number>(0);
  const remainingRef = useRef<number>(toast.durationMs);
  const sticky = toast.durationMs <= 0;

  const schedule = useCallback(
    (ms: number) => {
      if (ms <= 0 || sticky || toast.leaving) return;
      deadlineRef.current = Date.now() + ms;
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => onDismiss(toast.id), ms);
    },
    [onDismiss, toast.id, toast.leaving, sticky],
  );

  useEffect(() => {
    schedule(toast.durationMs);
    return () => {
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
    };
  }, [schedule, toast.durationMs]);

  const handleMouseEnter = () => {
    // Pause the JS countdown; the CSS progress bar pauses via :hover.
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
      remainingRef.current = Math.max(0, deadlineRef.current - Date.now());
    }
  };
  const handleMouseLeave = () => {
    schedule(remainingRef.current);
  };

  const Icon = KIND_ICON[toast.kind];
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    const text = toast.title ? `${toast.title}\n${toast.text}` : toast.text;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // silently ignore clipboard failures
    }
  }, [toast.text, toast.title]);

  return (
    <div
      className={`toast toast--${toast.kind}${toast.leaving ? " is-leaving" : ""}`}
      role={toast.kind === "error" ? "alert" : "status"}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      style={
        sticky
          ? undefined
          : ({ "--toast-duration": `${toast.durationMs}ms` } as CSSProperties)
      }
    >
      <Icon className="toast__icon" size={16} aria-hidden />
      <div className="toast__body">
        {toast.title && <div className="toast__title">{toast.title}</div>}
        <div className="toast__text">{toast.text}</div>
      </div>
      {toast.kind === "error" && (
        <button
          type="button"
          className="toast__copy"
          onClick={handleCopy}
          aria-label={copied ? "Copied to clipboard" : "Copy to clipboard"}
          title={copied ? "Copied" : "Copy"}
        >
          {copied ? <Check size={14} aria-hidden /> : <Copy size={14} aria-hidden />}
        </button>
      )}
      <button
        type="button"
        className="toast__close"
        onClick={() => onDismiss(toast.id)}
        aria-label="Dismiss notification"
      >
        <X size={14} aria-hidden />
      </button>
      {!sticky && <span className="toast__progress" />}
    </div>
  );
}

export function ToastStack({
  toasts,
  onDismiss,
}: {
  toasts: ToastMessage[];
  onDismiss: (id: number) => void;
}) {
  return (
    <div className="toast-stack">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

import { useEffect, useState } from "react";

export interface ToastMessage {
  id: number;
  text: string;
  kind: "info" | "success" | "warning" | "error";
  durationMs: number;
}

export function useToasts() {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  useEffect(() => {
    const timers: number[] = [];
    for (const t of toasts) {
      const id = window.setTimeout(() => {
        setToasts((current) => current.filter((x) => x.id !== t.id));
      }, t.durationMs);
      timers.push(id);
    }
    return () => {
      for (const id of timers) {
        window.clearTimeout(id);
      }
    };
  }, [toasts]);

  return {
    toasts,
    push(text: string, kind: ToastMessage["kind"] = "info", durationMs = 3000) {
      setToasts((current) => [
        ...current,
        { id: Date.now() + Math.random(), text, kind, durationMs },
      ]);
    },
    dismiss(id: number) {
      setToasts((current) => current.filter((t) => t.id !== id));
    },
  };
}

export function ToastStack({ toasts }: { toasts: ToastMessage[] }) {
  return (
    <div role="status" aria-live="polite" style={{ position: "fixed", inset: 0, pointerEvents: "none" }}>
      {toasts.map((t, i) => (
        <div
          key={t.id}
          className={`toast toast--${t.kind}`}
          style={{ bottom: 36 + i * 56, pointerEvents: "auto" }}
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}

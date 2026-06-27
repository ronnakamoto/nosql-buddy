import { createContext, useContext } from "react";
import type { ToastKind } from "../components/Toast";

export interface ToastApi {
  push: (text: string, kind?: ToastKind, durationMs?: number) => void;
  pushToast: (opts: {
    body: string;
    kind?: ToastKind;
    title?: string;
    durationMs?: number;
  }) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

export const ToastProvider = ToastContext.Provider;

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used inside a ToastProvider");
  }
  return ctx;
}

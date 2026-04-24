import { createSignal } from "solid-js";

export type Toast = { id: number; message: string; severity?: "error" | "info" };

export function useToast() {
  const [toasts, setToasts] = createSignal<Toast[]>([]);
  let toastSeq = 0;

  const showToast = (message: string, severity: Toast["severity"] = "error") => {
    const id = ++toastSeq;
    setToasts((ts) => [...ts, { id, message, severity }]);
    window.setTimeout(() => setToasts((ts) => ts.filter((t) => t.id !== id)), 4000);
  };

  return { toasts, showToast };
}

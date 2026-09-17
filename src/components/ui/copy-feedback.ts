import { createSignal, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";

export interface CopyFeedback {
  copied: Accessor<boolean>;
  copy: () => Promise<boolean>;
}

/**
 * SolidJS reactive hook for transient copy feedback.
 * @param text - The string to copy or an accessor returning the string.
 * @param durationMs - Time in ms to maintain the copied state (default: 1200ms).
 */
export function createCopyFeedback(
  text: string | (() => string),
  durationMs: number = 1200,
): CopyFeedback {
  const [copied, setCopied] = createSignal(false);
  let timer: number | undefined;

  const copy = async (): Promise<boolean> => {
    const value = typeof text === "function" ? text() : text;
    if (!value) return false;
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (timer) window.clearTimeout(timer);
      timer = window.setTimeout(() => setCopied(false), durationMs);
      return true;
    } catch {
      return false;
    }
  };

  onCleanup(() => {
    if (timer) window.clearTimeout(timer);
  });

  return { copied, copy };
}

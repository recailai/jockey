import { createSignal } from "solid-js";

type UseResizeOpts = {
  axis: "x" | "y";
  min: number;
  max: number;
  getStart: () => number;
  onCommit: (px: number) => void;
};

export type UseResize = {
  previewPx: () => number | null;
  beginResize: (e: MouseEvent) => void;
};

export function useResize(opts: UseResizeOpts): UseResize {
  const [previewPx, setPreviewPx] = createSignal<number | null>(null);
  const clamp = (v: number) => Math.min(opts.max, Math.max(opts.min, v));

  const beginResize = (startEvent: MouseEvent) => {
    startEvent.preventDefault();
    const startCoord = opts.axis === "x" ? startEvent.clientX : startEvent.clientY;
    const startPx = opts.getStart();
    setPreviewPx(clamp(startPx));
    document.body.style.cursor = opts.axis === "x" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";

    const onMove = (ev: MouseEvent) => {
      const cur = opts.axis === "x" ? ev.clientX : ev.clientY;
      setPreviewPx(clamp(startPx + (cur - startCoord)));
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      const final = previewPx();
      setPreviewPx(null);
      if (final !== null) opts.onCommit(final);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  return { previewPx, beginResize };
}

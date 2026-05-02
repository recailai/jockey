import { createEffect, onCleanup, onMount } from "solid-js";
import type { ImageAttachment } from "../lib/tauriApi";

const chipPreviews = new WeakMap<HTMLElement, HTMLElement>();

export type RichNode =
  | { kind: "text"; text: string }
  | { kind: "image"; index: number; img: ImageAttachment };

type Props = {
  nodes: () => RichNode[];
  placeholder: string;
  class?: string;
  ref?: (el: HTMLDivElement) => void;
  onNodesChange: (nodes: RichNode[]) => void;
  onCaretText: (text: string, caret: number) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  onFocus: (e: FocusEvent) => void;
  onBlur: (e: FocusEvent) => void;
  onClick: (e: MouseEvent) => void;
  onRemoveImage?: (index: number) => void;
};

function makeChipEl(node: RichNode & { kind: "image" }, onRemove?: (index: number) => void): HTMLElement {
  const chip = document.createElement("span");
  chip.contentEditable = "false";
  chip.dataset.imageIndex = String(node.index);
  chip.dataset.imageMime = node.img.mimeType;
  chip.dataset.imageData = node.img.data;
  chip.style.cssText = [
    "display:inline-flex",
    "align-items:center",
    "gap:4px",
    "border-radius:6px",
    "border:1px solid var(--ui-border)",
    "background:var(--ui-surface-muted)",
    "padding:2px 6px",
    "font-size:11px",
    "line-height:1.4",
    "color:var(--ui-muted)",
    "user-select:none",
    "cursor:default",
    "margin:0 2px",
    "vertical-align:middle",
    "position:relative",
  ].join(";");

  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", "10");
  svg.setAttribute("height", "10");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.style.flexShrink = "0";
  svg.innerHTML =
    '<rect x="3" y="3" width="18" height="18" rx="2"/>' +
    '<circle cx="8.5" cy="8.5" r="1.5"/>' +
    '<polyline points="21 15 16 10 5 21"/>';
  chip.appendChild(svg);

  const label = document.createElement("span");
  label.textContent = `Image ${node.index + 1}`;
  chip.appendChild(label);

  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.textContent = "✕";
  removeBtn.style.cssText = [
    "display:inline-flex",
    "align-items:center",
    "justify-content:center",
    "width:12px",
    "height:12px",
    "border-radius:50%",
    "border:none",
    "background:transparent",
    "color:inherit",
    "font-size:8px",
    "line-height:1",
    "padding:0",
    "cursor:pointer",
    "opacity:0.5",
    "margin-left:2px",
  ].join(";");
  removeBtn.addEventListener("mouseenter", () => { removeBtn.style.opacity = "1"; });
  removeBtn.addEventListener("mouseleave", () => { removeBtn.style.opacity = "0.5"; });
  removeBtn.addEventListener("mousedown", (e) => {
    e.preventDefault();
    e.stopPropagation();
    onRemove?.(node.index);
  });
  chip.appendChild(removeBtn);

  // Hover preview — fixed positioning so it escapes overflow:hidden containers
  const preview = document.createElement("span");
  preview.style.cssText = [
    "position:fixed",
    "z-index:9999",
    "pointer-events:none",
    "opacity:0",
    "transition:opacity 0.15s",
  ].join(";");
  const previewImg = document.createElement("img");
  previewImg.src = `data:${node.img.mimeType};base64,${node.img.data}`;
  previewImg.style.cssText = [
    "max-width:224px",
    "max-height:192px",
    "border-radius:8px",
    "box-shadow:0 8px 32px rgba(0,0,0,0.4)",
    "border:1px solid var(--ui-border)",
    "object-fit:contain",
    "background:var(--ui-surface)",
    "display:block",
  ].join(";");
  previewImg.alt = `image ${node.index + 1}`;
  preview.appendChild(previewImg);
  document.body.appendChild(preview);
  chipPreviews.set(chip, preview);

  chip.addEventListener("mouseenter", () => {
    const r = chip.getBoundingClientRect();
    preview.style.left = `${r.left}px`;
    preview.style.top = `${r.top - 8}px`;
    preview.style.transform = "translateY(-100%)";
    preview.style.opacity = "1";
  });
  chip.addEventListener("mouseleave", () => { preview.style.opacity = "0"; });

  return chip;
}

function removeChipPreviews(el: HTMLDivElement) {
  for (const child of Array.from(el.childNodes)) {
    if (child instanceof HTMLElement) {
      const preview = chipPreviews.get(child);
      if (preview) preview.remove();
    }
  }
}

function nodesToDom(el: HTMLDivElement, nodes: RichNode[], onRemove?: (index: number) => void) {
  removeChipPreviews(el);
  el.innerHTML = "";
  for (const node of nodes) {
    if (node.kind === "text") {
      if (node.text) el.appendChild(document.createTextNode(node.text));
    } else {
      el.appendChild(makeChipEl(node, onRemove));
    }
  }
  if (el.childNodes.length === 0) el.appendChild(document.createTextNode(""));
}

function domToNodes(el: HTMLDivElement): RichNode[] {
  const nodes: RichNode[] = [];
  for (const child of Array.from(el.childNodes)) {
    if (child.nodeType === Node.TEXT_NODE) {
      const text = child.textContent ?? "";
      if (text) nodes.push({ kind: "text", text });
    } else if (child instanceof HTMLElement && child.dataset.imageIndex != null) {
      nodes.push({
        kind: "image",
        index: parseInt(child.dataset.imageIndex, 10),
        img: { data: child.dataset.imageData ?? "", mimeType: child.dataset.imageMime ?? "" },
      });
    }
  }
  return nodes;
}

export function getRichInputCaretOffset(el: HTMLDivElement): number {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return -1;
  const range = sel.getRangeAt(0);
  if (!el.contains(range.startContainer)) return -1;
  let offset = 0;
  for (const child of Array.from(el.childNodes)) {
    if (child === range.startContainer || child.contains(range.startContainer)) {
      if (child.nodeType === Node.TEXT_NODE) offset += range.startOffset;
      break;
    }
    offset += child.nodeType === Node.TEXT_NODE ? (child.textContent?.length ?? 0) : 1;
  }
  return offset;
}

function domPositionAtOffset(el: HTMLDivElement, offset: number): { container: Node; offset: number } {
  let remaining = Math.max(0, offset);
  const children = Array.from(el.childNodes);
  for (let i = 0; i < children.length; i++) {
    const child = children[i];
    const len = child.nodeType === Node.TEXT_NODE ? (child.textContent?.length ?? 0) : 1;
    if (remaining <= len) {
      if (child.nodeType === Node.TEXT_NODE) {
        return { container: child, offset: Math.min(remaining, child.textContent?.length ?? 0) };
      }
      return { container: el, offset: remaining === 0 ? i : i + 1 };
    }
    remaining -= len;
  }
  return { container: el, offset: children.length };
}

export function setRichInputSelection(el: HTMLDivElement, start: number, end = start) {
  const range = document.createRange();
  const startPos = domPositionAtOffset(el, start);
  const endPos = domPositionAtOffset(el, Math.max(start, end));
  range.setStart(startPos.container, startPos.offset);
  range.setEnd(endPos.container, endPos.offset);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

function setCaretAtOffset(el: HTMLDivElement, offset: number) {
  if (offset < 0) return;
  setRichInputSelection(el, offset, offset);
}

export function placeCaretAfterChip(el: HTMLDivElement, chipIndex: number) {
  const children = Array.from(el.childNodes);
  const chipEl = children.find(
    (c) => c instanceof HTMLElement && c.dataset.imageIndex === String(chipIndex)
  );
  if (!chipEl) return;
  const idx = children.indexOf(chipEl);
  const range = document.createRange();
  const next = children[idx + 1];
  if (next && next.nodeType === Node.TEXT_NODE) {
    range.setStart(next, 0);
  } else {
    range.setStart(el, idx + 1);
  }
  range.collapse(true);
  window.getSelection()?.removeAllRanges();
  window.getSelection()?.addRange(range);
}

export function isImageNode(n: RichNode): n is RichNode & { kind: "image" } {
  return n.kind === "image";
}

export function getPlainText(nodes: RichNode[]): string {
  return nodes.map((n) => (n.kind === "text" ? n.text : `[image:${n.index}]`)).join("");
}

function normalizeRichNodes(nodes: RichNode[]): RichNode[] {
  const out: RichNode[] = [];
  for (const node of nodes) {
    if (node.kind === "text") {
      if (!node.text) continue;
      const last = out[out.length - 1];
      if (last?.kind === "text") {
        last.text += node.text;
      } else {
        out.push({ kind: "text", text: node.text });
      }
    } else {
      out.push(node);
    }
  }
  return out;
}

function richNodeCaretLength(node: RichNode): number {
  return node.kind === "text" ? node.text.length : 1;
}

export function insertTextIntoRichNodes(nodes: RichNode[], offset: number, text: string): { nodes: RichNode[]; caret: number } {
  const total = nodes.reduce((sum, node) => sum + richNodeCaretLength(node), 0);
  const insertAt = Math.min(total, Math.max(0, offset));
  if (nodes.length === 0) {
    return { nodes: text ? [{ kind: "text", text }] : [], caret: text.length };
  }

  let remaining = insertAt;
  let inserted = false;
  const out: RichNode[] = [];

  for (const node of nodes) {
    if (!inserted) {
      if (node.kind === "text") {
        if (remaining <= node.text.length) {
          const before = node.text.slice(0, remaining);
          const after = node.text.slice(remaining);
          if (before) out.push({ kind: "text", text: before });
          if (text) out.push({ kind: "text", text });
          if (after) out.push({ kind: "text", text: after });
          inserted = true;
          continue;
        }
        remaining -= node.text.length;
      } else {
        if (remaining <= 1) {
          if (remaining === 0) {
            if (text) out.push({ kind: "text", text });
            out.push(node);
          } else {
            out.push(node);
            if (text) out.push({ kind: "text", text });
          }
          inserted = true;
          continue;
        }
        remaining -= 1;
      }
    }
    out.push(node);
  }

  if (!inserted && text) out.push({ kind: "text", text });
  return { nodes: normalizeRichNodes(out), caret: insertAt + text.length };
}

export default function RichInput(props: Props) {
  let el: HTMLDivElement | undefined;
  let suppressEffect = false;
  let composing = false;

  onMount(() => {
    if (!el) return;
    nodesToDom(el, props.nodes(), props.onRemoveImage);
  });

  onCleanup(() => {
    if (el) removeChipPreviews(el);
  });

  createEffect(() => {
    if (!el) return;
    const nodes = props.nodes();
    if (suppressEffect) { suppressEffect = false; return; }
    const savedOffset = getRichInputCaretOffset(el);
    nodesToDom(el, nodes, props.onRemoveImage);
    setCaretAtOffset(el, savedOffset);
  });

  const emitCaret = () => {
    if (!el || composing) return;
    const nodes = domToNodes(el);
    props.onCaretText(getPlainText(nodes), getRichInputCaretOffset(el));
  };

  const emitCaretSoon = () => queueMicrotask(emitCaret);

  const emitChange = () => {
    if (!el || composing) return;
    suppressEffect = true;
    const nodes = domToNodes(el);
    props.onNodesChange(nodes);
    const plain = getPlainText(nodes);
    const caret = getRichInputCaretOffset(el);
    props.onCaretText(plain, caret);
  };

  const handlePaste = (e: ClipboardEvent) => {
    const text = e.clipboardData?.getData("text/plain");
    if (!text) return;
    e.preventDefault();
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return;
    const range = sel.getRangeAt(0);
    range.deleteContents();
    range.insertNode(document.createTextNode(text));
    range.collapse(false);
    sel.removeAllRanges();
    sel.addRange(range);
    emitChange();
  };

  return (
    <div
      ref={(d) => { el = d; props.ref?.(d); }}
      contentEditable
      onCompositionStart={() => { composing = true; }}
      onCompositionEnd={() => { composing = false; emitChange(); }}
      onInput={emitChange}
      onPaste={handlePaste}
      onKeyDown={(e) => {
        if (e.isComposing || e.keyCode === 229) return;
        props.onKeyDown(e);
      }}
      onKeyUp={(e) => {
        if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(e.key)) {
          emitCaretSoon();
        }
      }}
      onFocus={(e) => {
        props.onFocus(e);
        emitCaretSoon();
      }}
      onBlur={props.onBlur}
      onClick={(e) => {
        props.onClick(e);
        emitCaretSoon();
      }}
      onMouseUp={emitCaretSoon}
      data-placeholder={props.placeholder}
      class={props.class}
      style={{ "min-height": "1.5em" }}
      spellcheck={false}
    />
  );
}

export type { Props as RichInputProps };

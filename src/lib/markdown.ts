import { marked } from "marked";

const COPY_BTN = `<button data-copy-code class="jui-code-copy" title="Copy code">Copy</button>`;

function injectCopyButtons(html: string): string {
  return html.replace(/<pre\b([^>]*)>/g, `<pre$1 class="group/pre relative">${COPY_BTN}`);
}

export const renderMd = (text: string): string => {
  try {
    return injectCopyButtons(marked.parse(text, { async: false }) as string);
  } catch (e) {
    return `<pre>${String(e)}</pre>`;
  }
};

const MD_CACHE_MAX = 500;
const mdCache = new Map<string, string>();

/** FNV-1a: cheap enough to run on every render, and we only need change detection. */
function digest(text: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

export function renderMdCached(id: string, text: string): string {
  // Keyed on content as well as id: finalizing a stream reuses a message's id while
  // replacing its text, so an id-only key can serve the pre-finalize HTML forever.
  const key = `${id}:${text.length}:${digest(text)}`;
  const hit = mdCache.get(key);
  if (hit !== undefined) return hit;
  const html = renderMd(text);
  if (mdCache.size >= MD_CACHE_MAX) {
    mdCache.delete(mdCache.keys().next().value!);
  }
  mdCache.set(key, html);
  return html;
}

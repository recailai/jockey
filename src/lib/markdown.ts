import { marked } from "marked";

const COPY_BTN = `<button data-copy-code class="absolute top-1.5 right-1.5 px-1.5 py-0.5 rounded text-[10px] font-mono opacity-0 group-hover/pre:opacity-100 transition-opacity cursor-pointer theme-muted hover:theme-text" style="background:var(--ui-panel)">Copy</button>`;

function injectCopyButtons(html: string): string {
  return html.replace(/<pre>/g, `<pre class="group/pre relative">${COPY_BTN}`);
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

export function renderMdCached(id: string, text: string): string {
  const hit = mdCache.get(id);
  if (hit !== undefined) return hit;
  const html = renderMd(text);
  if (mdCache.size >= MD_CACHE_MAX) {
    mdCache.delete(mdCache.keys().next().value!);
  }
  mdCache.set(id, html);
  return html;
}

import { marked } from "marked";

const COPY_BTN = `<button data-copy-code class="jui-code-copy" title="Copy code">Copy</button>`;

/** Each code block gets a sticky banner with its language and a copy button. The banner sits
 *  in the <pre> but outside <code>, so copying (which reads <code>) never includes it. */
function injectCopyButtons(html: string): string {
  return html.replace(
    /<pre\b([^>]*)>(<code(?: class="language-([\w+#.-]+)")?)/g,
    (_m, attrs: string, code: string, lang?: string) =>
      `<pre${attrs} class="md-code group/pre relative"><span class="md-code-banner"><span class="md-code-lang">${lang ?? "code"}</span>${COPY_BTN}</span>${code}`,
  );
}

export const renderMd = (text: string): string => {
  try {
    return injectCopyButtons(marked.parse(text, { async: false }) as string);
  } catch (e) {
    return `<pre>${String(e)}</pre>`;
  }
};

// Sized for segment-level entries across a long session, not one entry per message.
const MD_CACHE_MAX = 2000;
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

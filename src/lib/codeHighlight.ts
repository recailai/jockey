export type HighlightToken = { text: string; class?: string };

const KEYWORDS = new Set([
  "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete",
  "do", "else", "enum", "export", "extends", "false", "finally", "fn", "for", "from", "function",
  "if", "impl", "import", "in", "interface", "let", "match", "new", "null", "of", "pub", "return",
  "self", "struct", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined",
  "use", "var", "void", "while", "yield",
]);

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function pushText(tokens: HighlightToken[], text: string, cls?: string): void {
  if (!text) return;
  tokens.push(cls ? { text, class: cls } : { text });
}

function highlightGeneric(line: string): HighlightToken[] {
  const trimmed = line.trimStart();
  if (trimmed.startsWith("//") || trimmed.startsWith("#") || trimmed.startsWith("*")) {
    return [{ text: line, class: "code-token-comment" }];
  }

  const tokens: HighlightToken[] = [];
  const re = /("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`)|\b(?:0x[\da-fA-F]+|\d+(?:\.\d+)?)\b|\b[A-Za-z_][\w$]*\b/g;
  let last = 0;
  let match: RegExpExecArray | null;

  while ((match = re.exec(line)) !== null) {
    pushText(tokens, line.slice(last, match.index));
    const value = match[0];
    if (value.startsWith("\"") || value.startsWith("'") || value.startsWith("`")) {
      pushText(tokens, value, "code-token-string");
    } else if (/^\d/.test(value) || value.startsWith("0x")) {
      pushText(tokens, value, "code-token-number");
    } else if (KEYWORDS.has(value)) {
      pushText(tokens, value, "code-token-keyword");
    } else if (/^[A-Z]/.test(value)) {
      pushText(tokens, value, "code-token-type");
    } else {
      pushText(tokens, value);
    }
    last = match.index + value.length;
  }
  pushText(tokens, line.slice(last));
  return tokens;
}

export function languageFromPath(path: string): string | null {
  const base = path.split("/").pop() ?? path;
  const dot = base.lastIndexOf(".");
  const ext = dot === -1 ? "" : base.slice(dot + 1).toLowerCase();
  const name = base.toLowerCase();
  if (name === "dockerfile" || name === "makefile") return "shell";
  switch (ext) {
    case "ts":
    case "tsx":
    case "mts":
    case "cts":
      return "typescript";
    case "js":
    case "jsx":
    case "mjs":
    case "cjs":
      return "javascript";
    case "rs":
      return "rust";
    case "py":
      return "python";
    case "go":
      return "go";
    case "css":
    case "scss":
    case "less":
      return "css";
    case "html":
    case "vue":
    case "svelte":
      return "markup";
    case "json":
    case "jsonc":
      return "json";
    case "md":
    case "mdx":
      return "markdown";
    case "sh":
    case "bash":
    case "zsh":
      return "shell";
    case "toml":
    case "yaml":
    case "yml":
      return "config";
    default:
      return ext || null;
  }
}

export function highlightCodeLine(line: string, lang: string | null): HighlightToken[] {
  if (!lang || lang === "markdown" || lang === "json" || lang === "config" || lang === "markup") {
    return highlightGeneric(line);
  }
  return highlightGeneric(line);
}

export function highlightCodeLineHtml(line: string, lang: string | null): string {
  return highlightCodeLine(line, lang)
    .map((token) => (token.class
      ? `<span class="${token.class}">${escapeHtml(token.text)}</span>`
      : escapeHtml(token.text)))
    .join("");
}

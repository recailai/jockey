import type { AcpMcpServer } from "../components/types";

export function mcpTransport(s: AcpMcpServer): "stdio" | "http" | "sse" {
  if ("type" in s) return s.type;
  return "stdio";
}

export function mcpDisplayUri(s: AcpMcpServer): string {
  if ("command" in s) return `${s.command} ${s.args.join(" ")}`;
  return s.url;
}

export function parseCommandArgs(raw: string): string[] | null {
  const out: string[] = [];
  let buf = "";
  let quote: "'" | '"' | null = null;
  let escape = false;

  for (const ch of raw) {
    if (escape) {
      buf += ch;
      escape = false;
      continue;
    }
    if (ch === "\\") {
      escape = true;
      continue;
    }
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else {
        buf += ch;
      }
      continue;
    }
    if (ch === "'" || ch === "\"") {
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (buf) {
        out.push(buf);
        buf = "";
      }
      continue;
    }
    buf += ch;
  }

  if (escape || quote) return null;
  if (buf) out.push(buf);
  return out;
}

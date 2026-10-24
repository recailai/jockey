export const fmtDate = (ts: number): string =>
  new Date(ts).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

export const fmtRelative = (ts: number): string => {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
};

/**
 * Session names are space-free identifiers (`fix_the_login_page`) so they work in mentions
 * and terminals; reading surfaces show them with spaces.
 */
export function displayTitle(title: string): string {
  return title.replace(/_+/g, " ").trim() || title;
}

/** Metrics for a list capped with head/tail slices. */
export interface HeadTailCap {
  /** Rows beyond the cap (total - maxLines); <= 0 means all rows visible. */
  hidden: number;
  /** Whether the list is over the cap and collapsed. */
  capped: boolean;
  /** Head-slice line count: ceil(maxLines / 2). */
  headLines: number;
  /** Tail-slice line count: remainder after head. */
  tailLines: number;
}

/**
 * Compute the head/tail cap metrics for a list of total rows against maxLines.
 * When collapsed and total > maxLines, keeps the top half and bottom half while
 * folding the middle lines.
 *
 * @param total - Total row or line count.
 * @param maxLines - Maximum visible lines when collapsed (default: 16).
 * @param expanded - Whether the surface is currently expanded.
 */
export function headTailCap(
  total: number,
  maxLines: number = 16,
  expanded: boolean = false,
): HeadTailCap {
  const hidden = total - maxLines;
  const headLines = Math.ceil(maxLines / 2);
  return {
    hidden: Math.max(0, hidden),
    capped: hidden > 0 && !expanded,
    headLines,
    tailLines: Math.max(0, maxLines - headLines),
  };
}

type PermissionOptionLike = { optionId: string; kind?: string | null; title?: string | null };

export const isRejectOption = (o: PermissionOptionLike) => (o.kind ?? "").startsWith("reject");

/** Options to show for the current "remember my choice" state. With it on, the `*_always`
 *  variants replace their `*_once` siblings; options without a kind always show. */
export function visiblePermissionOptions<T extends PermissionOptionLike>(all: T[], remember: boolean): T[] {
  return all.filter((o) => {
    const kind = o.kind ?? "";
    if (!kind.endsWith("_once") && !kind.endsWith("_always")) return true;
    const family = kind.replace(/_(once|always)$/, "");
    const hasAlways = all.some((x) => x.kind === `${family}_always`);
    if (remember && hasAlways) return kind.endsWith("_always");
    return kind.endsWith("_once") || !all.some((x) => x.kind === `${family}_once`);
  });
}

/** Jockey's own Deny only when the agent did not offer a reject option itself; showing both
 *  gave the user two indistinguishable "no" buttons. */
export const needsGenericDeny = (all: PermissionOptionLike[]) => !all.some(isRejectOption);

export type PermissionOptionLike = {
  optionId: string;
  kind?: string | null;
  title?: string | null;
  description?: string | null;
};

type PermissionLike = { toolKind?: string | null; options: PermissionOptionLike[] };

export const isRejectOption = (o: PermissionOptionLike) => (o.kind ?? "").startsWith("reject");

export function allowOnceOption<T extends PermissionOptionLike>(options: T[]): T | undefined {
  return options.find((o) => o.kind === "allow_once" || (!o.kind && o.optionId === "allow_once"));
}

/** ACP names an option `name`; the UI reads `title`. A description comes from the option's
 *  `_meta` when the adapter supplies one. */
export function normalizePermissionOptions(raw: unknown): PermissionOptionLike[] {
  if (!Array.isArray(raw)) return [];
  return raw.map((entry) => {
    const o = (entry ?? {}) as Record<string, unknown>;
    const meta = (o._meta ?? {}) as Record<string, unknown>;
    const description = typeof meta.description === "string" ? meta.description : (o.description as string | undefined);
    return {
      optionId: String(o.optionId ?? ""),
      kind: (o.kind as string | undefined) ?? undefined,
      title: (o.title as string | undefined) ?? (o.name as string | undefined),
      description: description || undefined,
    };
  });
}

/** Approving a plan picks how the session continues, so every choice is shown as-is: no
 *  once/always folding and no remembered answer. */
export const isPlanApproval = (p: { toolKind?: string | null }) => p.toolKind === "switch_mode";

export const canRememberChoice = (p: PermissionLike) =>
  !isPlanApproval(p) && p.options.some((o) => (o.kind ?? "").endsWith("_always"));

/** Options to show for the current "remember my choice" state. With it on, the `*_always`
 *  variants replace their `*_once` siblings; options without a kind always show. */
export function visiblePermissionOptions<T extends PermissionOptionLike>(
  all: T[],
  remember: boolean,
  showAll = false,
): T[] {
  if (showAll) return all;
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

/** Markdown of the plan a switch_mode approval asks about. */
export function planMarkdown(p: { content?: unknown[] | null; rawInput?: unknown }): string | null {
  for (const item of p.content ?? []) {
    const text = (item as { content?: { text?: unknown } })?.content?.text;
    if (typeof text === "string" && text.trim()) return text;
  }
  const raw = p.rawInput as { plan?: unknown } | null | undefined;
  return typeof raw?.plan === "string" && raw.plan.trim() ? raw.plan : null;
}

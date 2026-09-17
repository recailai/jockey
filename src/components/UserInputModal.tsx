import { For, Show, createMemo, createSignal } from "solid-js";
import { MessageCircleQuestion } from "lucide-solid";
import type { Accessor } from "solid-js";
import type { AppSession, AppUserInputQuestion } from "./types";
import { Button, Dialog, DialogContent, Input } from "./ui";
import { assistantApi } from "../lib/tauriApi";

/**
 * Structured question form. Separate from PermissionModal because a question is not an
 * allow/deny decision: it can carry several questions, each with options, free text or a
 * secret value, and the agent blocks until every one is answered.
 */
export default function UserInputModal(props: {
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
}) {
  const [picked, setPicked] = createSignal<Record<string, string[]>>({});
  const [other, setOther] = createSignal<Record<string, string>>({});
  const [submitting, setSubmitting] = createSignal(false);

  const request = createMemo(() => props.activeSession()?.pendingUserInput?.[0] ?? null);

  const answerFor = (q: AppUserInputQuestion): string[] => {
    const free = (other()[q.id] ?? "").trim();
    if (free) return [free];
    return picked()[q.id] ?? [];
  };

  const complete = createMemo(() => {
    const req = request();
    return !!req && req.questions.every((q) => answerFor(q).length > 0);
  });

  const choose = (q: AppUserInputQuestion, value: string) => {
    setPicked((prev) => {
      const current = prev[q.id] ?? [];
      if (q.multi) {
        return {
          ...prev,
          [q.id]: current.includes(value)
            ? current.filter((v) => v !== value)
            : [...current, value],
        };
      }
      return { ...prev, [q.id]: [value] };
    });
    // Picking an option supersedes anything typed into the free-text box.
    setOther((prev) => ({ ...prev, [q.id]: "" }));
  };

  const dismiss = (answers: Record<string, string[]> | null) => {
    const req = request();
    if (!req) return;
    setSubmitting(true);
    void assistantApi
      .respondUserInput(req.requestId, answers)
      .catch(() => {})
      .finally(() => {
        props.patchActiveSession({
          pendingUserInput: (props.activeSession()?.pendingUserInput ?? []).filter(
            (q) => q.requestId !== req.requestId,
          ),
        });
        setPicked({});
        setOther({});
        setSubmitting(false);
      });
  };

  const submit = () => {
    const req = request();
    if (!req) return;
    const answers: Record<string, string[]> = {};
    for (const q of req.questions) answers[q.id] = answerFor(q);
    dismiss(answers);
  };

  return (
    <Dialog open={request() !== null} onOpenChange={(open) => { if (!open) dismiss(null); }}>
      <DialogContent
        title={request()?.title ?? "Agent needs your input"}
        description={undefined}
        icon={<MessageCircleQuestion size={20} class="text-blue-500" />}
      >
        <div class="mt-3 max-h-[60vh] space-y-4 overflow-auto">
          <For each={request()?.questions ?? []}>
            {(q) => (
              <div class="space-y-2">
                <Show when={q.header}>
                  <div class="text-[10px] font-semibold uppercase tracking-wide theme-muted">
                    {q.header}
                  </div>
                </Show>
                <div class="text-[12.5px] theme-text">{q.prompt}</div>
                <Show when={q.options.length > 0}>
                  <div class="space-y-1">
                    <For each={q.options}>
                      {(opt) => (
                        <button
                          type="button"
                          onClick={() => choose(q, opt.value)}
                          class="flex w-full items-start gap-2 rounded-lg border px-2.5 py-1.5 text-left transition-colors"
                          classList={{
                            "border-blue-500 bg-blue-500/10": (picked()[q.id] ?? []).includes(opt.value),
                            "theme-border hover:bg-[var(--ui-row-hover)]": !(picked()[q.id] ?? []).includes(opt.value),
                          }}
                        >
                          <span class="min-w-0 flex-1">
                            <span class="block text-[12px] theme-text">{opt.label}</span>
                            <Show when={opt.description}>
                              <span class="block text-[10px] theme-muted">{opt.description}</span>
                            </Show>
                          </span>
                        </button>
                      )}
                    </For>
                  </div>
                </Show>
                <Show when={q.allowOther || q.options.length === 0}>
                  <Input
                    type={q.secret ? "password" : "text"}
                    placeholder={q.options.length === 0 ? "Your answer" : "Something else…"}
                    value={other()[q.id] ?? ""}
                    onInput={(e) => {
                      const v = e.currentTarget.value;
                      setOther((prev) => ({ ...prev, [q.id]: v }));
                      if (v.trim()) setPicked((prev) => ({ ...prev, [q.id]: [] }));
                    }}
                    class="text-xs"
                  />
                </Show>
              </div>
            )}
          </For>
        </div>

        <div class="mt-4 flex justify-end gap-2 border-t border-[var(--ui-border)] pt-3">
          <Button variant="secondary" size="md" class="text-xs" onClick={() => dismiss(null)}>
            Skip
          </Button>
          <Button size="md" class="text-xs" disabled={!complete() || submitting()} onClick={submit}>
            {submitting() ? "Sending…" : "Answer"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

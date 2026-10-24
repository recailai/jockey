import { For, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { Check, MessageCircleQuestion } from "lucide-solid";
import type { Accessor } from "solid-js";
import type { AppSession, AppUserInputQuestion } from "./types";
import { Alert, Button, Dialog, DialogContent, Input, Panel, RowButton, Tabs, TabsContent, TabsList, TabsTrigger } from "./ui";
import { assistantApi } from "../lib/tauriApi";

const REVIEW = "review";

const focusComposer = () => {
  queueMicrotask(() => document.querySelector<HTMLElement>(".composer-rich-input")?.focus());
};

/**
 * Structured question form. Separate from PermissionModal because a question is not an
 * allow/deny decision: it can carry several questions, each with options, free text or a
 * secret value, and the agent blocks until it is answered, skipped or discussed in chat.
 * `inline` renders it as a card in the conversation; otherwise it is a dialog.
 */
export default function UserInputModal(props: {
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  onDefer?: () => void;
  inline?: boolean;
  /** After the form is declined; defaults to focusing the composer. */
  onChatAbout?: () => void;
}) {
  const [picked, setPicked] = createSignal<Record<string, string[]>>({});
  const [other, setOther] = createSignal<Record<string, string>>({});
  const [step, setStep] = createSignal<string>("");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const request = createMemo(() => props.activeSession()?.pendingUserInput?.[0] ?? null);
  const questions = () => request()?.questions ?? [];
  const multiStep = () => questions().length > 1;
  createEffect(on(() => request()?.requestId, () => {
    setPicked({});
    setOther({});
    setError(null);
    setStep(questions()[0]?.id ?? "");
  }));

  const answerFor = (q: AppUserInputQuestion): string[] => {
    const free = (other()[q.id] ?? "").trim();
    if (free) return [free];
    return picked()[q.id] ?? [];
  };
  const answered = (q: AppUserInputQuestion) => answerFor(q).length > 0;
  const required = (q: AppUserInputQuestion) => q.required ?? true;

  const complete = createMemo(() => {
    const qs = questions();
    return qs.length > 0
      && qs.every((q) => !required(q) || answered(q))
      && qs.some(answered);
  });

  const nextStep = (q: AppUserInputQuestion) => {
    const qs = questions();
    const next = qs[qs.findIndex((x) => x.id === q.id) + 1];
    setStep(next ? next.id : REVIEW);
  };

  const choose = (q: AppUserInputQuestion, value: string) => {
    setPicked((prev) => {
      const current = prev[q.id] ?? [];
      if (q.multi) {
        return {
          ...prev,
          [q.id]: current.includes(value) ? current.filter((v) => v !== value) : [...current, value],
        };
      }
      return { ...prev, [q.id]: [value] };
    });
    // Picking an option supersedes anything typed into the free-text box.
    setOther((prev) => ({ ...prev, [q.id]: "" }));
    if (multiStep() && !q.multi) nextStep(q);
  };

  const finish = (answers: Record<string, string[]> | null, after?: () => void) => {
    const req = request();
    if (!req || submitting()) return;
    const sid = props.activeSession()?.id;
    setSubmitting(true);
    setError(null);
    void assistantApi
      .respondUserInput(req.requestId, answers)
      .then(() => {
        if (props.activeSession()?.id !== sid) return;
        props.patchActiveSession({
          pendingUserInput: (props.activeSession()?.pendingUserInput ?? []).filter(
            (q) => q.requestId !== req.requestId,
          ),
        });
        setPicked({});
        setOther({});
        after?.();
      })
      .catch((e) => setError(`Could not submit answer: ${String(e)}`))
      .finally(() => setSubmitting(false));
  };

  const submit = () => {
    const answers: Record<string, string[]> = {};
    for (const q of questions()) answers[q.id] = answerFor(q);
    finish(answers);
  };

  const OptionList = (p: { q: AppUserInputQuestion }) => {
    const selected = (value: string) => (picked()[p.q.id] ?? []).includes(value);
    const preview = () => p.q.options.find((o) => selected(o.value))?.preview;
    return (
      <div class="space-y-2">
        <Show when={p.q.header}>
          <div class="text-[10px] font-semibold uppercase tracking-wide theme-muted">{p.q.header}</div>
        </Show>
        <div class="text-[12.5px] theme-text">{p.q.prompt}</div>
        <Show when={p.q.options.length > 0}>
          <div class="space-y-1">
            <For each={p.q.options}>
              {(opt) => (
                <RowButton
                  class="user-input-option"
                  classList={{ "is-selected": selected(opt.value) }}
                  aria-pressed={selected(opt.value)}
                  disabled={submitting()}
                  onClick={() => choose(p.q, opt.value)}
                >
                  <span class="min-w-0 flex-1">
                    <span class="block text-[12px] theme-text">{opt.label}</span>
                    <Show when={opt.description}>
                      <span class="block text-[10.5px] theme-muted">{opt.description}</span>
                    </Show>
                  </span>
                  <Show when={selected(opt.value)}><Check size={14} class="mt-0.5 shrink-0" /></Show>
                </RowButton>
              )}
            </For>
          </div>
          <Show when={preview()}><pre class="user-input-preview">{preview()}</pre></Show>
        </Show>
        <Show when={p.q.allowOther || p.q.options.length === 0}>
          <Input
            type={p.q.secret ? "password" : p.q.inputType === "number" ? "number" : "text"}
            placeholder={p.q.options.length === 0 ? "Your answer" : "Other…"}
            value={other()[p.q.id] ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              setOther((prev) => ({ ...prev, [p.q.id]: v }));
              if (v.trim()) setPicked((prev) => ({ ...prev, [p.q.id]: [] }));
            }}
            class="text-xs"
          />
        </Show>
      </div>
    );
  };

  const Review = () => (
    <div class="space-y-2">
      <For each={questions()}>
        {(q) => (
          <div class="text-[12px]">
            <div class="theme-muted">{q.header || q.prompt}</div>
            <div classList={{ "theme-text": answered(q), "theme-muted": !answered(q) }}>
              {answered(q) ? (q.secret ? "••••••" : answerFor(q).join(", ")) : "Not answered"}
            </div>
          </div>
        )}
      </For>
    </div>
  );

  const body = () => (
    <>
      <Show when={error()}><Alert tone="danger">{error()}</Alert></Show>
      <Show
        when={multiStep()}
        fallback={<For each={questions()}>{(q) => <OptionList q={q} />}</For>}
      >
        <Tabs value={step()} onChange={setStep}>
          <TabsList class="user-input-tabs">
            <For each={questions()}>
              {(q, i) => (
                <TabsTrigger value={q.id}>
                  {q.header || `Question ${i() + 1}`}
                  <Show when={answered(q)}><Check size={11} class="ml-1" /></Show>
                </TabsTrigger>
              )}
            </For>
            <TabsTrigger value={REVIEW}>Review</TabsTrigger>
          </TabsList>
          <For each={questions()}>
            {(q) => <TabsContent value={q.id} class="pt-3"><OptionList q={q} /></TabsContent>}
          </For>
          <TabsContent value={REVIEW} class="pt-3"><Review /></TabsContent>
        </Tabs>
      </Show>
      <div class="mt-3 flex flex-wrap justify-end gap-2 border-t border-[var(--ui-border)] pt-3">
        <Show when={props.onDefer}>
          <Button variant="ghost" disabled={submitting()} onClick={() => props.onDefer?.()}>Later</Button>
        </Show>
        <Button
          variant="ghost"
          size="md"
          class="text-xs"
          disabled={submitting()}
          onClick={() => finish(null, props.onChatAbout ?? focusComposer)}
        >
          Chat about this
        </Button>
        <Button variant="secondary" size="md" class="text-xs" disabled={submitting()} onClick={() => finish(null)}>
          Skip
        </Button>
        <Show when={multiStep() && step() !== REVIEW}>
          <Button size="md" class="text-xs" disabled={submitting()} onClick={() => setStep(REVIEW)}>
            Review
          </Button>
        </Show>
        <Show when={!multiStep() || step() === REVIEW}>
          <Button size="md" class="text-xs" disabled={!complete() || submitting()} onClick={submit}>
            {submitting() ? "Sending…" : multiStep() ? "Submit" : "Answer"}
          </Button>
        </Show>
      </div>
    </>
  );

  return (
    <Show
      when={props.inline}
      fallback={
        <Dialog open={request() !== null} onOpenChange={(open) => { if (!open && !submitting()) { if (props.onDefer) props.onDefer(); else finish(null); } }}>
          <DialogContent
            title={request()?.title ?? "Agent needs your input"}
            description={undefined}
            icon={<MessageCircleQuestion size={20} class="text-[var(--ui-state-info-text)]" />}
          >
            <div class="mt-3 max-h-[60vh] overflow-auto">{body()}</div>
          </DialogContent>
        </Dialog>
      }
    >
      <Show when={request()}>
        <Panel class="user-input-panel my-2 px-3 py-2">
          <Show when={request()?.title}>
            <div class="mb-2 text-xs font-semibold theme-text">{request()?.title}</div>
          </Show>
          {body()}
        </Panel>
      </Show>
    </Show>
  );
}

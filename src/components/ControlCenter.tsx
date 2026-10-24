import { For, Show, createMemo, createSignal, onCleanup } from "solid-js";
import type { JSX } from "solid-js";
import { MessageSquare, RefreshCw, Square } from "lucide-solid";
import type { Project } from "../lib/tauriApi";
import { assistantApi } from "../lib/tauriApi";
import { allowOnceOption, isPlanApproval } from "../lib/permissionOptions";
import type { SessionSummary } from "./types";
import { useControlCenter } from "../hooks/useControlCenter";
import { makeDefaultSession } from "../lib/sessionHelpers";
import { displayTitle, fmtRelative } from "../lib/formatHelpers";
import { Alert, AsyncContent, Badge, Button, Dialog, DialogBody, DialogContent, DialogFooter, IconButton, RowButton, StatusBadge, StatusDot, Textarea } from "./ui";
import { sessionStatus } from "../lib/sessionStatus";
import { PermissionModal } from "./PermissionModal";
import UserInputModal from "./UserInputModal";
import "./controlCenter.css";

export type ControlCenterProps = {
  projects: () => Project[];
  onOpenChat: (id: string) => void;
  onSend: (id: string, text: string) => Promise<boolean>;
  onStop: (id: string) => Promise<unknown>;
  onBrowseSessions: () => void;
  /** Extra header controls; the phone companion has no topbar, so it puts theme and navigation here. */
  headerActions?: JSX.Element;
  /** Own header and "all sessions" link. The desktop already has a topbar and a sidebar, and the
   *  list refreshes itself from engine events, so only the companion shows them. */
  chrome?: boolean;
};

export default function ControlCenter(props: ControlCenterProps) {
  const data = useControlCenter();
  const [selected, setSelected] = createSignal<string | null>(null);
  const [quick, setQuick] = createSignal<SessionSummary | null>(null);
  const [text, setText] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [batching, setBatching] = createSignal(false);
  const [individualBusy, setIndividualBusy] = createSignal(0);
  const [batchResult, setBatchResult] = createSignal<string | null>(null);
  const batchable = createMemo(() => data.pending().permissions.flatMap((permission) => {
    if (isPlanApproval(permission)) return [];
    const option = allowOnceOption(permission.options);
    return option ? [{ requestId: permission.requestId, optionId: option.optionId }] : [];
  }));
  const allowAllOnce = async () => {
    if (batching() || individualBusy() || data.error()) return;
    const requests = batchable();
    if (!requests.length) return;
    setBatching(true);
    setBatchResult(null);
    setActionError(null);
    let approved = 0;
    let failed = 0;
    for (const request of requests) {
      try {
        await assistantApi.respondPermission(request.requestId, request.optionId, false);
        approved++;
      } catch { failed++; }
    }
    await data.refresh();
    setBatchResult(`${approved} allowed once${failed ? ` · ${failed} failed; review the remaining cards` : ""}.`);
    setBatching(false);
  };
  const pendingFor = (id: string) => ({
    permissions: data.pending().permissions.filter((p) => p.appSessionId === id),
    userInputs: data.pending().userInputs.filter((p) => p.appSessionId === id),
  });
  const needsDecision = (id: string) => {
    const p = pendingFor(id);
    return p.permissions.length + p.userInputs.length > 0;
  };
  const attention = createMemo(() => data.sessions().filter((s) => needsDecision(s.id) || (!!s.lastError && !s.running && !s.externalActive)));
  const working = createMemo(() => data.sessions().filter((s) => !needsDecision(s.id) && (s.running || s.externalActive)));
  const recent = createMemo(() => data.sessions().filter((s) => !needsDecision(s.id) && !s.lastError && !s.running && !s.externalActive).sort((a, b) => b.lastActiveAt - a.lastActiveAt).slice(0, 8));
  const decisionCount = () => data.pending().permissions.length + data.pending().userInputs.length;
  const chosen = createMemo(() => {
    const id = selected();
    if (!id) return null;
    const summary = data.sessions().find((s) => s.id === id);
    return { ...makeDefaultSession(summary?.title ?? "Agent"), id,
      pendingPermissions: pendingFor(id).permissions, pendingUserInput: pendingFor(id).userInputs };
  });
  const projectName = (s: SessionSummary) => props.projects().find((p) => p.id === s.projectId)?.name ?? "No project";
  const openQuick = (s: SessionSummary) => { setText(""); setActionError(null); setQuick(s); };
  let hold: ReturnType<typeof setTimeout> | undefined;
  let held = false;
  let start = { x: 0, y: 0 };
  const cancelHold = () => clearTimeout(hold);
  onCleanup(cancelHold);

  const statusOf = (s: SessionSummary) => sessionStatus({
    running: s.running,
    externalActive: s.externalActive,
    error: s.lastError,
    pendingPermissions: pendingFor(s.id).permissions.length,
    pendingUserInputs: pendingFor(s.id).userInputs.length,
  });

  const row = (s: SessionSummary, decision = false) => {
    const panelSession = createMemo(() => ({ ...makeDefaultSession(s.title), id: s.id,
      pendingPermissions: pendingFor(s.id).permissions, pendingUserInput: pendingFor(s.id).userInputs }));
    return (
    <div class="control-center-row">
      <RowButton
        class="control-center-session"
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          held = false;
          start = { x: e.clientX, y: e.clientY };
          cancelHold();
          hold = setTimeout(() => { held = true; openQuick(s); }, 550);
        }}
        onPointerMove={(e) => { if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 10) cancelHold(); }}
        onPointerUp={cancelHold}
        onPointerCancel={cancelHold}
        onContextMenu={(e) => { e.preventDefault(); cancelHold(); held = true; openQuick(s); }}
        onClick={() => { if (held) { held = false; return; } props.onOpenChat(s.id); }}
      >
        <span class="control-center-row-meta">
          <StatusDot status={statusOf(s)} />
          <span class="control-center-row-where">{projectName(s)} · @{s.activeRole}</span>
          <span class="control-center-row-time">{fmtRelative(s.lastActiveAt)}</span>
        </span>
        <span class="control-center-row-title">{displayTitle(s.title)}</span>
        <Show when={decision && !pendingFor(s.id).permissions.length}><span class="control-center-row-preview">{pendingFor(s.id).userInputs[0]?.title ?? s.lastError ?? "Your answer is needed"}</span></Show>
        <Show when={!decision && statusOf(s) !== "idle"}>
          <span class="control-center-row-status"><StatusBadge status={statusOf(s)} /></span>
        </Show>
      </RowButton>
      <div class="control-center-row-actions">
        <IconButton title="Quick message" aria-label={`Quick message: ${s.title}`} onClick={() => openQuick(s)}><MessageSquare size={16} /></IconButton>
        <Show when={s.running}>
          <IconButton title="Stop agent" aria-label={`Stop: ${s.title}`} onClick={() => {
            void props.onStop(s.id).then(() => data.refresh()).catch((e) => setActionError(String(e)));
          }}><Square size={14} /></IconButton>
        </Show>
      </div>
      <Show when={decision && panelSession().pendingPermissions.length > 0}>
        <PermissionModal compact activeSession={panelSession} disabled={batching() || individualBusy() > 0} onSubmittingChange={(busy) => setIndividualBusy((n) => Math.max(0, n + (busy ? 1 : -1)))} patchActiveSession={() => { void data.refresh(); }} />
      </Show>
      <Show when={decision && panelSession().pendingUserInput.length > 0}>
        <Button variant="secondary" class="control-center-answer" onClick={() => setSelected(s.id)}>Answer questions</Button>
      </Show>
    </div>
    );
  };

  return (
    <main class="control-center">
      <Show when={props.chrome !== false}>
        <header class="control-center-header"><strong>Jockey</strong>{props.headerActions}<IconButton title="Refresh" aria-label="Refresh control center" onClick={() => void data.refresh()}><RefreshCw size={17} /></IconButton></header>
      </Show>
      <div class="control-center-scroll">
        <div class="control-center-column" classList={{ "is-empty": !data.loading() && !data.error() && data.sessions().length === 0 }}>
        <div class="control-center-intro">
          <h1>{data.loading() ? "Connecting to your agents…" : data.error() ? "Connection needs attention" : data.sessions().length === 0 ? "No active sessions" : decisionCount() ? `${decisionCount()} decision${decisionCount() === 1 ? "" : "s"} waiting` : attention().length ? "Your agents need attention" : "You’re all caught up"}</h1>
        </div>
        <Show when={data.error() || actionError()}><Alert tone="danger">{data.error() || actionError()} <Button variant="ghost" onClick={() => { setActionError(null); void data.refresh(); }}>Retry</Button></Alert></Show>
        <Show when={batchable().length > 1}><div class="control-center-batch"><Button variant="secondary" disabled={batching() || individualBusy() > 0 || !!data.error()} onClick={() => void allowAllOnce()}>{batching() ? "Submitting decisions…" : `Allow all once (${batchable().length})`}</Button></div></Show>
        <Show when={batchResult()}><p class="control-center-batch-result" role="status">{batchResult()}</p></Show>
        <AsyncContent state={data.loading() ? "loading" : "ready"}>
          <Show when={attention().length}><section class="control-center-section"><h2>Needs you <Badge tone="warning">{attention().length}</Badge></h2><For each={attention()}>{(s) => row(s, true)}</For></section></Show>
          <Show when={working().length}><section class="control-center-section"><h2>In progress <Badge tone="info">{working().length}</Badge></h2><For each={working()}>{(s) => row(s)}</For></section></Show>
          <Show when={recent().length}><section class="control-center-section"><h2>Recent sessions</h2><For each={recent()}>{(s) => row(s)}</For></section></Show>
        </AsyncContent>
        <Show when={props.chrome !== false}>
          <Button variant="secondary" class="control-center-browse" onClick={props.onBrowseSessions}>All projects & sessions</Button>
        </Show>
        </div>
      </div>
      <Show when={chosen()}>
        <UserInputModal activeSession={chosen} onChatAbout={(() => { const id = selected(); return () => { const target = data.sessions().find((x) => x.id === id); if (target) openQuick(target); }; })()} onDefer={() => setSelected(null)} patchActiveSession={() => { setSelected(null); void data.refresh(); }} />
      </Show>
      <Dialog open={!!quick()} onOpenChange={(o) => { if (!o && !sending()) setQuick(null); }}>
        <DialogContent title="Quick message" description={quick()?.title}>
          <form onSubmit={(e) => {
            e.preventDefault();
            const target = quick();
            if (!target || !text().trim() || sending()) return;
            setSending(true);
            setActionError(null);
            void props.onSend(target.id, text().trim()).then((ok) => {
              if (ok) { setQuick(null); setText(""); void data.refresh(); }
              else setActionError("Message was not sent. Your draft is preserved.");
            }).catch((err) => setActionError(String(err))).finally(() => setSending(false));
          }}>
            <DialogBody><Show when={actionError()}><Alert tone="danger">{actionError()}</Alert></Show><Textarea class="control-center-message" aria-label="Quick message" placeholder="Give a direction or ask for an update…" value={text()} onInput={(e) => setText(e.currentTarget.value)} rows={4} />
            </DialogBody>
            <DialogFooter><Button variant="secondary" disabled={sending()} onClick={() => setQuick(null)}>Cancel</Button><Button type="submit" disabled={!text().trim() || sending()}>{sending() ? "Sending…" : "Send message"}</Button></DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </main>
  );
}

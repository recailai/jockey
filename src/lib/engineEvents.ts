import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppMessage,
  AppSession,
  MessageAddedEvent,
  ProjectChangedEvent,
  QueueChangedEvent,
  SessionChangedEvent,
  QueuedItem,
  LiveTurn,
  TurnProgressEvent,
  TurnFinishedEvent,
  TurnStartedEvent,
} from "../components/types";
import { now } from "../components/types";
import { sessionFromRaw } from "./sessionHelpers";
import { appSessionApi } from "./tauriApi";

type EngineEventsInput = {
  sessions: AppSession[];
  getSessionIndex: (id: string) => number;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  setQueuedItems: (id: string, items: QueuedItem[]) => void;
  appendSessions: (fresh: AppSession[]) => void;
  /** Drop a session another client closed; picks a new active session if needed. */
  removeSession: (id: string) => void;
  adoptMessages: (sessionId: string, messages: readonly AppMessage[]) => void;
  appendMessageLocal: (sessionId: string, message: AppMessage) => void;
  settleSessionStream: (sessionId: string, messages: readonly AppMessage[], expectedRunToken?: number) => void;
  acceptingStreams: Map<string, number>;
  nextRunToken: () => number;
  clearSessionStream: (sid: string) => void;
  refreshProjects: () => Promise<void>;
  showToast: (message: string, severity?: "error" | "info") => void;
};


type LiveTurnInput = Pick<
  EngineEventsInput,
  "getSessionIndex" | "adoptMessages" | "updateSession" | "acceptingStreams"
>;

/**
 * Show a turn this window did not see start (opened mid-turn, reloaded, switched in): its
 * snapshot replaces the checkpoint row under the same message id, and `turn/finished` settles
 * it like any other turn. Windows following the live frames ignore it.
 */
// A snapshot requested before its turn ended can arrive after `turn/finished`; it must not
// overwrite the persisted record or mark the session running again.
const finishedTurns = new Set<string>();

export function adoptLiveTurn(input: LiveTurnInput, sid: string, live: LiveTurn | null) {
  if (!live || finishedTurns.has(live.turnId)) return;
  if (input.getSessionIndex(sid) === -1 || input.acceptingStreams.has(sid)) return;
  input.adoptMessages(sid, [live.message]);
  input.updateSession(sid, { submitting: true, status: "running", turnPhase: "running" });
}

/**
 * The desktop's view of the engine. Every turn — sent from this window, the web companion,
 * or drained from the queue — is opened, streamed and settled from the same `turn/*` events,
 * so this window never needs to know who started a turn. The live projection is opened under
 * the engine's message id, and `turn/finished` replaces it with the persisted record.
 */
export async function registerEngineEvents(input: EngineEventsInput): Promise<UnlistenFn> {
  const known = (sid: string) => input.getSessionIndex(sid) !== -1;
  const tokenByTurn = new Map<string, number>();

  const unlisteners = await Promise.all([
    listen<TurnStartedEvent>("turn/started", (ev) => {
      const { appSessionId: sid, turnId, assistantMessageId, roleLabel, userMessage, session } = ev.payload;
      if (!known(sid)) return;
      const idx = input.getSessionIndex(sid);
      // The sender already shows its own optimistic bubble (with image data); everyone else
      // gets the announced copy, whose `images` is only a count.
      if (!input.sessions[idx].messages.some((m) => m.id === userMessage.id)) {
        const { images, ...rest } = userMessage;
        input.adoptMessages(sid, [Array.isArray(images) ? { ...rest, images } : rest]);
      }
      input.clearSessionStream(sid);
      const token = input.nextRunToken();
      tokenByTurn.set(turnId, token);
      input.acceptingStreams.set(sid, token);
      input.updateSession(sid, {
        ...session,
        streamingMessage: { id: assistantMessageId, roleName: roleLabel, text: "", at: now() },
        streamingRunToken: token,
        toolCalls: {},
        streamSegments: [],
        currentPlan: null,
        pendingPermissions: [],
        pendingUserInput: [],
        thoughtText: "",
        agentState: undefined,
        lastError: null,
        submitting: true,
        status: "running",
        turnPhase: "running",
      });
    }),
    listen<TurnFinishedEvent>("turn/finished", (ev) => {
      const { appSessionId: sid, turnId, ok, cancelled, error, messages } = ev.payload;
      finishedTurns.add(turnId);
      if (finishedTurns.size > 200) finishedTurns.delete(finishedTurns.values().next().value as string);
      if (!known(sid)) return;
      const token = tokenByTurn.get(turnId);
      tokenByTurn.delete(turnId);
      // Only the turn that owns the live stream may clear it or mark the session idle: events
      // can be delivered late, and a finished turn A must not reset a running turn B. A turn
      // that never began (no token) owns it only when nothing is streaming.
      const owner = input.acceptingStreams.get(sid);
      const ownsSession = token !== undefined ? owner === undefined || owner === token : owner === undefined;
      if (ownsSession) input.settleSessionStream(sid, messages, token);
      else input.adoptMessages(sid, messages);
      if (ownsSession) {
        input.updateSession(sid, {
          submitting: false,
          status: ok || cancelled ? "done" : "error",
          turnPhase: "idle",
        });
      }
      // The engine records the failure as an `event` message in `messages` (adopted above).
      if (!ok && !cancelled && error) input.showToast(error);
    }),
    listen<QueueChangedEvent>("queue/changed", (ev) => {
      const { appSessionId: sid, items } = ev.payload;
      if (!known(sid)) return;
      input.setQueuedItems(sid, items.map((item) => ({
        id: item.id,
        clientId: item.id,
        text: item.text,
        attachments: item.attachments ?? [],
        roleName: item.roleName,
        delivery: item.delivery === "nextStep" ? "nextStep" : "nextTurn",
        createdAt: item.createdAt,
        status: "queued",
      })));
    }),
    listen<TurnProgressEvent>("turn/progress", async (ev) => {
      const sid = ev.payload.appSessionId;
      if (!known(sid) || input.acceptingStreams.has(sid)) return;
      try {
        adoptLiveTurn(input, sid, await appSessionApi.liveTurn(sid));
      } catch {
        // The next progress event or turn/finished catches up.
      }
    }),
    listen<MessageAddedEvent>("message/added", (ev) => {
      if (known(ev.payload.appSessionId)) input.adoptMessages(ev.payload.appSessionId, [ev.payload.message]);
    }),
    // Sessions changed by any client. Own changes arrive here too and are no-ops: the
    // summary matches what this window already shows.
    listen<SessionChangedEvent>("session/changed", async (ev) => {
      const { appSessionId: sid, reason, session } = ev.payload;
      if (reason === "closed") {
        if (known(sid)) input.removeSession(sid);
        return;
      }
      if (known(sid)) {
        if (session) {
          input.updateSession(sid, {
            title: session.title,
            activeRole: session.activeRole,
            runtimeKind: session.runtimeKind,
            cwd: session.cwd,
            externalActive: session.externalActive,
          });
        }
        return;
      }
      // New to this window: load it once, with its transcript.
      try {
        const [raw] = await appSessionApi.getMany([sid]);
        if (raw && !raw.closedAt && !known(sid)) input.appendSessions([sessionFromRaw(raw)]);
      } catch {
        // A failed load only means the session shows up on the next project switch.
      }
    }),
    listen<ProjectChangedEvent>("project/changed", () => {
      void input.refreshProjects();
    }),
  ]);
  return () => unlisteners.forEach((u) => u());
}

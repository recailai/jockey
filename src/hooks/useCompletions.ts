import type { AgentContext } from "./useAgentContext";
import type { SessionManager } from "./useSessionManager";
import { useMentionCompletion } from "./useMentionCompletion";
import { useSlashCompletion } from "./useSlashCompletion";

export function useCompletions(
  agentContext: AgentContext,
  sessionManager: SessionManager,
  input: () => string,
  setInput: (v: string) => void,
  getInputEl: () => HTMLInputElement | undefined,
  onTriggerCommandUi?: (commandName: string) => boolean,
  getCommandList?: () => Array<{ name: string; description?: string }>,
) {
  const {
    roles,
    skills,
    normalizeRuntimeKey,
    commandCacheKey,
    hydrateAgentCommandsForSession,
  } = agentContext;

  const {
    activeSessionId,
    activeSession,
  } = sessionManager;

  const mention = useMentionCompletion(
    input,
    setInput,
    getInputEl,
    activeSessionId,
    roles,
    skills,
  );

  const slash = useSlashCompletion(
    input,
    setInput,
    getInputEl,
    activeSessionId,
    activeSession,
    roles,
    normalizeRuntimeKey,
    commandCacheKey,
    hydrateAgentCommandsForSession,
    onTriggerCommandUi,
    getCommandList,
  );

  const refreshInputCompletions = (value: string, caret: number) => {
    if (slash.extractSlashContext(value, caret)) {
      mention.closeMentionMenu();
      void slash.refreshSlashSuggestions(value, caret);
      return;
    }
    slash.closeSlashMenu();
    void mention.refreshMentionSuggestions(value, caret);
  };


  return {
    mentionOpen: mention.mentionOpen,
    mentionItems: mention.mentionItems,
    mentionActiveIndex: mention.mentionActiveIndex,
    mentionRange: mention.mentionRange,
    slashOpen: slash.slashOpen,
    slashItems: slash.slashItems,
    slashActiveIndex: slash.slashActiveIndex,
    slashRange: slash.slashRange,
    mentionCloseTimerRef: mention.mentionCloseTimerRef,
    mentionDebounceTimerRef: mention.mentionDebounceTimerRef,
    closeMentionMenu: mention.closeMentionMenu,
    closeSlashMenu: slash.closeSlashMenu,
    refreshInputCompletions,
    applyMentionCandidate: mention.applyMentionCandidate,
    applySlashCandidate: slash.applySlashCandidate,
    _setSlashActiveIndex: slash._setSlashActiveIndex,
    _setMentionActiveIndex: mention._setMentionActiveIndex,
  };
}

export type Completions = ReturnType<typeof useCompletions>;

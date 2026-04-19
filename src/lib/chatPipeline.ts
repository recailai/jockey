type ResolveRouteInput = {
  text: string;
  activeRole: string;
  roleNames: string[];
  isCustomRole: boolean;
  defaultRoleAlias: string;
  defaultBackendRole: string;
};

export type ResolveRouteResult = {
  sendRoleLabel: string;
  effectiveRole: string;
  routedText: string;
  isCommand: boolean;
  isAppCommand: boolean;
  inRoleContext: boolean;
  activateRole?: string;
  prefetchRole?: string;
  explicitRoleMention: boolean;
  error?: string;
};

export type AgentControlCmd = "plan" | "act" | "auto" | "cancel";

export const resolveRoute = (input: ResolveRouteInput): ResolveRouteResult => {
  const { text, activeRole, roleNames, isCustomRole, defaultRoleAlias, defaultBackendRole } = input;
  const isCommand = text.startsWith("/");
  const isAppCommand = text.startsWith("/app_");
  let sendRoleLabel = activeRole;
  let effectiveRole = activeRole;
  let inRoleContext = effectiveRole !== defaultRoleAlias && effectiveRole !== defaultBackendRole;
  let routedText = text;
  let activateRole: string | undefined;
  let prefetchRole: string | undefined;
  let explicitRoleMention = false;
  const roleExists = (name: string) => roleNames.includes(name);

  if (!isCommand) {
    const mentionMatch = text.match(/^@(\S+)/);
    const isFileLikeMention = (s: string) =>
      s.startsWith("file:")
      || s.startsWith("dir:")
      || s.includes("/")
      || s.includes(".")
      || s.startsWith("~");
    const isExplicitRole = (s: string) => s.startsWith("role:");
    if (mentionMatch && (isExplicitRole(mentionMatch[1]) || !isFileLikeMention(mentionMatch[1]))) {
      const rawTarget = mentionMatch[1];
      const target = rawTarget.startsWith("role:") ? rawTarget.slice(5) : rawTarget;
      if (
        target === "assistant"
        || target === defaultRoleAlias
        || target === defaultBackendRole
      ) {
        activateRole = defaultRoleAlias;
        sendRoleLabel = defaultRoleAlias;
        effectiveRole = defaultRoleAlias;
        inRoleContext = false;
        routedText = text.replace(/^@\S+\s*/, "").trim();
      } else {
        if (!roleExists(target)) {
          return {
            sendRoleLabel,
            effectiveRole,
            routedText,
            isCommand,
            isAppCommand,
            inRoleContext,
            explicitRoleMention,
            error: `role not found: ${target}`,
          };
        }
        explicitRoleMention = true;
        activateRole = target;
        prefetchRole = target;
        sendRoleLabel = target;
        effectiveRole = target;
        inRoleContext = true;
        routedText = text.replace(/^@\S+\s*/, "").trim();
      }
    } else {
      const startsWithFileMention = !!mentionMatch && !isExplicitRole(mentionMatch[1]) && isFileLikeMention(mentionMatch[1]);
      const needsRoleWrap = isCustomRole && (!text.startsWith("@") || startsWithFileMention);
      if (needsRoleWrap) {
        if (!roleExists(effectiveRole)) {
          return {
            sendRoleLabel,
            effectiveRole,
            routedText,
            isCommand,
            isAppCommand,
            inRoleContext,
            explicitRoleMention,
            error: `active role not found: ${effectiveRole}`,
          };
        }
        routedText = `@${effectiveRole} ${text}`;
      }
    }
  }

  const isRoleSlashCmd = isCommand && inRoleContext && !isAppCommand;
  if (isRoleSlashCmd) {
    if (!roleExists(effectiveRole)) {
      return {
        sendRoleLabel,
        effectiveRole,
        routedText,
        isCommand,
        isAppCommand,
        inRoleContext,
        explicitRoleMention,
        error: `active role not found: ${effectiveRole}`,
      };
    }
    routedText = `@${effectiveRole} ${text}`;
  }

  return {
    sendRoleLabel,
    effectiveRole,
    routedText,
    isCommand,
    isAppCommand,
    inRoleContext,
    activateRole,
    prefetchRole,
    explicitRoleMention,
  };
};

export const parseAgentControlCommand = (
  text: string,
  isCommand: boolean,
  inRoleContext: boolean,
): AgentControlCmd | null => {
  if (!isCommand || inRoleContext) return null;
  const match = text.match(/^\/(plan|act|auto|cancel)\b/);
  if (!match) return null;
  return match[1] as AgentControlCmd;
};

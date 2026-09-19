type ResolveRouteInput = {
  text: string;
  activeRole: string;
  roleNames: string[];
  isCustomRole?: boolean;
  defaultRoleAlias: string;
  defaultBackendRole?: string;
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
  const { text, activeRole, roleNames, defaultRoleAlias } = input;
  const isCommand = text.startsWith("/");
  const isAppCommand = text.startsWith("/app_");
  let sendRoleLabel = activeRole;
  let effectiveRole = activeRole;
  const inRoleContext = true;
  const routedText = text;
  let activateRole: string | undefined;
  let prefetchRole: string | undefined;
  let explicitRoleMention = false;
  const roleExists = (name: string) => roleNames.includes(name);

  if (!isCommand) {
    const isFileLikeMention = (s: string) =>
      s.startsWith("file:")
      || s.startsWith("dir:")
      || s.includes("/")
      || s.includes(".")
      || s.startsWith("~");
    const isExplicitRole = (s: string) => s.startsWith("role:");

    // Match all consecutive mentions at the beginning of the text
    const words = text.trim().split(/\s+/);
    const matchedRoles: string[] = [];

    for (const w of words) {
      if (!w.startsWith("@")) break;
      const rawTarget = w.slice(1);
      if (isExplicitRole(rawTarget) || !isFileLikeMention(rawTarget)) {
        const target = rawTarget.startsWith("role:") ? rawTarget.slice(5) : rawTarget;
        if (
          target === "assistant"
          || target === defaultRoleAlias
          || target.toLowerCase() === defaultRoleAlias.toLowerCase()
        ) {
          matchedRoles.push(defaultRoleAlias);
        } else if (roleExists(target)) {
          matchedRoles.push(target);
        } else {
          return {
            sendRoleLabel,
            effectiveRole,
            routedText,
            isCommand,
            isAppCommand,
            inRoleContext,
            explicitRoleMention: true,
            error: `role not found: ${target}`,
          };
        }
      } else {
        break;
      }
    }

    if (matchedRoles.length > 0) {
      explicitRoleMention = true;
      activateRole = matchedRoles[0];
      prefetchRole = matchedRoles[0];
      effectiveRole = matchedRoles[0];
      sendRoleLabel = matchedRoles.length === 1 ? matchedRoles[0] : matchedRoles.join(" & ");
    }
  }

  if (isCommand && !isAppCommand) {
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
  _inRoleContext?: boolean,
): AgentControlCmd | null => {
  if (!isCommand) return null;
  const match = text.match(/^\/(plan|act|auto|cancel)\b/);
  if (!match) return null;
  return match[1] as AgentControlCmd;
};

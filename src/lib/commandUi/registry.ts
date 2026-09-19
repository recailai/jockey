import type { AppSession, Role } from "../../components/types";
import { DEFAULT_ROLE_ALIAS } from "../../components/types";
import { assistantApi, projectAgentApi, ruleApi, skillApi } from "../tauriApi";
import { readRuntimeOptions, findModelOption } from "../runtimeOptions";
import type {
  CommandDecoration,
  CommandContribution,
  CommandUiSpec,
} from "./contract";

export interface CommandUiContext {
  activeSession: () => AppSession | null;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  roles: () => Role[];
  resetActiveAgentContext?: () => void;
  toggleRightDock?: () => void;
  showToast?: (message: string, tone?: "info" | "success" | "warning" | "danger") => void;
  pushMessage?: (roleName: string, text: string) => void;
  sendRaw?: (text: string, silent?: boolean) => Promise<void>;
  prewarmRoleConfig?: (
    roleName: string,
    sessionId: string,
    projectId?: string | null,
  ) => Promise<{ configOptions: unknown[]; modes: string[] }>;
}

export function createCommandUiRegistry(ctx: CommandUiContext) {
  const decorations = new Map<string, CommandDecoration>();
  const contributions = new Map<string, CommandContribution>();

  // Ensure config options are discovered if active session has none
  const ensureConfigOptions = async (session: AppSession) => {
    if (session.discoveredConfigOptions && session.discoveredConfigOptions.length > 0) {
      return session.discoveredConfigOptions;
    }
    const roleName = session.activeRole || DEFAULT_ROLE_ALIAS;
    if (ctx.prewarmRoleConfig) {
      try {
        const res = await ctx.prewarmRoleConfig(roleName, session.id, session.projectId);
        const options = (res.configOptions ?? []) as any[];
        ctx.patchActiveSession({ discoveredConfigOptions: options });
        return options;
      } catch {
        return [];
      }
    }
    return [];
  };

  // 1. /model: popupSelect to inspect & switch models
  decorations.set("model", {
    name: "model",
    ui: {
      kind: "popupSelect",
      async options(session) {
        const opts = await ensureConfigOptions(session);
        const declared = readRuntimeOptions(opts);
        const modelOpt = findModelOption(declared);
        if (!modelOpt || modelOpt.values.length === 0) {
          return [];
        }
        let currentModel = "";
        try {
          const cfg = await projectAgentApi.getConfig(session.projectId ?? null, session.activeRole || DEFAULT_ROLE_ALIAS);
          currentModel = cfg.configOptions?.model ?? "";
        } catch {
          // fallback to session default
        }
        return modelOpt.values.map((v) => ({
          id: v.value,
          label: v.name,
          detail: v.description,
          badge: v.isDefault ? "default" : v.oneMillion ? "1M" : undefined,
          active: v.value === currentModel || (!currentModel && v.isDefault),
        }));
      },
      async onSelect(option, session) {
        const roleName = session.activeRole || DEFAULT_ROLE_ALIAS;
        await projectAgentApi.setConfig(session.projectId ?? null, roleName, "model", option.id);
        ctx.showToast?.(`Switched model to ${option.label}`, "success");
      },
    },
  });

  // 2. /mode: popupSelect to inspect & switch execution mode
  decorations.set("mode", {
    name: "mode",
    ui: {
      kind: "popupSelect",
      async options(session) {
        const opts = await ensureConfigOptions(session);
        const declared = readRuntimeOptions(opts);
        const modeOpt = declared.find((o) => o.id === "mode");
        if (!modeOpt || modeOpt.values.length === 0) {
          return [];
        }
        const currentMode = session.currentMode || modeOpt.runtimeDefault || "default";
        return modeOpt.values.map((v) => ({
          id: v.value,
          label: v.name,
          detail: v.description,
          active: v.value === currentMode,
        }));
      },
      async onSelect(option, session) {
        const roleName = session.activeRole || DEFAULT_ROLE_ALIAS;
        await assistantApi.setMode(roleName, option.id, session.id);
        ctx.patchActiveSession({ currentMode: option.id });
        ctx.showToast?.(`Switched mode to ${option.label}`, "success");
      },
    },
  });

  // 3. /effort: popupSelect for reasoning effort
  decorations.set("effort", {
    name: "effort",
    ui: {
      kind: "popupSelect",
      async options(session) {
        const opts = await ensureConfigOptions(session);
        const declared = readRuntimeOptions(opts);
        const effortOpt = declared.find((o) => o.id === "effort" || o.id === "reasoning_effort");
        if (!effortOpt || effortOpt.values.length === 0) {
          return [];
        }
        let currentEffort = "";
        try {
          const cfg = await projectAgentApi.getConfig(session.projectId ?? null, session.activeRole || DEFAULT_ROLE_ALIAS);
          currentEffort = cfg.configOptions?.effort ?? cfg.configOptions?.reasoning_effort ?? "";
        } catch {
          // fallback
        }
        return effortOpt.values.map((v) => ({
          id: v.value,
          label: v.name,
          detail: v.description,
          active: v.value === currentEffort || (!currentEffort && v.isDefault),
        }));
      },
      async onSelect(option, session) {
        const roleName = session.activeRole || DEFAULT_ROLE_ALIAS;
        await projectAgentApi.setConfig(session.projectId ?? null, roleName, "effort", option.id);
        ctx.showToast?.(`Reasoning effort set to ${option.label}`, "success");
      },
    },
  });

  // 4. /usage & /cost & /stats: display session token usage & spend
  const usageAction: CommandUiSpec = {
    kind: "action",
    async run(session) {
      const u = session.usage;
      if (!u || (!u.totalTokens && !u.inputTokens && !u.outputTokens && u.costUsd == null)) {
        ctx.pushMessage?.("event", "No token usage recorded yet for this session.");
        ctx.showToast?.("No token usage recorded yet", "info");
        return;
      }
      const fmt = (n: number | null | undefined) =>
        n == null
          ? "0"
          : n >= 1_000_000
          ? `${(n / 1_000_000).toFixed(2)}M`
          : n >= 1_000
          ? `${(n / 1_000).toFixed(1)}k`
          : `${n}`;
      const pressure =
        u.contextWindow && u.totalTokens
          ? Math.min(100, Math.round((u.totalTokens / u.contextWindow) * 100))
          : null;

      const lines = [
        `📊 **Session Telemetry & Usage**`,
        `• **Input tokens**: ${fmt(u.inputTokens)}`,
        `• **Output tokens**: ${fmt(u.outputTokens)}`,
        u.reasoningTokens && u.reasoningTokens > 0 ? `• **Reasoning (thinking)**: ${fmt(u.reasoningTokens)}` : null,
        u.cacheReadTokens && u.cacheReadTokens > 0 ? `• **Cached read**: ${fmt(u.cacheReadTokens)}` : null,
        u.totalTokens ? `• **Total tokens**: ${fmt(u.totalTokens)}` : null,
        pressure !== null ? `• **Context pressure**: ${pressure}% (${fmt(u.totalTokens)} / ${fmt(u.contextWindow)})` : null,
        u.costUsd != null ? `• **Estimated cost**: $${u.costUsd.toFixed(4)}` : null,
      ]
        .filter(Boolean)
        .join("\n");

      ctx.pushMessage?.("event", lines);
    },
  };

  contributions.set("usage", {
    name: "usage",
    label: "Session Token Usage",
    description: "Inspect token usage, context pressure, and cost for the active session",
    ui: usageAction,
  });
  contributions.set("cost", {
    name: "cost",
    label: "Session Spend & Cost",
    description: "Alias for /usage",
    ui: usageAction,
  });
  contributions.set("stats", {
    name: "stats",
    label: "Session Statistics",
    description: "Alias for /usage",
    ui: usageAction,
  });

  // 5. /compact: request context compaction / summarization
  contributions.set("compact", {
    name: "compact",
    label: "Compact Conversation",
    description: "Compact the current conversation context to save tokens",
    ui: {
      kind: "action",
      async run() {
        ctx.pushMessage?.("event", "Requesting conversation context compaction...");
        if (ctx.sendRaw) {
          await ctx.sendRaw("/compact", false);
        }
      },
    },
  });

  // 6. /clear or /reset: action to reset agent session
  contributions.set("clear", {
    name: "clear",
    label: "Clear session context",
    description: "Reset the active agent conversation context",
    ui: {
      kind: "action",
      async run() {
        if (ctx.resetActiveAgentContext) {
          ctx.resetActiveAgentContext();
        }
      },
    },
  });
  contributions.set("reset", {
    name: "reset",
    label: "Reset session context",
    description: "Reset the active agent conversation context",
    ui: {
      kind: "action",
      async run() {
        if (ctx.resetActiveAgentContext) {
          ctx.resetActiveAgentContext();
        }
      },
    },
  });

  // 7. /context: action to toggle context / files panel
  contributions.set("context", {
    name: "context",
    label: "Toggle context panel",
    description: "Open or close the workspace files and git context panel",
    ui: {
      kind: "action",
      async run() {
        if (ctx.toggleRightDock) {
          ctx.toggleRightDock();
        }
      },
    },
  });

  // 8. /help: list available commands
  contributions.set("help", {
    name: "help",
    label: "Command Directory",
    description: "Show available interactive slash commands and usage",
    ui: {
      kind: "action",
      async run() {
        const lines = [
          `💡 **Available Slash Commands**`,
          `• **/model** — Switch LLM model and context window`,
          `• **/mode** — Switch execution mode (plan / act / auto)`,
          `• **/effort** — Set reasoning effort level`,
          `• **/usage** (or **/cost**, **/stats**) — View token usage, context pressure & cost`,
          `• **/compact** — Compact conversation context`,
          `• **/clear** (or **/reset**) — Reset active conversation context`,
          `• **/context** — Toggle workspace files & git context panel`,
          `• **/rules** — View active instructions and project rules`,
          `• **/skills** — View registered agent skills`,
        ];
        ctx.pushMessage?.("event", lines.join("\n"));
      },
    },
  });

  // 9. /rules & /skills: inspect active configuration
  contributions.set("rules", {
    name: "rules",
    label: "Active Rules",
    description: "View active rules and behavioral instructions",
    ui: {
      kind: "action",
      async run(session) {
        const role = ctx.roles().find((r) => r.roleName === (session.activeRole || DEFAULT_ROLE_ALIAS));
        if (!role) {
          ctx.pushMessage?.("event", `No custom rules bound to role **${session.activeRole || DEFAULT_ROLE_ALIAS}**.`);
          return;
        }
        const rules = await ruleApi.listAllRulesForRole(role.id);
        const enabledRules = rules.filter((rule) => rule.enabled);
        if (enabledRules.length === 0) {
          ctx.pushMessage?.("event", `No custom rules bound to role **${session.activeRole || DEFAULT_ROLE_ALIAS}**.`);
          return;
        }
        ctx.pushMessage?.("event", `📜 **Active Rules for ${session.activeRole || DEFAULT_ROLE_ALIAS}**:\n${enabledRules.map((rule) => `• ${rule.name}`).join("\n")}`);
      },
    },
  });

  contributions.set("skills", {
    name: "skills",
    label: "Active Skills",
    description: "View skills configured for this role",
    ui: {
      kind: "action",
      async run(session) {
        const role = ctx.roles().find((r) => r.roleName === (session.activeRole || DEFAULT_ROLE_ALIAS));
        if (!role) {
          ctx.pushMessage?.("event", `No skills bound to role **${session.activeRole || DEFAULT_ROLE_ALIAS}**.`);
          return;
        }
        const skills = await skillApi.listAllSkillsForRole(role.id);
        const enabledSkills = skills.filter((skill) => skill.enabled);
        if (enabledSkills.length === 0) {
          ctx.pushMessage?.("event", `No skills bound to role **${session.activeRole || DEFAULT_ROLE_ALIAS}**.`);
          return;
        }
        ctx.pushMessage?.("event", `⚡ **Bound Skills for ${session.activeRole || DEFAULT_ROLE_ALIAS}**:\n${enabledSkills.map((skill) => `• ${skill.name}`).join("\n")}`);
      },
    },
  });

  const getCommandUi = (name: string): CommandUiSpec | undefined => {
    const clean = name.toLowerCase().replace(/^\//, "").trim();
    return decorations.get(clean)?.ui ?? contributions.get(clean)?.ui;
  };

  const getCommandList = (): Array<{ name: string; label?: string; description?: string; kind: string }> => {
    const list: Array<{ name: string; label?: string; description?: string; kind: string }> = [];
    for (const [name, dec] of decorations) {
      list.push({
        name,
        label: name,
        description: dec.ui.kind === "popupSelect" ? `Configure /${name}` : undefined,
        kind: dec.ui.kind,
      });
    }
    for (const [name, contrib] of contributions) {
      list.push({
        name,
        label: contrib.label ?? name,
        description: contrib.description,
        kind: contrib.ui.kind,
      });
    }
    return list;
  };

  return {
    decorations,
    contributions,
    getCommandUi,
    getCommandList,
  };
}

export type CommandUiRegistry = ReturnType<typeof createCommandUiRegistry>;

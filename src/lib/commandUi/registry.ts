import type { AppSession, Role } from "../../components/types";
import { DEFAULT_ROLE_ALIAS } from "../../components/types";
import { assistantApi, projectAgentApi } from "../tauriApi";
import { readRuntimeOptions, findModelOption } from "../runtimeOptions";
import type {
  CommandDecoration,
  CommandContribution,
  CommandUiSpec,
  SelectOption,
  PopupSelectSpec,
  ActionSpec,
} from "./contract";

export interface CommandUiContext {
  activeSession: () => AppSession | null;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  roles: () => Role[];
  resetActiveAgentContext?: () => void;
  toggleRightDock?: () => void;
  showToast?: (message: string, tone?: "info" | "success" | "warning" | "danger") => void;
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
          const cfg = await projectAgentApi.getConfig(session.projectId, session.activeRole || DEFAULT_ROLE_ALIAS);
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
        await projectAgentApi.setConfig(session.projectId, roleName, "model", option.id);
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
          const cfg = await projectAgentApi.getConfig(session.projectId, session.activeRole || DEFAULT_ROLE_ALIAS);
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
        await projectAgentApi.setConfig(session.projectId, roleName, "effort", option.id);
        ctx.showToast?.(`Reasoning effort set to ${option.label}`, "success");
      },
    },
  });

  // 4. /clear or /reset: action to reset agent session
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

  // 5. /context: action to toggle context / files panel
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

  const getCommandUi = (name: string): CommandUiSpec | undefined => {
    const clean = name.toLowerCase().replace(/^\//, "").trim();
    return decorations.get(clean)?.ui ?? contributions.get(clean)?.ui;
  };

  return {
    decorations,
    contributions,
    getCommandUi,
  };
}

export type CommandUiRegistry = ReturnType<typeof createCommandUiRegistry>;

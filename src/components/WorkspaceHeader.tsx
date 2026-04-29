import { openUrl } from "@tauri-apps/plugin-opener";
import { For, Show, createMemo, createSignal } from "solid-js";
import {
  ChevronDown,
  Check,
  Folder,
  GitBranch,
  GitBranchPlus,
  GitCommitHorizontal,
  GitPullRequestCreate,
  LoaderCircle,
  Plus,
  Play,
  Terminal as TerminalIcon,
  Upload,
  X,
} from "lucide-solid";
import type { Accessor, JSX, Setter } from "solid-js";
import type { AppSession, AssistantRuntime, Role } from "./types";
import { DEFAULT_ROLE_ALIAS, INTERACTIVE_MOTION, RUNTIME_COLOR } from "./types";
import type { GitStatusStore } from "../hooks/useGitPoller";
import { gitApi, parseError, workspaceApi, type WorkspaceOpenTarget } from "../lib/tauriApi";
import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DropdownContent,
  DropdownDescription,
  DropdownItem,
  DropdownLabel,
  DropdownMenu,
  DropdownSeparator,
  DropdownTrigger,
  Input,
  RowButton,
  SplitButton,
  SplitButtonMain,
  Switch as UiSwitch,
  Textarea,
  ToolbarButton,
} from "./ui";

export type WorkspaceToolPanel = "files" | "git" | "terminal" | "commit";

type WorkspaceHeaderProps = {
  sessions: AppSession[];
  activeSessionId: Accessor<string | null>;
  setActiveSessionId: Setter<string | null>;
  activeSession: Accessor<AppSession | null>;
  leftRailOpen: Accessor<boolean>;
  roles: Accessor<Role[]>;
  assistants: Accessor<AssistantRuntime[]>;
  gitStatus: () => GitStatusStore;
  gitChangeCount: () => number;
  activeToolPanel: Accessor<WorkspaceToolPanel | null>;
  onNewSession: () => void;
  onCloseSession: (id: string) => void;
  onToggleToolPanel: (panel: WorkspaceToolPanel) => void;
  onSelectRole: (roleName: string) => void;
  onCancelRun: () => void;
  onRunAction: (command: string) => void;
  onRefreshGit?: () => void;
};

const ACTION_STORAGE_KEY = "jockey:toolbar.action";
const IDE_STORAGE_KEY = "jockey:toolbar.ide";
type ToolbarAction = { name: string; command: string };
type GitFlowStep = "commit" | "push" | "pr";
type GitPrimaryActionKind = "commit" | "push" | "pr" | "changes" | "disabled";
type WorkspaceOpenOption = { target: WorkspaceOpenTarget; label: string; icon: string; tone: string };

const IDE_OPTIONS: WorkspaceOpenOption[] = [
  { target: "vscode", label: "VS Code", icon: "VS", tone: "vscode" },
  { target: "cursor", label: "Cursor", icon: "C", tone: "cursor" },
  { target: "sublime", label: "Sublime Text", icon: "S", tone: "sublime" },
  { target: "zed", label: "Zed", icon: "Z", tone: "zed" },
  { target: "antigravity", label: "Antigravity", icon: "A", tone: "antigravity" },
  { target: "finder", label: "Finder", icon: "F", tone: "finder" },
  { target: "terminal", label: "Terminal", icon: ">", tone: "terminal" },
];

function loadToolbarAction(): ToolbarAction {
  try {
    const raw = window.localStorage.getItem(ACTION_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed.name === "string" && typeof parsed.command === "string") {
        return {
          name: parsed.name.trim() || "Run",
          command: parsed.command.trim() || "pnpm tauri dev",
        };
      }
    }
  } catch { /* ignore */ }
  return { name: "Run", command: "pnpm tauri dev" };
}

function loadIdeTarget(): WorkspaceOpenTarget {
  try {
    const raw = window.localStorage.getItem(IDE_STORAGE_KEY);
    if (IDE_OPTIONS.some((option) => option.target === raw)) return raw as WorkspaceOpenTarget;
  } catch { /* ignore */ }
  return "vscode";
}

function statusView(gitStatus: GitStatusStore) {
  const s = gitStatus.state;
  return s && s.kind === "status" ? s : null;
}

function sessionStatusClass(session: AppSession): string {
  if (session.status === "error") return "ui-status-danger";
  return "ui-status-muted";
}

function sessionIsRunning(session: AppSession): boolean {
  return session.submitting || session.status === "running";
}

function sessionStatusTitle(session: AppSession): string {
  if (sessionIsRunning(session)) return "Running";
  if (session.status === "error") return "Error";
  return "Idle";
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

function basename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}

function isPrimaryBranch(branch: string | null | undefined): boolean {
  return branch === "main" || branch === "master";
}

function renderGitPrimaryIcon(kind: GitPrimaryActionKind, busy: boolean): JSX.Element {
  if (busy) return <LoaderCircle size={14} class="ui-running-spinner" />;
  switch (kind) {
    case "push":
      return <Upload size={14} />;
    case "pr":
      return <GitPullRequestCreate size={14} />;
    case "changes":
      return <GitBranch size={14} />;
    default:
      return <GitCommitHorizontal size={14} />;
  }
}

export default function WorkspaceHeader(props: WorkspaceHeaderProps) {
  const [roleOpen, setRoleOpen] = createSignal(false);
  const [action, setAction] = createSignal<ToolbarAction>(loadToolbarAction());
  const [actionMenuOpen, setActionMenuOpen] = createSignal(false);
  const [actionEditorOpen, setActionEditorOpen] = createSignal(false);
  const [draftActionName, setDraftActionName] = createSignal(action().name);
  const [draftActionCommand, setDraftActionCommand] = createSignal(action().command);
  const [gitMenuOpen, setGitMenuOpen] = createSignal(false);
  const [gitActionError, setGitActionError] = createSignal<string | null>(null);
  const [gitFlowOpen, setGitFlowOpen] = createSignal(false);
  const [gitFlowStep, setGitFlowStep] = createSignal<GitFlowStep>("commit");
  const [includeUnstaged, setIncludeUnstaged] = createSignal(true);
  const [commitMessage, setCommitMessage] = createSignal("");
  const [prDraft, setPrDraft] = createSignal(true);
  const [gitFlowSubmitting, setGitFlowSubmitting] = createSignal(false);
  const [branchEditorOpen, setBranchEditorOpen] = createSignal(false);
  const [draftBranchName, setDraftBranchName] = createSignal("");
  const [selectedIde, setSelectedIde] = createSignal<WorkspaceOpenTarget>(loadIdeTarget());
  const [ideMenuOpen, setIdeMenuOpen] = createSignal(false);
  const [ideBusy, setIdeBusy] = createSignal(false);
  const [ideError, setIdeError] = createSignal<string | null>(null);

  const activeRole = () => props.activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS;
  const roleOptions = createMemo(() => {
    const seen = new Set<string>([DEFAULT_ROLE_ALIAS]);
    const options: Array<{ roleName: string; runtimeKind: string | null; model: string | null }> = [
      { roleName: DEFAULT_ROLE_ALIAS, runtimeKind: props.activeSession()?.runtimeKind ?? props.assistants().find((a) => a.available)?.key ?? null, model: null },
    ];
    for (const role of props.roles()) {
      if (seen.has(role.roleName)) continue;
      seen.add(role.roleName);
      options.push({ roleName: role.roleName, runtimeKind: role.runtimeKind, model: role.model });
    }
    return options;
  });

  const git = () => statusView(props.gitStatus());
  const dirty = () => props.gitChangeCount();
  const stagedCount = createMemo(() => git()?.staged.length ?? 0);
  const unstagedCount = createMemo(() => (git()?.unstaged.length ?? 0) + (git()?.untracked.length ?? 0));
  const canPush = createMemo(() => {
    const s = git();
    return !!s?.branch && !s.detached;
  });
  const canCreatePr = createMemo(() => {
    const s = git();
    return !!s?.branch && !s.detached && !isPrimaryBranch(s.branch);
  });
  const gitPrimaryAction = createMemo(() => {
    const s = git();
    if (!s) {
      return {
        kind: "disabled" as const,
        label: "Commit",
        title: "Git actions are unavailable for this session",
        disabled: true,
      };
    }
    if (dirty() > 0) {
      return {
        kind: "commit" as const,
        label: "Commit",
        title: `Open changes to prepare a commit (${dirty()} files changed)`,
        disabled: false,
      };
    }
    if (s.branch && !s.detached && (s.ahead > 0 || !s.upstream)) {
      return {
        kind: "push" as const,
        label: "Push",
        title: `Push ${s.branch}`,
        disabled: false,
      };
    }
    if (s.branch && !s.detached && !isPrimaryBranch(s.branch)) {
      return {
        kind: "pr" as const,
        label: "PR",
        title: `Create a pull request for ${s.branch}`,
        disabled: false,
      };
    }
    return {
      kind: "changes" as const,
      label: "Changes",
      title: "Open source control",
      disabled: false,
    };
  });
  const hasCommitableChanges = createMemo(() => {
    const s = git();
    if (!s) return false;
    if (includeUnstaged()) return s.staged.length + s.unstaged.length + s.untracked.length > 0;
    return s.staged.length > 0;
  });
  const willCreateCommit = createMemo(() => dirty() > 0 && hasCommitableChanges());
  const effectiveGitFlowStep = createMemo<GitFlowStep>(() => {
    if (gitFlowStep() === "pr" && !canCreatePr()) return canPush() ? "push" : "commit";
    if (gitFlowStep() === "push" && !canPush()) return "commit";
    return gitFlowStep();
  });
  const canSubmitGitFlow = createMemo(() => {
    const s = git();
    if (gitFlowSubmitting()) return false;
    if (!s) return false;
    if (effectiveGitFlowStep() === "commit") return willCreateCommit();
    if (effectiveGitFlowStep() === "push") return canPush() && (willCreateCommit() || s.ahead > 0 || !s.upstream);
    return canCreatePr();
  });
  const activeIde = createMemo(() => IDE_OPTIONS.find((option) => option.target === selectedIde()) ?? IDE_OPTIONS[0]);
  const autoCommitMessage = () => {
    const s = git();
    if (!s) return "chore: update files";
    const entries = includeUnstaged()
      ? [...s.staged, ...s.unstaged, ...s.untracked]
      : [...s.staged];
    const files = Array.from(new Set(entries.map((entry) => basename(entry.path))));
    if (files.length === 1) return `chore: update ${files[0]}`;
    if (files.length > 1) return `chore: update ${files.length} files`;
    return "chore: update files";
  };
  const runAction = () => {
    const command = action().command.trim();
    if (!command) {
      setDraftActionName(action().name);
      setDraftActionCommand(action().command);
      setActionEditorOpen(true);
      return;
    }
    props.onRunAction(command);
    setActionMenuOpen(false);
  };
  const openActionEditor = () => {
    setDraftActionName(action().name);
    setDraftActionCommand(action().command);
    setActionMenuOpen(false);
    setActionEditorOpen(true);
  };
  const saveAction = () => {
    const next = {
      name: draftActionName().trim() || "Run",
      command: draftActionCommand().trim(),
    };
    setAction(next);
    try { window.localStorage.setItem(ACTION_STORAGE_KEY, JSON.stringify(next)); } catch { /* ignore */ }
    setActionEditorOpen(false);
  };
  const openGitFlow = (preferred?: GitFlowStep) => {
    const s = git();
    if (!s) return;
    const primaryStep = gitPrimaryAction().kind === "push"
      ? "push"
      : gitPrimaryAction().kind === "pr"
        ? "pr"
        : "commit";
    const nextStep = preferred ?? primaryStep;
    setGitFlowStep(nextStep);
    setIncludeUnstaged(s.unstaged.length + s.untracked.length > 0 || s.staged.length === 0);
    setCommitMessage("");
    setPrDraft(nextStep === "pr");
    setGitFlowSubmitting(false);
    setGitActionError(null);
    setGitMenuOpen(false);
    setGitFlowOpen(true);
  };
  const submitGitFlow = async () => {
    const s = git();
    if (!s || gitFlowSubmitting()) return;
    const step = effectiveGitFlowStep();
    const shouldCommit = willCreateCommit();
    const shouldPush = step === "push" || step === "pr";
    const shouldRunPush = shouldPush && (shouldCommit || s.ahead > 0 || !s.upstream);
    const message = commitMessage().trim() || autoCommitMessage();

    if (!shouldCommit && !shouldRunPush && step !== "pr") {
      setGitActionError("No git action is available for the current state.");
      return;
    }
    setGitFlowSubmitting(true);
    setGitActionError(null);
    try {
      const appSessionId = props.activeSession()?.id ?? null;
      if (shouldCommit) {
        await gitApi.commit(appSessionId, {
          message,
          includeUnstaged: includeUnstaged(),
        });
      }
      if (shouldRunPush) {
        await gitApi.push(appSessionId);
      }
      if (step === "pr") {
        const created = await gitApi.createPr(appSessionId, {
          title: message.split(/\r?\n/, 1)[0] || null,
          draft: prDraft(),
        });
        try {
          await openUrl(created.url);
        } catch {
          // PR is already created even if the browser fails to open.
        }
      }
      props.onRefreshGit?.();
      setGitFlowOpen(false);
      setGitMenuOpen(false);
    } catch (err) {
      const parsed = parseError(err);
      setGitActionError(parsed.message);
    } finally {
      setGitFlowSubmitting(false);
    }
  };
  const openCreateBranchEditor = () => {
    setDraftBranchName("");
    setGitActionError(null);
    setGitMenuOpen(false);
    setBranchEditorOpen(true);
  };
  const createBranch = () => {
    const branchName = draftBranchName().trim();
    if (!branchName) return;
    props.onRunAction(`git switch -c ${shellQuote(branchName)}`);
    setBranchEditorOpen(false);
  };
  const openWorkspace = async (target: WorkspaceOpenTarget, persist = true) => {
    const option = IDE_OPTIONS.find((item) => item.target === target);
    if (!option) return;
    if (persist) {
      setSelectedIde(target);
      try { window.localStorage.setItem(IDE_STORAGE_KEY, target); } catch { /* ignore */ }
    }
    setIdeBusy(true);
    setIdeError(null);
    try {
      await workspaceApi.open(target, props.activeSession()?.id ?? null);
      setIdeMenuOpen(false);
    } catch (err) {
      console.error("[workspace-topbar] open workspace failed", err);
      setIdeError(err instanceof Error ? err.message : String(err));
    } finally {
      setIdeBusy(false);
    }
  };
  const runPrimaryGitAction = () => {
    const action = gitPrimaryAction();
    if (action.disabled) return;
    if (action.kind === "changes") {
      props.onToggleToolPanel("git");
      return;
    }
    openGitFlow(action.kind === "push" ? "push" : action.kind === "pr" ? "pr" : "commit");
  };

  return (
    <header
      data-tauri-drag-region
      class="workspace-topbar"
      classList={{ "is-rail-closed": !props.leftRailOpen() }}
    >
      <div class="session-chip-strip">
        <For each={props.sessions}>
          {(session) => (
            <div
              role="button"
              tabindex="0"
              class="session-chip"
              classList={{ "is-active": session.id === props.activeSessionId() }}
              title={session.cwd ?? session.title}
              onClick={() => props.setActiveSessionId(session.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") props.setActiveSessionId(session.id);
              }}
            >
              <Show when={sessionIsRunning(session)} fallback={
                <span class={`ui-status-dot ${sessionStatusClass(session)}`} title={sessionStatusTitle(session)} />
              }>
                <span title="Running">
                  <LoaderCircle size={13} class="ui-running-spinner" />
                </span>
              </Show>
              <span class="truncate">{session.title}</span>
              <Show when={session.id === props.activeSessionId() && props.sessions.length > 1}>
                <button
                  type="button"
                  class="session-chip-close"
                  title="Close AppSession"
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onCloseSession(session.id);
                  }}
                >
                  <X size={12} />
                </button>
              </Show>
            </div>
          )}
        </For>
        <button
          type="button"
          class="session-chip-add"
          onClick={props.onNewSession}
          title="New AppSession"
        >
          <Plus size={15} />
        </button>
      </div>

      <div class="relative ml-2 flex min-w-0 items-center gap-2">
        <DropdownMenu open={roleOpen()} onOpenChange={setRoleOpen}>
          <DropdownTrigger
            variant="plain"
            class={`role-switcher ${INTERACTIVE_MOTION}`}
            title="Switch role inside this AppSession"
          >
            <span class="h-1.5 w-1.5 rounded-full bg-[var(--ui-accent)]" />
            <span class="truncate">{activeRole()}</span>
            <ChevronDown size={13} class="theme-muted" />
          </DropdownTrigger>
          <DropdownContent placement="bottom-start" class="jui-role-menu">
            <DropdownLabel>AppSession Roles</DropdownLabel>
            <For each={roleOptions()}>
              {(role) => (
                <DropdownItem
                  class={role.roleName === activeRole() ? "is-active" : ""}
                  onSelect={() => props.onSelectRole(role.roleName)}
                >
                  <span class="truncate text-[12px] font-medium theme-text">{role.roleName}</span>
                  <Show when={role.runtimeKind}>
                    <span class={`ml-auto shrink-0 font-mono text-[10px] ${RUNTIME_COLOR[role.runtimeKind ?? ""] ?? "theme-muted"}`}>
                      {role.runtimeKind}
                    </span>
                  </Show>
                  <Show when={role.model}>
                    <span class="shrink-0 truncate font-mono text-[10px] theme-muted">{role.model}</span>
                  </Show>
                </DropdownItem>
              )}
            </For>
          </DropdownContent>
        </DropdownMenu>
      </div>

      <div class="ml-auto flex items-center gap-1.5">
        <Show when={git()}>
          {(s) => (
            <button
              type="button"
              class="topbar-branch"
              classList={{ "is-active": props.activeToolPanel() === "git" || props.activeToolPanel() === "commit" }}
              title={`Open source control for ${s().branch ?? "(detached)"}`}
              onClick={() => props.onToggleToolPanel("git")}
            >
              <GitBranch size={14} />
              <span class="hidden max-w-[140px] truncate md:inline">{s().branch ?? "(detached)"}</span>
              <div class="topbar-branch-meta">
                <Show when={s().behind > 0}>
                  <Badge tone="warning" title={`${s().behind} commits behind upstream`}>↓{s().behind}</Badge>
                </Show>
                <Show when={s().ahead > 0}>
                  <Badge tone="info" title={`${s().ahead} commits ahead of upstream`}>↑{s().ahead}</Badge>
                </Show>
              </div>
              <Show when={dirty() > 0}>
                <Badge tone="warning" title={`${dirty()} changed files`}>M{dirty()}</Badge>
              </Show>
            </button>
          )}
        </Show>

        <ToolbarButton
          active={props.activeToolPanel() === "files"}
          onClick={() => props.onToggleToolPanel("files")}
          title="Files"
        >
          <Folder size={15} />
        </ToolbarButton>
        <ToolbarButton
          active={props.activeToolPanel() === "terminal"}
          onClick={() => props.onToggleToolPanel("terminal")}
          title="Terminal"
        >
          <TerminalIcon size={15} />
        </ToolbarButton>
        <Show when={props.activeSession()?.submitting} fallback={
          <DropdownMenu open={actionMenuOpen()} onOpenChange={setActionMenuOpen}>
          <div class="run-action-wrap">
            <SplitButton class="run-action-control">
              <SplitButtonMain
                class="run-action-main"
                title={`Run: ${action().command || action().name}`}
                onClick={runAction}
              >
                <Play size={15} />
              </SplitButtonMain>
              <DropdownTrigger
                variant="plain"
                class="run-action-caret jui-split-button-trigger"
                title="Run actions"
              >
                <ChevronDown size={13} />
              </DropdownTrigger>
            </SplitButton>
            <DropdownContent class="jui-run-action-menu">
              <DropdownItem icon={<Play size={13} />} onSelect={runAction}>
                {action().name}
              </DropdownItem>
              <DropdownDescription>{action().command || "No command configured"}</DropdownDescription>
              <DropdownSeparator />
              <DropdownItem onSelect={openActionEditor}>
                Edit action...
              </DropdownItem>
            </DropdownContent>
          </div>
          </DropdownMenu>
        }>
          <ToolbarButton
            active
            onClick={props.onCancelRun}
            title="Cancel current run"
          >
            <X size={14} />
          </ToolbarButton>
        </Show>
        <DropdownMenu open={ideMenuOpen()} onOpenChange={setIdeMenuOpen}>
        <div class="ide-open-wrap">
          <SplitButton class="ide-open-control">
            <SplitButtonMain
              class="ide-open-main"
              disabled={ideBusy()}
              title={`Open in ${activeIde().label}`}
              onClick={() => { void openWorkspace(activeIde().target, false); }}
            >
              <span class={`ide-app-icon ${activeIde().tone}`}>{activeIde().icon}</span>
            </SplitButtonMain>
            <DropdownTrigger
              variant="plain"
              class="ide-open-caret jui-split-button-trigger"
              title="Open workspace in..."
            >
              <ChevronDown size={13} />
            </DropdownTrigger>
          </SplitButton>
            <DropdownContent class="jui-ide-open-menu">
              <For each={IDE_OPTIONS}>
                {(option) => (
                  <DropdownItem
                    class={`ide-open-row ${option.target === selectedIde() ? "is-active" : ""}`}
                    disabled={ideBusy()}
                    onSelect={() => { void openWorkspace(option.target); }}
                  >
                    <span class={`ide-app-icon ${option.tone}`}>{option.icon}</span>
                    <span>{option.label}</span>
                  </DropdownItem>
                )}
              </For>
              <Show when={ideError()}>
                {(error) => <div class="ide-open-error">{error()}</div>}
              </Show>
            </DropdownContent>
        </div>
        </DropdownMenu>
        <DropdownMenu open={gitMenuOpen()} onOpenChange={setGitMenuOpen}>
        <div class="git-actions-wrap">
          <SplitButton class={`commit-button ${gitMenuOpen() || props.activeToolPanel() === "commit" ? "is-active" : ""}`}>
            <SplitButtonMain
              class="commit-action-main"
              title={gitPrimaryAction().title}
              disabled={gitPrimaryAction().disabled}
              onClick={runPrimaryGitAction}
            >
              {renderGitPrimaryIcon(gitPrimaryAction().kind, false)}
              <span>{gitPrimaryAction().label}</span>
            </SplitButtonMain>
            <DropdownTrigger
              variant="plain"
              class="commit-action-caret jui-split-button-trigger"
              active={gitMenuOpen() || props.activeToolPanel() === "commit"}
              title="More git actions"
              disabled={!git()}
            >
              <ChevronDown size={13} />
            </DropdownTrigger>
          </SplitButton>
            <DropdownContent class="jui-git-actions-menu">
              <DropdownLabel>Git actions</DropdownLabel>
              <DropdownItem icon={<GitCommitHorizontal size={14} />} disabled={dirty() === 0} onSelect={() => openGitFlow("commit")}>
                Commit
              </DropdownItem>
              <DropdownItem
                icon={<Upload size={14} />}
                disabled={!canPush()}
                onSelect={() => openGitFlow("push")}
              >
                Push
              </DropdownItem>
              <DropdownItem
                icon={<GitPullRequestCreate size={14} />}
                disabled={!canCreatePr()}
                onSelect={() => openGitFlow("pr")}
              >
                Create PR
              </DropdownItem>
              <DropdownItem
                icon={<GitBranchPlus size={14} />}
                disabled={!git()}
                onSelect={openCreateBranchEditor}
              >
                Create branch
              </DropdownItem>
              <Show when={gitActionError()}>
                {(error) => <div class="git-actions-error">{error()}</div>}
              </Show>
            </DropdownContent>
        </div>
        </DropdownMenu>
      </div>
      <Dialog open={actionEditorOpen()} onOpenChange={setActionEditorOpen}>
        <DialogContent
          class="run-action-modal"
          title="Edit action"
          description="Update the command that runs from the toolbar"
          icon={<Play size={22} />}
        >
            <label class="run-action-label mt-9">Name</label>
            <div class="run-action-name-row">
              <div class="run-action-name-icon">
                <Play size={18} />
              </div>
              <Input
                value={draftActionName()}
                onInput={(e) => setDraftActionName(e.currentTarget.value)}
                class="run-action-input"
              />
            </div>
            <label class="run-action-label mt-8">Command to run</label>
            <Textarea
              value={draftActionCommand()}
              onInput={(e) => setDraftActionCommand(e.currentTarget.value)}
              class="run-action-command-input"
              spellcheck={false}
            />
            <div class="mt-8 flex items-center justify-end gap-3">
              <Button variant="ghost" size="lg" class="run-action-secondary" onClick={() => setActionEditorOpen(false)}>
                Cancel
              </Button>
              <Button variant="primary" size="lg" class="run-action-save" onClick={saveAction}>
                Save
              </Button>
            </div>
        </DialogContent>
      </Dialog>
      <Dialog open={branchEditorOpen()} onOpenChange={setBranchEditorOpen}>
        <DialogContent
          class="git-branch-modal"
          title="Create branch"
          description="Create and switch to a new branch in this AppSession"
          icon={<GitBranchPlus size={22} />}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              createBranch();
            }}
          >
            <label class="run-action-label mt-8">Branch name</label>
            <Input
              value={draftBranchName()}
              onInput={(e) => setDraftBranchName(e.currentTarget.value)}
              class="git-branch-input"
              placeholder="codex/my-change"
              autofocus
              spellcheck={false}
            />
            <div class="mt-8 flex items-center justify-end gap-3">
              <Button variant="ghost" size="lg" class="run-action-secondary" onClick={() => setBranchEditorOpen(false)}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" size="lg" class="run-action-save" disabled={!draftBranchName().trim()}>
                Create
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
      <Dialog open={gitFlowOpen()} onOpenChange={setGitFlowOpen}>
        <DialogContent
          class="git-flow-modal"
          title={effectiveGitFlowStep() === "pr" ? "Ship your branch" : "Commit your changes"}
          description="Review the git action before sending it to the terminal"
          icon={renderGitPrimaryIcon(effectiveGitFlowStep() === "pr" ? "pr" : effectiveGitFlowStep() === "push" ? "push" : "commit", false)}
        >
          <div class="git-flow-stack">
            <div class="git-flow-summary-grid">
              <div class="git-flow-summary-card">
                <span class="git-flow-summary-label">Branch</span>
                <div class="git-flow-summary-value">
                  <GitBranch size={14} />
                  <span>{git()?.branch ?? "(detached)"}</span>
                </div>
              </div>
              <div class="git-flow-summary-card">
                <span class="git-flow-summary-label">Changes</span>
                <div class="git-flow-summary-value">
                  <span>{dirty()} files</span>
                  <Show when={stagedCount() > 0}>
                    <Badge tone="success">staged {stagedCount()}</Badge>
                  </Show>
                  <Show when={unstagedCount() > 0}>
                    <Badge tone="warning">unstaged {unstagedCount()}</Badge>
                  </Show>
                </div>
              </div>
            </div>

            <Show when={unstagedCount() > 0}>
              <div class="git-flow-toggle-row">
                <UiSwitch
                  checked={includeUnstaged()}
                  onChange={setIncludeUnstaged}
                  label="Include unstaged changes"
                />
              </div>
            </Show>

            <Show when={dirty() > 0}>
              <div class="git-flow-field">
                <div class="git-flow-field-row">
                  <label class="git-flow-field-label">Commit message</label>
                  <span class="git-flow-helper">Blank uses an auto-generated message</span>
                </div>
                <Textarea
                  value={commitMessage()}
                  onInput={(e) => setCommitMessage(e.currentTarget.value)}
                  class="git-flow-message-input"
                  spellcheck={false}
                  rows={3}
                  placeholder={autoCommitMessage()}
                />
              </div>
            </Show>

            <div class="git-flow-field">
              <label class="git-flow-field-label">Next step</label>
              <div class="git-flow-step-list">
                <RowButton
                  active={effectiveGitFlowStep() === "commit"}
                  class="git-flow-step-row"
                  disabled={dirty() === 0}
                  onClick={() => setGitFlowStep("commit")}
                >
                  <div class="git-flow-step-copy">
                    <span class="git-flow-step-title">Commit</span>
                    <span class="git-flow-step-subtitle">Create a local commit only</span>
                  </div>
                  <Show when={effectiveGitFlowStep() === "commit"}>
                    <Check size={16} />
                  </Show>
                </RowButton>
                <RowButton
                  active={effectiveGitFlowStep() === "push"}
                  class="git-flow-step-row"
                  disabled={!canPush()}
                  onClick={() => setGitFlowStep("push")}
                >
                  <div class="git-flow-step-copy">
                    <span class="git-flow-step-title">Commit & push</span>
                    <span class="git-flow-step-subtitle">Commit first, then push to the upstream branch</span>
                  </div>
                  <Show when={effectiveGitFlowStep() === "push"}>
                    <Check size={16} />
                  </Show>
                </RowButton>
                <RowButton
                  active={effectiveGitFlowStep() === "pr"}
                  class="git-flow-step-row"
                  disabled={!canCreatePr()}
                  onClick={() => setGitFlowStep("pr")}
                >
                  <div class="git-flow-step-copy">
                    <span class="git-flow-step-title">Commit & create PR</span>
                    <span class="git-flow-step-subtitle">Push the branch, then open `gh pr create --fill`</span>
                  </div>
                  <Show when={effectiveGitFlowStep() === "pr"}>
                    <Check size={16} />
                  </Show>
                </RowButton>
              </div>
            </div>

            <Show when={effectiveGitFlowStep() === "pr"}>
              <div class="git-flow-toggle-row">
                <UiSwitch
                  checked={prDraft()}
                  onChange={setPrDraft}
                  label="Create draft PR"
                />
              </div>
            </Show>

            <Show when={gitActionError()}>
              {(error) => <div class="git-flow-error">{error()}</div>}
            </Show>

            <div class="git-flow-footer">
              <Button variant="ghost" size="lg" class="run-action-secondary" onClick={() => setGitFlowOpen(false)}>
                Cancel
              </Button>
              <Button variant="primary" size="lg" class="git-flow-submit" disabled={!canSubmitGitFlow()} onClick={() => void submitGitFlow()}>
                {gitFlowSubmitting() ? "Running..." : "Continue"}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </header>
  );
}

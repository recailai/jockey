import { openUrl } from "@tauri-apps/plugin-opener";
import { For, Show, createMemo, createSignal } from "solid-js";
import {
  ChevronDown,
  FileText,
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
import type { Accessor, Setter } from "solid-js";
import type { AppSession, AssistantRuntime, Role } from "./types";
import { DEFAULT_ROLE_ALIAS, INTERACTIVE_MOTION, RUNTIME_COLOR } from "./types";
import type { GitStatusStore } from "../hooks/useGitPoller";
import { gitApi, workspaceApi, type WorkspaceOpenTarget } from "../lib/tauriApi";
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
  SplitButton,
  SplitButtonMain,
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
};

const ACTION_STORAGE_KEY = "jockey:toolbar.action";
const IDE_STORAGE_KEY = "jockey:toolbar.ide";
type ToolbarAction = { name: string; command: string };
type GitActionBusy = "pr" | null;
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

export default function WorkspaceHeader(props: WorkspaceHeaderProps) {
  const [roleOpen, setRoleOpen] = createSignal(false);
  const [action, setAction] = createSignal<ToolbarAction>(loadToolbarAction());
  const [actionMenuOpen, setActionMenuOpen] = createSignal(false);
  const [actionEditorOpen, setActionEditorOpen] = createSignal(false);
  const [draftActionName, setDraftActionName] = createSignal(action().name);
  const [draftActionCommand, setDraftActionCommand] = createSignal(action().command);
  const [gitMenuOpen, setGitMenuOpen] = createSignal(false);
  const [gitActionBusy, setGitActionBusy] = createSignal<GitActionBusy>(null);
  const [gitActionError, setGitActionError] = createSignal<string | null>(null);
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
  const activeIde = createMemo(() => IDE_OPTIONS.find((option) => option.target === selectedIde()) ?? IDE_OPTIONS[0]);
  const pushCommand = () => {
    const s = git();
    if (!s?.branch || s.detached) return "git push";
    if (!s.upstream) return `git push -u origin ${shellQuote(s.branch)}`;
    return "git push";
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
  const openCommitTools = () => {
    props.onToggleToolPanel("commit");
    setGitMenuOpen(false);
  };
  const pushChanges = () => {
    props.onRunAction(pushCommand());
    setGitMenuOpen(false);
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
  const createPullRequest = async () => {
    setGitActionBusy("pr");
    setGitActionError(null);
    try {
      const info = await gitApi.remoteInfo(props.activeSession()?.id ?? null);
      const url = info?.prUrl ?? info?.compareUrl ?? null;
      if (url) {
        await openUrl(url);
      } else {
        props.onRunAction("gh pr create --web");
      }
      setGitMenuOpen(false);
    } catch (err) {
      console.error("[workspace-topbar] create pull request failed", err);
      props.onRunAction("gh pr create --web");
      setGitMenuOpen(false);
    } finally {
      setGitActionBusy(null);
    }
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
            <div class="topbar-branch" title="Current branch">
              <GitBranch size={14} />
              <span class="hidden max-w-[140px] truncate md:inline">{s().branch ?? "(detached)"}</span>
              <Show when={dirty() > 0}>
                <Badge tone="success">+{dirty()}</Badge>
              </Show>
            </div>
          )}
        </Show>

        <ToolbarButton
          active={props.activeToolPanel() === "files"}
          onClick={() => props.onToggleToolPanel("files")}
          title="Files"
        >
          <FileText size={16} />
        </ToolbarButton>
        <ToolbarButton
          active={props.activeToolPanel() === "git" || props.activeToolPanel() === "commit"}
          onClick={() => props.onToggleToolPanel("git")}
          title="Source Control"
        >
          <GitBranch size={16} />
          <Show when={dirty() > 0}>
            <span class="toolbar-dot" />
          </Show>
        </ToolbarButton>
        <ToolbarButton
          active={props.activeToolPanel() === "terminal"}
          onClick={() => props.onToggleToolPanel("terminal")}
          title="Terminal"
        >
          <TerminalIcon size={16} />
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
                <Play size={16} />
              </SplitButtonMain>
              <DropdownTrigger
                variant="plain"
                class="run-action-caret jui-split-button-trigger"
                title="Run actions"
              >
                <ChevronDown size={14} />
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
            <X size={15} />
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
              <ChevronDown size={14} />
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
          <DropdownTrigger
            variant="plain"
            class="commit-button"
            active={gitMenuOpen() || props.activeToolPanel() === "commit"}
            title="Git actions"
          >
            <GitCommitHorizontal size={15} />
            <span>Commit</span>
            <ChevronDown size={13} />
          </DropdownTrigger>
            <DropdownContent class="jui-git-actions-menu">
              <DropdownLabel>Git actions</DropdownLabel>
              <DropdownItem icon={<GitCommitHorizontal size={14} />} onSelect={openCommitTools}>
                Commit
              </DropdownItem>
              <DropdownItem
                icon={<Upload size={14} />}
                disabled={!git()}
                onSelect={pushChanges}
              >
                Push
              </DropdownItem>
              <DropdownItem
                icon={<GitPullRequestCreate size={14} />}
                disabled={!git()}
                onSelect={() => { void createPullRequest(); }}
              >
                {gitActionBusy() === "pr" ? "Opening PR..." : "Create PR"}
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
    </header>
  );
}

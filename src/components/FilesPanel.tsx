import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import {
  ChevronRight,
  Folder,
  FolderOpen,
  RefreshCw,
  Eye,
  EyeOff,
  PanelRightClose,
} from "lucide-solid";
import { fsApi, type DirEntry } from "../lib/tauriApi";
import { useGitChanged } from "../hooks/useGitChanged";
import { EmptyState, ListRow, Panel, PanelBody, PanelHeader, PanelHeaderAction } from "./ui";
import FileGlyph from "./FileGlyph";

type FilesPanelProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  onOpenFile: (relPath: string) => void;
  onCollapse?: () => void;
  embedded?: boolean;
};

type NodeState = {
  expanded: boolean;
  loading: boolean;
  error: string | null;
  children: DirEntry[] | null;
};

const ROOT_KEY = "";
const INDENT_PX = 12;
const BASE_INDENT_PX = 8;

function expandedStorageKey(sid: string, cwd: string): string {
  return `jockey.filesPanel.expanded:${sid}:${cwd}`;
}

function loadExpanded(sid: string, cwd: string): Set<string> {
  try {
    const raw = localStorage.getItem(expandedStorageKey(sid, cwd));
    if (!raw) return new Set([ROOT_KEY]);
    const arr = JSON.parse(raw);
    if (Array.isArray(arr)) return new Set(arr.filter((x) => typeof x === "string"));
  } catch { }
  return new Set([ROOT_KEY]);
}

function saveExpanded(sid: string, cwd: string, set: Set<string>) {
  try {
    localStorage.setItem(expandedStorageKey(sid, cwd), JSON.stringify(Array.from(set)));
  } catch { }
}

function rowStyle(depth: number) {
  const pad = BASE_INDENT_PX + depth * INDENT_PX;
  if (depth <= 0) return { "padding-left": `${pad}px` };
  const guideX = BASE_INDENT_PX + (depth - 1) * INDENT_PX + 5;
  return {
    "padding-left": `${pad}px`,
    "box-shadow": `inset ${guideX}px 0 0 -${guideX - 1}px var(--ui-border)`,
  };
}

type TreeNodeProps = {
  nodeKey: string;
  depth: number;
  nodes: Record<string, NodeState>;
  toggle: (key: string) => void;
  onOpenFile: (key: string) => void;
};

function TreeNode(props: TreeNodeProps) {
  const state = () => props.nodes[props.nodeKey];
  const isExpanded = () => state()?.expanded ?? false;
  const isLoading = () => state()?.loading ?? false;
  const hasNoChildren = () => state()?.children === null;
  const error = () => state()?.error ?? null;
  const children = () => state()?.children ?? [];

  return (
    <Show when={isExpanded()}>
      <Show when={isLoading() && hasNoChildren()}>
        <ListRow class="text-[11px] theme-muted" style={rowStyle(props.depth)}>
          Loading…
        </ListRow>
      </Show>
      <Show when={!isLoading() && error()}>
        <ListRow class="text-[11px] text-[var(--ui-state-danger-text)]" style={rowStyle(props.depth)} title={error()!}>
          <span class="truncate">{error()}</span>
        </ListRow>
      </Show>
      <Show when={!isLoading() || !hasNoChildren()}>
        <For each={children()}>
          {(entry) => {
            const childKey = props.nodeKey ? `${props.nodeKey}/${entry.name}` : entry.name;
            if (entry.isDir) {
              const isOpen = () => !!props.nodes[childKey]?.expanded;
              return (
                <>
                  <ListRow
                    class="file-tree-row"
                    style={rowStyle(props.depth)}
                    classList={{ "opacity-60": entry.name.startsWith(".") }}
                    onClick={() => props.toggle(childKey)}
                  >
                    <ChevronRight
                      size={12}
                      class={`shrink-0 transition-transform ${isOpen() ? "rotate-90" : ""}`}
                    />
                    <Show when={isOpen()} fallback={<Folder size={14} class="file-tree-folder-icon shrink-0" />}>
                      <FolderOpen size={14} class="file-tree-folder-icon shrink-0" />
                    </Show>
                    <span class="truncate">{entry.name}</span>
                  </ListRow>
                  <TreeNode
                    nodeKey={childKey}
                    depth={props.depth + 1}
                    nodes={props.nodes}
                    toggle={props.toggle}
                    onOpenFile={props.onOpenFile}
                  />
                </>
              );
            }
            return (
              <ListRow
                class="file-tree-row"
                style={rowStyle(props.depth + 1)}
                classList={{ "opacity-60": entry.name.startsWith(".") }}
                onClick={() => props.onOpenFile(childKey)}
              >
                <FileGlyph name={entry.name} />
                <span class="truncate">{entry.name}</span>
              </ListRow>
            );
          }}
        </For>
      </Show>
    </Show>
  );
}

export default function FilesPanel(props: FilesPanelProps) {
  const [nodes, setNodes] = createStore<Record<string, NodeState>>({});
  const [showHidden, setShowHidden] = createSignal(false);
  let epoch = 0;

  const ensureNode = (key: string) => {
    if (!nodes[key]) {
      setNodes(key, { expanded: false, loading: false, error: null, children: null });
    }
  };

  const fetchChildren = async (key: string) => {
    const sid = props.appSessionId();
    if (!sid) return;
    const myEpoch = epoch;
    setNodes(key, "loading", true);
    setNodes(key, "error", null);
    try {
      const entries = await fsApi.listDir(sid, key, showHidden());
      if (myEpoch !== epoch) return;
      setNodes(key, "children", entries);
    } catch (e) {
      if (myEpoch !== epoch) return;
      setNodes(key, "error", String(e));
      setNodes(key, "children", []);
    } finally {
      if (myEpoch !== epoch) return;
      setNodes(key, "loading", false);
    }
  };

  const persistExpansion = () => {
    const sid = props.appSessionId();
    const cwd = props.cwd();
    if (!sid || !cwd) return;
    const set = new Set<string>();
    for (const [k, v] of Object.entries(nodes)) {
      if (v?.expanded) set.add(k);
    }
    saveExpanded(sid, cwd, set);
  };

  const reset = () => {
    const sid = props.appSessionId();
    const cwd = props.cwd();
    epoch++;
    setNodes(reconcile({}));
    const toExpand = sid && cwd ? loadExpanded(sid, cwd) : new Set<string>([ROOT_KEY]);
    for (const key of toExpand) {
      ensureNode(key);
      setNodes(key, "expanded", true);
      void fetchChildren(key);
    }
    if (!toExpand.has(ROOT_KEY)) {
      ensureNode(ROOT_KEY);
      setNodes(ROOT_KEY, "expanded", true);
      void fetchChildren(ROOT_KEY);
    }
  };

  const refreshAll = () => {
    for (const [key, node] of Object.entries(nodes)) {
      if (node?.expanded) void fetchChildren(key);
    }
  };

  const sessionKey = createMemo(() => `${props.appSessionId() ?? ""}:${props.cwd() ?? ""}`);
  createEffect(on(sessionKey, () => {
    reset();
  }));

  createEffect(on(showHidden, () => {
    refreshAll();
  }, { defer: true }));

  let gitChangedTimer: ReturnType<typeof setTimeout> | null = null;
  const pendingRefreshKeys = new Set<string>();

  useGitChanged(props.cwd, ({ relPath }) => {
    const slash = relPath.lastIndexOf("/");
    const parent = slash === -1 ? "" : relPath.slice(0, slash);
    for (const key of Object.keys(nodes)) {
      if ((key === parent || (parent && parent.startsWith(key + "/")) || key === "") && nodes[key]?.expanded) {
        pendingRefreshKeys.add(key);
      }
    }
    if (gitChangedTimer !== null) clearTimeout(gitChangedTimer);
    gitChangedTimer = setTimeout(() => {
      gitChangedTimer = null;
      for (const key of pendingRefreshKeys) void fetchChildren(key);
      pendingRefreshKeys.clear();
    }, 300);
  });

  onCleanup(() => {
    if (gitChangedTimer !== null) clearTimeout(gitChangedTimer);
  });

  const toggle = (key: string) => {
    ensureNode(key);
    const wasExpanded = nodes[key].expanded;
    setNodes(key, "expanded", !wasExpanded);
    if (!wasExpanded && nodes[key].children === null) {
      void fetchChildren(key);
    }
    persistExpansion();
  };

  const body = (
    <PanelBody class="tool-panel-body flex-1 overflow-auto">
        <Show when={!props.appSessionId() || !props.cwd()}>
          <EmptyState>
            {!props.appSessionId() ? "No active session" : (
              <><div>No working directory</div><div class="opacity-60 mt-1 font-mono">/app_cd &lt;path&gt;</div></>
            )}
          </EmptyState>
        </Show>
        <Show when={props.appSessionId() && props.cwd()}>
          <TreeNode
            nodeKey={ROOT_KEY}
            depth={0}
            nodes={nodes}
            toggle={toggle}
            onOpenFile={props.onOpenFile}
          />
        </Show>
      </PanelBody>
  );

  if (props.embedded) return body;

  return (
    <Panel class="tool-panel flex h-full flex-col overflow-hidden">
      <PanelHeader class="panel-header">
        <span class="panel-header-title">Files</span>
        <div class="flex items-center gap-0.5">
          <PanelHeaderAction
            title={showHidden() ? "Hide dotfiles" : "Show dotfiles"}
            onClick={() => setShowHidden((v) => !v)}
          >
            <Show when={showHidden()} fallback={<Eye size={13} />}>
              <EyeOff size={13} />
            </Show>
          </PanelHeaderAction>
          <PanelHeaderAction title="Refresh" onClick={() => reset()}>
            <RefreshCw size={13} />
          </PanelHeaderAction>
          <Show when={props.onCollapse}>
            <PanelHeaderAction title="Hide files" onClick={() => props.onCollapse?.()}>
              <PanelRightClose size={13} />
            </PanelHeaderAction>
          </Show>
        </div>
      </PanelHeader>
      {body}
    </Panel>
  );
}

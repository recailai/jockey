import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { X } from "lucide-solid";
import type { AppSession } from "./types";
import PreviewContent from "./PreviewContent";
import { useGitChanged } from "../hooks/useGitChanged";

type PreviewAreaProps = {
  session: () => AppSession | null;
  appSessionId: () => string | undefined;
  onCloseTab: (tabId: string) => void;
  onCloseOthers: (keepTabId: string) => void;
  onCloseAll: () => void;
  onActivateTab: (tabId: string) => void;
  onAddMention?: (path: string) => void;
};

type TabMenu = { tabId: string; x: number; y: number };
type TabState = { version: number; dirty: boolean };

export default function PreviewArea(props: PreviewAreaProps) {
  const tabs = () => props.session()?.previewTabs ?? [];
  const activeId = () => props.session()?.activePreviewTabId ?? null;
  const activeTab = createMemo(() => tabs().find((t) => t.id === activeId()) ?? null);

  const [tabStates, setTabStates] = createStore<Record<string, TabState>>({});
  const [menu, setMenu] = createSignal<TabMenu | null>(null);
  const [rootEl, setRootEl] = createSignal<HTMLDivElement | null>(null);

  createEffect(() => {
    props.appSessionId();
    setTabStates(reconcile({}));
    setMenu(null);
  });

  const stateFor = (id: string): TabState => tabStates[id] ?? { version: 0, dirty: false };
  const versionFor = (id: string): number => stateFor(id).version;
  const isDirty = (id: string): boolean => stateFor(id).dirty;

  const bumpVersion = (id: string) => {
    const cur = stateFor(id);
    setTabStates(id, { version: cur.version + 1, dirty: false });
  };

  useGitChanged(
    () => props.session()?.cwd ?? "",
    ({ relPath }) => {
      const t = activeTab();
      if (!t) return;
      // Only the active tab needs a re-fetch if its own path changed.
      // Diff mode also bumps when index/HEAD shifts, but git/changed only
      // fires on working-tree paths, so path equality is a sufficient test.
      if (relPath === t.path) bumpVersion(t.id);
      for (const other of tabs()) {
        if (other.id === t.id) continue;
        if (other.path === relPath && !isDirty(other.id)) {
          setTabStates(other.id, "dirty", true);
        }
      }
    },
  );

  const handleActivate = (tabId: string) => {
    if (tabId === activeId()) return;
    if (isDirty(tabId)) bumpVersion(tabId);
    props.onActivateTab(tabId);
  };

  const removeTabState = (tabId: string) => {
    setTabStates(tabId, undefined as unknown as TabState);
  };

  const closeMenu = () => setMenu(null);

  onMount(() => {
    const onDocDown = (e: MouseEvent) => {
      if (!menu()) return;
      const tgt = e.target as HTMLElement | null;
      if (tgt && tgt.closest("[data-preview-tab-menu]")) return;
      closeMenu();
    };
    document.addEventListener("mousedown", onDocDown, true);
    document.addEventListener("scroll", closeMenu, true);
    onCleanup(() => {
      document.removeEventListener("mousedown", onDocDown, true);
      document.removeEventListener("scroll", closeMenu, true);
    });
  });

  createEffect(() => {
    const el = rootEl();
    if (!el) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (menu()) { closeMenu(); return; }
      const active = document.activeElement;
      if (!active || !el.contains(active)) return;
      if (tabs().length === 0) return;
      e.preventDefault();
      props.onCloseAll();
    };
    el.addEventListener("keydown", onKey);
    onCleanup(() => el.removeEventListener("keydown", onKey));
  });

  const onTabMiddleClick = (e: MouseEvent, tabId: string) => {
    if (e.button !== 1) return;
    e.preventDefault();
    removeTabState(tabId);
    props.onCloseTab(tabId);
  };

  const onTabContextMenu = (e: MouseEvent, tabId: string) => {
    e.preventDefault();
    setMenu({ tabId, x: e.clientX, y: e.clientY });
  };

  const menuItems = () => {
    const m = menu();
    if (!m) return [];
    const multiple = tabs().length > 1;
    return [
      { label: "Close", onSelect: () => { removeTabState(m.tabId); props.onCloseTab(m.tabId); closeMenu(); } },
      { label: "Close Others", disabled: !multiple, onSelect: () => {
        for (const t of tabs()) if (t.id !== m.tabId) removeTabState(t.id);
        props.onCloseOthers(m.tabId); closeMenu();
      } },
      { label: "Close All", onSelect: () => {
        for (const t of tabs()) removeTabState(t.id);
        props.onCloseAll(); closeMenu();
      } },
    ];
  };

  return (
    <div
      ref={setRootEl}
      tabindex="-1"
      class="flex flex-col h-full overflow-hidden theme-bg outline-none"
    >
      <div class="flex items-stretch border-b theme-border shrink-0 overflow-x-hidden h-[34px] min-w-0">
        <For each={tabs()}>
          {(tab) => {
            const isActive = () => tab.id === activeId();
            return (
              <div
                class={`group relative flex items-center min-w-0 select-none transition-colors ${
                  isActive() ? "theme-text" : "theme-muted hover:theme-text"
                }`}
                style={{
                  "border-right": "1px solid var(--ui-border)",
                  "background-color": isActive() ? "var(--ui-surface-muted)" : "transparent",
                }}
                onMouseDown={(e) => onTabMiddleClick(e, tab.id)}
                onContextMenu={(e) => onTabContextMenu(e, tab.id)}
              >
                <Show when={isActive()}>
                  <span class="absolute top-0 left-0 right-0 h-[2px] bg-[var(--ui-accent)] opacity-80" />
                </Show>
                <button
                  type="button"
                  onClick={() => handleActivate(tab.id)}
                  class="flex items-center gap-1.5 pl-3 pr-1 py-1.5 text-[11.5px] font-mono min-w-0"
                  title={tab.path}
                >
                  <Show when={isDirty(tab.id)}>
                    <span class="h-1.5 w-1.5 rounded-full bg-amber-400 shrink-0" title="changed on disk" />
                  </Show>
                  <span class="truncate">{tab.label}</span>
                </button>
                <button
                  type="button"
                  onClick={(e) => { e.stopPropagation(); removeTabState(tab.id); props.onCloseTab(tab.id); }}
                  class="mr-1 ml-0.5 flex h-5 w-5 items-center justify-center rounded theme-muted hover:theme-text hover:bg-[var(--ui-accent-soft)] transition-opacity opacity-0 group-hover:opacity-100 focus:opacity-100"
                  title="Close (Middle-click)"
                >
                  <X size={12} />
                </button>
              </div>
            );
          }}
        </For>
        <div class="flex-1 min-w-2" />
        <button
          type="button"
          onClick={() => { for (const t of tabs()) removeTabState(t.id); props.onCloseAll(); }}
          title="Close All (Esc)"
          class="shrink-0 icon-btn mr-1.5 self-center"
        >
          <X size={13} />
        </button>
      </div>

      <div class="flex-1 min-h-0">
        <Show when={activeTab()} keyed>
          {(tab) => (
            <PreviewContent
              appSessionId={props.appSessionId}
              cwd={tab.cwd}
              path={tab.path}
              initialMode={tab.initialMode}
              staged={tab.staged}
              untracked={tab.untracked}
              version={() => versionFor(tab.id)}
              onAddMention={props.onAddMention}
            />
          )}
        </Show>
      </div>

      <Show when={menu()} keyed>
        {(m) => (
          <div
            data-preview-tab-menu
            class="fixed z-[90] min-w-[170px] rounded-md border theme-border theme-dropdown py-1 text-[12px]"
            style={{ top: `${m.y}px`, left: `${m.x}px` }}
          >
            <For each={menuItems()}>
              {(item) => (
                <button
                  type="button"
                  disabled={item.disabled}
                  onClick={item.onSelect}
                  class={`flex w-full items-center px-3 py-1.5 text-left theme-dropdown-item ${
                    item.disabled ? "opacity-40 cursor-not-allowed" : ""
                  }`}
                >
                  {item.label}
                </button>
              )}
            </For>
          </div>
        )}
      </Show>
    </div>
  );
}

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type Accessor,
} from "solid-js";
import { Check, Search, Loader2 } from "lucide-solid";
import type { AppSession } from "../types";
import type { PopupSelectSpec, SelectOption } from "../../lib/commandUi/contract";
import { Badge } from "./badge";

export interface PopupSelectViewProps {
  open: Accessor<boolean>;
  commandName: Accessor<string>;
  spec: Accessor<PopupSelectSpec | null>;
  session: Accessor<AppSession | null>;
  onClose: (focusComposer?: boolean) => void;
}

export function PopupSelectView(props: PopupSelectViewProps) {
  const [options, setOptions] = createSignal<readonly SelectOption[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [search, setSearch] = createSignal("");
  const [activeIndex, setActiveIndex] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);

  let cardEl: HTMLDivElement | undefined;
  let searchInputEl: HTMLInputElement | undefined;
  let viewportEl: HTMLDivElement | undefined;

  createEffect(() => {
    if (!props.open()) return;
    const currentSpec = props.spec();
    const currentSession = props.session();
    setSearch("");
    setActiveIndex(0);
    setError(null);
    if (!currentSpec || !currentSession) {
      setOptions([]);
      return;
    }

    setLoading(true);
    let cancelled = false;
    currentSpec
      .options(currentSession)
      .then((opts) => {
        if (cancelled) return;
        setOptions(opts);
        const activeIdx = opts.findIndex((o) => o.active);
        setActiveIndex(activeIdx >= 0 ? activeIdx : 0);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
        setOptions([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    queueMicrotask(() => {
      searchInputEl?.focus();
    });

    onCleanup(() => {
      cancelled = true;
    });
  });

  const filteredOptions = createMemo(() => {
    const q = search().trim().toLowerCase();
    const all = options();
    if (!q) return all;
    return all.filter(
      (o) =>
        o.label.toLowerCase().includes(q) ||
        (o.detail && o.detail.toLowerCase().includes(q)),
    );
  });

  createEffect(() => {
    const len = filteredOptions().length;
    if (len === 0) {
      setActiveIndex(0);
    } else if (activeIndex() >= len) {
      setActiveIndex(len - 1);
    }
  });

  createEffect(() => {
    const idx = activeIndex();
    if (!viewportEl) return;
    const target = viewportEl.children[idx] as HTMLElement | undefined;
    target?.scrollIntoView({ block: "nearest" });
  });

  onMount(() => {
    const onPointerDown = (ev: PointerEvent) => {
      if (!props.open()) return;
      if (cardEl && ev.target instanceof Node && cardEl.contains(ev.target)) {
        return;
      }
      props.onClose(false);
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    onCleanup(() => document.removeEventListener("pointerdown", onPointerDown, true));
  });

  const selectOption = (opt: SelectOption) => {
    const currentSpec = props.spec();
    const currentSession = props.session();
    if (!currentSpec || !currentSession) return;
    void currentSpec.onSelect(opt, currentSession);
    props.onClose(true);
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    const items = filteredOptions();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (items.length > 0) {
        setActiveIndex((i) => (i + 1) % items.length);
      }
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (items.length > 0) {
        setActiveIndex((i) => (i - 1 + items.length) % items.length);
      }
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const current = items[activeIndex()];
      if (current) selectOption(current);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      props.onClose(true);
      return;
    }
  };

  return (
    <Show when={props.open()}>
      <div
        ref={(el) => { cardEl = el; }}
        class="jui-popup-select-card"
        onKeyDown={handleKeyDown}
      >
        <div class="jui-popup-select-header">
          <Search size={13} class="jui-popup-select-search-icon" />
          <input
            ref={(el) => { searchInputEl = el; }}
            type="text"
            class="jui-popup-select-search"
            placeholder={`Select ${props.commandName()}...`}
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
          <span class="jui-popup-select-hint">esc to cancel</span>
        </div>

        <div ref={(el) => { viewportEl = el; }} class="jui-popup-select-viewport">
          <Show when={loading()}>
            <div class="jui-popup-select-status">
              <Loader2 size={14} class="animate-spin opacity-70" />
              <span>Loading {props.commandName()} options...</span>
            </div>
          </Show>

          <Show when={!loading() && error()}>
            <div class="jui-popup-select-status is-error">
              <span>{error()}</span>
            </div>
          </Show>

          <Show when={!loading() && !error() && filteredOptions().length === 0}>
            <div class="jui-popup-select-status">
              <span>No matching options</span>
            </div>
          </Show>

          <Show when={!loading() && !error()}>
            <For each={filteredOptions()}>
              {(option, i) => (
                <button
                  type="button"
                  class="jui-popup-select-row"
                  classList={{ "is-active": i() === activeIndex() }}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    selectOption(option);
                  }}
                  onMouseEnter={() => setActiveIndex(i())}
                >
                  <span class="jui-popup-select-row-main">
                    <span class="jui-popup-select-label">{option.label}</span>
                    <Show when={option.badge}>
                      <Badge tone={option.badge === "default" ? "info" : "neutral"} class="text-[10px] py-0 px-1">
                        {option.badge}
                      </Badge>
                    </Show>
                  </span>
                  <Show when={option.detail}>
                    <span class="jui-popup-select-detail">{option.detail}</span>
                  </Show>
                  <Show when={option.active}>
                    <Check size={14} class="jui-popup-select-check" />
                  </Show>
                </button>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
}

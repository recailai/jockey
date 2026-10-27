import { createSignal } from "solid-js";
import { createCommandUiRegistry, type CommandUiContext } from "../lib/commandUi/registry";
import type { PopupSelectSpec } from "../lib/commandUi/contract";

export function useComposerCommands(context: CommandUiContext, closeSuggestions: () => void, focusInput: () => void) {
  const commandRegistry = createCommandUiRegistry(context);
  const [popupSelectState, setPopupSelectState] = createSignal<{
    open: boolean;
    commandName: string;
    spec: PopupSelectSpec | null;
  }>({ open: false, commandName: "", spec: null });

  const handleTriggerCommandUi = (name: string): boolean => {
    const spec = commandRegistry.getCommandUi(name);
    const session = context.activeSession();
    if (!spec || !session) return false;
    if (spec.kind === "action") {
      void Promise.resolve().then(() => spec.run(session)).catch((error) => context.showToast?.(String(error), "danger"));
    } else if (spec.kind === "popupSelect") {
      closeSuggestions();
      setPopupSelectState({ open: true, commandName: name, spec });
    }
    return true;
  };

  const handleClosePopupSelect = (focusComposer = true) => {
    setPopupSelectState({ open: false, commandName: "", spec: null });
    if (focusComposer) queueMicrotask(focusInput);
  };

  return { commandRegistry, popupSelectState, handleTriggerCommandUi, handleClosePopupSelect };
}

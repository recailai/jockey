import type { Completions } from "../hooks/useCompletions";

export function handleCompletionKeyDown(event: KeyboardEvent, completions: Completions): boolean {
  const slash = completions.slashOpen() && completions.slashItems().length > 0;
  const mention = completions.mentionOpen() && completions.mentionItems().length > 0;
  if (!slash && !mention) return false;
  const items = slash ? completions.slashItems() : completions.mentionItems();
  const index = slash ? completions.slashActiveIndex() : completions.mentionActiveIndex();
  const setIndex = slash ? completions._setSlashActiveIndex : completions._setMentionActiveIndex;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    setIndex((index) => (index + (event.key === "ArrowDown" ? 1 : items.length - 1)) % items.length);
  } else if (event.key === "Tab" || event.key === "Enter") {
    (slash ? completions.applySlashCandidate : completions.applyMentionCandidate)(items[index] ?? items[0]);
  } else if (event.key === "Escape") {
    (slash ? completions.closeSlashMenu : completions.closeMentionMenu)();
  } else {
    return false;
  }
  event.preventDefault();
  return true;
}

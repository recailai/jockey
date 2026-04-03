const HISTORY_MAX = 200;

export function useInputHistory(setInput: (v: string) => void) {
  let history: string[] = [];
  let index = -1;
  let savedInput = "";

  const push = (text: string) => {
    history.unshift(text);
    if (history.length > HISTORY_MAX) history.length = HISTORY_MAX;
    index = -1;
    savedInput = "";
  };

  const resetIndex = () => {
    index = -1;
  };

  /** Returns true if the key was handled and should prevent default. */
  const handleKey = (e: KeyboardEvent, currentInput: () => string): boolean => {
    if (e.key === "ArrowUp" && history.length > 0) {
      if (index === -1) savedInput = currentInput();
      if (index < history.length - 1) {
        index++;
        setInput(history[index]);
      }
      return true;
    }
    if (e.key === "ArrowDown") {
      if (index > 0) {
        index--;
        setInput(history[index]);
      } else if (index === 0) {
        index = -1;
        setInput(savedInput);
      }
      return true;
    }
    return false;
  };

  return { push, resetIndex, handleKey };
}

export type InputHistory = ReturnType<typeof useInputHistory>;

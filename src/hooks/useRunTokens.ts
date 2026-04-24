import { createRunTokenSource, type RunToken } from "../lib/runToken";

export function useRunTokens() {
  const runTokens = createRunTokenSource();

  const getRunToken = () => runTokens.current();
  const bumpRunToken = () => runTokens.next();
  const getCanceledRunToken = () => runTokens.cancelledUpTo();
  const isRunCancelled = (token: RunToken) => runTokens.isCancelled(token);

  const markCancelled = () => runTokens.markCancelled();

  return {
    runTokens,
    getRunToken,
    bumpRunToken,
    getCanceledRunToken,
    isRunCancelled,
    markCancelled,
  };
}

export type RunTokens = ReturnType<typeof useRunTokens>;

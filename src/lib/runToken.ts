export type RunToken = number & { readonly __brand: "RunToken" };

export type RunTokenSource = {
  next: () => RunToken;
  current: () => RunToken;
  cancelledUpTo: () => RunToken;
  markCancelled: () => RunToken;
  isCancelled: (token: RunToken) => boolean;
};

export function createRunTokenSource(): RunTokenSource {
  let seq = 0;
  let cancelledUpTo = 0;

  return {
    next: () => (++seq) as RunToken,
    current: () => seq as RunToken,
    cancelledUpTo: () => cancelledUpTo as RunToken,
    markCancelled: () => {
      cancelledUpTo = Math.max(cancelledUpTo, seq);
      return cancelledUpTo as RunToken;
    },
    isCancelled: (token) => token <= cancelledUpTo,
  };
}

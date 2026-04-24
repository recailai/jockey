import { createSignal, createMemo, createEffect, onMount, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type { Accessor } from "solid-js";
import { gitApi, type GitState } from "../lib/tauriApi";
import { useGitChanged } from "./useGitChanged";
import type { AppSession } from "../components/types";

export type GitStatusStore = {
  state: GitState | null;
  loading: boolean;
  error: string | null;
};

export function useGitPoller(activeSession: Accessor<AppSession | null>): {
  gitStatus: () => GitStatusStore;
  gitChangeCount: Accessor<number>;
  refetch: () => void;
} {
  const [store, setStore] = createStore<GitStatusStore>({
    state: null,
    loading: false,
    error: null,
  });

  const [sessionId, setSessionId] = createSignal<string | null>(null);
  createEffect(() => {
    setSessionId(activeSession()?.id ?? null);
  });

  let inflight = 0;
  let lastSid: string | null = null;

  const refetch = () => {
    const sid = sessionId();
    const mySid = sid;
    const myReq = ++inflight;

    if (mySid !== lastSid) {
      setStore(reconcile({ state: null, loading: true, error: null }));
      lastSid = mySid;
    } else {
      setStore("loading", true);
    }

    void gitApi
      .status(mySid)
      .then((next) => {
        if (myReq !== inflight || mySid !== sessionId()) return;
        setStore("state", reconcile(next as GitState, { merge: true }));
        setStore("loading", false);
        setStore("error", null);
      })
      .catch((err) => {
        if (myReq !== inflight || mySid !== sessionId()) return;
        setStore("loading", false);
        setStore("error", String(err));
      });
  };

  createEffect(() => {
    sessionId();
    refetch();
  });

  useGitChanged(() => activeSession()?.cwd ?? null, () => refetch());

  onMount(() => {
    const t = setInterval(() => refetch(), 5000);
    onCleanup(() => clearInterval(t));
  });

  const gitChangeCount = createMemo(() => {
    const s = store.state;
    if (!s || s.kind !== "status") return 0;
    return s.staged.length + s.unstaged.length + s.untracked.length;
  });

  return {
    gitStatus: () => store,
    gitChangeCount,
    refetch,
  };
}

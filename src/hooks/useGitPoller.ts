import { createResource, createMemo, onMount, onCleanup } from "solid-js";
import type { Accessor, Resource } from "solid-js";
import { gitApi, type GitState } from "../lib/tauriApi";
import { useGitChanged } from "./useGitChanged";
import type { AppSession } from "../components/types";

export function useGitPoller(activeSession: Accessor<AppSession | null>): {
  gitStatusRes: Resource<GitState>;
  gitChangeCount: Accessor<number>;
} {
  const [gitStatusRes, { refetch: refetchGitStatus }] = createResource(
    () => activeSession()?.id ?? null,
    (sid) => gitApi.status(sid),
  );

  const gitChangeCount = createMemo(() => {
    const s = gitStatusRes() as GitState | undefined;
    if (!s || s.kind !== "status") return 0;
    return s.staged.length + s.unstaged.length + s.untracked.length;
  });

  useGitChanged(() => activeSession()?.cwd ?? null, () => { void refetchGitStatus(); });

  onMount(() => {
    const t = setInterval(() => { void refetchGitStatus(); }, 5000);
    onCleanup(() => clearInterval(t));
  });

  return { gitStatusRes, gitChangeCount };
}

import { Show, createResource } from "solid-js";
import type { WorkspaceOpenTarget } from "../lib/tauriApi";
import { workspaceApi } from "../lib/tauriApi";
import { isCustomWorkspaceTarget, workspaceAppFor } from "../lib/workspaceApps";

const remoteIconCache = new Map<string, string | null>();

function remoteIconCacheKey(target: WorkspaceOpenTarget, appName: string, bundleId?: string): string {
  return `${target}:${appName}:${bundleId ?? ""}`;
}

type IdeAppIconProps = {
  target: WorkspaceOpenTarget;
  size?: number;
  class?: string;
};

export default function IdeAppIcon(props: IdeAppIconProps) {
  const app = () => workspaceAppFor(props.target);
  const size = () => props.size ?? 20;
  const bundledIcon = () => app().icon ?? null;
  const needsRemoteIcon = () => isCustomWorkspaceTarget(props.target);
  const remoteCacheKey = () => remoteIconCacheKey(props.target, app().appName, app().bundleId);

  const [remoteIconUrl] = createResource(
    () => (needsRemoteIcon() ? remoteCacheKey() : null),
    async (key) => {
      if (!key) return null;
      const cached = remoteIconCache.get(key);
      if (cached !== undefined) return cached;
      const option = app();
      try {
        const url = await workspaceApi.getAppIcon(props.target, {
          appName: option.appName,
          bundleId: option.bundleId ?? null,
        });
        remoteIconCache.set(key, url);
        return url;
      } catch {
        remoteIconCache.set(key, null);
        return null;
      }
    },
  );

  const resolvedUrl = () => {
    const bundled = bundledIcon();
    if (bundled) return bundled;
    if (!needsRemoteIcon()) return null;
    if (remoteIconUrl.loading || remoteIconUrl.error) return null;
    const url = remoteIconUrl();
    return typeof url === "string" && url.length > 0 ? url : null;
  };

  return (
    <Show
      when={resolvedUrl()}
      fallback={
        <span
          class={`ide-app-icon ${app().tone} ${props.class ?? ""}`}
          style={{ width: `${size()}px`, height: `${size()}px`, "font-size": `${Math.max(8, size() * 0.45)}px` }}
        >
          {app().fallback}
        </span>
      }
    >
      {(url) => (
        <img
          src={url()}
          alt=""
          class={`ide-app-icon-img ${props.class ?? ""}`}
          width={size()}
          height={size()}
          draggable={false}
          onError={() => {
            if (needsRemoteIcon()) remoteIconCache.delete(remoteCacheKey());
          }}
        />
      )}
    </Show>
  );
}

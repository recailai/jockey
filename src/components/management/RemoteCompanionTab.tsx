import { Show, createSignal, onMount } from "solid-js";
import { Check, Copy, ExternalLink, RefreshCw, Shield, Smartphone, Wifi } from "lucide-solid";
import { Alert, Badge, Button, Panel, PanelBody, StateDot, Switch as UiSwitch } from "../ui";
import { gatewayApi, type GatewayStatus } from "../../lib/tauriApi";
import { openUrl } from "@tauri-apps/plugin-opener";

export function RemoteCompanionTab() {
  const [status, setStatus] = createSignal<GatewayStatus | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [copiedKey, setCopiedKey] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const loadStatus = async () => {
    try {
      const s = await gatewayApi.getStatus();
      setStatus(s);
    } catch (e) {
      console.error("Failed to get gateway status:", e);
    }
  };

  onMount(() => {
    void loadStatus();
  });

  const toggleGateway = async (enable: boolean) => {
    setLoading(true);
    setError(null);
    try {
      setStatus(enable ? await gatewayApi.start() : await gatewayApi.stop());
    } catch (e) {
      setError(String(e));
      await loadStatus();
    } finally {
      setLoading(false);
    }
  };

  const regenerateToken = async () => {
    setError(null);
    try {
      setStatus(await gatewayApi.regenerateToken());
    } catch (e) {
      setError(String(e));
    }
  };

  const copyToClipboard = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => setCopiedKey(null), 2000);
    } catch (e) {
      console.error("Copy failed:", e);
    }
  };

  return (
    <div class="space-y-6">
      {/* Main Switch Card */}
      <Panel class="settings-card-list">
        <PanelBody class="settings-card-list-body">
          <div class="settings-toggle-row jui-row-button flex items-center justify-between p-4">
            <div class="min-w-0 pr-4">
              <div class="flex items-center gap-2">
                <span class="text-[14px] font-medium theme-text">启用远程 Web 伴侣</span>
                <Show when={status()?.running}>
                  <Badge tone="success" class="inline-flex items-center gap-1">
                    <StateDot state="done" size={6} />
                    运行中 (Port {status()?.port})
                  </Badge>
                </Show>
              </div>
            </div>
            <UiSwitch
              checked={status()?.running ?? false}
              disabled={loading()}
              onChange={(val) => void toggleGateway(val)}
            />
          </div>
        </PanelBody>
      </Panel>

      <Show when={error()}>
        <Alert tone="danger" title="远程 Web 伴侣操作失败" onClose={() => setError(null)}>
          {error()}
        </Alert>
      </Show>

      {/* Details when running */}
      <Show when={status()?.running}>
        <div class="space-y-4">
          {/* Tailscale / Connection URLs */}
          <div class="rounded-xl border border-[var(--ui-border)] bg-[var(--ui-bg-subtle)] p-4 space-y-3">
            <div class="flex items-center gap-2 text-[13px] font-medium theme-text">
              <Wifi size={16} class="theme-muted" />
              <span>连接地址 (在外通过 Tailscale 访问)</span>
            </div>

            <Show
              when={status()?.tailscaleUrl}
              fallback={
                <Alert tone="warning" title="未检测到 Tailscale 虚拟网卡 (100.x.y.z)">
                  请确保当前电脑已安装并登录 Tailscale；或者在同一 Wi-Fi 下使用下方局域网地址访问。
                </Alert>
              }
            >
              <div class="flex flex-col gap-2 rounded-lg border border-[var(--ui-border)] bg-[var(--ui-bg)] p-3">
                <div class="flex items-center justify-between">
                  <span class="text-[11px] font-medium uppercase tracking-wider theme-text">
                    Tailscale 直连地址 (最佳)
                  </span>
                  <div class="flex gap-2">
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 px-2 text-[12px]"
                      onClick={() =>
                        copyToClipboard(status()!.tailscaleUrl!, "tailscale")
                      }
                    >
                      <Show
                        when={copiedKey() === "tailscale"}
                        fallback={<Copy size={13} class="mr-1" />}
                      >
                        <Check size={13} class="mr-1" />
                      </Show>
                      {copiedKey() === "tailscale" ? "已复制" : "复制链接"}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 px-2 text-[12px]"
                      onClick={() => void openUrl(status()!.tailscaleUrl!)}
                    >
                      <ExternalLink size={13} class="mr-1" />
                      打开
                    </Button>
                  </div>
                </div>
                <div class="font-mono text-[12px] theme-text break-all select-all">
                  {status()?.tailscaleUrl}
                </div>
              </div>
            </Show>

            {/* LAN URLs */}
            <Show when={(status()?.lanUrls ?? []).length > 0}>
              <div class="flex flex-col gap-2 rounded-lg border border-[var(--ui-border)] bg-[var(--ui-bg)] p-3">
                <div class="flex items-center justify-between">
                  <span class="text-[11px] font-medium uppercase tracking-wider theme-muted">
                    局域网地址 (同一 Wi-Fi)
                  </span>
                  <Button
                    variant="ghost"
                    size="sm"
                    class="h-7 px-2 text-[12px]"
                    onClick={() =>
                      copyToClipboard(status()!.lanUrls[0], "lan")
                    }
                  >
                    <Show
                      when={copiedKey() === "lan"}
                      fallback={<Copy size={13} class="mr-1" />}
                    >
                      <Check size={13} class="mr-1" />
                    </Show>
                    {copiedKey() === "lan" ? "已复制" : "复制链接"}
                  </Button>
                </div>
                <div class="font-mono text-[12px] theme-text break-all select-all">
                  {status()?.lanUrls[0]}
                </div>
              </div>
            </Show>

            {/* Localhost URL */}
            <div class="flex flex-col gap-2 rounded-lg border border-[var(--ui-border)] bg-[var(--ui-bg)] p-3">
              <div class="flex items-center justify-between">
                <span class="text-[11px] font-medium uppercase tracking-wider theme-muted">
                  本机测试地址 (Localhost)
                </span>
                <div class="flex gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    class="h-7 px-2 text-[12px]"
                    onClick={() => copyToClipboard(status()!.localUrl, "local")}
                  >
                    <Show
                      when={copiedKey() === "local"}
                      fallback={<Copy size={13} class="mr-1" />}
                    >
                      <Check size={13} class="mr-1" />
                    </Show>
                    {copiedKey() === "local" ? "已复制" : "复制链接"}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    class="h-7 px-2 text-[12px]"
                    onClick={() => void openUrl(status()!.localUrl)}
                  >
                    <ExternalLink size={13} class="mr-1" />
                    打开
                  </Button>
                </div>
              </div>
              <div class="font-mono text-[12px] theme-muted break-all select-all">
                {status()?.localUrl}
              </div>
            </div>
          </div>

          {/* Token Security Card */}
          <div class="rounded-xl border border-[var(--ui-border)] bg-[var(--ui-bg-subtle)] p-4 space-y-3">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2 text-[13px] font-medium theme-text">
                <Shield size={16} class="theme-muted" />
                <span>访问安全令牌 (Access Token)</span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                class="h-7 px-2 text-[12px]"
                onClick={() => void regenerateToken()}
              >
                <RefreshCw size={13} class="mr-1" />
                重新生成
              </Button>
            </div>
            <p class="text-[12px] theme-muted">
              重新生成会让旧链接失效并断开已连接的设备。
            </p>
            <div class="flex items-center justify-between rounded-lg border border-[var(--ui-border)] bg-[var(--ui-bg)] px-3 py-2">
              <span class="font-mono text-[13px] theme-text">
                {status()?.token}
              </span>
              <Button
                variant="ghost"
                size="sm"
                class="h-6 px-2 text-[11px]"
                onClick={() => copyToClipboard(status()!.token, "token_only")}
              >
                <Show
                  when={copiedKey() === "token_only"}
                  fallback={<Copy size={12} class="mr-1" />}
                >
                  <Check size={12} class="mr-1" />
                </Show>
                {copiedKey() === "token_only" ? "已复制" : "复制"}
              </Button>
            </div>
          </div>

          {/* Mobile Usage Tips */}
          <Alert tone="info" title="手机使用小技巧" icon={<Smartphone size={16} />}>
            <ul class="list-disc list-inside space-y-1 text-[12px] leading-relaxed pl-1">
              <li>在手机 Safari 中打开连接后，点击「分享」并选择「添加到主屏幕」，即可作为独立全屏 App 使用。</li>
              <li>当 Agent 执行危险命令或修改文件时，手机顶部会即时弹出卡片，单手拇指即可点【批准】或【拒绝】。</li>
              <li>支持实时流式打字同步，在手机上可随时监控任务进度。</li>
            </ul>
          </Alert>
        </div>
      </Show>
    </div>
  );
}
export default RemoteCompanionTab;

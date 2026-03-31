# JockeyUI Frontend Best Practices (Tauri WebView)

本文只覆盖前端实现实践，重点解决以下问题：
- WebView 页面级滚动导致的白色 gap
- 透明窗口与 Overlay 标题栏的布局稳定性
- 交互动效和流式输出的性能损耗

## 1. 包管理与前端运行基线

1. 固定包管理器为 `pnpm`，并在 `package.json` 使用 `packageManager` 字段锁版本。
2. 只保留 `pnpm-lock.yaml`，不要混用 `package-lock.json`。
3. 统一命令入口：
   - 本地开发：`pnpm tauri dev`
   - 构建：`pnpm build`

## 2. 根布局与视口策略

1. 根层禁止页面级滚动：
   - `html, body, #root` 统一 `height: 100%`
   - `overflow: hidden`
   - `margin: 0`
2. 主容器高度优先使用 `100dvh`，避免动态视口误差。
3. 顶层提供稳定背景层（渐变或纹理），不要依赖默认白底。

示例：

```css
html,
body,
#root {
  height: 100%;
  margin: 0;
  overflow: hidden;
}

body {
  background: transparent;
}

.app-root {
  min-height: 100dvh;
  background: radial-gradient(120% 120% at 50% -10%, #1f1f24 0%, #09090b 100%);
}
```

## 3. 防止“下拉白边”与回弹露底

1. 禁止根容器 overscroll 回弹，避免露出 WebView 默认背景。
2. 只允许内部业务区域滚动（消息列表、侧栏列表）。
3. 页面外层固定，滚动由内层 `overflow: auto` 接管。

示例：

```css
html,
body {
  overscroll-behavior: none;
}

.page-shell {
  height: 100%;
  overflow: hidden;
}

.scroll-region {
  overflow: auto;
}
```

## 4. Overlay 标题栏与拖拽区

1. 使用 macOS Overlay 标题栏时，内容顶部必须预留安全区（traffic lights）。
2. 单独设置拖拽区，且交互控件必须排除拖拽行为。
3. 不把输入框、按钮放在可拖拽层上。

实践建议：
- 维护统一变量：`--window-top-inset`
- 布局高度统一按 `calc(100dvh - var(--window-top-inset) - gap)` 计算

## 5. 视觉材质的性能边界

1. 毛玻璃强度控制在轻量范围（建议 `blur(6~10px)`）。
2. 阴影减少层数，避免多层高半径阴影叠加。
3. 大面积卡片优先半透明 + 轻阴影，不堆叠重特效。

示例：

```css
.card {
  background: rgba(24, 24, 27, 0.66);
  border: 1px solid rgba(255, 255, 255, 0.09);
  backdrop-filter: blur(8px) saturate(1.06);
  -webkit-backdrop-filter: blur(8px) saturate(1.06);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.03), 0 6px 18px rgba(0, 0, 0, 0.28);
}
```

## 6. 交互动效与低延迟策略

1. 禁止 `transition-all`，只过渡必须属性（颜色、透明度、transform）。
2. 时长建议 `150ms~200ms`，避免拖沓和高频重算。
3. 提交态反馈简化为轻量动画（小圆点 pulse + 文案）。
4. 支持 `prefers-reduced-motion` 降级。

示例：

```css
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

## 7. 流式输出渲染规范

1. 不要逐 token 直接刷 UI，使用缓冲 + `requestAnimationFrame` 批量刷新。
2. 缓冲阈值适中（例如 16~32 字符）可平衡延迟和重排成本。
3. 渲染后再滚动到底部，避免连续强制布局。

目标：
- 首 token 尽快出现
- 长输出时 CPU 曲线稳定、界面不抖动

## 8. 前端验收清单（PR 必测）

1. 快速上下滚动消息区，无白边、无页面回弹露底。
2. 连续发送并接收流式输出，帧率稳定、无明显卡顿。
3. 窗口缩放/全屏切换后，主面板高度不跳变。
4. 顶部拖拽区可拖动，输入框和按钮点击不误拖拽。
5. `prefers-reduced-motion` 开启后动画明显收敛。

## 9. 常见故障与定位

1. 看到白色 gap：
   - 检查 `html/body/#root` 是否仍可滚动
   - 检查根层是否设置了 `overscroll-behavior: none`
2. 点击按钮时窗口被拖动：
   - 检查拖拽区域是否覆盖了交互控件
3. 流式输出卡顿：
   - 检查是否仍在逐 token 更新 DOM
   - 检查是否存在 `transition-all` 或高成本阴影动画

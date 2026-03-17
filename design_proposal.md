# UnionAI: AppSession 架构与类拉伸式多标签页终端设计方案 (AppSession & Terminal-like Tabs Design Proposal)

作为产品与设计大师，我完全赞同你的直觉：仅仅把原本的“对话框(Chat)”换个皮仍然停留在表面，**引入 `AppSession` (应用级会话) 的防线抽象，并使用类似现代终端（如 Warp / iTerm2 甚至是浏览器标签）的 Tab 页交互**，这是让产品“脱胎换骨”，获得极客感与专业度的神仙级做法！

当前系统里混淆了底层的 `CLI Session`（后台 Agent 进程）和上层的 `用户界面会话`。我们将重新定义这两个概念，并在交互和 UI/UX 上重塑一切。

---

## 🎯 一、 核心概念界定 (AppSession vs CLI Session)

明确分层是架构和体验设计的基石：

| 概念名称 | 全称 | 对应实体 | 作用与生命周期 | 用户感知 |
| :--- | :--- | :--- | :--- | :--- |
| **AppSession** | 应用级会话层（App Session / Workspace Tab） | Frontend / SharedBrain | **上层容器**。用户开启的一个个 Tabs，对应一个任务上下文。它的生命周期伴随用户的创建、关闭和持久化。 | 顶部的 Tab 标签条（每个 Tab 都有一个名字，如“重构验证模块”）。 |
| **CLI Session** | 进程/协议会话层 (Agent CLI Session) | ACP Transport / Agent | **底层引擎**。负责实际跑 Claude Code / Gemini 模型。多个 Role 可以在后台复用或者挂载到不同的 CLI Session 上。 | **隐形**（除了运行时的小圆点或日志提示外，用户不直接管理它们）。 |

**为什么这么做？**
1. **隔离复杂度**：在同一个 `AppSession`（Tab）里，用户可以任意通过 `@Reviewer` 或 `@Developer` 切换和调用多个 Role的 `CLI Session`。这真正实现了“多 Agent 编排（Orchestration）”。
2. **体验顺滑**：每个 `AppSession` 可以保存独立的消息流（Messages）和上下文状态，类似你打开了多个 Terminal Window 处理不同的项目需求。

---

## 🚀 二、 解决持久化与恢复问题 (The State Recovery Issue)

你觉察到了痛点：“感觉每次启动并不会恢复session”。
**根本原因：** 根据 `todo.md` 中的记录，`SharedBrain` 已经有了 `SQLite` 做冷数据持久化结构（L2 存储），但前台的 UI（Solid.js 的 `messages` 数组）和启动时的流程没有完全打通。

**架构修改建议 (Backend & Frontend State Sync)：**
1. **DB 抽象层级提升**：数据库中不仅需要存储 `teams` 和 `roles` 的关系，更要插入 `app_sessions` 表。
    *   `app_sessions` (`id`, `title`, `workspace_path`, `team_id`, `created_at`, `last_active_at`)
2. **启动恢复 (Hydration)**：前端 `App.tsx` 挂载时，先执行 `invoke("list_app_sessions")`。如果有未关闭的 `app_sessions`，则恢复为一个个 Tab，并加载最近活跃 Tab 的历史消息。

---

## 🎨 三、 交互重塑：类似 Terminal 的分选项卡(Multi-Tab) 终端设计

这是最能展现 “Taste（品味）”的地方。我们摒弃臃肿的“左右两大结构”，走向 **“顶部操作区 (Workspace Tabs) + 沉浸式流式终端舱 (Fluid Terminal Deck)”**。

### 1. 顶部：极简 Tabs 导航栏 (The Top-Bar) 与状态透出
*   类似 VS Code 或 Warp Terminal 的顶部标签页。用细滑的线框区分未激活和激活的 Tab。
*   左上角融入红绿灯（Mac Traffic Lights）。紧靠着就是 `[+] 新建会话` 按钮。
*   **关于分屏（Split-screen）状态监控的克制设计**：
    *   **不要做生硬的分屏**：分屏会导致屏幕极其拥挤，而且像你说的，多个输入框极其反直觉和多余。
    *   **用“会话指示器（Session Indicators）”代替分屏**：后台 AppSession 正在跑一个耗时任务（如自动找 Bug）时，用户切换到别的 Tab 摸鱼，此时那个正在跑的 Tab 标签上会出现一个**微弱的呼吸灯/加载环（Pulse/Spinner）**。
    *   当任务大功告成（或报错中断），Tab 会闪烁或者变成一个蓝色的未读圆点提示你。这样你只需**单屏 + 扫一眼 Tab 栏**就能掌控所有的全局 AppSession 进度，极度优雅。

### 2. 左侧或浮动：全局抽屉面板 (The Sliding Drawer)
*   **原来左侧厚重的侧边栏（Teams / Roles / Assistants）去哪了？** 取消它！
*   这不是一个你频频要在 Teams 之间切换的产品。Teams 和 Roles 是配置层。把它们折叠成左侧一个极窄的边缘条（Icon Only），或者用 `Cmd+K` / 全局设置抽屉（Drawer）的方式唤出。
*   主视图完完全全留给**当前的 AppSession (聊天终端)**。

### 3. 中心：流式终端对话面板 (The Terminal Canvas)
*   这不再是网页那种一个框一个框“堆砌”出来的聊天气泡，而是如同**终端命令行**那样的顺滑输出界面。
*   **用户侧**：每条消息带有一个 `>` 或极简用户名提示。
*   **Role 编排侧**：在同一个 AppSession 里，不同 Role 的输出可以平滑穿插：
    ```text
    > [You] 帮我看看这个 src/index.css 有什么问题？
    [Developer] (claude-code)
    我看了一下，冗余的代码还是挺多的...[处理流]...这里需要修改。

    > [You] @Reviewer 他的做法没毛病吗？
    [Reviewer] (gemini-cli)
    我这里有一点不同的见解...[验证流]...建议采取方案B。
    ```
*   不同的 Agent (`claude-code`, `gemini-cli`) 只通过前边的极其隐秘的标签或者文字颜色进行微弱的区分。

### 4. 底部：胶囊输入舱 (The Command Capsule)
我们不叫它“表单(Form)”，而是“命令输入舱 (Command Capsule)”。
*   悬浮在主内容区底部（`fixed bottom-6 left-1/2 -translate-x-1/2`）的磨砂黑曜石长条胶囊。
*   它融合了 `Type Message / @Role / /Command` 的所有能力。自动完成和提示语做在浮窗（Popover）内。
*   输入时，它会微微发亮 (`focus-within:ring`), 拥有极为细腻的手感和阻尼感。右侧静静躺着一个上箭头（发光点亮代表可以直接 Send）。

---

## 🛠 四、 行动路线：从哪里落刀？

如果你认可这个大师级的 Taste（AppSession + Multi-Tab + 沉浸终端），那么落地步骤如下，我们一层层啃：

1.  **【骨架手术 - UI 重构】**：我可以直接手撕 `App.tsx`，把原先丑陋的侧边栏布局废掉。换成我上面描述的：**Top Tabs + Main Terminal Content + Floating Capsule Input** 的布局结构。哪怕我们先用内存假数据渲染 Tabs（比如写死两个 Tab），也先把这层高级的皮囊搭建起来。
2.  **【逻辑梳理 - AppSession 概念绑定】**：前端开始封装 `AppSession` 的类型 (`AppSession { id, title, teamId, activeRole, messages[] }`)，所有对话输入和推送（listen 事件）都必须携带 / 路由到对应的 `appSessionId` 中。
3.  **【心智模型 - 数据库挂载】**：你（或者我）需要在后端的 `src-tauri/src/lib.rs` 的 `SharedBrain` 和 SQLite 建表中，补齐 `app_sessions` 的持久化逻辑。让“AppSession 打开、切换、历史记录拉取”真正走通。

第一步是立竿见影的。**如果您说 “搞起”，我马上先去切除 `App.tsx` 中腐朽的旧布局，把极简、黑客感的 “Tab 版终端UI”给您勾勒出来。看图施工！**

# TODO

## Refactor: ACP Complete + Role Redesign + UI Overhaul — DONE

### Phase 1: Role Model Redesign — DONE
- [x] Role struct 扩展 model/mode/mcp_servers_json/config_options_json/auto_approve
- [x] DB migration (ALTER TABLE + silent error ignore)
- [x] upsert_role / list_roles_for_team / load_role 全部更新
- [x] RoleInput struct for frontend→backend
- [x] /role edit model|mode|auto-approve|mcp-add|mcp-remove 命令
- [x] /role copy 命令
- [x] 移除 system prompt textarea (改为 role-level)
- [x] 前端 role creation form 加 model/mode/auto-approve 字段
- [x] 前端 role list 显示 model/mode badges + copy/delete 按钮
- [x] 更新 Role TypeScript 类型

### Phase 2: Remove Hardcoded Values — DONE
- [x] adapter versions → @latest
- [x] DEFAULT_MCP_SERVERS / DEFAULT_SKILLS → empty
- [x] PATH construction 加 cfg!(target_os = "macos") guard
- [x] welcome message 改为 generic
- [x] ensure_quick_workflow / resolve_model_runtime / /run fallback → "mock"

### Phase 3: Complete ACP Protocol — DONE
- [x] AcpEvent typed enum (11 variants)
- [x] session_notification 全部 SessionUpdate variants
- [x] permission request: auto_approve + oneshot
- [x] Client trait fs/terminal 全实现
- [x] ClientCapabilities (fs + terminal)
- [x] MCP servers passthrough / acp/stream events / respond_permission

### Phase 4: Session Lifecycle — DONE
- [x] Cancel / SetMode / SetConfigOption WorkerMsg + Tauri commands
- [x] cold_start 后 emit modes + apply role config
- [x] 前端 mode switching UI

### Phase 5: Code Quality — DONE
- [x] 模块拆分: lib.rs → types/db/commands/chat/assistant + acp/ → worker/client/session/adapter
- [x] role_from_row 提取消除重复
- [x] TerminalHandle 移到 client.rs
- [x] eventKind → kind 命名修正
- [x] 零 warning

---

## Next: Assistant 层设计 + Role/Command/Context/Session/Workflow 优化

### 1. Assistant 抽象层
- [ ] Assistant 作为独立实体: 当前 assistant = runtime key (claude-code/gemini-cli/codex-cli)，没有持久化配置
- [ ] Assistant struct: id, name, runtime_kind, default_model, default_mode, default_mcp_servers, status
- [ ] assistant 表 + CRUD
- [ ] assistant 与 role 的关系: role 绑定到 assistant (而非 runtime_kind 字符串)
- [ ] 前端 assistant 面板: 不只是选择，还能配置 (model/mode/mcp)
- [ ] assistant prewarm 改为基于 assistant 配置而非 catalog 硬编码

### 2. Role 改进
- [ ] 侧边栏 role list 隐藏 UnionAIAssistant 默认角色（仅在 AssistantConfigPanel 中管理，不混入用户自建 role 列表）
- [ ] /role bind 同名已存在时报错提示，不要静默覆盖 (当前 ON CONFLICT DO UPDATE)
- [ ] /role bind 参数补全: 缺少 model/mode 时提示可选参数
- [ ] /role edit 支持内联编辑全部字段 (当前只有分字段 edit)
- [ ] Role 绑定 Assistant 而非 runtime_kind 字符串

### 3. 命令体系重设计 + Shell-Prompt 交互
- [ ] activeRole 状态追踪: @RoleName 进入 role context，@assistant 退出
- [ ] Shell-prompt 风格输入框: `Dev >` / `UnionAI >` 显示当前 context
- [ ] 有 activeRole 时，直接输入自动路由到该 role（不需要 @前缀）
- [ ] 侧边栏点击 role 也切换 activeRole
- [ ] 区分 App 命令 vs Agent 命令:
  - App 命令 (`/assistant`, `/role`, `/workflow`, `/session`, `/context`, `/model`, `/mcp`, `/skill`): 管理类，全局可用
  - Agent 命令 (`/plan`, `/act`, `/auto`, `/cancel`): 在当前 role context 内对 agent 发指令
- [ ] Agent 命令通过 ACP session/set_mode 等方法下发，不走 apply_chat_command
- [ ] 前端补全分区: 有 activeRole 时 agent 命令在前 + app 命令在后；无 activeRole 时只显示 app 命令
- [ ] CLI agent 返回的 available_commands 透传到前端补全

### 3.1 /run Workflow 全链路 — 待设计和实现

**当前链路:**
- `/run <prompt>` → `commands/mod.rs:303-344`
  - 查 `latest_workflow_id()` → 没有则 `ensure_quick_workflow()` 自动创建 Planner + Executor role + quick workflow
  - `start_workflow()` 创建 Session 记录 → `tauri::async_runtime::spawn` 跑 `run_workflow()`
- `run_workflow()` (`session.rs:176-320`) 串行遍历 workflow.steps:
  1. emit `workflow/state_changed` (step started) + 写 `StepStarted` event
  2. `acp::execute_runtime()` 发 prompt 给 CLI agent
  3. 流式 delta → emit `session/update` + 写 `DeltaReceived` event
  4. output summary → 写入 shared context `summary.{role_name}`
  5. emit `session/update` (done=true) + 写 `StepCompleted` event
  6. summary append 到 prompt 传给下一步
- 完成: `update_session_status("completed")` + emit `workflow/state_changed`
- 前端: `session/update` → 追加 `[roleName] delta`；`workflow/state_changed` → 显示状态

**核心问题:**
- [ ] Planner/Executor 污染侧边栏 — 自动创建的 role 没有 hidden/system_managed 标记
- [ ] workflow 进度无专属 UI — 事件当普通 message push，没有进度条/步骤卡片
- [ ] Session 概念混乱 — Session = workflow execution record，前端无 session 管理 UI
- [ ] 步骤间传递靠 hack — summary 写 shared context + string concat，无结构化 handoff
- [ ] 没有 cancel workflow — 只能 cancel 单个 ACP session，不能取消整个 workflow
- [ ] `/run` 复用 bug — `latest_workflow_id` 取最新 workflow，应按 name="quick" 查

### 3.2 跨 Role Context Sharing — 待设计

**问题:** 切换 activeRole 后，新 role 的 ACP session 看不到之前 role 的对话内容。每个 role 是独立的 ACP session（连接池按 `runtime_key:role_name` 隔离），前端消息流是全局混在一起的。

**方案（按实现难度排）:**

#### 方案 A: 前端消息分 channel（纯 UI，最小方案）
- [ ] 每个 role 维护独立的消息历史（Map<roleName, ChatMessage[]>）
- [ ] 切换 activeRole 时切换显示的消息流
- [ ] agent 侧 ACP session 本身已保持对话历史，无需后端改动
- [ ] 不解决跨 role context，但解决 UI 混乱

#### 方案 B: 自动注入上一个 role 的 summary（中等方案）
- [ ] 切换 role 时，把前一个 role 的最近对话 summary 写入 shared context
- [ ] 新 role 的 context_pairs 自动包含 `previous_role_summary`
- [ ] 已有基础设施（shared context），改动小

#### 方案 C: MCP-over-ACP Tool Injection（长期方案，参考 docs/rfd_mcp_over_acp.mdx）
- [ ] UnionAI 作为 ACP client 向每个 agent session 注入 MCP tools:
  - `get_role_output(role_name)` — 读取其他 role 的最新输出
  - `get_shared_context(key)` — 读 UnionAI 的 shared context
  - `list_roles()` — 查看当前 team 的 role 列表
- [ ] Agent 可主动查询其他 role 的输出，比 string concat hack 更优雅
- [ ] 前提: 当前 ACP adapters（claude-agent-acp、gemini-cli）不支持 `mcpCapabilities.acp`，需要 bridging shim
- [ ] 参考 RFD: session/new 时声明 `"transport": "acp"` 的 MCP server，agent 通过 `mcp/connect` / `mcp/message` 回调
- [ ] 不支持 ACP transport 的 agent 需要 stdio shim bridging（spawn shim 进程中转）

### 4. Context 优化
- [ ] 当前设计问题: context 是 flat key-value 存储 (shared_context_snapshots 表)，scope 用字符串区分 (assistant:main / role:Name)
- [ ] 问题1: context 没有类型系统，所有值都是 string
- [ ] 问题2: context 生命周期不明确 — session 级 vs workflow 级 vs 永久
- [ ] 设计方向:
  - 分层 context: global → assistant → role → session
  - session context 随 session 结束自动清理
  - workflow context 在 steps 间传递 (当前 summary.{role} 是 hack)
  - context 支持 JSON 值而非纯 string
- [ ] 实现: context 表加 scope_type (global/assistant/role/session/workflow) + scope_id
- [ ] 前端: context panel 按 scope 分组显示

### 5. Session 设计
- [ ] 当前问题: Session = workflow execution record，没有独立的 chat session 概念
- [ ] Chat session: 用户与某个 role/assistant 的对话历史，持久化
- [ ] ACP session: 底层 agent 进程的 session，由 connection pool 管理
- [ ] 需要区分:
  - ChatSession: UI 层对话，存 messages，可恢复
  - AcpSession: 传输层连接，存 session_id，warm/cold
- [ ] ChatSession 表: id, role_name, runtime_kind, messages_json, created_at, updated_at
- [ ] 前端: session list 可切换/恢复历史对话
- [ ] 会话恢复: 应用重启后从 ChatSession 恢复 UI，AcpSession 重新 cold start

### 6. Workflow 设计
- [ ] 当前问题: workflow = 固定的 role 列表 + 串行执行，太简陋
- [ ] 改进方向:
  - step 支持条件分支 (if/else based on previous output)
  - step 支持并行执行 (fan-out + merge)
  - step 支持 human-in-the-loop 审批
  - step output schema 定义 (结构化输出而非纯文本)
- [ ] 短期: workflow steps 加 config (每步的 prompt template, timeout, retry)
- [ ] 中期: DAG 执行引擎替代当前线性循环

### 7. 交互与 Session 体验 — 本轮已修，待验证

#### 已完成
- [x] acp/stream 协议对齐：前端改为解包 `{ role, event: { kind, ... } }`，toolCall/plan/permission/statusUpdate 现在能正常触发
- [x] Session 串台修复：`streamOriginSessionId` 全局变量锁定发起 session，`acp/delta` 和 `acp/stream` 事件均按 `appSessionId` 路由
- [x] ACP 连接池 key 改为 `appSessionId:runtime:role`，每个 AppSession 独立 agent session，取消/模式切换不互相影响
- [x] 流式缓冲优化：后端去掉 30ms/128B buffer，每个 TextDelta 直接 emit；前端改为 RAF 批合并（每帧最多一次 store 写）
- [x] 工具调用持久化：`AppMessage` 新增 `toolCalls` 字段，response 完成时 snapshot 当前 tool calls，渲染在消息内
- [x] Session ID 对齐：`newSession` 和初始加载均 `await create_app_session` 并使用后端返回的 UUID
- [x] Prewarm 两阶段策略：Phase 1 先热最近 AppSession 的 role（带 resume），Phase 2 异步热其余 role
- [x] thoughtDelta 不再混入正文，改为更新 `agentState`（显示在 thinking 指示器）
- [x] slash 补全过滤 `/app_` 判断修复（缺少前缀 `/`）
- [x] completion.rs `/role edit` → `/app_role edit`

#### 待做
- [ ] 错误状态机：catch 后应设 `status: "error"` 而非 finally 无条件 `"done"`，支持重试 UI
- [ ] queuedInputs / inputHistory / canceledRunToken 未按 session 隔离，多 tab 下队列语义混乱
- [ ] session/update 和 workflow/state_changed 事件仍写入当前激活 tab，workflow 多 session 下不可信
- [ ] selectedAssistant `null` 清空语义：后端 `update_app_session` 只在 `Some` 时更新，无法显式清空
- [ ] complete_mentions 的 `std::fs::read_dir` 是同步调用，大目录下卡顿（改 spawn_blocking）
- [ ] skill N+1 查询：chat.rs 逐个 skill 名查询，改为 `WHERE name IN (...)`
- [ ] ConfigDrawer JSON.parse 在渲染循环中执行，角色多时交互发涩（移出渲染循环）
- [ ] DB pool Condvar 无超时等待，极端并发下可能长尾阻塞（加 5s timeout）

### 8. 前端 UI 加载优化
- [ ] 首屏骨架屏：onMount 完成前显示 skeleton placeholder，避免白屏/闪烁
- [ ] ConfigDrawer 代码拆分：已 lazy() 但首次打开仍卡顿，预加载或 prefetch 改善
- [ ] 消息列表虚拟滚动：长会话 500+ 条消息全量渲染导致掉帧（接入 virtual list）
- [ ] ConfigDrawer JSON.parse 移出渲染循环：角色多时 configOptionsJson 每帧反复 parse
- [ ] 字体/CSS 资源预加载：index.html 加 `<link rel="preload">` 减少 FOUC
- [ ] Tauri invoke 批量化：onMount 时 refreshAssistants + refreshRoles + refreshSkills 可合并为单次 invoke 减少 IPC 往返
- [ ] 会话恢复懒加载：list_app_sessions 只加载最近活跃 session 的消息，其余按需加载
- [ ] 图片/图标资源：SVG inline 替代外部加载，减少请求数

### 9. 遗留项
- [ ] MCP servers JSON → Vec<McpServer> 解析并传入 execute_runtime
- [ ] 前端 MCP servers JSON editor in role form
- [ ] virtual list for long sessions
- [ ] macOS 原生窗口 (titleBarStyle overlay, hiddenTitle, trafficLightPosition)
- [ ] backgroundThrottling 调整
- [ ] 高风险命令二次确认

### 10. UI/UX 优化方案
- [ ] 极深视觉基调: 底色采用极致深色 (`#000000` / `#030305`)，加入径向高光模糊打破平面感。
- [ ] 次世代毛玻璃: 半透背景配合 `backdrop-blur-xl` + 极细高光描边 `border-white/[0.05]`。
- [ ] 发光交互态: 交互热点（激活的 Agent、Submit 按钮等）使用大体积弥散阴影 (`shadow-[0_0_20px_rgba(99,102,241,0.2)]`)。
- [ ] 字体与排版重组: 切换系统字体为 `Inter`/`SF Pro Display`，增加空间留白 (`space-y-6`) 和呼吸感。
- [ ] 悬浮指令台: 底栏脱离底部贴边，改为居中悬浮的 Command Bar，聚焦时泛起科技微光。
- [ ] 无界视效: 取消硬分隔线，基于投影和纵深（Elevation）建立组件层级。
- [ ] 微动效体系: 消息体 `fadeIn` 缓冲滑动入场、工具调用点阵高亮、弹窗弹性展示。

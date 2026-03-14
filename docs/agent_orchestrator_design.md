# Multi-Agent Tauri Orchestrator Design (ACP-Based)

本项目旨在利用 Tauri (Rust) 构建一个高度可定制的多智能体编排器，通过 **Agent Client Protocol (ACP)** 标准连接并协调多个独立的 AI CLI Agents（如 Claude Code, Gemini CLI），实现复杂的协同工作流。

---

## 1. 核心架构：Conductor + Shared Brain

为了解决多 Agent 协作中的状态不一致、上下文孤岛及兼容性问题，本项目采用 ACP 提案中的前沿设计模式。

### 1.1 Tauri 作为 Conductor (代理网关)
> 原型参考：`docs/rfds/proxy-chains.mdx`

在本项目中，Tauri App 不仅仅是一个展示界面，它是所有 Agent 消息流的 **Conductor (指挥家)**。

*   **流量拦截与观测**：所有的 JSON-RPC 流量（基于 PTY 或 stdio）都会经过 Tauri Backend。Tauri 实时解析 `session/update` 通知，提取 Agent 产生的临时思维链、Plan 或生成片段。
*   **单向顺序工作流 (One-way Sequential Workflow)**：Tauri 维护一个全局状态机，支持由用户或 Controller Role 发起的线性任务下发。
    *   **示例**：当 Architect Role (Gemini) 完成设计后，Conductor 捕获其信号，触发 **Context Summary Hook**。
    *   **优势**：Agent 之间不直接对话，由 Conductor 作为唯一的指令来源，确保任务流的可预测性和安全性。

### 1.2 基于 MCP-over-ACP 的共享大脑 (Shared Brain)
> 原型参考：`docs/rfds/mcp-over-acp.mdx`

为了实现不同厂商 CLI (Claude vs Gemini) 之间的深度上下文同步，我们不依赖脆弱的文件数据库分享，而是让 Tauri 自身成为一个 **Shared Brain (共享大脑)**。

*   **Tauri 即 MCP Server**：Tauri App 在处理 ACP 连接的同时，反向赋予 Agent 访问其内部 MCP 工具的能力。
*   **黑板模式 (Blackboard Memory)**：Tauri 内部维护一块隔离的共享内存或缓存库，暴露 `get_shared_context` 和 `update_shared_context` 工具。
*   **透明桥接实现 (Shim Bridging)**：
    *   由于大部分现有 CLI仅支持 stdio/HTTP 形式的传统 MCP，Tauri 通过**动态生成虚拟 MCP 配置文件**并劫持 stdio 管道。
    *   CLI 以为在连接本地进程，实际上是在访问 Tauri 后端的逻辑。通过这种“障眼法”，我们实现了跨引擎的变量同步。

---

## 2. 团队与角色配置 (Team & Role Configuration)

为了支持复杂的多样化任务，系统引入 **Team (智能体团队)** 概念。每个 Team 是一个独立的协作单元，拥有专属的角色集合与协作上下文。

### 2.1 团队定义 (Multi-Team Architecture)
用户可以创建多个 Team（例如：`Code_Team`, `Ops_Team`）。
- **Team Scope**：一个 Team 对应一个物理 Workspace 或一组特定的项目目标。
- **Team Registry**：每个 Team 包含一组绑定的角色（Roles），角色之间默认开启共享黑板（Shared Brain）。

### 2.2 典型团队配置示例 (Example Team Configurations)

系统预设或支持自定义多种专业团队，以应对不同的工程场景：

| 团队类型 | 典型角色 (Roles) | 绑定 CLI / 引擎 | 协作重点 |
| :--- | :--- | :--- | :--- |
| **Code Team (代码开发)** | Architect, Developer, Reviewer | Gemini (规划), Claude (编码), Ollama (本地扫描) | 架构对齐、代码实现、自动化 Review |
| **Ops Team (运维/SRE)** | SRE, Log Analyzer, Security | Claude (脚本编写), Gemini (长日志分析), Local Tool | 故障诊断、脚本自动化、安全合规检查 |
| **Docs Team (文档/技术写作)** | Writer, Translator, Proofreader | Gemini (全文结构), Claude (术语校对), DeepL API | 结构化写作、多语言翻译、风格一致性 |

### 2.3 角色绑定与 CLI 映射 (Role-to-CLI Mapping)
在 Team 内部，每个角色根据其职责被分配最适合的运行时：
- **逻辑复杂型 (Complex Logic)**: 优先绑定 `claude-code`，利用其极高的指令遵循度。
- **长文本/全局扫描型 (Long Context)**: 优先绑定 `gemini-cli`，处理数万行代码的全局依赖。
- **隐私/高频重复型 (Privacy/High Frequency)**: 绑定本地 `ollama` 实例，节省 Token 成本并保证数据安全。

### 2.4 动态协作模式 (Collaborative Workflow)
- **@Role 路由拦截**：前端 UI 正则匹配消息头（如 `@Architect`），解析负载后路由至该 Team 下对应的 ACP 管道。
- **跨 CLI 接力**：
    - **物理同步**：同一 Team 下的所有角色挂载同一个 `workspace_path`，通过 `fs/*` 接口感知物理文件变化。
    - **逻辑同步**：通过 **MCP-over-ACP** 接口，实现非文件型状态（如“当前选定的配色方案”、“API 字段约定”）在不同 CLI（Claude vs Gemini）之间的即时同步。

---

## 3. 会话管理与状态识别 (Session & State)

### 3.1 状态识别机
Tauri 通过四种信号识别每个角色在 Team 中的当前状态：
- **Thinking / Writing**：接收到持续的 `session/update` 通知流。
- **Idle / Waiting**：`session/prompt` 的 RPC Response 已落地返回。
- **Paused**：人为挂起或等待用户审批 (Human-in-the-loop)。
- **Errored**：管道断开或收到标准错误回包。

### 3.2 动态模式切换 (session/set_mode)
利用 ACP 的 `session/set_mode` 功能，可以动态在 Team 内部切换角色的行为：
- **Architect** 启动时强制设为 `plan` 模式，确保其只做设计不改代码。
- **Dev** 随后切回默认执行模式，承接设计产出进行代码变更。

---

## 4. 关键组件技术实现设计 (Technical Implementation)

### 4.1 核心指挥官 (The Conductor)
作为 JSON-RPC 中件间转发器，负责流量劫持与动态路由。
- **协议拦截**：利用 `tokio` 异步运行时。拦截 CLI 进程的 `stdout` 流，解析 `session/update` 通知。
- **上下文提取与 Hook 机制 (Context Hooks)**：
    - **原理**：不采取“全量历史透传”模式，而是在任务节点结束时触发 Hook。
    - **逻辑**：Conductor 拦截 Role A 的结果，利用轻量级模型或预设逻辑生成 **Context Summary**。只将下游 Agent (Role B) 执行任务所需的最小核心上下文注入到下一轮消息中。
- **动态消息构造**：根据 Workflow 定义，自动组装下一阶段角色的 `session/prompt`，注入经由 Hook 处理后的精简上下文，极大地节省 Token 消耗。

### 4.2 共享黑板 (MCP-over-ACP Shared Brain)
利用官方 `mcp-over-acp` 提案实现跨代理的“逻辑大脑”同步。
- **逆向工具注入**：Tauri Rust 进程在处理 ACP 连接的同时，反向作为 MCP Server 暴露。
- **内存池管理**：在 Rust 层使用 `dashmap` (高并发哈希表) 维护 Team 级内存。
- **核心工具集**：提供 `set_team_context(key, val)` 和 `get_team_context(key)` 接口，供所有 CLI 调用以交换非文件型状态（如变量约定、全局参数）。

### 4.3 垫片桥接器 (The Shim/Bridge)
解决现有 CLI 仅支持传统 stdio/HTTP MCP 的兼容性问题。
- **虚拟配置注入**：Tauri 在启动 CLI 前动态生成临时的 MCP `config.json`。
- **流劫持**：通过管道重定向，让 CLI 以为在连接本地进程，实则所有 `mcp/call` 请求被重定向至 Tauri 内部的 RPC 处理函数。

### 4.4 传输层设计：双轨制 (PTY vs Raw Pipes)
本项目采用 **双轨传输架构**，兼顾协议纯度与 UI 交互：

1.  **逻辑层 (ACP Protocol Track)**：
    - 使用标准 Stdio Pipes 进行高速、纯净的 JSON-RPC 交互。
2.  **交互层 (PTY Track)**：
    - **UI 增强**：使用 `portable-pty` 包装进程，确保 ANSI 颜色、进度条等在 Dashboard (xterm.js) 中完美渲染。
    - **人工接管 (Human-in-the-loop)**：当 AI 遇到无法通过协议处理的错误（如终端卡住、环境冲突）时，允许用户直接切换到 PTY 模式进行手动指令干预。
    - **容错处理**：捕捉 CLI 在非 RPC 状态下输出的警告或交互式询问，增强系统的健壮性。

### 4.5 存储与记忆架构：热/冷分离 (Persistence & Memory Architecture)

为了解决传统磁盘数据库读写慢的问题，本作采用 **“内存优先、异步回仓”** 的二级存储架构。

#### 1. L1: 热记忆层 (Hot Memory - DashMap)
针对高频的“共享黑板”读写。
- **技术选型**：Rust `dashmap`。
- **作用**：存储当前活跃 Team 的共享变量、任务状态、锁信息。
- **性能**：纳秒级读写，确保 Agent 间的状态同步“零延迟”。

#### 2. L2: 持久化层 (Warm/Cold Storage - Optimized SQLite)
针对全量历史与长效存储。
- **性能优化策略**：
    - **WAL 模式**：启用 Write-Ahead Logging，支持并发读写，大幅提升写入吞吐量。
    - **异步缓冲 (Batching)**：利用 Rust Channel 实现批量提交，减少磁盘 I/O 频次。
    - **Synchronous = NORMAL**：平衡安全性与写入速度。

#### 3. 数据库表结构设计 (Table Schema)
SQLite 数据库不仅用于记忆，还承载 App 全局配置：

*   **App 配置相关表** (Static Config):
    - `app_settings`：存储全局偏好（主题、语言、并发上限、开发者 Key）。
    - `provider_endpoints`：自定义模型端点、API 协议转换配置。
*   **Team & Role 结构表** (Structural):
    - `teams`：团队元数据、物理工作区路径。
    - `roles`：角色与 CLI 引擎映射、专属 System Prompt 模板。
    - `workflow_definitions`：预设的单向流编排逻辑 (JSON/YAML)。
*   **运行时状态表** (Dynamic):
    - `session_history`：全量 ACP 通信日志、Trace ID 链路记录。
    - `shared_memory_snapshot`：Shared Brain 的持久化回仓数据。

---

## 5. 资源与生命周期管理 (Resource & Lifecycle Management)

由于每个 Role 往往绑定不同的 CLI 二进制文件或需要隔离的环境变量，多团队并行可能导致系统资源（CPU/内存）过载。本项目设计了一套智能生命周期调度系统。

### 5.1 进程分配策略 (Process Allocation)
*   **异构引擎独占**：针对不同厂商的 CLI（如 Claude Code vs Gemini CLI），Tauri 为每个 Role 启动独立的 PTY 子进程。
*   **环境隔离**：每个进程拥有专用的临时目录和环境变量配置，通过物理隔离规避配置污染。

### 5.2 智能生命周期转换 (Smart Lifecycle)
为了优化资源占用，Conductor 遵循以下状态转换逻辑：
- **按需唤醒 (Lazy Launch)**：Team 启动时仅初始化基础环境，不立即拉起所有 CLI。只有当 Workflow 路由到特定 Role 或用户手动 `@Role` 时，才会真正执行进程创建。
- **协作后休眠 (Collaborative Hibernate)**：
    - 当一个角色完成其阶段性任务（如 Architect 输出完毕）并进入长时间 Idle 状态。
    - Conductor 会将其物理进程杀掉以释放内存，但会在 SQLite 中完整保留其 **SessionID**、**最近的历史摘要 (Context Summary)** 以及 **共享黑板状态**。
- **自动恢复 (Hydration)**：当下游节点需要该角色协助时，Conductor 重新拉起进程并利用 ACP 的 `session/load` 接口快速恢复会话。

### 5.3 资源配额与熔断 (Quota & Circuit Breaker)
- **并发上限 (LRU Management)**：系统设置全局活跃进程上限（如 5 个活跃 CLI）。当超出上限时，利用 **LRU (最近最少使用)** 算法强制休眠最不活跃的角色进程。
- **消耗熔断**：为每个 Team 设置 Token 消耗阈值。当单次协作流产生的累计消耗超过阈值时，Conductor 自动挂起 Workflow 并请求用户人工确认。

---

## 6. 链路审计与全链路追踪 (Observability & Tracing)
> 原型参考：`docs/rfds/meta-propagation.mdx`

为了解决多 Agent 长时间协作下的“黑盒”问题，本项目引入了全链路追踪机制。

### 6.1 Trace ID 传播 (Meta Propagation)
*   **全局追踪**：Conductor 为每次用户发起的原始请求生成一个唯一的 `traceparent` (遵循 W3C Trace Context 标准)。
*   **上下文透传**：在向 CLI 发送 `session/prompt` 时，将 Trace 信息注入 RPC 的 `_meta` 字段。所有产生的日志、文件变更和工具调用都将关联此 ID。

### 6.2 协作时序图 (Collaboration Timeline)
- **UI 呈现**：提供一个可视化的时序面板，记录：
    - **节点耗时**：每个角色思考与生成的精确时长。
    - **Context 演变**：查看每一环 Hook 处理前后的 Context 变化对比。
    - **Token 审计**：实时累计并展示该条链路产生的总成本。

---

## 7. 人在回路与扩展机制 (HITL & Extensibility)

### 7.1 检查点机制 (Checkpoints & Approval)
在 Workflow 的节点之间支持插入 **人机交互检查点**。
- **动作决策**：当上游输出完成后，Workflow 进入 `Waiting for Approval` 状态。
- **干预选项**：
    - **Approve (通过)**：立即启动下一环。
    - **Refine (打磨)**：用户对当前产出进行追加提问，强制上游角色重试。
    - **Redirect (重定向)**：手动修改任务流向，跳过某些节点。

### 7.2 插件化钩子系统 (Extensible Hooks)
- **脚本引擎**：支持使用 JavaScript/TypeScript (Deno 运行时) 编写自定义 Hook 脚本。
- **应用场景**：用户可以自定义“敏感词检查 Hook”、“自动提交 Git Hook” 或 “Slack 通知 Hook”，挂载到任何角色的输出尾部。

### 7.3 流清洗与健壮性 (Stream Sanitizer)
- **协议补丁**：针对部分 CLI 可能在流中混入非标准文本（如环境警告）的情况，Conductor 内部实现了一套针对 ACP 消息的正则提取与归一化逻辑，确保 JSON-RPC 解析的健壮性。

---

## 8. 工作空间安全与凭据管理 (Security & Storage)

多团队、多角色的环境下，安全与隐私是系统的底线。

### 8.1 隔离沙箱 (Workspace Sandboxing)
*   **路径锁定**：Conductor 会强制锁定每个 Team 的 `workspace_path`。
*   **接口拦截**：拦截所有 `fs/*` 相关 ACP 调用。如果 Agent 试图访问非授权路径（如 `/etc/`），Conductor 将直接拦截并返回协议错误，或在前端弹出安全警报。

### 8.2 凭据保险箱 (Credential Vault)
*   **按需注入**：API Key 及敏感凭据不作为静态配置存储在磁盘，而是存储在加密的本地数据库中。
*   **进程级隔离**：在拉起 CLI 进程时，仅将该角色所需的变量注入其环境变量。A 角色的 Key 对 B 角色完全不可见。

---

## 9. Workflow 编排逻辑深度设计 (Orchestration Deep Dive)

### 9.1 Controller Role 决策机制
*   **结构化输出**：系统支持定义一个 `Controller` 角色。Conductor 专门监听其产生的结构化输出（如 `<workflow_next>role_name</workflow_next>`）。
*   **任务拆解 (Decomposition)**：Controller 负责将复杂目标拆分为子任务，并通过 Conductor 分发给对应的专业角色。

### 9.2 工作流 Schema (Workflow JSON/YAML)
团队协作通过标准化的 JSON/YAML 进行定义，支持：
- **节点定义**：RoleID, RoleType, InitialPrompt。
- **转移条件**：基于上游状态码或输出内容正则匹配。
- **并发控制**：虽然主推单向流，但支持定义多个节点并行执行（如“并行 Review”），最后由 Conductor 汇聚结果。

### 9.3 共享黑板冲突处理 (Concurrency Control)
由于多个角色可能并行更新“共享大脑”：
- **版本控制**：Shared Brain 记录每次写入的时间戳与版本号。
- **写覆盖策略**：默认采用“最后写入者胜 (Last-write-wins)”，但关键变量支持“追加模式 (Append)”，防止信息丢失。

---

## 10. 项目参考与演进

本项目深度参考了 Crewly 的 Multi-Agent Team 逻辑、Swarmify 的并行思维以及 ACP 官方的最新标准。
- **未来扩展**：支持基于 WebAssembly 的代理过滤件 (Proxy Filters)，允许用户在 Team 级别自定义消息处理逻辑。

**总结**：通过将 **Tauri 定位为多团队指挥中心 (Grand Conductor)**，并利用角色与 CLI 引擎的灵活绑定，本项目实现了“一个框架，多种专家团队”的深度协作体验，真正发挥了不同 AI 厂商 CLI 的互补优势。

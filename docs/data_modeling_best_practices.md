# Data Modeling Best Practices

## 1. 先定义“身份”，再定义“状态”
- 身份字段（ID）必须稳定且不可复用，例如：`appSessionId`、`roleId`。
- 状态字段（title、mode、selectedAssistant）允许变更，但不能承担身份职责。
- 任何跨模块关联都应依赖身份字段，不依赖可变展示字段（如 name/title）。

## 2. 关联键必须覆盖完整维度
- 如果关系受多维约束（如 session + runtime + role），键必须是复合键。
- 示例：`(appSessionId, runtimeKey, roleName) -> cliSessionId`。
- 不要只用 `roleName` 这类部分键，否则会发生跨 runtime 串联与污染。

## 3. “实体主键”与“实例主键”分层
- 实体主键：描述业务对象本身（如 app 会话、角色定义）。
- 实例主键：描述运行时实例（如某 runtime 返回的 `cliSessionId`）。
- 建议模式：
  - `appSessionId` 串联 UI 会话数据。
  - `cliSessionId` 串联 runtime 内部上下文。
  - 通过显式映射层连接二者，而不是相互覆盖。

## 4. 空值语义必须可表达
- 更新模型应能区分三种状态：
  - 字段未出现（不更新）
  - 字段显式置空（clear）
  - 字段设置为具体值
- 在 Rust/TS 场景，常用 `Option<Option<T>>` 或等价三态模型表达 clear 语义。

## 5. 禁止“隐式兜底”掩盖关联错误
- 读取失败时优先暴露关联问题，不要自动降级到不完整 key。
- 允许的兼容策略仅用于数据迁移期，并应有明确淘汰窗口。
- 新写入应只写入 canonical key；兼容读取应是短期策略。

## 6. 单一写路径，读写闭环
- 每个可持久化字段应有唯一写入口（避免多处分散更新）。
- 前端内存状态变更后，应在同一流程触发持久化写入。
- 加载时读取同一来源；避免“A 写内存、B 读数据库”导致漂移。

## 7. 异步更新必须绑定上下文 key
- 异步回调更新状态时，必须携带并校验目标 `sessionId`/`entityId`。
- 禁止在异步完成时直接写“当前活动对象”，避免串会话写错。

## 8. 推荐的 CRUD 闭合检查清单
- Create：创建后是否补齐默认关联（如默认 assistant）并持久化？
- Read：是否按 canonical key 读取？是否存在迁移兼容逻辑？
- Update：是否只更新声明字段？是否支持 clear 语义？
- Delete：是否清理关联映射与子表，避免悬挂引用？

## 9. 设计评审时的反模式
- 用展示名当主键。
- 复合关系只存部分维度。
- 依赖 fallback 让“看起来可用”。
- 异步回调写当前全局状态而非目标实体。
- 持久化与内存状态分别由不同入口维护。

## 10. 实践建议
- 为每个关系定义 canonical key 函数，统一构造。
- 为关键映射写最小回归测试（跨 runtime、跨 role、重启恢复）。
- 在日志中打印 key 维度（session/runtime/role），便于定位关联错误。

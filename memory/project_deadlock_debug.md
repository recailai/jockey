---
name: Deadlock debug todo
description: Jockey ACP session still hangs/deadlocks — need to debug and locate root cause
type: project
---

Deadlock still occurs despite the prompt_lock + mutex-release-before-prompt fix (commit 2637b1d).

**Why:** Exact trigger conditions and stack not yet identified. The fix prevents same-role concurrent prompt deadlock but another scenario may still block.

## 现象更新（已确认）

**不是 Rust deadlock，是前端 UI 卡死：**
- 单 session 正常
- 开第二个 session 后，点击切换 session 无反应
- 说明问题在「切换 session」这个动作触发时发生

---

## 排查思路（按优先级）

### 优先级 1 — 前端 invoke() 挂起
切换 session 触发某个 `invoke()` 永不返回，阻塞了 SolidJS 的 async 事件处理链。

**怎么排查：**
1. 打开 Tauri DevTools (Cmd+Option+I) → Console
2. 在 session 卡死后执行：检查有无 unresolved Promise、unhandled rejection
3. 在 `App.tsx` 的 `setActiveSessionId` 调用处临时加 `console.log` 确认点击事件是否触达

### 优先级 2 — SQLite Mutex 被第二个 session 的写操作持有
切换 session 触发 `list_session_events` / `list_sessions` 等读取命令，如果 `Mutex<Connection>` 被另一个并发写占住，invoke 永不返回。

**怎么排查：**
- `AppState.db` 是 `Mutex<Connection>`，所有命令共享同一把锁
- 复现后看 Rust 侧有无 `lock()` 未释放（在 lib.rs 各 `#[command]` 加 eprintln 时间戳）
- 关键命令：`assistant_chat` 持锁期间调用了哪些 DB 操作？

### 优先级 3 — Worker LocalSet 被 cold_start 占满
第二个 session 触发 prewarm，worker 单线程 LocalSet 被 `cold_start`（30s timeout）霸占，其他 WorkerMsg 在 channel 里排队但无法被处理。

**怎么排查：**
- 复现时看 `acp_log("prewarm.start")` 有无输出但没有 `prewarm.ok`
- cold_start 期间 worker 是否还能处理 Cancel/SetMode 等消息（不能 → 是这个问题）

### 优先级 4 — submitting 状态未清除
第二个 session 的 `submitting = true` 没有正确 reset，导致 UI 层的点击 guard `if (activeSession()?.submitting) return` 屏蔽了输入，看起来像卡死。

**怎么排查：**
- DevTools Console: `window.__sessions` 或 SolidJS devtools 查看各 session 的 `submitting` 字段

---

## 下一步行动
1. 先 DevTools 确认是前端还是后端问题
2. 如果是前端：找 `setActiveSessionId` 触发链上的第一个挂起 invoke
3. 如果是后端：在 `assistant_chat` 和 `list_session_events` 加时间戳日志确认锁竞争

**How to apply:** 复现问题时先开 DevTools，不要猜，看日志。

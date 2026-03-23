# ACP SDK 踩坑记录

`agent-client-protocol` v0.10.2 / `agent-client-protocol-schema` v0.11.2

---

## 1. 几乎所有 Response 类型都是 `#[non_exhaustive]`，不能直接构造

**问题**: 按照直觉用 struct literal 构造返回值，编译报 `E0639: cannot create non-exhaustive struct`。

```rust
// 编译失败
Ok(acp::ReadTextFileResponse { content, meta: None })
Ok(acp::TerminalExitStatus { exit_code: 0 })
```

**原因**: schema crate 对所有 response 结构体加了 `#[non_exhaustive]`，防止后续加字段时破坏下游。

**解决**: 一律用 `::new()` 构造器 + builder 方法链。

```rust
Ok(acp::ReadTextFileResponse::new(content))
Ok(acp::TerminalExitStatus::new().exit_code(Some(0)))
```

**受影响的类型**: `FileSystemCapabilities`, `ReadTextFileResponse`, `WriteTextFileResponse`,
`CreateTerminalResponse`, `TerminalExitStatus`, `TerminalOutputResponse`,
`WaitForTerminalExitResponse`, `KillTerminalResponse`, `ReleaseTerminalResponse`。
基本上所有从 Client trait 方法返回的类型都是这样。

---

## 2. `TerminalExitStatus.exit_code` 类型不是 `i64`

**问题**: 想当然地用 `i64` 存 exit code（参考 Unix 惯例），编译报类型不匹配。

**实际类型**: `Option<u32>`。进程可能还没退出（None），且 ACP 协议规定无符号。

```rust
// 错误
acp::TerminalExitStatus { exit_code: status.code().unwrap_or(-1) as i64 }

// 正确
acp::TerminalExitStatus::new().exit_code(status.code().map(|c| c as u32))
```

---

## 3. `acp::Error::new` 第一个参数是 `i32`，不是 `ErrorCode`

**问题**: 传 `ErrorCode::InternalError` 进去，报类型不匹配。

**原因**: `Error::new` 签名是 `(i32, impl Into<String>)`，ErrorCode 需要 `.into()` 转换。

```rust
// 编译失败
acp::Error::new(acp::ErrorCode::InternalError, "msg")

// 正确
acp::Error::new(acp::ErrorCode::InternalError.into(), "msg")
```

---

## 4. `ToolCallUpdate` 的字段藏在 `.fields` 里

**问题**: 直接访问 `tcu.status`、`tcu.title`、`tcu.content`，报找不到字段。

**原因**: `ToolCallUpdate` 用了 `#[serde(flatten)]` 把 `ToolCallUpdateFields` 嵌入进去，
所以 Rust 端字段挂在 `tcu.fields.status`、`tcu.fields.title`、`tcu.fields.content`。

**另外**: `tcu.fields.content` 是 `Option<Vec<...>>`，迭代时要 `.as_ref().map()` 而不是 `.map()`，否则会 move。

---

## 5. `ToolCall.title` 是 `String`，不是 `Option<String>`

**问题**: 以为 title 可选，写了 `.unwrap_or_default()`，编译报 `String` 没有这个方法。

**实际**: 在 `ToolCall` 里 title 是必填的 `String`，直接 `.clone()` 就行。
但 `ToolCallUpdate.fields.title` 是 `Option<String>`，两个地方不一致。

---

## 6. `ToolCall.kind` 和 `.status` 是枚举，不是字符串

**问题**: 想直接当 `String` 用，类型不匹配。

**解决**: 需要先 `serde_json::to_value(&tc.kind)` 序列化再取字符串值。

```rust
serde_json::to_value(&tc.kind)
    .and_then(|v| Ok(v.as_str().unwrap_or("unknown").to_string()))
    .unwrap_or_else(|_| "unknown".to_string())
```

---

## 7. `CurrentModeUpdate` 字段名不是 `mode_id`

**问题**: 文档/直觉以为字段叫 `mode_id`，编译报不存在。

**实际字段名**: `current_mode_id`。

```rust
// 错误
mode.mode_id.to_string()

// 正确
mode.current_mode_id.to_string()
```

---

## 8. `ConfigOptionUpdate` 字段是复数 `config_options`

**问题**: 以为是单个 `config_option`，实际是 `Vec`。

```rust
// 错误
cfg.config_option

// 正确（是个 Vec，需要 iter）
cfg.config_options.iter().map(|o| serde_json::to_value(o).unwrap_or(json!({})))
```

---

## 9. `SessionInfoUpdate.title` 是 `MaybeUndefined<String>`

**问题**: 以为是 `Option<String>`，写了 `.unwrap_or_default()`，类型不对。

**原因**: ACP 协议区分 "没传" 和 "传了 null"，所以用了三态类型 `MaybeUndefined`。

```rust
// 错误
info.title.unwrap_or_default()

// 正确
match info.title {
    acp::MaybeUndefined::Value(v) => Some(v),
    _ => None,
}
```

---

## 10. `AvailableCommandsUpdate` 字段名不是 `commands`

**问题**: 字段名写短了。

**实际字段名**: `available_commands`。

---

## 11. `RequestPermissionRequest` 没有顶层 `title`/`description`

**问题**: 以为 permission 请求有自己的 title 和 description 字段，实际没有。

**原因**: 权限请求的描述信息挂在 `args.tool_call.fields.title` 上，
因为 permission 是针对某个 tool call 的审批。

```rust
// 错误
args.title.clone()

// 正确
args.tool_call.fields.title.clone().unwrap_or_default()
```

---

## 12. serde `#[serde(tag = "...")]` 不能和 variant 字段名冲突

**问题**: 定义枚举时用了 `#[serde(tag = "kind")]`，同时某个 variant 里也有 `kind: String` 字段，
编译报 `variant field name conflicts with internal tag`。

**解决**: 把 tag 名或字段名改掉。我们把 tag 改成了 `eventKind`，字段改成了 `tool_kind`。

```rust
// 冲突
#[serde(tag = "kind")]
enum AcpEvent {
    ToolCall { kind: String, ... }  // "kind" 和 tag 撞了
}

// 修复
#[serde(tag = "eventKind")]
enum AcpEvent {
    ToolCall { tool_kind: String, ... }
}
```

---

## 13. `SetSessionConfigOptionRequest.value` 类型随 feature flag 变

**问题**: 不开 `unstable_boolean_config` feature 时，`value` 是 `SessionConfigValueId`（字符串包装类型）；
开了之后变成 `SessionConfigOptionValue` 枚举。

**当前做法**: 不开 unstable feature，统一用 `SessionConfigValueId::from(value_string)`。

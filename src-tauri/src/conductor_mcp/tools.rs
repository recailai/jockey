use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub struct ToolContext {
    db_path: String,
}

impl ToolContext {
    pub fn new(db_path: &str) -> Result<Self, String> {
        Connection::open(db_path).map_err(|e| format!("cannot open db: {e}"))?;
        Ok(Self { db_path: db_path.to_string() })
    }

    fn conn(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")
            .map_err(|e| e.to_string())?;
        Ok(conn)
    }

    pub fn call_tool(&self, name: &str, args: Value) -> Result<String, String> {
        match name {
            "list_roles" => self.list_roles(),
            "get_role" => self.get_role(args),
            "add_mcp_to_role" => self.add_mcp_to_role(args),
            "remove_mcp_from_role" => self.remove_mcp_from_role(args),
            "list_mcp_servers" => self.list_mcp_servers(args),
            "upsert_role" => self.upsert_role(args),
            "list_sessions" => self.list_sessions(),
            "list_skills" => self.list_skills(),
            "set_shared_context" => self.set_shared_context(args),
            "get_shared_context" => self.get_shared_context(args),
            _ => Err(format!("unknown tool: {name}")),
        }
    }

    fn list_roles(&self) -> Result<String, String> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT role_name, runtime_kind, model, mode, mcp_servers_json, auto_approve FROM roles ORDER BY role_name"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<Value> = stmt.query_map([], |row| {
            Ok(json!({
                "roleName": row.get::<_, String>(0)?,
                "runtimeKind": row.get::<_, String>(1)?,
                "model": row.get::<_, Option<String>>(2)?,
                "mode": row.get::<_, Option<String>>(3)?,
                "mcpServers": serde_json::from_str::<Value>(
                    &row.get::<_, Option<String>>(4)?.unwrap_or_else(|| "[]".into())
                ).unwrap_or(json!([])),
                "autoApprove": row.get::<_, Option<bool>>(5)?.unwrap_or(true),
            }))
        }).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }

    fn get_role(&self, args: Value) -> Result<String, String> {
        let role_name = args.get("roleName").and_then(|v| v.as_str())
            .ok_or("roleName is required")?;
        let conn = self.conn()?;
        let row: Option<Value> = conn.query_row(
            "SELECT role_name, runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve FROM roles WHERE role_name = ?1",
            params![role_name],
            |row| Ok(json!({
                "roleName": row.get::<_, String>(0)?,
                "runtimeKind": row.get::<_, String>(1)?,
                "systemPrompt": row.get::<_, String>(2)?,
                "model": row.get::<_, Option<String>>(3)?,
                "mode": row.get::<_, Option<String>>(4)?,
                "mcpServers": serde_json::from_str::<Value>(
                    &row.get::<_, Option<String>>(5)?.unwrap_or_else(|| "[]".into())
                ).unwrap_or(json!([])),
                "configOptions": serde_json::from_str::<Value>(
                    &row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "{}".into())
                ).unwrap_or(json!({})),
                "autoApprove": row.get::<_, Option<bool>>(7)?.unwrap_or(true),
            }))
        ).optional().map_err(|e| e.to_string())?;
        match row {
            Some(v) => serde_json::to_string_pretty(&v).map_err(|e| e.to_string()),
            None => Err(format!("role not found: {role_name}")),
        }
    }

    fn add_mcp_to_role(&self, args: Value) -> Result<String, String> {
        let role_name = args.get("roleName").and_then(|v| v.as_str())
            .ok_or("roleName is required")?;
        let server = args.get("server")
            .ok_or("server object is required")?;

        let conn = self.conn()?;
        let existing_json: String = conn.query_row(
            "SELECT COALESCE(mcp_servers_json, '[]') FROM roles WHERE role_name = ?1",
            params![role_name],
            |row| row.get(0),
        ).optional().map_err(|e| e.to_string())?
        .ok_or_else(|| format!("role not found: {role_name}"))?;

        let mut servers: Vec<Value> = serde_json::from_str(&existing_json)
            .map_err(|e| e.to_string())?;

        let new_name = server.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if servers.iter().any(|s| s.get("name").and_then(|v| v.as_str()) == Some(new_name)) {
            return Err(format!("MCP server '{new_name}' already exists on role '{role_name}'"));
        }

        servers.push(server.clone());
        let updated = serde_json::to_string(&servers).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE roles SET mcp_servers_json = ?1, updated_at = strftime('%s','now')*1000 WHERE role_name = ?2",
            params![&updated, role_name],
        ).map_err(|e| e.to_string())?;

        Ok(format!("Added MCP server '{new_name}' to role '{role_name}'. Total servers: {}", servers.len()))
    }

    fn remove_mcp_from_role(&self, args: Value) -> Result<String, String> {
        let role_name = args.get("roleName").and_then(|v| v.as_str())
            .ok_or("roleName is required")?;
        let server_name = args.get("serverName").and_then(|v| v.as_str())
            .ok_or("serverName is required")?;

        let conn = self.conn()?;
        let existing_json: String = conn.query_row(
            "SELECT COALESCE(mcp_servers_json, '[]') FROM roles WHERE role_name = ?1",
            params![role_name],
            |row| row.get(0),
        ).optional().map_err(|e| e.to_string())?
        .ok_or_else(|| format!("role not found: {role_name}"))?;

        let mut servers: Vec<Value> = serde_json::from_str(&existing_json)
            .map_err(|e| e.to_string())?;
        let before = servers.len();
        servers.retain(|s| s.get("name").and_then(|v| v.as_str()) != Some(server_name));
        if servers.len() == before {
            return Err(format!("MCP server '{server_name}' not found on role '{role_name}'"));
        }

        let updated = serde_json::to_string(&servers).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE roles SET mcp_servers_json = ?1, updated_at = strftime('%s','now')*1000 WHERE role_name = ?2",
            params![&updated, role_name],
        ).map_err(|e| e.to_string())?;

        Ok(format!("Removed MCP server '{server_name}' from role '{role_name}'. Remaining: {}", servers.len()))
    }

    fn list_mcp_servers(&self, args: Value) -> Result<String, String> {
        let role_name = args.get("roleName").and_then(|v| v.as_str());
        let conn = self.conn()?;

        if let Some(rn) = role_name {
            let json_str: String = conn.query_row(
                "SELECT COALESCE(mcp_servers_json, '[]') FROM roles WHERE role_name = ?1",
                params![rn],
                |row| row.get(0),
            ).optional().map_err(|e| e.to_string())?
            .ok_or_else(|| format!("role not found: {rn}"))?;
            return Ok(json_str);
        }

        let mut stmt = conn.prepare(
            "SELECT role_name, COALESCE(mcp_servers_json, '[]') FROM roles ORDER BY role_name"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<Value> = stmt.query_map([], |row| {
            let rn: String = row.get(0)?;
            let raw: String = row.get(1)?;
            let servers: Value = serde_json::from_str(&raw).unwrap_or(json!([]));
            Ok(json!({ "roleName": rn, "mcpServers": servers }))
        }).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }

    fn upsert_role(&self, args: Value) -> Result<String, String> {
        let role_name = args.get("roleName").and_then(|v| v.as_str())
            .ok_or("roleName is required")?;
        let runtime_kind = args.get("runtimeKind").and_then(|v| v.as_str())
            .unwrap_or("claude");
        let system_prompt = args.get("systemPrompt").and_then(|v| v.as_str())
            .unwrap_or("");
        let model = args.get("model").and_then(|v| v.as_str());
        let mode = args.get("mode").and_then(|v| v.as_str());
        let auto_approve = args.get("autoApprove").and_then(|v| v.as_bool()).unwrap_or(true);

        let conn = self.conn()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let id = uuid::Uuid::new_v4().to_string();

        conn.execute(
            "INSERT INTO roles (id, role_name, runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', '{}', ?7, ?8, ?9)
             ON CONFLICT(role_name) DO UPDATE SET
               runtime_kind = excluded.runtime_kind,
               system_prompt = excluded.system_prompt,
               model = COALESCE(excluded.model, roles.model),
               mode = COALESCE(excluded.mode, roles.mode),
               auto_approve = excluded.auto_approve,
               updated_at = excluded.updated_at",
            params![&id, role_name, runtime_kind, system_prompt, &model, &mode, auto_approve, now, now],
        ).map_err(|e| e.to_string())?;

        Ok(format!("Role '{role_name}' saved (runtime: {runtime_kind})"))
    }

    fn list_sessions(&self) -> Result<String, String> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, active_role, runtime_kind, created_at FROM app_sessions WHERE closed_at IS NULL ORDER BY created_at DESC LIMIT 20"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<Value> = stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "title": row.get::<_, Option<String>>(1)?,
                "activeRole": row.get::<_, Option<String>>(2)?,
                "runtimeKind": row.get::<_, Option<String>>(3)?,
                "createdAt": row.get::<_, i64>(4)?,
            }))
        }).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }

    fn list_skills(&self) -> Result<String, String> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description FROM skills ORDER BY name"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<Value> = stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "description": row.get::<_, String>(2)?,
            }))
        }).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }

    fn set_shared_context(&self, args: Value) -> Result<String, String> {
        let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("global");
        let key = args.get("key").and_then(|v| v.as_str())
            .ok_or("key is required")?;
        let value = args.get("value").and_then(|v| v.as_str())
            .ok_or("value is required")?;
        let conn = self.conn()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO context_entries (scope, key, value, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value, created_at = excluded.created_at",
            params![scope, key, value, now],
        ).map_err(|e| e.to_string())?;
        Ok(format!("Context set: [{scope}] {key}"))
    }

    fn get_shared_context(&self, args: Value) -> Result<String, String> {
        let scope = args.get("scope").and_then(|v| v.as_str());
        let conn = self.conn()?;
        if let Some(s) = scope {
            let mut stmt = conn.prepare(
                "SELECT key, value FROM context_entries WHERE scope = ?1 ORDER BY key"
            ).map_err(|e| e.to_string())?;
            let rows: Vec<Value> = stmt.query_map(params![s], |row| {
                Ok(json!({ "key": row.get::<_, String>(0)?, "value": row.get::<_, String>(1)? }))
            }).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
            return serde_json::to_string_pretty(&rows).map_err(|e| e.to_string());
        }
        let mut stmt = conn.prepare(
            "SELECT scope, key, value FROM context_entries ORDER BY scope, key"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<Value> = stmt.query_map([], |row| {
            Ok(json!({ "scope": row.get::<_, String>(0)?, "key": row.get::<_, String>(1)?, "value": row.get::<_, String>(2)? }))
        }).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_roles",
            "description": "List all configured roles in JockeyUI with their runtime, model, and MCP server summary.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_role",
            "description": "Get full details of a specific role including system prompt and MCP servers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "The role name to look up" }
                },
                "required": ["roleName"]
            }
        }),
        json!({
            "name": "add_mcp_to_role",
            "description": "Add an MCP server to a role. The server will be available to the agent in future sessions. For stdio transport: provide name, command, args. For http/sse: provide name, type, url.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Target role name" },
                    "server": {
                        "type": "object",
                        "description": "MCP server definition. For stdio: {name, command, args[], env[]}. For http: {name, type:'http', url, headers[]}. For sse: {name, type:'sse', url, headers[]}.",
                        "properties": {
                            "name": { "type": "string" },
                            "command": { "type": "string" },
                            "args": { "type": "array", "items": { "type": "string" } },
                            "env": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "value": { "type": "string" } } } },
                            "type": { "type": "string", "enum": ["http", "sse"] },
                            "url": { "type": "string" },
                            "headers": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "value": { "type": "string" } } } }
                        },
                        "required": ["name"]
                    }
                },
                "required": ["roleName", "server"]
            }
        }),
        json!({
            "name": "remove_mcp_from_role",
            "description": "Remove an MCP server from a role by server name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Target role name" },
                    "serverName": { "type": "string", "description": "MCP server name to remove" }
                },
                "required": ["roleName", "serverName"]
            }
        }),
        json!({
            "name": "list_mcp_servers",
            "description": "List MCP servers configured on roles. Optionally filter by role name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Optional: filter to a specific role" }
                }
            }
        }),
        json!({
            "name": "upsert_role",
            "description": "Create or update a role. Sets runtime kind, system prompt, model, mode, and auto-approve settings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Role name (alphanumeric, -, _)" },
                    "runtimeKind": { "type": "string", "description": "Runtime: claude, gemini, codex", "default": "claude" },
                    "systemPrompt": { "type": "string", "description": "System prompt for the role", "default": "" },
                    "model": { "type": "string", "description": "Model override (e.g. claude-sonnet-4-20250514)" },
                    "mode": { "type": "string", "description": "Mode override (e.g. plan, code)" },
                    "autoApprove": { "type": "boolean", "description": "Auto-approve tool calls", "default": true }
                },
                "required": ["roleName"]
            }
        }),
        json!({
            "name": "list_sessions",
            "description": "List active (open) JockeyUI sessions.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list_skills",
            "description": "List all skills registered in JockeyUI.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "set_shared_context",
            "description": "Set a shared context entry that persists across sessions and roles.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "description": "Context scope (default: global)", "default": "global" },
                    "key": { "type": "string", "description": "Context key" },
                    "value": { "type": "string", "description": "Context value" }
                },
                "required": ["key", "value"]
            }
        }),
        json!({
            "name": "get_shared_context",
            "description": "Get shared context entries. Optionally filter by scope.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "description": "Optional: filter by scope" }
                }
            }
        }),
    ]
}

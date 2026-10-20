//! Binds each injected Jockey MCP server entry to the (appSessionId, roleName) it was
//! built for, so tools can default those parameters instead of relying on the model
//! being told them in the prompt. The header is not a credential: the bridge's Bearer
//! token still authenticates, and explicit tool arguments keep overriding the binding.

use crate::acp::protocol as acp;
use serde_json::Value;

pub(crate) const BINDING_HEADER: &str = "X-Jockey-Binding";
const JOCKEY_SERVER_NAME: &str = "jockey";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpBinding {
    pub(crate) app_session_id: String,
    pub(crate) role_name: String,
}

impl McpBinding {
    pub(crate) fn encode(&self) -> String {
        let raw = format!("{}\0{}", self.app_session_id, self.role_name);
        raw.bytes().map(|b| format!("{b:02x}")).collect()
    }

    pub(crate) fn decode(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() || !value.len().is_multiple_of(2) || !value.is_ascii() {
            return None;
        }
        let bytes: Option<Vec<u8>> = (0..value.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
            .collect();
        let text = String::from_utf8(bytes?).ok()?;
        let (session, role) = text.split_once('\0')?;
        if session.is_empty() {
            return None;
        }
        Some(Self {
            app_session_id: session.to_string(),
            role_name: role.to_string(),
        })
    }
}

/// Adds the binding header to the built-in `jockey` server entry, replacing any stale one.
pub(crate) fn bind_jockey_servers(
    servers: &mut [acp::McpServer],
    app_session_id: &str,
    role_name: &str,
) {
    let value = McpBinding {
        app_session_id: app_session_id.to_string(),
        role_name: role_name.to_string(),
    }
    .encode();
    for server in servers.iter_mut() {
        if let acp::McpServer::Http(http) = server {
            if http.name == JOCKEY_SERVER_NAME {
                http.headers
                    .retain(|h| !h.name.eq_ignore_ascii_case(BINDING_HEADER));
                http.headers
                    .push(acp::HttpHeader::new(BINDING_HEADER, value.clone()));
            }
        }
    }
}

/// Tools whose `appSessionId` falls back to the bound session. `roleName` is deliberately
/// not defaulted: it is a target for `invoke_role` and a filter for `get_session_context`.
pub(crate) fn apply_binding(tool: &str, params: &mut Value, binding: Option<&McpBinding>) {
    let Some(binding) = binding else { return };
    if !matches!(
        tool,
        "get_session_context" | "get_session_history" | "invoke_role"
    ) {
        return;
    }
    let Some(map) = params.as_object_mut() else {
        return;
    };
    let missing = map
        .get("appSessionId")
        .and_then(Value::as_str)
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    if missing {
        map.insert(
            "appSessionId".to_string(),
            Value::String(binding.app_session_id.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding() -> McpBinding {
        McpBinding {
            app_session_id: "sess-1".into(),
            role_name: "Dev".into(),
        }
    }

    #[test]
    fn encode_roundtrips() {
        let b = binding();
        assert_eq!(McpBinding::decode(&b.encode()), Some(b));
        assert_eq!(McpBinding::decode("zz"), None);
        assert_eq!(McpBinding::decode(""), None);
    }

    #[test]
    fn defaults_session_but_keeps_explicit_values() {
        let b = binding();
        let mut p = json!({"view": "summary"});
        apply_binding("get_session_context", &mut p, Some(&b));
        assert_eq!(p["appSessionId"], "sess-1");
        assert!(p.get("roleName").is_none());

        let mut p = json!({"appSessionId": "other"});
        apply_binding("invoke_role", &mut p, Some(&b));
        assert_eq!(p["appSessionId"], "other");

        let mut p = json!({});
        apply_binding("list_roles", &mut p, Some(&b));
        assert!(p.get("appSessionId").is_none());
        apply_binding("get_session_context", &mut p, None);
        assert!(p.get("appSessionId").is_none());
    }

    #[test]
    fn binds_only_the_builtin_jockey_server() {
        let mut servers = vec![
            acp::McpServer::Http(acp::McpServerHttp::new("jockey", "http://127.0.0.1:1")),
            acp::McpServer::Http(acp::McpServerHttp::new("other", "http://x")),
        ];
        bind_jockey_servers(&mut servers, "sess-1", "Dev");
        bind_jockey_servers(&mut servers, "sess-1", "Dev");
        let acp::McpServer::Http(j) = &servers[0] else {
            panic!()
        };
        assert_eq!(j.headers.len(), 1);
        assert_eq!(McpBinding::decode(&j.headers[0].value), Some(binding()));
        let acp::McpServer::Http(o) = &servers[1] else {
            panic!()
        };
        assert!(o.headers.is_empty());
    }
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use super::tools;

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

fn ok_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
}

fn err_response(id: Value, code: i64, message: String) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".into(), id, result: None, error: Some(JsonRpcError { code, message }) }
}

pub fn run_stdio_server(db_path: String) -> io::Result<()> {
    let ctx = tools::ToolContext::new(&db_path)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = err_response(Value::Null, -32700, format!("parse error: {e}"));
                write_response(&mut out, &resp)?;
                continue;
            }
        };

        if req.jsonrpc != "2.0" {
            let resp = err_response(req.id.unwrap_or(Value::Null), -32600, "invalid jsonrpc version".into());
            write_response(&mut out, &resp)?;
            continue;
        }

        let id = req.id.unwrap_or(Value::Null);

        let resp = match req.method.as_str() {
            "initialize" => ok_response(id, json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "unionai-conductor",
                    "version": "0.1.0"
                }
            })),
            "notifications/initialized" => continue,
            "tools/list" => ok_response(id, json!({
                "tools": tools::tool_definitions()
            })),
            "tools/call" => {
                let name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));
                match ctx.call_tool(name, arguments) {
                    Ok(result) => ok_response(id, json!({
                        "content": [{ "type": "text", "text": result }]
                    })),
                    Err(e) => ok_response(id, json!({
                        "content": [{ "type": "text", "text": format!("Error: {e}") }],
                        "isError": true
                    })),
                }
            }
            "ping" => ok_response(id, json!({})),
            _ => err_response(id, -32601, format!("method not found: {}", req.method)),
        };

        write_response(&mut out, &resp)?;
    }

    Ok(())
}

fn write_response(out: &mut impl Write, resp: &JsonRpcResponse) -> io::Result<()> {
    let json = serde_json::to_string(resp)?;
    writeln!(out, "{json}")?;
    out.flush()
}

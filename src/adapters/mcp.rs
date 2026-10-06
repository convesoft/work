//! Stdio transport over the same delivered operations as CLI.
use super::cli;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn handle(request: Value, initialized: &mut bool) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !(id.is_null() || id.is_string() || id.is_number())
        || !request.is_object()
        || request["jsonrpc"] != "2.0"
        || !request["method"].is_string()
    {
        return Some(rpc_error(Value::Null, -32600, "Invalid request"));
    }
    let method = request["method"].as_str().unwrap();
    request.get("id")?;
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Some(rpc_error(id, -32602, "Named object parameters required"));
    }
    let result = match method {
        "initialize" => {
            if !params["protocolVersion"].is_string() {
                return Some(rpc_error(id, -32602, "Missing protocolVersion"));
            }
            *initialized = true;
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"work","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        _ if !*initialized => return Some(rpc_error(id, -32000, "Initialize first")),
        "tools/list" => {
            json!({"tools":cli::OPERATIONS.iter().map(|name|json!({"name":name,"description":"Delivered beta.2 hosting/material operation; no execution initialization on reads","inputSchema":cli::schema(name),"outputSchema":cli::output_schema(name)})).collect::<Vec<_>>()})
        }
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return Some(rpc_error(id, -32602, "Missing tool name"));
            };
            if !cli::OPERATIONS.contains(&name) {
                return Some(rpc_error(id, -32602, "Unknown tool"));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (value, is_error) = match cli::call(name, &args) {
                Ok(v) => (v, false),
                Err(e) => (json!({"error":cli::error(e)}), true),
            };
            json!({"structuredContent":value,"content":[{"type":"text","text":value.to_string()}],"isError":is_error})
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
pub fn run() -> i32 {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut initialized = false;
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("work mcp: {e}");
                return 1;
            }
        };
        let result = match serde_json::from_str(&line) {
            Ok(request) => handle(request, &mut initialized),
            Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = result
            && writeln!(stdout, "{response}")
                .and_then(|_| stdout.flush())
                .is_err()
        {
            return 1;
        }
    }
    0
}

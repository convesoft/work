//! MCP stdio transport over the durable operation layer.

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Map, Value, json};
use work::core::graph::ItemGraph;
use work::core::items::{Completion, ItemStore};
use work::core::operations::{DurableOperations, MetadataChange, RelationKind};
use work::core::project::{Project, discover};

use super::cli::{self, CliError};

const VERSION: &str = "2025-06-18";

pub fn run() -> i32 {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut initialized = false;
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("work mcp: {error}");
                return 1;
            }
        };
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(request, &mut initialized),
            Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response
            && writeln!(stdout, "{response}")
                .and_then(|_| stdout.flush())
                .is_err()
        {
            return 1;
        }
    }
    0
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn handle(request: Value, initialized: &mut bool) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
    };
    let id = object.get("id").cloned();
    if id
        .as_ref()
        .is_some_and(|id| !id.is_null() && !id.is_string() && !id.is_number())
    {
        return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    };
    if object.get("jsonrpc") != Some(&json!("2.0")) {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    }
    let id = id?;
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            if params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .is_none()
            {
                return Some(rpc_error(id, -32602, "Missing protocolVersion"));
            }
            *initialized = true;
            json!({"protocolVersion":VERSION,"capabilities":{"tools":{}},
                "serverInfo":{"name":"work","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        _ if !*initialized => return Some(rpc_error(id, -32000, "Initialize first")),
        "tools/list" => json!({"tools":tools()}),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "Missing tool name"));
            };
            if !TOOL_NAMES.contains(&name) {
                return Some(rpc_error(id, -32602, "Unknown tool"));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (value, is_error) = match call(name, &args) {
                Ok(value) => (value, false),
                Err(error) => (json!({"error":error.value()}), true),
            };
            json!({"content":[{"type":"text","text":value.to_string()}],
                "structuredContent":value,"isError":is_error})
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

const TOOL_NAMES: &[&str] = &[
    "discover",
    "item_create",
    "item_list",
    "item_inspect",
    "item_inspect_raw",
    "item_diagnose",
    "item_ready",
    "item_update",
    "item_close",
    "item_reopen",
    "item_repair",
    "relation_add",
    "relation_remove",
    "storage_inspect",
    "storage_rebuild",
    "storage_backup",
    "storage_migrate",
    "storage_restore",
    "storage_recreate",
];

fn tool(name: &str, properties: Value, required: &[&str], description: &str) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object",
        "properties":properties,"required":required,"additionalProperties":false}})
}

fn tools() -> Vec<Value> {
    let s = json!({"type":"string"});
    let b = json!({"type":"boolean"});
    let common = json!({"worktree":s});
    let item = json!({"worktree":s,"id":s});
    let metadata = json!({"title":s,"completion":{"type":"string","enum":["manual","children"]},
        "priority":{"type":"integer","minimum":0,"maximum":4},"parent":s,
        "clear_parent":b,"labels":{"type":"array","items":{"type":"string"}},
        "model":s,"clear_model":b,"thinking":s,"clear_thinking":b});
    let mut create = metadata.as_object().unwrap().clone();
    create.insert("worktree".into(), s.clone());
    create.insert("body".into(), s.clone());
    let mut update = create.clone();
    update.remove("body");
    update.insert("id".into(), s.clone());
    vec![
        tool(
            "discover",
            common.clone(),
            &[],
            "Resolve the selected Git working checkout and its shared Git common directory.",
        ),
        tool(
            "item_create",
            Value::Object(create),
            &["title"],
            "Create one durable item in the selected checkout. The body is opaque; the result includes a generated full ID.",
        ),
        tool(
            "item_list",
            common.clone(),
            &[],
            "List valid durable items with graph state and blockers. Use item_diagnose for malformed files.",
        ),
        tool(
            "item_inspect",
            item.clone(),
            &["id"],
            "Inspect an item by full ID or unique w- prefix, including its opaque body, relations, and graph state.",
        ),
        tool(
            "item_inspect_raw",
            item.clone(),
            &["id"],
            "Inspect original source bytes and diagnostics by full ID, including malformed items.",
        ),
        tool(
            "item_diagnose",
            common.clone(),
            &[],
            "Report source and graph diagnostics for the selected checkout.",
        ),
        tool(
            "item_ready",
            common.clone(),
            &[],
            "List open executable manual items whose lifecycle prerequisites are resolved. Readiness does not claim or execute work.",
        ),
        tool(
            "item_update",
            Value::Object(update),
            &["id"],
            "Update supplied item header fields while preserving the existing opaque body.",
        ),
        tool(
            "item_close",
            json!({"worktree":s,"id":s,"reason":s}),
            &["id"],
            "Record a manual item as done, optionally with an opaque reason. Other manual items are not closed automatically.",
        ),
        tool(
            "item_reopen",
            item,
            &["id"],
            "Record a manual item as open and remove its close reason. Recompute graph state without reopening other manual items.",
        ),
        tool(
            "item_repair",
            json!({"worktree":s,"id":s,"raw_hex":s}),
            &["id", "raw_hex"],
            "Replace a diagnosed invalid item source by full ID using complete source bytes in raw_hex. Inspect diagnostics first; healthy items cannot be repaired.",
        ),
        tool(
            "relation_add",
            json!({"worktree":s,"source":s,"kind":{"type":"string","enum":["parent","depends_on","related","discovered_from"]},"target":s}),
            &["source", "kind", "target"],
            "Add one relation from source to target: parent is child to parent; depends_on is dependent to prerequisite; related and discovered_from are informational.",
        ),
        tool(
            "relation_remove",
            json!({"worktree":s,"source":s,"kind":{"type":"string","enum":["parent","depends_on","related","discovered_from"]},"target":s}),
            &["source", "kind", "target"],
            "Remove an existing relation. Parent and depends_on use the authored child or dependent as source; related may be removed from either endpoint.",
        ),
        tool(
            "storage_inspect",
            common.clone(),
            &[],
            "Inspect shared coordination storage without creating or changing it.",
        ),
        tool(
            "storage_rebuild",
            common.clone(),
            &[],
            "Rebuild derived indexes for the selected checkout without changing claims or runtime records.",
        ),
        tool(
            "storage_backup",
            common.clone(),
            &[],
            "Retain a consistent backup of shared coordination storage.",
        ),
        tool(
            "storage_migrate",
            common.clone(),
            &[],
            "Back up and transactionally migrate a recognized older shared storage schema.",
        ),
        tool(
            "storage_restore",
            json!({"worktree":s,"backup_path":s}),
            &["backup_path"],
            "Explicitly restore a verified storage backup with active execution stopped.",
        ),
        tool(
            "storage_recreate",
            common,
            &[],
            "Explicitly recreate a diagnosed faulty store with active execution stopped; lost claims cannot be reconstructed.",
        ),
    ]
}

fn invalid(message: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", message)
}

fn validate<'a>(name: &str, args: &'a Value) -> Result<&'a Map<String, Value>, CliError> {
    let object = args
        .as_object()
        .ok_or_else(|| invalid("tool arguments must be an object"))?;
    let schema = tools()
        .into_iter()
        .find(|tool| tool["name"] == name)
        .unwrap();
    let properties = schema["inputSchema"]["properties"].as_object().unwrap();
    for key in object.keys() {
        if !properties.contains_key(key) {
            return Err(invalid(format!("unknown field {key}")));
        }
    }
    for field in schema["inputSchema"]["required"].as_array().unwrap() {
        let field = field.as_str().unwrap();
        if !object.contains_key(field) {
            return Err(invalid(format!("missing field {field}")));
        }
    }
    for (key, value) in object {
        let spec = &properties[key];
        let valid = match spec["type"].as_str().unwrap() {
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "integer" => value.as_u64().is_some_and(|n| n <= 4),
            "array" => value
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string)),
            _ => false,
        };
        if !valid
            || spec
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|e| !e.contains(value))
        {
            return Err(invalid(format!("invalid field {key}")));
        }
    }
    Ok(object)
}

fn string<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required<'a>(args: &'a Map<String, Value>, key: &str) -> &'a str {
    string(args, key).expect("validated required string")
}

fn selected(args: &Map<String, Value>) -> Result<Project, CliError> {
    Ok(discover(string(args, "worktree").map(Path::new))?)
}

fn metadata(project: &Project, args: &Map<String, Value>) -> Result<MetadataChange, CliError> {
    let mut change = MetadataChange {
        title: string(args, "title").map(str::to_owned),
        completion: string(args, "completion").map(|value| {
            if value == "manual" {
                Completion::Manual
            } else {
                Completion::Children
            }
        }),
        priority: args
            .get("priority")
            .and_then(Value::as_u64)
            .map(|n| n as u8),
        ..Default::default()
    };
    if args.get("clear_parent") == Some(&json!(true)) && args.contains_key("parent") {
        return Err(invalid("parent and clear_parent conflict"));
    }
    if args.get("clear_parent") == Some(&json!(true)) {
        change.parent = Some(None);
    }
    if let Some(parent) = string(args, "parent") {
        change.parent = Some(Some(cli::resolve(project, parent)?));
    }
    change.labels = args.get("labels").map(|value| {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    });
    for (key, clear, target) in [
        ("model", "clear_model", &mut change.model),
        ("thinking", "clear_thinking", &mut change.thinking),
    ] {
        if args.get(clear) == Some(&json!(true)) && args.contains_key(key) {
            return Err(invalid(format!("{key} and {clear} conflict")));
        }
        if args.get(clear) == Some(&json!(true)) {
            *target = Some(None);
        }
        if let Some(value) = string(args, key) {
            *target = Some(Some(value.to_owned()));
        }
    }
    Ok(change)
}

fn relation_kind(value: &str) -> RelationKind {
    match value {
        "parent" => RelationKind::Parent,
        "depends_on" => RelationKind::DependsOn,
        "related" => RelationKind::Related,
        "discovered_from" => RelationKind::DiscoveredFrom,
        _ => unreachable!("validated enum"),
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, CliError> {
    if !value.len().is_multiple_of(2) {
        return Err(invalid("raw_hex must have even length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            match (digit(pair[0]), digit(pair[1])) {
                (Some(high), Some(low)) => Ok((high << 4) | low),
                _ => Err(invalid("raw_hex must contain hexadecimal bytes")),
            }
        })
        .collect()
}

fn call(name: &str, input: &Value) -> Result<Value, CliError> {
    let args = validate(name, input)?;
    let project = selected(args)?;
    let ops = DurableOperations::new(&project.worktree_root);
    match name {
        "discover" => Ok(
            json!({"worktree_root":cli::encode_path(&project.worktree_root),"git_common_dir":cli::encode_path(&project.git_common_dir)}),
        ),
        "item_list" => Ok(json!({"items":cli::items_value(&project,&ops.list()?)?})),
        "item_ready" => cli::ready_value(&project, &ops),
        "storage_inspect" => cli::storage_command(&project, "inspect", &[]),
        "storage_rebuild" => cli::storage_command(&project, "rebuild", &[]),
        "storage_backup" => cli::storage_command(&project, "backup", &[]),
        "storage_migrate" => cli::storage_command(&project, "migrate", &[]),
        "storage_recreate" => cli::storage_command(&project, "recreate", &[]),
        "storage_restore" => cli::storage_command(
            &project,
            "restore",
            &[
                "--backup".to_owned(),
                required(args, "backup_path").to_owned(),
            ],
        ),
        "item_diagnose" => {
            let store =
                ItemStore::load(&project).map_err(|e| CliError::new("io", e.to_string()))?;
            let graph = ItemGraph::from_store(&store);
            Ok(
                json!({"diagnostics":graph.diagnostics().iter().map(cli::diagnostic).collect::<Vec<_>>()}),
            )
        }
        "item_inspect" => {
            let input = required(args, "id");
            let candidate = input.strip_prefix("w-").unwrap_or(input);
            let id = match cli::resolve(&project, input) {
                Ok(id) => id,
                Err(error) if error.value()["code"] == "not_found" && candidate.len() == 32 => {
                    if let Ok(raw) = ops.inspect_raw(candidate) {
                        return Err(cli::invalid_source(raw.file.diagnostics));
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            Ok(json!({"item":cli::one_item_value(&project,&ops.inspect(&id)?)?}))
        }
        "item_inspect_raw" => {
            let id = required(args, "id");
            Ok(json!({"source":cli::source_value(id,&ops.inspect_raw(id)?)}))
        }
        "item_create" => {
            let change = metadata(&project, args)?;
            cli::validate_create_fields(required(args, "title"), &change)?;
            let item = ops.create(
                required(args, "title").to_owned(),
                string(args, "body").unwrap_or("").as_bytes().to_vec(),
                change,
            )?;
            Ok(json!({"item":cli::mutation_item_value(&project,&item)?}))
        }
        "item_update" => {
            let id = cli::resolve(&project, required(args, "id"))?;
            let change = metadata(&project, args)?;
            if change.title.is_none()
                && change.completion.is_none()
                && change.priority.is_none()
                && change.parent.is_none()
                && change.labels.is_none()
                && change.model.is_none()
                && change.thinking.is_none()
            {
                return Err(invalid("item update requires at least one metadata field"));
            }
            Ok(json!({"item":cli::mutation_item_value(&project,&ops.update(&id,change)?)?}))
        }
        "item_close" => {
            let id = cli::resolve(&project, required(args, "id"))?;
            Ok(
                json!({"item":cli::mutation_item_value(&project,&ops.close(&id,string(args,"reason").map(str::to_owned))?)?}),
            )
        }
        "item_reopen" => {
            let id = cli::resolve(&project, required(args, "id"))?;
            Ok(json!({"item":cli::mutation_item_value(&project,&ops.reopen(&id)?)?}))
        }
        "item_repair" => {
            let id = required(args, "id");
            let source = decode_hex(required(args, "raw_hex"))?;
            Ok(json!({"source":cli::source_value(id,&ops.repair(id,source)?)}))
        }
        "relation_add" | "relation_remove" => {
            let source = cli::resolve(&project, required(args, "source"))?;
            let target = cli::resolve(&project, required(args, "target"))?;
            let kind = relation_kind(required(args, "kind"));
            let item = if name == "relation_add" {
                ops.relation_add(&source, kind, &target)?
            } else {
                ops.relation_remove(&source, kind, &target)?
            };
            Ok(json!({"item":cli::mutation_item_value(&project,&item)?}))
        }
        _ => unreachable!(),
    }
}

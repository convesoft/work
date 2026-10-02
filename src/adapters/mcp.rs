use work::core::execution::ExecutionOperations;
// MCP stdio transport over shared operations.

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Map, Value, json};
use work::core::graph::ItemGraph;
use work::core::items::Completion;
use work::core::operations::{MetadataChange, RelationKind};
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
            if !TOOL_NAMES.contains(&name)
                && !super::claims::tools().iter().any(|t| t["name"] == name)
                && !super::runs::tools().iter().any(|t| t["name"] == name)
                && !super::handoffs::tools().iter().any(|t| t["name"] == name)
            {
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
    "template_list",
    "template_validate",
    "template_preview",
    "storage_inspect",
    "storage_init",
    "storage_recreate",
    "storage_recover",
];

fn tool(name: &str, properties: Value, required: &[&str], description: &str) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object",
        "properties":properties,"required":required,"additionalProperties":false}})
}

fn tools() -> Vec<Value> {
    let mut result = base_tools();
    let session = json!({"type":"object","properties":{"namespace":{"type":"string"},"id":{"type":"string"}},"required":["namespace","id"],"additionalProperties":false});
    for tool in &mut result {
        let name = tool["name"].as_str().unwrap().to_owned();
        if matches!(
            name.as_str(),
            "item_create"
                | "item_update"
                | "item_close"
                | "item_reopen"
                | "item_repair"
                | "relation_add"
                | "relation_remove"
        ) {
            tool["inputSchema"]["properties"]["authorization"] = json!({"type":"array","items":{"type":"object","properties":{"claim_id":{"type":"string"},"session":session},"required":["claim_id","session"],"additionalProperties":false}});
        }
        if name == "item_close" {
            tool["inputSchema"]["properties"]["handoffs"] =
                json!({"type":"array","items":super::handoffs::input_schema()});
        }
        if matches!(
            name.as_str(),
            "item_list" | "item_inspect" | "item_ready" | "item_diagnose"
        ) {
            tool["inputSchema"]["properties"]["view"] =
                json!({"type":"string","enum":["resolved","checkout"]});
        }
    }
    result.retain(|t| t["name"] != "template_preview");
    result.extend(super::runs::tools());
    result.extend(super::claims::tools());
    result.extend(super::handoffs::tools());
    result
}
fn base_tools() -> Vec<Value> {
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
            "storage_inspect",
            common.clone(),
            &[],
            "Inspect shared file storage health without writes.",
        ),
        tool(
            "storage_init",
            common.clone(),
            &[],
            "Explicitly initialize fresh shared storage; a healthy store is unchanged.",
        ),
        tool(
            "storage_recreate",
            json!({"worktree":s,"expected_store_id":s,"expected_generation":s,
            "executors_stopped":{"type":"boolean","enum":[true]},"acknowledge_loss":{"type":"boolean","enum":[true]},"all_clients_stopped":b}),
            &["executors_stopped", "acknowledge_loss"],
            "Retain prior state and explicitly reset live coordination to a new generation. Stop executors first; a missing root/lock also requires all clients stopped.",
        ),
        tool(
            "storage_recover",
            json!({"worktree":s,"operation_id":s,"executors_stopped":b,"acknowledge_loss":b,"all_clients_stopped":b}),
            &["operation_id"],
            "Resume a supported interrupted initialization or recreation. Recreation requires renewed stopped-executor and loss affirmations.",
        ),
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
            "template_list",
            common.clone(),
            &[],
            "Discover valid and invalid version-1/version-2 YAML templates in the selected checkout.",
        ),
        tool(
            "template_validate",
            json!({"worktree":s,"name":s}),
            &["name"],
            "Validate one version-1/version-2 YAML template definition.",
        ),
        tool(
            "template_preview",
            json!({"worktree":s,"name":s,"root":s,
            "parameters":{"type":"object","additionalProperties":{"type":"string"}},
            "existing":{"type":"object","additionalProperties":{"type":"string"}}}),
            &["name"],
            "Render a complete symbolic item graph without publishing a run, items, permanent IDs, or claims.",
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
            "array" if key == "authorization" => super::claims::authorization(Some(value)).is_ok(),
            "array" if key == "handoffs" => super::handoffs::close_inputs(Some(value)).is_ok(),
            "array" => value
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string)),
            "object" if key == "session" => {
                work::core::coordination::SessionIdentity::from_json(value).is_ok()
            }
            "object" => value
                .as_object()
                .is_some_and(|map| map.values().all(Value::is_string)),
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
    let result = call_inner(name, input);
    if matches!(
        name,
        "storage_init" | "storage_recreate" | "storage_recover"
    ) {
        result.map_err(CliError::storage_failure)
    } else {
        result
    }
}

fn call_inner(name: &str, input: &Value) -> Result<Value, CliError> {
    let args = validate(name, input)?;
    if let Some(verb) = name.strip_prefix("handoff_") {
        return super::handoffs::execute(&selected(args)?, verb, args);
    }
    if let Some(verb) = name.strip_prefix("storage_") {
        let request = super::storage::from_fields(verb, args)?;
        return super::storage::execute(&selected(args)?, request);
    }
    if name.starts_with("run_") || matches!(name, "template_preview" | "template_expand") {
        super::runs::validate(name, args)?;
        return super::runs::execute(&selected(args)?, name, args);
    }
    let project = selected(args)?;
    if let Some(verb) = name.strip_prefix("claim_") {
        return super::claims::execute(&project, verb, args);
    }
    let mut ops = ExecutionOperations::new(project.clone());
    ops.authorization = super::claims::authorization(args.get("authorization"))?;
    ops.checkout_view = string(args, "view") == Some("checkout");
    let result = match name {
        "discover" => Ok(
            json!({"worktree_root":cli::encode_path(&project.worktree_root),"git_common_dir":cli::encode_path(&project.git_common_dir)}),
        ),
        "item_list" => Ok(json!({"items":cli::items_view(&ops,&ops.list()?)?})),
        "item_ready" => Ok(json!({"items":cli::items_view(&ops,&ops.ready()?)?})),
        "template_list" => cli::template_command(&project, "list", &[]),
        "template_validate" => {
            cli::template_command(&project, "validate", &[required(args, "name").to_owned()])
        }
        "template_preview" => {
            let mut arguments = vec![required(args, "name").to_owned()];
            if let Some(root) = string(args, "root") {
                arguments.extend(["--root".to_owned(), root.to_owned()]);
            }
            for (field, flag) in [("parameters", "--param"), ("existing", "--existing")] {
                if let Some(bindings) = args.get(field).and_then(Value::as_object) {
                    for (name, value) in bindings {
                        if !name
                            .bytes()
                            .next()
                            .is_some_and(|byte| byte.is_ascii_lowercase())
                            || !name.bytes().all(|byte| {
                                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                            })
                        {
                            return Err(invalid(format!("invalid {field} binding name {name:?}")));
                        }
                        arguments.push(flag.to_owned());
                        arguments.push(format!(
                            "{name}={}",
                            value.as_str().expect("validated string")
                        ));
                    }
                }
            }
            cli::template_command(&project, "preview", &arguments)
        }
        "item_diagnose" => {
            let store = ops.view()?;
            let graph = ItemGraph::from_store(&store);
            Ok(
                json!({"diagnostics":graph.diagnostics().iter().map(cli::diagnostic).collect::<Vec<_>>()}),
            )
        }
        "item_inspect" => {
            let input = required(args, "id");
            let candidate = input.strip_prefix("w-").unwrap_or(input);
            let id = match cli::resolve_view(&ops, input) {
                Ok(id) => id,
                Err(error) if error.value()["code"] == "not_found" && candidate.len() == 32 => {
                    if let Ok(raw) = ops.inspect_raw(candidate) {
                        return Err(cli::invalid_source(raw.file.diagnostics));
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            Ok(json!({"item":cli::one_item_view(&ops,&ops.inspect(&id)?)?}))
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
                json!({"item":cli::snapshot_item_value(&ops.close_with_handoffs(&id,string(args,"reason").map(str::to_owned),&super::handoffs::close_inputs(args.get("handoffs"))?)?)?}),
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
    }?;
    if matches!(
        name,
        "item_list" | "item_ready" | "item_diagnose" | "item_inspect" | "item_inspect_raw"
    ) {
        Ok(super::storage::attach_read(
            &project,
            result,
            ops.take_read_storage(),
        ))
    } else {
        Ok(result)
    }
}

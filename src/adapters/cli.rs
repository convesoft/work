//! Delivered beta.2 subset. No beta.1 operational compatibility dispatch.
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;
use work::core::coordination::{ExecutionError as Error, ExecutionResult as Result, decode_path};
use work::core::project::discover;
use work::core::{
    scope_items, scope_wire as wire,
    scopes::{self, ScopeOperations},
};

pub(super) const OPERATIONS: &[&str] = &[
    "discover",
    "scope_list",
    "scope_inspect",
    "scope_host",
    "item_create",
    "item_list",
    "item_inspect",
    "item_inspect_raw",
    "item_diagnose",
    "item_ready",
    "item_update",
    "relation_add",
    "relation_remove",
];
pub(super) fn expanded(schema: &Value) -> Value {
    if let Some(reference) = schema["$ref"].as_str() {
        return expanded(wire::catalog().pointer(&reference[1..]).unwrap());
    }
    match schema {
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), expanded(v))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(expanded).collect()),
        _ => schema.clone(),
    }
}
pub(super) fn schema(name: &str) -> Value {
    expanded(
        wire::catalog()
            .pointer(
                &wire::catalog()["operations"][name]["input"]["$ref"]
                    .as_str()
                    .unwrap()[1..],
            )
            .unwrap(),
    )
}
pub(super) fn output_schema(name: &str) -> Value {
    expanded(&wire::catalog()["operations"][name]["output"])
}
pub(super) fn error(e: Error) -> Value {
    let code = if wire::catalog()["$defs"]["Error"]["properties"]["code"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == e.code)
    {
        e.code
    } else {
        "io"
    };
    let mut out = json!({"code":code,"message":e.message});
    if let Some(p) = e.path {
        out["path"] = json!(scopes::path(&p));
    }
    if let Some(fields) = e.details.as_object() {
        for (k, v) in fields {
            if !v.is_null() {
                out[k] = v.clone();
            }
        }
    }
    if let Some(p) = out.get_mut("partial").and_then(Value::as_object_mut) {
        for (k, v) in scopes::partial().as_object().unwrap() {
            p.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    if let Some(diagnostics) = out.get_mut("diagnostics").and_then(Value::as_array_mut) {
        for d in diagnostics {
            if let Some(fields) = d.as_object_mut() {
                for key in ["path", "line", "item_id", "run_id", "scope_id"] {
                    fields.entry(key.to_owned()).or_insert(Value::Null);
                }
                fields
                    .entry("code".to_owned())
                    .or_insert_with(|| json!(code));
            }
        }
    }
    normalize_paths(&mut out, None);
    out
}
fn normalize_paths(value: &mut Value, key: Option<&str>) {
    match value {
        Value::String(s)
            if key.is_some_and(|k| {
                matches!(k, "path" | "checkout" | "destination" | "source_worktree")
                    || k.ends_with("_path")
                    || k.ends_with("_paths")
            }) =>
        {
            if let Ok(p) = decode_path(s) {
                *s = scopes::path(&p);
            }
        }
        Value::Array(a) => {
            for v in a {
                normalize_paths(v, key);
            }
        }
        Value::Object(m) => {
            for (k, v) in m {
                normalize_paths(v, Some(k));
            }
        }
        _ => {}
    }
}
pub(super) fn call(name: &str, input: &Value) -> Result<Value> {
    let operation = &wire::catalog()["operations"][name];
    if operation.is_null() || !OPERATIONS.contains(&name) {
        return Err(Error::new("invalid_argument", "operation not delivered"));
    }
    let definition = operation["input"]["$ref"]
        .as_str()
        .unwrap()
        .strip_prefix("#/$defs/")
        .unwrap();
    wire::validate(definition, input)?;
    let mutation = matches!(
        name,
        "scope_host" | "item_create" | "item_update" | "relation_add" | "relation_remove"
    );
    if mutation && input["view"] == "physical" {
        return Err(Error::new(
            "invalid_argument",
            "physical-copy mode is read-only",
        ));
    }
    let selected = input["worktree"].as_str().map(decode_path).transpose()?;
    let discovery_path = if name == "discover" && selected.is_none() {
        input["path"].as_str().map(decode_path).transpose()?
    } else {
        selected.clone()
    };
    let p = discover(discovery_path.as_deref())
        .map_err(|e| Error::new("unsupported_project", e.to_string()))?;
    let out = match name {
        "discover" => {
            if let Some(s) = input["path"].as_str() {
                let other = discover(Some(&decode_path(s)?))
                    .map_err(|e| Error::new("unsupported_project", e.to_string()))?;
                if selected.is_some() && other != p {
                    return Err(Error::new(
                        "invalid_argument",
                        "path/worktree selectors disagree",
                    ));
                }
                return discovered(&other);
            }
            discovered(&p)?
        }
        "scope_list" => ScopeOperations::new(p.clone(), input["view"] == "physical").read(None)?,
        "scope_inspect" => ScopeOperations::new(p.clone(), input["view"] == "physical")
            .read(input["scope"].as_str())?,
        "scope_host" => ScopeOperations::new(p.clone(), false).host("host", input)?,
        _ => scope_items::call(&p, name, input)?,
    };
    let result = operation["output"]["$ref"]
        .as_str()
        .unwrap()
        .strip_prefix("#/$defs/")
        .unwrap();
    wire::validate(result, &out).map_err(|e| {
        Error::new(
            "io",
            format!("cannot deliver a wire-correct result: {}", e.message),
        )
    })?;
    Ok(out)
}
fn discovered(p: &work::core::project::Project) -> Result<Value> {
    Ok(
        json!({"worktree_root":scopes::path(&p.worktree_root),"git_common_dir":scopes::path(&p.git_common_dir),"git_dir":scopes::path(&p.git_dir().map_err(|e|Error::new("io",e.to_string()))?),"primary_checkout":scopes::path(&p.checkouts().map_err(|e|Error::new("io",e.to_string()))?[0].root)}),
    )
}
fn absolute(s: &std::ffi::OsStr) -> Result<PathBuf> {
    let p = PathBuf::from(s);
    if p.is_absolute() {
        Ok(p)
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| Error::new("io", e.to_string()))?
            .join(p))
    }
}
fn text(s: &std::ffi::OsStr) -> Result<String> {
    s.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::new("invalid_argument", "text must be UTF-8"))
}
fn help() -> String {
    format!(
        "Usage: work [--json] [--worktree PATH] COMMAND\nDelivered: discover; scope list|inspect|host; item create|list|inspect|diagnose|ready|update; relation add|remove; mcp.\nReads default to authoritative primary/scope routing. --view physical is read-only.\nHosting requires an externally prepared identical destination and --expected-inventory HASH from scope inspect.\nOnly the listed operation subset is implemented here. The complete beta.2 catalog, lifecycle and package gates are not claimed.\nitem COMMAND: material operations; create assigns a generated full ID. Ready reserves nothing and excludes claimed items.\nrelation add: parent is child to parent; depends_on is dependent to prerequisite; related and discovered_from are informational.\nmcp: MCP tools over stdio.\nF (selection): --scope S, --root ITEM, --run ID, repeated --label TEXT, --priority-max 0..4, --persistence material|wisp.\nM (metadata): --completion manual|children, --priority 0..4, --parent ITEM, repeated --label TEXT, --model TEXT, --thinking TEXT.\nC (clear): --clear-parent, --clear-labels, --clear-model, --clear-thinking.\nO (owner): --session SELECTOR [--claim ID], or --session-namespace TEXT --session-id TEXT --claim ID.\nZ (authorization): repeated --authorize JSON; cannot be combined with O.\n{}",
        OPERATIONS
            .iter()
            .filter(|n| **n != "discover")
            .map(|n| wire::catalog()["operations"][*n]["cli"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    )
}
fn request(args: &[OsString]) -> Result<(String, Value)> {
    let mut a = json!({});
    let mut at = 0;
    let mut saw_json = false;
    while let Some(flag) = args.get(at) {
        if flag == "--json" {
            if saw_json {
                return Err(Error::new("invalid_argument", "duplicate --json"));
            }
            saw_json = true;
            at += 1;
        } else if flag == "--worktree" {
            if a.get("worktree").is_some() {
                return Err(Error::new("invalid_argument", "duplicate worktree"));
            }
            let p = args
                .get(at + 1)
                .ok_or_else(|| Error::new("usage", "missing worktree path"))?;
            a["worktree"] = json!(scopes::path(&absolute(p)?));
            at += 2;
        } else {
            break;
        }
    }
    let command = args
        .get(at)
        .ok_or_else(|| Error::new("usage", "missing command"))?;
    if command == "--help" || command == "help" || command == "--version" {
        if at + 1 != args.len() || a.get("worktree").is_some() {
            return Err(Error::new(
                "usage",
                "meta command takes no selectors/arguments",
            ));
        }
        return Ok((
            "meta".into(),
            if command == "--version" {
                json!({"version":env!("CARGO_PKG_VERSION")})
            } else {
                json!({"help":help()})
            },
        ));
    }
    if command == "discover" {
        if let Some(p) = args.get(at + 1) {
            if p == "--help" && at + 2 == args.len() {
                return Ok((
                    "meta".into(),
                    json!({"help":"work discover [PATH]: Git working checkout, metadata directory, common directory and primary checkout; read-only"}),
                ));
            }
            a["path"] = json!(scopes::path(&absolute(p)?));
        }
        if at + 2 < args.len() {
            return Err(Error::new("usage", "discover takes at most one path"));
        }
        return Ok(("discover".into(), a));
    }
    let noun = text(command)?;
    let verb = text(
        args.get(at + 1)
            .ok_or_else(|| Error::new("usage", "missing subcommand"))?,
    )?;
    at += 2;
    if verb == "--help" || args.get(at).is_some_and(|s| s == "--help") {
        return Ok(("meta".into(), json!({"help":help()})));
    }
    let mut name = format!("{noun}_{}", verb.replace('-', "_"));
    if !OPERATIONS.contains(&name.as_str()) {
        return Err(Error::new("usage", "unknown or undelivered command"));
    }
    let positional: &[&str] = match name.as_str() {
        "scope_inspect" | "scope_host" => &["scope"],
        "item_inspect" | "item_update" => &["id"],
        "relation_add" | "relation_remove" => &["kind", "source", "target"],
        _ => &[],
    };
    for field in positional {
        a[*field] = json!(text(
            args.get(at)
                .ok_or_else(|| Error::new("usage", format!("missing {field}")))?
        )?);
        at += 1;
    }
    let mut singular = a
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    while at < args.len() {
        let flag = text(&args[at])?;
        at += 1;
        if flag == "--raw" && name == "item_inspect" {
            name = "item_inspect_raw".into();
            if !singular.insert(flag) {
                return Err(Error::new("usage", "duplicate raw"));
            }
            continue;
        }
        if matches!(flag.as_str(), "--session-namespace" | "--session-id") {
            if !singular.insert(flag.clone()) {
                return Err(Error::new("invalid_argument", "duplicate identity field"));
            }
            let value = text(
                args.get(at)
                    .ok_or_else(|| Error::new("usage", "missing identity value"))?,
            )?;
            at += 1;
            if a.get("identity").is_none() {
                a["identity"] = json!({});
            }
            a["identity"][if flag == "--session-namespace" {
                "namespace"
            } else {
                "id"
            }] = json!(value);
            continue;
        }
        if flag == "--identity" {
            return Err(Error::new(
                "usage",
                "use --session-namespace and --session-id",
            ));
        }
        let (field, list) = match flag.as_str() {
            "--label" => (
                if matches!(name.as_str(), "item_list" | "item_ready") {
                    "labels_all"
                } else {
                    "labels"
                },
                true,
            ),
            "--authorize" => ("authorization", true),
            "--all" => ("include_terminal", false),
            "--claim" => ("claim_id", false),
            "--continue" => ("continue_id", false),
            "--run" => ("run_id", false),
            "--clear-labels" => ("labels", false),
            _ => {
                let key = flag
                    .strip_prefix("--")
                    .ok_or_else(|| Error::new("usage", "unexpected positional argument"))?;
                let key = key.replace('-', "_");
                let properties = schema(&name);
                let Some((field, _)) = properties["properties"]
                    .as_object()
                    .unwrap()
                    .get_key_value(&key)
                else {
                    return Err(Error::new("usage", format!("unknown {flag}")));
                };
                (Box::leak(field.clone().into_boxed_str()) as &str, false)
            }
        };
        if !list && !singular.insert(field.to_owned()) {
            return Err(Error::new("invalid_argument", format!("duplicate {field}")));
        }
        if flag == "--clear-labels" {
            if a.get("labels").is_some() {
                return Err(Error::new("invalid_argument", "clear-labels conflicts"));
            }
            a["labels"] = json!([]);
            continue;
        }
        let s = schema(&name);
        let spec = &s["properties"][field];
        if spec.is_null() {
            return Err(Error::new("usage", format!("unknown {flag}")));
        }
        let value = if spec["type"] == "boolean" || spec["const"].is_boolean() {
            json!(true)
        } else {
            let raw = args
                .get(at)
                .ok_or_else(|| Error::new("usage", format!("missing value for {flag}")))?;
            at += 1;
            if field == "destination" {
                json!(scopes::path(&absolute(raw)?))
            } else if list && field == "authorization" {
                serde_json::from_str(&text(raw)?)
                    .map_err(|_| Error::new("invalid_argument", "invalid authorization JSON"))?
            } else if spec["type"] == "integer" {
                json!(
                    text(raw)?
                        .parse::<u64>()
                        .map_err(|_| Error::new("invalid_argument", "expected integer"))?
                )
            } else if field == "identity" {
                serde_json::from_str(&text(raw)?)
                    .map_err(|_| Error::new("invalid_argument", "invalid identity JSON"))?
            } else {
                json!(text(raw)?)
            }
        };
        if list {
            if singular.contains(field) {
                return Err(Error::new("invalid_argument", "set/clear conflict"));
            }
            if a.get(field).is_none() {
                a[field] = json!([]);
            }
            a[field].as_array_mut().unwrap().push(value);
        } else {
            a[field] = value;
        }
    }
    let definition = wire::catalog()["operations"][&name]["input"]["$ref"]
        .as_str()
        .unwrap()
        .strip_prefix("#/$defs/")
        .unwrap();
    wire::validate(definition, &a)?;
    if let Some(destination) = a["destination"].as_str() {
        let canonical = std::fs::canonicalize(decode_path(destination)?).map_err(|e| {
            Error::new(
                "invalid_argument",
                format!("destination path unavailable: {e}"),
            )
        })?;
        a["destination"] = json!(scopes::path(&canonical));
    }
    if a["body"] == "-" {
        let mut body = String::new();
        io::stdin()
            .read_to_string(&mut body)
            .map_err(|e| Error::new("invalid_argument", e.to_string()))?;
        a["body"] = json!(body);
    }
    Ok((name, a))
}
pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<_> = args.collect();
    let mut prefix = args.iter();
    let mut json_mode = false;
    while let Some(option) = prefix.next() {
        match option.to_str() {
            Some("--worktree") => {
                prefix.next();
            }
            Some("--json") => json_mode = true,
            Some("--version" | "--help" | "-V" | "-h") => {}
            _ => break,
        }
    }

    let result = request(&args).and_then(|(name, a)| {
        if name == "meta" {
            Ok(a)
        } else {
            call(&name, &a)
        }
    });
    match result {
        Ok(value) => {
            if json_mode {
                println!("{}", json!({"ok":true,"result":value}));
            } else if let Some(h) = value["help"].as_str() {
                println!("{h}");
            } else if let Some(v) = value["version"].as_str() {
                println!("work {v}");
            } else {
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            }
            0
        }
        Err(e) => {
            let code = e.code;
            let v = error(e);
            if json_mode {
                println!("{}", json!({"ok":false,"error":v}));
            } else {
                eprintln!("work: {code}: {}", v["message"].as_str().unwrap());
            }
            wire::catalog()["error_exit_codes"][code]
                .as_i64()
                .unwrap_or(1) as i32
        }
    }
}

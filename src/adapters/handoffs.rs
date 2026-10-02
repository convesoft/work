//! Handoff CLI/MCP mapping; retention and ownership stay in core.
use super::cli::CliError;
use serde_json::{Map, Value, json};
use std::io::{self, Read};
use work::core::{execution::ExecutionOperations, handoffs::HandoffInput, project::Project};
pub(super) const HELP: &str = "Usage: work handoff create --from ITEM... --to ITEM... --body TEXT|- [--session-namespace N --session-id S] [--workspace ID] [--authorize JSON]... | inspect ID | list [--to ITEM] | receivers ID --to ITEM... | prune [ID...]\nContext is opaque and retained until all receivers resolve. Receiver changes replace the exact set. Cancel receivers with ordinary item close --reason. Close with outgoing context: item close ID [--reason TEXT] [--handoff JSON]... (create input without authorization).";
fn invalid(message: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", message)
}
pub(super) fn close_inputs(value: Option<&Value>) -> Result<Vec<HandoffInput>, CliError> {
    value
        .map(|v| {
            v.as_array()
                .ok_or_else(|| invalid("handoffs must be an array"))?
                .iter()
                .map(|v| HandoffInput::from_json(v).map_err(Into::into))
                .collect()
        })
        .unwrap_or_else(|| Ok(vec![]))
}
pub(super) fn input_schema() -> Value {
    let s = json!({"type":"string"});
    let items = json!({"type":"array","minItems":1,"items":s});
    json!({"type":"object","properties":{"from_items":items,"to_items":items,"body":s,"session":{"type":"object","properties":{"namespace":s,"id":s},"required":["namespace","id"],"additionalProperties":false},"workspace_id":s},"required":["from_items","to_items","body"],"additionalProperties":false})
}
pub(super) fn tools() -> Vec<Value> {
    let mut result = Vec::new();
    for (verb, required) in [
        ("create", vec!["from_items", "to_items", "body"]),
        ("inspect", vec!["handoff_id"]),
        ("list", vec![]),
        ("receivers", vec!["handoff_id", "to_items"]),
        ("prune", vec![]),
    ] {
        let s = json!({"type":"string"});
        let mut props = json!({"worktree":s});
        match verb {
            "create" => {
                props
                    .as_object_mut()
                    .unwrap()
                    .extend(input_schema()["properties"].as_object().unwrap().clone());
                props["authorization"] = json!({"type":"array","items":{"type":"object","properties":{"claim_id":s,"session":input_schema()["properties"]["session"]},"required":["claim_id","session"],"additionalProperties":false}});
            }
            "inspect" => props["handoff_id"] = s,
            "list" => props["to_item"] = s,
            "receivers" => {
                props["handoff_id"] = s;
                props["to_items"] = input_schema()["properties"]["to_items"].clone();
            }
            "prune" => props["ids"] = json!({"type":"array","items":s}),
            _ => unreachable!(),
        }
        result.push(json!({"name":format!("handoff_{verb}"),"description":format!("{verb} opaque receiver-scoped context; no graph edges or executor control."),"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false}}));
    }
    result
}
pub(super) fn from_cli(words: &[String]) -> Result<(String, Map<String, Value>), CliError> {
    let verb = words.first().ok_or_else(|| invalid(HELP))?.clone();
    let mut m = Map::new();
    let mut at = 1;
    if matches!(verb.as_str(), "inspect" | "receivers") {
        m.insert(
            "handoff_id".into(),
            json!(words.get(at).ok_or_else(|| invalid("missing handoff ID"))?),
        );
        at += 1;
    }
    if verb == "prune" {
        if words.len() > 1 {
            m.insert("ids".into(), json!(&words[1..]));
        }
        return Ok((verb, m));
    }
    let mut session = Map::new();
    while at < words.len() {
        let flag = &words[at];
        let value = words
            .get(at + 1)
            .ok_or_else(|| invalid(format!("missing {flag} value")))?;
        at += 2;
        let key = match flag.as_str() {
            "--from" => "from_items",
            "--to" if verb == "list" => "to_item",
            "--to" => "to_items",
            "--body" => "body",
            "--workspace" => "workspace_id",
            "--session-namespace" => "namespace",
            "--session-id" => "id",
            "--authorize" => "authorization",
            _ => return Err(invalid(format!("unknown handoff argument {flag}"))),
        };
        if matches!(key, "namespace" | "id") {
            if session.insert(key.into(), json!(value)).is_some() {
                return Err(invalid("duplicate session field"));
            }
        } else if matches!(key, "from_items" | "to_items" | "authorization") {
            let values = m
                .entry(key)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap();
            values.push(if key == "authorization" {
                serde_json::from_str(value).map_err(|_| invalid("invalid authorization JSON"))?
            } else {
                json!(value)
            });
            // Accept repeated flags as well as multiple endpoints per flag.
            if key != "authorization" {
                while at < words.len() && !words[at].starts_with("--") {
                    values.push(json!(words[at]));
                    at += 1;
                }
            }
        } else if m.insert(key.into(), json!(value)).is_some() {
            return Err(invalid(format!("duplicate {flag}")));
        }
    }
    if !session.is_empty() {
        m.insert("session".into(), Value::Object(session));
    }
    // Validate structure before consuming stdin or accessing a repository.
    validate(&verb, &m)?;
    if m.get("body") == Some(&json!("-")) {
        let mut body = String::new();
        io::stdin()
            .read_to_string(&mut body)
            .map_err(|e| invalid(e.to_string()))?;
        m.insert("body".into(), json!(body));
    }
    Ok((verb, m))
}
fn validate(verb: &str, m: &Map<String, Value>) -> Result<(), CliError> {
    let tool = tools()
        .into_iter()
        .find(|v| v["name"] == format!("handoff_{verb}"))
        .ok_or_else(|| invalid("unknown handoff operation"))?;
    let props = tool["inputSchema"]["properties"].as_object().unwrap();
    if m.keys().any(|k| !props.contains_key(k))
        || tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| !m.contains_key(k.as_str().unwrap()))
    {
        return Err(invalid("unknown or missing handoff fields"));
    }
    for (k, v) in m {
        let valid = match k.as_str() {
            "session" => work::core::coordination::SessionIdentity::from_json(v).is_ok(),
            "authorization" => super::claims::authorization(Some(v)).is_ok(),
            "from_items" | "to_items" | "ids" => {
                v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
            }
            _ => v.is_string(),
        };
        if !valid {
            return Err(invalid(format!("invalid {k}")));
        }
    }
    Ok(())
}
pub(super) fn execute(
    project: &Project,
    verb: &str,
    m: &Map<String, Value>,
) -> Result<Value, CliError> {
    validate(verb, m)?;
    let mut ops = ExecutionOperations::new(project.clone());
    ops.authorization = super::claims::authorization(m.get("authorization"))?;
    let text = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("");
    let array = |k: &str| {
        m.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Ok(match verb {
        "create" => {
            let mut input = m.clone();
            input.remove("worktree");
            input.remove("authorization");
            ops.handoff_create(&HandoffInput::from_json(&Value::Object(input))?)?
        }
        "inspect" => ops.handoff_inspect(text("handoff_id"))?,
        "list" => ops.handoff_list(m.get("to_item").and_then(Value::as_str))?,
        "receivers" => ops.handoff_receivers(text("handoff_id"), &array("to_items"))?,
        "prune" => ops.handoff_prune(m.contains_key("ids").then(|| array("ids")).as_deref())?,
        _ => return Err(invalid("unknown handoff operation")),
    })
}

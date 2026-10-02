//! CLI/MCP context mappings; no independent registries or lifecycle policy.
use super::cli::CliError;
use serde_json::{Map, Value, json};
use std::path::Path;
use work::core::coordination::SessionIdentity;
use work::core::execution::ExecutionOperations;
use work::core::project::Project;
pub(super) const HELP: &str = "Usage: work workspace register PATH [--branch B] [--commit C] | inspect ID | list | bind ITEM WORKSPACE_ID | unbind ITEM\nwork session set RUN NAME --namespace N --session-id S [--availability available|unavailable|unknown --observed-at TIME] | list RUN | remove RUN NAME\nWork records externally managed context. It never creates or deletes worktrees or controls sessions. Cleanup and run finalization are not implemented.";
fn invalid(s: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", s)
}
fn spec(name: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    Some(match name {
        "workspace_register" => (&["path"], &["branch", "commit"]),
        "workspace_inspect" => (&["workspace_id"], &[]),
        "workspace_list" => (&[], &[]),
        "workspace_bind" => (&["item", "workspace_id"], &[]),
        "workspace_unbind" => (&["item"], &[]),
        "session_set" => (&["run_id", "name", "session"], &["availability"]),
        "session_list" => (&["run_id"], &[]),
        "session_remove" => (&["run_id", "name"], &[]),
        _ => return None,
    })
}
pub(super) fn from_cli(
    noun: &str,
    words: &[String],
) -> Result<(String, Map<String, Value>), CliError> {
    let verb = words.first().ok_or_else(|| invalid(HELP))?;
    let name = format!("{noun}_{verb}");
    spec(&name).ok_or_else(|| invalid("unknown context operation"))?;
    let mut m = Map::new();
    let positional: &[&str] = match name.as_str() {
        "workspace_register" => &["path"],
        "workspace_inspect" => &["workspace_id"],
        "workspace_bind" => &["item", "workspace_id"],
        "workspace_unbind" => &["item"],
        "session_set" | "session_remove" => &["run_id", "name"],
        "session_list" => &["run_id"],
        _ => &[],
    };
    let mut at = 1;
    for key in positional {
        m.insert(
            (*key).into(),
            json!(
                words
                    .get(at)
                    .ok_or_else(|| invalid(format!("missing {key}")))?
            ),
        );
        at += 1;
    }
    let mut session = Map::new();
    let mut availability = Map::new();
    while at < words.len() {
        let flag = &words[at];
        let key = match flag.as_str() {
            "--branch" => "branch",
            "--commit" => "commit",
            "--namespace" => "namespace",
            "--session-id" => "id",
            "--availability" => "state",
            "--observed-at" => "observed_at",
            _ => return Err(invalid(format!("unknown argument {flag}"))),
        };
        let value = words
            .get(at + 1)
            .ok_or_else(|| invalid(format!("missing {flag} value")))?;
        let target = match key {
            "namespace" | "id" => &mut session,
            "state" | "observed_at" => &mut availability,
            _ => &mut m,
        };
        if target.insert(key.into(), json!(value)).is_some() {
            return Err(invalid(format!("duplicate {flag}")));
        }
        at += 2;
    }
    if !session.is_empty() {
        m.insert("session".into(), json!(session));
    }
    if !availability.is_empty() {
        m.insert("availability".into(), json!(availability));
    }
    validate(&name, &m)?;
    Ok((name, m))
}
fn validate(name: &str, m: &Map<String, Value>) -> Result<(), CliError> {
    let (required, optional) = spec(name).ok_or_else(|| invalid("unknown context operation"))?;
    if required.iter().any(|k| !m.contains_key(*k))
        || m.keys().any(|k| {
            k != "worktree" && !required.contains(&k.as_str()) && !optional.contains(&k.as_str())
        })
    {
        return Err(invalid("unknown or missing context fields"));
    }
    for (key, value) in m {
        match key.as_str() {
            "session" => {
                SessionIdentity::from_json(value).map_err(|e| invalid(e.message))?;
            }
            "availability" => work::core::sessions::validate_availability(value)?,
            "run_id" | "workspace_id" => {
                if !value
                    .as_str()
                    .is_some_and(work::core::coordination::valid_id)
                {
                    return Err(invalid(format!("{key} must be a full UUIDv4")));
                }
            }
            "name" | "path" => {
                if !value.as_str().is_some_and(|s| !s.is_empty()) {
                    return Err(invalid(format!("{key} must be nonempty text")));
                }
            }
            _ if value.is_string() => {}
            _ => return Err(invalid(format!("{key} must be text"))),
        }
    }
    Ok(())
}
pub(super) fn execute(
    project: &Project,
    name: &str,
    m: &Map<String, Value>,
) -> Result<Value, CliError> {
    validate(name, m)?;
    let ops = ExecutionOperations::new(project.clone());
    let text = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("");
    let optional = |k: &str| m.get(k).and_then(Value::as_str);
    Ok(match name {
        "workspace_register" => ops.workspace_register(
            Path::new(text("path")),
            optional("branch"),
            optional("commit"),
        )?,
        "workspace_inspect" => ops.workspace_inspect(text("workspace_id"))?,
        "workspace_list" => ops.workspace_list()?,
        "workspace_bind" => ops.workspace_bind(text("item"), text("workspace_id"))?,
        "workspace_unbind" => ops.workspace_unbind(text("item"))?,
        "session_list" => ops.session_list(text("run_id"))?,
        "session_remove" => ops.session_remove(text("run_id"), text("name"))?,
        "session_set" => ops.session_set(
            text("run_id"),
            text("name"),
            &SessionIdentity::from_json(&m["session"])?,
            m.get("availability"),
        )?,
        _ => return Err(invalid("unknown context operation")),
    })
}
pub(super) fn tools() -> Vec<Value> {
    let s = json!({"type":"string"});
    let session = json!({"type":"object","properties":{"namespace":s,"id":s},"required":["namespace","id"],"additionalProperties":false});
    let availability = json!({"type":"object","properties":{"state":{"type":"string","enum":["available","unavailable","unknown"]},"observed_at":s},"required":["state","observed_at"],"additionalProperties":false});
    ["workspace_register", "workspace_inspect", "workspace_list", "workspace_bind", "workspace_unbind", "session_set", "session_list", "session_remove"].into_iter().map(|name| {
        let (required, optional) = spec(name).unwrap();
        let mut properties = json!({"worktree":s});
        for k in required.iter().chain(optional) {
            properties[*k] = match *k { "session" => session.clone(), "availability" => availability.clone(), _ => s.clone() };
        }
        json!({"name":name,"description":format!("{name}: record or inspect external context; no process/worktree control."),"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect()
}

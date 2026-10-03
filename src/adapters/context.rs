//! CLI/MCP context mappings; no independent registries or lifecycle policy.
use super::cli::CliError;
use serde_json::{Map, Value, json};
use std::path::Path;
use work::core::coordination::SessionIdentity;
use work::core::execution::ExecutionOperations;
use work::core::project::Project;
pub(super) const HELP: &str = "Usage: work workspace register PATH [--branch B] [--commit C] | inspect ID | list | bind ITEM WORKSPACE_ID | unbind ITEM\nwork workspace cleanup begin ID --item ITEM --controller-workspace ID | report ID (--removed | --failure TEXT) | cancel ID\nwork session set RUN NAME --namespace N --session-id S [--availability available|unavailable|unknown --observed-at TIME] | list RUN | remove RUN NAME\nCleanup begin attests required commits/results are retained; transfer material bindings first. External tooling removes the target after begin returns. Work never creates or deletes worktrees, controls sessions, or closes the cleanup item. Run finalization is not implemented.";
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
        "workspace_cleanup_begin" => (&["workspace_id", "item", "controller_workspace_id"], &[]),
        "workspace_cleanup_report" => (&["workspace_id", "removed"], &["failure"]),
        "workspace_cleanup_cancel" => (&["workspace_id"], &[]),
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
    let cleanup = noun == "workspace" && verb == "cleanup";
    let name = if cleanup {
        format!(
            "workspace_cleanup_{}",
            words.get(1).ok_or_else(|| invalid(HELP))?
        )
    } else {
        format!("{noun}_{verb}")
    };
    spec(&name).ok_or_else(|| invalid("unknown context operation"))?;
    let mut m = Map::new();
    let positional: &[&str] = match name.as_str() {
        "workspace_register" => &["path"],
        "workspace_inspect"
        | "workspace_cleanup_begin"
        | "workspace_cleanup_report"
        | "workspace_cleanup_cancel" => &["workspace_id"],
        "workspace_bind" => &["item", "workspace_id"],
        "workspace_unbind" => &["item"],
        "session_set" | "session_remove" => &["run_id", "name"],
        "session_list" => &["run_id"],
        _ => &[],
    };
    let mut at = if cleanup { 2 } else { 1 };
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
        if flag == "--removed" {
            if m.insert("removed".into(), json!(true)).is_some() {
                return Err(invalid("duplicate or conflicting --removed"));
            }
            at += 1;
            continue;
        }
        let key = match flag.as_str() {
            "--item" => "item",
            "--controller-workspace" => "controller_workspace_id",
            "--failure" => "failure",
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
    if m.contains_key("failure") {
        if m.contains_key("removed") {
            return Err(invalid("--failure conflicts with --removed"));
        }
        m.insert("removed".into(), json!(false));
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
    if name == "workspace_cleanup_report"
        && (!m["removed"].is_boolean()
            || (m["removed"] == true && m.contains_key("failure"))
            || (m["removed"] == false && !m.contains_key("failure")))
    {
        return Err(invalid("failure is required only when removed is false"));
    }
    for (key, value) in m {
        match key.as_str() {
            "removed" if value.is_boolean() => {}
            "session" => {
                SessionIdentity::from_json(value).map_err(|e| invalid(e.message))?;
            }
            "availability" => work::core::sessions::validate_availability(value)?,
            "run_id" | "workspace_id" | "controller_workspace_id" => {
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
        "workspace_cleanup_begin" => ops.workspace_cleanup_begin(
            text("workspace_id"),
            text("item"),
            text("controller_workspace_id"),
        )?,
        "workspace_cleanup_report" => ops.workspace_cleanup_report(
            text("workspace_id"),
            m["removed"].as_bool().unwrap(),
            optional("failure"),
        )?,
        "workspace_cleanup_cancel" => ops.workspace_cleanup_cancel(text("workspace_id"))?,
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
    ["workspace_register", "workspace_inspect", "workspace_list", "workspace_bind", "workspace_unbind", "workspace_cleanup_begin", "workspace_cleanup_report", "workspace_cleanup_cancel", "session_set", "session_list", "session_remove"].into_iter().map(|name| {
        let (required, optional) = spec(name).unwrap();
        let mut properties = json!({"worktree":s});
        for k in required.iter().chain(optional) {
            properties[*k] = match *k { "session" => session.clone(), "availability" => availability.clone(), "removed" => json!({"type":"boolean"}), _ => s.clone() };
        }
        json!({"name":name,"description":format!("{name}: record or inspect external context; no process/worktree control."),"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect()
}

//! Claim transport mapping; ownership policy lives in the core.
use super::cli::{CliError, one_item_value};
use serde_json::{Map, Value, json};
use work::core::claims::{ClaimAuthorization, ClaimStore};
use work::core::coordination::{CoordinationGuard, ExecutionError, SessionIdentity, valid_id};
use work::core::execution::ExecutionOperations;
use work::core::project::Project;
pub(super) const HELP: &str = "Usage: work claim acquire ITEM --actor A --session-namespace N --session-id S | inspect CLAIM_ID | list [--item ITEM] [--current] | release CLAIM_ID --session-namespace N --session-id S [--reason TEXT] | recover CLAIM_ID --actor A --reason TEXT --executors-stopped | reassign CLAIM_ID --actor A --session-namespace N --session-id S --reason TEXT --executors-stopped\nClaim ID plus session identity authorizes ownership. No separate token or automatic expiry.";
fn invalid(s: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", s)
}
pub(super) fn authorization(value: Option<&Value>) -> Result<Vec<ClaimAuthorization>, CliError> {
    let Some(v) = value else {
        return Ok(Vec::new());
    };
    let array = v
        .as_array()
        .ok_or_else(|| invalid("authorization must be an array"))?;
    array
        .iter()
        .map(|v| {
            let m = v
                .as_object()
                .ok_or_else(|| invalid("authorization must be an object"))?;
            if m.len() != 2 || !m.contains_key("claim_id") || !m.contains_key("session") {
                return Err(invalid("authorization needs exactly claim_id and session"));
            }
            let id = m["claim_id"]
                .as_str()
                .filter(|id| valid_id(id))
                .ok_or_else(|| invalid("claim_id must be a full UUIDv4"))?;
            let session =
                SessionIdentity::from_json(&m["session"]).map_err(|e| invalid(e.message))?;
            Ok(ClaimAuthorization {
                claim_id: id.into(),
                session,
            })
        })
        .collect()
}
pub(super) fn from_cli(words: &[String]) -> Result<(String, Map<String, Value>), CliError> {
    let verb = words.first().ok_or_else(|| invalid(HELP))?.clone();
    if !matches!(
        verb.as_str(),
        "acquire" | "inspect" | "list" | "release" | "recover" | "reassign"
    ) {
        return Err(CliError::new("usage", "unknown claim command"));
    }
    let mut m = Map::new();
    let mut at = 1;
    if verb != "list" {
        let id = words
            .get(at)
            .ok_or_else(|| invalid("missing item or claim ID"))?;
        m.insert(
            if verb == "acquire" {
                "item"
            } else {
                "claim_id"
            }
            .into(),
            json!(id),
        );
        at += 1;
    }
    let mut session = Map::new();
    while at < words.len() {
        let flag = words[at].as_str();
        at += 1;
        let key = match flag {
            "--actor" => "actor",
            "--reason" => "reason",
            "--item" => "item",
            "--session-namespace" => "namespace",
            "--session-id" => "id",
            "--current" => "current_only",
            "--executors-stopped" => "executors_stopped",
            _ => return Err(invalid(format!("unknown claim argument {flag}"))),
        };
        let value = if matches!(key, "current_only" | "executors_stopped") {
            json!(true)
        } else {
            let value = words
                .get(at)
                .ok_or_else(|| invalid(format!("missing {flag} value")))?;
            at += 1;
            json!(value)
        };
        let target = if matches!(key, "namespace" | "id") {
            &mut session
        } else {
            &mut m
        };
        if target.insert(key.into(), value).is_some() {
            return Err(invalid(format!("duplicate {flag}")));
        }
    }
    if !session.is_empty() {
        m.insert("session".into(), Value::Object(session));
    }
    validate(&verb, &m)?;
    Ok((verb, m))
}
fn text<'a>(m: &'a Map<String, Value>, key: &str) -> &'a str {
    m.get(key).and_then(Value::as_str).unwrap_or("")
}
fn validate(verb: &str, m: &Map<String, Value>) -> Result<(), CliError> {
    let (required, optional): (&[&str], &[&str]) = match verb {
        "acquire" => (&["item", "actor", "session"], &[]),
        "inspect" => (&["claim_id"], &[]),
        "list" => (&[], &["item", "current_only"]),
        "release" => (&["claim_id", "session"], &["reason"]),
        "recover" => (&["claim_id", "actor", "reason", "executors_stopped"], &[]),
        "reassign" => (
            &[
                "claim_id",
                "actor",
                "reason",
                "executors_stopped",
                "session",
            ],
            &[],
        ),
        _ => return Err(invalid("unknown claim operation")),
    };
    if required.iter().any(|k| !m.contains_key(*k))
        || m.keys().any(|k| {
            k != "worktree" && !required.contains(&k.as_str()) && !optional.contains(&k.as_str())
        })
    {
        return Err(invalid("unknown or missing claim fields"));
    }
    for (key, value) in m {
        let valid = match key.as_str() {
            "session" => SessionIdentity::from_json(value).is_ok(),
            "executors_stopped" | "current_only" => value.is_boolean(),
            _ => value.is_string(),
        };
        if !valid {
            return Err(invalid(format!("invalid {key}")));
        }
    }
    if m.contains_key("claim_id") && !valid_id(text(m, "claim_id")) {
        return Err(invalid("claim_id must be full UUIDv4"));
    }
    if m.contains_key("actor") && text(m, "actor").is_empty() {
        return Err(invalid("actor must be nonempty"));
    }
    if matches!(verb, "recover" | "reassign")
        && (text(m, "reason").is_empty() || m["executors_stopped"] != true)
    {
        return Err(invalid(
            "recovery requires reason and executors_stopped:true",
        ));
    }
    Ok(())
}
pub(super) fn execute(
    project: &Project,
    verb: &str,
    m: &Map<String, Value>,
) -> Result<Value, CliError> {
    validate(verb, m)?;
    let ops = ExecutionOperations::new(project.clone());
    if verb == "acquire" {
        let session = SessionIdentity::from_json(&m["session"])?;
        let (claim, item) = ops.acquire(text(m, "item"), text(m, "actor"), &session)?;
        return Ok(json!({"claim":claim,"item":one_item_value(project,&item)?,"changed":true}));
    }
    if verb == "reassign" {
        let session = SessionIdentity::from_json(&m["session"])?;
        let (claim, item) = ops.reassign(
            text(m, "claim_id"),
            text(m, "actor"),
            &session,
            text(m, "reason"),
            true,
        )?;
        return Ok(
            json!({"previous_claim_id":text(m,"claim_id"),"claim":claim,"item":one_item_value(project,&item)?,"changed":true}),
        );
    }
    let guard = CoordinationGuard::acquire(project, !matches!(verb, "inspect" | "list"))?;
    match verb {
        "inspect" => Ok(ClaimStore::inspect(&guard, text(m, "claim_id"))?.to_json()),
        "list" => {
            let item = m
                .get("item")
                .and_then(Value::as_str)
                .map(|s| {
                    if valid_id(s) {
                        Ok(s.to_owned())
                    } else {
                        super::cli::resolve(project, s)
                    }
                })
                .transpose()?;
            Ok(
                json!({"claims":ClaimStore::list(&guard,item.as_deref(),m.get("current_only")==Some(&json!(true)))?.iter().map(|c|c.to_json()).collect::<Vec<_>>()}),
            )
        }
        "release" => Ok(ClaimStore::release(
            &guard,
            text(m, "claim_id"),
            &SessionIdentity::from_json(&m["session"])?,
            text(m, "reason"),
        )?
        .to_json()),
        "recover" => Ok(ClaimStore::recover(
            &guard,
            text(m, "claim_id"),
            text(m, "actor"),
            text(m, "reason"),
            true,
        )?
        .to_json()),
        _ => Err(invalid("unknown claim operation")),
    }
}
pub(super) fn tools() -> Vec<Value> {
    let s = json!({"type":"string"});
    let session = json!({"type":"object","properties":{"namespace":s,"id":s},"required":["namespace","id"],"additionalProperties":false});
    let mut result = Vec::new();
    for (verb, required, optional) in [
        ("acquire", vec!["item", "actor", "session"], vec![]),
        ("inspect", vec!["claim_id"], vec![]),
        ("list", vec![], vec!["item", "current_only"]),
        ("release", vec!["claim_id", "session"], vec!["reason"]),
        (
            "recover",
            vec!["claim_id", "actor", "reason", "executors_stopped"],
            vec![],
        ),
        (
            "reassign",
            vec![
                "claim_id",
                "actor",
                "reason",
                "executors_stopped",
                "session",
            ],
            vec![],
        ),
    ] {
        let mut props = json!({"worktree":s});
        for k in required.iter().chain(optional.iter()) {
            props[*k] = match *k {
                "session" => session.clone(),
                "current_only" | "executors_stopped" => json!({"type":"boolean"}),
                _ => s.clone(),
            };
        }
        result.push(json!({"name":format!("claim_{verb}"),"description":format!("{verb} repository-wide ownership; no executor process control."),"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false}}));
    }
    result
}
impl From<ExecutionError> for CliError {
    fn from(e: ExecutionError) -> Self {
        let mut d = e.details;
        if let Some(path) = e.path {
            d["path"] = json!(super::cli::encode_path(&path));
        }
        Self::with_details(e.code, e.message, d)
    }
}

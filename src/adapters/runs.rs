//! Run and expansion transport mapping.
use super::cli::{CliError, template_preview_value};
use serde_json::{Map, Value, json};
use work::core::{
    context::ResolvedView,
    coordination::{CoordinationGuard, valid_id},
    execution::ExecutionOperations,
    project::Project,
    templates::{PreviewRequest, TemplateCatalog},
};
pub(super) const HELP: &str = "Usage: work run start ROOT [--workspace ID] [--output-workspace ID] | inspect RUN_ID | list [--all] | attach RUN_ID ITEM... | detach RUN_ID ITEM...";
fn invalid(s: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", s)
}
fn text<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}
pub(super) fn from_cli(
    noun: &str,
    words: &[String],
) -> Result<(String, Map<String, Value>), CliError> {
    let verb = words.first().ok_or_else(|| invalid(HELP))?;
    let name = format!("{noun}_{verb}");
    let mut m = Map::new();
    let mut at = 1;
    let key = match name.as_str() {
        "run_start" => Some("root"),
        "run_inspect" | "run_attach" | "run_detach" => Some("run_id"),
        "template_preview" | "template_expand" => Some("name"),
        "run_list" => None,
        _ => return Err(invalid("unknown run/template operation")),
    };
    if let Some(key) = key {
        m.insert(
            key.into(),
            json!(words.get(at).ok_or_else(|| invalid("missing identifier"))?),
        );
        at += 1;
    }
    if matches!(verb.as_str(), "attach" | "detach") {
        m.insert("items".into(), json!(&words[at..]));
        at = words.len();
    }
    while at < words.len() {
        let flag = &words[at];
        at += 1;
        let key = match flag.as_str() {
            "--workspace" => "default_workspace_id",
            "--output-workspace" => "output_workspace_id",
            "--all" => "include_terminal",
            "--run" => "run_id",
            "--root" => "root",
            "--param" => "parameters",
            "--existing" => "existing",
            "--authorize" => "authorization",
            _ => return Err(invalid(format!("unknown option {flag}"))),
        };
        if key == "include_terminal" {
            if m.insert(key.into(), json!(true)).is_some() {
                return Err(invalid("duplicate --all"));
            }
            continue;
        }
        let value = words
            .get(at)
            .ok_or_else(|| invalid(format!("missing {flag} value")))?;
        at += 1;
        if matches!(key, "parameters" | "existing") {
            let (k, v) = value
                .split_once('=')
                .ok_or_else(|| invalid("expected NAME=VALUE"))?;
            let map = m.entry(key).or_insert(json!({})).as_object_mut().unwrap();
            if map.insert(k.into(), json!(v)).is_some() {
                return Err(invalid("duplicate binding"));
            }
        } else if key == "authorization" {
            let v: Value =
                serde_json::from_str(value).map_err(|_| invalid("invalid authorization JSON"))?;
            m.entry(key)
                .or_insert(json!([]))
                .as_array_mut()
                .unwrap()
                .push(v);
        } else if m.insert(key.into(), json!(value)).is_some() {
            return Err(invalid("duplicate option"));
        }
    }
    validate(&name, &m)?;
    Ok((name, m))
}
pub(super) fn validate(name: &str, m: &Map<String, Value>) -> Result<(), CliError> {
    let schema = tools()
        .into_iter()
        .find(|t| t["name"] == name)
        .ok_or_else(|| invalid("unknown operation"))?;
    let props = schema["inputSchema"]["properties"].as_object().unwrap();
    for k in schema["inputSchema"]["required"].as_array().unwrap() {
        if !m.contains_key(k.as_str().unwrap()) {
            return Err(invalid(format!("missing {k}")));
        }
    }
    for (k, v) in m {
        let Some(spec) = props.get(k) else {
            return Err(invalid(format!("unknown {k}")));
        };
        let valid = match spec["type"].as_str().unwrap() {
            "string" => v.as_str().is_some_and(|s| !s.is_empty()),
            "boolean" => v.is_boolean(),
            "array" if k == "authorization" => super::claims::authorization(Some(v)).is_ok(),
            "array" => v.as_array().is_some_and(|a| {
                !a.is_empty() && a.iter().all(|s| s.as_str().is_some_and(|s| !s.is_empty()))
            }),
            "object" => v
                .as_object()
                .is_some_and(|a| a.iter().all(|(k, v)| !k.is_empty() && v.is_string())),
            _ => false,
        };
        if !valid {
            return Err(invalid(format!("invalid {k}")));
        }
    }
    for k in ["run_id", "default_workspace_id", "output_workspace_id"] {
        if text(m, k).is_some_and(|s| !valid_id(s)) {
            return Err(invalid(format!("{k} must be full UUIDv4")));
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
    let mut ops = ExecutionOperations::new(project.clone());
    ops.authorization = super::claims::authorization(m.get("authorization"))?;
    Ok(match name {
        "run_start" => ops.run_start(
            text(m, "root").unwrap(),
            text(m, "default_workspace_id").map(str::to_owned),
            text(m, "output_workspace_id").map(str::to_owned),
        )?,
        "run_inspect" => ops.run_inspect(text(m, "run_id").unwrap())?,
        "run_list" => ops.run_list(m.get("include_terminal") == Some(&json!(true)))?,
        "run_attach" | "run_detach" => ops.run_membership(
            text(m, "run_id").unwrap(),
            &m["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            name == "run_attach",
        )?,
        "template_preview" | "template_expand" => {
            let mut req = PreviewRequest {
                root: text(m, "root").map(str::to_owned),
                parameters: Default::default(),
                existing: Default::default(),
            };
            for (field, target) in [
                ("parameters", &mut req.parameters),
                ("existing", &mut req.existing),
            ] {
                if let Some(map) = m.get(field).and_then(Value::as_object) {
                    for (k, v) in map {
                        target.insert(k.clone(), v.as_str().unwrap().to_owned());
                    }
                }
            }
            if name == "template_expand" {
                ops.expand(text(m, "name").unwrap(), &req, text(m, "run_id"))?
            } else {
                let catalog = TemplateCatalog::load_from_root(&project.worktree_root)
                    .map_err(|e| CliError::new("io", e.to_string()))?;
                let view = if let Some(id) = text(m, "run_id") {
                    let g = CoordinationGuard::acquire(project, false)?;
                    let v = ResolvedView::load(&g)?;
                    let root = &v.runs.get(id)?.manifest.root_item_id;
                    if req.root.as_ref().is_some_and(|r| r != root) {
                        return Err(invalid("root disagrees with run"));
                    }
                    req.root = Some(root.clone());
                    let preview = catalog.preview(text(m, "name").unwrap(), &req, &v.store)?;
                    return Ok(json!({"preview":template_preview_value(&preview)}));
                } else {
                    ops.view()?
                };
                super::storage::attach_read(
                    project,
                    json!({"preview":template_preview_value(&catalog.preview(text(m,"name").unwrap(),&req,&view)?)}),
                    ops.take_read_storage(),
                )
            }
        }
        _ => return Err(invalid("unknown operation")),
    })
}
pub(super) fn tools() -> Vec<Value> {
    let s = json!({"type":"string"});
    let session = json!({"type":"object","properties":{"namespace":s,"id":s},"required":["namespace","id"],"additionalProperties":false});
    let mut out = Vec::new();
    for (name, required, optional) in [
        (
            "run_start",
            vec!["root"],
            vec!["default_workspace_id", "output_workspace_id"],
        ),
        ("run_inspect", vec!["run_id"], vec![]),
        ("run_list", vec![], vec!["include_terminal"]),
        ("run_attach", vec!["run_id", "items"], vec![]),
        ("run_detach", vec!["run_id", "items"], vec![]),
        (
            "template_preview",
            vec!["name"],
            vec!["root", "run_id", "parameters", "existing"],
        ),
        (
            "template_expand",
            vec!["name"],
            vec!["root", "run_id", "parameters", "existing", "authorization"],
        ),
    ] {
        let mut props = json!({"worktree":s});
        for k in required.iter().chain(optional.iter()) {
            props[*k] = match *k {
                "include_terminal" => json!({"type":"boolean"}),
                "items" => json!({"type":"array","items":s,"minItems":1}),
                "parameters" | "existing" => json!({"type":"object","additionalProperties":s}),
                "authorization" => {
                    json!({"type":"array","items":{"type":"object","properties":{"claim_id":s,"session":session},"required":["claim_id","session"],"additionalProperties":false}})
                }
                _ => s.clone(),
            };
        }
        out.push(json!({"name":name,"description":format!("{name}: file-backed runs and mixed template items; no implicit finalization."),"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false}}));
    }
    out
}

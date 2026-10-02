//! Independent, receiver-scoped opaque context.
//! @mara implements DES-CONTEXT-API
//! @mara implements DES-HANDOFF-RECORDS
use super::context::ResolvedView;
use super::coordination::*;
use super::graph::ItemGraph;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct HandoffInput {
    pub from_items: Vec<String>,
    pub to_items: Vec<String>,
    pub body: String,
    pub session: Option<SessionIdentity>,
    pub workspace_id: Option<String>,
}
impl HandoffInput {
    pub fn from_json(v: &Value) -> ExecutionResult<Self> {
        exact_keys(
            v,
            &["from_items", "to_items", "body"],
            &["session", "workspace_id"],
        )
        .map_err(argument)?;
        let result = Self {
            from_items: inputs(v, "from_items")?,
            to_items: inputs(v, "to_items")?,
            body: string(v, "body").map_err(argument)?,
            session: v
                .get("session")
                .map(SessionIdentity::from_json)
                .transpose()
                .map_err(argument)?,
            workspace_id: v
                .get("workspace_id")
                .map(|_| string(v, "workspace_id"))
                .transpose()
                .map_err(argument)?,
        };
        if result
            .workspace_id
            .as_deref()
            .is_some_and(|id| !valid_id(id))
        {
            return Err(ExecutionError::new(
                "invalid_argument",
                "workspace_id must be a full entity ID",
            ));
        }
        Ok(result)
    }
}
fn argument(e: ExecutionError) -> ExecutionError {
    ExecutionError::new("invalid_argument", e.message)
}
fn inputs(v: &Value, key: &str) -> ExecutionResult<Vec<String>> {
    let values = v[key].as_array().ok_or_else(|| {
        ExecutionError::new("invalid_argument", format!("{key} must be an array"))
    })?;
    let mut result = Vec::new();
    for value in values {
        result.push(
            value
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ExecutionError::new(
                        "invalid_argument",
                        format!("{key} must contain item references"),
                    )
                })?
                .to_owned(),
        );
    }
    if result.is_empty() {
        return Err(ExecutionError::new(
            "invalid_argument",
            format!("{key} must be nonempty"),
        ));
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Handoff {
    pub header: Value,
    pub body: String,
    pub path: PathBuf,
    pub(crate) source: EntitySource,
    pub(crate) relative: PathBuf,
}
impl Handoff {
    pub fn id(&self) -> &str {
        self.header["id"].as_str().unwrap()
    }
    pub fn receivers(&self) -> impl Iterator<Item = &str> {
        self.header["to_items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
    }
    pub fn to_json(&self) -> Value {
        let mut value = self.header.clone();
        value["body"] = json!(self.body);
        value["path"] = json!(encode_path(&self.path));
        value
    }
}
pub struct HandoffStore;
impl HandoffStore {
    pub fn load(g: &CoordinationGuard) -> ExecutionResult<Vec<Handoff>> {
        g.names(Path::new("handoffs"))?
            .iter()
            .map(|name| {
                let id = name
                    .strip_suffix(".md")
                    .filter(|id| valid_id(id))
                    .ok_or_else(|| {
                        ExecutionError::new("invalid_format", "unexpected handoff entry")
                            .at(g.root_path().join("handoffs").join(name))
                    })?;
                Self::inspect(g, id)
            })
            .collect()
    }
    pub fn inspect(g: &CoordinationGuard, id: &str) -> ExecutionResult<Handoff> {
        if !valid_id(id) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "handoff ID must be full",
            ));
        }
        let relative = PathBuf::from(format!("handoffs/{id}.md"));
        let path = g.root_path().join(&relative);
        let source = g
            .optional(&relative)?
            .ok_or_else(|| ExecutionError::new("not_found", "handoff not found").at(&path))?;
        decode(g, &source.raw, id, &path)
            .map(|(header, body)| Handoff {
                header,
                body,
                path: path.clone(),
                source,
                relative,
            })
            .map_err(|mut e| {
                e.path = Some(path);
                e
            })
    }
    pub(crate) fn create(g: &CoordinationGuard, input: &HandoffInput) -> ExecutionResult<Handoff> {
        let id = new_id()?;
        let mut header = json!({"format_version":1,"store_id":g.metadata.store_id,"recovery_generation":g.metadata.recovery_generation,"id":id,"from_items":input.from_items,"to_items":input.to_items,"created_at":now_timestamp()});
        if let Some(s) = &input.session {
            header["session"] = s.to_json();
        }
        if let Some(w) = &input.workspace_id {
            header["workspace_id"] = json!(w);
        }
        let relative = PathBuf::from(format!("handoffs/{id}.md"));
        let path = g.root_path().join(&relative);
        g.create(&relative, &encode(&header, &input.body))
            .map_err(|e| publication_error(e, &id, &path, "created"))?;
        Self::inspect(g, &id).map_err(|mut e| {
            e.details["publication"] = json!("published");
            publication_error(e, &id, &path, "created")
        })
    }
    pub(crate) fn receivers(
        g: &CoordinationGuard,
        id: &str,
        receivers: Vec<String>,
    ) -> ExecutionResult<(Handoff, bool)> {
        let mut handoff = Self::inspect(g, id)?;
        let changed = handoff.header["to_items"] != json!(receivers);
        if changed {
            handoff.header["to_items"] = json!(receivers);
            g.replace(
                &handoff.relative,
                &handoff.source,
                &encode(&handoff.header, &handoff.body),
            )
            .map_err(|e| publication_error(e, id, &handoff.path, "updated"))?;
            handoff = Self::inspect(g, id).map_err(|mut e| {
                e.details["publication"] = json!("published");
                publication_error(e, id, &handoff.path, "updated")
            })?;
        }
        Ok((handoff, changed))
    }
    /// A bad/missing receiver or graph never establishes completion.
    pub(crate) fn prune(
        g: &CoordinationGuard,
        view: &ResolvedView,
        ids: Option<&[String]>,
    ) -> ExecutionResult<Value> {
        let handoffs = match ids {
            Some(ids) => ids
                .iter()
                .map(|id| Self::inspect(g, id))
                .collect::<ExecutionResult<Vec<_>>>()?,
            None => Self::load(g)?,
        };
        let graph = ItemGraph::from_store(&view.store);
        let mut deleted = Vec::new();
        let mut retained = Vec::new();
        for handoff in &handoffs {
            let mut diagnostics = Vec::new();
            let mut resolved = true;
            for id in handoff.receivers() {
                match view.store.resolve(id) {
                    Ok(_) if graph.is_valid() => {
                        if !graph.evaluate(id).is_ok_and(|e| e.effective_done) {
                            resolved = false;
                        }
                    }
                    Ok(_) => {
                        resolved = false;
                        diagnostics.extend(graph.diagnostics().iter().map(|d|json!({"path":encode_path(&d.path),"message":d.message,"line":d.line})));
                    }
                    Err(e) => {
                        resolved = false;
                        diagnostics.push(json!({"item_id":id,"code":"source_unavailable","message":format!("receiver lookup: {e:?}")}));
                    }
                }
            }
            if resolved {
                if let Err(e) = view
                    .recheck(g)
                    .and_then(|_| g.delete(&handoff.relative, &handoff.source))
                {
                    let mut e = publication_error(e, handoff.id(), &handoff.path, "deleted");
                    let mut records = e.details["partial"]["deleted"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    records.extend(deleted.iter().cloned());
                    e.details["partial"]["deleted"] = json!(records);
                    e.details["remaining_handoffs"] = json!(
                        handoffs
                            .iter()
                            .filter(|h| !deleted.iter().any(|v: &Value| v["id"] == h.id()))
                            .map(|h| h.to_json())
                            .collect::<Vec<_>>()
                    );
                    return Err(e);
                }
                deleted.push(json!({"id":handoff.id(),"path":encode_path(&handoff.path)}));
            } else {
                retained.push(json!({"id":handoff.id(),"path":encode_path(&handoff.path),"diagnostics":diagnostics}));
            }
        }
        Ok(json!({"changed":!deleted.is_empty(),"deleted":deleted,"retained":retained}))
    }
}
fn encode(header: &Value, body: &str) -> Vec<u8> {
    let mut raw = b"---\n".to_vec();
    raw.extend(yaml_bytes(header));
    raw.extend(b"---\n");
    raw.extend(body.as_bytes());
    raw
}
fn decode(
    g: &CoordinationGuard,
    raw: &[u8],
    id: &str,
    path: &Path,
) -> ExecutionResult<(Value, String)> {
    let text = std::str::from_utf8(raw)
        .map_err(|_| ExecutionError::new("invalid_format", "handoff must be UTF-8"))?;
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(|s| s.trim_end_matches(['\r', '\n'])) != Some("---") {
        return Err(ExecutionError::new(
            "invalid_format",
            "missing handoff frontmatter",
        ));
    }
    let start = text.find('\n').ok_or_else(|| {
        ExecutionError::new("invalid_format", "missing closing handoff delimiter")
    })? + 1;
    let mut offset = start;
    let mut close = None;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            close = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let (end, body) = close.ok_or_else(|| {
        ExecutionError::new("invalid_format", "missing closing handoff delimiter")
    })?;
    let header = super::storage::format::yaml(&raw[start..end], path)?;
    let version = header["format_version"].as_u64().ok_or_else(|| {
        ExecutionError::new("invalid_format", "format_version must be an integer")
    })?;
    if version != 1 {
        return Err(ExecutionError::new(
            "unsupported_format",
            format!("unsupported handoff format {version}"),
        ));
    }
    exact_keys(
        &header,
        &[
            "format_version",
            "store_id",
            "recovery_generation",
            "id",
            "from_items",
            "to_items",
            "created_at",
        ],
        &["session", "workspace_id"],
    )?;
    for key in ["id", "store_id", "recovery_generation"] {
        if !valid_id(&string(&header, key)?) {
            return Err(ExecutionError::new(
                "invalid_format",
                format!("invalid {key}"),
            ));
        }
    }
    if header["id"] != id
        || header["store_id"] != g.metadata.store_id
        || header["recovery_generation"] != g.metadata.recovery_generation
    {
        return Err(ExecutionError::new(
            "identity_mismatch",
            "handoff identity/generation mismatch",
        ));
    }
    for key in ["from_items", "to_items"] {
        let values =
            inputs(&header, key).map_err(|e| ExecutionError::new("invalid_format", e.message))?;
        if values.iter().any(|id| !valid_id(id))
            || values.iter().collect::<BTreeSet<_>>().len() != values.len()
        {
            return Err(ExecutionError::new(
                "invalid_format",
                format!("{key} must contain unique full IDs"),
            ));
        }
    }
    if !super::claims::valid_timestamp(&string(&header, "created_at")?) {
        return Err(ExecutionError::new(
            "invalid_format",
            "created_at must be RFC3339",
        ));
    }
    if let Some(s) = header.get("session") {
        SessionIdentity::from_json(s)
            .map_err(|e| ExecutionError::new("invalid_format", e.message))?;
    }
    if header.get("workspace_id").is_some() && !valid_id(&string(&header, "workspace_id")?) {
        return Err(ExecutionError::new(
            "invalid_format",
            "invalid workspace_id",
        ));
    }
    Ok((header, text[body..].to_owned()))
}
#[cfg(test)]
mod tests;

pub(crate) fn publication_error(
    mut e: ExecutionError,
    id: &str,
    path: &Path,
    bucket: &str,
) -> ExecutionError {
    let mut partial =
        e.details.get("partial").cloned().unwrap_or_else(
            || json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]}),
        );
    let record = json!({"id":id,"path":encode_path(path)});
    match e.details["publication"].as_str() {
        Some("published") => {
            let mut records = partial[bucket].as_array().cloned().unwrap_or_default();
            if !records.contains(&record) {
                records.push(record);
            }
            partial[bucket] = json!(records);
        }
        Some("possible") => {
            let mut paths = partial["uncertain_paths"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            paths.push(json!(encode_path(path)));
            partial["uncertain_paths"] = json!(paths);
        }
        _ => {}
    }
    e.details["partial"] = partial;
    e.details["handoff_id"] = json!(id);
    e
}

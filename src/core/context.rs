//! Material source locations, resolved without copying item state.
//! @mara implements DES-EXECUTION-IO
//! @mara implements DES-CONTEXT-API
use super::coordination::*;
use super::items::{Diagnostic, ItemFile, ItemStore};
use super::project::{Project, discover};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub path: PathBuf,
    pub state: String,
    pub value: Value,
}
#[derive(Debug, Clone)]
pub struct MaterialBinding {
    pub item_id: String,
    pub workspace_id: String,
}
pub struct ContextStore {
    pub workspaces: BTreeMap<String, Workspace>,
    pub bindings: BTreeMap<String, MaterialBinding>,
}
fn envelope(v: &Value, g: &CoordinationGuard) -> ExecutionResult<()> {
    match v.get("format_version").and_then(Value::as_u64) {
        Some(1) => {}
        Some(n) if n > 1 => {
            return Err(ExecutionError::new(
                "unsupported_format",
                "unsupported execution format",
            ));
        }
        _ => {
            return Err(ExecutionError::new(
                "invalid_format",
                "format_version must be 1",
            ));
        }
    };
    if v.get("store_id").and_then(Value::as_str) != Some(&g.metadata.store_id)
        || v.get("recovery_generation").and_then(Value::as_str)
            != Some(&g.metadata.recovery_generation)
    {
        return Err(ExecutionError::new(
            "identity_mismatch",
            "entity belongs to another store or generation",
        ));
    }
    Ok(())
}
impl ContextStore {
    pub fn load(g: &CoordinationGuard) -> ExecutionResult<Self> {
        let mut workspaces = BTreeMap::new();
        for name in g.names(Path::new("workspaces"))? {
            if name == "items" {
                continue;
            }
            let id = name
                .strip_suffix(".yaml")
                .filter(|s| valid_id(s))
                .ok_or_else(|| {
                    ExecutionError::new("invalid_format", "unexpected workspace entry")
                        .at(g.root_path().join("workspaces").join(&name))
                })?;
            let path = Path::new("workspaces").join(&name);
            let v = parse_yaml(&g.read(&path)?.raw)?;
            envelope(&v, g)?;
            exact_keys(
                &v,
                &[
                    "format_version",
                    "store_id",
                    "recovery_generation",
                    "id",
                    "path",
                    "state",
                    "created_at",
                ],
                &["branch", "commit", "cleanup"],
            )?;
            if string(&v, "id")? != id {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "workspace ID does not match filename",
                ));
            }
            let state = string(&v, "state")?;
            if !matches!(state.as_str(), "open" | "closing") {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "invalid workspace state",
                ));
            }
            for key in ["created_at", "branch", "commit"] {
                if v.get(key).is_some() {
                    string(&v, key)?;
                }
            }
            let path = decode_path(&string(&v, "path")?)?;
            if workspaces.values().any(|w: &Workspace| w.path == path) {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "duplicate workspace path",
                ));
            }
            workspaces.insert(
                id.to_owned(),
                Workspace {
                    id: id.to_owned(),
                    path,
                    state,
                    value: v,
                },
            );
        }
        let names = match g.names(Path::new("workspaces/items")) {
            Ok(v) => v,
            Err(e) if e.code == "storage_missing" => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut bindings = BTreeMap::new();
        for name in names {
            let id = name
                .strip_suffix(".yaml")
                .filter(|s| valid_id(s))
                .ok_or_else(|| ExecutionError::new("invalid_format", "unexpected binding entry"))?;
            let v = parse_yaml(&g.read(&Path::new("workspaces/items").join(&name))?.raw)?;
            envelope(&v, g)?;
            exact_keys(
                &v,
                &[
                    "format_version",
                    "store_id",
                    "recovery_generation",
                    "item_id",
                    "workspace_id",
                ],
                &[],
            )?;
            let wid = string(&v, "workspace_id")?;
            if string(&v, "item_id")? != id || !valid_id(&wid) || !workspaces.contains_key(&wid) {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "invalid material workspace binding",
                ));
            }
            bindings.insert(
                id.into(),
                MaterialBinding {
                    item_id: id.into(),
                    workspace_id: wid,
                },
            );
        }
        Ok(Self {
            workspaces,
            bindings,
        })
    }
    pub fn register(g: &CoordinationGuard, path: &Path) -> ExecutionResult<Workspace> {
        let p = discover(Some(path))
            .map_err(|e| ExecutionError::new("source_unavailable", e.to_string()).at(path))?;
        if p.git_common_dir != g.project().git_common_dir {
            return Err(ExecutionError::new(
                "invalid_argument",
                "workspace belongs to another repository",
            ));
        }
        let current = Self::load(g)?;
        if let Some(w) = current
            .workspaces
            .values()
            .find(|w| w.path == p.worktree_root)
        {
            if w.state != "open" {
                return Err(ExecutionError::new(
                    "workspace_busy",
                    "workspace is closing",
                ));
            }
            return Ok(w.clone());
        }
        let id = new_id()?;
        let value = json!({"format_version":1,"store_id":g.metadata.store_id,"recovery_generation":g.metadata.recovery_generation,"id":id,"path":encode_path(&p.worktree_root),"state":"open","created_at":now_timestamp()});
        create_context(
            g,
            &Path::new("workspaces").join(format!("{id}.yaml")),
            &id,
            &yaml_bytes(&value),
        )?;
        Ok(Workspace {
            id,
            path: p.worktree_root,
            state: "open".into(),
            value,
        })
    }
    /// Main calls this only after graph/source/ownership checks (new IDs are also allowed).
    pub(crate) fn bind(
        g: &CoordinationGuard,
        id: &str,
        workspace: &Workspace,
    ) -> ExecutionResult<()> {
        if !valid_id(id) || workspace.state != "open" {
            return Err(ExecutionError::new(
                "invalid_argument",
                "invalid binding destination",
            ));
        }
        g.ensure_dir(Path::new("workspaces/items"))?;
        let path = Path::new("workspaces/items").join(format!("{id}.yaml"));
        let v = json!({"format_version":1,"store_id":g.metadata.store_id,"recovery_generation":g.metadata.recovery_generation,"item_id":id,"workspace_id":workspace.id});
        match g.optional(&path)? {
            None => create_context(g, &path, id, &yaml_bytes(&v)),
            Some(old) => {
                if parse_yaml(&old.raw)? == v {
                    Ok(())
                } else {
                    Err(ExecutionError::new(
                        "conflict",
                        "material item already has another binding",
                    ))
                }
            }
        }
    }
}
// Report the bounded setup write when publication itself fails. Successful
// setup entities are accumulated by the coordinator; no receipt is persisted.
fn create_context(g: &CoordinationGuard, path: &Path, id: &str, raw: &[u8]) -> ExecutionResult<()> {
    g.create(path, raw).map_err(|mut error| {
        let path = encode_path(&g.root_path().join(path));
        let created = if error.details["publication"] == "published" {
            vec![json!({"id":id,"path":path})]
        } else {
            Vec::new()
        };
        let uncertain = if error.details["publication"] == "possible" {
            vec![path]
        } else {
            Vec::new()
        };
        error.details["partial"] =
            json!({"created":created,"updated":[],"deleted":[],"uncertain_paths":uncertain});
        error
    })
}
pub struct ResolvedView {
    pub project: Project,
    pub store: ItemStore,
    pub context: ContextStore,
    pub sources: BTreeMap<String, PathBuf>,
    pub run_ids: BTreeMap<String, String>,
    snapshots: BTreeMap<PathBuf, ItemStore>,
}
impl ResolvedView {
    pub fn load(g: &CoordinationGuard) -> ExecutionResult<Self> {
        let project = g.project().clone();
        let context = ContextStore::load(g)?;
        let selected = ItemStore::load_optional_catalog(&project.worktree_root)?;
        // Remove selected-checkout copies before adding any bound sources.
        // A later binding must never erase an earlier binding's diagnostics.
        let mut files: Vec<_> = selected
            .files
            .iter()
            .filter(|file| {
                !file
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_suffix(".md"))
                    .is_some_and(|id| context.bindings.contains_key(id))
            })
            .cloned()
            .collect();
        let mut snapshots = BTreeMap::from([(project.worktree_root.clone(), selected)]);
        let mut sources = BTreeMap::new();
        for b in context.bindings.values() {
            let w = &context.workspaces[&b.workspace_id];
            let loaded = (|| {
                let p = discover(Some(&w.path)).map_err(|e| {
                    ExecutionError::new("source_unavailable", e.to_string()).at(&w.path)
                })?;
                if p.git_common_dir != project.git_common_dir || p.worktree_root != w.path {
                    return Err(ExecutionError::new(
                        "source_unavailable",
                        "bound workspace repository changed",
                    )
                    .at(&w.path));
                }
                if !snapshots.contains_key(&w.path) {
                    snapshots.insert(w.path.clone(), ItemStore::load(&p)?);
                }
                Ok(())
            })();
            match loaded {
                Ok(()) => {
                    let found: Vec<_> = snapshots[&w.path]
                        .files
                        .iter()
                        .filter(|f| {
                            f.header.as_ref().is_some_and(|h| h.id == b.item_id)
                                || f.path.file_stem().and_then(|s| s.to_str()) == Some(&b.item_id)
                        })
                        .cloned()
                        .collect();
                    if found.is_empty() {
                        files.push(unavailable(
                            w.path.join(format!(".work/items/{}.md", b.item_id)),
                            "bound material source is missing",
                        ));
                    } else {
                        files.extend(found);
                    }
                }
                Err(e) => files.push(unavailable(
                    w.path.join(format!(".work/items/{}.md", b.item_id)),
                    &e.to_string(),
                )),
            }
            sources.insert(b.item_id.clone(), w.path.clone());
        }
        for f in &files {
            if let Some(h) = &f.header {
                sources
                    .entry(h.id.clone())
                    .or_insert_with(|| project.worktree_root.clone());
            }
        }
        let store = ItemStore::from_candidate_files(files);
        Ok(Self {
            project,
            store,
            context,
            sources,
            run_ids: BTreeMap::new(),
            snapshots,
        })
    }
    pub fn recheck(&self) -> ExecutionResult<()> {
        for (root, prior) in &self.snapshots {
            let now = if root == &self.project.worktree_root {
                ItemStore::load_optional_catalog(root)?
            } else {
                ItemStore::load_from_root(root)?
            };
            if prior.files.len() != now.files.len()
                || prior.files.iter().any(|f| {
                    !now.files.iter().any(|n| {
                        n.path == f.path && n.raw == f.raw && n.fingerprint == f.fingerprint
                    })
                })
            {
                return Err(ExecutionError::new(
                    "conflict",
                    "material sources changed after graph validation",
                )
                .at(root));
            }
        }
        Ok(())
    }
}
fn unavailable(path: PathBuf, message: &str) -> ItemFile {
    let mut file = super::items::parse_candidate(path.clone(), Vec::new());
    file.diagnostics = vec![Diagnostic {
        path,
        line: None,
        message: message.into(),
    }];
    file
}

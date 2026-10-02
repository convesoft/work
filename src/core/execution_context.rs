//! Reusable workspace and named-session operations over the existing authority.
//! @mara implements DES-CONTEXT-API
//! @mara implements DES-WORKSPACE-ASSOCIATIONS
use super::claims::ClaimStore;
use super::context::{ContextStore, ResolvedView, Workspace};
use super::coordination::*;
use super::execution::{ExecutionOperations, resolve_item};
use super::graph::ItemGraph;
use super::items::ItemStore;
use super::operations::CheckoutWriter;
use super::runs::{RunPhase, RunStore};
use super::sessions;
use serde_json::{Value, json};
use std::path::Path;

fn full_id(id: &str) -> ExecutionResult<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ExecutionError::new(
            "invalid_argument",
            "entity reference must be a full UUIDv4",
        ))
    }
}
fn workspace<'a>(context: &'a ContextStore, id: &str) -> ExecutionResult<&'a Workspace> {
    full_id(id)?;
    context
        .workspaces
        .get(id)
        .ok_or_else(|| ExecutionError::new("not_found", "workspace is not registered"))
}
fn active_run(runs: &RunStore, id: &str) -> ExecutionResult<()> {
    full_id(id)?;
    if runs.get(id)?.manifest.phase != RunPhase::Active {
        return Err(ExecutionError::new(
            "run_not_current",
            "run is frozen or terminal",
        ));
    }
    Ok(())
}
fn recheck_sessions(g: &CoordinationGuard, run: &super::runs::RunRecord) -> ExecutionResult<()> {
    let id = &run.manifest.id;
    g.recheck(&Path::new("runs").join(id).join("run.yaml"), &run.source)?;
    let dir = sessions::directory(id);
    let expected: Vec<_> = run
        .sessions
        .iter()
        .map(|s| format!("{}.yaml", s.id))
        .collect();
    if g.names(&dir)? != expected {
        return Err(
            ExecutionError::new("conflict", "session entries changed before publication")
                .at(g.root_path().join(&dir)),
        );
    }
    for s in &run.sessions {
        g.recheck(&dir.join(format!("{}.yaml", s.id)), &s.source)?;
    }
    Ok(())
}
fn name_valid(name: &str) -> ExecutionResult<()> {
    if name.is_empty() {
        Err(ExecutionError::new(
            "invalid_argument",
            "session name must be nonempty",
        ))
    } else {
        Ok(())
    }
}
// Publication diagnostics retain the one attempted entity's identity and path.
fn write_error(
    mut e: ExecutionError,
    g: &CoordinationGuard,
    path: &Path,
    id: &str,
    bucket: &str,
) -> ExecutionError {
    let path = encode_path(&g.root_path().join(path));
    let mut partial = json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]});
    match e.details["publication"].as_str() {
        Some("published") => partial[bucket] = json!([{"id":id,"path":path}]),
        Some("possible") => partial["uncertain_paths"] = json!([path]),
        _ => {}
    }
    e.details["partial"] = partial;
    e
}
impl ExecutionOperations {
    pub fn workspace_register(
        &self,
        path: &Path,
        branch: Option<&str>,
        commit: Option<&str>,
    ) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let old = ContextStore::load(&g)?;
        let w = ContextStore::register_observed(&g, path, branch, commit)?;
        Ok(json!({"changed":!old.workspaces.contains_key(&w.id),"workspace":w.value}))
    }
    pub fn workspace_list(&self) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, false)?;
        Ok(
            json!({"workspaces":ContextStore::load(&g)?.workspaces.values().map(|w| &w.value).collect::<Vec<_>>()}),
        )
    }
    pub fn workspace_inspect(&self, id: &str) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, false)?;
        let v = ResolvedView::load(&g)?;
        let w = workspace(&v.context, id)?;
        let graph = ItemGraph::from_store(&v.store);
        let mut users = Vec::new();
        let bindings: Vec<_> = v
            .context
            .bindings
            .values()
            .filter(|b| b.workspace_id == id)
            .map(|b| json!({"item_id":b.item_id,"workspace_id":b.workspace_id}))
            .collect();
        for b in &bindings {
            let item = b["item_id"].as_str().unwrap();
            let (done, diagnostics) = match graph.evaluate(item) {
                Ok(e) if graph.is_valid() => (Some(e.effective_done), json!([])),
                _ => (
                    None,
                    json!([{"code":"source_unavailable","message":"bound item cannot be evaluated in the resolved graph"}]),
                ),
            };
            users.push(
                json!({"kind":"binding","id":item,"effective_done":done,"diagnostics":diagnostics}),
            );
        }
        for run in &v.runs.records {
            if !run.manifest.phase.is_current() {
                continue;
            }
            for (kind, wid) in [
                ("run_default", &run.manifest.default_workspace_id),
                ("run_output", &run.manifest.output_workspace_id),
            ] {
                if wid.as_deref() == Some(id) {
                    users.push(json!({"kind":kind,"id":run.manifest.id}));
                }
            }
            // Current roots and members still use their material source, even
            // when their default points elsewhere or their item is already done.
            for item in
                std::iter::once(&run.manifest.root_item_id).chain(&run.manifest.material_items)
            {
                if v.context
                    .bindings
                    .get(item)
                    .is_some_and(|b| b.workspace_id == id)
                {
                    users.push(json!({"kind":"run_material","id":run.manifest.id,"item_id":item}));
                }
            }
        }
        for c in ClaimStore::list(&g, None, true)? {
            if c.claim.workspace_id.as_deref() == Some(id) {
                users.push(json!({"kind":"claim","id":c.claim.id,"item_id":c.claim.item_id}));
            }
        }
        users.sort_by_key(Value::to_string);
        Ok(json!({"workspace":w.value,"bindings":bindings,"users":users}))
    }
    pub fn workspace_bind(&self, input: &str, wid: &str) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let context = ContextStore::load(&g)?;
        let w = workspace(&context, wid)?.clone();
        let workspace_source = context.workspace_sources[wid].clone();
        if w.state != "open" {
            return Err(ExecutionError::new(
                "workspace_busy",
                "workspace is closing",
            ));
        }
        let p = super::project::discover(Some(&w.path))
            .map_err(|e| ExecutionError::new("source_unavailable", e.to_string()).at(&w.path))?;
        if p.git_common_dir != self.project.git_common_dir || p.worktree_root != w.path {
            return Err(ExecutionError::new(
                "source_unavailable",
                "workspace repository identity changed",
            ));
        }
        // A full ID permits repairing an unavailable old binding; resolve
        // abbreviations through the current view rather than silently retargeting.
        let id = if valid_id(input) {
            input.to_owned()
        } else {
            let current = ResolvedView::load(&g)?;
            resolve_item(&current.store, input)?
                .header
                .as_ref()
                .unwrap()
                .id
                .clone()
        };
        let path = Path::new("workspaces/items").join(format!("{id}.yaml"));
        let old = context.binding_sources.get(&id).cloned();
        let target = ItemStore::load(&p)?;
        let file = resolve_item(&target, &id)?;
        if !file.is_valid() {
            return Err(ExecutionError::new(
                "invalid_source",
                "destination item is invalid",
            ));
        }
        if ClaimStore::list(&g, Some(&id), true)?
            .iter()
            .any(|c| c.current)
        {
            return Err(ExecutionError::new(
                "claim_conflict",
                "active claim prevents binding changes",
            ));
        }
        let changed = context
            .bindings
            .get(&id)
            .is_none_or(|b| b.workspace_id != wid);
        // Binding repair is a location change, not a graph-dependent item
        // mutation. Requiring every old source to resolve would deadlock when
        // several current-run items lost the same checkout: neither could be
        // rebound first, and membership would prevent unbinding them.
        let runs = RunStore::load(&g)?;
        if runs
            .records
            .iter()
            .flat_map(|r| &r.wisps)
            .any(|f| f.header.as_ref().is_some_and(|h| h.id == id))
        {
            return Err(ExecutionError::new(
                "invalid_argument",
                "a wisp cannot have a material binding",
            ));
        }
        let lock = CheckoutWriter::open(&w.path)?;
        let current = ItemStore::load(&p)?;
        let current_file = resolve_item(&current, &id)?;
        if current_file.path != file.path
            || current_file.raw != file.raw
            || current_file.fingerprint != file.fingerprint
        {
            return Err(ExecutionError::new(
                "conflict",
                "binding destination changed during validation",
            )
            .at(&file.path));
        }
        lock.verify()?;
        g.recheck(
            &Path::new("workspaces").join(format!("{wid}.yaml")),
            &workspace_source,
        )?;
        if let Some(old) = &old {
            g.recheck(&path, old)?;
        }
        let value = json!({"format_version":1,"store_id":g.metadata.store_id,"recovery_generation":g.metadata.recovery_generation,"item_id":id,"workspace_id":wid});
        if changed {
            g.ensure_dir(Path::new("workspaces/items"))?;
            let result = match &old {
                Some(old) => g.replace(&path, old, &yaml_bytes(&value)),
                None => g.create(&path, &yaml_bytes(&value)),
            };
            result.map_err(|e| {
                write_error(
                    e,
                    &g,
                    &path,
                    &id,
                    if old.is_some() { "updated" } else { "created" },
                )
            })?;
        }
        Ok(json!({"binding":value,"changed":changed}))
    }
    pub fn workspace_unbind(&self, input: &str) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let context = ContextStore::load(&g)?;
        let id = if valid_id(input) {
            input.to_owned()
        } else {
            let v = ResolvedView::load(&g)?;
            resolve_item(&v.store, input)?
                .header
                .as_ref()
                .unwrap()
                .id
                .clone()
        };
        if !context.bindings.contains_key(&id) {
            let current = ResolvedView::load(&g)?;
            resolve_item(&current.store, &id)?;
        }
        if !ClaimStore::list(&g, Some(&id), true)?.is_empty() {
            return Err(ExecutionError::new(
                "claim_conflict",
                "active claim prevents unbinding",
            ));
        }
        let runs = RunStore::load(&g)?;
        if runs.membership(&id).is_some() || runs.current_for_root(&id).is_some() {
            return Err(ExecutionError::new(
                "run_conflict",
                "current run context prevents unbinding",
            ));
        }
        let path = Path::new("workspaces/items").join(format!("{id}.yaml"));
        let old = context.binding_sources.get(&id);
        if let Some(old) = old {
            g.delete(&path, old)
                .map_err(|e| write_error(e, &g, &path, &id, "deleted"))?;
        }
        Ok(json!({"item_id":id,"changed":old.is_some()}))
    }
    pub fn session_list(&self, run: &str) -> ExecutionResult<Value> {
        full_id(run)?;
        let g = CoordinationGuard::acquire(&self.project, false)?;
        let runs = RunStore::load(&g)?;
        Ok(json!({"sessions":runs.get(run)?.sessions.iter().map(|r| &r.value).collect::<Vec<_>>()}))
    }
    pub fn session_set(
        &self,
        run: &str,
        name: &str,
        session: &SessionIdentity,
        availability: Option<&Value>,
    ) -> ExecutionResult<Value> {
        name_valid(name)?;
        session.validate()?;
        if let Some(a) = availability {
            sessions::validate_availability(a)?;
        }
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let runs = RunStore::load(&g)?;
        active_run(&runs, run)?;
        let records = &runs.get(run)?.sessions;
        let old = records.iter().find(|r| r.name == name);
        let id = match old {
            Some(r) => r.id.clone(),
            None => new_id()?,
        };
        let value = sessions::value(&g, run, &id, name, session, availability);
        let changed = old.is_none_or(|r| r.value != value);
        let path = sessions::directory(run).join(format!("{id}.yaml"));
        recheck_sessions(&g, runs.get(run)?)?;
        if changed {
            let result = match old {
                Some(r) => g.replace(&path, &r.source, &yaml_bytes(&value)),
                None => g.create(&path, &yaml_bytes(&value)),
            };
            result.map_err(|e| {
                write_error(
                    e,
                    &g,
                    &path,
                    &id,
                    if old.is_some() { "updated" } else { "created" },
                )
            })?;
        }
        Ok(json!({"session_record":value,"changed":changed}))
    }
    pub fn session_remove(&self, run: &str, name: &str) -> ExecutionResult<Value> {
        name_valid(name)?;
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let runs = RunStore::load(&g)?;
        active_run(&runs, run)?;
        let old = runs.get(run)?.sessions.iter().find(|r| r.name == name);
        recheck_sessions(&g, runs.get(run)?)?;
        if let Some(r) = old {
            let path = sessions::directory(run).join(format!("{}.yaml", r.id));
            g.delete(&path, &r.source)
                .map_err(|e| write_error(e, &g, &path, &r.id, "deleted"))?;
        }
        Ok(json!({"changed":old.is_some()}))
    }
}

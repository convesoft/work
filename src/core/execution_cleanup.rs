//! Explicit cleanup of externally removed workspaces. No Git/filesystem removal.
//! @mara implements REQ-WORKSPACE-CLEANUP
//! @mara implements DES-CONTEXT-API
//! @mara implements DES-FILE-COORDINATION
#[cfg(test)]
mod tests;

use super::claims::ClaimStore;
use super::context::{ResolvedView, Workspace};
use super::coordination::*;
use super::execution::{ExecutionOperations, require_valid, resolve_item};
use super::execution_context::{full_id, workspace, write_error};
use super::graph::ItemGraph;
use super::operations::CheckoutWriter;
use super::project::discover;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn path(id: &str) -> PathBuf {
    Path::new("workspaces").join(format!("{id}.yaml"))
}
fn checkout(g: &CoordinationGuard, w: &Workspace) -> ExecutionResult<()> {
    let p = discover(Some(&w.path))
        .map_err(|e| ExecutionError::new("source_unavailable", e.to_string()).at(&w.path))?;
    if p.worktree_root != w.path || p.git_common_dir != g.project().git_common_dir {
        return Err(ExecutionError::new(
            "source_unavailable",
            "workspace repository identity changed",
        )
        .at(&w.path));
    }
    Ok(())
}
fn absent(w: &Workspace) -> ExecutionResult<()> {
    // exists() suppresses permission/IO errors and follows dangling symlinks.
    match std::fs::symlink_metadata(&w.path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ExecutionError::from(e).at(&w.path)),
        Ok(_) => Err(
            ExecutionError::new("workspace_busy", "external target path still exists").at(&w.path),
        ),
    }
}
fn closing(w: &Workspace) -> ExecutionResult<&Value> {
    if w.state != "closing" || w.value.get("cleanup").is_none() {
        return Err(ExecutionError::new(
            "workspace_busy",
            "workspace has no active cleanup",
        ));
    }
    Ok(&w.value["cleanup"])
}

// Narrow source preconditions for cleanup. Runs/material snapshots are owned by
// ResolvedView; capture claim sources before parsing ownership so a raw editor
// cannot replace a claim between the check and workspace publication.
struct CleanupClaims {
    names: Vec<String>,
    sources: Vec<(PathBuf, EntitySource)>,
}
impl CleanupClaims {
    fn capture(g: &CoordinationGuard) -> ExecutionResult<Self> {
        let names = g.names(Path::new("claims"))?;
        let sources = names
            .iter()
            .map(|name| {
                let path = Path::new("claims").join(name);
                g.read(&path).map(|source| (path, source))
            })
            .collect::<ExecutionResult<_>>()?;
        Ok(Self { names, sources })
    }
    fn recheck(&self, g: &CoordinationGuard) -> ExecutionResult<()> {
        if g.names(Path::new("claims"))? != self.names {
            return Err(ExecutionError::new(
                "conflict",
                "claim entries changed during cleanup validation",
            ));
        }
        for (path, source) in &self.sources {
            g.recheck(path, source)?;
        }
        Ok(())
    }
}
fn recheck(g: &CoordinationGuard, v: &ResolvedView, claims: &CleanupClaims) -> ExecutionResult<()> {
    v.recheck(g)?;
    claims.recheck(g)?;
    let mut expected: Vec<_> = v
        .context
        .workspaces
        .keys()
        .map(|id| format!("{id}.yaml"))
        .collect();
    let bindings = match g.names(Path::new("workspaces/items")) {
        Ok(names) => {
            expected.push("items".into());
            names
        }
        Err(e) if e.code == "storage_missing" => Vec::new(),
        Err(e) => return Err(e),
    };
    expected.sort();
    if g.names(Path::new("workspaces"))? != expected
        || bindings
            != v.context
                .bindings
                .keys()
                .map(|id| format!("{id}.yaml"))
                .collect::<Vec<_>>()
    {
        return Err(ExecutionError::new(
            "conflict",
            "workspace entries changed during cleanup validation",
        ));
    }
    for (id, source) in &v.context.workspace_sources {
        g.recheck(&path(id), source)?;
    }
    for (id, source) in &v.context.binding_sources {
        g.recheck(
            &Path::new("workspaces/items").join(format!("{id}.yaml")),
            source,
        )?;
    }
    let handoffs = v.handoffs.as_ref().map_err(Clone::clone)?;
    if g.names(Path::new("handoffs"))?
        != handoffs
            .iter()
            .map(|h| format!("{}.md", h.id()))
            .collect::<Vec<_>>()
    {
        return Err(ExecutionError::new(
            "conflict",
            "handoff entries changed during cleanup validation",
        ));
    }
    for h in handoffs {
        g.recheck(&h.relative, &h.source)?;
    }
    g.verify()
}
fn locks(v: &ResolvedView) -> ExecutionResult<Vec<CheckoutWriter>> {
    // Shared lock first, then sorted unique authoritative material checkouts.
    v.sources
        .values()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|root| CheckoutWriter::open(root).map_err(Into::into))
        .collect()
}
fn no_users(g: &CoordinationGuard, v: &ResolvedView, target: &Workspace) -> ExecutionResult<()> {
    require_valid(&v.store)?;
    v.ownership.as_ref().map_err(Clone::clone)?.exclusion()?;
    v.handoffs.as_ref().map_err(Clone::clone)?;
    let mut blockers = Vec::new();
    for b in v
        .context
        .bindings
        .values()
        .filter(|b| b.workspace_id == target.id)
    {
        blockers.push(json!({"kind":"binding","id":b.item_id}));
    }
    for c in ClaimStore::list(g, None, true)? {
        if c.claim.workspace_id.as_deref() == Some(&target.id) {
            blockers.push(json!({"kind":"claim","id":c.claim.id,"item_id":c.claim.item_id}));
        }
    }
    for r in &v.runs.records {
        if !r.manifest.phase.is_current() {
            continue;
        }
        for (kind, wid) in [
            ("run_default", &r.manifest.default_workspace_id),
            ("run_output", &r.manifest.output_workspace_id),
        ] {
            if wid.as_deref() == Some(&target.id) {
                blockers.push(json!({"kind":kind,"id":r.manifest.id}));
            }
        }
        for item in std::iter::once(&r.manifest.root_item_id).chain(&r.manifest.material_items) {
            if v.sources.get(item) == Some(&target.path) {
                blockers.push(json!({"kind":"run_material","id":r.manifest.id,"item_id":item}));
            }
        }
    }
    for w in v.context.workspaces.values() {
        // Distinct Git worktree IDs do not mean physical independence. An
        // ancestor removal would also erase a nested registered checkout.
        if w.id != target.id && w.path.starts_with(&target.path) {
            blockers.push(json!({"kind":"nested_workspace","id":w.id,"path":encode_path(&w.path)}));
        }
        if w.state == "closing"
            && w.value["cleanup"]["controller_workspace_id"].as_str() == Some(&target.id)
        {
            blockers.push(json!({"kind":"cleanup_controller","id":w.id}));
        }
    }
    if g.project().worktree_root.starts_with(&target.path) {
        blockers.push(
            json!({"kind":"selected_checkout","path":encode_path(&g.project().worktree_root)}),
        );
    }
    if g.project().git_common_dir.starts_with(&target.path) {
        blockers
            .push(json!({"kind":"shared_storage","path":encode_path(&g.project().git_common_dir)}));
    }
    if !blockers.is_empty() {
        blockers.sort_by_key(Value::to_string);
        let mut e = ExecutionError::new(
            "workspace_busy",
            "workspace still has users; transfer material sources explicitly before cleanup",
        )
        .at(&target.path);
        e.details["workspace_id"] = json!(target.id);
        e.details["blockers"] = json!(blockers);
        return Err(e);
    }
    Ok(())
}
fn controller<'a>(
    g: &CoordinationGuard,
    v: &'a ResolvedView,
    target: &Workspace,
    id: &str,
) -> ExecutionResult<&'a Workspace> {
    let w = workspace(&v.context, id)?;
    if w.path.starts_with(&target.path) || w.state != "open" {
        return Err(ExecutionError::new(
            "workspace_busy",
            "cleanup requires an open controller outside the target directory",
        ));
    }
    checkout(g, w)?;
    Ok(w)
}
impl ExecutionOperations {
    pub fn workspace_cleanup_begin(
        &self,
        id: &str,
        item: &str,
        controller_id: &str,
    ) -> ExecutionResult<Value> {
        full_id(id)?;
        full_id(controller_id)?;
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let context = super::context::ContextStore::load(&g)?;
        let target = workspace(&context, id)?;
        if target.state == "closing" {
            // Reading a matching persisted operation is not a fresh dispatch.
            // It must work even if a retained binding now has missing bytes.
            let item_id = if valid_id(item) {
                item.to_owned()
            } else {
                let view = ResolvedView::load(&g)?;
                resolve_item(&view.store, item)?
                    .header
                    .as_ref()
                    .unwrap()
                    .id
                    .clone()
            };
            let cleanup = closing(target)?;
            if cleanup["item_id"] != item_id || cleanup["controller_workspace_id"] != controller_id
            {
                return Err(ExecutionError::new(
                    "workspace_busy",
                    "workspace is closing for another cleanup",
                ));
            }
            g.recheck(&path(id), &context.workspace_sources[id])?;
            return Ok(json!({"workspace":target.value,"changed":false}));
        }
        let claims = CleanupClaims::capture(&g)?;
        let v = ResolvedView::load(&g)?;
        g.recheck(&path(id), &context.workspace_sources[id])?;
        let target = workspace(&v.context, id)?;
        no_users(&g, &v, target)?;
        let control = controller(&g, &v, target, controller_id)?;
        let item_id = resolve_item(&v.store, item)?
            .header
            .as_ref()
            .unwrap()
            .id
            .clone();
        let evaluation = ItemGraph::from_store(&v.store)
            .evaluate(&item_id)
            .map_err(|e| ExecutionError::new("not_ready", format!("{e:?}")))?;
        if v.runs
            .membership(&item_id)
            .is_some_and(|r| r.manifest.phase != super::runs::RunPhase::Active)
        {
            return Err(ExecutionError::new(
                "run_not_current",
                "cleanup item belongs to a frozen run",
            ));
        }
        if !evaluation.executable {
            let mut e = ExecutionError::new("not_ready", "cleanup item is not executable");
            e.details["item_id"] = json!(item_id);
            return Err(e);
        }
        let execution_workspace = if let Some(source) = v.sources.get(&item_id) {
            Some(source.as_path())
        } else {
            v.runs
                .membership(&item_id)
                .and_then(|r| r.manifest.default_workspace_id.as_ref())
                .and_then(|id| v.context.workspaces.get(id))
                .map(|w| w.path.as_path())
        };
        if execution_workspace != Some(control.path.as_path()) {
            return Err(ExecutionError::new(
                "reference_blocked",
                "cleanup item must execute in the surviving controller checkout",
            ));
        }
        checkout(&g, target)?;
        let held = locks(&v)?;
        for lock in &held {
            lock.verify()?;
        }
        recheck(&g, &v, &claims)?;
        checkout(&g, control)?;
        checkout(&g, target)?;
        let mut value = target.value.clone();
        value["state"] = json!("closing");
        value["cleanup"] = json!({"item_id":item_id,"controller_workspace_id":controller_id,"started_at":now_timestamp()});
        g.replace(
            &path(id),
            &v.context.workspace_sources[id],
            &yaml_bytes(&value),
        )
        .map_err(|e| write_error(e, &g, &path(id), id, "updated"))?;
        // All guards drop on return. External deletion is never called here.
        Ok(json!({"workspace":value,"changed":true}))
    }
    pub fn workspace_cleanup_report(
        &self,
        id: &str,
        removed: bool,
        failure: Option<&str>,
    ) -> ExecutionResult<Value> {
        full_id(id)?;
        if (removed && failure.is_some()) || (!removed && failure.is_none()) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "failure is required only when removed is false",
            ));
        }
        let g = CoordinationGuard::acquire(&self.project, true)?;
        if !removed {
            // Failure must remain recordable even when the external attempt
            // made a run or material source unavailable.
            let context = super::context::ContextStore::load(&g)?;
            let target = workspace(&context, id)?;
            closing(target)?;
            let mut value = target.value.clone();
            value["cleanup"]["failure"] = json!(failure.unwrap());
            let changed = value != target.value;
            if changed {
                g.replace(
                    &path(id),
                    &context.workspace_sources[id],
                    &yaml_bytes(&value),
                )
                .map_err(|e| write_error(e, &g, &path(id), id, "updated"))?;
            }
            return Ok(json!({"workspace":value,"changed":changed}));
        }
        let claims = CleanupClaims::capture(&g)?;
        let v = ResolvedView::load(&g)?;
        let target = workspace(&v.context, id)?;
        let context = closing(target)?;
        {
            no_users(&g, &v, target)?;
            let control = controller(
                &g,
                &v,
                target,
                context["controller_workspace_id"].as_str().unwrap(),
            )?;
            absent(target)?;
            let held = locks(&v)?;
            for lock in &held {
                lock.verify()?;
            }
            recheck(&g, &v, &claims)?;
            checkout(&g, control)?;
            absent(target)?;
            g.delete(&path(id), &v.context.workspace_sources[id])
                .map_err(|e| write_error(e, &g, &path(id), id, "deleted"))?;
            Ok(json!({"workspace_id":id,"removed":true,"changed":true}))
        }
    }
    pub fn workspace_cleanup_cancel(&self, id: &str) -> ExecutionResult<Value> {
        full_id(id)?;
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let context = super::context::ContextStore::load(&g)?;
        let target = workspace(&context, id)?;
        closing(target)?;
        checkout(&g, target)?;
        let mut value = target.value.clone();
        value["state"] = json!("open");
        value.as_object_mut().unwrap().remove("cleanup");
        checkout(&g, target)?;
        g.replace(
            &path(id),
            &context.workspace_sources[id],
            &yaml_bytes(&value),
        )
        .map_err(|e| write_error(e, &g, &path(id), id, "updated"))?;
        Ok(json!({"workspace":value,"changed":true}))
    }
}

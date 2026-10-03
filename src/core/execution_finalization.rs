//! Bounded retain-before-delete run finalization.
//! @mara implements DES-FINALIZATION-API
use super::claims::ClaimStore;
use super::context::ResolvedView;
use super::coordination::*;
use super::execution::{ExecutionOperations, require_valid, resolve_item};
use super::graph::ItemGraph;
use super::handoffs::HandoffStore;
use super::items::ItemStore;
use super::operations::CheckoutWriter;
use super::runs::{CleanupKind, RunCleanup, RunPhase, RunSnapshot};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn manifest_path(run: &str) -> PathBuf {
    PathBuf::from(format!("runs/{run}/run.yaml"))
}
fn conflict() -> ExecutionError {
    ExecutionError::new(
        "run_conflict",
        "request does not match recorded run cleanup",
    )
}
fn check_targets(
    g: &CoordinationGuard,
    v: &ResolvedView,
    run: &str,
    cleanup: &RunCleanup,
) -> ExecutionResult<()> {
    let r = v.runs.get(run)?;
    if r.manifest
        .cleanup
        .as_ref()
        .is_some_and(|recorded| recorded != cleanup)
    {
        return Err(conflict());
    }
    let targets: BTreeSet<_> = cleanup.item_ids.iter().cloned().collect();
    // Recorded missing IDs are already deleted, never material or another run's wisps.
    for id in &targets {
        if v.sources.contains_key(id) || v.run_ids.get(id).is_some_and(|owner| owner != run) {
            return Err(conflict());
        }
    }
    if cleanup.finalize
        && (r.wisp_sources.keys().any(|id| !targets.contains(id))
            || r.sessions
                .iter()
                .any(|s| !cleanup.session_ids.contains(&s.id)))
    {
        return Err(conflict());
    }
    let claims = ClaimStore::list(g, None, true)?;
    let blockers: Vec<_> = claims
        .iter()
        .filter(|c| {
            targets.contains(&c.claim.item_id)
                || cleanup.finalize
                    && (c.claim.run_id.as_deref() == Some(run)
                        || c.claim.item_id == r.manifest.root_item_id
                        || r.manifest.material_items.contains(&c.claim.item_id))
        })
        .map(|c| json!({"kind":"claim","id":c.claim.id,"item_id":c.claim.item_id}))
        .collect();
    if !blockers.is_empty() {
        let mut e = ExecutionError::new("claim_conflict", "active claims prevent run cleanup");
        e.details["blockers"] = json!(blockers);
        return Err(e);
    }
    let mut blockers = Vec::new();
    for f in &v.store.files {
        if let Some(h) = &f.header {
            if targets.contains(&h.id) {
                // Related is symmetric even when the target owns its assertion.
                for outside in h.related.iter().filter(|id| !targets.contains(*id)) {
                    blockers.push(json!({"kind":"item","id":outside,"target":h.id,"relation":"related","path":encode_path(&f.path)}));
                }
                continue;
            }
            for target in h
                .parent
                .iter()
                .chain(&h.depends_on)
                .chain(&h.related)
                .chain(&h.discovered_from)
            {
                if targets.contains(target) {
                    blockers.push(json!({"kind":"item","id":h.id,"target":target,"path":encode_path(&f.path)}));
                }
            }
        }
    }
    // Ignore the recorded deletion set, including internal dangling edges
    // left by an interrupted cleanup, when evaluating outside receivers.
    let surviving = ItemStore::from_candidate_files(
        v.store
            .files
            .iter()
            .filter(|f| !f.header.as_ref().is_some_and(|h| targets.contains(&h.id)))
            .cloned()
            .collect(),
    );
    validate_squash_edits_except(v, &surviving, Some(run))?;
    let graph = ItemGraph::from_store(&surviving);
    for h in v.handoffs.as_ref().map_err(Clone::clone)? {
        let references_target = ["from_items", "to_items"].iter().any(|k| {
            h.header[*k]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| targets.contains(id.as_str().unwrap()))
        });
        let needed_outside = h.receivers().any(|id| {
            !targets.contains(id) && graph.evaluate(id).map_or(true, |e| !e.effective_done)
        });
        if references_target && needed_outside {
            blockers.push(json!({"kind":"handoff","id":h.id(),"path":encode_path(&h.path)}));
        }
        g.recheck(&h.relative, &h.source)?;
    }
    if !blockers.is_empty() {
        let mut e = ExecutionError::new(
            "reference_blocked",
            "surviving references prevent run cleanup",
        );
        e.details["blockers"] = json!(blockers);
        return Err(e);
    }
    require_valid(&surviving)?;
    Ok(())
}
fn pending_item(v: &ResolvedView, cleanup: &RunCleanup, input: &str) -> ExecutionResult<String> {
    if valid_id(input) {
        return Ok(input.into());
    }
    let prefix = input.strip_prefix("w-").unwrap_or(input);
    if prefix.is_empty()
        || prefix.len() > 32
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || prefix.len() == 32 && !valid_id(prefix)
    {
        return resolve_item(&v.store, input).map(|f| f.header.as_ref().unwrap().id.clone());
    }
    let candidates: BTreeSet<_> = cleanup
        .item_ids
        .iter()
        .chain(
            v.store
                .files
                .iter()
                .filter_map(|f| f.header.as_ref().map(|h| &h.id)),
        )
        .filter(|id| id.starts_with(prefix))
        .collect();
    match candidates.len() {
        0 => Err(ExecutionError::new(
            "not_found",
            format!("item {input} was not found"),
        )),
        1 => Ok((*candidates.first().unwrap()).clone()),
        _ => Err(ExecutionError::new(
            "ambiguous_id",
            format!(
                "item {input} matches {}",
                candidates
                    .into_iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}
pub(crate) fn require_squash_mutable(
    runs: &super::runs::RunStore,
    id: &str,
) -> ExecutionResult<()> {
    if let Some(run) = runs
        .records
        .iter()
        .find(|r| r.manifest.phase == RunPhase::Squashing && r.members().contains(id))
    {
        let mut error = ExecutionError::new(
            "run_not_current",
            "squash freezes member item mutations and source rebinding",
        );
        error.details =
            json!({"run_id":run.manifest.id,"item_id":id,"publication":"not_published"});
        return Err(error);
    }
    Ok(())
}
/// Validate actual prospective files against the captured squashing graph.
/// This is a mutation guard, not an overlay or reconstructed deleted state.
pub(crate) fn validate_squash_edits(
    v: &ResolvedView,
    candidate: &ItemStore,
) -> ExecutionResult<()> {
    validate_squash_edits_except(v, candidate, None)
}
fn validate_squash_edits_except(
    v: &ResolvedView,
    candidate: &ItemStore,
    deleting_run: Option<&str>,
) -> ExecutionResult<()> {
    use super::items::Completion;
    let frozen: BTreeSet<_> = v
        .runs
        .records
        .iter()
        .filter(|r| {
            r.manifest.phase == RunPhase::Squashing && deleting_run != Some(r.manifest.id.as_str())
        })
        .flat_map(|r| r.members())
        .collect();
    if frozen.is_empty() {
        return Ok(());
    }
    let old: std::collections::BTreeMap<_, _> = v
        .store
        .files
        .iter()
        .filter_map(|f| f.header.as_ref().map(|h| (h.id.as_str(), f)))
        .collect();
    let new: std::collections::BTreeMap<_, _> = candidate
        .files
        .iter()
        .filter_map(|f| f.header.as_ref().map(|h| (h.id.as_str(), f)))
        .collect();
    let affects = |id: &str, files: &std::collections::BTreeMap<&str, &super::items::ItemFile>| {
        let mut at = id.to_owned();
        let mut seen = BTreeSet::new();
        while seen.insert(at.clone()) {
            let Some(parent) = files
                .get(at.as_str())
                .and_then(|f| f.header.as_ref())
                .and_then(|h| h.parent.as_deref())
            else {
                break;
            };
            let Some(header) = files.get(parent).and_then(|f| f.header.as_ref()) else {
                break;
            };
            if header.completion != Completion::Children {
                break;
            }
            if frozen.contains(parent) {
                return true;
            }
            at = parent.to_owned();
        }
        false
    };
    for id in old
        .keys()
        .chain(new.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let before = old.get(id).copied();
        let after = new.get(id).copied();
        let old_h = before.and_then(|f| f.header.as_ref());
        let new_h = after.and_then(|f| f.header.as_ref());
        let changed = old_h != new_h
            || before.and_then(|f| f.body.as_ref()) != after.and_then(|f| f.body.as_ref());
        if changed && frozen.contains(id) {
            require_squash_mutable(&v.runs, id)?;
        }
        let completion_changed = old_h.map(|h| (&h.completion, &h.state, &h.parent))
            != new_h.map(|h| (&h.completion, &h.state, &h.parent));
        if completion_changed && (affects(id, &old) || affects(id, &new)) {
            let mut error = ExecutionError::new(
                "run_not_current",
                "outside change affects a squashing member's derived completion",
            );
            error.details = json!({"item_id":id,"publication":"not_published"});
            return Err(error);
        }
    }
    Ok(())
}
fn output(g: &CoordinationGuard, v: &ResolvedView, run: &str) -> ExecutionResult<PathBuf> {
    let r = v.runs.get(run)?;
    let id =
        r.manifest.output_workspace_id.as_ref().ok_or_else(|| {
            ExecutionError::new("source_unavailable", "run has no output workspace")
        })?;
    let w =
        v.context.workspaces.get(id).ok_or_else(|| {
            ExecutionError::new("source_unavailable", "output workspace is missing")
        })?;
    v.context.require_open_path(&w.path)?;
    let p = super::project::discover(Some(&w.path))
        .map_err(|e| ExecutionError::new("source_unavailable", e.to_string()).at(&w.path))?;
    if p.git_common_dir != g.project().git_common_dir || p.worktree_root != w.path {
        return Err(ExecutionError::new(
            "source_unavailable",
            "output repository changed",
        ));
    }
    if r.manifest.phase.is_current()
        && (v
            .context
            .bindings
            .get(&r.manifest.root_item_id)
            .map(|b| &b.workspace_id)
            != Some(id)
            || v.sources.get(&r.manifest.root_item_id) != Some(&w.path))
    {
        return Err(ExecutionError::new(
            "run_conflict",
            "explicitly bind the root to the captured output workspace before squash",
        ));
    }
    Ok(w.path.clone())
}
impl ExecutionOperations {
    pub fn run_squash(&self, run: &str, summary: &str) -> ExecutionResult<Value> {
        self.finalize_run(run, Some(summary), true, &[])
    }
    pub fn run_discard(&self, run: &str, all: bool, items: &[String]) -> ExecutionResult<Value> {
        if all == !items.is_empty() {
            return Err(ExecutionError::new(
                "invalid_argument",
                "provide exactly all:true or nonempty items",
            ));
        }
        self.finalize_run(run, None, all, items)
    }
    fn finalize_run(
        &self,
        run: &str,
        summary: Option<&str>,
        finalize: bool,
        inputs: &[String],
    ) -> ExecutionResult<Value> {
        if !valid_id(run) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "run ID must be full",
            ));
        }
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let mut v = ResolvedView::load(&g)?;
        let r = v.runs.get(run)?;
        let kind = if summary.is_some() {
            CleanupKind::Squash
        } else {
            CleanupKind::Discard
        };
        let root = r.manifest.root_item_id.clone();
        let mut ids = BTreeSet::new();
        for input in inputs {
            let id = if let Some(cleanup) = &r.manifest.cleanup {
                pending_item(&v, cleanup, input)?
            } else {
                resolve_item(&v.store, input)?
                    .header
                    .as_ref()
                    .unwrap()
                    .id
                    .clone()
            };
            if !ids.insert(id.clone()) {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "duplicate discard item",
                ));
            }
            if r.manifest.phase == RunPhase::Active && !r.wisp_sources.contains_key(&id) {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "selected discard accepts only this run's wisps",
                ));
            }
        }
        let cleanup = if let Some(c) = &r.manifest.cleanup {
            if c.kind != kind
                || c.finalize != finalize
                || !finalize && c.item_ids != ids.iter().cloned().collect::<Vec<_>>()
            {
                return Err(conflict());
            }
            c.clone()
        } else {
            RunCleanup {
                kind,
                finalize,
                item_ids: if finalize {
                    r.wisp_sources.keys().cloned().collect()
                } else {
                    ids.into_iter().collect()
                },
                session_ids: if finalize {
                    r.sessions.iter().map(|s| s.id.clone()).collect()
                } else {
                    Vec::new()
                },
                started_at: now_timestamp(),
            }
        };
        if r.manifest.phase.is_terminal() {
            if kind == CleanupKind::Discard {
                return Ok(json!({"run_id":run,"phase":"disposed","deleted":[],"changed":false}));
            }
            let out = output(&g, &v, run)?;
            let lock = CheckoutWriter::open(&out)?;
            lock.verify()?;
            super::digests::retain(&out, &root, run, summary.unwrap(), true)?;
            lock.verify()?;
            return Ok(
                json!({"run_id":run,"phase":"finalized","digest_path":encode_path(&super::digests::path(&out,&root,run)),"deleted":[],"changed":false}),
            );
        }
        let out = summary.map(|_| output(&g, &v, run)).transpose()?;
        let locks = v
            .sources
            .values()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|p| CheckoutWriter::open(p).map_err(ExecutionError::from))
            .collect::<ExecutionResult<Vec<_>>>()?;
        for lock in &locks {
            lock.verify()?;
        }
        v.recheck(&g)?;
        let mut progress = json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]});
        let mut attempted: Option<(&str, String, PathBuf)> = None;
        let result = (|| {
            let pruned = HandoffStore::prune(&g, &v, None)?;
            progress["deleted"] = pruned["deleted"].clone();
            v = ResolvedView::load(&g)?;
            check_targets(&g, &v, run, &cleanup)?;
            let r = v.runs.get(run)?;
            if kind == CleanupKind::Squash && r.manifest.phase == RunPhase::Active {
                require_valid(&v.store)?;
                let claims = ClaimStore::list(&g, None, true)?
                    .iter()
                    .map(|c| c.claim.item_id.clone())
                    .collect();
                let material = v.sources.keys().cloned().collect();
                if !r.finished(&RunSnapshot {
                    view: &v.store,
                    active_claims: &claims,
                    material_ids: &material,
                })? {
                    return Err(ExecutionError::new(
                        "run_conflict",
                        "squash requires nonempty finished member work",
                    ));
                }
            }
            if r.manifest.phase == RunPhase::Active {
                let mut m = r.manifest.clone();
                m.cleanup = Some(cleanup.clone());
                m.phase = if kind == CleanupKind::Squash {
                    RunPhase::Squashing
                } else {
                    RunPhase::Discarding
                };
                let path = manifest_path(run);
                attempted = Some(("updated", run.into(), g.root_path().join(&path)));
                v.recheck(&g)?;
                for lock in &locks {
                    lock.verify()?;
                }
                g.replace(&path, &r.source, &yaml_bytes(&m.to_json()))?;
                progress["updated"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":run,"path":encode_path(&g.root_path().join(path))}));
                attempted = None;
            }
            if let Some(out) = &out {
                #[cfg(test)]
                super::storage::files::inject(
                    "finalization_before_digest",
                    &super::digests::path(out, &root, run),
                )?;
                attempted = Some(("created", run.into(), super::digests::path(out, &root, run)));
                for lock in &locks {
                    lock.verify()?;
                }
                if super::digests::retain(out, &root, run, summary.unwrap(), false)? {
                    progress["created"].as_array_mut().unwrap().push(
                        json!({"id":run,"path":encode_path(&super::digests::path(out,&root,run))}),
                    );
                }
                attempted = None;
                #[cfg(test)]
                super::storage::files::inject(
                    "finalization_digest_retained",
                    &super::digests::path(out, &root, run),
                )?;
                for lock in &locks {
                    lock.verify()?;
                }
            }
            for (dir, extension, set) in [
                ("items", "md", &cleanup.item_ids),
                ("sessions", "yaml", &cleanup.session_ids),
            ] {
                for id in set {
                    v = ResolvedView::load(&g)?;
                    if v.runs.get(run)?.manifest.cleanup.as_ref() != Some(&cleanup) {
                        return Err(conflict());
                    }
                    check_targets(&g, &v, run, &cleanup)?;
                    v.recheck(&g)?;
                    for lock in &locks {
                        lock.verify()?;
                    }
                    let path = PathBuf::from(format!("runs/{run}/{dir}/{id}.{extension}"));
                    let record = v.runs.get(run)?;
                    let source = if dir == "items" {
                        record.wisp_sources.get(id)
                    } else {
                        record
                            .sessions
                            .iter()
                            .find(|s| &s.id == id)
                            .map(|s| &s.source)
                    };
                    if let Some(source) = source {
                        attempted = Some(("deleted", id.clone(), g.root_path().join(&path)));
                        g.delete(&path, source)?;
                        progress["deleted"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({"id":id,"path":encode_path(&g.root_path().join(&path))}));
                        attempted = None;
                        #[cfg(test)]
                        super::storage::files::inject(
                            "finalization_after_delete",
                            &g.root_path().join(&path),
                        )?;
                    } else {
                        if g.optional(&path)?.is_some() {
                            return Err(ExecutionError::new(
                                "conflict",
                                "recorded target appeared after validation",
                            )
                            .at(g.root_path().join(&path)));
                        }
                        // A prior unlink may have stopped before directory sync.
                        g.sync_dir(path.parent().unwrap())?;
                    }
                }
            }
            v = ResolvedView::load(&g)?;
            if v.runs.get(run)?.manifest.cleanup.as_ref() != Some(&cleanup) {
                return Err(conflict());
            }
            check_targets(&g, &v, run, &cleanup)?;
            v.recheck(&g)?;
            for lock in &locks {
                lock.verify()?;
            }
            let r = v.runs.get(run)?;
            #[cfg(test)]
            super::storage::files::inject(
                "finalization_before_terminal",
                &g.root_path().join(manifest_path(run)),
            )?;
            let mut m = r.manifest.clone();
            m.phase = if !finalize {
                RunPhase::Active
            } else if kind == CleanupKind::Squash {
                RunPhase::Finalized
            } else {
                RunPhase::Disposed
            };
            if finalize {
                m.ended_at = Some(now_timestamp());
            } else {
                m.cleanup = None;
            }
            let path = manifest_path(run);
            attempted = Some(("updated", run.into(), g.root_path().join(&path)));
            g.replace(&path, &r.source, &yaml_bytes(&m.to_json()))?;
            attempted = None;
            let mut result = json!({"run_id":run,"phase":m.phase.as_str(),"deleted":progress["deleted"],"changed":true});
            if let Some(out) = &out {
                result["digest_path"] = json!(encode_path(&super::digests::path(out, &root, run)));
            }
            Ok(result)
        })();
        result.map_err(|mut e: ExecutionError| {
            if let Some((bucket, id, path)) = attempted {
                let record = json!({"id":id,"path":encode_path(&path)});
                if bucket == "deleted" && e.details["deleted_path"] == encode_path(&path) {
                    progress["deleted"]
                        .as_array_mut()
                        .unwrap()
                        .push(record.clone());
                }
                match e.details["publication"].as_str() {
                    Some("published") => progress[bucket].as_array_mut().unwrap().push(record),
                    Some("possible") => progress["uncertain_paths"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(encode_path(&path))),
                    _ => {}
                }
            }
            if let Some(partial) = e.details.get("partial") {
                for bucket in ["created", "updated", "deleted", "uncertain_paths"] {
                    if let Some(records) = partial[bucket].as_array() {
                        progress[bucket]
                            .as_array_mut()
                            .unwrap()
                            .extend(records.iter().cloned());
                    }
                }
            }
            e.details["partial"] = progress;
            e.details["run_id"] = json!(run);
            e.details["remaining_items"] = json!(
                cleanup
                    .item_ids
                    .iter()
                    .filter(|id| g
                        .optional(&PathBuf::from(format!("runs/{run}/items/{id}.md")))
                        .map_or(true, |source| source.is_some()))
                    .collect::<Vec<_>>()
            );
            e.details["remaining_sessions"] = json!(
                cleanup
                    .session_ids
                    .iter()
                    .filter(|id| g
                        .optional(&PathBuf::from(format!("runs/{run}/sessions/{id}.yaml")))
                        .map_or(true, |source| source.is_some()))
                    .collect::<Vec<_>>()
            );
            if let Some(out) = out {
                e.details["digest_path"] =
                    json!(encode_path(&super::digests::path(&out, &root, run)));
            }
            e
        })
    }
}

#[cfg(test)]
mod tests;

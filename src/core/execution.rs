//! Coordinator facade: resolved source files and repository-wide ownership.
//! @mara implements DES-EXECUTION-IO
//! @mara implements DES-CLAIM-API
use super::claims::{ClaimAuthorization, ClaimCandidate, ClaimStore, OwnershipSnapshot};
use super::context::{ContextStore, ResolvedView};
use super::coordination::*;
use super::graph::ItemGraph;
use super::items::{Completion, ItemHeader, ItemStore, ManualState, parse_candidate};
use super::operations::{
    self, CheckoutWriter, DurableOperations, Inspection, MetadataChange, OperationError,
    RawInspection, RelationKind,
};
use super::project::Project;
use super::storage::{Storage, StorageState};
use serde_json::{Value, json};
use std::path::Path;

pub struct ExecutionOperations {
    pub project: Project,
    pub authorization: Vec<ClaimAuthorization>,
    pub checkout_view: bool,
}
impl ExecutionOperations {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            authorization: Vec::new(),
            checkout_view: false,
        }
    }
    fn physical(&self) -> DurableOperations {
        DurableOperations::new(&self.project.worktree_root)
    }
    fn guard(&self, write: bool) -> ExecutionResult<Option<CoordinationGuard>> {
        if self.checkout_view && !write {
            return Ok(None);
        }
        let s = Storage::new(self.project.clone()).inspect()?;
        if !s.coordination_available {
            if !write
                || (s.state == StorageState::Uninitialized
                    && s.storage_warning.is_none()
                    && self.authorization.is_empty())
            {
                return Ok(None);
            }
            return Err(ExecutionError::new(
                "recovery_required",
                "coordination unavailable; storage requires explicit recovery",
            ));
        }
        Ok(Some(CoordinationGuard::acquire(&self.project, write)?))
    }
    pub fn view(&self) -> ExecutionResult<ItemStore> {
        match self.guard(false)? {
            Some(g) => Ok(ResolvedView::load(&g)?.store),
            None => Ok(ItemStore::load(&self.project)?),
        }
    }
    pub fn inspect(&self, id: &str) -> ExecutionResult<Inspection> {
        match self.guard(false)? {
            None => Ok(self.physical().inspect(id)?),
            Some(g) => {
                let v = ResolvedView::load(&g)?;
                inspect(&g, &v, id)
            }
        }
    }
    pub fn list(&self) -> ExecutionResult<Vec<Inspection>> {
        match self.guard(false)? {
            None => Ok(self.physical().list()?),
            Some(g) => {
                let v = ResolvedView::load(&g)?;
                let graph = ItemGraph::from_store(&v.store);
                let ownership = ClaimStore::ownership_snapshot(&g);
                let mut result: Vec<_> = v
                    .store
                    .files
                    .iter()
                    .filter(|f| f.is_valid())
                    .filter_map(|f| f.header.as_ref())
                    .map(|h| inspect_with_snapshot(&v, &graph, &ownership, &h.id))
                    .collect::<ExecutionResult<_>>()?;
                result.sort_by(|a, b| {
                    a.file
                        .header
                        .as_ref()
                        .unwrap()
                        .id
                        .cmp(&b.file.header.as_ref().unwrap().id)
                });
                Ok(result)
            }
        }
    }
    pub fn ready(&self) -> ExecutionResult<Vec<Inspection>> {
        let Some(g) = self.guard(false)? else {
            return Ok(self.physical().ready()?);
        };
        let v = ResolvedView::load(&g)?;
        let graph = ItemGraph::from_store(&v.store);
        if !graph.is_valid() {
            return Err(OperationError::InvalidSource(graph.diagnostics().to_vec()).into());
        }
        let ownership = Ok(ClaimStore::ownership_snapshot(&g)?);
        let mut result = Vec::new();
        for file in &v.store.files {
            if let Some(h) = &file.header {
                let i = inspect_with_snapshot(&v, &graph, &ownership, &h.id)?;
                if i.evaluation.as_ref().is_some_and(|e| e.executable) {
                    result.push(i);
                }
            }
        }
        result.sort_by_key(|i| {
            let h = i.file.header.as_ref().unwrap();
            (h.priority, h.id.clone())
        });
        Ok(result)
    }
    pub fn inspect_raw(&self, id: &str) -> ExecutionResult<RawInspection> {
        Ok(self.physical().inspect_raw(id)?)
    }
    pub fn repair(&self, id: &str, raw: Vec<u8>) -> ExecutionResult<RawInspection> {
        let g = self.guard(true)?;
        if let Some(g) = &g {
            ClaimStore::authorize(g, id, &self.authorization)?;
        }
        Ok(self.physical().repair(id, raw)?)
    }
    pub fn create(
        &self,
        title: String,
        body: Vec<u8>,
        change: MetadataChange,
    ) -> ExecutionResult<Inspection> {
        let Some(g) = self.guard(true)? else {
            return Ok(self.physical().create(title, body, change)?);
        };
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        let h = operations::apply_change(operations::default_header(new_id()?, title), change);
        ClaimStore::authorize(&g, &h.id, &self.authorization)?;
        let path = self
            .project
            .worktree_root
            .join(format!(".work/items/{}.md", h.id));
        validate_candidate(&v.store, &path, &h, &body)?;
        let mut writer = CheckoutWriter::open(&self.project.worktree_root)?;
        v.recheck()?;
        g.verify()?;
        let recovery = writer.publish(&h, &body, None)?;
        let v = ResolvedView::load(&g)?;
        let mut result = inspect(&g, &v, &h.id)?;
        result.recovery_path = recovery;
        Ok(result)
    }
    pub fn update(&self, id: &str, change: MetadataChange) -> ExecutionResult<Inspection> {
        let fallback = change.clone();
        self.mutate(
            id,
            false,
            |h| {
                *h = operations::apply_change(h.clone(), change);
                Ok(())
            },
            |ops| ops.update(id, fallback),
        )
    }
    pub fn close(&self, id: &str, reason: Option<String>) -> ExecutionResult<Inspection> {
        let fallback = reason.clone();
        self.mutate(
            id,
            true,
            |h| {
                if h.completion != Completion::Manual {
                    return Err(ExecutionError::new(
                        "invalid_argument",
                        "aggregate cannot be closed",
                    ));
                }
                h.state = Some(ManualState::Done);
                h.close_reason = reason;
                Ok(())
            },
            |ops| ops.close(id, fallback),
        )
    }
    pub fn reopen(&self, id: &str) -> ExecutionResult<Inspection> {
        self.mutate(
            id,
            false,
            |h| {
                if h.completion != Completion::Manual {
                    return Err(ExecutionError::new(
                        "invalid_argument",
                        "aggregate cannot be reopened",
                    ));
                }
                h.state = Some(ManualState::Open);
                h.close_reason = None;
                Ok(())
            },
            |ops| ops.reopen(id),
        )
    }
    fn mutate(
        &self,
        id: &str,
        complete: bool,
        edit: impl FnOnce(&mut ItemHeader) -> ExecutionResult<()>,
        fallback: impl FnOnce(&DurableOperations) -> Result<Inspection, OperationError>,
    ) -> ExecutionResult<Inspection> {
        let Some(g) = self.guard(true)? else {
            // The original operation loads and applies only its requested edit
            // under the checkout lock, preserving concurrent unrelated changes.
            return Ok(fallback(&self.physical())?);
        };
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        let before = v
            .store
            .resolve(id)
            .map_err(|e| ExecutionError::new("not_found", format!("item lookup: {e:?}")))?
            .clone();
        let mut h = before
            .header
            .clone()
            .ok_or_else(|| ExecutionError::new("invalid_source", "invalid item"))?;
        let claim = ClaimStore::authorize(&g, &h.id, &self.authorization)?;
        edit(&mut h)?;
        let body = before.body.as_deref().unwrap();
        validate_candidate(&v.store, &before.path, &h, body)?;
        let root = v.sources.get(&h.id).ok_or_else(|| {
            ExecutionError::new("source_unavailable", "material source is missing")
        })?;
        let mut writer = CheckoutWriter::open(root)?;
        v.recheck()?;
        g.verify()?;
        let recovery = if before.header.as_ref() == Some(&h) {
            None
        } else {
            writer.publish(&h, body, Some(&before))?
        };
        if complete && let Some(claim) = claim {
            ClaimStore::completed(
                &g,
                &claim.id,
                &claim.session,
                h.close_reason.as_deref().unwrap_or(""),
            )
            .map_err(|e| completion_error(e, &h.id, &before.path))?;
        }
        let v = ResolvedView::load(&g)?;
        let mut result = inspect(&g, &v, &h.id)?;
        result.recovery_path = recovery;
        Ok(result)
    }
    pub fn relation_add(
        &self,
        id: &str,
        kind: RelationKind,
        target: &str,
    ) -> ExecutionResult<Inspection> {
        self.relate(id, kind, target, true)
    }
    pub fn relation_remove(
        &self,
        id: &str,
        kind: RelationKind,
        target: &str,
    ) -> ExecutionResult<Inspection> {
        self.relate(id, kind, target, false)
    }
    fn relate(
        &self,
        id: &str,
        kind: RelationKind,
        target: &str,
        add: bool,
    ) -> ExecutionResult<Inspection> {
        let view = self.view()?;
        let graph = ItemGraph::from_store(&view);
        let mut source = id.to_owned();
        let mut other = target.to_owned();
        if matches!(kind, RelationKind::Related) {
            if add
                && graph
                    .relations(id)
                    .is_ok_and(|r| r.related.contains(&other))
            {
                return Err(ExecutionError::new(
                    "already_exists",
                    "related edge already exists",
                ));
            }
            if !add
                && !view
                    .resolve(id)
                    .ok()
                    .and_then(|f| f.header.as_ref())
                    .is_some_and(|h| h.related.contains(&other))
            {
                source = target.into();
                other = id.into();
            }
        }
        self.mutate(
            &source,
            false,
            |h| {
                if h.id == other {
                    return Err(ExecutionError::new("invalid_argument", "self relationship"));
                }
                if matches!(kind, RelationKind::Parent) {
                    if add {
                        if h.parent.is_some() {
                            return Err(ExecutionError::new(
                                "already_exists",
                                "parent already set",
                            ));
                        }
                        h.parent = Some(other.clone());
                    } else if h.parent.as_deref() == Some(&other) {
                        h.parent = None;
                    } else {
                        return Err(ExecutionError::new("not_found", "parent edge"));
                    }
                } else {
                    let edges = match kind {
                        RelationKind::DependsOn => &mut h.depends_on,
                        RelationKind::Related => &mut h.related,
                        RelationKind::DiscoveredFrom => &mut h.discovered_from,
                        _ => unreachable!(),
                    };
                    if add {
                        if edges.contains(&other) {
                            return Err(ExecutionError::new(
                                "already_exists",
                                "edge already exists",
                            ));
                        }
                        edges.push(other.clone());
                    } else if let Some(n) = edges.iter().position(|e| e == &other) {
                        edges.remove(n);
                    } else {
                        return Err(ExecutionError::new("not_found", "edge"));
                    }
                }
                Ok(())
            },
            |ops| {
                if add {
                    ops.relation_add(&source, kind, &other)
                } else {
                    ops.relation_remove(&source, kind, &other)
                }
            },
        )
    }
    pub fn acquire(
        &self,
        input: &str,
        actor: &str,
        session: &SessionIdentity,
    ) -> ExecutionResult<(Value, Inspection)> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        let mut candidate = candidate(&g, &v, input)?;
        ClaimStore::validate_acquire(&g, &candidate, actor, session)?;
        let mut created = Vec::new();
        let result = (|| {
            prepare_claim_context(&g, &v, &mut candidate, &mut created)?;
            let result =
                ClaimStore::acquire_checked(&g, &candidate, actor, session, || v.recheck())?;
            created.push(json!({"id":result.claim.id,"path":encode_path(&g.root_path().join(format!("claims/{}.yaml",result.claim.id)))}));
            let current = ResolvedView::load(&g)?;
            Ok((
                result.claim.to_json(),
                inspect(&g, &current, &candidate.header.id)?,
            ))
        })();
        result.map_err(|error| with_setup_progress(error, &created))
    }
    pub fn reassign(
        &self,
        id: &str,
        actor: &str,
        session: &SessionIdentity,
        reason: &str,
        stopped: bool,
    ) -> ExecutionResult<(Value, Inspection)> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let old = ClaimStore::inspect(&g, id)?;
        let v = ResolvedView::load(&g)?;
        let mut candidate = candidate(&g, &v, &old.claim.item_id)?;
        ClaimStore::validate_reassign(&g, id, &candidate, actor, session, reason, stopped)?;
        let mut created = Vec::new();
        let result = (|| {
            prepare_claim_context(&g, &v, &mut candidate, &mut created)?;
            let result = ClaimStore::reassign_checked(
                &g,
                id,
                &candidate,
                actor,
                session,
                reason,
                stopped,
                || v.recheck(),
            )?;
            for (id, name) in [
                (
                    &result.previous_claim_id,
                    format!("{}.end.yaml", result.previous_claim_id),
                ),
                (&result.claim.id, format!("{}.yaml", result.claim.id)),
            ] {
                created.push(
                    json!({"id":id,"path":encode_path(&g.root_path().join("claims").join(name))}),
                );
            }
            Ok((
                result.claim.to_json(),
                inspect(&g, &v, &candidate.header.id)?,
            ))
        })();
        result.map_err(|error| with_setup_progress(error, &created))
    }
}
pub(crate) fn require_valid(store: &ItemStore) -> ExecutionResult<()> {
    let graph = ItemGraph::from_store(store);
    if graph.is_valid() {
        Ok(())
    } else {
        Err(OperationError::InvalidSource(graph.diagnostics().to_vec()).into())
    }
}
pub(crate) fn validate_candidate(
    store: &ItemStore,
    path: &Path,
    h: &ItemHeader,
    body: &[u8],
) -> ExecutionResult<()> {
    let mut files: Vec<_> = store
        .files
        .iter()
        .filter(|f| f.path != path)
        .cloned()
        .collect();
    files.push(parse_candidate(
        path.to_owned(),
        operations::serialize(h, body),
    ));
    let graph = ItemGraph::from_store(&ItemStore::from_candidate_files(files));
    if graph.is_valid() {
        Ok(())
    } else {
        Err(OperationError::InvalidCandidate(graph.diagnostics().to_vec()).into())
    }
}
fn candidate(
    g: &CoordinationGuard,
    v: &ResolvedView,
    input: &str,
) -> ExecutionResult<ClaimCandidate> {
    require_valid(&v.store)?;
    let source = v
        .store
        .resolve(input)
        .map_err(|e| ExecutionError::new("not_found", format!("item lookup: {e:?}")))?;
    let h = source
        .header
        .clone()
        .ok_or_else(|| ExecutionError::new("invalid_source", "invalid item"))?;
    let evaluation = ItemGraph::from_store(&v.store)
        .evaluate(&h.id)
        .map_err(|e| ExecutionError::new("not_ready", format!("{e:?}")))?;
    let candidate = ClaimCandidate {
        header: h,
        evaluation,
        workspace_id: None,
        run_id: None,
        session_record_id: None,
    };
    ClaimStore::validate_candidate(&candidate)?;
    g.verify()?;
    Ok(candidate)
}

fn prepare_claim_context(
    g: &CoordinationGuard,
    v: &ResolvedView,
    candidate: &mut ClaimCandidate,
    created: &mut Vec<Value>,
) -> ExecutionResult<()> {
    let id = &candidate.header.id;
    let root = v
        .sources
        .get(id)
        .ok_or_else(|| ExecutionError::new("source_unavailable", "material source is missing"))?;
    v.recheck()?;
    let workspace = if let Some(binding) = v.context.bindings.get(id) {
        v.context.workspaces[&binding.workspace_id].clone()
    } else {
        let workspace = ContextStore::register(g, root)?;
        if !v.context.workspaces.contains_key(&workspace.id) {
            created.push(json!({"id":workspace.id,"path":encode_path(&g.root_path().join(format!("workspaces/{}.yaml",workspace.id)))}));
        }
        ContextStore::bind(g, id, &workspace)?;
        created.push(json!({"id":id,"path":encode_path(&g.root_path().join(format!("workspaces/items/{id}.yaml")))}));
        workspace
    };
    if workspace.state != "open" {
        return Err(ExecutionError::new(
            "workspace_busy",
            "source workspace is closing",
        ));
    }
    candidate.workspace_id = Some(workspace.id);
    Ok(())
}
fn with_setup_progress(mut error: ExecutionError, created: &[Value]) -> ExecutionError {
    if !error.details.is_object() {
        error.details = json!({"cause_details":error.details});
    }
    let mut partial =
        error.details.get("partial").cloned().unwrap_or_else(
            || json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]}),
        );
    let mut all_created = created.to_vec();
    if let Some(records) = partial["created"].as_array() {
        for record in records {
            if !all_created.contains(record) {
                all_created.push(record.clone());
            }
        }
    }
    partial["created"] = json!(all_created);
    error.details["partial"] = partial;
    // Preserve publication from the last attempted write, even when setup
    // files have already been published. A pre-publication check has no write.
    if error.details.get("publication").is_none() {
        error.details["publication"] = json!("not_published");
    }
    error
}
pub(crate) fn inspect(
    g: &CoordinationGuard,
    v: &ResolvedView,
    id: &str,
) -> ExecutionResult<Inspection> {
    let graph = ItemGraph::from_store(&v.store);
    let ownership = ClaimStore::ownership_snapshot(g);
    inspect_with_snapshot(v, &graph, &ownership, id)
}
fn inspect_with_snapshot(
    v: &ResolvedView,
    graph: &ItemGraph,
    ownership: &ExecutionResult<OwnershipSnapshot>,
    id: &str,
) -> ExecutionResult<Inspection> {
    let mut i = operations::inspect_with_graph(&v.store, graph, id)?;
    let claim = match ownership {
        Ok(snapshot) => snapshot.current(id),
        Err(error) => Err(error.clone()),
    };
    let (claim, warning) = match claim {
        Ok(c) => (c, None),
        Err(e) => (
            None,
            Some(
                json!({"code":e.code,"message":e.message,"path":e.path.as_deref().map(encode_path)}),
            ),
        ),
    };
    if (claim.is_some() || warning.is_some())
        && let Some(e) = i.evaluation.as_mut()
    {
        e.executable = false;
    }
    i.context = json!({"source_worktree":v.sources.get(id).map(|p|encode_path(p)),"persistence":"material","run_id":v.run_ids.get(id),"claim":claim.map(|c|c.to_json())});
    if let Some(w) = warning {
        i.context["ownership_warning"] = w;
    }
    Ok(i)
}

fn completion_error(mut error: ExecutionError, id: &str, path: &Path) -> ExecutionError {
    let item = json!({"id":id,"path":encode_path(path)});
    error.details["published_item"] = item.clone();
    let mut partial =
        error.details.get("partial").cloned().unwrap_or_else(
            || json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]}),
        );
    let mut updated = partial["updated"].as_array().cloned().unwrap_or_default();
    if !updated.contains(&item) {
        updated.push(item);
    }
    partial["updated"] = json!(updated);
    error.details["partial"] = partial;
    error
}

#[cfg(test)]
mod tests {
    use super::super::project::discover;
    use super::*;
    use std::{fs, path::PathBuf, process::Command};

    struct Fixture {
        root: PathBuf,
        ops: ExecutionOperations,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("work-claim-execution-core-{}", new_id().unwrap()));
            fs::create_dir(&root).unwrap();
            assert!(
                Command::new("git")
                    .args(["init", "-q"])
                    .arg(&root)
                    .status()
                    .unwrap()
                    .success()
            );
            fs::create_dir_all(root.join(".work/items")).unwrap();
            let ops = ExecutionOperations::new(discover(Some(&root)).unwrap());
            Self { root, ops }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn uninitialized_fallback_does_not_inspect_or_edit_an_unlocked_header() {
        let fixture = Fixture::new();
        let item = fixture
            .ops
            .create(
                "original".into(),
                b"body".to_vec(),
                MetadataChange::default(),
            )
            .unwrap();
        let id = &item.file.header.as_ref().unwrap().id;
        fixture
            .ops
            .mutate(
                id,
                false,
                |_| panic!("fallback must not reconstruct an unlocked header"),
                |ops| {
                    ops.update(
                        id,
                        MetadataChange {
                            priority: Some(0),
                            ..MetadataChange::default()
                        },
                    )
                },
            )
            .unwrap();
        let final_item = fixture.ops.inspect(id).unwrap();
        assert_eq!(final_item.file.header.as_ref().unwrap().title, "original");
        assert_eq!(final_item.file.header.as_ref().unwrap().priority, 0);
    }

    #[test]
    fn binding_failure_reports_the_workspace_already_created_by_setup() {
        let fixture = Fixture::new();
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        let guard = CoordinationGuard::acquire(&fixture.ops.project, true).unwrap();
        let workspace = ContextStore::register(&guard, &fixture.root).unwrap();
        let id = new_id().unwrap();
        let workspace_record = json!({"id":workspace.id,"path":encode_path(&guard.root_path().join(format!("workspaces/{}.yaml",workspace.id)))});
        guard.ensure_dir(Path::new("workspaces/items")).unwrap();
        guard
            .create(
                &Path::new("workspaces/items").join(format!("{id}.yaml")),
                b"different",
            )
            .unwrap();
        let cause = ContextStore::bind(&guard, &id, &workspace).unwrap_err();
        let error = with_setup_progress(cause, std::slice::from_ref(&workspace_record));
        assert_eq!(error.code, "invalid_format");
        assert_eq!(error.details["publication"], "not_published");
        assert_eq!(
            error.details["partial"]["created"],
            json!([workspace_record])
        );
        assert!(
            guard
                .root_path()
                .join(format!("workspaces/{}.yaml", workspace.id))
                .exists()
        );
    }

    #[test]
    fn setup_progress_merges_claim_partial_and_preserves_uncertain_last_publication() {
        let workspace = json!({"id":"workspace","path":"/store/workspaces/workspace.yaml"});
        let ending = json!({"id":"old-claim","path":"/store/claims/old-claim.end.yaml"});
        let mut cause = ExecutionError::new("io", "replacement sync interrupted");
        cause.details = json!({"publication":"possible","errno":5,"partial":{
            "created":[ending],"updated":[],"deleted":[],"uncertain_paths":["/store/claims/new-claim.yaml"]}});
        let error = with_setup_progress(cause, std::slice::from_ref(&workspace));
        assert_eq!(error.code, "io");
        assert_eq!(error.details["errno"], 5);
        assert_eq!(error.details["publication"], "possible");
        assert_eq!(
            error.details["partial"]["created"],
            json!([workspace, ending])
        );
        assert_eq!(
            error.details["partial"]["uncertain_paths"],
            json!(["/store/claims/new-claim.yaml"])
        );
    }

    #[test]
    fn completion_partial_preserves_each_last_publication_and_underlying_error() {
        for publication in ["not_published", "possible", "published"] {
            let mut cause = ExecutionError::new("io", "ending publication interrupted")
                .at("/store/claims/ending.yaml");
            cause.details = json!({"publication":publication,"errno":5,"partial":{
                "created":[{"id":"ending","path":"/store/claims/ending.yaml"}],
                "updated":[],"deleted":[],"uncertain_paths":["/store/claims/ending.yaml"]}});
            let error = completion_error(cause, "item", Path::new("/checkout/.work/items/item.md"));
            assert_eq!(error.code, "io");
            assert_eq!(error.details["errno"], 5);
            assert_eq!(error.path.unwrap(), Path::new("/store/claims/ending.yaml"));
            assert_eq!(error.details["publication"], publication);
            assert_eq!(
                error.details["partial"]["created"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                error.details["partial"]["uncertain_paths"],
                json!(["/store/claims/ending.yaml"])
            );
            assert_eq!(
                error.details["partial"]["updated"],
                json!([error.details["published_item"]])
            );
        }
    }
}

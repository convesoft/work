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
use super::storage::{
    Storage, StorageDiagnostic, StorageErrorCode, StorageInspection, StorageState,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;

pub struct ExecutionOperations {
    pub project: Project,
    pub authorization: Vec<ClaimAuthorization>,
    pub checkout_view: bool,
    read_storage: RefCell<Option<StorageInspection>>,
}
impl ExecutionOperations {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            authorization: Vec::new(),
            checkout_view: false,
            read_storage: RefCell::new(None),
        }
    }
    fn physical(&self) -> DurableOperations {
        DurableOperations::new(&self.project.worktree_root)
    }
    fn fresh_physical(&self) -> DurableOperations {
        self.physical()
            .with_initialization_fence(&self.project.git_common_dir)
    }
    pub fn take_read_storage(&self) -> Option<StorageInspection> {
        self.read_storage.borrow_mut().take()
    }
    fn capture_read_storage(&self, storage: StorageInspection) {
        let mut captured = self.read_storage.borrow_mut();
        if captured.is_none() {
            *captured = Some(storage);
        }
    }
    fn guard(&self, write: bool) -> ExecutionResult<Option<CoordinationGuard>> {
        if self.checkout_view && !write {
            return Ok(None);
        }
        let s = Storage::new(self.project.clone()).inspect()?;
        if !s.coordination_available {
            if !write {
                self.capture_read_storage(s);
                return Ok(None);
            }
            if s.state == StorageState::Uninitialized
                && s.storage_warning.is_none()
                && self.authorization.is_empty()
            {
                return Ok(None);
            }
            if let Some(warning) = &s.storage_warning
                && warning.code == super::storage::StorageErrorCode::StorageBusy
            {
                let mut error = ExecutionError::new(warning.code.code(), warning.message.clone());
                error.path = warning.path.clone();
                error.details = json!({"publication":"not_published","diagnostics":s.diagnostics.iter()
                    .map(|d|json!({"code":d.code.code(),"message":d.message,"path":d.path.as_deref().map(encode_path),"line":d.line}))
                    .collect::<Vec<_>>()});
                return Err(error);
            }
            return Err(ExecutionError::new(
                "recovery_required",
                "coordination unavailable; storage requires explicit recovery",
            ));
        }
        #[cfg(test)]
        super::storage::files::inject("execution_guard_acquire", &s.lock_path)?;
        match CoordinationGuard::acquire(&self.project, write) {
            Ok(guard) => Ok(Some(guard)),
            Err(error) if !write && error.code == "storage_busy" => {
                let mut storage = s;
                let diagnostic = StorageDiagnostic {
                    code: StorageErrorCode::StorageBusy,
                    message: error.message,
                    path: error.path,
                    line: None,
                };
                storage.state = StorageState::RecoveryRequired;
                storage.coordination_available = false;
                storage.storage_warning = Some(diagnostic.clone());
                storage.diagnostics.push(diagnostic);
                self.capture_read_storage(storage);
                Ok(None)
            }
            Err(error) => Err(error),
        }
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
        if g.is_some() {
            Ok(self.physical().repair(id, raw)?)
        } else {
            Ok(self.fresh_physical().repair(id, raw)?)
        }
    }
    pub fn create(
        &self,
        title: String,
        body: Vec<u8>,
        change: MetadataChange,
    ) -> ExecutionResult<Inspection> {
        let Some(g) = self.guard(true)? else {
            return Ok(self.fresh_physical().create(title, body, change)?);
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
            return Ok(fallback(&self.fresh_physical())?);
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
        let _source_lock = material_source_lock(&v, &candidate.header.id)?;
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
        let _source_lock = material_source_lock(&v, &candidate.header.id)?;
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
fn material_source_lock(v: &ResolvedView, id: &str) -> ExecutionResult<CheckoutWriter> {
    let root = v
        .sources
        .get(id)
        .ok_or_else(|| ExecutionError::new("source_unavailable", "material source is missing"))?;
    Ok(CheckoutWriter::open(root)?)
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

    #[test]
    fn completion_sync_failure_reports_uncertain_ending_and_successfully_saved_item() {
        let mut fixture = Fixture::new();
        let ops = &mut fixture.ops;
        let item = ops
            .create("item".into(), Vec::new(), MetadataChange::default())
            .unwrap();
        let id = item.file.header.as_ref().unwrap().id.clone();
        Storage::new(ops.project.clone()).initialize().unwrap();
        let session = SessionIdentity {
            namespace: "provider".into(),
            id: "session".into(),
        };
        let (claim, _) = ops.acquire(&id, "worker", &session).unwrap();
        let claim_id = claim["id"].as_str().unwrap().to_owned();
        ops.authorization.push(ClaimAuthorization {
            claim_id: claim_id.clone(),
            session,
        });
        let failure = super::super::storage::files::fail_next("publication_sync");
        let error = ops.close(&id, None).unwrap_err();
        drop(failure);
        let ending = ops
            .project
            .git_common_dir
            .join(format!("work/claims/{claim_id}.end.yaml"));
        assert!(ending.exists());
        assert_eq!(error.code, "io");
        assert_eq!(error.details["errno"], 5);
        assert_eq!(error.details["publication"], "possible");
        assert_eq!(error.path.as_deref(), Some(ending.as_path()));
        assert_eq!(
            error.details["partial"]["uncertain_paths"],
            json!([encode_path(&ending)])
        );
        assert_eq!(
            error.details["partial"]["updated"],
            json!([error.details["published_item"]])
        );
        assert_eq!(
            ops.inspect(&id).unwrap().file.header.unwrap().state,
            Some(ManualState::Done)
        );
        let guard = CoordinationGuard::acquire(&ops.project, false).unwrap();
        assert!(!ClaimStore::inspect(&guard, &claim_id).unwrap().current);
    }

    fn fixture_item(fixture: &Fixture) -> String {
        fixture
            .ops
            .create("original".into(), Vec::new(), MetadataChange::default())
            .unwrap()
            .file
            .header
            .unwrap()
            .id
    }
    fn test_session() -> SessionIdentity {
        SessionIdentity {
            namespace: "provider".into(),
            id: "session".into(),
        }
    }
    fn controlled_hook(
        point: &'static str,
        entered: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    ) -> super::super::storage::files::FailureGuard {
        super::super::storage::files::on_next(point, move || {
            entered.send(()).unwrap();
            resume
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        })
    }

    #[test]
    fn delayed_fresh_fallback_refuses_after_initialization_and_claim_under_checkout_lock() {
        let fixture = Fixture::new();
        let id = fixture_item(&fixture);
        let holder = CheckoutWriter::open(&fixture.root).unwrap();
        let project = fixture.ops.project.clone();
        let worker_id = id.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let fallback = std::thread::spawn(move || {
            let _hook = controlled_hook("checkout_before_lock", entered_tx, resume_rx);
            ExecutionOperations::new(project).update(
                &worker_id,
                MetadataChange {
                    title: Some("unauthorized".into()),
                    ..MetadataChange::default()
                },
            )
        });
        // The fallback observed fresh storage while an actual checkout lock was
        // held. Delay its lock acquisition, initialize, and publish ownership.
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        drop(holder);
        let (claim, _) = fixture.ops.acquire(&id, "owner", &test_session()).unwrap();
        resume_tx.send(()).unwrap();
        let error = fallback.join().unwrap().unwrap_err();
        assert_eq!(error.code, "conflict");
        let item = fixture.ops.inspect(&id).unwrap();
        assert_eq!(item.file.header.unwrap().title, "original");
        assert_eq!(item.context["claim"]["id"], claim["id"]);
    }

    #[test]
    fn initialization_after_fresh_fence_orders_material_acquisition_after_mutation() {
        let fixture = Fixture::new();
        let id = fixture_item(&fixture);
        let project = fixture.ops.project.clone();
        let worker_id = id.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let fallback = std::thread::spawn(move || {
            let _hook = controlled_hook("checkout_fresh_checked", entered_tx, resume_rx);
            ExecutionOperations::new(project).update(
                &worker_id,
                MetadataChange {
                    title: Some("before ownership".into()),
                    ..MetadataChange::default()
                },
            )
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        let project = fixture.ops.project.clone();
        let claim_item = id.clone();
        let (claim_entered_tx, claim_entered_rx) = std::sync::mpsc::channel();
        let (claim_resume_tx, claim_resume_rx) = std::sync::mpsc::channel();
        let claimant = std::thread::spawn(move || {
            let _hook = controlled_hook("checkout_before_lock", claim_entered_tx, claim_resume_rx);
            ExecutionOperations::new(project).acquire(&claim_item, "owner", &test_session())
        });
        claim_entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            fs::read_dir(fixture.ops.project.git_common_dir.join("work/claims"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            CoordinationGuard::acquire(&fixture.ops.project, false)
                .err()
                .unwrap()
                .code,
            "storage_busy"
        );
        claim_resume_tx.send(()).unwrap();
        resume_tx.send(()).unwrap();
        fallback.join().unwrap().unwrap();
        // The initial candidate preceded the mutation. Once it gets the same
        // checkout lock its source check rejects that obsolete snapshot.
        assert_eq!(claimant.join().unwrap().unwrap_err().code, "conflict");
        let (_, item) = fixture.ops.acquire(&id, "owner", &test_session()).unwrap();
        assert_eq!(item.file.header.unwrap().title, "before ownership");
    }

    #[test]
    fn fresh_initialization_fence_covers_every_durable_fallback_mutation() {
        for operation in [
            "create",
            "update",
            "close",
            "reopen",
            "relation_add",
            "relation_remove",
            "repair",
        ] {
            let fixture = Fixture::new();
            let id = fixture_item(&fixture);
            let target = fixture_item(&fixture);
            if operation == "relation_remove" {
                fixture
                    .ops
                    .physical()
                    .relation_add(&id, RelationKind::DependsOn, &target)
                    .unwrap();
            }
            let prior = ItemStore::load(&fixture.ops.project).unwrap();
            let raw = prior.resolve(&id).unwrap().raw.clone();
            let fresh = fixture.ops.fresh_physical();
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let error = match operation {
                "create" => fresh
                    .create("new".into(), Vec::new(), MetadataChange::default())
                    .unwrap_err(),
                "update" => fresh
                    .update(
                        &id,
                        MetadataChange {
                            priority: Some(0),
                            ..MetadataChange::default()
                        },
                    )
                    .unwrap_err(),
                "close" => fresh.close(&id, None).unwrap_err(),
                "reopen" => fresh.reopen(&id).unwrap_err(),
                "relation_add" => fresh
                    .relation_add(&id, RelationKind::DependsOn, &target)
                    .unwrap_err(),
                "relation_remove" => fresh
                    .relation_remove(&id, RelationKind::DependsOn, &target)
                    .unwrap_err(),
                "repair" => fresh.repair(&id, raw).unwrap_err(),
                _ => unreachable!(),
            };
            assert_eq!(error.code(), "conflict", "{operation}: {error}");
            let after = ItemStore::load(&fixture.ops.project).unwrap();
            assert_eq!(prior.files.len(), after.files.len());
            for file in prior.files {
                assert_eq!(fs::read(file.path).unwrap(), file.raw);
            }
        }
    }

    #[test]
    fn first_fallback_storage_warning_survives_writer_release_and_later_resolved_read() {
        let fixture = Fixture::new();
        let id = fixture_item(&fixture);
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        fixture.ops.acquire(&id, "owner", &test_session()).unwrap();
        let writer = CoordinationGuard::acquire(&fixture.ops.project, true).unwrap();
        assert_eq!(fixture.ops.ready().unwrap().len(), 1); // Physical, unchecked ownership.
        drop(writer);
        assert!(fixture.ops.ready().unwrap().is_empty()); // Later resolved presentation.
        let captured = fixture.ops.take_read_storage().unwrap();
        assert!(!captured.coordination_available);
        assert_eq!(
            captured.storage_warning.unwrap().code,
            StorageErrorCode::StorageBusy
        );
        assert!(fixture.ops.take_read_storage().is_none());
        let next = ExecutionOperations::new(fixture.ops.project.clone());
        assert!(next.ready().unwrap().is_empty());
        assert!(next.take_read_storage().is_none());
    }

    #[test]
    fn second_guard_acquisition_contention_falls_back_only_for_reads_with_actual_warning() {
        use std::{cell::RefCell, rc::Rc};
        let fixture = Fixture::new();
        let id = fixture_item(&fixture);
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        for write in [false, true] {
            let ops = ExecutionOperations::new(fixture.ops.project.clone());
            let held = Rc::new(RefCell::new(None));
            let hook_held = held.clone();
            let project = ops.project.clone();
            let hook =
                super::super::storage::files::on_next("execution_guard_acquire", move || {
                    *hook_held.borrow_mut() =
                        Some(CoordinationGuard::acquire(&project, true).unwrap());
                });
            if write {
                assert_eq!(
                    ops.update(
                        &id,
                        MetadataChange {
                            title: Some("blocked".into()),
                            ..MetadataChange::default()
                        }
                    )
                    .unwrap_err()
                    .code,
                    "storage_busy"
                );
                assert!(ops.take_read_storage().is_none());
            } else {
                assert_eq!(ops.list().unwrap().len(), 1);
                let captured = ops.take_read_storage().unwrap();
                assert!(!captured.coordination_available);
                let warning = captured.storage_warning.unwrap();
                assert_eq!(warning.code, StorageErrorCode::StorageBusy);
                assert_eq!(warning.path.as_deref(), Some(captured.lock_path.as_path()));
                assert_eq!(captured.diagnostics.last().unwrap().path, warning.path);
            }
            drop(hook);
            held.borrow_mut().take();
        }
        assert_eq!(
            fixture.ops.inspect(&id).unwrap().file.header.unwrap().title,
            "original"
        );
    }

    #[test]
    fn material_source_lock_is_held_through_acquire_and_reassign_publication() {
        use rustix::fs::{FlockOperation, flock};
        use std::{cell::Cell, rc::Rc};
        let fixture = Fixture::new();
        let id = fixture_item(&fixture);
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        for reassign in [false, true] {
            let verified = Rc::new(Cell::new(false));
            let hook_verified = verified.clone();
            let path = fixture.root.join(".work/operations.lock");
            let project = fixture.ops.project.clone();
            // Initial acquisition publishes workspace, binding, then claim.
            // Reassignment publishes ending, then replacement acquisition.
            let hook = super::super::storage::files::on_nth(
                "before_publication",
                if reassign { 2 } else { 3 },
                move || {
                    let lock = fs::File::open(path).unwrap();
                    assert_eq!(
                        flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap_err(),
                        rustix::io::Errno::WOULDBLOCK
                    );
                    assert_eq!(
                        CoordinationGuard::acquire(&project, false)
                            .err()
                            .unwrap()
                            .code,
                        "storage_busy"
                    );
                    hook_verified.set(true);
                },
            );
            if reassign {
                let guard = CoordinationGuard::acquire(&fixture.ops.project, false).unwrap();
                let current = ClaimStore::current(&guard, &id).unwrap().unwrap();
                drop(guard);
                fixture
                    .ops
                    .reassign(&current.id, "new owner", &test_session(), "stopped", true)
                    .unwrap();
            } else {
                fixture.ops.acquire(&id, "owner", &test_session()).unwrap();
            }
            drop(hook);
            assert!(verified.get());
        }
    }
}

//! Coordinator facade: resolved source files and repository-wide ownership.
//! @mara implements DES-EXECUTION-IO
//! @mara implements DES-CLAIM-API
use super::claims::{ClaimAuthorization, ClaimCandidate, ClaimStore, OwnershipSnapshot};
use super::context::{ContextStore, ResolvedView};
use super::coordination::*;
use super::graph::ItemGraph;
use super::items::{Completion, ItemFile, ItemHeader, ItemStore, LookupError, ManualState};
use super::operations::{
    self, CheckoutWriter, DurableOperations, Inspection, MetadataChange, OperationError,
    RawInspection, RelationKind,
};
use super::project::Project;
use super::selection::{SelectionFilters, SelectionScope};
use super::storage::{Storage, StorageErrorCode, StorageInspection, StorageState};
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
            if s.diagnostics.iter().any(|diagnostic| {
                matches!(
                    diagnostic.code,
                    StorageErrorCode::PermissionDenied | StorageErrorCode::Io
                )
            }) {
                // Value-only inspection omits the original errno/provenance.
                // Re-read its evidence under the actual mutation guard.
                return CoordinationGuard::acquire(&self.project, true).map(Some);
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
        match CoordinationGuard::acquire_with_storage(&self.project, write) {
            Ok(guard) => Ok(Some(guard)),
            Err(failure) if !write => {
                let (_, storage) = *failure;
                self.capture_read_storage(storage);
                Ok(None)
            }
            Err(failure) => {
                let (error, _) = *failure;
                Err(error)
            }
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
            None => {
                let mut item = self.physical().inspect(id)?;
                if !self.checkout_view {
                    let id = &item.file.header.as_ref().unwrap().id;
                    item.context["digests"] =
                        json!(super::digests::list(&self.project.worktree_root, id)?);
                }
                Ok(item)
            }
            Some(g) => {
                let v = ResolvedView::load(&g)?;
                let graph = ItemGraph::from_store(&v.store);
                inspect_with_snapshot(&v, &graph, &v.ownership, id)
            }
        }
    }
    pub fn list(&self) -> ExecutionResult<Vec<Inspection>> {
        self.list_filtered(&SelectionFilters::default())
    }
    pub fn ready(&self) -> ExecutionResult<Vec<Inspection>> {
        self.ready_filtered(&SelectionFilters::default())
    }
    pub fn list_filtered(&self, filters: &SelectionFilters) -> ExecutionResult<Vec<Inspection>> {
        self.select_read(filters, false)
    }
    pub fn ready_filtered(&self, filters: &SelectionFilters) -> ExecutionResult<Vec<Inspection>> {
        self.select_read(filters, true)
    }
    fn select_read(
        &self,
        filters: &SelectionFilters,
        ready: bool,
    ) -> ExecutionResult<Vec<Inspection>> {
        filters.validate(self.checkout_view)?;
        let guard = self.guard(false)?;
        let resolved = guard.as_ref().map(ResolvedView::load).transpose()?;
        let physical;
        let store = if let Some(v) = &resolved {
            &v.store
        } else {
            physical = ItemStore::load(&self.project)?;
            &physical
        };
        let graph = ItemGraph::from_store(store);
        let scope = SelectionScope::new(filters, store, &graph, resolved.as_ref())?;
        if ready {
            require_valid(store)?;
            if let Some(v) = &resolved {
                v.ownership.as_ref().map_err(Clone::clone)?.exclusion()?;
            }
        }
        let mut result = Vec::new();
        for file in store.files.iter().filter(|f| f.is_valid()) {
            let h = file.header.as_ref().unwrap();
            let material = resolved
                .as_ref()
                .is_none_or(|v| v.sources.contains_key(&h.id));
            if scope.matches(h, material) {
                let inspection = if let Some(v) = &resolved {
                    inspect_with_snapshot(v, &graph, &v.ownership, &h.id)?
                } else {
                    let mut inspection = operations::inspect_with_graph(store, &graph, &h.id)?;
                    if !self.checkout_view {
                        inspection.context["digests"] =
                            json!(super::digests::list(&self.project.worktree_root, &h.id)?);
                    }
                    inspection
                };
                if !ready || inspection.evaluation.as_ref().is_some_and(|e| e.executable) {
                    result.push(inspection);
                }
            }
        }
        if ready {
            result.sort_by_key(|i| {
                let h = i.file.header.as_ref().unwrap();
                (h.priority, h.id.clone())
            });
        } else {
            result.sort_by_key(|i| i.file.header.as_ref().unwrap().id.clone());
        }
        Ok(result)
    }
    pub fn inspect_raw(&self, id: &str) -> ExecutionResult<RawInspection> {
        Ok(self.physical().inspect_raw(id)?)
    }
    pub fn repair(&self, id: &str, raw: Vec<u8>) -> ExecutionResult<RawInspection> {
        let g = self.guard(true)?;
        if let Some(g) = &g {
            ContextStore::load(g)?.require_open_path(&self.project.worktree_root)?;
            ClaimStore::authorize(g, id, &self.authorization)?;
            let v = ResolvedView::load(g)?;
            super::execution_finalization::require_squash_mutable(&v.runs, id)?;
            let path = self
                .project
                .worktree_root
                .join(format!(".work/items/{id}.md"));
            super::execution_finalization::validate_squash_edits(
                &v,
                &operations::candidate_store(&v.store, &path, raw.clone())?,
            )?;
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
        v.context.require_open_path(&self.project.worktree_root)?;
        require_valid(&v.store)?;
        let h = operations::apply_change(operations::default_header(new_id()?, title), change);
        ClaimStore::authorize(&g, &h.id, &self.authorization)?;
        let path = self
            .project
            .worktree_root
            .join(format!(".work/items/{}.md", h.id));
        validate_candidate(&v.store, &path, &h, &body)?;
        super::execution_finalization::validate_squash_edits(
            &v,
            &operations::candidate_store(&v.store, &path, operations::serialize(&h, &body))?,
        )?;
        let mut writer = CheckoutWriter::open(&self.project.worktree_root)?;
        v.recheck(&g)?;
        g.verify()?;
        let recovery = writer.publish(&h, &body, None)?;
        let mut result = reload_valid_inspection(&g, &h.id)
            .map_err(|e| published_reload_error(e, &h.id, &path, recovery.as_deref(), true))?;
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
        self.close_with_handoffs(id, reason, &[])
    }
    pub fn close_with_handoffs(
        &self,
        id: &str,
        reason: Option<String>,
        handoffs: &[super::handoffs::HandoffInput],
    ) -> ExecutionResult<Inspection> {
        let fallback = reason.clone();
        self.mutate_selected_handoffs(
            true,
            handoffs,
            |_| Ok((id.to_owned(), ())),
            |h, ()| {
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
        self.mutate_selected(
            complete,
            |_| Ok((id.to_owned(), ())),
            |h, ()| edit(h),
            fallback,
        )
    }
    fn mutate_selected<T>(
        &self,
        complete: bool,
        select: impl FnOnce(&ItemStore) -> ExecutionResult<(String, T)>,
        edit: impl FnOnce(&mut ItemHeader, T) -> ExecutionResult<()>,
        fallback: impl FnOnce(&DurableOperations) -> Result<Inspection, OperationError>,
    ) -> ExecutionResult<Inspection> {
        self.mutate_selected_handoffs(complete, &[], select, edit, fallback)
    }
    fn mutate_selected_handoffs<T>(
        &self,
        complete: bool,
        handoffs: &[super::handoffs::HandoffInput],
        select: impl FnOnce(&ItemStore) -> ExecutionResult<(String, T)>,
        edit: impl FnOnce(&mut ItemHeader, T) -> ExecutionResult<()>,
        fallback: impl FnOnce(&DurableOperations) -> Result<Inspection, OperationError>,
    ) -> ExecutionResult<Inspection> {
        // Supplying context requires initialized coordination, even on a fresh clone.
        let guard = if handoffs.is_empty() {
            self.guard(true)?
        } else {
            Some(CoordinationGuard::acquire(&self.project, true)?)
        };
        let Some(g) = guard else {
            // The original operation loads and applies only its requested edit
            // under the checkout lock, preserving concurrent unrelated changes.
            return Ok(fallback(&self.fresh_physical())?);
        };
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        let (id, selected) = select(&v.store)?;
        let before = v
            .store
            .resolve(&id)
            .map_err(|e| ExecutionError::new("not_found", format!("item lookup: {e:?}")))?
            .clone();
        let mut h = before
            .header
            .clone()
            .ok_or_else(|| ExecutionError::new("invalid_source", "invalid item"))?;
        if let Some(root) = v.sources.get(&h.id) {
            v.context.require_open_path(root)?;
        }
        let claim = ClaimStore::authorize(&g, &h.id, &self.authorization)?;
        edit(&mut h, selected)?;
        let body = before.body.as_deref().unwrap();
        validate_candidate(&v.store, &before.path, &h, body)?;
        super::execution_finalization::validate_squash_edits(
            &v,
            &operations::candidate_store(&v.store, &before.path, operations::serialize(&h, body))?,
        )?;
        if !handoffs.is_empty() {
            super::handoffs::HandoffStore::load(&g)?;
        }
        let prepared = handoffs
            .iter()
            .map(|input| super::execution_handoffs::prepare(&g, &v, input, &self.authorization))
            .collect::<ExecutionResult<Vec<_>>>()?;
        v.recheck(&g)?;
        g.verify()?;
        let changed = before.header.as_ref() != Some(&h);
        let mut saved = Vec::new();
        let result = (|| {
            for input in &prepared {
                let handoff = super::handoffs::HandoffStore::create(&g, input)?;
                saved.push(json!({"id":handoff.id(),"path":encode_path(&handoff.path)}));
            }
            // Outgoing context is now retained. Recheck before modifying the source.
            #[cfg(test)]
            if !prepared.is_empty() {
                super::storage::files::inject("handoffs_saved", &before.path)?;
            }
            v.recheck(&g)?;
            let recovery = if let Some(root) = v.sources.get(&h.id) {
                let mut writer = CheckoutWriter::open(root)?;
                v.recheck(&g)?;
                if !changed {
                    None
                } else {
                    writer.publish(&h, body, Some(&before))?
                }
            } else if changed {
                let run = v
                    .runs
                    .membership(&h.id)
                    .ok_or_else(|| ExecutionError::new("source_unavailable", "wisp run missing"))?;
                let expected = &run.wisp_sources[&h.id];
                let (_, recovery) = super::runs::RunStore::replace_wisp_with_recovery(
                    &g,
                    &run.manifest.id,
                    expected,
                    &h,
                    body,
                )
                .map_err(|error| {
                    if error.details["publication"] == "published" {
                        saved_item_error(error, &h.id, &before.path, "updated")
                    } else {
                        error
                    }
                })?;
                recovery
            } else {
                None
            };
            let mut endings = Vec::new();
            if complete && let Some(claim) = claim {
                let ended = ClaimStore::completed(
                    &g,
                    &claim.id,
                    &claim.session,
                    h.close_reason.as_deref().unwrap_or(""),
                )
                .map_err(|e| {
                    let mut error = completion_error(e, &h.id, &before.path);
                    if let Some(path) = &recovery {
                        error.details["previous_source_path"] = json!(encode_path(path));
                    }
                    error
                })?;
                if ended.changed {
                    endings.push(json!({"id":claim.id,"path":encode_path(&g.root_path().join(format!("claims/{}.end.yaml",claim.id)))}));
                }
            }
            let mut result = reload_valid_inspection(&g, &h.id).map_err(|e| {
                let error = if changed {
                    published_reload_error(e, &h.id, &before.path, recovery.as_deref(), false)
                } else {
                    e
                };
                if endings.is_empty() {
                    error
                } else {
                    with_setup_progress(claim_postpublication_error(error), &endings)
                }
            })?;
            result.recovery_path = recovery;
            if !saved.is_empty() {
                result.context["saved_handoffs"] = json!(saved);
            }
            // Best-effort retention cleanup is after completion and its ending.
            // Failure is explicit; completed work is never rolled back.
            if complete {
                let pruning = (|| {
                    let current = ResolvedView::load(&g)?;
                    super::handoffs::HandoffStore::prune(&g, &current, None)
                })();
                let pruned = pruning.map_err(|mut e| {
                    if e.details.get("publication").is_none() {
                        e.details["publication"] = json!("published");
                    }
                    if let Some(path) = &result.recovery_path {
                        e.details["previous_source_path"] = json!(encode_path(path));
                    }
                    let e = completion_error(e, &h.id, &before.path);
                    with_setup_progress(e, &endings)
                })?;
                if let Some(incoming) = result.context["incoming_handoffs"].as_array_mut() {
                    incoming.retain(|h| {
                        !pruned["deleted"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|d| d["id"] == h["id"])
                    });
                }
                result.context["handoff_pruning"] = pruned;
            }
            Ok(result)
        })();
        result.map_err(|mut e| {
            e = with_setup_progress(e, &saved);
            let known: Vec<_> = e.details["partial"]["created"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|record| {
                    record["path"]
                        .as_str()
                        .is_some_and(|path| path.contains("/handoffs/"))
                })
                .cloned()
                .collect();
            if !known.is_empty() {
                e.details["saved_handoffs"] = json!(known);
            }
            e
        })
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
        self.mutate_selected(
            false,
            |view| {
                let graph = ItemGraph::from_store(view);
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
                Ok((source, other))
            },
            |h, other| {
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
                    ops.relation_add(id, kind, target)
                } else {
                    ops.relation_remove(id, kind, target)
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
        self.acquire_with_session_record(input, actor, session, None)
    }
    pub fn acquire_with_session_record(
        &self,
        input: &str,
        actor: &str,
        session: &SessionIdentity,
        session_record_id: Option<&str>,
    ) -> ExecutionResult<(Value, Inspection)> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        let mut candidate = candidate(&g, &v, input)?;
        if let Some(id) = session_record_id {
            if !valid_id(id) {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "session_record_id must be a full UUIDv4",
                ));
            }
            let run_id = candidate.run_id.as_deref().ok_or_else(|| {
                ExecutionError::new(
                    "invalid_argument",
                    "named-session context requires current run membership",
                )
            })?;
            let record = v
                .runs
                .get(run_id)?
                .sessions
                .iter()
                .find(|r| r.id == id)
                .ok_or_else(|| {
                    ExecutionError::new(
                        "not_found",
                        "session record is not in the item's current run",
                    )
                })?;
            if &record.session != session {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "session identity does not match named record",
                ));
            }
            candidate.session_record_id = Some(id.into());
        }
        self.acquire_candidate(&g, &v, candidate, actor, session)
    }
    pub fn claim_next(
        &self,
        filters: &SelectionFilters,
        actor: &str,
        session: &SessionIdentity,
    ) -> ExecutionResult<Option<(Value, Inspection)>> {
        filters.validate(false)?;
        if actor.is_empty() {
            return Err(ExecutionError::new(
                "invalid_argument",
                "actor must be nonempty",
            ));
        }
        session.validate()?;
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        let graph = ItemGraph::from_store(&v.store);
        let scope = SelectionScope::new(filters, &v.store, &graph, Some(&v))?;
        v.ownership.as_ref().map_err(Clone::clone)?.exclusion()?;
        let mut eligible = Vec::new();
        for file in &v.store.files {
            let h = file.header.as_ref().unwrap();
            if scope.matches(h, v.sources.contains_key(&h.id)) {
                let i = inspect_with_snapshot(&v, &graph, &v.ownership, &h.id)?;
                if i.evaluation.as_ref().is_some_and(|e| e.executable) {
                    eligible.push(h);
                }
            }
        }
        eligible.sort_by_key(|h| (h.priority, &h.id));
        let Some(header) = eligible.first() else {
            v.recheck(&g)?;
            g.verify()?;
            return Ok(None);
        };
        let candidate = candidate(&g, &v, &header.id)?;
        self.acquire_candidate(&g, &v, candidate, actor, session)
            .map(Some)
    }
    fn acquire_candidate(
        &self,
        g: &CoordinationGuard,
        v: &ResolvedView,
        mut candidate: ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
    ) -> ExecutionResult<(Value, Inspection)> {
        ClaimStore::validate_acquire(g, &candidate, actor, session)?;
        let source_lock = material_source_lock(v, &candidate.header.id)?;
        let mut created = Vec::new();
        let result = (|| {
            verify_claim_source(&source_lock, v, g)?;
            prepare_claim_context(g, v, &mut candidate, &mut created)?;
            let result = ClaimStore::acquire_checked(g, &candidate, actor, session, || {
                verify_claim_source(&source_lock, v, g)
            })?;
            created.push(json!({"id":result.claim.id,"path":encode_path(&g.root_path().join(format!("claims/{}.yaml",result.claim.id)))}));
            let current = ResolvedView::load(g).map_err(claim_postpublication_error)?;
            let inspection =
                inspect(g, &current, &candidate.header.id).map_err(claim_postpublication_error)?;
            verify_claim_source(&source_lock, v, g).map_err(claim_postpublication_error)?;
            Ok((result.claim.to_json(), inspection))
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
        let source_lock = material_source_lock(&v, &candidate.header.id)?;
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
                || verify_claim_source(&source_lock, &v, &g),
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
            let inspection =
                inspect(&g, &v, &candidate.header.id).map_err(claim_postpublication_error)?;
            verify_claim_source(&source_lock, &v, &g).map_err(claim_postpublication_error)?;
            Ok((result.claim.to_json(), inspection))
        })();
        result.map_err(|error| with_setup_progress(error, &created))
    }
}
fn verify_claim_source(
    writer: &Option<CheckoutWriter>,
    view: &ResolvedView,
    guard: &CoordinationGuard,
) -> ExecutionResult<()> {
    if let Some(writer) = writer {
        writer.verify()?;
    }
    view.recheck(guard)
}
fn claim_postpublication_error(mut error: ExecutionError) -> ExecutionError {
    error.details["publication"] = json!("published");
    error
}
fn material_source_lock(v: &ResolvedView, id: &str) -> ExecutionResult<Option<CheckoutWriter>> {
    if let Some(root) = v.sources.get(id) {
        return Ok(Some(CheckoutWriter::open(root)?));
    }
    if v.runs
        .membership(id)
        .is_some_and(|r| r.wisp_sources.contains_key(id))
    {
        return Ok(None);
    }
    Err(ExecutionError::new(
        "source_unavailable",
        "material source is missing",
    ))
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
    let candidate = operations::candidate_store(store, path, operations::serialize(h, body))?;
    let graph = ItemGraph::from_store(&candidate);
    if graph.is_valid() {
        Ok(())
    } else {
        Err(OperationError::InvalidCandidate(graph.diagnostics().to_vec()).into())
    }
}
fn validate_item_reference(input: &str) -> ExecutionResult<()> {
    let id = input.strip_prefix("w-").unwrap_or(input);
    if id.len() == 32 && !valid_id(id) {
        return Err(ExecutionError::new(
            "invalid_argument",
            "full item ID must be a lowercase UUIDv4",
        ));
    }
    Ok(())
}

/// Shared run/claim item lookup; callers retain their graph/source validation.
pub(crate) fn resolve_item<'a>(store: &'a ItemStore, input: &str) -> ExecutionResult<&'a ItemFile> {
    validate_item_reference(input)?;
    store.resolve(input).map_err(|error| match error {
        LookupError::InvalidInput => ExecutionError::new(
            "invalid_argument",
            "item ID must be lowercase hexadecimal, optionally prefixed with w-",
        ),
        LookupError::NotFound => {
            ExecutionError::new("not_found", format!("item {input} was not found"))
        }
        LookupError::Ambiguous(ids) => ExecutionError::new(
            "ambiguous_id",
            format!("item {input} matches {}", ids.join(", ")),
        ),
        LookupError::Invalid(diagnostics) => OperationError::InvalidSource(diagnostics).into(),
    })
}

fn candidate(
    g: &CoordinationGuard,
    v: &ResolvedView,
    input: &str,
) -> ExecutionResult<ClaimCandidate> {
    validate_item_reference(input)?;
    require_valid(&v.store)?;
    let source = resolve_item(&v.store, input)?;
    let h = source
        .header
        .clone()
        .ok_or_else(|| ExecutionError::new("invalid_source", "invalid item"))?;
    let evaluation = ItemGraph::from_store(&v.store)
        .evaluate(&h.id)
        .map_err(|e| ExecutionError::new("not_ready", format!("{e:?}")))?;
    let run = v.runs.membership(&h.id);
    if run.is_some_and(|r| r.manifest.phase != super::runs::RunPhase::Active) {
        return Err(ExecutionError::new(
            "run_not_current",
            "run is frozen for cleanup",
        ));
    }
    let candidate = ClaimCandidate {
        header: h,
        evaluation,
        workspace_id: None,
        run_id: run.map(|r| r.manifest.id.clone()),
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
    let Some(root) = v.sources.get(id) else {
        let run = v
            .runs
            .membership(id)
            .ok_or_else(|| ExecutionError::new("source_unavailable", "wisp owner is missing"))?;
        if let Some(wid) = &run.manifest.default_workspace_id {
            let workspace = v.context.workspaces.get(wid).ok_or_else(|| {
                ExecutionError::new("source_unavailable", "run default workspace is missing")
            })?;
            if workspace.state != "open" {
                return Err(ExecutionError::new(
                    "workspace_busy",
                    "execution workspace is closing",
                ));
            }
            let project = super::project::discover(Some(&workspace.path)).map_err(|e| {
                ExecutionError::new("source_unavailable", e.to_string()).at(&workspace.path)
            })?;
            if project.git_common_dir != g.project().git_common_dir {
                return Err(ExecutionError::new(
                    "source_unavailable",
                    "execution workspace repository changed",
                ));
            }
            candidate.workspace_id = Some(wid.clone());
        }
        return Ok(());
    };
    v.recheck(g)?;
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
pub(crate) fn with_setup_progress(mut error: ExecutionError, created: &[Value]) -> ExecutionError {
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
    let canonical_id = i.file.header.as_ref().unwrap().id.clone();
    let id = canonical_id.as_str();
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
    let frozen = i
        .file
        .header
        .as_ref()
        .and_then(|header| v.runs.membership(&header.id))
        .is_some_and(|run| run.manifest.phase != super::runs::RunPhase::Active);
    if (claim.is_some() || warning.is_some() || frozen)
        && let Some(e) = i.evaluation.as_mut()
    {
        e.executable = false;
    }
    i.context = json!({"source_worktree":v.sources.get(id).map(|p|encode_path(p)),"persistence":if v.sources.contains_key(id){"material"}else{"wisp"},"run_id":v.run_ids.get(id),"claim":claim.map(|c|c.to_json())});
    i.context["digests"] = json!(
        v.sources
            .get(id)
            .map(|root| super::digests::list(root, id))
            .transpose()?
            .unwrap_or_default()
    );
    if let Some(w) = warning {
        i.context["ownership_warning"] = w;
    }
    match &v.handoffs {
        Ok(handoffs) => {
            i.context["incoming_handoffs"] = json!(
                handoffs
                    .iter()
                    .filter(|h| h.receivers().any(|receiver| receiver == id))
                    .map(|h| h.to_json())
                    .collect::<Vec<_>>()
            )
        }
        Err(e) => {
            i.context["incoming_handoffs"] = Value::Null;
            i.context["handoff_warning"] = json!({"code":e.code,"message":e.message,"path":e.path.as_deref().map(encode_path)});
        }
    }
    Ok(i)
}

fn reload_valid_inspection(g: &CoordinationGuard, id: &str) -> ExecutionResult<Inspection> {
    let view = ResolvedView::load(g)?;
    require_valid(&view.store)?;
    inspect(g, &view, id)
}

fn published_reload_error(
    mut error: ExecutionError,
    id: &str,
    path: &Path,
    recovery: Option<&Path>,
    created: bool,
) -> ExecutionError {
    // The item write succeeded; a later read's default publication status
    // describes that read, rather than the saved item.
    error.details["publication"] = json!("published");
    if let Some(path) = recovery {
        error.details["previous_source_path"] = json!(encode_path(path));
    }
    saved_item_error(error, id, path, if created { "created" } else { "updated" })
}

fn completion_error(error: ExecutionError, id: &str, path: &Path) -> ExecutionError {
    saved_item_error(error, id, path, "updated")
}

fn saved_item_error(
    mut error: ExecutionError,
    id: &str,
    path: &Path,
    bucket: &str,
) -> ExecutionError {
    let item = json!({"id":id,"path":encode_path(path)});
    error.details["published_item"] = item.clone();
    let mut partial =
        error.details.get("partial").cloned().unwrap_or_else(
            || json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]}),
        );
    let mut records = partial[bucket].as_array().cloned().unwrap_or_default();
    if !records.contains(&item) {
        records.push(item);
    }
    partial[bucket] = json!(records);
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
    fn close_reload_failure_retains_published_ending_including_already_done_retry() {
        for already_done in [false, true] {
            let mut fixture = Fixture::new();
            let id = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let session = test_session();
            let (claim, _) = fixture.ops.acquire(&id, "owner", &session).unwrap();
            let claim_id = claim["id"].as_str().unwrap().to_owned();
            fixture.ops.authorization.push(ClaimAuthorization {
                claim_id: claim_id.clone(),
                session,
            });
            let ending = fixture
                .ops
                .project
                .git_common_dir
                .join(format!("work/claims/{claim_id}.end.yaml"));
            if already_done {
                let failure = super::super::storage::files::fail_next("before_publication");
                fixture.ops.close(&id, None).unwrap_err();
                drop(failure);
                assert!(!ending.exists());
            }
            let unexpected = fixture
                .ops
                .project
                .git_common_dir
                .join("work/workspaces/unexpected");
            let damaged = unexpected.clone();
            let hook = super::super::storage::files::on_next("after_directory_sync", move || {
                fs::write(damaged, b"invalid").unwrap();
            });
            let error = fixture.ops.close(&id, None).unwrap_err();
            drop(hook);
            assert_eq!(error.code, "invalid_format");
            assert_eq!(error.path, Some(unexpected.clone()));
            assert!(ending.is_file());
            assert_eq!(error.details["publication"], "published");
            assert_eq!(
                error.details["partial"]["created"],
                json!([{"id":claim_id,"path":encode_path(&ending)}])
            );
            assert_eq!(
                error.details["partial"]["updated"]
                    .as_array()
                    .unwrap()
                    .len(),
                usize::from(!already_done)
            );
            if already_done {
                assert!(error.details.get("published_item").is_none());
            } else {
                assert_eq!(error.details["published_item"]["id"], id);
                assert!(error.details["previous_source_path"].is_string());
            }
            fs::remove_file(unexpected).unwrap();
            assert_eq!(
                fixture.ops.close(&id, None).unwrap_err().code,
                "stale_claim"
            );
            let guard = CoordinationGuard::acquire(&fixture.ops.project, false).unwrap();
            let inspection = ClaimStore::inspect(&guard, &claim_id).unwrap();
            assert!(!inspection.current);
            assert_eq!(
                inspection.ending.unwrap().outcome,
                super::super::claims::ClaimOutcome::Completed
            );
        }
    }

    #[test]
    fn initialized_create_reports_saved_item_on_every_final_reload_failure() {
        for failure in ["graph", "load", "inspect"] {
            let fixture = Fixture::new();
            let parent = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let root = fixture.root.clone();
            let workspaces = fixture.ops.project.git_common_dir.join("work/workspaces");
            let parent_path = root.join(format!(".work/items/{parent}.md"));
            let hook = super::super::storage::files::on_next("checkout_after_publish", move || {
                match failure {
                    "graph" => fs::remove_file(parent_path).unwrap(),
                    "load" => fs::write(workspaces.join("unexpected"), b"invalid").unwrap(),
                    "inspect" => {
                        for entry in fs::read_dir(root.join(".work/items")).unwrap() {
                            fs::remove_file(entry.unwrap().path()).unwrap();
                        }
                    }
                    _ => unreachable!(),
                }
            });
            let error = fixture
                .ops
                .create(
                    "child".into(),
                    b"child body".to_vec(),
                    MetadataChange {
                        parent: Some(Some(parent)),
                        ..Default::default()
                    },
                )
                .unwrap_err();
            drop(hook);
            assert_eq!(
                error.code,
                match failure {
                    "graph" => "invalid_source",
                    "load" => "invalid_format",
                    _ => "not_found",
                }
            );
            assert_eq!(error.details["publication"], "published");
            let saved = &error.details["published_item"];
            let id = saved["id"].as_str().unwrap();
            let path = fixture.root.join(format!(".work/items/{id}.md"));
            assert_eq!(saved["path"], encode_path(&path));
            assert_eq!(error.details["partial"]["created"], json!([saved]));
            if failure != "inspect" {
                assert_eq!(
                    super::super::items::ItemStore::load_from_root(&fixture.root)
                        .unwrap()
                        .resolve(id)
                        .unwrap()
                        .header
                        .as_ref()
                        .unwrap()
                        .title,
                    "child"
                );
            }
            if failure == "load" {
                assert_eq!(
                    error.path,
                    Some(
                        fixture
                            .ops
                            .project
                            .git_common_dir
                            .join("work/workspaces/unexpected")
                    )
                );
            }
        }
    }

    #[test]
    fn initialized_mutation_reports_saved_item_and_recovery_on_every_final_reload_failure() {
        for failure in ["graph", "load", "inspect"] {
            let fixture = Fixture::new();
            let parent = fixture_item(&fixture);
            let child = fixture
                .ops
                .create(
                    "child".into(),
                    b"child body".to_vec(),
                    MetadataChange {
                        parent: Some(Some(parent.clone())),
                        ..Default::default()
                    },
                )
                .unwrap();
            let id = child.file.header.as_ref().unwrap().id.clone();
            let path = child.file.path.clone();
            let original = fs::read(&path).unwrap();
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let parent_path = fixture.root.join(format!(".work/items/{parent}.md"));
            let child_path = path.clone();
            let workspaces = fixture.ops.project.git_common_dir.join("work/workspaces");
            let hook = super::super::storage::files::on_next("checkout_after_publish", move || {
                match failure {
                    "graph" => fs::remove_file(parent_path).unwrap(),
                    "load" => fs::write(workspaces.join("unexpected"), b"invalid").unwrap(),
                    "inspect" => fs::remove_file(child_path).unwrap(),
                    _ => unreachable!(),
                }
            });
            let error = fixture
                .ops
                .update(
                    &id,
                    MetadataChange {
                        title: Some("saved change".into()),
                        ..Default::default()
                    },
                )
                .unwrap_err();
            drop(hook);
            assert_eq!(
                error.code,
                match failure {
                    "graph" => "invalid_source",
                    "load" => "invalid_format",
                    _ => "not_found",
                }
            );
            assert_eq!(error.details["publication"], "published");
            assert_eq!(
                error.details["published_item"],
                json!({"id":id,"path":encode_path(&path)})
            );
            assert_eq!(
                error.details["partial"]["updated"],
                json!([error.details["published_item"]])
            );
            let recovery = error.details["previous_source_path"].as_str().unwrap();
            assert_eq!(fs::read(recovery).unwrap(), original);
            if failure != "inspect" {
                assert_eq!(
                    super::super::items::ItemStore::load_from_root(&fixture.root)
                        .unwrap()
                        .resolve(&id)
                        .unwrap()
                        .header
                        .as_ref()
                        .unwrap()
                        .title,
                    "saved change"
                );
            }
        }
    }

    #[test]
    fn published_reload_error_preserves_underlying_diagnostics_and_errno() {
        let mut cause = ExecutionError::new("io", "reload interrupted").at("/store/workspaces");
        cause.details = json!({"publication":"not_published","errno":5,"diagnostics":["original"]});
        let error = published_reload_error(cause, "item", Path::new("/items/item.md"), None, true);
        assert_eq!(error.code, "io");
        assert_eq!(error.message, "reload interrupted");
        assert_eq!(error.path.as_deref(), Some(Path::new("/store/workspaces")));
        assert_eq!(error.details["errno"], 5);
        assert_eq!(error.details["diagnostics"], json!(["original"]));
        assert_eq!(error.details["publication"], "published");
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
        let original = item.file.raw.clone();
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
        let recovery = error.details["previous_source_path"].as_str().unwrap();
        assert_eq!(fs::read(recovery).unwrap(), original);
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
    fn original_inspection_io_refusal_survives_claim_and_mutation_guards() {
        use super::super::storage::files;
        fn fail_metadata_open() -> files::FailureGuard {
            // The lock receipt opens first, followed by store.yaml evidence.
            files::on_nth("source_open", 2, || {
                std::mem::forget(files::fail_next("source_open"));
            })
        }
        let fixture = Fixture::new();
        let id = fixture_item(&fixture);
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        let original = fs::read(fixture.root.join(format!(".work/items/{id}.md"))).unwrap();
        let metadata = fixture.ops.project.git_common_dir.join("work/store.yaml");
        for claim in [false, true] {
            let hook = if claim {
                fail_metadata_open()
            } else {
                files::on_next("execution_guard_acquire", || {
                    // The outer action guard clears the one-shot failure too.
                    std::mem::forget(fail_metadata_open());
                })
            };
            let error = if claim {
                fixture
                    .ops
                    .acquire(&id, "owner", &test_session())
                    .unwrap_err()
            } else {
                fixture
                    .ops
                    .update(
                        &id,
                        MetadataChange {
                            title: Some("refused".into()),
                            ..Default::default()
                        },
                    )
                    .unwrap_err()
            };
            drop(hook);
            assert_eq!(error.code, "io");
            assert_eq!(error.path.as_deref(), Some(metadata.as_path()));
            assert_eq!(error.details["errno"], 5);
            assert_eq!(error.details["publication"], "not_published");
            assert!(error.details.get("operation_id").is_some());
            assert_eq!(error.details["recovery_paths"], json!([]));
            assert!(!error.message.contains("recovery"));
            assert_eq!(
                fs::read(fixture.root.join(format!(".work/items/{id}.md"))).unwrap(),
                original
            );
        }
        let hook = fail_metadata_open();
        let ops = ExecutionOperations::new(fixture.ops.project.clone());
        let read = ops.list().unwrap();
        drop(hook);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].file.header.as_ref().unwrap().id, id);
        let storage = ops.take_read_storage().unwrap();
        let warning = storage.storage_warning.unwrap();
        assert_eq!(warning.code, StorageErrorCode::Io);
        assert_eq!(warning.path.as_deref(), Some(metadata.as_path()));
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
    fn changed_foundation_at_second_acquisition_falls_back_with_actual_snapshot_for_all_reads() {
        use std::os::unix::fs::PermissionsExt;
        for unavailable in ["missing", "corrupt", "unreadable"] {
            for operation in ["list", "inspect", "ready", "write"] {
                let fixture = Fixture::new();
                let id = fixture_item(&fixture);
                Storage::new(fixture.ops.project.clone())
                    .initialize()
                    .unwrap();
                let metadata = fixture.ops.project.git_common_dir.join("work/store.yaml");
                let raw = fs::read(&metadata).unwrap();
                let mode = fs::metadata(&metadata).unwrap().permissions().mode();
                let item_path = fixture.root.join(format!(".work/items/{id}.md"));
                let item = fs::read(&item_path).unwrap();
                let changed = metadata.clone();
                let hook =
                    super::super::storage::files::on_next("execution_guard_acquire", move || {
                        match unavailable {
                            "missing" => fs::remove_file(changed).unwrap(),
                            "corrupt" => fs::write(changed, b"invalid metadata").unwrap(),
                            "unreadable" => {
                                fs::set_permissions(changed, fs::Permissions::from_mode(0o000))
                                    .unwrap()
                            }
                            _ => unreachable!(),
                        }
                    });
                if operation == "write" {
                    let error = fixture
                        .ops
                        .update(
                            &id,
                            MetadataChange {
                                title: Some("refused".into()),
                                ..Default::default()
                            },
                        )
                        .unwrap_err();
                    assert_eq!(
                        error.code,
                        if unavailable == "unreadable" {
                            "permission_denied"
                        } else {
                            "recovery_required"
                        }
                    );
                    assert!(fixture.ops.take_read_storage().is_none());
                } else {
                    let values = match operation {
                        "list" => fixture.ops.list().unwrap(),
                        "inspect" => vec![fixture.ops.inspect(&id).unwrap()],
                        "ready" => fixture.ops.ready().unwrap(),
                        _ => unreachable!(),
                    };
                    assert_eq!(values.len(), 1);
                    assert_eq!(values[0].file.header.as_ref().unwrap().id, id);
                    // Restore healthy storage and read again before consuming
                    // the first unavailable snapshot; no cross-request cache.
                    if unavailable == "unreadable" {
                        fs::set_permissions(&metadata, fs::Permissions::from_mode(mode)).unwrap();
                    }
                    fs::write(&metadata, &raw).unwrap();
                    fs::set_permissions(&metadata, fs::Permissions::from_mode(mode)).unwrap();
                    assert_eq!(fixture.ops.list().unwrap().len(), 1);
                    let snapshot = fixture.ops.take_read_storage().unwrap();
                    assert!(!snapshot.coordination_available);
                    assert_eq!(snapshot.state, StorageState::RecoveryRequired);
                    assert!(snapshot.metadata.is_none());
                    let warning = snapshot.storage_warning.unwrap();
                    assert_eq!(warning.path.as_deref(), Some(metadata.as_path()));
                    assert_eq!(
                        warning.code,
                        match unavailable {
                            "missing" => StorageErrorCode::StorageMissing,
                            "corrupt" => StorageErrorCode::InvalidFormat,
                            "unreadable" => StorageErrorCode::PermissionDenied,
                            _ => unreachable!(),
                        }
                    );
                    assert!(snapshot.diagnostics.contains(&warning));
                    assert!(fixture.ops.take_read_storage().is_none());
                }
                drop(hook);
                assert_eq!(fs::read(item_path).unwrap(), item);
            }
        }
    }

    #[test]
    fn related_removal_selects_and_authorizes_locked_snapshot_after_opposite_endpoint_rewrite() {
        for authorize_new_source in [false, true] {
            let mut fixture = Fixture::new();
            let first = fixture_item(&fixture);
            let second = fixture_item(&fixture);
            fixture
                .ops
                .relation_add(&first, RelationKind::Related, &second)
                .unwrap();
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let session = test_session();
            let (first_claim, _) = fixture
                .ops
                .acquire(&first, "first owner", &session)
                .unwrap();
            let (second_claim, _) = fixture
                .ops
                .acquire(&second, "second owner", &session)
                .unwrap();
            let first_auth = ClaimAuthorization {
                claim_id: first_claim["id"].as_str().unwrap().into(),
                session: session.clone(),
            };
            let second_auth = ClaimAuthorization {
                claim_id: second_claim["id"].as_str().unwrap().into(),
                session,
            };
            if authorize_new_source {
                fixture.ops.authorization.push(second_auth.clone());
            }
            let mut concurrent = ExecutionOperations::new(fixture.ops.project.clone());
            concurrent.authorization = vec![first_auth, second_auth];
            let a = first.clone();
            let b = second.clone();
            let hook =
                super::super::storage::files::on_next("execution_guard_acquire", move || {
                    concurrent
                        .relation_remove(&a, RelationKind::Related, &b)
                        .unwrap();
                    concurrent
                        .relation_add(&b, RelationKind::Related, &a)
                        .unwrap();
                });
            let result = fixture
                .ops
                .relation_remove(&first, RelationKind::Related, &second);
            drop(hook);
            if authorize_new_source {
                let saved = result.unwrap();
                assert_eq!(saved.file.header.as_ref().unwrap().id, second);
                assert!(saved.file.header.unwrap().related.is_empty());
            } else {
                assert_eq!(result.unwrap_err().code, "claim_conflict");
            }
            let store = ItemStore::load(&fixture.ops.project).unwrap();
            assert!(
                store
                    .resolve(&first)
                    .unwrap()
                    .header
                    .as_ref()
                    .unwrap()
                    .related
                    .is_empty()
            );
            assert_eq!(
                store
                    .resolve(&second)
                    .unwrap()
                    .header
                    .as_ref()
                    .unwrap()
                    .related
                    .is_empty(),
                authorize_new_source
            );
        }
    }

    #[test]
    fn mutation_selector_runs_under_the_same_exclusive_guard_as_source_authorization() {
        let fixture = Fixture::new();
        let first = fixture_item(&fixture);
        let second = fixture_item(&fixture);
        fixture
            .ops
            .relation_add(&second, RelationKind::Related, &first)
            .unwrap();
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        let saved = fixture
            .ops
            .mutate_selected(
                false,
                |store| {
                    assert_eq!(
                        CoordinationGuard::acquire(&fixture.ops.project, true)
                            .err()
                            .unwrap()
                            .code,
                        "storage_busy"
                    );
                    assert_eq!(
                        store
                            .resolve(&second)
                            .unwrap()
                            .header
                            .as_ref()
                            .unwrap()
                            .related,
                        vec![first.clone()]
                    );
                    Ok((second.clone(), first.clone()))
                },
                |header, target| {
                    header.related.retain(|id| id != &target);
                    Ok(())
                },
                |_| panic!("initialized mutation must use guarded selection"),
            )
            .unwrap();
        assert_eq!(saved.file.header.unwrap().id, second);
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
    fn claim_next_source_conflicts_preserve_pre_and_post_publication_evidence() {
        for after in [false, true] {
            let fixture = Fixture::new();
            let id = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let path = fixture.root.join(format!(".work/items/{id}.md"));
            let hook = super::super::storage::files::on_nth(
                if after {
                    "before_publication"
                } else {
                    "checkout_verify"
                },
                if after { 3 } else { 1 },
                move || {
                    let raw = fs::read_to_string(&path).unwrap();
                    fs::write(path, format!("{raw}\nExternal editor change\n")).unwrap();
                },
            );
            let error = fixture
                .ops
                .claim_next(&SelectionFilters::default(), "owner", &test_session())
                .unwrap_err();
            drop(hook);
            assert_eq!(error.code, "conflict");
            assert_eq!(
                error.details["publication"],
                if after { "published" } else { "not_published" }
            );
            let guard = CoordinationGuard::acquire(&fixture.ops.project, false).unwrap();
            let current = ClaimStore::current(&guard, &id).unwrap();
            if after {
                let claim = current.unwrap();
                assert!(
                    error.details["partial"]["created"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["id"] == claim.id)
                );
            } else {
                assert!(current.is_none());
                assert!(
                    error.details["partial"]["created"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn acquisition_rejects_substituted_checkout_lock_before_and_after_publication() {
        for after_publication_check in [false, true] {
            let fixture = Fixture::new();
            let id = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let root = fixture.root.clone();
            let item_id = id.clone();
            let hook = super::super::storage::files::on_nth(
                if after_publication_check {
                    "before_publication"
                } else {
                    "checkout_verify"
                },
                if after_publication_check { 3 } else { 1 },
                move || {
                    fs::rename(
                        root.join(".work/operations.lock"),
                        root.join(".work/old.lock"),
                    )
                    .unwrap();
                    fs::write(root.join(".work/operations.lock"), b"").unwrap();
                    if after_publication_check {
                        DurableOperations::new(&root).close(&item_id, None).unwrap();
                    }
                },
            );
            let error = fixture
                .ops
                .acquire(&id, "owner", &test_session())
                .unwrap_err();
            drop(hook);
            assert_eq!(error.code, "conflict");
            assert!(error.message.contains("operation lock changed"));
            assert_eq!(
                error.details["publication"],
                if after_publication_check {
                    "published"
                } else {
                    "not_published"
                }
            );
            let guard = CoordinationGuard::acquire(&fixture.ops.project, false).unwrap();
            let current = ClaimStore::current(&guard, &id).unwrap();
            if after_publication_check {
                let claim = current.unwrap();
                let record = json!({"id":claim.id,"path":encode_path(&guard.root_path().join(format!("claims/{}.yaml",claim.id)))});
                assert!(
                    error.details["partial"]["created"]
                        .as_array()
                        .unwrap()
                        .contains(&record)
                );
                assert_eq!(
                    DurableOperations::new(&fixture.root)
                        .inspect(&id)
                        .unwrap()
                        .file
                        .header
                        .unwrap()
                        .state,
                    Some(ManualState::Done)
                );
            } else {
                assert!(current.is_none());
                assert!(
                    error.details["partial"]["created"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|r| !r["path"].as_str().unwrap().contains("/claims/"))
                );
            }
        }
    }

    #[test]
    fn reassignment_rejects_substituted_checkout_lock_at_both_boundaries_and_after_publication() {
        for boundary in ["ending", "replacement", "after"] {
            let fixture = Fixture::new();
            let id = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let (claim, _) = fixture.ops.acquire(&id, "owner", &test_session()).unwrap();
            let old_id = claim["id"].as_str().unwrap();
            let root = fixture.root.clone();
            let item_id = id.clone();
            let hook = super::super::storage::files::on_nth(
                if boundary == "after" {
                    "before_publication"
                } else {
                    "checkout_verify"
                },
                if boundary == "ending" { 1 } else { 2 },
                move || {
                    fs::rename(
                        root.join(".work/operations.lock"),
                        root.join(".work/old.lock"),
                    )
                    .unwrap();
                    fs::write(root.join(".work/operations.lock"), b"").unwrap();
                    if boundary == "after" {
                        DurableOperations::new(&root).close(&item_id, None).unwrap();
                    }
                },
            );
            let error = fixture
                .ops
                .reassign(old_id, "replacement", &test_session(), "stopped", true)
                .unwrap_err();
            drop(hook);
            assert_eq!(error.code, "conflict");
            assert!(error.message.contains("operation lock changed"));
            assert_eq!(
                error.details["publication"],
                if boundary == "after" {
                    "published"
                } else {
                    "not_published"
                }
            );
            let guard = CoordinationGuard::acquire(&fixture.ops.project, false).unwrap();
            let old = ClaimStore::inspect(&guard, old_id).unwrap();
            let current = ClaimStore::current(&guard, &id).unwrap();
            let created = error.details["partial"]["created"].as_array().unwrap();
            if boundary == "ending" {
                assert!(old.current);
                assert!(old.ending.is_none());
                assert_eq!(current.unwrap().id, old_id);
                assert!(created.is_empty());
            } else {
                assert!(!old.current);
                assert!(old.ending.is_some());
                let ending = json!({"id":old_id,"path":encode_path(&guard.root_path().join(format!("claims/{old_id}.end.yaml")))});
                assert!(created.contains(&ending));
                if boundary == "replacement" {
                    assert!(current.is_none());
                    assert_eq!(created.len(), 1);
                } else {
                    let claim = current.unwrap();
                    assert_ne!(claim.id, old_id);
                    assert!(created.contains(&json!({"id":claim.id,"path":encode_path(&guard.root_path().join(format!("claims/{}.yaml",claim.id)))})));
                    assert_eq!(created.len(), 2);
                    assert_eq!(
                        DurableOperations::new(&fixture.root)
                            .inspect(&id)
                            .unwrap()
                            .file
                            .header
                            .unwrap()
                            .state,
                        Some(ManualState::Done)
                    );
                }
            }
        }
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
    #[test]
    fn run_entry_additions_refuse_material_update_before_publication() {
        for add_run in [false, true] {
            let fixture = Fixture::new();
            let target = fixture_item(&fixture);
            let other_root = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let started = fixture.ops.run_start(&target, None, None).unwrap();
            let run_id = started["run"]["id"].as_str().unwrap().to_owned();
            let source_path = fixture.root.join(format!(".work/items/{target}.md"));
            let before = fs::read(&source_path).unwrap();
            let shared = fixture.ops.project.git_common_dir.join("work");
            let mut manifest: Value = serde_json::from_slice(
                &fs::read(shared.join(format!("runs/{run_id}/run.yaml"))).unwrap(),
            )
            .unwrap();
            let added_run = new_id().unwrap();
            manifest["id"] = json!(added_run);
            manifest["root_item_id"] = json!(other_root);
            let wisp_id = new_id().unwrap();
            let mut header =
                operations::default_header(wisp_id.clone(), "unexpected graph source".into());
            header.depends_on = vec![wisp_id.clone()];
            let run_dir = shared
                .join("runs")
                .join(if add_run { &added_run } else { &run_id });
            let unexpected = run_dir.join("items").join(format!("{wisp_id}.md"));
            let added_path = unexpected.clone();
            // The existing edit callback executes after loading/validating the
            // view. A direct editor changes the source set while that snapshot
            // remains captured, before its publication recheck.
            let error = fixture
                .ops
                .mutate(
                    &target,
                    false,
                    move |edited| {
                        if add_run {
                            fs::create_dir_all(run_dir.join("items")).unwrap();
                            fs::create_dir_all(run_dir.join("sessions")).unwrap();
                            fs::write(run_dir.join("run.yaml"), yaml_bytes(&manifest)).unwrap();
                        }
                        fs::write(added_path, operations::serialize(&header, b"direct editor"))
                            .unwrap();
                        edited.title = "must not publish".into();
                        Ok(())
                    },
                    |_| panic!("initialized mutation must use the resolved snapshot"),
                )
                .unwrap_err();
            assert_eq!(error.code, "conflict", "{error:?}");
            assert!(error.message.contains(if add_run {
                "run entries changed"
            } else {
                "wisp entries changed"
            }));
            assert!(unexpected.is_file());
            assert_eq!(fs::read(&source_path).unwrap(), before);
            assert!(error.details.get("published_item").is_none());
        }
    }

    #[test]
    fn run_snapshot_recheck_refuses_missing_entries_and_same_byte_source_swaps() {
        for change in ["missing_run", "missing_wisp", "swap_manifest", "swap_wisp"] {
            let fixture = Fixture::new();
            let target = fixture_item(&fixture);
            Storage::new(fixture.ops.project.clone())
                .initialize()
                .unwrap();
            let run = fixture.ops.run_start(&target, None, None).unwrap();
            let run_id = run["run"]["id"].as_str().unwrap();
            let guard = CoordinationGuard::acquire(&fixture.ops.project, true).unwrap();
            let header = operations::default_header(new_id().unwrap(), "wisp".into());
            let wisp = super::super::runs::RunStore::create_wisp(&guard, run_id, &header, b"body")
                .unwrap();
            let view = ResolvedView::load(&guard).unwrap();
            let run_dir = guard.root_path().join("runs").join(run_id);
            match change {
                "missing_run" => fs::remove_dir_all(&run_dir).unwrap(),
                "missing_wisp" => fs::remove_file(&wisp).unwrap(),
                "swap_manifest" | "swap_wisp" => {
                    let path = if change == "swap_manifest" {
                        run_dir.join("run.yaml")
                    } else {
                        wisp
                    };
                    let raw = fs::read(&path).unwrap();
                    let replacement = path.with_extension("replacement");
                    fs::write(&replacement, raw).unwrap();
                    fs::rename(replacement, path).unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                view.recheck(&guard).unwrap_err().code,
                "conflict",
                "{change}"
            );
        }
    }

    #[test]
    fn certainly_published_wisp_mutation_retains_updated_progress() {
        let fixture = Fixture::new();
        let root = fixture_item(&fixture);
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        let run = fixture.ops.run_start(&root, None, None).unwrap();
        let run_id = run["run"]["id"].as_str().unwrap();
        let guard = CoordinationGuard::acquire(&fixture.ops.project, true).unwrap();
        let header = operations::default_header(new_id().unwrap(), "original wisp".into());
        let path =
            super::super::runs::RunStore::create_wisp(&guard, run_id, &header, b"body").unwrap();
        let original = fs::read(&path).unwrap();
        let lock = guard.root_path().join("coordination.lock");
        drop(guard);
        let lock_for_hook = lock.clone();
        let hook = super::super::storage::files::on_next("after_directory_sync", move || {
            fs::rename(&lock_for_hook, lock_for_hook.with_extension("retained")).unwrap();
            fs::write(lock_for_hook, b"").unwrap();
        });
        let error = fixture
            .ops
            .update(
                &header.id,
                MetadataChange {
                    title: Some("saved wisp".into()),
                    ..MetadataChange::default()
                },
            )
            .unwrap_err();
        drop(hook);
        assert_eq!(error.code, "conflict");
        assert_eq!(error.path, Some(lock));
        assert_eq!(error.details["publication"], "published");
        assert_eq!(error.details["published_path"], encode_path(&path));
        assert_eq!(
            error.details["partial"]["updated"],
            json!([{"id":header.id,"path":encode_path(&path)}])
        );
        assert_eq!(error.details["partial"]["uncertain_paths"], json!([]));
        assert!(
            String::from_utf8(fs::read(path).unwrap())
                .unwrap()
                .contains("saved wisp")
        );
        assert_eq!(error.details["recovery_paths"].as_array().unwrap().len(), 1);
        let previous = error.details["previous_source_path"].as_str().unwrap();
        assert_eq!(error.details["recovery_paths"][0], previous);
        assert_eq!(fs::read(decode_path(previous).unwrap()).unwrap(), original);
    }
    #[test]
    fn shared_item_lookup_preserves_invalid_source_diagnostics() {
        let id = new_id().unwrap();
        let file = super::super::items::parse_candidate(
            std::path::PathBuf::from("/fixture/wrong.md"),
            operations::serialize(
                &operations::default_header(id.clone(), "item".into()),
                b"body",
            ),
        );
        let store = ItemStore::from_candidate_files(vec![file]);
        for input in [&id, &format!("w-{id}")] {
            let error = resolve_item(&store, input).unwrap_err();
            assert_eq!(error.code, "invalid_source");
            assert!(
                error.details["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|diagnostic| diagnostic["message"]
                        .as_str()
                        .unwrap()
                        .contains("filename must be"))
            );
        }
    }

    fn fixture_wisp(fixture: &Fixture) -> (String, PathBuf) {
        let root = fixture_item(fixture);
        Storage::new(fixture.ops.project.clone())
            .initialize()
            .unwrap();
        let started = fixture.ops.run_start(&root, None, None).unwrap();
        let guard = CoordinationGuard::acquire(&fixture.ops.project, true).unwrap();
        let mut header = operations::default_header(new_id().unwrap(), "original wisp".into());
        header.parent = Some(root);
        let path = super::super::runs::RunStore::create_wisp(
            &guard,
            started["run"]["id"].as_str().unwrap(),
            &header,
            b"wisp body",
        )
        .unwrap();
        (header.id, path)
    }

    #[test]
    fn wisp_mutation_reload_failures_preserve_actual_retained_source() {
        for failure in ["graph", "load"] {
            let fixture = Fixture::new();
            let (id, path) = fixture_wisp(&fixture);
            let original = fs::read(&path).unwrap();
            let before = fixture.ops.inspect(&id).unwrap();
            let parent = before.file.header.unwrap().parent.unwrap();
            let parent_path = fixture.root.join(format!(".work/items/{parent}.md"));
            let unexpected = fixture
                .ops
                .project
                .git_common_dir
                .join("work/workspaces/unexpected");
            let damage = unexpected.clone();
            let hook =
                super::super::storage::files::on_next(
                    "after_directory_sync",
                    move || match failure {
                        "graph" => fs::remove_file(parent_path).unwrap(),
                        "load" => fs::write(damage, b"invalid").unwrap(),
                        _ => unreachable!(),
                    },
                );
            let error = fixture
                .ops
                .update(
                    &id,
                    MetadataChange {
                        title: Some("saved wisp".into()),
                        ..Default::default()
                    },
                )
                .unwrap_err();
            drop(hook);
            assert_eq!(
                error.code,
                match failure {
                    "graph" => "invalid_source",
                    "load" => "invalid_format",
                    _ => "not_found",
                },
                "{error:?}"
            );
            assert_eq!(error.details["publication"], "published");
            assert_eq!(
                error.details["published_item"],
                json!({"id":id,"path":encode_path(&path)})
            );
            assert_eq!(
                error.details["partial"]["updated"],
                json!([error.details["published_item"]])
            );
            let retained =
                decode_path(error.details["previous_source_path"].as_str().unwrap()).unwrap();
            assert_eq!(retained.parent(), path.parent());
            assert_eq!(fs::read(retained).unwrap(), original);
            if failure == "load" {
                assert_eq!(error.path, Some(unexpected));
            }
        }
    }

    #[test]
    fn wisp_completion_failures_keep_recovery_and_last_attempted_ending_publication() {
        for status in ["not_published", "possible", "published"] {
            let mut fixture = Fixture::new();
            let (id, path) = fixture_wisp(&fixture);
            let original = fs::read(&path).unwrap();
            let session = test_session();
            let (claim, _) = fixture.ops.acquire(&id, "worker", &session).unwrap();
            let claim_id = claim["id"].as_str().unwrap().to_owned();
            fixture.ops.authorization.push(ClaimAuthorization {
                claim_id: claim_id.clone(),
                session,
            });
            let ending = fixture
                .ops
                .project
                .git_common_dir
                .join(format!("work/claims/{claim_id}.end.yaml"));
            let lock = fixture
                .ops
                .project
                .git_common_dir
                .join("work/coordination.lock");
            let substituted = lock.clone();
            let installed = std::rc::Rc::new(std::cell::RefCell::new(None));
            let retained_fault = installed.clone();
            let hook = super::super::storage::files::on_nth(
                "after_directory_sync",
                if status == "published" { 2 } else { 1 },
                move || {
                    if status == "published" {
                        fs::rename(&substituted, substituted.with_extension("retained")).unwrap();
                        fs::write(substituted, b"").unwrap();
                    } else {
                        *installed.borrow_mut() = Some(super::super::storage::files::fail_next(
                            if status == "possible" {
                                "publication_sync"
                            } else {
                                "before_publication"
                            },
                        ));
                    }
                },
            );
            let error = fixture.ops.close(&id, None).unwrap_err();
            drop(hook);
            drop(retained_fault);
            assert_eq!(error.details["publication"], status, "{error:?}");
            assert_eq!(
                error.code,
                if status == "published" {
                    "conflict"
                } else {
                    "io"
                }
            );
            assert_eq!(
                error.path.as_deref(),
                Some(if status == "published" {
                    lock.as_path()
                } else {
                    ending.as_path()
                })
            );
            assert_eq!(
                error.details["errno"],
                if status == "published" {
                    Value::Null
                } else {
                    json!(5)
                }
            );
            assert_eq!(
                error.details["partial"]["updated"],
                json!([{"id":id,"path":encode_path(&path)}])
            );
            assert_eq!(
                error.details["partial"]["uncertain_paths"],
                if status == "possible" {
                    json!([encode_path(&ending)])
                } else {
                    json!([])
                }
            );
            assert_eq!(
                error.details["partial"]["created"],
                if status == "published" {
                    json!([{"id":claim_id,"path":encode_path(&ending)}])
                } else {
                    json!([])
                }
            );
            assert_eq!(ending.is_file(), status != "not_published");
            let recovery =
                decode_path(error.details["previous_source_path"].as_str().unwrap()).unwrap();
            assert_eq!(fs::read(recovery).unwrap(), original);
            assert!(
                String::from_utf8(fs::read(&path).unwrap())
                    .unwrap()
                    .contains("state: done")
            );
            if status == "not_published" {
                // Retry saves only the ending; the unchanged wisp must not be
                // exchanged again or invent a new previous-source path.
                let noop = fixture.ops.close(&id, None).unwrap();
                assert!(noop.recovery_path.is_none());
                assert!(ending.is_file());
            }
        }
    }
}

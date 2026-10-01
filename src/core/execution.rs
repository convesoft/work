//! Coordinator facade: resolved source files and repository-wide ownership.
//! @mara implements DES-EXECUTION-IO
//! @mara implements DES-CLAIM-API
use super::claims::{ClaimAuthorization, ClaimCandidate, ClaimStore};
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
                v.store
                    .files
                    .iter()
                    .filter_map(|f| f.header.as_ref())
                    .map(|h| inspect(&g, &v, &h.id))
                    .collect()
            }
        }
    }
    pub fn ready(&self) -> ExecutionResult<Vec<Inspection>> {
        let Some(g) = self.guard(false)? else {
            return Ok(self.physical().ready()?);
        };
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        ClaimStore::list(&g, None, true)?;
        let mut result = Vec::new();
        for file in &v.store.files {
            if let Some(h) = &file.header {
                let i = inspect(&g, &v, &h.id)?;
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
        self.mutate(
            id,
            false,
            |h| {
                *h = operations::apply_change(h.clone(), change);
                Ok(())
            },
            |ops| ops.update(id, MetadataChange::default()),
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
            // Apply the same requested edit through the original facade in fresh stores.
            let before = self.physical().inspect(id)?;
            let mut h = before.file.header.clone().unwrap();
            edit(&mut h)?;
            if complete {
                return Ok(fallback(&self.physical())?);
            }
            let old = before.file.header.as_ref().unwrap();
            if old.state != h.state {
                return Ok(self.physical().reopen(id)?);
            }
            let change = MetadataChange {
                title: Some(h.title),
                completion: Some(h.completion),
                priority: Some(h.priority),
                parent: Some(h.parent),
                depends_on: Some(h.depends_on),
                related: Some(h.related),
                discovered_from: Some(h.discovered_from),
                labels: Some(h.labels),
                model: Some(h.model),
                thinking: Some(h.thinking),
            };
            return Ok(self.physical().update(id, change)?);
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
            .map_err(|mut e| {
                e.details["published_item"] = json!({"id":h.id,"path":encode_path(&before.path)});
                e.details["publication"] = json!("published");
                e
            })?;
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
        let candidate = candidate(&g, &v, input)?;
        let result = ClaimStore::acquire(&g, &candidate, actor, session)?;
        let current = ResolvedView::load(&g)?;
        Ok((
            result.claim.to_json(),
            inspect(&g, &current, &candidate.header.id)?,
        ))
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
        let candidate = candidate(&g, &v, &old.claim.item_id)?;
        let result = ClaimStore::reassign(&g, id, &candidate, actor, session, reason, stopped)?;
        Ok((
            result.claim.to_json(),
            inspect(&g, &v, &candidate.header.id)?,
        ))
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
    require_valid(&ItemStore::from_candidate_files(files))
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
    if !evaluation.executable {
        return Err(ExecutionError::new(
            "not_ready",
            "item has unresolved lifecycle prerequisites",
        ));
    }
    let root = v
        .sources
        .get(&h.id)
        .ok_or_else(|| ExecutionError::new("source_unavailable", "material source is missing"))?;
    v.recheck()?;
    let workspace = if let Some(b) = v.context.bindings.get(&h.id) {
        v.context.workspaces[&b.workspace_id].clone()
    } else {
        let w = ContextStore::register(g, root)?;
        ContextStore::bind(g, &h.id, &w)?;
        w
    };
    if workspace.state != "open" {
        return Err(ExecutionError::new(
            "workspace_busy",
            "source workspace is closing",
        ));
    }
    v.recheck()?;
    g.verify()?;
    Ok(ClaimCandidate {
        header: h,
        evaluation,
        workspace_id: Some(workspace.id),
        run_id: None,
        session_record_id: None,
    })
}
pub(crate) fn inspect(
    g: &CoordinationGuard,
    v: &ResolvedView,
    id: &str,
) -> ExecutionResult<Inspection> {
    let mut i = operations::inspect_store(&v.store, id)?;
    let (claim, warning) = match ClaimStore::current(g, id) {
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

//! Handoff orchestration over the shared execution snapshot and lock.
//! @mara implements REQ-HANDOFF-CONTEXT
use super::claims::{ClaimAuthorization, ClaimStore};
use super::context::ResolvedView;
use super::coordination::*;
use super::execution::{ExecutionOperations, resolve_item};
use super::handoffs::{HandoffInput, HandoffStore};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) fn endpoints(v: &ResolvedView, references: &[String]) -> ExecutionResult<Vec<String>> {
    if references.is_empty() {
        return Err(ExecutionError::new(
            "invalid_argument",
            "handoff endpoints must be nonempty",
        ));
    }
    let mut ids = BTreeSet::new();
    for reference in references {
        let id = resolve_item(&v.store, reference)?
            .header
            .as_ref()
            .unwrap()
            .id
            .clone();
        if !ids.insert(id) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "duplicate handoff endpoint",
            ));
        }
    }
    Ok(ids.into_iter().collect())
}
pub(crate) fn prepare(
    g: &CoordinationGuard,
    v: &ResolvedView,
    input: &HandoffInput,
    authorization: &[ClaimAuthorization],
) -> ExecutionResult<HandoffInput> {
    let from_items = endpoints(v, &input.from_items)?;
    let to_items = endpoints(v, &input.to_items)?;
    for id in &from_items {
        ClaimStore::authorize(g, id, authorization)?;
    }
    if let Some(session) = &input.session {
        session.validate()?;
    }
    if let Some(id) = &input.workspace_id {
        if !valid_id(id) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "workspace ID must be full",
            ));
        }
        if !v.context.workspaces.contains_key(id) {
            return Err(ExecutionError::new("not_found", "workspace not registered"));
        }
    }
    Ok(HandoffInput {
        from_items,
        to_items,
        body: input.body.clone(),
        session: input.session.clone(),
        workspace_id: input.workspace_id.clone(),
    })
}
impl ExecutionOperations {
    pub fn handoff_create(&self, input: &HandoffInput) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        // Corruption is visible rather than an apparently empty context set.
        HandoffStore::load(&g)?;
        let input = prepare(&g, &v, input, &self.authorization)?;
        v.recheck(&g)?;
        let handoff = HandoffStore::create(&g, &input)?;
        Ok(json!({"handoff":handoff.to_json(),"changed":true}))
    }
    pub fn handoff_inspect(&self, id: &str) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, false)?;
        Ok(json!({"handoff":HandoffStore::inspect(&g,id)?.to_json()}))
    }
    pub fn handoff_list(&self, to: Option<&str>) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, false)?;
        let to = to
            .map(|to| -> ExecutionResult<String> {
                // Full historical IDs remain usable after their source run ends.
                if valid_id(to) {
                    Ok(to.to_owned())
                } else {
                    let v = ResolvedView::load(&g)?;
                    Ok(resolve_item(&v.store, to)?
                        .header
                        .as_ref()
                        .unwrap()
                        .id
                        .clone())
                }
            })
            .transpose()?;
        Ok(
            json!({"handoffs":HandoffStore::load(&g)?.iter().filter(|h|to.as_deref().is_none_or(|id|h.receivers().any(|receiver|receiver==id))).map(|h|h.to_json()).collect::<Vec<_>>()}),
        )
    }
    pub fn handoff_receivers(&self, id: &str, receivers: &[String]) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        HandoffStore::load(&g)?;
        let receivers = endpoints(&v, receivers)?;
        v.recheck(&g)?;
        let (handoff, changed) = HandoffStore::receivers(&g, id, receivers)?;
        Ok(json!({"handoff":handoff.to_json(),"changed":changed}))
    }
    pub fn handoff_prune(&self, ids: Option<&[String]>) -> ExecutionResult<Value> {
        if let Some(ids) = ids {
            let mut seen = BTreeSet::new();
            if ids.iter().any(|id| !valid_id(id) || !seen.insert(id)) {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "prune requires unique full handoff IDs",
                ));
            }
        }
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        v.recheck(&g)?;
        HandoffStore::prune(&g, &v, ids)
    }
}

//! Shared selected-view filters. Filtering never removes prerequisites from the graph.
//! @mara implements DES-CLAIM-API
use std::collections::BTreeSet;

use super::context::ResolvedView;
use super::coordination::{ExecutionError, ExecutionResult, valid_id};
use super::execution::resolve_item;
use super::graph::ItemGraph;
use super::items::{ItemHeader, ItemStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persistence {
    Material,
    Wisp,
}

#[derive(Debug, Clone, Default)]
pub struct SelectionFilters {
    pub root: Option<String>,
    pub run_id: Option<String>,
    pub labels_all: Vec<String>,
    pub priority_max: Option<u8>,
    pub persistence: Option<Persistence>,
}

impl SelectionFilters {
    pub fn validate(&self, checkout_view: bool) -> ExecutionResult<()> {
        if self.priority_max.is_some_and(|p| p > 4) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "priority_max must be 0–4",
            ));
        }
        if self.run_id.as_deref().is_some_and(|id| !valid_id(id)) {
            return Err(ExecutionError::new(
                "invalid_argument",
                "run_id must be a full UUIDv4",
            ));
        }
        if checkout_view && self.run_id.is_some() {
            return Err(ExecutionError::new(
                "invalid_argument",
                "run filtering requires the resolved view",
            ));
        }
        Ok(())
    }
}

pub(crate) struct SelectionScope<'a> {
    filters: &'a SelectionFilters,
    descendants: Option<BTreeSet<String>>,
    members: Option<BTreeSet<String>>,
}
impl<'a> SelectionScope<'a> {
    pub(crate) fn new(
        filters: &'a SelectionFilters,
        store: &ItemStore,
        graph: &ItemGraph,
        resolved: Option<&ResolvedView>,
    ) -> ExecutionResult<Self> {
        let descendants = filters
            .root
            .as_deref()
            .map(|input| {
                let root = resolve_item(store, input)?
                    .header
                    .as_ref()
                    .unwrap()
                    .id
                    .clone();
                let mut found = BTreeSet::new();
                let mut pending = vec![root];
                while let Some(id) = pending.pop() {
                    if found.insert(id.clone()) {
                        // Relations remain inspectable even when the graph is invalid.
                        pending.extend(
                            graph
                                .relations(&id)
                                .map_err(|_| {
                                    ExecutionError::new("not_found", format!("item {id} not found"))
                                })?
                                .children,
                        );
                    }
                }
                Ok::<_, ExecutionError>(found)
            })
            .transpose()?;
        let members = filters
            .run_id
            .as_deref()
            .map(|id| {
                let view = resolved.ok_or_else(|| {
                    ExecutionError::new(
                        "recovery_required",
                        "run filtering requires available coordination",
                    )
                })?;
                let run = view.runs.get(id)?;
                if !run.manifest.phase.is_current() {
                    return Err(ExecutionError::new("run_not_current", "run is terminal"));
                }
                Ok(run.members())
            })
            .transpose()?;
        Ok(Self {
            filters,
            descendants,
            members,
        })
    }

    pub(crate) fn matches(&self, header: &ItemHeader, material: bool) -> bool {
        self.descendants
            .as_ref()
            .is_none_or(|ids| ids.contains(&header.id))
            && self
                .members
                .as_ref()
                .is_none_or(|ids| ids.contains(&header.id))
            && self
                .filters
                .labels_all
                .iter()
                .all(|label| header.labels.contains(label))
            && self
                .filters
                .priority_max
                .is_none_or(|max| header.priority <= max)
            && self
                .filters
                .persistence
                .is_none_or(|p| material == (p == Persistence::Material))
    }
}

//! Read-only relationship and lifecycle evaluation for one selected item view.
//!
//! Item IDs remain the public identity. Petgraph indices are temporary mechanics
//! used only while validating the selected graph.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;

use super::items::{Completion, Diagnostic, ItemHeader, ItemStore, ManualState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    Invalid(Vec<Diagnostic>),
    NotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    Completed,
    Aggregate,
    Prerequisite {
        id: String,
        inherited_from: Option<String>,
    },
    Child(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub id: String,
    pub effective_done: bool,
    pub executable: bool,
    pub blockers: Vec<Blocker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relations {
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub depends_on: Vec<String>,
    pub blocks: Vec<String>,
    pub related: Vec<String>,
    pub discovered_from: Vec<String>,
    pub discovered_by: Vec<String>,
}

#[derive(Debug, Clone)]
struct Node {
    header: ItemHeader,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ItemGraph {
    nodes: BTreeMap<String, Node>,
    children: BTreeMap<String, BTreeSet<String>>,
    blocks: BTreeMap<String, BTreeSet<String>>,
    related: BTreeMap<String, BTreeSet<String>>,
    discovered_by: BTreeMap<String, BTreeSet<String>>,
    diagnostics: Vec<Diagnostic>,
    cycle_members: BTreeSet<String>,
}

impl ItemGraph {
    pub fn from_store(store: &ItemStore) -> Self {
        let mut result = Self {
            nodes: BTreeMap::new(),
            children: BTreeMap::new(),
            blocks: BTreeMap::new(),
            related: BTreeMap::new(),
            discovered_by: BTreeMap::new(),
            diagnostics: store.diagnostics().cloned().collect(),
            cycle_members: BTreeSet::new(),
        };
        for file in &store.files {
            if file.is_valid() {
                let header = file.header.as_ref().expect("valid file has header");
                result.nodes.insert(
                    header.id.clone(),
                    Node {
                        header: header.clone(),
                        path: file.path.clone(),
                    },
                );
            }
        }

        let mut related_edges = BTreeSet::new();
        for (id, node) in result.nodes.clone() {
            if let Some(parent) = &node.header.parent
                && result.check_target(&node, "parent", parent)
            {
                result
                    .children
                    .entry(parent.clone())
                    .or_default()
                    .insert(id.clone());
            }
            for target in &node.header.depends_on {
                if result.check_target(&node, "depends_on", target) {
                    result
                        .blocks
                        .entry(target.clone())
                        .or_default()
                        .insert(id.clone());
                }
            }
            for target in &node.header.related {
                if result.check_target(&node, "related", target) {
                    let edge = if id < *target {
                        (id.clone(), target.clone())
                    } else {
                        (target.clone(), id.clone())
                    };
                    if !related_edges.insert(edge) {
                        result.diagnose(
                            &node,
                            format!("duplicate semantic related edge between {id} and {target}"),
                        );
                    } else {
                        result
                            .related
                            .entry(id.clone())
                            .or_default()
                            .insert(target.clone());
                        result
                            .related
                            .entry(target.clone())
                            .or_default()
                            .insert(id.clone());
                    }
                }
            }
            for target in &node.header.discovered_from {
                if result.check_target(&node, "discovered_from", target) {
                    result
                        .discovered_by
                        .entry(target.clone())
                        .or_default()
                        .insert(id.clone());
                }
            }
        }

        result.check_cycles();
        result
            .diagnostics
            .sort_by(|a, b| (&a.path, a.line, &a.message).cmp(&(&b.path, b.line, &b.message)));
        result
    }

    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub(crate) fn is_cycle_member(&self, id: &str) -> bool {
        self.cycle_members.contains(id)
    }

    /// Inspection remains available when the selected graph is invalid.
    pub fn relations(&self, id: &str) -> Result<Relations, GraphError> {
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| GraphError::NotFound(id.into()))?;
        Ok(Relations {
            parent: node.header.parent.clone(),
            children: values(&self.children, id),
            depends_on: sorted(&node.header.depends_on),
            blocks: values(&self.blocks, id),
            related: values(&self.related, id),
            discovered_from: sorted(&node.header.discovered_from),
            discovered_by: values(&self.discovered_by, id),
        })
    }

    pub fn evaluate(&self, id: &str) -> Result<Evaluation, GraphError> {
        self.require_valid()?;
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| GraphError::NotFound(id.into()))?;
        let mut done_cache = BTreeMap::new();
        let effective_done = self.done(id, &mut done_cache);
        let mut blockers = Vec::new();
        match node.header.completion {
            Completion::Children => blockers.push(Blocker::Aggregate),
            Completion::Manual if node.header.state == Some(ManualState::Done) => {
                blockers.push(Blocker::Completed)
            }
            Completion::Manual => {
                let mut seen = BTreeSet::new();
                let mut ancestor = Some(id.to_owned());
                while let Some(source) = ancestor {
                    let current = &self.nodes[&source].header;
                    for target in &current.depends_on {
                        if seen.insert(target.clone()) && !self.done(target, &mut done_cache) {
                            blockers.push(Blocker::Prerequisite {
                                id: target.clone(),
                                inherited_from: (source != id).then_some(source.clone()),
                            });
                        }
                    }
                    ancestor = current.parent.clone();
                }
                for child in values(&self.children, id) {
                    if !self.done(&child, &mut done_cache) {
                        blockers.push(Blocker::Child(child));
                    }
                }
                blockers.sort_by_key(|blocker| {
                    let target = match blocker {
                        Blocker::Prerequisite { id, .. } | Blocker::Child(id) => id,
                        Blocker::Completed | Blocker::Aggregate => unreachable!("open manual item"),
                    };
                    (self.nodes[target].header.priority, target.clone())
                });
            }
        }
        Ok(Evaluation {
            id: id.into(),
            effective_done,
            executable: blockers.is_empty(),
            blockers,
        })
    }

    /// Uses the same evaluator as per-item explanations.
    pub fn ready(&self) -> Result<Vec<Evaluation>, GraphError> {
        self.require_valid()?;
        let mut ids: Vec<_> = self.nodes.keys().collect();
        ids.sort_by_key(|id| (self.nodes[*id].header.priority, (*id).clone()));
        ids.into_iter()
            .map(|id| self.evaluate(id))
            .filter_map(|result| match result {
                Ok(evaluation) if evaluation.executable => Some(Ok(evaluation)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn require_valid(&self) -> Result<(), GraphError> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(GraphError::Invalid(self.diagnostics.clone()))
        }
    }

    fn done(&self, id: &str, cache: &mut BTreeMap<String, bool>) -> bool {
        if let Some(done) = cache.get(id) {
            return *done;
        }
        let node = &self.nodes[id];
        let done = match node.header.completion {
            Completion::Manual => node.header.state == Some(ManualState::Done),
            Completion::Children => self.children.get(id).is_some_and(|children| {
                !children.is_empty() && children.iter().all(|child| self.done(child, cache))
            }),
        };
        cache.insert(id.into(), done);
        done
    }

    fn check_target(&mut self, node: &Node, relation: &str, target: &str) -> bool {
        if target == node.header.id {
            self.diagnose(node, format!("self-relation {relation} to {target}"));
            false
        } else if !self.nodes.contains_key(target) {
            self.diagnose(node, format!("unresolved {relation} target {target}"));
            false
        } else {
            true
        }
    }

    fn diagnose(&mut self, node: &Node, message: String) {
        self.diagnostics.push(Diagnostic {
            path: node.path.clone(),
            line: None,
            message,
        });
    }

    fn check_cycles(&mut self) {
        let mut index = BTreeMap::new();
        let mut hierarchy = DiGraph::<String, ()>::new();
        for id in self.nodes.keys() {
            index.insert(id.clone(), hierarchy.add_node(id.clone()));
        }
        let mut dependency = hierarchy.clone();
        let mut lifecycle = hierarchy.clone();
        for (id, node) in &self.nodes {
            if let Some(parent) = &node.header.parent
                && let Some(&target) = index.get(parent)
            {
                hierarchy.add_edge(index[id], target, ());
                lifecycle.add_edge(target, index[id], ()); // parent waits for child
            }
            for target in &node.header.depends_on {
                if let Some(&target_index) = index.get(target) {
                    dependency.add_edge(index[id], target_index, ());
                }
            }
            // Explicit prerequisites of all ancestors gate this descendant.
            let mut ancestor = Some(id.clone());
            let mut visited = BTreeSet::new();
            while let Some(source) = ancestor {
                if !visited.insert(source.clone()) {
                    break;
                }
                let current = &self.nodes[&source].header;
                for target in &current.depends_on {
                    if let Some(&target_index) = index.get(target) {
                        lifecycle.add_edge(index[id], target_index, ());
                    }
                }
                ancestor = current
                    .parent
                    .as_ref()
                    .filter(|parent| self.nodes.contains_key(*parent))
                    .cloned();
            }
        }
        self.cycle_diagnostics(&hierarchy, "hierarchy cycle");
        self.cycle_diagnostics(&dependency, "dependency cycle");
        self.cycle_diagnostics(&lifecycle, "combined lifecycle deadlock");
    }

    fn cycle_diagnostics(&mut self, graph: &DiGraph<String, ()>, label: &str) {
        for component in kosaraju_scc(graph) {
            if component.len() > 1
                || component
                    .iter()
                    .any(|node| graph.contains_edge(*node, *node))
            {
                let mut ids: Vec<_> = component.iter().map(|node| graph[*node].clone()).collect();
                ids.sort();
                self.cycle_members.extend(ids.iter().cloned());
                let path = self.nodes[&ids[0]].path.clone();
                self.diagnostics.push(Diagnostic {
                    path,
                    line: None,
                    message: format!("{label}: {}", ids.join(", ")),
                });
            }
        }
    }
}

fn values(map: &BTreeMap<String, BTreeSet<String>>, id: &str) -> Vec<String> {
    map.get(id)
        .map_or_else(Vec::new, |values| values.iter().cloned().collect())
}

fn sorted(values: &[String]) -> Vec<String> {
    let mut values = values.to_vec();
    values.sort();
    values
}

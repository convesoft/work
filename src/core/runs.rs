//! Guarded run manifests and wisps. The coordinator supplies material sources,
//! ownership and the resolved graph; runs never acquire a second lock.
//!
//! @mara implements DES-RUN-API

mod format;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::coordination::{
    CoordinationGuard, EntitySource, ExecutionError, ExecutionResult, new_id, now_timestamp,
    valid_id, yaml_bytes,
};
use super::graph::ItemGraph;
use super::items::{ItemFile, ItemHeader, ItemStore, parse_candidate};
pub use format::{CleanupKind, RunCleanup, RunManifest, RunPhase};

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub manifest: RunManifest,
    pub source: EntitySource,
    pub wisps: Vec<ItemFile>,
    pub wisp_sources: BTreeMap<String, EntitySource>,
}

/// Validated material routing and claim IDs supplied by the coordinator under
/// the same guard. No claim reader or duplicate material state lives here.
pub struct RunSnapshot<'a> {
    pub view: &'a ItemStore,
    pub active_claims: &'a BTreeSet<String>,
    pub material_ids: &'a BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct RunMutation {
    pub run: RunManifest,
    pub finished: bool,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct RunStore {
    pub records: Vec<RunRecord>,
}

fn fail(code: &'static str, message: impl Into<String>) -> ExecutionError {
    ExecutionError::new(code, message)
}

fn run_path(id: &str) -> PathBuf {
    PathBuf::from("runs").join(id)
}
fn manifest_path(id: &str) -> PathBuf {
    run_path(id).join("run.yaml")
}

impl RunRecord {
    pub fn members(&self) -> BTreeSet<String> {
        self.manifest
            .material_items
            .iter()
            .cloned()
            .chain(
                self.wisps
                    .iter()
                    .filter_map(|f| f.header.as_ref().map(|h| h.id.clone())),
            )
            .collect()
    }

    pub fn finished(&self, snapshot: &RunSnapshot<'_>) -> ExecutionResult<bool> {
        let graph = checked_graph(snapshot)?;
        let members = self.members();
        if members.is_empty() {
            return Ok(false);
        }
        let mut done = true;
        for id in members {
            let evaluation = graph.evaluate(&id).map_err(|_| {
                fail(
                    "source_unavailable",
                    format!("run member {id} is missing from the resolved graph"),
                )
            })?;
            done &= evaluation.effective_done && !snapshot.active_claims.contains(&id);
        }
        Ok(done)
    }
}

fn checked_graph(snapshot: &RunSnapshot<'_>) -> ExecutionResult<ItemGraph> {
    let graph = ItemGraph::from_store(snapshot.view);
    if !graph.is_valid() {
        let mut error = fail("invalid_source", "resolved item graph is invalid");
        error.details = serde_json::json!({"diagnostics":graph.diagnostics().iter().map(|d| serde_json::json!({"path":super::coordination::encode_path(&d.path),"message":d.message,"line":d.line})).collect::<Vec<_>>()});
        return Err(error);
    }
    Ok(graph)
}

fn require_material(snapshot: &RunSnapshot<'_>, id: &str) -> ExecutionResult<()> {
    if !valid_id(id) {
        return Err(fail(
            "invalid_argument",
            "material reference must be a full UUIDv4 ID",
        ));
    }
    if !snapshot.material_ids.contains(id) {
        return Err(fail(
            "invalid_argument",
            format!("{id} must resolve to a material item"),
        ));
    }
    if !snapshot
        .view
        .files
        .iter()
        .any(|f| f.header.as_ref().is_some_and(|h| h.id == id))
    {
        return Err(fail(
            "source_unavailable",
            format!("material source {id} is unavailable"),
        ));
    }
    Ok(())
}

impl RunStore {
    /// Complete, sorted load. Malformed entries, orphan directories and duplicate
    /// current membership are errors, never an apparently empty set of runs.
    pub fn load(guard: &CoordinationGuard) -> ExecutionResult<Self> {
        let mut records = Vec::new();
        let mut wisp_ids = BTreeSet::new();
        let mut current_roots = BTreeSet::new();
        let mut membership = BTreeSet::new();
        for id in guard.names(Path::new("runs"))? {
            let dir = run_path(&id);
            let absolute = guard.root_path().join(&dir);
            if !valid_id(&id) {
                return Err(fail("invalid_format", "unexpected run directory name").at(absolute));
            }
            let entries = guard.names(&dir)?;
            if entries
                .iter()
                .any(|n| !matches!(n.as_str(), "run.yaml" | "items" | "sessions"))
            {
                return Err(
                    fail("invalid_format", "unexpected entry in run directory").at(absolute)
                );
            }
            let source = guard.optional(&manifest_path(&id))?.ok_or_else(|| fail("invalid_format", "interrupted run directory has no manifest; inspect/remove it explicitly before retry").at(&absolute))?;
            let manifest = RunManifest::parse(&source.raw, &guard.metadata, &id)
                .map_err(|e| e.at(absolute.join("run.yaml")))?;
            let mut wisps = Vec::new();
            let mut wisp_sources = BTreeMap::new();
            for name in guard.names(&dir.join("items"))? {
                let relative = dir.join("items").join(&name);
                let path = guard.root_path().join(&relative);
                let item_id = name
                    .strip_suffix(".md")
                    .filter(|id| valid_id(id))
                    .ok_or_else(|| {
                        fail("invalid_format", "wisp filename must be <full-id>.md").at(&path)
                    })?;
                let item_source = guard.read(&relative)?;
                let item = parse_candidate(path.clone(), item_source.raw.clone());
                if !item.is_valid() || item.header.as_ref().is_none_or(|h| h.id != item_id) {
                    let mut error = fail(
                        "invalid_format",
                        "invalid wisp item document or filename identity",
                    )
                    .at(path);
                    error.details = serde_json::json!({"diagnostics":item.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()});
                    return Err(error);
                }
                if !wisp_ids.insert(item_id.to_owned()) {
                    return Err(fail("invalid_format", "duplicate wisp ID across runs").at(path));
                }
                wisp_sources.insert(item_id.to_owned(), item_source);
                wisps.push(item);
            }
            // Verify required empty directory exists, without implementing sessions.
            guard.names(&dir.join("sessions"))?;
            if manifest.phase.is_terminal() && !wisps.is_empty() {
                return Err(fail(
                    "invalid_format",
                    "terminal run retains unexpected wisp files",
                )
                .at(absolute));
            }
            if manifest.phase.is_current() {
                if !current_roots.insert(manifest.root_item_id.clone()) {
                    return Err(
                        fail("run_conflict", "multiple current runs for one root").at(&absolute)
                    );
                }
                for member in
                    std::iter::once(&manifest.root_item_id).chain(&manifest.material_items)
                {
                    if !membership.insert(member.clone()) {
                        return Err(fail(
                            "run_conflict",
                            format!("material item {member} belongs to multiple current runs"),
                        )
                        .at(&absolute));
                    }
                }
            }
            records.push(RunRecord {
                manifest,
                source,
                wisps,
                wisp_sources,
            });
        }
        if records.iter().any(|r| {
            wisp_ids.contains(&r.manifest.root_item_id)
                || r.manifest
                    .material_items
                    .iter()
                    .any(|id| wisp_ids.contains(id))
        }) {
            return Err(fail(
                "invalid_format",
                "run material membership names a wisp",
            ));
        }
        Ok(Self { records })
    }

    pub fn get(&self, id: &str) -> ExecutionResult<&RunRecord> {
        if !valid_id(id) {
            return Err(fail("invalid_argument", "run ID must be a full UUIDv4 ID"));
        }
        self.records
            .iter()
            .find(|r| r.manifest.id == id)
            .ok_or_else(|| fail("not_found", format!("run {id} not found")))
    }

    pub fn current_for_root(&self, root: &str) -> Option<&RunRecord> {
        self.records
            .iter()
            .find(|r| r.manifest.phase.is_current() && r.manifest.root_item_id == root)
    }

    /// Includes the root for exclusivity/context; `RunRecord::members` excludes it.
    pub fn membership(&self, id: &str) -> Option<&RunRecord> {
        self.records.iter().find(|r| {
            r.manifest.phase.is_current()
                && (r.manifest.root_item_id == id
                    || r.manifest.material_items.iter().any(|m| m == id)
                    || r.wisps
                        .iter()
                        .any(|f| f.header.as_ref().is_some_and(|h| h.id == id)))
        })
    }

    /// Defaults here are resolved workspace identities supplied by main, not paths.
    pub fn start(
        guard: &CoordinationGuard,
        snapshot: &RunSnapshot<'_>,
        root: &str,
        default_workspace_id: Option<String>,
        output_workspace_id: Option<String>,
    ) -> ExecutionResult<RunMutation> {
        checked_graph(snapshot)?;
        require_material(snapshot, root)?;
        for id in default_workspace_id.iter().chain(&output_workspace_id) {
            if !valid_id(id) {
                return Err(fail(
                    "invalid_argument",
                    "workspace ID must be a full UUIDv4 ID",
                ));
            }
        }
        let store = Self::load(guard)?;
        if let Some(record) = store.current_for_root(root) {
            if default_workspace_id
                .as_ref()
                .is_some_and(|id| Some(id) != record.manifest.default_workspace_id.as_ref())
                || output_workspace_id
                    .as_ref()
                    .is_some_and(|id| Some(id) != record.manifest.output_workspace_id.as_ref())
            {
                return Err(fail(
                    "run_conflict",
                    format!(
                        "current run {} has different workspace defaults",
                        record.manifest.id
                    ),
                ));
            }
            return Ok(RunMutation {
                run: record.manifest.clone(),
                finished: record.finished(snapshot)?,
                changed: false,
            });
        }
        if let Some(record) = store.membership(root) {
            return Err(fail(
                "run_conflict",
                format!("root is already in current run {}", record.manifest.id),
            ));
        }
        let occupied: BTreeSet<_> = guard.names(Path::new("runs"))?.into_iter().collect();
        let id = loop {
            let id = new_id()?;
            if !occupied.contains(&id) {
                break id;
            }
        };
        let run = RunManifest {
            format_version: 1,
            store_id: guard.metadata.store_id.clone(),
            recovery_generation: guard.metadata.recovery_generation.clone(),
            id: id.clone(),
            root_item_id: root.into(),
            created_at: now_timestamp(),
            phase: RunPhase::Active,
            material_items: Vec::new(),
            default_workspace_id,
            output_workspace_id,
            ended_at: None,
            cleanup: None,
        };
        let dir = run_path(&id);
        // Manifest is last. Failed directory creation remains visible as an orphan.
        let publish = || -> ExecutionResult<()> {
            guard.ensure_dir(&dir)?;
            guard.ensure_dir(&dir.join("items"))?;
            guard.ensure_dir(&dir.join("sessions"))?;
            guard.create(&manifest_path(&id), &yaml_bytes(&run.to_json()))
        };
        publish().map_err(|mut error| {
            error.details["run_id"] = serde_json::json!(id);
            error.details["run_directory"] = serde_json::json!(super::coordination::encode_path(
                &guard.root_path().join(&dir)
            ));
            error
        })?;
        Ok(RunMutation {
            run,
            finished: false,
            changed: true,
        })
    }

    pub fn attach(
        guard: &CoordinationGuard,
        snapshot: &RunSnapshot<'_>,
        run_id: &str,
        items: &[String],
    ) -> ExecutionResult<RunMutation> {
        Self::change_membership(guard, snapshot, run_id, items, true)
    }
    pub fn detach(
        guard: &CoordinationGuard,
        snapshot: &RunSnapshot<'_>,
        run_id: &str,
        items: &[String],
    ) -> ExecutionResult<RunMutation> {
        Self::change_membership(guard, snapshot, run_id, items, false)
    }

    fn change_membership(
        guard: &CoordinationGuard,
        snapshot: &RunSnapshot<'_>,
        run_id: &str,
        items: &[String],
        attach: bool,
    ) -> ExecutionResult<RunMutation> {
        checked_graph(snapshot)?;
        if items.is_empty() {
            return Err(fail(
                "invalid_argument",
                "membership items must be nonempty",
            ));
        }
        let store = Self::load(guard)?;
        let record = store.get(run_id)?;
        if record.manifest.phase != RunPhase::Active {
            return Err(fail(
                "run_not_current",
                "run is terminal or membership is frozen for cleanup",
            ));
        }
        let mut members: BTreeSet<_> = record.manifest.material_items.iter().cloned().collect();
        for id in items {
            require_material(snapshot, id)?;
            if id == &record.manifest.root_item_id {
                return Err(fail(
                    "invalid_argument",
                    "root is excluded from run members",
                ));
            }
            if attach
                && let Some(other) = store.membership(id)
                && other.manifest.id != run_id
            {
                return Err(fail(
                    "run_conflict",
                    format!("{id} is already in current run {}", other.manifest.id),
                ));
            }
            let changes = members.contains(id) != attach;
            if changes && snapshot.active_claims.contains(id) {
                return Err(fail(
                    "claim_conflict",
                    format!("active claim prevents changing membership of {id}"),
                ));
            }
            if attach {
                members.insert(id.clone());
            } else {
                members.remove(id);
            }
        }
        let mut next = record.clone();
        next.manifest.material_items = members.into_iter().collect();
        let finished = next.finished(snapshot)?;
        let changed = next.manifest != record.manifest;
        if changed {
            guard.replace(
                &manifest_path(run_id),
                &record.source,
                &yaml_bytes(&next.manifest.to_json()),
            )?;
        }
        Ok(RunMutation {
            run: next.manifest,
            finished,
            changed,
        })
    }

    /// Replace an authorized existing wisp using the token captured with the
    /// resolved view. Main validates the final graph before this one-file write.
    pub fn replace_wisp(
        guard: &CoordinationGuard,
        run_id: &str,
        expected: &EntitySource,
        header: &ItemHeader,
        body: &[u8],
    ) -> ExecutionResult<PathBuf> {
        let store = Self::load(guard)?;
        let record = store.get(run_id)?;
        if record.manifest.phase != RunPhase::Active {
            return Err(fail(
                "run_not_current",
                "run does not accept wisp mutations",
            ));
        }
        if !valid_id(&header.id) || !record.wisp_sources.contains_key(&header.id) {
            return Err(fail("not_found", "wisp not found in target run"));
        }
        let relative = run_path(run_id)
            .join("items")
            .join(format!("{}.md", header.id));
        let path = guard.root_path().join(&relative);
        let raw = super::operations::serialize(header, body);
        if !parse_candidate(path.clone(), raw.clone()).is_valid() {
            return Err(fail("invalid_format", "invalid wisp fields").at(path));
        }
        guard.replace(&relative, expected, &raw)?;
        Ok(path)
    }

    /// Run-side publication hook for an already validated expansion plan. Main
    /// checks the complete prospective graph, material collisions and authorization
    /// before the first write. Forward references must not trigger per-item checks.
    pub fn create_wisp(
        guard: &CoordinationGuard,
        run_id: &str,
        header: &ItemHeader,
        body: &[u8],
    ) -> ExecutionResult<PathBuf> {
        let store = Self::load(guard)?;
        let record = store.get(run_id)?;
        if record.manifest.phase != RunPhase::Active {
            return Err(fail("run_not_current", "run does not accept wisps"));
        }
        if !valid_id(&header.id) {
            return Err(fail("invalid_argument", "wisp ID must be a full UUIDv4 ID"));
        }
        if store.records.iter().any(|r| {
            r.wisps
                .iter()
                .any(|f| f.header.as_ref().is_some_and(|h| h.id == header.id))
                || r.manifest.root_item_id == header.id
                || r.manifest.material_items.contains(&header.id)
        }) {
            return Err(fail("run_conflict", "wisp ID collides with run membership"));
        }
        let relative = run_path(run_id)
            .join("items")
            .join(format!("{}.md", header.id));
        let path = guard.root_path().join(&relative);
        let raw = super::operations::serialize(header, body);
        if !parse_candidate(path.clone(), raw.clone()).is_valid() {
            return Err(fail("invalid_format", "invalid wisp fields").at(path));
        }
        guard.create(&relative, &raw)?;
        Ok(path)
    }
}

//! Shared durable item operations. Adapters only translate these results.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rustix::fs::{
    AtFlags, CWD, FlockOperation, Mode, OFlags, RenameFlags, flock, fsync, openat, renameat_with,
    statat, unlinkat,
};

use super::graph::{Evaluation, GraphError, ItemGraph, Relations};
use super::items::{
    Completion, Diagnostic, FileFingerprint, ItemFile, ItemHeader, ItemStore, ManualState,
    parse_candidate,
};

#[derive(Debug)]
pub enum OperationError {
    InvalidArgument(String),
    NotFound(String),
    AlreadyExists(String),
    InvalidSource(Vec<Diagnostic>),
    InvalidCandidate(Vec<Diagnostic>),
    Conflict(String),
    Io(io::Error),
    Published {
        id: String,
        path: PathBuf,
        previous_source_path: Option<PathBuf>,
        cause: Box<OperationError>,
    },
}

impl OperationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "invalid_argument",
            Self::NotFound(_) => "not_found",
            Self::AlreadyExists(_) => "already_exists",
            Self::InvalidSource(_) => "invalid_source",
            Self::InvalidCandidate(_) => "invalid_candidate",
            Self::Conflict(_) => "conflict",
            Self::Io(_) => "io",
            Self::Published { cause, .. } => cause.code(),
        }
    }

    pub fn published_item(&self) -> Option<(&str, &Path)> {
        match self {
            Self::Published { id, path, .. } => Some((id, path)),
            _ => None,
        }
    }

    pub fn previous_source_path(&self) -> Option<&Path> {
        match self {
            Self::Published {
                previous_source_path: Some(path),
                ..
            } => Some(path),
            _ => None,
        }
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message)
            | Self::NotFound(message)
            | Self::AlreadyExists(message)
            | Self::Conflict(message) => write!(f, "{}: {message}", self.code()),
            Self::InvalidSource(diagnostics) | Self::InvalidCandidate(diagnostics) => {
                write!(f, "{}: {} diagnostic(s)", self.code(), diagnostics.len())
            }
            Self::Io(error) => write!(f, "io: {error}"),
            Self::Published {
                id,
                path,
                previous_source_path,
                cause,
            } => {
                write!(
                    f,
                    "{cause}; item {id} may have been published at {}",
                    path.display()
                )?;
                if let Some(previous) = previous_source_path {
                    write!(f, "; previous source path was {}", previous.display())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for OperationError {}

impl From<io::Error> for OperationError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone)]
pub struct Inspection {
    pub context: serde_json::Value,
    pub file: ItemFile,
    pub relations: Relations,
    pub evaluation: Option<Evaluation>,
    pub graph_diagnostics: Vec<Diagnostic>,
    /// Previous source retained so writes through an already open handle remain recoverable.
    pub recovery_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RawInspection {
    pub file: ItemFile,
    pub graph_diagnostics: Vec<Diagnostic>,
    pub recovery_path: Option<PathBuf>,
    pub additional_recovery_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct MetadataChange {
    pub title: Option<String>,
    pub completion: Option<Completion>,
    pub priority: Option<u8>,
    pub parent: Option<Option<String>>,
    pub depends_on: Option<Vec<String>>,
    pub related: Option<Vec<String>>,
    pub discovered_from: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub model: Option<Option<String>>,
    pub thinking: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Parent,
    DependsOn,
    Related,
    DiscoveredFrom,
}

pub struct DurableOperations {
    root: PathBuf,
}

struct OperationLock {
    file: File,
    root_dir: OwnedFd,
    work_dir: OwnedFd,
}

impl DurableOperations {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = root.into();
        Self {
            root: root.components().collect(),
        }
    }

    pub fn inspect(&self, id: &str) -> Result<Inspection, OperationError> {
        let store = self.load()?;
        inspect_store(&store, id)
    }

    /// Return source and diagnostics even when the header or graph is invalid.
    pub fn inspect_raw(&self, id: &str) -> Result<RawInspection, OperationError> {
        if !valid_id(id) {
            return Err(OperationError::InvalidArgument(
                "item must be a full UUIDv4 ID".into(),
            ));
        }
        let store = self.load()?;
        let file = self.raw_source(&store, id)?.clone();
        Ok(RawInspection {
            file,
            graph_diagnostics: ItemGraph::from_store(&store).diagnostics().to_vec(),
            recovery_path: None,
            additional_recovery_paths: Vec::new(),
        })
    }

    /// Explicitly replace one malformed source document. Residual graph errors may
    /// remain while a caller repairs other documents in later steps.
    pub fn repair(&self, id: &str, raw: Vec<u8>) -> Result<RawInspection, OperationError> {
        self.repair_with_hook(id, raw, || Ok(()))
    }

    fn repair_with_hook(
        &self,
        id: &str,
        raw: Vec<u8>,
        after_canonical_publish: impl FnOnce() -> Result<(), OperationError>,
    ) -> Result<RawInspection, OperationError> {
        if !valid_id(id) {
            return Err(OperationError::InvalidArgument(
                "item must be a full UUIDv4 ID".into(),
            ));
        }
        let lock = self.lock()?;
        let items = self.items_dir(&lock)?;
        let mut store = self.load_from_dir(&items)?;
        let mut shadow_recovery = None;
        let target = self.item_path(id);
        if let Some(canonical) = store
            .files
            .iter()
            .find(|file| file.path == target && file.header.is_none())
        {
            let matches: Vec<_> = store
                .files
                .iter()
                .filter(|file| {
                    file.path != target
                        && file.header.as_ref().is_some_and(|header| header.id == id)
                })
                .collect();
            if let [misnamed] = matches.as_slice() {
                source_mode(canonical)?;
                let candidate = parse_candidate(target.clone(), raw.clone());
                if !candidate.is_valid() {
                    return Err(OperationError::InvalidArgument(
                        candidate
                            .diagnostics
                            .iter()
                            .map(|d| d.message.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                    ));
                }
                let mut files = store.files.clone();
                files.retain(|file| file.path != target && file.path != misnamed.path);
                files.push(candidate);
                let after = ItemGraph::from_store(&ItemStore::from_candidate_files(files));
                let before = ItemGraph::from_store(&store);
                let new_diagnostics: Vec<_> = after
                    .diagnostics()
                    .iter()
                    .filter(|d| !before.diagnostics().contains(d))
                    .cloned()
                    .collect();
                if !new_diagnostics.is_empty() {
                    return Err(OperationError::InvalidCandidate(new_diagnostics));
                }
                self.check_snapshot(&lock, &items, &store)?;
                let recovery = self.archive_shadow(&lock, &items, misnamed)?;
                store = self
                    .load_from_dir(&items)
                    .map_err(|error| with_recovery(error, &recovery))?;
                shadow_recovery = Some(recovery);
            }
        }
        let result = self.repair_loaded(&lock, &items, id, &raw, &store, after_canonical_publish);
        match shadow_recovery {
            Some(recovery) => result
                .map(|mut inspection| {
                    inspection.additional_recovery_paths.push(recovery.clone());
                    inspection
                })
                .map_err(|error| with_recovery(error, &recovery)),
            None => result,
        }
    }

    fn repair_loaded(
        &self,
        lock: &OperationLock,
        items: &OwnedFd,
        id: &str,
        raw: &[u8],
        store: &ItemStore,
        after_canonical_publish: impl FnOnce() -> Result<(), OperationError>,
    ) -> Result<RawInspection, OperationError> {
        let original = self.raw_source(store, id)?;
        if let Some(header) = &original.header
            && header.id != id
        {
            return Err(OperationError::InvalidArgument(format!(
                "source contains item {}; repair it by that ID",
                header.id
            )));
        }
        let target_path = self.item_path(id);
        if original.path != target_path && store.files.iter().any(|file| file.path == target_path) {
            return self.resume_filename_repair(
                lock,
                items,
                id,
                raw,
                (store, original),
                after_canonical_publish,
            );
        }
        let graph_before = ItemGraph::from_store(store);
        if original.is_valid()
            && !graph_before.is_cycle_member(id)
            && !graph_before
                .diagnostics()
                .iter()
                .any(|d| d.path == original.path)
        {
            return Err(OperationError::InvalidArgument(
                "item has no source or graph diagnostic to repair".into(),
            ));
        }
        let candidate = parse_candidate(target_path.clone(), raw.to_vec());
        if !candidate.is_valid() {
            return Err(OperationError::InvalidArgument(
                candidate
                    .diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        let candidate_store = if original.path == target_path {
            candidate_store(store, &target_path, raw.to_vec())?
        } else {
            let mut view = store.clone();
            view.files.retain(|file| file.path != original.path);
            view.files.push(candidate.clone());
            ItemStore::from_candidate_files(view.files)
        };
        let graph_after = ItemGraph::from_store(&candidate_store);
        let new_diagnostics: Vec<_> = graph_after
            .diagnostics()
            .iter()
            .filter(|d| !graph_before.diagnostics().contains(d))
            .cloned()
            .collect();
        if !new_diagnostics.is_empty() {
            return Err(OperationError::InvalidCandidate(new_diagnostics));
        }
        let staged = self.stage_with_mode(items, raw, Some(source_mode(original)?))?;
        if let Err(error) = self.check_snapshot(lock, items, store) {
            remove_stage(items, &staged);
            return Err(error);
        }
        if original.path != target_path {
            match rename_in_dir(items, &staged, &target_path, RenameFlags::NOREPLACE) {
                Ok(()) => {}
                Err(error) if error == rustix::io::Errno::EXIST => {
                    remove_stage(items, &staged);
                    return Err(OperationError::Conflict(
                        "canonical item path appeared during repair".into(),
                    ));
                }
                Err(error) => {
                    remove_stage(items, &staged);
                    return Err(OperationError::Io(io::Error::from(error)));
                }
            }
            return (|| {
                after_canonical_publish()?;
                self.sync_items(items)?;
                rename_in_dir(items, &original.path, &staged, RenameFlags::NOREPLACE)
                    .map_err(|error| OperationError::Io(io::Error::from(error)))?;
                self.sync_items(items)
                    .map_err(|error| with_recovery(error, &staged))?;
                if !archive_matches(items, original, &staged) {
                    return Err(OperationError::Conflict(format!(
                        "source changed during filename repair; recovery copy retained at {}",
                        staged.display()
                    )));
                }
                self.ensure_selected_dir_after_publication(lock, items, &staged)?;
                let mut result = self
                    .inspect_raw_from_dir(items, id)
                    .map_err(|error| with_recovery(error, &staged))?;
                result.recovery_path = Some(staged);
                Ok(result)
            })()
            .map_err(|cause| OperationError::Published {
                id: id.to_owned(),
                path: target_path,
                previous_source_path: Some(original.path.clone()),
                cause: Box::new(cause),
            });
        }
        let published = fingerprint_at(items, &staged)?;
        if let Err(error) = rename_in_dir(items, &staged, &original.path, RenameFlags::EXCHANGE) {
            remove_stage(items, &staged);
            return Err(exchange_error(error));
        }
        self.verify_exchange(items, store, original, &staged, raw, &published)?;
        self.sync_items(items)
            .map_err(|error| with_recovery(error, &staged))?;
        self.ensure_selected_dir_after_publication(lock, items, &staged)?;
        let mut result = self
            .inspect_raw_from_dir(items, id)
            .map_err(|error| with_recovery(error, &staged))?;
        result.recovery_path = Some(staged);
        Ok(result)
    }

    pub fn list(&self) -> Result<Vec<Inspection>, OperationError> {
        let store = self.load()?;
        let graph = ItemGraph::from_store(&store);
        let mut ids: Vec<_> = store
            .files
            .iter()
            .filter_map(|f| {
                f.is_valid()
                    .then(|| f.header.as_ref().map(|h| h.id.clone()))
                    .flatten()
            })
            .collect();
        ids.sort();
        ids.into_iter()
            .map(|id| inspect_with_graph(&store, &graph, &id))
            .collect()
    }

    /// Select executable items from one validated durable view.
    pub fn ready(&self) -> Result<Vec<Inspection>, OperationError> {
        let store = self.load()?;
        let graph = ItemGraph::from_store(&store);
        let evaluations = graph.ready().map_err(|error| match error {
            GraphError::Invalid(diagnostics) => OperationError::InvalidSource(diagnostics),
            GraphError::NotFound(id) => OperationError::NotFound(id),
        })?;
        evaluations
            .into_iter()
            .map(|evaluation| inspect_with_graph(&store, &graph, &evaluation.id))
            .collect()
    }

    pub fn create(
        &self,
        title: String,
        body: Vec<u8>,
        change: MetadataChange,
    ) -> Result<Inspection, OperationError> {
        self.create_with_hooks(title, body, change, |_| Ok(()), || Ok(()))
    }

    fn create_with_hooks(
        &self,
        title: String,
        body: Vec<u8>,
        change: MetadataChange,
        before_publish: impl Fn(&str) -> Result<(), OperationError>,
        after_publish: impl Fn() -> Result<(), OperationError>,
    ) -> Result<Inspection, OperationError> {
        let lock = self.lock()?;
        let items = self.items_dir(&lock)?;
        let mut store = self.load_from_dir(&items)?;
        require_valid(&store)?;
        if std::str::from_utf8(&body).is_err() {
            return Err(OperationError::InvalidArgument("body must be UTF-8".into()));
        }
        for _ in 0..16 {
            let id = new_id()?;
            if store.files.iter().any(|f| {
                f.path.file_name().and_then(|n| n.to_str()) == Some(format!("{id}.md").as_str())
            }) {
                continue;
            }
            let header = apply_change(default_header(id.clone(), title.clone()), change.clone());
            let raw = serialize(&header, &body);
            let candidate = candidate_store(&store, &self.item_path(&id), raw.clone())?;
            require_candidate(&candidate)?;
            let staged = self.stage_with_mode(&items, &raw, None)?;
            if let Err(error) = self.check_snapshot(&lock, &items, &store) {
                remove_stage(&items, &staged);
                return Err(error);
            }
            if let Err(error) = before_publish(&id) {
                remove_stage(&items, &staged);
                return Err(error);
            }
            let path = self.item_path(&id);
            match rename_in_dir(&items, &staged, &path, RenameFlags::NOREPLACE) {
                Ok(()) => {
                    return (|| {
                        after_publish()?;
                        self.sync_items(&items)?;
                        self.ensure_selected_dir(&lock, &items)?;
                        self.inspect_valid_from_dir(&items, &id)
                    })()
                    .map_err(|cause| OperationError::Published {
                        id,
                        path,
                        previous_source_path: None,
                        cause: Box::new(cause),
                    });
                }
                Err(error) if error == rustix::io::Errno::EXIST => {
                    remove_stage(&items, &staged);
                    store = self.load_from_dir(&items)?;
                    require_valid(&store)?;
                    continue;
                }
                Err(error) => {
                    remove_stage(&items, &staged);
                    return Err(OperationError::Io(io::Error::from(error)));
                }
            }
        }
        Err(OperationError::Conflict(
            "could not reserve a new item ID".into(),
        ))
    }

    pub fn update(&self, id: &str, change: MetadataChange) -> Result<Inspection, OperationError> {
        self.mutate(id, |_, header| {
            *header = apply_change(header.clone(), change);
            Ok(())
        })
    }

    pub fn relation_add(
        &self,
        source: &str,
        kind: RelationKind,
        target: &str,
    ) -> Result<Inspection, OperationError> {
        self.relate(source, kind, target, true)
    }

    pub fn relation_remove(
        &self,
        source: &str,
        kind: RelationKind,
        target: &str,
    ) -> Result<Inspection, OperationError> {
        self.relate(source, kind, target, false)
    }

    pub fn close(&self, id: &str, reason: Option<String>) -> Result<Inspection, OperationError> {
        self.mutate(id, |_, header| {
            if header.completion != Completion::Manual {
                return Err(OperationError::InvalidArgument(
                    "aggregate cannot be closed".into(),
                ));
            }
            header.state = Some(ManualState::Done);
            header.close_reason = reason;
            Ok(())
        })
    }

    pub fn reopen(&self, id: &str) -> Result<Inspection, OperationError> {
        self.mutate(id, |_, header| {
            if header.completion != Completion::Manual {
                return Err(OperationError::InvalidArgument(
                    "aggregate cannot be reopened".into(),
                ));
            }
            header.state = Some(ManualState::Open);
            header.close_reason = None;
            Ok(())
        })
    }

    fn relate(
        &self,
        source: &str,
        kind: RelationKind,
        target: &str,
        add: bool,
    ) -> Result<Inspection, OperationError> {
        if !valid_id(target) {
            return Err(OperationError::InvalidArgument(
                "target must be a full UUIDv4 ID".into(),
            ));
        }
        if kind == RelationKind::Related && !add {
            if !valid_id(source) {
                return Err(OperationError::InvalidArgument(
                    "source must be a full UUIDv4 ID".into(),
                ));
            }
            return self.mutate_selected_with_hook(
                |store| {
                    let authored_at = |id: &str, other: &str| {
                        store.files.iter().any(|file| {
                            file.header.as_ref().is_some_and(|header| {
                                header.id == id && header.related.iter().any(|edge| edge == other)
                            })
                        })
                    };
                    if authored_at(source, target) {
                        Ok(source.to_owned())
                    } else if authored_at(target, source) {
                        Ok(target.to_owned())
                    } else {
                        Err(OperationError::NotFound("edge".into()))
                    }
                },
                |_, header| {
                    let other = if header.id == source { target } else { source };
                    let index = header
                        .related
                        .iter()
                        .position(|edge| edge == other)
                        .ok_or_else(|| OperationError::NotFound("edge".into()))?;
                    header.related.remove(index);
                    Ok(())
                },
                || Ok(()),
            );
        }
        self.mutate(source, |store, header| {
            if source == target {
                return Err(OperationError::InvalidArgument("self relationship".into()));
            }
            match kind {
                RelationKind::Parent => {
                    if add {
                        if header.parent.is_some() {
                            return Err(OperationError::AlreadyExists("parent already set".into()));
                        }
                        header.parent = Some(target.into());
                    } else if header.parent.as_deref() == Some(target) {
                        header.parent = None;
                    } else {
                        return Err(OperationError::NotFound("parent edge".into()));
                    }
                }
                _ => {
                    if add
                        && kind == RelationKind::Related
                        && ItemGraph::from_store(store)
                            .relations(source)
                            .is_ok_and(|r| r.related.contains(&target.to_string()))
                    {
                        return Err(OperationError::AlreadyExists(
                            "related edge already exists".into(),
                        ));
                    }
                    let edges = match kind {
                        RelationKind::DependsOn => &mut header.depends_on,
                        RelationKind::Related => &mut header.related,
                        RelationKind::DiscoveredFrom => &mut header.discovered_from,
                        RelationKind::Parent => unreachable!(),
                    };
                    if add {
                        if edges.iter().any(|id| id == target) {
                            return Err(OperationError::AlreadyExists(
                                "edge already exists".into(),
                            ));
                        }
                        edges.push(target.into());
                    } else if let Some(index) = edges.iter().position(|id| id == target) {
                        edges.remove(index);
                    } else {
                        return Err(OperationError::NotFound("edge".into()));
                    }
                }
            }
            Ok(())
        })
    }

    fn mutate(
        &self,
        id: &str,
        edit: impl FnOnce(&ItemStore, &mut ItemHeader) -> Result<(), OperationError>,
    ) -> Result<Inspection, OperationError> {
        self.mutate_with_hook(id, edit, || Ok(()))
    }

    fn mutate_with_hook(
        &self,
        id: &str,
        edit: impl FnOnce(&ItemStore, &mut ItemHeader) -> Result<(), OperationError>,
        before_publish: impl FnOnce() -> Result<(), OperationError>,
    ) -> Result<Inspection, OperationError> {
        if !valid_id(id) {
            return Err(OperationError::InvalidArgument(
                "source must be a full UUIDv4 ID".into(),
            ));
        }
        self.mutate_selected_with_hook(|_| Ok(id.to_owned()), edit, before_publish)
    }

    fn mutate_selected_with_hook(
        &self,
        choose: impl FnOnce(&ItemStore) -> Result<String, OperationError>,
        edit: impl FnOnce(&ItemStore, &mut ItemHeader) -> Result<(), OperationError>,
        before_publish: impl FnOnce() -> Result<(), OperationError>,
    ) -> Result<Inspection, OperationError> {
        let lock = self.lock()?;
        let items = self.items_dir(&lock)?;
        let store = self.load_from_dir(&items)?;
        require_valid(&store)?;
        let id = choose(&store)?;
        let source = store
            .files
            .iter()
            .find(|f| f.header.as_ref().is_some_and(|h| h.id == id))
            .ok_or_else(|| OperationError::NotFound(id.clone()))?;
        let mut header = source.header.clone().expect("valid store header");
        edit(&store, &mut header)?;
        let raw = serialize(&header, source.body.as_deref().expect("valid store body"));
        let candidate = candidate_store(&store, &source.path, raw.clone())?;
        require_candidate(&candidate)?;
        let staged = self.stage_with_mode(&items, &raw, Some(source_mode(source)?))?;
        if let Err(error) = before_publish() {
            remove_stage(&items, &staged);
            return Err(error);
        }
        if let Err(error) = self.check_snapshot(&lock, &items, &store) {
            remove_stage(&items, &staged);
            return Err(error);
        }
        // Exchange retains the previous file at the staging path for recovery.
        let published = fingerprint_at(&items, &staged)?;
        if let Err(error) = rename_in_dir(&items, &staged, &source.path, RenameFlags::EXCHANGE) {
            remove_stage(&items, &staged);
            return Err(exchange_error(error));
        }
        self.verify_exchange(&items, &store, source, &staged, &raw, &published)?;
        self.sync_items(&items)
            .map_err(|error| with_recovery(error, &staged))?;
        self.ensure_selected_dir_after_publication(&lock, &items, &staged)?;
        let mut result = self
            .inspect_valid_from_dir(&items, &id)
            .map_err(|error| with_recovery(error, &staged))?;
        result.recovery_path = Some(staged);
        Ok(result)
    }

    fn load(&self) -> Result<ItemStore, OperationError> {
        Ok(ItemStore::load_from_root(&self.root)?)
    }
    fn items_dir(&self, lock: &OperationLock) -> Result<OwnedFd, OperationError> {
        let items = openat(
            &lock.work_dir,
            "items",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
        self.ensure_selected_dir(lock, &items)?;
        Ok(items)
    }
    fn load_from_dir(&self, items: &OwnedFd) -> Result<ItemStore, OperationError> {
        Ok(ItemStore::load_from_open_items(
            items,
            &self.root.join(".work/items"),
        )?)
    }
    fn ensure_selected_dir(
        &self,
        lock: &OperationLock,
        items: &OwnedFd,
    ) -> Result<(), OperationError> {
        let root = fs::symlink_metadata(&self.root)
            .map_err(|_| OperationError::Conflict("checkout root changed on disk".into()))?;
        let held_root = rustix::fs::fstat(&lock.root_dir).map_err(io::Error::from)?;
        if !root.is_dir()
            || root.dev() != held_root.st_dev as u64
            || root.ino() != held_root.st_ino as u64
        {
            return Err(OperationError::Conflict(
                "checkout root changed on disk".into(),
            ));
        }
        let work = fs::symlink_metadata(self.root.join(".work"))
            .map_err(|_| OperationError::Conflict("work directory changed on disk".into()))?;
        let held_work = rustix::fs::fstat(&lock.work_dir).map_err(io::Error::from)?;
        if !work.is_dir()
            || work.dev() != held_work.st_dev as u64
            || work.ino() != held_work.st_ino as u64
        {
            return Err(OperationError::Conflict(
                "work directory changed on disk".into(),
            ));
        }
        let selected_lock = statat(&lock.work_dir, "operations.lock", AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| OperationError::Conflict("operation lock changed on disk".into()))?;
        let held_lock = rustix::fs::fstat(&lock.file).map_err(io::Error::from)?;
        if selected_lock.st_mode as u32 & 0o170000 != 0o100000
            || selected_lock.st_dev != held_lock.st_dev
            || selected_lock.st_ino != held_lock.st_ino
        {
            return Err(OperationError::Conflict(
                "operation lock changed on disk".into(),
            ));
        }
        let selected = fs::symlink_metadata(self.root.join(".work/items"))
            .map_err(|_| OperationError::Conflict("items directory changed on disk".into()))?;
        let held = rustix::fs::fstat(items).map_err(io::Error::from)?;
        if !selected.is_dir()
            || selected.dev() != held.st_dev as u64
            || selected.ino() != held.st_ino as u64
        {
            return Err(OperationError::Conflict(
                "items directory changed on disk".into(),
            ));
        }
        Ok(())
    }
    fn ensure_selected_dir_after_publication(
        &self,
        lock: &OperationLock,
        items: &OwnedFd,
        recovery: &Path,
    ) -> Result<(), OperationError> {
        self.ensure_selected_dir(lock, items).map_err(|_| {
            OperationError::Conflict(format!(
                "selected path or lock changed after publication; recovery copy {} remains in the originally opened items directory (former path {})",
                entry_name(recovery).to_string_lossy(),
                recovery.display()
            ))
        })
    }
    fn inspect_raw_from_dir(
        &self,
        items: &OwnedFd,
        id: &str,
    ) -> Result<RawInspection, OperationError> {
        let store = self.load_from_dir(items)?;
        let file = self.raw_source(&store, id)?.clone();
        Ok(RawInspection {
            file,
            graph_diagnostics: ItemGraph::from_store(&store).diagnostics().to_vec(),
            recovery_path: None,
            additional_recovery_paths: Vec::new(),
        })
    }
    fn inspect_valid_from_dir(
        &self,
        items: &OwnedFd,
        id: &str,
    ) -> Result<Inspection, OperationError> {
        let store = self.load_from_dir(items)?;
        require_valid(&store)?;
        inspect_store(&store, id)
    }
    fn archive_shadow(
        &self,
        lock: &OperationLock,
        items: &OwnedFd,
        source: &ItemFile,
    ) -> Result<PathBuf, OperationError> {
        for _ in 0..16 {
            let recovery = self
                .root
                .join(".work/items")
                .join(format!(".operation-{}", new_id()?));
            match rename_in_dir(items, &source.path, &recovery, RenameFlags::NOREPLACE) {
                Ok(()) => {
                    self.sync_items(items)
                        .map_err(|error| with_recovery(error, &recovery))?;
                    if !archive_matches(items, source, &recovery) {
                        return Err(OperationError::Conflict(format!(
                            "shadowed source changed during repair; recovery copy retained at {}",
                            recovery.display()
                        )));
                    }
                    self.ensure_selected_dir_after_publication(lock, items, &recovery)?;
                    return Ok(recovery);
                }
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(exchange_error(error)),
            }
        }
        Err(OperationError::Conflict(
            "could not reserve shadowed source recovery path".into(),
        ))
    }
    fn item_path(&self, id: &str) -> PathBuf {
        self.root.join(".work/items").join(format!("{id}.md"))
    }
    fn raw_source<'a>(
        &self,
        store: &'a ItemStore,
        id: &str,
    ) -> Result<&'a ItemFile, OperationError> {
        if let Some(file) = self.interrupted_filename_source(store, id) {
            return Ok(file);
        }
        if let Some(file) = store.files.iter().find(|f| f.path == self.item_path(id)) {
            return Ok(file);
        }
        let matches: Vec<_> = store
            .files
            .iter()
            .filter(|f| f.header.as_ref().is_some_and(|h| h.id == id))
            .collect();
        match matches.as_slice() {
            [file] => Ok(*file),
            [] => Err(OperationError::NotFound(id.into())),
            _ => Err(OperationError::Conflict(format!(
                "multiple source files claim item {id}"
            ))),
        }
    }
    fn interrupted_filename_source<'a>(
        &self,
        store: &'a ItemStore,
        id: &str,
    ) -> Option<&'a ItemFile> {
        let matching: Vec<_> = store
            .files
            .iter()
            .filter(|file| file.header.as_ref().is_some_and(|header| header.id == id))
            .collect();
        let [first, second] = matching.as_slice() else {
            return None;
        };
        let target = self.item_path(id);
        let (canonical, old) = if first.path == target {
            (*first, *second)
        } else if second.path == target {
            (*second, *first)
        } else {
            return None;
        };
        if canonical
            .diagnostics
            .iter()
            .all(|d| d.message == format!("duplicate item ID {id}"))
            && old.path != target
            && old
                .diagnostics
                .iter()
                .any(|d| d.message == format!("filename must be {id}.md"))
        {
            Some(old)
        } else {
            None
        }
    }
    fn resume_filename_repair(
        &self,
        lock: &OperationLock,
        items: &OwnedFd,
        id: &str,
        raw: &[u8],
        source: (&ItemStore, &ItemFile),
        before_archive: impl FnOnce() -> Result<(), OperationError>,
    ) -> Result<RawInspection, OperationError> {
        let (store, old) = source;
        let canonical = store
            .files
            .iter()
            .find(|file| file.path == self.item_path(id))
            .ok_or_else(|| OperationError::Conflict("canonical item disappeared".into()))?;
        if raw != canonical.raw {
            return Err(OperationError::Conflict(
                "retry source differs from the already published canonical item".into(),
            ));
        }
        (|| {
            let before = ItemGraph::from_store(store);
            let candidate = ItemStore::from_candidate_files(
                store
                    .files
                    .iter()
                    .filter(|file| file.path != old.path)
                    .cloned()
                    .collect(),
            );
            let new_diagnostics: Vec<_> = ItemGraph::from_store(&candidate)
                .diagnostics()
                .iter()
                .filter(|diagnostic| !before.diagnostics().contains(diagnostic))
                .cloned()
                .collect();
            if !new_diagnostics.is_empty() {
                return Err(OperationError::InvalidCandidate(new_diagnostics));
            }
            self.check_snapshot(lock, items, store)?;
            before_archive()?;
            for _ in 0..16 {
                let recovery = self
                    .root
                    .join(".work/items")
                    .join(format!(".operation-{}", new_id()?));
                match rename_in_dir(items, &old.path, &recovery, RenameFlags::NOREPLACE) {
                    Ok(()) => {
                        self.sync_items(items)
                            .map_err(|error| with_recovery(error, &recovery))?;
                        if !archive_matches(items, old, &recovery) {
                            return Err(OperationError::Conflict(format!(
                                "source changed during filename repair; recovery copy retained at {}",
                                recovery.display()
                            )));
                        }
                        self.ensure_selected_dir_after_publication(lock, items, &recovery)?;
                        let mut result = self
                            .inspect_raw_from_dir(items, id)
                            .map_err(|error| with_recovery(error, &recovery))?;
                        result.recovery_path = Some(recovery);
                        return Ok(result);
                    }
                    Err(rustix::io::Errno::EXIST) => continue,
                    Err(error) => return Err(OperationError::Io(io::Error::from(error))),
                }
            }
            Err(OperationError::Conflict(
                "could not reserve filename repair recovery path".into(),
            ))
        })()
        .map_err(|cause| OperationError::Published {
            id: id.to_owned(),
            path: canonical.path.clone(),
            previous_source_path: Some(old.path.clone()),
            cause: Box::new(cause),
        })
    }
    fn lock(&self) -> Result<OperationLock, OperationError> {
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let root_dir =
            openat(CWD, &self.root, directory_flags, Mode::empty()).map_err(io::Error::from)?;
        let work_dir =
            openat(&root_dir, ".work", directory_flags, Mode::empty()).map_err(io::Error::from)?;
        let file = File::from(
            openat(
                &work_dir,
                "operations.lock",
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(io::Error::from)?,
        );
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        flock(&file, FlockOperation::LockExclusive).map_err(io::Error::from)?;
        Ok(OperationLock {
            file,
            root_dir,
            work_dir,
        })
    }
    #[cfg(test)]
    fn stage(&self, raw: &[u8]) -> Result<PathBuf, OperationError> {
        self.stage_with_mode(&self.items_dir(&self.lock()?)?, raw, None)
    }
    fn stage_with_mode(
        &self,
        items: &OwnedFd,
        raw: &[u8],
        mode: Option<u32>,
    ) -> Result<PathBuf, OperationError> {
        for _ in 0..16 {
            let path = self
                .root
                .join(".work/items")
                .join(format!(".operation-{}", new_id()?));
            let name = path.file_name().expect("staging file name");
            match openat(
                items,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => {
                    let mut file = File::from(fd);
                    let result = file
                        .set_permissions(fs::Permissions::from_mode(0o600))
                        .and_then(|_| file.write_all(raw))
                        .and_then(|_| {
                            if let Some(mode) = mode {
                                file.set_permissions(fs::Permissions::from_mode(mode))?;
                            }
                            file.sync_all()
                        });
                    if let Err(error) = result {
                        drop(file);
                        let _ = unlinkat(items, name, AtFlags::empty());
                        return Err(OperationError::Io(error));
                    }
                    return Ok(path);
                }
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(OperationError::Io(io::Error::from(error))),
            }
        }
        Err(OperationError::Conflict(
            "could not reserve staging path".into(),
        ))
    }
    fn check_snapshot(
        &self,
        lock: &OperationLock,
        items: &OwnedFd,
        store: &ItemStore,
    ) -> Result<(), OperationError> {
        self.ensure_selected_dir(lock, items)?;
        let current = self.load_from_dir(items)?;
        self.ensure_selected_dir(lock, items)?;
        if current.files.len() != store.files.len()
            || current
                .files
                .iter()
                .zip(&store.files)
                .any(|(a, b)| a.path != b.path || a.raw != b.raw || a.fingerprint != b.fingerprint)
        {
            return Err(OperationError::Conflict("items changed on disk".into()));
        }
        Ok(())
    }
    fn verify_exchange(
        &self,
        items: &OwnedFd,
        before: &ItemStore,
        source: &ItemFile,
        staged: &Path,
        new_raw: &[u8],
        published: &FileFingerprint,
    ) -> Result<(), OperationError> {
        self.verify_exchange_with_hook(items, before, source, staged, (new_raw, published), || {})
    }

    fn verify_exchange_with_hook(
        &self,
        items: &OwnedFd,
        before: &ItemStore,
        source: &ItemFile,
        staged: &Path,
        publication: (&[u8], &FileFingerprint),
        before_rollback: impl FnOnce(),
    ) -> Result<(), OperationError> {
        let (new_raw, published) = publication;
        let old_matches = fingerprint_at(items, staged).is_ok_and(|now| {
            source
                .fingerprint
                .as_ref()
                .is_some_and(|prior| prior.same_identity_and_mode(&now))
                && if now.mode & 0o170000 != 0o100000 {
                    source.diagnostics.iter().any(|d| {
                        d.message.contains("must not be a symlink")
                            || d.message.contains("must be a regular file")
                    })
                } else {
                    read_at(items, staged).is_ok_and(|raw| raw == source.raw)
                }
        });
        let current = self.load_from_dir(items);
        let stable = current.as_ref().is_ok_and(|now| {
            now.files.len() == before.files.len()
                && before.files.iter().all(|prior| {
                    now.files
                        .iter()
                        .find(|file| file.path == prior.path)
                        .is_some_and(|file| {
                            if file.path == source.path {
                                file.raw == new_raw
                                    && file.fingerprint.as_ref().is_some_and(|current| {
                                        published.same_identity_and_mode(current)
                                    })
                            } else {
                                file.raw == prior.raw && file.fingerprint == prior.fingerprint
                            }
                        })
                })
        });
        if old_matches && stable {
            return Ok(());
        }
        let published_untouched = fingerprint_at(items, &source.path).is_ok_and(|current| {
            published.same_identity_and_mode(&current)
                && read_at(items, &source.path).is_ok_and(|raw| raw == new_raw)
        });
        if !published_untouched {
            return Err(OperationError::Conflict(format!(
                "item changed after publication; previous source retained at {}",
                staged.display()
            )));
        }
        before_rollback();
        rename_in_dir(items, staged, &source.path, RenameFlags::EXCHANGE).map_err(|e| {
            OperationError::Conflict(format!(
                "publication conflict; rollback failed: {e}; recovery copy retained at {}",
                staged.display()
            ))
        })?;
        self.sync_items(items).map_err(|e| {
            OperationError::Io(io::Error::other(format!(
                "{e}; recovery copy retained at {}",
                staged.display()
            )))
        })?;
        Err(OperationError::Conflict(format!(
            "items changed during publication; recovery copy retained at {}",
            staged.display()
        )))
    }
    fn sync_items(&self, items: &OwnedFd) -> Result<(), OperationError> {
        fsync(items).map_err(io::Error::from)?;
        Ok(())
    }
}

pub(crate) fn inspect_store(store: &ItemStore, id: &str) -> Result<Inspection, OperationError> {
    let graph = ItemGraph::from_store(store);
    inspect_with_graph(store, &graph, id)
}

fn inspect_with_graph(
    store: &ItemStore,
    graph: &ItemGraph,
    id: &str,
) -> Result<Inspection, OperationError> {
    if !valid_id(id) {
        return Err(OperationError::InvalidArgument(
            "item must be a full UUIDv4 ID".into(),
        ));
    }
    let file = store
        .files
        .iter()
        .find(|f| f.header.as_ref().is_some_and(|h| h.id == id))
        .ok_or_else(|| OperationError::NotFound(id.into()))?
        .clone();
    let relations = graph
        .relations(id)
        .map_err(|_| OperationError::NotFound(id.into()))?;
    let evaluation = if graph.is_valid() {
        graph.evaluate(id).ok()
    } else {
        None
    };
    Ok(Inspection {
        context: serde_json::json!({"source_worktree": super::coordination::encode_path(file.path.parent().unwrap().parent().unwrap().parent().unwrap()), "persistence":"material","run_id":null,"claim":null}),
        file,
        relations,
        evaluation,
        graph_diagnostics: graph.diagnostics().to_vec(),
        recovery_path: None,
    })
}

fn require_valid(store: &ItemStore) -> Result<(), OperationError> {
    let graph = ItemGraph::from_store(store);
    if graph.is_valid() {
        Ok(())
    } else {
        Err(OperationError::InvalidSource(graph.diagnostics().to_vec()))
    }
}
fn require_candidate(store: &ItemStore) -> Result<(), OperationError> {
    let graph = ItemGraph::from_store(store);
    if graph.is_valid() {
        Ok(())
    } else {
        Err(OperationError::InvalidCandidate(
            graph.diagnostics().to_vec(),
        ))
    }
}
fn candidate_store(
    store: &ItemStore,
    path: &Path,
    raw: Vec<u8>,
) -> Result<ItemStore, OperationError> {
    let mut candidate = store.clone();
    let parsed = parse_candidate(path.into(), raw);
    if !parsed.is_valid() {
        return Err(OperationError::InvalidArgument(
            parsed
                .diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    if let Some(file) = candidate.files.iter_mut().find(|f| f.path == path) {
        *file = parsed;
    } else {
        candidate.files.push(parsed);
    }
    Ok(ItemStore::from_candidate_files(candidate.files))
}

pub(crate) fn default_header(id: String, title: String) -> ItemHeader {
    ItemHeader {
        id,
        title,
        completion: Completion::Manual,
        state: Some(ManualState::Open),
        priority: 2,
        parent: None,
        depends_on: vec![],
        related: vec![],
        discovered_from: vec![],
        labels: vec![],
        model: None,
        thinking: None,
        close_reason: None,
    }
}
pub(crate) fn apply_change(mut h: ItemHeader, c: MetadataChange) -> ItemHeader {
    if let Some(v) = c.title {
        h.title = v;
    }
    if let Some(v) = c.completion
        && h.completion != v
    {
        h.completion = v;
        h.state = if v == Completion::Children {
            None
        } else {
            Some(ManualState::Open)
        };
        h.close_reason = None;
    }
    if let Some(v) = c.priority {
        h.priority = v;
    }
    if let Some(v) = c.parent {
        h.parent = v;
    }
    if let Some(v) = c.depends_on {
        h.depends_on = v;
    }
    if let Some(v) = c.related {
        h.related = v;
    }
    if let Some(v) = c.discovered_from {
        h.discovered_from = v;
    }
    if let Some(v) = c.labels {
        h.labels = v;
    }
    if let Some(v) = c.model {
        h.model = v;
    }
    if let Some(v) = c.thinking {
        h.thinking = v;
    }
    h
}

pub(crate) fn serialize(h: &ItemHeader, body: &[u8]) -> Vec<u8> {
    let mut text = format!(
        "---\nformat_version: 1\nid: {}\ntitle: {}\n",
        quote(&h.id),
        quote(&h.title)
    );
    text.push_str(match h.completion {
        Completion::Manual => "completion: manual\n",
        Completion::Children => "completion: children\n",
    });
    if let Some(state) = h.state {
        text.push_str(match state {
            ManualState::Open => "state: open\n",
            ManualState::Done => "state: done\n",
        });
    }
    text.push_str(&format!("priority: {}\n", h.priority));
    if let Some(v) = &h.parent {
        text.push_str(&format!("parent: {}\n", quote(v)));
    }
    for (name, values) in [
        ("depends_on", &h.depends_on),
        ("related", &h.related),
        ("discovered_from", &h.discovered_from),
        ("labels", &h.labels),
    ] {
        if !values.is_empty() {
            text.push_str(&format!(
                "{name}: [{}]\n",
                values
                    .iter()
                    .map(|v| quote(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    for (name, value) in [
        ("model", &h.model),
        ("thinking", &h.thinking),
        ("close_reason", &h.close_reason),
    ] {
        if let Some(v) = value {
            text.push_str(&format!("{name}: {}\n", quote(v)));
        }
    }
    text.push_str("---\n");
    let mut raw = text.into_bytes();
    raw.extend_from_slice(body);
    raw
}
fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn valid_id(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 32
        && b.iter()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && b[12] == b'4'
        && matches!(b[16], b'8' | b'9' | b'a' | b'b')
}
fn source_mode(file: &ItemFile) -> Result<u32, OperationError> {
    let fingerprint = file.fingerprint.as_ref().ok_or_else(|| {
        OperationError::Conflict(format!("could not snapshot item {}", file.path.display()))
    })?;
    if fingerprint.mode & 0o170000 == 0o100000 && fingerprint.mode & 0o400 == 0 {
        return Err(OperationError::InvalidArgument(format!(
            "item {} has no owner-read permission; make it readable before repair",
            file.path.display()
        )));
    }
    Ok(if fingerprint.mode & 0o170000 == 0o100000 {
        fingerprint.mode & 0o7777
    } else {
        0o600
    })
}
fn archive_matches(items: &OwnedFd, original: &ItemFile, archived_path: &Path) -> bool {
    fingerprint_at(items, archived_path).is_ok_and(|current| {
        original
            .fingerprint
            .as_ref()
            .is_some_and(|prior| prior.same_identity_and_mode(&current))
            && if current.mode & 0o170000 != 0o100000 {
                true
            } else {
                read_at(items, archived_path).is_ok_and(|bytes| bytes == original.raw)
            }
    })
}
fn entry_name(path: &Path) -> &std::ffi::OsStr {
    path.file_name().expect("item entry name")
}
fn fingerprint_at(items: &OwnedFd, path: &Path) -> io::Result<FileFingerprint> {
    FileFingerprint::capture_at(items, entry_name(path))
}
fn read_at(items: &OwnedFd, path: &Path) -> io::Result<Vec<u8>> {
    let fd = openat(
        items,
        entry_name(path),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;
    let mut file = File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "item is not regular",
        ));
    }
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)?;
    Ok(raw)
}
fn rename_in_dir(
    items: &OwnedFd,
    source: &Path,
    target: &Path,
    flags: RenameFlags,
) -> rustix::io::Result<()> {
    renameat_with(items, entry_name(source), items, entry_name(target), flags)
}
fn remove_stage(items: &OwnedFd, path: &Path) {
    let _ = unlinkat(items, entry_name(path), AtFlags::empty());
}
fn exchange_error(error: rustix::io::Errno) -> OperationError {
    if error == rustix::io::Errno::NOENT {
        OperationError::Conflict("item changed during publication".into())
    } else {
        OperationError::Io(io::Error::from(error))
    }
}
fn with_recovery(error: OperationError, path: &Path) -> OperationError {
    let context = format!("; recovery copy retained at {}", path.display());
    match error {
        OperationError::Published {
            id,
            path: published_path,
            previous_source_path,
            cause,
        } => OperationError::Published {
            id,
            path: published_path,
            previous_source_path,
            cause: Box::new(with_recovery(*cause, path)),
        },
        OperationError::Io(inner) => {
            OperationError::Io(io::Error::other(format!("{inner}{context}")))
        }
        OperationError::Conflict(message) => {
            OperationError::Conflict(format!("{message}{context}"))
        }
        other => OperationError::Conflict(format!("{other}{context}")),
    }
}
pub(crate) fn new_id() -> Result<String, OperationError> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Coordinator-owned checkout lock and publication primitives. The caller has
/// already validated the complete resolved graph under the common lock.
pub(crate) struct CheckoutWriter {
    ops: DurableOperations,
    lock: OperationLock,
    items: OwnedFd,
    snapshot: ItemStore,
}
impl CheckoutWriter {
    pub(crate) fn open(root: &Path) -> Result<Self, OperationError> {
        let ops = DurableOperations::new(root);
        let lock = ops.lock()?;
        let items = ops.items_dir(&lock)?;
        let snapshot = ops.load_from_dir(&items)?;
        Ok(Self {
            ops,
            lock,
            items,
            snapshot,
        })
    }
    pub(crate) fn publish(
        &mut self,
        header: &ItemHeader,
        body: &[u8],
        expected: Option<&ItemFile>,
    ) -> Result<Option<PathBuf>, OperationError> {
        let path = self.ops.item_path(&header.id);
        let raw = serialize(header, body);
        self.ops
            .check_snapshot(&self.lock, &self.items, &self.snapshot)?;
        if let Some(old) = expected {
            if old.path != path
                || !self
                    .snapshot
                    .files
                    .iter()
                    .any(|f| f.path == path && f.raw == old.raw && f.fingerprint == old.fingerprint)
            {
                return Err(OperationError::Conflict(
                    "resolved source changed before checkout write".into(),
                ));
            }
        } else if self.snapshot.files.iter().any(|f| f.path == path) {
            return Err(OperationError::AlreadyExists(header.id.clone()));
        }
        let staged =
            self.ops
                .stage_with_mode(&self.items, &raw, expected.map(source_mode).transpose()?)?;
        let staged_identity = fingerprint_at(&self.items, &staged)?;
        self.ops
            .check_snapshot(&self.lock, &self.items, &self.snapshot)?;
        rename_in_dir(
            &self.items,
            &staged,
            &path,
            if expected.is_some() {
                RenameFlags::EXCHANGE
            } else {
                RenameFlags::NOREPLACE
            },
        )
        .map_err(exchange_error)?;
        (|| {
            if let Some(old) = expected {
                self.ops.verify_exchange(
                    &self.items,
                    &self.snapshot,
                    old,
                    &staged,
                    &raw,
                    &staged_identity,
                )?;
            }
            self.ops.sync_items(&self.items)?;
            self.ops.ensure_selected_dir(&self.lock, &self.items)?;
            self.snapshot = self.ops.load_from_dir(&self.items)?;
            Ok(expected.map(|_| staged.clone()))
        })()
        .map_err(|cause| OperationError::Published {
            id: header.id.clone(),
            path,
            previous_source_path: expected.map(|_| staged),
            cause: Box::new(cause),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, String) {
        let root = std::env::temp_dir().join(format!("work-operation-test-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        let id = "d66b0ba51d2c4a7aa15de40cb3c9d507".to_string();
        let raw = serialize(&default_header(id.clone(), "Original".into()), b"Body");
        fs::write(root.join(".work/items").join(format!("{id}.md")), raw).unwrap();
        (root, id)
    }

    fn staged_count(root: &Path) -> usize {
        fs::read_dir(root.join(".work/items"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".operation-"))
            .count()
    }

    #[test]
    fn changed_on_disk_before_publication_returns_conflict_and_keeps_external_edit() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let mut external = fs::read(&path).unwrap();
        external.extend_from_slice(b" external");
        let result = ops.mutate_with_hook(
            &id,
            |_, h| {
                h.title = "Changed".into();
                Ok(())
            },
            || {
                fs::write(&path, &external)?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(OperationError::Conflict(_))));
        assert_eq!(fs::read(path).unwrap(), external);
        assert_eq!(staged_count(&root), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_mode_before_publication_returns_conflict_and_keeps_external_mode() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let original = fs::read(&path).unwrap();
        let changed_mode =
            (fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o7777) ^ 0o100;
        let result = ops.mutate_with_hook(
            &id,
            |_, h| {
                h.title = "Changed".into();
                Ok(())
            },
            || {
                fs::set_permissions(&path, fs::Permissions::from_mode(changed_mode))?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(OperationError::Conflict(_))));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o7777,
            changed_mode
        );
        assert_eq!(staged_count(&root), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_swap_before_publication_does_not_write_outside_checkout() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let original = fs::read(ops.item_path(&id)).unwrap();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let outside_item = outside.join(format!("{id}.md"));
        fs::write(&outside_item, b"outside content").unwrap();
        let items = root.join(".work/items");
        let held = root.join(".work/held");
        let result = ops.mutate_with_hook(
            &id,
            |_, header| {
                header.title = "Changed".into();
                Ok(())
            },
            || {
                fs::rename(&items, &held)?;
                std::os::unix::fs::symlink(&outside, &items)?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(OperationError::Conflict(_))));
        assert_eq!(fs::read(outside_item).unwrap(), b"outside content");
        assert_eq!(fs::read(held.join(format!("{id}.md"))).unwrap(), original);
        assert!(
            !fs::read_dir(held)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".operation-"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn held_directory_exchange_cannot_follow_a_replaced_directory_path() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let items = ops.items_dir(&ops.lock().unwrap()).unwrap();
        let target = ops.item_path(&id);
        let staged = ops.stage_with_mode(&items, b"replacement", None).unwrap();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let outside_item = outside.join(format!("{id}.md"));
        fs::write(&outside_item, b"outside content").unwrap();
        let held = root.join(".work/held");
        fs::rename(root.join(".work/items"), &held).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".work/items")).unwrap();
        rename_in_dir(&items, &staged, &target, RenameFlags::EXCHANGE).unwrap();
        assert_eq!(fs::read(outside_item).unwrap(), b"outside content");
        assert_eq!(
            fs::read(held.join(format!("{id}.md"))).unwrap(),
            b"replacement"
        );
        let error = ops
            .ensure_selected_dir_after_publication(&ops.lock().unwrap(), &items, &staged)
            .unwrap_err();
        assert!(matches!(error, OperationError::Conflict(_)));
        assert!(
            error
                .to_string()
                .contains(entry_name(&staged).to_str().unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn items_directory_is_opened_from_the_locked_work_directory() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let lock = ops.lock().unwrap();
        let outside_work = root.join("outside-work");
        fs::create_dir_all(outside_work.join("items")).unwrap();
        let outside_item = outside_work.join("items").join(format!("{id}.md"));
        fs::write(&outside_item, b"outside content").unwrap();
        fs::rename(root.join(".work"), root.join("held-work")).unwrap();
        std::os::unix::fs::symlink(&outside_work, root.join(".work")).unwrap();
        assert!(matches!(
            ops.items_dir(&lock),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(fs::read(outside_item).unwrap(), b"outside content");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn work_path_symlink_to_held_directory_is_still_a_conflict() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let original = fs::read(ops.item_path(&id)).unwrap();
        let work = root.join(".work");
        let held = root.join("held-work");
        let result = ops.mutate_with_hook(
            &id,
            |_, header| {
                header.title = "Changed".into();
                Ok(())
            },
            || {
                fs::rename(&work, &held)?;
                std::os::unix::fs::symlink(&held, &work)?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(OperationError::Conflict(_))));
        assert_eq!(
            fs::read(held.join("items").join(format!("{id}.md"))).unwrap(),
            original
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replaced_lock_file_is_rejected_before_publication() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let original = fs::read(&path).unwrap();
        let lock_path = root.join(".work/operations.lock");
        let result = ops.mutate_with_hook(
            &id,
            |_, header| {
                header.title = "Changed".into();
                Ok(())
            },
            || {
                fs::rename(&lock_path, root.join(".work/old-lock"))?;
                fs::write(&lock_path, b"")?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(OperationError::Conflict(_))));
        assert_eq!(fs::read(path).unwrap(), original);
        assert_eq!(staged_count(&root), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symlink_checkout_root_is_rejected_for_reads_and_writes() {
        let (root, _id) = fixture();
        let alias = root.with_extension("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        for selected in [
            alias.clone(),
            PathBuf::from(format!("{}/", alias.display())),
            alias.join("."),
        ] {
            let ops = DurableOperations::new(selected);
            assert!(matches!(ops.list(), Err(OperationError::Io(_))));
            assert!(matches!(
                ops.create("New".into(), Vec::new(), MetadataChange::default()),
                Err(OperationError::Io(_))
            ));
        }
        fs::remove_file(alias).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn related_removal_chooses_author_after_acquiring_the_lock() {
        let (root, first) = fixture();
        let second = "e66b0ba51d2c4a7aa15de40cb3c9d507".to_string();
        let ops = DurableOperations::new(&root);
        let mut first_header = default_header(first.clone(), "First".into());
        let mut second_header = default_header(second.clone(), "Second".into());
        first_header.related.push(second.clone());
        fs::write(
            ops.item_path(&first),
            serialize(&first_header, b"First body"),
        )
        .unwrap();
        fs::write(
            ops.item_path(&second),
            serialize(&second_header, b"Second body"),
        )
        .unwrap();
        let lock = ops.lock().unwrap();
        let (ready, started) = std::sync::mpsc::channel();
        let thread_root = root.clone();
        let thread_first = first.clone();
        let thread_second = second.clone();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            DurableOperations::new(thread_root).relation_remove(
                &thread_second,
                RelationKind::Related,
                &thread_first,
            )
        });
        started.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        first_header.related.clear();
        second_header.related.push(first.clone());
        fs::write(
            ops.item_path(&first),
            serialize(&first_header, b"First body"),
        )
        .unwrap();
        fs::write(
            ops.item_path(&second),
            serialize(&second_header, b"Second body"),
        )
        .unwrap();
        drop(lock);
        let result = worker.join().unwrap().unwrap();
        assert_eq!(result.file.header.unwrap().id, second);
        assert!(ops.inspect(&second).unwrap().relations.related.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacement_work_directory_cannot_reuse_the_held_items_inode() {
        let (root, _id) = fixture();
        let ops = DurableOperations::new(&root);
        let lock = ops.lock().unwrap();
        let items = ops.items_dir(&lock).unwrap();
        let held_work = root.join("held-work");
        fs::rename(root.join(".work"), &held_work).unwrap();
        fs::create_dir(root.join(".work")).unwrap();
        fs::rename(held_work.join("items"), root.join(".work/items")).unwrap();
        assert!(matches!(
            ops.ensure_selected_dir(&lock, &items),
            Err(OperationError::Conflict(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replaced_symlink_before_publication_returns_conflict() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("missing-old", &path).unwrap();
        let before = ops.load().unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("missing-new", &path).unwrap();
        let lock = ops.lock().unwrap();
        let items = ops.items_dir(&lock).unwrap();
        assert!(matches!(
            ops.check_snapshot(&lock, &items, &before),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(fs::read_link(path).unwrap(), Path::new("missing-new"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepublication_failure_keeps_original_and_cleans_stage() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let original = fs::read(&path).unwrap();
        let result = ops.mutate_with_hook(
            &id,
            |_, h| {
                h.title = "Changed".into();
                Ok(())
            },
            || {
                Err(OperationError::Io(io::Error::other(
                    "injected publication failure",
                )))
            },
        );
        assert!(matches!(result, Err(OperationError::Io(_))));
        assert_eq!(fs::read(path).unwrap(), original);
        assert_eq!(staged_count(&root), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exchange_detects_changed_source_and_rolls_back() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let before = ops.load().unwrap();
        let source = &before.files[0];
        let proposed = serialize(&default_header(id, "Proposed".into()), b"Body");
        let staged = ops.stage(&proposed).unwrap();
        let published = FileFingerprint::capture(&staged).unwrap();
        let mut external = fs::read(&path).unwrap();
        external.extend_from_slice(b" external");
        fs::write(&path, &external).unwrap();
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        assert!(matches!(
            ops.verify_exchange(
                &ops.items_dir(&ops.lock().unwrap()).unwrap(),
                &before,
                source,
                &staged,
                &proposed,
                &published
            ),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), external);
        assert_eq!(staged_count(&root), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exchange_preserves_edit_made_to_published_target() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let before = ops.load().unwrap();
        let source = &before.files[0];
        let proposed = serialize(&default_header(id, "Proposed".into()), b"Body");
        let staged = ops.stage(&proposed).unwrap();
        let published = FileFingerprint::capture(&staged).unwrap();
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        let mut external = proposed.clone();
        external.extend_from_slice(b" external edit");
        fs::write(&path, &external).unwrap();
        assert!(matches!(
            ops.verify_exchange(
                &ops.items_dir(&ops.lock().unwrap()).unwrap(),
                &before,
                source,
                &staged,
                &proposed,
                &published
            ),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), external);
        assert_eq!(fs::read(&staged).unwrap(), source.raw);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exchange_preserves_mode_change_to_previous_source() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let before = ops.load().unwrap();
        let source = &before.files[0];
        let proposed = serialize(&default_header(id, "Proposed".into()), b"Body");
        let staged = ops.stage(&proposed).unwrap();
        let published = FileFingerprint::capture(&staged).unwrap();
        let changed_mode = (source.fingerprint.as_ref().unwrap().mode & 0o7777) ^ 0o100;
        fs::set_permissions(&path, fs::Permissions::from_mode(changed_mode)).unwrap();
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        assert!(matches!(
            ops.verify_exchange(
                &ops.items_dir(&ops.lock().unwrap()).unwrap(),
                &before,
                source,
                &staged,
                &proposed,
                &published
            ),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o7777,
            changed_mode
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vanished_exchange_target_is_a_conflict() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let lock = ops.lock().unwrap();
        let items = ops.items_dir(&lock).unwrap();
        let path = ops.item_path(&id);
        let staged = ops.stage_with_mode(&items, b"replacement", None).unwrap();
        fs::remove_file(&path).unwrap();
        let error = rename_in_dir(&items, &staged, &path, RenameFlags::EXCHANGE).unwrap_err();
        assert_eq!(exchange_error(error).code(), "conflict");
        remove_stage(&items, &staged);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn post_publication_io_error_identifies_recovery_copy() {
        let (root, _id) = fixture();
        let recovery = root.join(".work/items/.operation-recovery");
        let error = with_recovery(
            OperationError::Io(io::Error::other("sync failed")),
            &recovery,
        );
        assert_eq!(error.code(), "io");
        assert!(error.to_string().contains(recovery.to_str().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_post_publication_error_identifies_published_item() {
        let (root, _id) = fixture();
        let ops = DurableOperations::new(&root);
        let error = ops
            .create_with_hooks(
                "New item".into(),
                b"Body".to_vec(),
                MetadataChange::default(),
                |_| Ok(()),
                || {
                    Err(OperationError::Io(io::Error::other(
                        "injected sync failure",
                    )))
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), "io");
        let (published_id, path) = error.published_item().unwrap();
        assert_eq!(path, ops.item_path(published_id));
        assert_eq!(
            ops.inspect(published_id).unwrap().file.body.as_deref(),
            Some(&b"Body"[..])
        );
        assert!(error.to_string().contains(published_id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_filename_repair_identifies_both_source_paths() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let canonical = ops.item_path(&id);
        let old_path = root.join(".work/items/wrong.md");
        fs::rename(&canonical, &old_path).unwrap();
        let replacement = serialize(&default_header(id.clone(), "Replacement".into()), b"New");
        let error = ops
            .repair_with_hook(&id, replacement.clone(), || {
                fs::remove_file(&old_path)?;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.code(), "io");
        assert_eq!(
            error.published_item(),
            Some((id.as_str(), canonical.as_path()))
        );
        assert_eq!(error.previous_source_path(), Some(old_path.as_path()));
        assert!(error.to_string().contains(old_path.to_str().unwrap()));
        assert_eq!(fs::read(&canonical).unwrap(), replacement);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_filename_repair_retry_identifies_both_source_paths() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let canonical = ops.item_path(&id);
        let old_path = root.join(".work/items/wrong.md");
        fs::copy(&canonical, &old_path).unwrap();
        let replacement = fs::read(&canonical).unwrap();
        let error = ops
            .repair_with_hook(&id, replacement.clone(), || {
                fs::remove_file(&old_path)?;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.code(), "io");
        assert_eq!(
            error.published_item(),
            Some((id.as_str(), canonical.as_path()))
        );
        assert_eq!(error.previous_source_path(), Some(old_path.as_path()));
        assert_eq!(fs::read(&canonical).unwrap(), replacement);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_reloads_store_after_id_collision() {
        let (root, _id) = fixture();
        let ops = DurableOperations::new(&root);
        let collided = std::sync::atomic::AtomicBool::new(false);
        let created = ops
            .create_with_hooks(
                "New item".into(),
                b"Body".to_vec(),
                MetadataChange::default(),
                |id| {
                    if !collided.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        fs::write(
                            ops.item_path(id),
                            serialize(&default_header(id.into(), "Competing".into()), b"Other"),
                        )?;
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert!(collided.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(created.file.body.as_deref(), Some(&b"Body"[..]));
        assert_eq!(ops.list().unwrap().len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_reports_invalid_graph_after_publication() {
        let (root, existing_id) = fixture();
        let ops = DurableOperations::new(&root);
        let error = ops
            .create_with_hooks(
                "New item".into(),
                b"Body".to_vec(),
                MetadataChange::default(),
                |_| Ok(()),
                || {
                    fs::write(ops.item_path(&existing_id), b"broken")?;
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), "invalid_source");
        let (published_id, path) = error.published_item().unwrap();
        assert_eq!(path, ops.item_path(published_id));
        assert!(path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exchange_retains_published_file_changed_after_swap() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let before = ops.load().unwrap();
        let source = &before.files[0];
        let proposed = serialize(&default_header(id, "Proposed".into()), b"Body");
        let staged = ops.stage(&proposed).unwrap();
        let published = FileFingerprint::capture(&staged).unwrap();
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(matches!(
            ops.verify_exchange(
                &ops.items_dir(&ops.lock().unwrap()).unwrap(),
                &before,
                source,
                &staged,
                &proposed,
                &published
            ),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o400
        );
        assert_eq!(fs::read(&staged).unwrap(), source.raw);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclusive_create_publication_never_overwrites_existing_target() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let original = fs::read(&path).unwrap();
        let staged = ops.stage(b"replacement").unwrap();
        assert_eq!(
            renameat_with(CWD, &staged, CWD, &path, RenameFlags::NOREPLACE),
            Err(rustix::io::Errno::EXIST)
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(staged).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_rollback_reports_retained_recovery_path() {
        let (root, id) = fixture();
        let ops = DurableOperations::new(&root);
        let path = ops.item_path(&id);
        let before = ops.load().unwrap();
        let source = &before.files[0];
        let proposed = serialize(&default_header(id, "Proposed".into()), b"Body");
        let staged = ops.stage(&proposed).unwrap();
        let published = FileFingerprint::capture(&staged).unwrap();
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        fs::write(&staged, b"changed old source").unwrap();
        let error = ops
            .verify_exchange_with_hook(
                &ops.items_dir(&ops.lock().unwrap()).unwrap(),
                &before,
                source,
                &staged,
                (&proposed, &published),
                || {
                    fs::remove_file(&path).unwrap();
                },
            )
            .unwrap_err();
        assert!(matches!(error, OperationError::Conflict(_)));
        assert!(error.to_string().contains(staged.to_str().unwrap()));
        assert_eq!(fs::read(&staged).unwrap(), b"changed old source");
        fs::remove_dir_all(root).unwrap();
    }
}

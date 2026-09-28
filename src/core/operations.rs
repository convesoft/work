//! Shared durable item operations. Adapters only translate these results.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::{CWD, FlockOperation, Mode, OFlags, RenameFlags, flock, openat, renameat_with};

use super::graph::{Evaluation, ItemGraph, Relations};
use super::items::{
    Completion, Diagnostic, ItemFile, ItemHeader, ItemStore, ManualState, parse_candidate,
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
    pub file: ItemFile,
    pub relations: Relations,
    pub evaluation: Option<Evaluation>,
    pub graph_diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct RawInspection {
    pub file: ItemFile,
    pub graph_diagnostics: Vec<Diagnostic>,
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

impl DurableOperations {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
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
        })
    }

    /// Explicitly replace one malformed source document. Residual graph errors may
    /// remain while a caller repairs other documents in later steps.
    pub fn repair(&self, id: &str, raw: Vec<u8>) -> Result<RawInspection, OperationError> {
        if !valid_id(id) {
            return Err(OperationError::InvalidArgument(
                "item must be a full UUIDv4 ID".into(),
            ));
        }
        let _lock = self.lock()?;
        let store = self.load()?;
        let original = self.raw_source(&store, id)?;
        if let Some(header) = &original.header
            && header.id != id
        {
            return Err(OperationError::InvalidArgument(format!(
                "source contains item {}; repair it by that ID",
                header.id
            )));
        }
        let target_path = self.item_path(id);
        let graph_before = ItemGraph::from_store(&store);
        if original.is_valid()
            && !graph_before
                .diagnostics()
                .iter()
                .any(|d| d.path == original.path)
        {
            return Err(OperationError::InvalidArgument(
                "item has no source or graph diagnostic to repair".into(),
            ));
        }
        let candidate = parse_candidate(target_path.clone(), raw.clone());
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
            candidate_store(&store, &target_path, raw.clone())?
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
        let staged = self.stage_with_mode(&raw, Some(source_mode(&original.path)?))?;
        if let Err(error) = self.check_snapshot(&store) {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        if original.path != target_path {
            match renameat_with(CWD, &staged, CWD, &target_path, RenameFlags::NOREPLACE) {
                Ok(()) => {}
                Err(error) if error == rustix::io::Errno::EXIST => {
                    let _ = fs::remove_file(&staged);
                    return Err(OperationError::Conflict(
                        "canonical item path appeared during repair".into(),
                    ));
                }
                Err(error) => {
                    let _ = fs::remove_file(&staged);
                    return Err(OperationError::Io(io::Error::from(error)));
                }
            }
            self.sync_items()?;
            renameat_with(CWD, &original.path, CWD, &staged, RenameFlags::NOREPLACE).map_err(
                |error| {
                    OperationError::Io(io::Error::other(format!(
                        "canonical item published but old path could not be retained: {error}"
                    )))
                },
            )?;
            self.sync_items()?;
            let archived = fs::symlink_metadata(&staged)
                .ok()
                .filter(|meta| meta.file_type().is_file())
                .and_then(|_| fs::read(&staged).ok());
            if archived.as_deref() != Some(original.raw.as_slice()) {
                return Err(OperationError::Conflict(format!(
                    "source changed during filename repair; recovery copy retained at {}",
                    staged.display()
                )));
            }
            return self.inspect_raw(id);
        }
        if let Err(error) = renameat_with(CWD, &staged, CWD, &original.path, RenameFlags::EXCHANGE)
        {
            let _ = fs::remove_file(staged);
            return Err(OperationError::Io(io::Error::from(error)));
        }
        self.verify_exchange(&store, original, &staged, &raw)?;
        self.sync_items()?;
        let _ = fs::remove_file(&staged);
        self.inspect_raw(id)
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

    pub fn create(
        &self,
        title: String,
        body: Vec<u8>,
        change: MetadataChange,
    ) -> Result<Inspection, OperationError> {
        let _lock = self.lock()?;
        let store = self.load()?;
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
            let staged = self.stage(&raw)?;
            if let Err(error) = self.check_snapshot(&store) {
                let _ = fs::remove_file(&staged);
                return Err(error);
            }
            let path = self.item_path(&id);
            match renameat_with(CWD, &staged, CWD, &path, RenameFlags::NOREPLACE) {
                Ok(()) => {
                    self.sync_items()?;
                    return inspect_store(&self.load()?, &id);
                }
                Err(error) if error == rustix::io::Errno::EXIST => {
                    let _ = fs::remove_file(staged);
                    continue;
                }
                Err(error) => {
                    let _ = fs::remove_file(staged);
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
            let store = self.load()?;
            let authored_at_target = store.files.iter().any(|file| {
                file.header.as_ref().is_some_and(|header| {
                    header.id == target && header.related.iter().any(|id| id == source)
                })
            });
            let authored_at_source = store.files.iter().any(|file| {
                file.header.as_ref().is_some_and(|header| {
                    header.id == source && header.related.iter().any(|id| id == target)
                })
            });
            if !authored_at_source && authored_at_target {
                return self.relate(target, kind, source, false);
            }
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
        let _lock = self.lock()?;
        let store = self.load()?;
        require_valid(&store)?;
        let source = store
            .files
            .iter()
            .find(|f| f.header.as_ref().is_some_and(|h| h.id == id))
            .ok_or_else(|| OperationError::NotFound(id.into()))?;
        let mut header = source.header.clone().expect("valid store header");
        edit(&store, &mut header)?;
        let raw = serialize(&header, source.body.as_deref().expect("valid store body"));
        let candidate = candidate_store(&store, &source.path, raw.clone())?;
        require_candidate(&candidate)?;
        let staged = self.stage_with_mode(&raw, Some(source_mode(&source.path)?))?;
        if let Err(error) = before_publish() {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        if let Err(error) = self.check_snapshot(&store) {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        // Exchange retains the previous file at the staging path for recovery.
        if let Err(error) = renameat_with(CWD, &staged, CWD, &source.path, RenameFlags::EXCHANGE) {
            let _ = fs::remove_file(staged);
            return Err(OperationError::Io(io::Error::from(error)));
        }
        self.verify_exchange(&store, source, &staged, &raw)?;
        self.sync_items()?;
        let _ = fs::remove_file(&staged);
        self.sync_items()?;
        inspect_store(&self.load()?, id)
    }

    fn load(&self) -> Result<ItemStore, OperationError> {
        Ok(ItemStore::load_from_root(&self.root)?)
    }
    fn item_path(&self, id: &str) -> PathBuf {
        self.root.join(".work/items").join(format!("{id}.md"))
    }
    fn raw_source<'a>(
        &self,
        store: &'a ItemStore,
        id: &str,
    ) -> Result<&'a ItemFile, OperationError> {
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
    fn lock(&self) -> Result<File, OperationError> {
        let work_dir = openat(
            CWD,
            self.root.join(".work"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
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
        Ok(file)
    }
    fn stage(&self, raw: &[u8]) -> Result<PathBuf, OperationError> {
        self.stage_with_mode(raw, None)
    }
    fn stage_with_mode(&self, raw: &[u8], mode: Option<u32>) -> Result<PathBuf, OperationError> {
        for _ in 0..16 {
            let path = self
                .root
                .join(".work/items")
                .join(format!(".operation-{}", new_id()?));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(mut file) => {
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
                        let _ = fs::remove_file(path);
                        return Err(OperationError::Io(error));
                    }
                    return Ok(path);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(OperationError::Io(error)),
            }
        }
        Err(OperationError::Conflict(
            "could not reserve staging path".into(),
        ))
    }
    fn check_snapshot(&self, store: &ItemStore) -> Result<(), OperationError> {
        let current = self.load()?;
        if current.files.len() != store.files.len()
            || current
                .files
                .iter()
                .zip(&store.files)
                .any(|(a, b)| a.path != b.path || a.raw != b.raw)
        {
            return Err(OperationError::Conflict("items changed on disk".into()));
        }
        Ok(())
    }
    fn verify_exchange(
        &self,
        before: &ItemStore,
        source: &ItemFile,
        staged: &Path,
        new_raw: &[u8],
    ) -> Result<(), OperationError> {
        self.verify_exchange_with_hook(before, source, staged, new_raw, || {})
    }

    fn verify_exchange_with_hook(
        &self,
        before: &ItemStore,
        source: &ItemFile,
        staged: &Path,
        new_raw: &[u8],
        before_rollback: impl FnOnce(),
    ) -> Result<(), OperationError> {
        let old_metadata = fs::symlink_metadata(staged);
        let old_matches = old_metadata.as_ref().is_ok_and(|meta| {
            if meta.file_type().is_file() {
                fs::read(staged).is_ok_and(|raw| raw == source.raw)
            } else {
                source.diagnostics.iter().any(|d| {
                    d.message.contains("must not be a symlink")
                        || d.message.contains("must be a regular file")
                })
            }
        });
        let current = self.load();
        let stable = current.as_ref().is_ok_and(|now| {
            now.files.len() == before.files.len()
                && before.files.iter().all(|prior| {
                    now.files
                        .iter()
                        .find(|file| file.path == prior.path)
                        .is_some_and(|file| {
                            file.raw
                                == if file.path == source.path {
                                    new_raw
                                } else {
                                    &prior.raw
                                }
                        })
                })
        });
        if old_matches && stable {
            return Ok(());
        }
        let published_untouched = fs::symlink_metadata(&source.path)
            .ok()
            .filter(|meta| meta.file_type().is_file())
            .and_then(|_| fs::read(&source.path).ok())
            .is_some_and(|raw| raw == new_raw);
        if !published_untouched {
            return Err(OperationError::Conflict(format!(
                "item changed after publication; previous source retained at {}",
                staged.display()
            )));
        }
        before_rollback();
        renameat_with(CWD, staged, CWD, &source.path, RenameFlags::EXCHANGE).map_err(|e| {
            OperationError::Conflict(format!(
                "publication conflict; rollback failed: {e}; recovery copy retained at {}",
                staged.display()
            ))
        })?;
        self.sync_items().map_err(|e| {
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
    fn sync_items(&self) -> Result<(), OperationError> {
        File::open(self.root.join(".work/items"))?.sync_all()?;
        Ok(())
    }
}

fn inspect_store(store: &ItemStore, id: &str) -> Result<Inspection, OperationError> {
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
        file,
        relations,
        evaluation,
        graph_diagnostics: graph.diagnostics().to_vec(),
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

fn default_header(id: String, title: String) -> ItemHeader {
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
fn apply_change(mut h: ItemHeader, c: MetadataChange) -> ItemHeader {
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

fn serialize(h: &ItemHeader, body: &[u8]) -> Vec<u8> {
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
fn source_mode(path: &Path) -> Result<u32, OperationError> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(if metadata.file_type().is_file() {
        metadata.permissions().mode() & 0o7777
    } else {
        0o600
    })
}
fn new_id() -> Result<String, OperationError> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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
        let mut external = fs::read(&path).unwrap();
        external.extend_from_slice(b" external");
        fs::write(&path, &external).unwrap();
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        assert!(matches!(
            ops.verify_exchange(&before, source, &staged, &proposed),
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
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        let mut external = proposed.clone();
        external.extend_from_slice(b" external edit");
        fs::write(&path, &external).unwrap();
        assert!(matches!(
            ops.verify_exchange(&before, source, &staged, &proposed),
            Err(OperationError::Conflict(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), external);
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
        renameat_with(CWD, &staged, CWD, &path, RenameFlags::EXCHANGE).unwrap();
        fs::write(&staged, b"changed old source").unwrap();
        let error = ops
            .verify_exchange_with_hook(&before, source, &staged, &proposed, || {
                fs::remove_file(&path).unwrap();
            })
            .unwrap_err();
        assert!(matches!(error, OperationError::Conflict(_)));
        assert!(error.to_string().contains(staged.to_str().unwrap()));
        assert_eq!(fs::read(&staged).unwrap(), b"changed old source");
        fs::remove_dir_all(root).unwrap();
    }
}

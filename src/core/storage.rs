//! Shared, plain-file coordination foundation. Entity ownership belongs to later slices.
//!
//! @mara implements DES-STORE-FOUNDATION
//! @mara implements DES-STORAGE-API

// The agreed public error keeps publication/recovery provenance directly available
// to both adapters. Preserve that API instead of boxing away its fields.
#![allow(clippy::result_large_err)]

mod files;
mod format;

use std::fmt;
use std::path::PathBuf;

use super::project::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreMetadata {
    pub format_version: u32,
    pub store_id: String,
    pub recovery_generation: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageState {
    Uninitialized,
    Initialized,
    RecoveryRequired,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageDiagnostic {
    pub code: StorageErrorCode,
    pub message: String,
    pub path: Option<PathBuf>,
    pub line: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOperation {
    pub id: Option<String>,
    pub path: PathBuf,
    pub kind: Option<String>,
    pub phase: Option<String>,
    pub supported: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageInspection {
    pub state: StorageState,
    pub path: PathBuf,
    pub identity_path: PathBuf,
    pub lock_path: PathBuf,
    pub metadata: Option<StoreMetadata>,
    pub retained_store_id: Option<String>,
    pub coordination_available: bool,
    pub storage_warning: Option<StorageDiagnostic>,
    pub diagnostics: Vec<StorageDiagnostic>,
    pub pending_operations: Vec<PendingOperation>,
}
#[derive(Debug, Clone, Default)]
pub struct RecreateRequest {
    pub expected_store_id: Option<String>,
    pub expected_generation: Option<String>,
    pub executors_stopped: bool,
    pub acknowledge_loss: bool,
    pub all_clients_stopped: bool,
}
#[derive(Debug, Clone, Default)]
pub struct RecoverRequest {
    pub executors_stopped: bool,
    pub acknowledge_loss: bool,
    pub all_clients_stopped: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LossReport {
    pub coordination_reset: bool,
    pub missing_or_damaged: bool,
    pub prior_state_path: Option<PathBuf>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageOutcome {
    pub changed: bool,
    pub storage: StorageInspection,
    pub operation_id: Option<String>,
    pub recovery_paths: Vec<PathBuf>,
    pub loss: Option<LossReport>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publication {
    NotPublished,
    Possible,
    Published,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageErrorCode {
    InvalidArgument,
    StorageBusy,
    InvalidFormat,
    UnsupportedFormat,
    StorageMissing,
    StorageCorrupt,
    RecoveryRequired,
    IdentityMismatch,
    UnsafePath,
    Conflict,
    PermissionDenied,
    Io,
}
impl StorageErrorCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::StorageBusy => "storage_busy",
            Self::InvalidFormat => "invalid_format",
            Self::UnsupportedFormat => "unsupported_format",
            Self::StorageMissing => "storage_missing",
            Self::StorageCorrupt => "storage_corrupt",
            Self::RecoveryRequired => "recovery_required",
            Self::IdentityMismatch => "identity_mismatch",
            Self::UnsafePath => "unsafe_path",
            Self::Conflict => "conflict",
            Self::PermissionDenied => "permission_denied",
            Self::Io => "io",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    pub code: StorageErrorCode,
    pub message: String,
    pub path: Option<PathBuf>,
    pub operation_id: Option<String>,
    pub publication: Publication,
    pub recovery_paths: Vec<PathBuf>,
    pub diagnostics: Vec<StorageDiagnostic>,
    pub errno: Option<i32>,
}
impl StorageError {
    fn new(code: StorageErrorCode, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            code,
            message: message.into(),
            path,
            operation_id: None,
            publication: Publication::NotPublished,
            recovery_paths: Vec::new(),
            diagnostics: Vec::new(),
            errno: None,
        }
    }
    fn diagnostic(&self) -> StorageDiagnostic {
        StorageDiagnostic {
            code: self.code,
            message: self.message.clone(),
            path: self.path.clone(),
            line: self.diagnostics.first().and_then(|d| d.line),
        }
    }
    fn io(error: std::io::Error, path: PathBuf) -> Self {
        let code = if error.kind() == std::io::ErrorKind::PermissionDenied {
            StorageErrorCode::PermissionDenied
        } else {
            StorageErrorCode::Io
        };
        let mut result = Self::new(code, error.to_string(), Some(path));
        result.errno = error.raw_os_error();
        result
    }
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.code(), self.message)
    }
}
impl std::error::Error for StorageError {}

pub struct Storage {
    project: Project,
}
const LIVE: [&str; 4] = ["claims", "handoffs", "runs", "workspaces"];
const DIRECTORIES: [&str; 6] = [
    "claims",
    "handoffs",
    "operations",
    "recovery",
    "runs",
    "workspaces",
];

impl Storage {
    pub fn new(project: Project) -> Self {
        Self { project }
    }
    fn empty_inspection(&self) -> StorageInspection {
        let path = self.project.git_common_dir.join("work");
        StorageInspection {
            state: StorageState::Uninitialized,
            identity_path: self.project.git_common_dir.join("work.identity.yaml"),
            lock_path: path.join("coordination.lock"),
            path,
            metadata: None,
            retained_store_id: None,
            coordination_available: false,
            storage_warning: None,
            diagnostics: Vec::new(),
            pending_operations: Vec::new(),
        }
    }
    /// Read-only health. Unavailable coordination is data, not an empty owner set.
    pub fn inspect(&self) -> Result<StorageInspection, StorageError> {
        let mut result = self.empty_inspection();
        let common = match files::Directory::open(&self.project.git_common_dir) {
            Ok(common) => common,
            Err(error) => {
                warn(&mut result, error);
                return Ok(result);
            }
        };
        let root_present = common.exists("work")?;
        let witness_present = common.exists("work.identity.yaml")?;
        if !root_present && !witness_present {
            return Ok(result);
        }
        if !root_present {
            match common
                .read("work.identity.yaml")
                .and_then(|s| format::identity(&s.raw, &result.identity_path))
            {
                Ok(id) => result.retained_store_id = Some(id),
                Err(e) => warn(&mut result, e),
            }
            let missing_path = result.path.clone();
            warn(
                &mut result,
                StorageError::new(
                    StorageErrorCode::StorageMissing,
                    "shared storage is missing after initialization",
                    Some(missing_path),
                ),
            );
            return Ok(result);
        }
        let locked = match files::Locked::open(common, false, false) {
            Ok(locked) => locked,
            Err(e) => {
                // Advisory evidence remains available when exclusion cannot be acquired.
                // Mutations re-read it under an exclusive lock before acting.
                if let Ok(common) = files::Directory::open(&self.project.git_common_dir) {
                    if let Ok(root) = common.child("work")
                        && let Ok(source) = root.read("store.yaml")
                    {
                        result.metadata =
                            format::metadata(&source.raw, &root.path.join("store.yaml")).ok();
                    }
                    if let Ok(source) = common.read("work.identity.yaml") {
                        result.retained_store_id =
                            format::identity(&source.raw, &common.path.join("work.identity.yaml"))
                                .ok();
                    }
                }
                warn(&mut result, e);
                return Ok(result);
            }
        };
        self.inspect_locked(&locked, &mut result);
        Ok(result)
    }
    fn inspect_locked(&self, locked: &files::Locked, result: &mut StorageInspection) {
        match locked
            .root
            .read("store.yaml")
            .and_then(|s| format::metadata(&s.raw, &result.path.join("store.yaml")))
        {
            Ok(meta) => result.metadata = Some(meta),
            Err(e) => warn(result, e),
        }
        match locked
            .common
            .read("work.identity.yaml")
            .and_then(|s| format::identity(&s.raw, &result.identity_path))
        {
            Ok(id) => result.retained_store_id = Some(id),
            Err(e) => warn(result, e),
        }
        if let (Some(meta), Some(id)) = (&result.metadata, &result.retained_store_id)
            && &meta.store_id != id
        {
            warn(
                result,
                StorageError::new(
                    StorageErrorCode::IdentityMismatch,
                    "metadata and retained local identity disagree",
                    Some(result.identity_path.clone()),
                ),
            );
        }
        for name in DIRECTORIES {
            if let Err(e) = locked.root.child(name) {
                warn(result, e);
            }
        }
        if let Ok(operations) = locked.root.child("operations") {
            match operations.names() {
                Ok(names) => {
                    for name in names {
                        // Same-directory retained/staged copies are not live operation entries.
                        if name.to_string_lossy().starts_with(".storage-") {
                            continue;
                        }
                        let path = operations.path.join(&name);
                        let id = name
                            .to_str()
                            .filter(|s| format::valid_id(s))
                            .map(str::to_owned);
                        let decoded = if id.is_none() {
                            Err(StorageError::new(
                                StorageErrorCode::UnsafePath,
                                "invalid operation directory ID",
                                Some(path.clone()),
                            ))
                        } else {
                            operations.child(&name).and_then(|dir| {
                                let source = dir.read("operation.yaml")?;
                                let op = format::operation(
                                    &source.raw,
                                    &dir.path.join("operation.yaml"),
                                )?;
                                if Some(&op.id) != id.as_ref() {
                                    return Err(StorageError::new(
                                        StorageErrorCode::InvalidFormat,
                                        "operation ID differs from directory",
                                        Some(path.clone()),
                                    ));
                                }
                                let meta = dir.read("store.yaml")?;
                                let identity = dir.read("identity.yaml")?;
                                op.check_stages(&meta.raw, &identity.raw, &dir.path)?;
                                format::context(
                                    &dir.read("context.yaml")?.raw,
                                    &op,
                                    &dir.path.join("context.yaml"),
                                )?;
                                Ok(op)
                            })
                        };
                        match decoded {
                            Ok(op) if op.phase == "complete" => {}
                            Ok(op) => {
                                result.pending_operations.push(PendingOperation {
                                    id,
                                    path: path.clone(),
                                    kind: Some(op.kind),
                                    phase: Some(op.phase),
                                    supported: true,
                                });
                                warn(
                                    result,
                                    StorageError::new(
                                        StorageErrorCode::RecoveryRequired,
                                        "foundation operation is incomplete",
                                        Some(path),
                                    ),
                                );
                            }
                            Err(e) => {
                                result.pending_operations.push(PendingOperation {
                                    id,
                                    path,
                                    kind: None,
                                    phase: None,
                                    supported: false,
                                });
                                warn(result, e);
                            }
                        }
                    }
                }
                Err(e) => warn(result, e),
            }
        }
        if let Err(e) = locked.verify() {
            warn(result, e);
        }
        if result.diagnostics.is_empty() {
            result.state = StorageState::Initialized;
            result.coordination_available = true;
        }
    }
}
fn warn(result: &mut StorageInspection, error: StorageError) {
    result.state = StorageState::RecoveryRequired;
    result.coordination_available = false;
    let diagnostic = error.diagnostic();
    if result.storage_warning.is_none() {
        result.storage_warning = Some(diagnostic.clone());
    }
    result.diagnostics.push(diagnostic);
}

impl Storage {
    /// Explicit initialization; root remnants are never adopted as fresh state.
    pub fn initialize(&self) -> Result<StorageOutcome, StorageError> {
        self.initialize_inner(&mut |_| Ok(()))
    }
    fn initialize_inner(
        &self,
        checkpoint: &mut dyn FnMut(&str) -> Result<(), StorageError>,
    ) -> Result<StorageOutcome, StorageError> {
        let common = files::Directory::open(&self.project.git_common_dir)?;
        if common.exists("work")? {
            let locked = files::Locked::open(common, true, false)?;
            let mut inspection = self.empty_inspection();
            self.inspect_locked(&locked, &mut inspection);
            if inspection.coordination_available {
                return Ok(StorageOutcome {
                    changed: false,
                    storage: inspection,
                    operation_id: None,
                    recovery_paths: Vec::new(),
                    loss: None,
                });
            }
            return Err(inspection_error(&inspection));
        }
        if common.exists("work.identity.yaml")? {
            // Refuse a future witness version even before creating a root.
            let source = common.read("work.identity.yaml")?;
            format::identity(&source.raw, &common.path.join("work.identity.yaml"))?;
            return Err(StorageError::new(
                StorageErrorCode::RecoveryRequired,
                "retained identity exists; use explicit recreation, not initialization",
                Some(common.path.join("work")),
            ));
        }
        common.create("work")?;
        let locked = files::Locked::open(common, true, true)?;
        locked.verify()?;
        if locked.common.exists("work.identity.yaml")? {
            return Err(files::conflict(
                &locked.common.path,
                "identity appeared during initialization",
            ));
        }
        checkpoint("root_lock")?;
        let operations = locked.root.create("operations")?;
        let mut operation = format::Operation {
            id: new_id()?,
            kind: "initialize".into(),
            store_id: new_id()?,
            previous_generation: None,
            next_generation: new_id()?,
            phase: "prepared".into(),
            prior_folders: Vec::new(),
            prior_operations: Vec::new(),
            executors_stopped: false,
        };
        let dir = operations.create(&operation.id)?;
        let context = format::Context::empty();
        let mut publication = Publication::NotPublished;
        let result = (|| {
            prepare(&dir, &operation, &context, checkpoint)?;
            self.finish(
                &locked,
                &dir,
                &mut operation,
                &context,
                checkpoint,
                &mut publication,
            )
        })();
        operation_result(result, &operation, &dir.path, publication)
    }
    /// Retain surviving bytes before publishing an empty, fresh operational generation.
    pub fn recreate(&self, request: RecreateRequest) -> Result<StorageOutcome, StorageError> {
        self.recreate_inner(request, &mut |_| Ok(()))
    }
    fn recreate_inner(
        &self,
        request: RecreateRequest,
        checkpoint: &mut dyn FnMut(&str) -> Result<(), StorageError>,
    ) -> Result<StorageOutcome, StorageError> {
        validate_recreate(&request)?;
        let common = files::Directory::open(&self.project.git_common_dir)?;
        // Validate witness version before any explicit lost-root creation.
        let advisory_identity = common.optional("work.identity.yaml")?;
        let witness =
            parse_optional_identity(&advisory_identity, &common.path.join("work.identity.yaml"))?;
        let root_present = common.exists("work")?;
        // Even explicit lost-lock recreation must never downgrade a future metadata format.
        if root_present {
            let advisory_root = common.child("work")?;
            let advisory_store = advisory_root.optional("store.yaml")?;
            reject_future(
                &advisory_store,
                false,
                &advisory_root.path.join("store.yaml"),
            )?;
        }
        let lock_present = if root_present {
            common.child("work")?.exists("coordination.lock")?
        } else {
            false
        };
        if !lock_present {
            let advisory_meta = if root_present {
                let root = common.child("work")?;
                parse_optional_metadata(
                    &root.optional("store.yaml")?,
                    &root.path.join("store.yaml"),
                )?
            } else {
                None
            };
            if let (Some(meta), Some(id)) = (&advisory_meta, &witness)
                && meta.store_id != *id
            {
                return Err(StorageError::new(
                    StorageErrorCode::IdentityMismatch,
                    "metadata and local witness disagree",
                    Some(common.path.join("work.identity.yaml")),
                ));
            }
            let id = witness
                .clone()
                .or_else(|| advisory_meta.as_ref().map(|m| m.store_id.clone()));
            let generation = advisory_meta.map(|m| m.recovery_generation);
            if request.expected_store_id != id || request.expected_generation != generation {
                return Err(files::conflict(
                    &common.path,
                    "expected evidence changed before lost-lock recovery",
                ));
            }
        }
        if (!root_present || !lock_present) && !request.all_clients_stopped {
            return Err(StorageError::new(
                StorageErrorCode::InvalidArgument,
                "missing root/lock recreation additionally requires all_clients_stopped",
                Some(common.path.join("work/coordination.lock")),
            ));
        }
        if !root_present {
            if request.expected_store_id != witness || request.expected_generation.is_some() {
                return Err(files::conflict(
                    &common.path,
                    "expected identity/generation does not match lost-root evidence",
                ));
            }
            common.create("work")?;
        }
        let locked = files::Locked::open(common, true, !lock_present)?;
        let store = locked.root.optional("store.yaml")?;
        let identity = locked.common.optional("work.identity.yaml")?;
        let meta = parse_optional_metadata(&store, &locked.root.path.join("store.yaml"))?;
        let retained_id =
            parse_optional_identity(&identity, &locked.common.path.join("work.identity.yaml"))?;
        if let (Some(meta), Some(id)) = (&meta, &retained_id)
            && meta.store_id != *id
        {
            return Err(StorageError::new(
                StorageErrorCode::IdentityMismatch,
                "metadata and local witness disagree",
                Some(locked.common.path.join("work.identity.yaml")),
            ));
        }
        let expected_id = retained_id
            .clone()
            .or_else(|| meta.as_ref().map(|m| m.store_id.clone()));
        let expected_generation = meta.as_ref().map(|m| m.recovery_generation.clone());
        if request.expected_store_id != expected_id
            || request.expected_generation != expected_generation
        {
            return Err(files::conflict(
                &locked.root.path,
                "expected identity/generation does not match current validated evidence",
            ));
        }
        let mut context = format::Context {
            store,
            identity,
            folders: Default::default(),
            operations: Default::default(),
        };
        for name in LIVE {
            if locked.root.exists(name)? {
                context
                    .folders
                    .insert(name.to_owned(), locked.root.child(name)?.identity);
            }
        }
        let operations = locked.root.ensure("operations")?;
        for name in operations.names()? {
            let id = name
                .to_str()
                .filter(|id| format::valid_id(id))
                .ok_or_else(|| {
                    StorageError::new(
                        StorageErrorCode::UnsafePath,
                        "noncanonical prior operation directory",
                        Some(operations.path.join(&name)),
                    )
                })?;
            let dir = operations.child(id)?;
            if let Ok(source) = dir.read("operation.yaml")
                && let Ok(prior) = format::operation(&source.raw, &dir.path.join("operation.yaml"))
                && prior.kind == "recreate"
                && prior.phase != "complete"
            {
                // A healthy committed intent still needs its own completion.
                // Supersession is only for terminal structure that cannot replay
                // safely; explicit recreation then retains it in a fresh generation.
                if prior.phase == "committed"
                    && terminal_structure_damaged(
                        &locked.root,
                        &locked.common,
                        &dir,
                        &prior,
                        !lock_present,
                    )?
                {
                    context.operations.insert(id.to_owned(), dir.identity);
                    continue;
                }
                return Err(StorageError::new(
                    StorageErrorCode::RecoveryRequired,
                    "resume the existing recreation by ID instead of superseding it",
                    Some(dir.path),
                ));
            }
            context.operations.insert(id.to_owned(), dir.identity);
        }
        let mut operation = format::Operation {
            id: new_id()?,
            kind: "recreate".into(),
            store_id: retained_id.unwrap_or(new_id()?),
            previous_generation: expected_generation,
            next_generation: new_id()?,
            phase: "prepared".into(),
            prior_folders: context.folders.keys().cloned().collect(),
            prior_operations: context.operations.keys().cloned().collect(),
            executors_stopped: true,
        };
        let dir = operations.create(&operation.id)?;
        let mut publication = Publication::NotPublished;
        let result = (|| {
            prepare(&dir, &operation, &context, checkpoint)?;
            self.finish(
                &locked,
                &dir,
                &mut operation,
                &context,
                checkpoint,
                &mut publication,
            )
        })();
        operation_result(result, &operation, &dir.path, publication)
    }
    /// Resume only a strictly validated foundation intent; never replay a foreign generation.
    pub fn recover(
        &self,
        operation_id: &str,
        request: RecoverRequest,
    ) -> Result<StorageOutcome, StorageError> {
        self.recover_inner(operation_id, request, &mut |_| Ok(()))
    }
    fn recover_inner(
        &self,
        operation_id: &str,
        request: RecoverRequest,
        checkpoint: &mut dyn FnMut(&str) -> Result<(), StorageError>,
    ) -> Result<StorageOutcome, StorageError> {
        if !format::valid_id(operation_id) {
            return Err(StorageError::new(
                StorageErrorCode::InvalidArgument,
                "operation ID must be a canonical UUIDv4",
                None,
            ));
        }
        let common = files::Directory::open(&self.project.git_common_dir)?;
        let root = common.child("work")?;
        let missing_lock = !root.exists("coordination.lock")?;
        if missing_lock {
            let advisory_dir = root.child("operations")?.child(operation_id)?;
            let advisory_op = format::operation(
                &advisory_dir.read("operation.yaml")?.raw,
                &advisory_dir.path.join("operation.yaml"),
            )?;
            if advisory_op.id != operation_id {
                return Err(StorageError::new(
                    StorageErrorCode::InvalidFormat,
                    "operation ID does not match its directory",
                    Some(advisory_dir.path),
                ));
            }
            if matches!(advisory_op.phase.as_str(), "committed" | "complete") {
                return Err(StorageError::new(
                    StorageErrorCode::StorageMissing,
                    "completed receipt cannot replace a lost coordination lock; explicit recreation required",
                    Some(root.path.join("coordination.lock")),
                ));
            }
            if advisory_op.kind == "recreate"
                && (!request.executors_stopped || !request.acknowledge_loss)
            {
                return Err(StorageError::new(
                    StorageErrorCode::InvalidArgument,
                    "recreation recovery requires stopped executors and loss acknowledgement",
                    Some(advisory_dir.path),
                ));
            }
            reject_future(
                &root.optional("store.yaml")?,
                false,
                &root.path.join("store.yaml"),
            )?;
            reject_future(
                &common.optional("work.identity.yaml")?,
                true,
                &common.path.join("work.identity.yaml"),
            )?;
        }
        if missing_lock && !request.all_clients_stopped {
            return Err(StorageError::new(
                StorageErrorCode::InvalidArgument,
                "lost lock recovery requires all_clients_stopped",
                Some(root.path.join("coordination.lock")),
            ));
        }
        let locked = files::Locked::open(common, true, missing_lock)?;
        let operations = locked.root.child("operations")?;
        if !operations.exists(operation_id)? {
            // Explicit replay of a retained obsolete receipt never regresses generation.
            if let Ok(recovery) = locked.root.child("recovery") {
                for name in recovery.names()? {
                    let Some(id) = name.to_str().filter(|id| format::valid_id(id)) else {
                        continue;
                    };
                    if let Ok(prior_operations) = recovery.child(id)?.child("operations")
                        && prior_operations.exists(operation_id)?
                    {
                        return Err(files::conflict(
                            &prior_operations.path.join(operation_id),
                            "operation was archived by recreation; current generation cannot replay its receipt",
                        ));
                    }
                }
            }
        }
        let dir = operations.child(operation_id)?;
        let source = dir.read("operation.yaml")?;
        let mut operation = format::operation(&source.raw, &dir.path.join("operation.yaml"))?;
        if operation.id != operation_id {
            return Err(StorageError::new(
                StorageErrorCode::InvalidFormat,
                "operation ID does not match its directory",
                Some(dir.path),
            ));
        }
        let intended_store = dir.read("store.yaml")?;
        let intended_identity = dir.read("identity.yaml")?;
        operation.check_stages(&intended_store.raw, &intended_identity.raw, &dir.path)?;
        let context = format::context(
            &dir.read("context.yaml")?.raw,
            &operation,
            &dir.path.join("context.yaml"),
        )?;
        if operation.kind == "recreate" && (!request.executors_stopped || !request.acknowledge_loss)
        {
            return Err(StorageError::new(
                StorageErrorCode::InvalidArgument,
                "recreation recovery requires stopped executors and loss acknowledgement on each attempt",
                Some(dir.path),
            ));
        }
        let mut publication = Publication::NotPublished;
        let result = self.finish(
            &locked,
            &dir,
            &mut operation,
            &context,
            checkpoint,
            &mut publication,
        );
        operation_result(result, &operation, &dir.path, publication)
    }
    fn finish(
        &self,
        locked: &files::Locked,
        dir: &files::Directory,
        operation: &mut format::Operation,
        context: &format::Context,
        checkpoint: &mut dyn FnMut(&str) -> Result<(), StorageError>,
        publication: &mut Publication,
    ) -> Result<StorageOutcome, StorageError> {
        locked.verify()?;
        dir.verify()?;
        let intended_metadata = format::metadata_bytes(&operation.metadata());
        let intended_identity = format::identity_bytes(&operation.store_id);
        // Reject future versions and changed prior evidence before moving any folder.
        let current_store = locked.root.optional("store.yaml")?;
        let current_identity = locked.common.optional("work.identity.yaml")?;
        reject_future(&current_store, false, &locked.root.path.join("store.yaml"))?;
        reject_future(
            &current_identity,
            true,
            &locked.common.path.join("work.identity.yaml"),
        )?;
        validate_source(
            &current_store,
            &context.store,
            &intended_metadata,
            &locked.root.path.join("store.yaml"),
        )?;
        validate_source(
            &current_identity,
            &context.identity,
            &intended_identity,
            &locked.common.path.join("work.identity.yaml"),
        )?;
        if matches!(operation.phase.as_str(), "complete" | "committed")
            && (current_store.as_ref().map(|s| s.raw.as_slice())
                != Some(intended_metadata.as_slice())
                || current_identity.as_ref().map(|s| s.raw.as_slice())
                    != Some(intended_identity.as_slice()))
        {
            return Err(files::conflict(
                &dir.path,
                "completed receipt no longer matches current store generation",
            ));
        }
        if matches!(operation.phase.as_str(), "complete" | "committed") {
            validate_terminal_structure(&locked.root, operation, context)?;
        }
        let terminal = matches!(operation.phase.as_str(), "complete" | "committed");
        let readonly_archive =
            terminal || (operation.kind == "recreate" && operation.phase == "archived");
        let recovery = if readonly_archive {
            locked.root.child("recovery")?
        } else {
            locked.root.ensure("recovery")?
        };
        let mut recovery_paths = Vec::new();
        if operation.kind == "recreate" {
            let archive = if readonly_archive {
                recovery.child(&operation.id)?
            } else {
                recovery.ensure(&operation.id)?
            };
            let prior = if readonly_archive {
                archive.child("prior")?
            } else {
                archive.ensure("prior")?
            };
            let prior_operations = if readonly_archive {
                archive.child("operations")?
            } else {
                archive.ensure("operations")?
            };
            if operation.phase == "prepared" {
                for (name, identity) in &context.folders {
                    archive_one(&locked.root, &prior, name, identity)?;
                    checkpoint(&format!("archive_folder:{name}"))?;
                }
                let operations = locked.root.child("operations")?;
                for (id, identity) in &context.operations {
                    archive_one(&operations, &prior_operations, id, identity)?;
                    checkpoint(&format!("archive_operation:{id}"))?;
                }
                operation.phase = "archived".into();
                publish_phase(dir, operation)?;
                checkpoint("archived")?;
            } else {
                for (name, identity) in &context.folders {
                    check_archived(&prior, name, identity)?;
                }
                for (id, identity) in &context.operations {
                    check_archived(&prior_operations, id, identity)?;
                }
            }
            recovery_paths.push(archive.path);
        }
        for name in LIVE {
            if matches!(operation.phase.as_str(), "prepared" | "archived")
                && locked.root.exists(name)?
                && !locked.root.child(name)?.names()?.is_empty()
            {
                return Err(files::conflict(
                    &locked.root.path.join(name),
                    "unexpected live content during pending initialization/recreation",
                ));
            }
            locked.root.ensure(name)?;
        }
        // Newly prepared initialization has the required operations and recovery folders.
        locked.verify()?;
        checkpoint("live_folders")?;
        publish_source(
            &locked.common,
            "work.identity.yaml",
            &intended_identity,
            &context.identity,
            operation,
            &mut recovery_paths,
            publication,
        )?;
        checkpoint("witness")?;
        locked.verify()?;
        publish_source(
            &locked.root,
            "store.yaml",
            &intended_metadata,
            &context.store,
            operation,
            &mut recovery_paths,
            publication,
        )?;
        checkpoint("metadata")?;
        locked.verify()?;
        if operation.phase != "complete" {
            operation.phase = "committed".into();
            publish_phase(dir, operation)?;
            checkpoint("committed")?;
            locked.verify()?;
            if locked.root.read("store.yaml")?.raw != intended_metadata
                || locked.common.read("work.identity.yaml")?.raw != intended_identity
            {
                return Err(files::conflict(
                    &locked.root.path,
                    "published metadata changed before operation completion",
                ));
            }
            operation.phase = "complete".into();
            publish_phase(dir, operation)?;
            checkpoint("complete")?;
        }
        let mut inspection = self.empty_inspection();
        self.inspect_locked(locked, &mut inspection);
        if !inspection.coordination_available {
            return Err(inspection_error(&inspection));
        }
        let loss = if operation.kind == "recreate" {
            Some(LossReport {
                coordination_reset: true,
                missing_or_damaged: context
                    .store
                    .as_ref()
                    .and_then(|s| format::metadata(&s.raw, &dir.path).ok())
                    .is_none()
                    || context
                        .identity
                        .as_ref()
                        .and_then(|s| format::identity(&s.raw, &dir.path).ok())
                        .is_none()
                    || context.folders.len() != LIVE.len(),
                prior_state_path: Some(recovery.path.join(&operation.id)),
            })
        } else {
            None
        };
        Ok(StorageOutcome {
            changed: true,
            storage: inspection,
            operation_id: Some(operation.id.clone()),
            recovery_paths,
            loss,
        })
    }
}
fn inspection_error(inspection: &StorageInspection) -> StorageError {
    let diagnostic = inspection
        .storage_warning
        .as_ref()
        .expect("unavailable initialized inspection has warning");
    let mut error = StorageError::new(
        diagnostic.code,
        diagnostic.message.clone(),
        diagnostic.path.clone(),
    );
    error.diagnostics = inspection.diagnostics.clone();
    error
}
fn validate_recreate(request: &RecreateRequest) -> Result<(), StorageError> {
    if !request.executors_stopped || !request.acknowledge_loss {
        return Err(StorageError::new(
            StorageErrorCode::InvalidArgument,
            "recreation requires stopped executors and explicit loss acknowledgement",
            None,
        ));
    }
    for id in [&request.expected_store_id, &request.expected_generation]
        .into_iter()
        .flatten()
    {
        if !format::valid_id(id) {
            return Err(StorageError::new(
                StorageErrorCode::InvalidArgument,
                "expected identity/generation must be canonical UUIDv4",
                None,
            ));
        }
    }
    Ok(())
}
fn new_id() -> Result<String, StorageError> {
    use std::io::Read;
    let mut raw = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut raw))
        .map_err(|e| StorageError::io(e, PathBuf::from("/dev/urandom")))?;
    raw[6] = (raw[6] & 15) | 64;
    raw[8] = (raw[8] & 63) | 128;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}
fn prepare(
    dir: &files::Directory,
    operation: &format::Operation,
    context: &format::Context,
    checkpoint: &mut dyn FnMut(&str) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    dir.stage("store.yaml", &format::metadata_bytes(&operation.metadata()))?;
    checkpoint("stage_store")?;
    dir.stage(
        "identity.yaml",
        &format::identity_bytes(&operation.store_id),
    )?;
    checkpoint("stage_identity")?;
    dir.stage("context.yaml", &context.bytes())?;
    checkpoint("context")?;
    dir.publish(
        "operation.yaml",
        &operation.bytes(),
        None,
        &format!(".storage-{}", new_id()?),
    )?;
    dir.sync()?;
    checkpoint("prepared")
}
fn publish_phase(
    dir: &files::Directory,
    operation: &format::Operation,
) -> Result<(), StorageError> {
    let previous = dir.read("operation.yaml")?;
    dir.publish(
        "operation.yaml",
        &operation.bytes(),
        Some(&previous),
        &format!(".storage-{}", new_id()?),
    )?;
    dir.sync()
}
fn parse_optional_metadata(
    source: &Option<files::Source>,
    path: &std::path::Path,
) -> Result<Option<StoreMetadata>, StorageError> {
    match source {
        None => Ok(None),
        Some(s) => match format::metadata(&s.raw, path) {
            Ok(meta) => Ok(Some(meta)),
            Err(e) if e.code == StorageErrorCode::UnsupportedFormat => Err(e),
            Err(_) => Ok(None),
        },
    }
}
fn parse_optional_identity(
    source: &Option<files::Source>,
    path: &std::path::Path,
) -> Result<Option<String>, StorageError> {
    match source {
        None => Ok(None),
        Some(s) => match format::identity(&s.raw, path) {
            Ok(id) => Ok(Some(id)),
            Err(e) if e.code == StorageErrorCode::UnsupportedFormat => Err(e),
            Err(_) => Ok(None),
        },
    }
}
fn reject_future(
    source: &Option<files::Source>,
    identity: bool,
    path: &std::path::Path,
) -> Result<(), StorageError> {
    if identity {
        parse_optional_identity(source, path).map(|_| ())
    } else {
        parse_optional_metadata(source, path).map(|_| ())
    }
}
fn validate_source(
    current: &Option<files::Source>,
    prior: &Option<files::Source>,
    intended: &[u8],
    path: &std::path::Path,
) -> Result<(), StorageError> {
    if current == prior || current.as_ref().is_some_and(|s| s.raw == intended) {
        Ok(())
    } else {
        Err(files::conflict(
            path,
            "current source is neither captured prior state nor intended publication",
        ))
    }
}
fn publish_source(
    dir: &files::Directory,
    name: &str,
    raw: &[u8],
    prior: &Option<files::Source>,
    operation: &format::Operation,
    recovery: &mut Vec<PathBuf>,
    publication: &mut Publication,
) -> Result<(), StorageError> {
    let current = dir.optional(name)?;
    let label = if name == "store.yaml" {
        "store"
    } else {
        "identity"
    };
    let stage_name = format!(".storage-{}-{label}", operation.id);
    let stage_path = dir.path.join(&stage_name);
    if current.as_ref().is_some_and(|s| s.raw == raw) {
        // Even a completed retry re-syncs published objects before reporting success.
        let _synced_source =
            dir.sync_source(name, current.as_ref().expect("matching published source"))?;
        dir.sync()?;
        *publication = Publication::Published;
        if dir.exists(&stage_name)? {
            recovery.push(stage_path);
        }
        return Ok(());
    }
    if &current != prior {
        return Err(files::conflict(
            &dir.path.join(name),
            "prior metadata changed before replacement",
        ));
    }
    if dir.exists(&stage_name)? {
        let staged = dir.read(&stage_name)?;
        if staged.raw != raw {
            return Err(files::conflict(
                &stage_path,
                "existing publication stage has unexpected bytes",
            ));
        }
        // Correct bytes may remain after a failed fsync. Retry that held-file
        // sync before publication, even when the stage need not be rewritten.
        let _synced_stage = dir.sync_source(&stage_name, &staged)?;
        let flags = if prior.is_some() {
            rustix::fs::RenameFlags::EXCHANGE
        } else {
            rustix::fs::RenameFlags::NOREPLACE
        };
        dir.verify()?;
        if dir.optional(name)?.as_ref() != prior.as_ref() {
            return Err(files::conflict(
                &dir.path.join(name),
                "source changed before staged retry",
            ));
        }
        rustix::fs::renameat_with(&dir.file, &stage_name, &dir.file, name, flags)
            .map_err(|e| StorageError::io(std::io::Error::from(e), dir.path.join(name)))?;
        *publication = Publication::Possible;
        dir.sync()?;
        dir.verify()?;
        if let Some(before) = prior
            && !before.same_content_identity(&dir.read(&stage_name)?)
        {
            return Err(files::conflict(
                &stage_path,
                "previous source changed during retry publication",
            ));
        }
        if prior.is_some() {
            recovery.push(stage_path);
        }
    } else {
        let result = dir.publish(name, raw, prior.as_ref(), &stage_name);
        if let Err(error) = &result
            && error.publication != Publication::NotPublished
        {
            *publication = error.publication;
        }
        if let Some(path) = result? {
            recovery.push(path);
        }
    }
    *publication = Publication::Published;
    Ok(())
}
// Read-only validation shared by completed replay and explicit recreation's
// pending-operation guard. Permission/I/O uncertainty is not proof of damage.
fn terminal_structure_damaged(
    root: &files::Directory,
    common: &files::Directory,
    dir: &files::Directory,
    operation: &format::Operation,
    missing_lock: bool,
) -> Result<bool, StorageError> {
    let store = root.optional("store.yaml")?;
    let witness = common.optional("work.identity.yaml")?;
    // Missing/invalid terminal evidence requires a fresh generation. Supported
    // intact evidence must still name this intent: another valid generation is
    // a conflict, even when some terminal directory or the lock is missing.
    let metadata = parse_optional_metadata(&store, &root.path.join("store.yaml"))?;
    let identity = parse_optional_identity(&witness, &common.path.join("work.identity.yaml"))?;
    if let Some(meta) = &metadata
        && (meta.store_id != operation.store_id
            || meta.recovery_generation != operation.next_generation)
    {
        return Err(files::conflict(
            &root.path.join("store.yaml"),
            "committed receipt does not match current validated generation",
        ));
    }
    if let Some(id) = &identity
        && *id != operation.store_id
    {
        return Err(StorageError::new(
            StorageErrorCode::IdentityMismatch,
            "committed receipt does not match retained local identity",
            Some(common.path.join("work.identity.yaml")),
        ));
    }
    let evidence_damaged = missing_lock || metadata.is_none() || identity.is_none();
    let result = (|| {
        let context = format::context(
            &dir.read("context.yaml")?.raw,
            operation,
            &dir.path.join("context.yaml"),
        )?;
        validate_terminal_structure(root, operation, &context)
    })();
    match result {
        Ok(()) => Ok(evidence_damaged),
        Err(error)
            if matches!(
                error.code,
                StorageErrorCode::StorageMissing
                    | StorageErrorCode::StorageCorrupt
                    | StorageErrorCode::UnsafePath
                    | StorageErrorCode::Conflict
                    | StorageErrorCode::InvalidFormat
            ) =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}
fn validate_terminal_structure(
    root: &files::Directory,
    operation: &format::Operation,
    context: &format::Context,
) -> Result<(), StorageError> {
    for name in DIRECTORIES {
        root.child(name)?;
    }
    if operation.kind == "recreate" {
        let archive = root.child("recovery")?.child(&operation.id)?;
        let prior = archive.child("prior")?;
        let prior_operations = archive.child("operations")?;
        for (name, identity) in &context.folders {
            check_archived(&prior, name, identity)?;
        }
        for (id, identity) in &context.operations {
            check_archived(&prior_operations, id, identity)?;
        }
    }
    Ok(())
}
fn archive_one(
    source: &files::Directory,
    archive: &files::Directory,
    name: &str,
    identity: &files::Identity,
) -> Result<(), StorageError> {
    match (source.exists(name)?, archive.exists(name)?) {
        (true, false) => source.rename_child(name, archive, identity),
        (false, true) => {
            check_archived(archive, name, identity)?;
            source.sync_archive_parents(archive)?;
            check_archived(archive, name, identity)
        }
        _ => Err(files::conflict(
            &archive.path.join(name),
            "archival requires exactly one original or retained directory",
        )),
    }
}
fn check_archived(
    archive: &files::Directory,
    name: &str,
    identity: &files::Identity,
) -> Result<(), StorageError> {
    if &archive.child(name)?.identity != identity {
        return Err(files::conflict(
            &archive.path.join(name),
            "retained directory differs from captured identity",
        ));
    }
    Ok(())
}
fn operation_result(
    result: Result<StorageOutcome, StorageError>,
    operation: &format::Operation,
    path: &std::path::Path,
    publication: Publication,
) -> Result<StorageOutcome, StorageError> {
    result.map_err(|mut error| {
        error.operation_id = Some(operation.id.clone());
        if publication != Publication::NotPublished {
            error.publication = publication;
        }
        error.recovery_paths.push(path.to_owned());
        error
    })
}

#[cfg(test)]
mod tests;

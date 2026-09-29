//! Shared SQLite coordination and derived source indexes.
//!
//! Durable item files are always read from the selected checkout. This module
//! records a reconciled projection; it never supplies durable completion state
//! to the graph evaluator.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rusqlite::{
    Connection, ErrorCode, MAIN_DB, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
    params,
};
use rustix::fs::{CWD, FlockOperation, Mode, OFlags, flock, openat};

use super::graph::ItemGraph;
use super::items::{Completion, Diagnostic, ItemFile, ItemStore, ManualState};
use super::project::Project;

const SCHEMA_VERSION: i64 = 1;
const RECOVERY_MARKER: &str = "recovery.in_progress";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageStatus {
    Uninitialized,
    Ready {
        store_id: String,
        schema_version: i64,
    },
    MissingDatabase,
    MissingIdentity,
    IdentityMismatch,
    Busy,
    Corrupt(String),
    UnsupportedSchema(i64),
}

impl StorageStatus {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::Ready { .. } => "ready",
            Self::MissingDatabase => "missing_database",
            Self::MissingIdentity => "missing_identity",
            Self::IdentityMismatch => "identity_mismatch",
            Self::Busy => "storage_busy",
            Self::Corrupt(_) => "corrupt_database",
            Self::UnsupportedSchema(_) => "unsupported_schema",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageInspection {
    pub database_path: PathBuf,
    pub identity_path: PathBuf,
    pub status: StorageStatus,
}

#[derive(Debug)]
pub enum StorageError {
    Status(StorageStatus),
    InvalidSource(Vec<Diagnostic>),
    InvalidArgument(String),
    MigrationFailed {
        backup_path: PathBuf,
        cause: Box<StorageError>,
    },
    Io(io::Error),
    Sqlite(rusqlite::Error),
}

impl StorageError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Status(status) => status.code(),
            Self::InvalidSource(_) => "invalid_source",
            Self::InvalidArgument(_) => "invalid_argument",
            Self::MigrationFailed { .. } => "migration_failed",
            Self::Io(_) => "io",
            Self::Sqlite(_) => "storage_io",
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(StorageStatus::Busy) => f.write_str(
                "coordination storage is busy; retry after the current SQLite operation completes",
            ),
            Self::Status(status) => write!(
                f,
                "coordination storage needs inspection or explicit recovery: {status:?}"
            ),
            Self::InvalidSource(diagnostics) => {
                write!(f, "selected source has {} diagnostic(s)", diagnostics.len())
            }
            Self::InvalidArgument(message) => f.write_str(message),
            Self::MigrationFailed { backup_path, cause } => write!(
                f,
                "migration failed; previous database retained and backup at {}: {cause}",
                backup_path.display()
            ),
            Self::Io(error) => write!(f, "storage I/O: {error}"),
            Self::Sqlite(error) => write!(f, "SQLite: {error}"),
        }
    }
}

impl std::error::Error for StorageError {}
impl From<io::Error> for StorageError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub view_root: PathBuf,
    pub changed_files: usize,
    pub removed_files: usize,
    pub unchanged_files: usize,
    pub changed_ephemeral_files: usize,
    pub removed_ephemeral_files: usize,
    pub generation: i64,
}

/// The caller must stop active executors before restoring or recreating a DB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub database_path: PathBuf,
    pub retained_paths: Vec<PathBuf>,
    pub lost_coordination: bool,
    /// A restored backup may predate coordination changes made after capture.
    pub coordination_may_be_stale: bool,
    /// The restored snapshot uses a recognized older schema and needs migration.
    pub requires_migration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub from_version: i64,
    pub to_version: i64,
    pub backup_path: PathBuf,
}

pub struct Storage {
    common_dir: PathBuf,
    connection: Connection,
}

impl Storage {
    /// Inspect without creating storage or modifying an existing database.
    pub fn inspect(project: &Project) -> Result<StorageInspection, StorageError> {
        let database_path = project.work_database_path();
        let identity_path = project.work_store_identity_path();
        let status = if recovery_marker_exists(project)? {
            StorageStatus::Corrupt(
                "storage recreation was interrupted; inspect retained files and recover explicitly"
                    .into(),
            )
        } else {
            let db_exists = regular_file_or_absent(&database_path)?;
            let identity_exists = regular_file_or_absent(&identity_path)?;
            match (db_exists, identity_exists) {
                (false, false) => StorageStatus::Uninitialized,
                (false, true) => StorageStatus::MissingDatabase,
                (true, false) => StorageStatus::MissingIdentity,
                (true, true) => inspect_existing(&database_path, &identity_path),
            }
        };
        Ok(StorageInspection {
            database_path,
            identity_path,
            status,
        })
    }

    /// First use initializes an entirely absent store. Any later fault is
    /// reported rather than silently replacing coordination state.
    pub fn open(project: &Project) -> Result<Self, StorageError> {
        ensure_storage_dir(project)?;
        let _lock = storage_lock(project)?;
        let inspection = Self::inspect(project)?;
        match inspection.status {
            StorageStatus::Uninitialized => initialize(project)?,
            StorageStatus::Ready { .. } => {}
            status => return Err(StorageError::Status(status)),
        }
        let status = Self::inspect(project)?.status;
        if !matches!(status, StorageStatus::Ready { .. }) {
            return Err(StorageError::Status(status));
        }
        let connection = open_rw(&inspection.database_path)?;
        let expected_id = read_identity(&inspection.identity_path)?;
        let opened_id: String = connection
            .query_row("SELECT store_id FROM store_meta", [], |row| row.get(0))
            .map_err(|error| StorageError::Status(StorageStatus::Corrupt(error.to_string())))?;
        if opened_id != expected_id {
            return Err(StorageError::Status(StorageStatus::IdentityMismatch));
        }
        Ok(Self {
            common_dir: project.git_common_dir.clone(),
            connection,
        })
    }

    /// Re-read selected files before this call. The returned graph remains the
    /// caller's file-derived graph; index rows are never a readiness source.
    pub fn reconcile(
        &mut self,
        project: &Project,
        store: &ItemStore,
    ) -> Result<ReconcileReport, StorageError> {
        self.update_projection(project, store, false)
    }

    /// Replace the selected view and shared ephemeral inventory atomically.
    /// Other worktree views and all coordination tables remain untouched.
    pub fn rebuild(
        &mut self,
        project: &Project,
        store: &ItemStore,
    ) -> Result<ReconcileReport, StorageError> {
        self.update_projection(project, store, true)
    }

    fn update_projection(
        &mut self,
        project: &Project,
        store: &ItemStore,
        rebuild: bool,
    ) -> Result<ReconcileReport, StorageError> {
        if self.common_dir != project.git_common_dir {
            return Err(StorageError::InvalidArgument(
                "selected checkout belongs to another Git repository".into(),
            ));
        }
        let graph = ItemGraph::from_store(store);
        if !graph.is_valid() {
            return Err(StorageError::InvalidSource(graph.diagnostics().to_vec()));
        }
        let ephemeral = scan_ephemeral(&project.work_storage_dir().join("runs"))?;
        let root = project.worktree_root.as_os_str().as_bytes();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO views(root, generation) VALUES (?1, 0)",
            params![root],
        )?;
        let previous = read_sources(&tx, root)?;
        let previous_ephemeral = read_ephemeral(&tx)?;
        if rebuild {
            tx.execute(
                "DELETE FROM source_files WHERE view_root = ?1",
                params![root],
            )?;
            tx.execute("DELETE FROM ephemeral_sources", [])?;
        }
        let mut changed_files = 0;
        let mut removed_files = 0;
        let mut unchanged_files = 0;
        let mut current = BTreeMap::new();
        for file in &store.files {
            let path = file
                .path
                .strip_prefix(&project.worktree_root)
                .map_err(|_| {
                    StorageError::InvalidArgument(
                        "item source lies outside selected checkout".into(),
                    )
                })?
                .as_os_str()
                .as_bytes()
                .to_vec();
            let digest = digest(&file.raw);
            let same = previous
                .get(&path)
                .is_some_and(|old_digest| *old_digest == digest);
            if same && !rebuild {
                unchanged_files += 1;
            } else {
                changed_files += 1;
                tx.execute(
                    "DELETE FROM source_files WHERE view_root = ?1 AND path = ?2",
                    params![root, path],
                )?;
                insert_source(&tx, root, &path, &digest, file)?;
            }
            current.insert(path, ());
        }
        for path in previous.keys() {
            if !current.contains_key(path) {
                removed_files += 1;
                if !rebuild {
                    tx.execute(
                        "DELETE FROM source_files WHERE view_root = ?1 AND path = ?2",
                        params![root, path],
                    )?;
                }
            }
        }
        let mut changed_ephemeral_files = 0;
        let mut removed_ephemeral_files = 0;
        for (path, source_digest) in &ephemeral {
            let same = previous_ephemeral
                .get(path)
                .is_some_and(|old_digest| old_digest == source_digest);
            if !same || rebuild {
                changed_ephemeral_files += 1;
                tx.execute(
                    "INSERT OR REPLACE INTO ephemeral_sources(path, source_digest) VALUES (?1, ?2)",
                    params![path, source_digest],
                )?;
            }
        }
        for path in previous_ephemeral.keys() {
            if !ephemeral.contains_key(path) {
                removed_ephemeral_files += 1;
                if !rebuild {
                    tx.execute(
                        "DELETE FROM ephemeral_sources WHERE path = ?1",
                        params![path],
                    )?;
                }
            }
        }
        if rebuild
            || changed_files != 0
            || removed_files != 0
            || changed_ephemeral_files != 0
            || removed_ephemeral_files != 0
        {
            tx.execute(
                "UPDATE views SET generation = generation + 1 WHERE root = ?1",
                params![root],
            )?;
        }
        let generation = tx.query_row(
            "SELECT generation FROM views WHERE root = ?1",
            params![root],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(ReconcileReport {
            view_root: project.worktree_root.clone(),
            changed_files,
            removed_files,
            unchanged_files,
            changed_ephemeral_files,
            removed_ephemeral_files,
            generation,
        })
    }

    /// Retain a consistent SQLite backup for an operator or a future schema
    /// migration. A backup is never restored automatically.
    pub fn backup(&self, project: &Project) -> Result<PathBuf, StorageError> {
        if self.common_dir != project.git_common_dir {
            return Err(StorageError::InvalidArgument(
                "selected checkout belongs to another Git repository".into(),
            ));
        }
        retain_backup(&self.connection, project)
    }

    /// Migrate a recognized pre-v1 store after retaining a consistent backup.
    /// Unknown future versions remain inspectable and require newer software.
    pub fn migrate(project: &Project) -> Result<MigrationReport, StorageError> {
        ensure_storage_dir(project)?;
        let _lock = storage_lock(project)?;
        let status = Self::inspect(project)?.status;
        if status != StorageStatus::UnsupportedSchema(0) {
            return Err(StorageError::Status(status));
        }
        let marker = read_identity(&project.work_store_identity_path())?;
        let mut connection = open_rw(&project.work_database_path())?;
        validate_coordination_schema(&connection)
            .map_err(|error| StorageError::Status(StorageStatus::Corrupt(error.to_string())))?;
        let stored: String = connection
            .query_row("SELECT store_id FROM store_meta", [], |row| row.get(0))
            .map_err(|error| StorageError::Status(StorageStatus::Corrupt(error.to_string())))?;
        if stored != marker {
            return Err(StorageError::Status(StorageStatus::IdentityMismatch));
        }
        let backup_path = retain_backup(&connection, project)?;
        (|| -> Result<(), StorageError> {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            drop_derived_tables(&tx)?;
            create_schema_v1(&tx)?;
            validate_schema_v1(&tx)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
            Ok(())
        })()
        .map_err(|cause| StorageError::MigrationFailed {
            backup_path: backup_path.clone(),
            cause: Box::new(cause),
        })?;
        let after = Self::inspect(project)?.status;
        if !matches!(after, StorageStatus::Ready { .. }) {
            return Err(StorageError::Status(after));
        }
        Ok(MigrationReport {
            from_version: 0,
            to_version: SCHEMA_VERSION,
            backup_path,
        })
    }

    /// Restore only a verified backup with the same store identity. Existing
    /// database files are retained for inspection. Active execution must stop.
    pub fn restore_backup(
        project: &Project,
        backup: &Path,
    ) -> Result<RecoveryReport, StorageError> {
        ensure_storage_dir(project)?;
        let _lock = storage_lock(project)?;
        let interrupted = recovery_marker_exists(project)?;
        let mut retained_paths = if interrupted {
            if !regular_file_or_absent(&recovery_marker_path(project))? {
                return Err(StorageError::InvalidArgument(
                    "recovery marker disappeared before restore".into(),
                ));
            }
            existing_retained_paths(project)?
        } else {
            Vec::new()
        };
        let identity = read_identity(&project.work_store_identity_path())?;
        match inspect_database(backup, &identity) {
            StorageStatus::Ready { .. } | StorageStatus::UnsupportedSchema(0) => {}
            status => return Err(StorageError::Status(status)),
        }
        let suffix = random_id()?;
        let staged = project
            .work_storage_dir()
            .join(format!(".work.db.restore-{suffix}"));
        let source = Connection::open_with_flags(
            backup,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        source.backup(MAIN_DB, &staged, None)?;
        let requires_migration = match inspect_database(&staged, &identity) {
            StorageStatus::Ready { .. } => false,
            StorageStatus::UnsupportedSchema(0) => true,
            status => return Err(StorageError::Status(status)),
        };
        File::open(&staged)?.sync_all()?;
        retained_paths.extend(quarantine_database(project, &suffix)?);
        fs::rename(&staged, project.work_database_path())?;
        sync_dir(&project.work_storage_dir())?;
        if interrupted {
            remove_recovery_marker(project)?;
        }
        Ok(RecoveryReport {
            database_path: project.work_database_path(),
            retained_paths,
            lost_coordination: false,
            coordination_may_be_stale: true,
            requires_migration,
        })
    }

    /// Deliberately start a fresh coordination epoch after reporting loss.
    /// Existing database and identity files are retained for investigation.
    /// Active execution must stop; prior claim tokens must never be reused.
    pub fn recreate(project: &Project) -> Result<RecoveryReport, StorageError> {
        ensure_storage_dir(project)?;
        let _lock = storage_lock(project)?;
        let status = Self::inspect(project)?.status;
        if matches!(
            status,
            StorageStatus::Ready { .. } | StorageStatus::Uninitialized
        ) {
            return Err(StorageError::InvalidArgument(
                "recreation requires a diagnosed storage fault".into(),
            ));
        }
        if status == StorageStatus::Busy {
            return Err(StorageError::Status(status));
        }
        let mut retained_paths = if recovery_marker_exists(project)? {
            existing_retained_paths(project)?
        } else {
            Vec::new()
        };
        create_recovery_marker(project)?;
        let suffix = random_id()?;
        retained_paths.extend(quarantine_database(project, &suffix)?);
        let identity_path = project.work_store_identity_path();
        if identity_path.exists() {
            let retained = project
                .work_storage_dir()
                .join(format!("store.id.retained-{suffix}"));
            fs::rename(&identity_path, &retained)?;
            retained_paths.push(retained);
        }
        initialize(project)?;
        remove_recovery_marker(project)?;
        Ok(RecoveryReport {
            database_path: project.work_database_path(),
            retained_paths,
            lost_coordination: true,
            coordination_may_be_stale: false,
            requires_migration: false,
        })
    }
}

fn schema(connection: &Connection, store_id: &str) -> Result<(), StorageError> {
    let tx = connection.unchecked_transaction()?;
    create_schema_v1(&tx)?;
    tx.execute(
        "INSERT INTO store_meta(store_id) VALUES (?1)",
        params![store_id],
    )?;
    validate_schema_v1(&tx)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

fn create_schema_v1(tx: &Transaction<'_>) -> Result<(), StorageError> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS store_meta (store_id TEXT NOT NULL PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS views (root BLOB PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS source_files (
            view_root BLOB NOT NULL, path BLOB NOT NULL, source_digest BLOB NOT NULL,
            PRIMARY KEY(view_root, path),
            FOREIGN KEY(view_root) REFERENCES views(root) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS items (
            view_root BLOB NOT NULL, id TEXT NOT NULL, source_path BLOB NOT NULL,
            title TEXT NOT NULL, completion TEXT NOT NULL, state TEXT,
            priority INTEGER NOT NULL, PRIMARY KEY(view_root, id),
            FOREIGN KEY(view_root, source_path) REFERENCES source_files(view_root, path) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS edges (
            view_root BLOB NOT NULL, source_id TEXT NOT NULL, relation TEXT NOT NULL,
            target_id TEXT NOT NULL, PRIMARY KEY(view_root, source_id, relation, target_id),
            FOREIGN KEY(view_root, source_id) REFERENCES items(view_root, id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS ephemeral_sources (path BLOB PRIMARY KEY, source_digest BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS claims (
            item_id TEXT PRIMARY KEY, owner_token TEXT NOT NULL UNIQUE,
            actor_id TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            run_id TEXT, session_id TEXT, workspace_id TEXT
        );
        CREATE TABLE IF NOT EXISTS runtime_records (
            kind TEXT NOT NULL, record_key TEXT NOT NULL, value BLOB NOT NULL,
            PRIMARY KEY(kind, record_key)
        );")?;
    Ok(())
}

fn drop_derived_tables(tx: &Transaction<'_>) -> Result<(), StorageError> {
    // Drop dependents first while foreign-key enforcement is enabled. The
    // transaction also restores the old projection if creation later fails.
    tx.execute_batch(
        "DROP TABLE IF EXISTS edges;
         DROP TABLE IF EXISTS items;
         DROP TABLE IF EXISTS source_files;
         DROP TABLE IF EXISTS views;
         DROP TABLE IF EXISTS ephemeral_sources;",
    )?;
    Ok(())
}

fn validate_schema_v1(connection: &Connection) -> Result<(), StorageError> {
    validate_coordination_schema(connection)?;
    const VIEWS: &[(&str, &str, bool, i64)] = &[
        ("root", "BLOB", false, 1),
        ("generation", "INTEGER", true, 0),
    ];
    const SOURCES: &[(&str, &str, bool, i64)] = &[
        ("view_root", "BLOB", true, 1),
        ("path", "BLOB", true, 2),
        ("source_digest", "BLOB", true, 0),
    ];
    const ITEMS: &[(&str, &str, bool, i64)] = &[
        ("view_root", "BLOB", true, 1),
        ("id", "TEXT", true, 2),
        ("source_path", "BLOB", true, 0),
        ("title", "TEXT", true, 0),
        ("completion", "TEXT", true, 0),
        ("state", "TEXT", false, 0),
        ("priority", "INTEGER", true, 0),
    ];
    const EDGES: &[(&str, &str, bool, i64)] = &[
        ("view_root", "BLOB", true, 1),
        ("source_id", "TEXT", true, 2),
        ("relation", "TEXT", true, 3),
        ("target_id", "TEXT", true, 4),
    ];
    const EPHEMERAL: &[(&str, &str, bool, i64)] = &[
        ("path", "BLOB", false, 1),
        ("source_digest", "BLOB", true, 0),
    ];
    for (table, expected) in [
        ("views", VIEWS),
        ("source_files", SOURCES),
        ("items", ITEMS),
        ("edges", EDGES),
        ("ephemeral_sources", EPHEMERAL),
    ] {
        validate_columns(connection, table, expected)?;
    }
    for (table, expected) in [
        (
            "source_files",
            &[(0, 0, "views", "view_root", "root", "CASCADE")][..],
        ),
        (
            "items",
            &[
                (0, 0, "source_files", "view_root", "view_root", "CASCADE"),
                (0, 1, "source_files", "source_path", "path", "CASCADE"),
            ][..],
        ),
        (
            "edges",
            &[
                (0, 0, "items", "view_root", "view_root", "CASCADE"),
                (0, 1, "items", "source_id", "id", "CASCADE"),
            ][..],
        ),
    ] {
        let mut statement = connection.prepare(&format!("PRAGMA foreign_key_list({table})"))?;
        let actual: Vec<(i64, i64, String, String, String, String)> = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(6)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if actual.len() != expected.len()
            || actual.iter().zip(expected).any(|(foreign, required)| {
                foreign.0 != required.0
                    || foreign.1 != required.1
                    || foreign.2 != required.2
                    || foreign.3 != required.3
                    || foreign.4 != required.4
                    || foreign.5 != required.5
            })
        {
            return Err(StorageError::InvalidArgument(format!(
                "schema foreign key for {table} does not match version 1"
            )));
        }
    }
    let violations: i64 =
        connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if violations != 0 {
        return Err(StorageError::InvalidArgument(format!(
            "schema contains {violations} foreign key violation(s)"
        )));
    }
    Ok(())
}

/// Version 0 is recognized only when its nonderivable coordination data is
/// intact. Derived view tables may be repaired by the v1 migration.
fn validate_coordination_schema(connection: &Connection) -> Result<(), StorageError> {
    const META: &[(&str, &str, bool, i64)] = &[("store_id", "TEXT", true, 1)];
    const CLAIMS: &[(&str, &str, bool, i64)] = &[
        ("item_id", "TEXT", false, 1),
        ("owner_token", "TEXT", true, 0),
        ("actor_id", "TEXT", true, 0),
        ("created_at", "TEXT", true, 0),
        ("updated_at", "TEXT", true, 0),
        ("run_id", "TEXT", false, 0),
        ("session_id", "TEXT", false, 0),
        ("workspace_id", "TEXT", false, 0),
    ];
    const RECORDS: &[(&str, &str, bool, i64)] = &[
        ("kind", "TEXT", true, 1),
        ("record_key", "TEXT", true, 2),
        ("value", "BLOB", true, 0),
    ];
    for (table, expected) in [
        ("store_meta", META),
        ("claims", CLAIMS),
        ("runtime_records", RECORDS),
    ] {
        validate_columns(connection, table, expected)?;
        let mut statement = connection.prepare(&format!("PRAGMA foreign_key_list({table})"))?;
        if statement.query([])?.next()?.is_some() {
            return Err(StorageError::InvalidArgument(format!(
                "schema coordination table {table} must not have foreign keys"
            )));
        }
    }
    if !has_unique_column(connection, "claims", "owner_token")? {
        return Err(StorageError::InvalidArgument(
            "schema claims.owner_token must be unique".into(),
        ));
    }
    Ok(())
}

fn validate_columns(
    connection: &Connection,
    table: &str,
    expected: &[(&str, &str, bool, i64)],
) -> Result<(), StorageError> {
    let kind: Option<String> = connection
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    if kind.as_deref() != Some("table") {
        return Err(StorageError::InvalidArgument(format!(
            "schema table {table} is missing or is not a table"
        )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let actual: Vec<(String, String, bool, i64)> = statement
        .query_map([], |row| {
            Ok((
                row.get(1)?,
                row.get(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if actual.len() != expected.len()
        || actual.iter().zip(expected).any(|(column, required)| {
            column.0 != required.0
                || !column.1.eq_ignore_ascii_case(required.1)
                || column.2 != required.2
                || column.3 != required.3
        })
    {
        return Err(StorageError::InvalidArgument(format!(
            "schema table {table} does not match the recognized layout"
        )));
    }
    Ok(())
}

fn has_unique_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, StorageError> {
    let mut statement = connection.prepare(&format!("PRAGMA index_list({table})"))?;
    let indexes: Vec<(String, bool, bool)> = statement
        .query_map([], |row| {
            Ok((
                row.get(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(4)? != 0,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (name, unique, partial) in indexes {
        if !unique || partial {
            continue;
        }
        let mut statement = connection.prepare("SELECT name FROM pragma_index_info(?1)")?;
        let columns: Vec<String> = statement
            .query_map([name], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if columns == [column] {
            return Ok(true);
        }
    }
    Ok(false)
}

fn retain_backup(connection: &Connection, project: &Project) -> Result<PathBuf, StorageError> {
    let suffix = random_id()?;
    let destination = project
        .work_storage_dir()
        .join(format!("work.db.backup-{suffix}"));
    let staging = project
        .work_storage_dir()
        .join(format!(".work.db.backup-{suffix}"));
    connection.backup(MAIN_DB, &staging, None)?;
    File::open(&staging)?.sync_all()?;
    fs::rename(&staging, &destination)?;
    sync_dir(&project.work_storage_dir())?;
    Ok(destination)
}

fn initialize(project: &Project) -> Result<(), StorageError> {
    let store_id = random_id()?;
    let identity_path = project.work_store_identity_path();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&identity_path)?;
    writeln!(file, "{store_id}")?;
    file.sync_all()?;
    sync_dir(&project.work_storage_dir())?;
    let connection = Connection::open_with_flags(
        project.work_database_path(),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    schema(&connection, &store_id)?;
    File::open(project.work_database_path())?.sync_all()?;
    sync_dir(&project.work_storage_dir())?;
    Ok(())
}

fn inspect_existing(database: &Path, identity: &Path) -> StorageStatus {
    let marker = match read_identity(identity) {
        Ok(marker) => marker,
        Err(error) => return StorageStatus::Corrupt(format!("invalid store identity: {error}")),
    };
    inspect_database(database, &marker)
}

fn inspect_database(database: &Path, marker: &str) -> StorageStatus {
    let connection = match Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ) {
        Ok(connection) => connection,
        Err(error) => return inspection_error(error.into()),
    };
    if let Err(error) = connection.busy_timeout(std::time::Duration::from_millis(250)) {
        return inspection_error(error.into());
    }
    let result = (|| -> rusqlite::Result<(String, i64)> {
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok((integrity, version))
    })();
    match result {
        Ok((integrity, _)) if integrity != "ok" => StorageStatus::Corrupt(integrity),
        Ok((_, 0)) => {
            if let Err(error) = validate_coordination_schema(&connection) {
                return inspection_error(error);
            }
            match connection.query_row("SELECT store_id FROM store_meta", [], |row| {
                row.get::<_, String>(0)
            }) {
                Ok(store_id) if store_id != marker => StorageStatus::IdentityMismatch,
                Ok(_) => StorageStatus::UnsupportedSchema(0),
                Err(error) => inspection_error(error.into()),
            }
        }
        Ok((_, version)) if version != SCHEMA_VERSION => StorageStatus::UnsupportedSchema(version),
        Ok((_, schema_version)) => {
            if let Err(error) = validate_schema_v1(&connection) {
                return inspection_error(error);
            }
            match connection.query_row("SELECT store_id FROM store_meta", [], |row| {
                row.get::<_, String>(0)
            }) {
                Ok(store_id) if store_id != marker => StorageStatus::IdentityMismatch,
                Ok(store_id) => StorageStatus::Ready {
                    store_id,
                    schema_version,
                },
                Err(error) => inspection_error(error.into()),
            }
        }
        Err(error) => inspection_error(error.into()),
    }
}

fn inspection_error(error: StorageError) -> StorageStatus {
    match &error {
        StorageError::Sqlite(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(
                failure.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            StorageStatus::Busy
        }
        _ => StorageStatus::Corrupt(error.to_string()),
    }
}

fn open_rw(path: &Path) -> Result<Connection, StorageError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

fn regular_file_or_absent(path: &Path) -> Result<bool, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(StorageError::InvalidArgument(format!(
            "storage path is not a regular file: {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn recovery_marker_path(project: &Project) -> PathBuf {
    project.work_storage_dir().join(RECOVERY_MARKER)
}

fn recovery_marker_exists(project: &Project) -> Result<bool, StorageError> {
    match fs::symlink_metadata(recovery_marker_path(project)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn create_recovery_marker(project: &Project) -> Result<(), StorageError> {
    let path = recovery_marker_path(project);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(b"recreate in progress\n")?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if !regular_file_or_absent(&path)? {
                return Err(StorageError::InvalidArgument(
                    "recovery marker disappeared during recreation".into(),
                ));
            }
        }
        Err(error) => return Err(error.into()),
    }
    sync_dir(&project.work_storage_dir())?;
    Ok(())
}

fn remove_recovery_marker(project: &Project) -> Result<(), StorageError> {
    let path = recovery_marker_path(project);
    if !regular_file_or_absent(&path)? {
        return Err(StorageError::InvalidArgument(
            "recovery marker disappeared before completion".into(),
        ));
    }
    fs::remove_file(path)?;
    sync_dir(&project.work_storage_dir())?;
    Ok(())
}

fn existing_retained_paths(project: &Project) -> Result<Vec<PathBuf>, StorageError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(project.work_storage_dir())? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if [
            "work.db.retained-",
            "work.db-wal.retained-",
            "work.db-shm.retained-",
            "store.id.retained-",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
            && entry.file_type()?.is_file()
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn ensure_storage_dir(project: &Project) -> Result<(), StorageError> {
    let path = project.work_storage_dir();
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(StorageError::InvalidArgument(format!(
            "storage path is not a real directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    match fs::symlink_metadata(&path) {
                        Ok(metadata) if metadata.file_type().is_dir() => {}
                        Ok(_) => {
                            return Err(StorageError::InvalidArgument(format!(
                                "storage path is not a real directory: {}",
                                path.display()
                            )));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
            sync_dir(&project.git_common_dir)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn storage_lock(project: &Project) -> Result<File, StorageError> {
    let file = File::from(
        openat(
            CWD,
            project.work_storage_dir().join("storage.lock"),
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(io::Error::from)?,
    );
    flock(&file, FlockOperation::LockExclusive).map_err(io::Error::from)?;
    Ok(file)
}

fn read_identity(path: &Path) -> Result<String, StorageError> {
    let mut file = File::from(
        openat(
            CWD,
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StorageError::InvalidArgument(
            "store identity is malformed".into(),
        ));
    }
    Ok(value.to_owned())
}

fn random_id() -> Result<String, StorageError> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn sync_dir(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn quarantine_database(project: &Project, suffix: &str) -> Result<Vec<PathBuf>, StorageError> {
    let mut retained = Vec::new();
    let database = project.work_database_path();
    for path in [
        database.clone(),
        database.with_extension("db-wal"),
        database.with_extension("db-shm"),
    ] {
        if regular_file_or_absent(&path)? {
            let name = path
                .file_name()
                .ok_or_else(|| StorageError::InvalidArgument("invalid database path".into()))?;
            let mut retained_name = name.to_os_string();
            retained_name.push(format!(".retained-{suffix}"));
            let target = path.with_file_name(retained_name);
            fs::rename(&path, &target)?;
            retained.push(target);
        }
    }
    sync_dir(&project.work_storage_dir())?;
    Ok(retained)
}

fn read_sources(
    tx: &Transaction<'_>,
    root: &[u8],
) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, StorageError> {
    let mut statement =
        tx.prepare("SELECT path, source_digest FROM source_files WHERE view_root = ?1")?;
    let rows = statement.query_map(params![root], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn read_ephemeral(tx: &Transaction<'_>) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, StorageError> {
    let mut statement = tx.prepare("SELECT path, source_digest FROM ephemeral_sources")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn insert_source(
    tx: &Transaction<'_>,
    root: &[u8],
    path: &[u8],
    source_digest: &[u8],
    file: &ItemFile,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO source_files(view_root, path, source_digest) VALUES (?1, ?2, ?3)",
        params![root, path, source_digest],
    )?;
    let header = file
        .header
        .as_ref()
        .expect("validated graph contains only valid files");
    let completion = match header.completion {
        Completion::Manual => "manual",
        Completion::Children => "children",
    };
    let state = match header.state {
        Some(ManualState::Open) => Some("open"),
        Some(ManualState::Done) => Some("done"),
        None => None,
    };
    tx.execute("INSERT INTO items(view_root, id, source_path, title, completion, state, priority) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![root, header.id, path, header.title, completion, state, header.priority])?;
    if let Some(parent) = &header.parent {
        insert_edge(tx, root, &header.id, "parent", parent)?;
    }
    for target in &header.depends_on {
        insert_edge(tx, root, &header.id, "depends_on", target)?;
    }
    for target in &header.related {
        insert_edge(tx, root, &header.id, "related", target)?;
    }
    for target in &header.discovered_from {
        insert_edge(tx, root, &header.id, "discovered_from", target)?;
    }
    Ok(())
}

fn insert_edge(
    tx: &Transaction<'_>,
    root: &[u8],
    source: &str,
    relation: &str,
    target: &str,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO edges(view_root, source_id, relation, target_id) VALUES (?1, ?2, ?3, ?4)",
        params![root, source, relation, target],
    )?;
    Ok(())
}

fn scan_ephemeral(root: &Path) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, StorageError> {
    let mut result = BTreeMap::new();
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(StorageError::InvalidArgument(format!(
                "run source root is not a real directory: {}",
                root.display()
            )));
        }
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(StorageError::InvalidArgument(format!(
                    "ephemeral source is a symlink: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root).expect("walked beneath run root");
                result.insert(
                    relative.as_os_str().as_bytes().to_vec(),
                    digest_file(&path)?,
                );
            } else {
                return Err(StorageError::InvalidArgument(format!(
                    "ephemeral source is not a regular file: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(result)
}

/// SHA-256 source digest for derived file indexes.
fn digest(raw: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(raw);
    hasher.finish()
}

fn digest_file(path: &Path) -> Result<Vec<u8>, StorageError> {
    let mut file = File::from(
        openat(
            CWD,
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finish())
}

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    used: usize,
    byte_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            used: 0,
            byte_len: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.byte_len = self.byte_len.wrapping_add(input.len() as u64);
        if self.used != 0 {
            let take = (64 - self.used).min(input.len());
            self.block[self.used..self.used + take].copy_from_slice(&input[..take]);
            self.used += take;
            input = &input[take..];
            if self.used < 64 {
                return;
            }
            let block = self.block;
            self.compress(&block);
            self.used = 0;
        }
        while input.len() >= 64 {
            let block: &[u8; 64] = input[..64].try_into().expect("64-byte block");
            self.compress(block);
            input = &input[64..];
        }
        self.block[..input.len()].copy_from_slice(input);
        self.used = input.len();
    }

    fn finish(mut self) -> Vec<u8> {
        let bit_len = self.byte_len.wrapping_mul(8);
        self.block[self.used] = 0x80;
        self.used += 1;
        if self.used > 56 {
            self.block[self.used..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.used = 0;
        }
        self.block[self.used..56].fill(0);
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);
        self.state.into_iter().flat_map(u32::to_be_bytes).collect()
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut words = [0_u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(chunk.try_into().expect("four-byte word"));
        }
        for index in 16..64 {
            let a = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let b = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(a)
                .wrapping_add(words[index - 7])
                .wrapping_add(b);
        }
        let mut working = self.state;
        for index in 0..64 {
            let choose = (working[4] & working[5]) ^ (!working[4] & working[6]);
            let majority =
                (working[0] & working[1]) ^ (working[0] & working[2]) ^ (working[1] & working[2]);
            let a = working[0].rotate_right(2)
                ^ working[0].rotate_right(13)
                ^ working[0].rotate_right(22);
            let b = working[4].rotate_right(6)
                ^ working[4].rotate_right(11)
                ^ working[4].rotate_right(25);
            let first = working[7]
                .wrapping_add(b)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let second = a.wrapping_add(majority);
            working = [
                first.wrapping_add(second),
                working[0],
                working[1],
                working[2],
                working[3].wrapping_add(first),
                working[4],
                working[5],
                working[6],
            ];
        }
        for (value, updated) in self.state.iter_mut().zip(working) {
            *value = value.wrapping_add(updated);
        }
    }
}

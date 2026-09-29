use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use rusqlite::{Connection, params};
use work::core::graph::ItemGraph;
use work::core::items::ItemStore;
use work::core::project::{Project, discover};
use work::core::storage::{Storage, StorageError, StorageStatus};

static NEXT: AtomicU64 = AtomicU64::new(0);
const FIRST: &str = "11111111000040008000000000000000";
const SECOND: &str = "22222222000040008000000000000000";

struct Fixture {
    root: PathBuf,
    checkout: PathBuf,
    linked: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "work storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let checkout = root.join("checkout");
        let linked = root.join("linked");
        fs::create_dir_all(checkout.join(".work/items")).unwrap();
        git(&checkout, &["init", "--initial-branch=main"]);
        write_item(&checkout, FIRST, "Original", "open", None);
        write_item(&checkout, SECOND, "Dependent", "open", Some(FIRST));
        git(&checkout, &["add", ".work/items"]);
        git(
            &checkout,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );
        git(
            &checkout,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        Self {
            root,
            checkout,
            linked,
        }
    }

    fn project(&self, path: &Path) -> Project {
        discover(Some(path)).unwrap()
    }
    fn store(&self, path: &Path) -> ItemStore {
        ItemStore::load(&self.project(path)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn git(path: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_item(root: &Path, id: &str, title: &str, state: &str, depends_on: Option<&str>) {
    let relation = depends_on.map_or(String::new(), |target| {
        format!("depends_on: [\"{target}\"]\n")
    });
    fs::create_dir_all(root.join(".work/items")).unwrap();
    fs::write(root.join(".work/items").join(format!("{id}.md")), format!(
        "---\nformat_version: 1\nid: \"{id}\"\ntitle: {title}\nstate: {state}\n{relation}---\nBody\n"
    )).unwrap();
}

fn indexed_state(connection: &Connection, project: &Project, id: &str) -> (String, String) {
    connection
        .query_row(
            "SELECT title, state FROM items WHERE view_root = ?1 AND id = ?2",
            params![project.worktree_root.as_os_str().as_encoded_bytes(), id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[test]
fn concurrent_first_open_shares_one_initialized_store() {
    let fixture = Fixture::new();
    let project = Arc::new(fixture.project(&fixture.checkout));
    assert!(!project.work_storage_dir().exists());
    let barrier = Arc::new(Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let project = Arc::clone(&project);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                drop(Storage::open(&project).unwrap());
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
}

#[test]
fn linked_views_reconcile_uncommitted_changes_without_leaking_readiness() {
    let fixture = Fixture::new();
    let main = fixture.project(&fixture.checkout);
    let linked = fixture.project(&fixture.linked);
    assert_eq!(main.work_database_path(), linked.work_database_path());
    assert_eq!(
        Storage::inspect(&main).unwrap().status,
        StorageStatus::Uninitialized
    );
    let mut storage = Storage::open(&main).unwrap();
    storage
        .reconcile(&main, &fixture.store(&fixture.checkout))
        .unwrap();
    write_item(&fixture.linked, FIRST, "Linked done", "done", None);
    let linked_store = fixture.store(&fixture.linked);
    assert_eq!(
        ItemGraph::from_store(&linked_store)
            .ready()
            .unwrap()
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![SECOND]
    );
    storage.reconcile(&linked, &linked_store).unwrap();
    let connection = Connection::open(main.work_database_path()).unwrap();
    assert_eq!(
        indexed_state(&connection, &main, FIRST),
        ("Original".into(), "open".into())
    );
    assert_eq!(
        indexed_state(&connection, &linked, FIRST),
        ("Linked done".into(), "done".into())
    );
    assert_eq!(
        ItemGraph::from_store(&fixture.store(&fixture.checkout))
            .ready()
            .unwrap()
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![FIRST]
    );

    write_item(&fixture.linked, FIRST, "Linked reopened", "open", None);
    let changed = storage
        .reconcile(&linked, &fixture.store(&fixture.linked))
        .unwrap();
    assert_eq!(changed.changed_files, 1);
    assert_eq!(
        indexed_state(&connection, &linked, FIRST),
        ("Linked reopened".into(), "open".into())
    );
    assert_eq!(
        indexed_state(&connection, &main, FIRST),
        ("Original".into(), "open".into())
    );

    fs::remove_file(
        fixture
            .linked
            .join(".work/items")
            .join(format!("{SECOND}.md")),
    )
    .unwrap();
    let removed = storage
        .reconcile(&linked, &fixture.store(&fixture.linked))
        .unwrap();
    assert_eq!(removed.removed_files, 1);
    assert_eq!(
        connection
            .query_row::<i64, _, _>(
                "SELECT count(*) FROM items WHERE view_root = ?1 AND id = ?2",
                params![linked.worktree_root.as_os_str().as_encoded_bytes(), SECOND],
                |row| row.get(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(indexed_state(&connection, &main, SECOND).0, "Dependent");
}

#[test]
fn rebuild_rolls_back_on_failure_and_preserves_coordination_and_run_inventory() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let runs = project.work_storage_dir().join("runs/run-one");
    fs::create_dir_all(&runs).unwrap();
    fs::write(runs.join("retained.bin"), b"abc").unwrap();
    let mut storage = Storage::open(&project).unwrap();
    storage
        .reconcile(&project, &fixture.store(&fixture.checkout))
        .unwrap();
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    connection.execute("INSERT INTO runtime_records(kind, record_key, value) VALUES ('observation', 'one', X'CAFE')", []).unwrap();
    let digest: String = connection
        .query_row(
            "SELECT hex(source_digest) FROM ephemeral_sources WHERE path = ?1",
            [b"run-one/retained.bin".as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        digest,
        "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
    );
    write_item(&fixture.checkout, FIRST, "Changed", "done", None);
    connection.execute_batch("CREATE TRIGGER fail_rebuild BEFORE INSERT ON source_files BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    assert!(
        storage
            .rebuild(&project, &fixture.store(&fixture.checkout))
            .is_err()
    );
    assert_eq!(
        indexed_state(&connection, &project, FIRST),
        ("Original".into(), "open".into())
    );
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "token"
    );
    assert_eq!(
        connection
            .query_row::<Vec<u8>, _, _>(
                "SELECT value FROM runtime_records WHERE record_key = 'one'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        b"\xca\xfe"
    );
    connection
        .execute_batch("DROP TRIGGER fail_rebuild")
        .unwrap();
    storage
        .rebuild(&project, &fixture.store(&fixture.checkout))
        .unwrap();
    assert_eq!(
        indexed_state(&connection, &project, FIRST),
        ("Changed".into(), "done".into())
    );
    assert!(runs.join("retained.bin").exists());
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM ephemeral_sources", [], |row| row
                .get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM claims", [], |row| row.get(0))
            .unwrap(),
        1
    );
    drop(storage);
    let mut restarted = Storage::open(&project).unwrap();
    let report = restarted
        .reconcile(&project, &fixture.store(&fixture.checkout))
        .unwrap();
    assert_eq!(report.changed_files, 0);
    assert_eq!(report.unchanged_files, 2);
}

#[test]
fn missing_and_corrupt_database_require_explicit_recovery() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let storage = Storage::open(&project).unwrap();
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'saved-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    drop(connection);
    let backup = storage.backup(&project).unwrap();
    drop(storage);
    fs::remove_file(project.work_database_path()).unwrap();
    assert_eq!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::MissingDatabase
    );
    assert!(matches!(
        Storage::open(&project),
        Err(StorageError::Status(StorageStatus::MissingDatabase))
    ));
    assert!(
        !ItemGraph::from_store(&fixture.store(&fixture.checkout))
            .ready()
            .unwrap()
            .is_empty()
    );
    let restored = Storage::restore_backup(&project, &backup).unwrap();
    assert!(!restored.lost_coordination);
    assert!(restored.coordination_may_be_stale);
    let connection = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "saved-token"
    );
    drop(connection);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
    fs::write(project.work_database_path(), b"not a sqlite database").unwrap();
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(_)
    ));
    assert!(matches!(
        Storage::open(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(_)))
    ));
    let recreated = Storage::recreate(&project).unwrap();
    assert!(recreated.lost_coordination);
    assert!(!recreated.retained_paths.is_empty());
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
}

#[test]
fn exclusive_sqlite_lock_is_retryable_busy_and_does_not_recreate_store() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let before = Storage::inspect(&project).unwrap().status;
    let locker = Connection::open(project.work_database_path()).unwrap();
    locker.execute_batch("BEGIN EXCLUSIVE").unwrap();

    let inspection = Storage::inspect(&project).unwrap();
    assert_eq!(inspection.status, StorageStatus::Busy);
    assert_eq!(inspection.status.code(), "storage_busy");
    let open_error = match Storage::open(&project) {
        Ok(_) => panic!("locked storage unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(
        open_error,
        StorageError::Status(StorageStatus::Busy)
    ));
    assert!(open_error.to_string().contains("retry"));
    assert!(matches!(
        Storage::recreate(&project),
        Err(StorageError::Status(StorageStatus::Busy))
    ));
    assert!(
        !project
            .work_storage_dir()
            .join("recovery.in_progress")
            .exists()
    );
    assert!(project.work_database_path().exists());
    assert!(project.work_store_identity_path().exists());

    locker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(Storage::inspect(&project).unwrap().status, before);
    drop(Storage::open(&project).unwrap());
}

#[test]
fn immediate_writer_reports_storage_busy_and_reconcile_succeeds_after_release() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let mut storage = Storage::open(&project).unwrap();
    let items = fixture.store(&fixture.checkout);
    assert!(!ItemGraph::from_store(&items).ready().unwrap().is_empty());
    let locker = Connection::open(project.work_database_path()).unwrap();
    locker.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));

    let error = storage.reconcile(&project, &items).unwrap_err();
    assert_eq!(error.code(), "storage_busy");
    assert!(error.to_string().contains("retry"));
    locker.execute_batch("ROLLBACK").unwrap();
    let report = storage.reconcile(&project, &items).unwrap();
    assert_eq!(report.changed_files, 2);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
}

#[test]
fn unreadable_intact_database_is_unavailable_without_claim_loss() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let database = project.work_database_path();
    let connection = Connection::open(&database).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'unreadable-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    drop(connection);
    let mode = fs::metadata(&database).unwrap().permissions().mode();
    fs::set_permissions(&database, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&database).is_ok() {
        // Privileged runners can bypass file modes, so they cannot exercise
        // SQLite's permission-denied path with this fixture.
        fs::set_permissions(&database, fs::Permissions::from_mode(mode)).unwrap();
        return;
    }

    let inspection = Storage::inspect(&project).unwrap();
    assert!(matches!(inspection.status, StorageStatus::Unavailable(_)));
    assert_eq!(inspection.status.code(), "storage_unavailable");
    let open_error = match Storage::open(&project) {
        Ok(_) => panic!("unreadable database unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(
        open_error,
        StorageError::Status(StorageStatus::Unavailable(_))
    ));
    assert!(open_error.to_string().contains("repair access"));
    assert!(matches!(
        Storage::recreate(&project),
        Err(StorageError::Status(StorageStatus::Unavailable(_)))
    ));
    assert!(
        !project
            .work_storage_dir()
            .join("recovery.in_progress")
            .exists()
    );

    fs::set_permissions(&database, fs::Permissions::from_mode(mode)).unwrap();
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "unreadable-token"
    );
}

#[test]
fn interrupted_recreation_cannot_silently_initialize_and_can_be_resumed() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'retained-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    drop(connection);
    let marker = project.work_storage_dir().join("recovery.in_progress");
    let retained_db = project
        .work_storage_dir()
        .join("work.db.retained-interrupted");
    let retained_identity = project
        .work_storage_dir()
        .join("store.id.retained-interrupted");
    fs::write(&marker, b"recreate in progress\n").unwrap();
    fs::rename(project.work_database_path(), &retained_db).unwrap();
    fs::rename(project.work_store_identity_path(), &retained_identity).unwrap();

    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(detail) if detail.contains("recreation was interrupted")
    ));
    assert!(matches!(
        Storage::open(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(_)))
    ));
    assert!(!project.work_database_path().exists());
    assert!(!project.work_store_identity_path().exists());

    let report = Storage::recreate(&project).unwrap();
    assert!(report.lost_coordination);
    assert!(report.retained_paths.contains(&retained_db));
    assert!(report.retained_paths.contains(&retained_identity));
    assert!(!marker.exists());
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
    let retained = Connection::open(&retained_db).unwrap();
    assert_eq!(
        retained
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "retained-token"
    );
}

#[test]
fn explicit_backup_restore_clears_interrupted_recreation_marker() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let storage = Storage::open(&project).unwrap();
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'backup-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    drop(connection);
    let backup = storage.backup(&project).unwrap();
    drop(storage);
    let marker = project.work_storage_dir().join("recovery.in_progress");
    let retained_db = project
        .work_storage_dir()
        .join("work.db.retained-interrupted");
    fs::write(&marker, b"recreate in progress\n").unwrap();
    fs::rename(project.work_database_path(), &retained_db).unwrap();

    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(_)
    ));
    let restored = Storage::restore_backup(&project, &backup).unwrap();
    assert!(!restored.lost_coordination);
    assert!(restored.retained_paths.contains(&retained_db));
    assert!(!marker.exists());
    assert!(retained_db.exists());
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "backup-token"
    );
}

#[test]
fn missing_required_table_is_reported_as_corrupt() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute_batch("DROP TABLE claims").unwrap();
    drop(connection);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(_)
    ));
    assert!(matches!(
        Storage::open(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(_)))
    ));
}

#[test]
fn missing_claim_uniqueness_is_reported_as_corrupt() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE claims;
        CREATE TABLE claims (
            item_id TEXT PRIMARY KEY, owner_token TEXT NOT NULL,
            actor_id TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            run_id TEXT, session_id TEXT, workspace_id TEXT
        );",
        )
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(detail) if detail.contains("owner_token must be unique")
    ));
    assert!(matches!(
        Storage::open(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(_)))
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection
        .execute_batch("CREATE UNIQUE INDEX only_some_tokens ON claims(owner_token) WHERE owner_token != 'excluded'")
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(detail) if detail.contains("owner_token must be unique")
    ));
    assert!(
        !ItemGraph::from_store(&fixture.store(&fixture.checkout))
            .ready()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn claim_cascade_to_derived_view_is_rejected_in_v1_and_schema_zero() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE claims;
             CREATE TABLE claims (
                 item_id TEXT PRIMARY KEY, owner_token TEXT NOT NULL UNIQUE,
                 actor_id TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                 run_id TEXT, session_id TEXT, workspace_id TEXT,
                 FOREIGN KEY(item_id) REFERENCES views(root) ON DELETE CASCADE
             );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO views(root, generation) VALUES (?1, 0)",
            [FIRST],
        )
        .unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'cascade-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap(),
        0
    );
    drop(connection);

    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(detail)
            if detail.contains("coordination table claims must not have foreign keys")
    ));
    assert!(matches!(
        Storage::open(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(_)))
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute_batch("PRAGMA user_version=0").unwrap();
    drop(connection);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(detail)
            if detail.contains("coordination table claims must not have foreign keys")
    ));
    assert!(matches!(
        Storage::migrate(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(_)))
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "cascade-token"
    );
}

#[test]
fn missing_derived_foreign_key_is_reported_as_corrupt() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE source_files;
        CREATE TABLE source_files (
            view_root BLOB NOT NULL, path BLOB NOT NULL, source_digest BLOB NOT NULL,
            PRIMARY KEY(view_root, path)
        );",
        )
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Corrupt(detail) if detail.contains("foreign key for source_files")
    ));
}

#[test]
fn restore_includes_claim_committed_only_to_backup_wal() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let storage = Storage::open(&project).unwrap();
    let backup = storage.backup(&project).unwrap();
    drop(storage);
    let wal_connection = Connection::open(&backup).unwrap();
    let mode: String = wal_connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    wal_connection
        .execute_batch("PRAGMA wal_autocheckpoint=0")
        .unwrap();
    wal_connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'wal-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    assert!(
        backup
            .with_file_name(format!(
                "{}-wal",
                backup.file_name().unwrap().to_string_lossy()
            ))
            .exists()
    );
    let main_only = project.work_storage_dir().join("main-only-copy.db");
    fs::copy(&backup, &main_only).unwrap();
    let main_connection = Connection::open(&main_only).unwrap();
    assert_eq!(
        main_connection
            .query_row::<i64, _, _>("SELECT count(*) FROM claims", [], |row| row.get(0))
            .unwrap(),
        0
    );
    drop(main_connection);

    Storage::restore_backup(&project, &backup).unwrap();
    let restored = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        restored
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "wal-token"
    );
}

#[test]
fn large_run_file_is_indexed_by_streamed_sha256_digest() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let runs = project.work_storage_dir().join("runs/run-large");
    let mut storage = Storage::open(&project).unwrap();
    fs::create_dir_all(&runs).unwrap();
    let path = runs.join("large.bin");
    fs::write(&path, vec![b'a'; 1_000_000]).unwrap();
    let report = storage
        .reconcile(&project, &fixture.store(&fixture.checkout))
        .unwrap();
    assert_eq!(report.changed_ephemeral_files, 1);
    let connection = Connection::open(project.work_database_path()).unwrap();
    let digest: String = connection
        .query_row(
            "SELECT hex(source_digest) FROM ephemeral_sources WHERE path = ?1",
            [b"run-large/large.bin".as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        digest,
        "CDC76E5C9914FB9281A1C7E284D73E67F1809A48A497200E046D39CCC7112CD0"
    );
    let columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('ephemeral_sources')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(columns, 2);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    use std::io::Write as _;
    file.write_all(b"b").unwrap();
    let changed = storage
        .reconcile(&project, &fixture.store(&fixture.checkout))
        .unwrap();
    assert_eq!(changed.changed_ephemeral_files, 1);
}

#[test]
fn encoded_backup_path_round_trips_through_cli_restore() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_work"))
        .args(["--json", "--worktree"])
        .arg(&fixture.checkout)
        .args(["storage", "backup"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let backup: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let encoded = backup["result"]["backup_path"].as_str().unwrap();
    assert!(encoded.contains("%20"), "{encoded}");
    fs::remove_file(project.work_database_path()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_work"))
        .args(["--json", "--worktree"])
        .arg(&fixture.checkout)
        .args(["storage", "restore", "--backup", encoded])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
}

#[test]
fn migration_retains_backup_and_rolls_back_failed_schema_change() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let mut storage = Storage::open(&project).unwrap();
    storage
        .reconcile(&project, &fixture.store(&fixture.checkout))
        .unwrap();
    drop(storage);
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'migration-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    connection.execute("INSERT INTO runtime_records(kind, record_key, value) VALUES ('observation', 'migration-record', X'CAFE')", []).unwrap();
    let old_source_count: i64 = connection
        .query_row("SELECT count(*) FROM source_files", [], |row| row.get(0))
        .unwrap();
    assert!(old_source_count > 0);
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP TABLE views;
             CREATE VIEW views AS SELECT CAST('old-root' AS BLOB) AS root;
             PRAGMA user_version = 0;",
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::UnsupportedSchema(0)
    );
    let failure = Storage::migrate(&project).unwrap_err();
    assert_eq!(failure.code(), "migration_failed");
    assert!(failure.to_string().contains("work.db.backup-"));
    assert_eq!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::UnsupportedSchema(0)
    );
    let connection = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM source_files", [], |row| row.get(0))
            .unwrap(),
        old_source_count
    );
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT type FROM sqlite_schema WHERE name = 'views'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        "view"
    );
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "migration-token"
    );
    let backups: Vec<_> = fs::read_dir(project.work_storage_dir())
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("work.db.backup-")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    connection
        .execute_batch("DROP VIEW views; CREATE TABLE views(root BLOB PRIMARY KEY)")
        .unwrap();
    drop(connection);
    let migrated = Storage::migrate(&project).unwrap();
    assert_eq!(migrated.from_version, 0);
    assert!(migrated.backup_path.exists());
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM source_files", [], |row| row.get(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM views", [], |row| row.get(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "migration-token"
    );
    assert_eq!(
        connection
            .query_row::<Vec<u8>, _, _>(
                "SELECT value FROM runtime_records WHERE record_key = 'migration-record'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        b"\xca\xfe"
    );
    let backup = Connection::open(migrated.backup_path).unwrap();
    assert_eq!(
        backup
            .query_row::<i64, _, _>("SELECT count(*) FROM source_files", [], |row| row.get(0))
            .unwrap(),
        old_source_count
    );
}

#[test]
fn restores_a_backup_created_before_migration_then_requires_migration() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'before-migration', 'actor', 'now', 'now')", [FIRST]).unwrap();
    connection.execute_batch("PRAGMA user_version=0").unwrap();
    drop(connection);
    let migrated = Storage::migrate(&project).unwrap();
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'after-migration', 'actor', 'later', 'later')", [SECOND]).unwrap();
    drop(connection);

    let restored = Storage::restore_backup(&project, &migrated.backup_path).unwrap();
    assert!(restored.requires_migration);
    assert!(restored.coordination_may_be_stale);
    assert_eq!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::UnsupportedSchema(0)
    );
    let connection = Connection::open(project.work_database_path()).unwrap();
    let tokens: Vec<String> = connection
        .prepare("SELECT owner_token FROM claims ORDER BY item_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(tokens, ["before-migration"]);
    drop(connection);
    Storage::migrate(&project).unwrap();
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
}

#[test]
fn refuses_schema_zero_backup_missing_coordination_without_replacing_current_store() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let storage = Storage::open(&project).unwrap();
    let backup = storage.backup(&project).unwrap();
    drop(storage);
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'current-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    connection.execute("INSERT INTO runtime_records(kind, record_key, value) VALUES ('observation', 'current', X'CAFE')", []).unwrap();
    drop(connection);
    let connection = Connection::open(&backup).unwrap();
    connection
        .execute_batch("DROP TABLE runtime_records; PRAGMA user_version=0")
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::restore_backup(&project, &backup),
        Err(StorageError::Status(StorageStatus::Corrupt(detail)))
            if detail.contains("runtime_records")
    ));
    let connection = Connection::open(&backup).unwrap();
    connection.execute_batch("DROP TABLE claims").unwrap();
    drop(connection);

    assert!(matches!(
        Storage::restore_backup(&project, &backup),
        Err(StorageError::Status(StorageStatus::Corrupt(detail)))
            if detail.contains("claims")
    ));
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "current-token"
    );
    assert_eq!(
        connection
            .query_row::<Vec<u8>, _, _>(
                "SELECT value FROM runtime_records WHERE record_key = 'current'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        b"\xca\xfe"
    );
}

#[test]
fn refuses_to_migrate_schema_zero_without_coordination_tables_or_claim_exclusion() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection
        .execute_batch("DROP TABLE claims; DROP TABLE runtime_records; PRAGMA user_version=0")
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::migrate(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(detail)))
            if detail.contains("claims")
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    let coordination_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN ('claims', 'runtime_records')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(coordination_tables, 0);
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        0
    );

    connection
        .execute_batch(
            "CREATE TABLE claims (
                item_id TEXT PRIMARY KEY, owner_token TEXT NOT NULL,
                actor_id TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                run_id TEXT, session_id TEXT, workspace_id TEXT
            );
            CREATE TABLE runtime_records (
                kind TEXT NOT NULL, record_key TEXT NOT NULL, value BLOB NOT NULL,
                PRIMARY KEY(kind, record_key)
            );",
        )
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::migrate(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(detail)))
            if detail.contains("owner_token must be unique")
    ));
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection
        .execute_batch(
            "CREATE UNIQUE INDEX claim_owner_token ON claims(owner_token);
             DROP TABLE runtime_records;
             CREATE TABLE runtime_records (
                 kind TEXT NOT NULL, record_key TEXT NOT NULL, value TEXT NOT NULL,
                 PRIMARY KEY(kind, record_key)
             );",
        )
        .unwrap();
    drop(connection);
    assert!(matches!(
        Storage::migrate(&project),
        Err(StorageError::Status(StorageStatus::Corrupt(detail)))
            if detail.contains("runtime_records")
    ));
}

#[test]
fn refuses_a_schema_zero_backup_with_a_different_store_identity() {
    let fixture = Fixture::new();
    let project = fixture.project(&fixture.checkout);
    let storage = Storage::open(&project).unwrap();
    let backup = storage.backup(&project).unwrap();
    drop(storage);
    let connection = Connection::open(&backup).unwrap();
    connection.execute_batch("UPDATE store_meta SET store_id='00000000000000000000000000000000'; PRAGMA user_version=0").unwrap();
    drop(connection);
    assert!(matches!(
        Storage::restore_backup(&project, &backup),
        Err(StorageError::Status(StorageStatus::IdentityMismatch))
    ));
    assert!(matches!(
        Storage::inspect(&project).unwrap().status,
        StorageStatus::Ready { .. }
    ));
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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
    drop(Storage::open(&project).unwrap());
    let connection = Connection::open(project.work_database_path()).unwrap();
    connection.execute("INSERT INTO claims(item_id, owner_token, actor_id, created_at, updated_at) VALUES (?1, 'migration-token', 'actor', 'now', 'now')", [FIRST]).unwrap();
    connection
        .execute_batch(
            "DROP TABLE views; CREATE TABLE views(root BLOB PRIMARY KEY); PRAGMA user_version = 0;",
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
    connection.execute_batch("DROP TABLE views; CREATE TABLE views(root BLOB PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0);").unwrap();
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
            .query_row::<String, _, _>(
                "SELECT owner_token FROM claims WHERE item_id = ?1",
                [FIRST],
                |row| row.get(0)
            )
            .unwrap(),
        "migration-token"
    );
}

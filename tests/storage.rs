//! Foundation acceptance uses disposable Git repositories and actual callers.
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use work::core::project::{Project, discover};
use work::core::storage::{
    Publication, RecoverRequest, RecreateRequest, Storage, StorageErrorCode, StorageState,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-file-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "--initial-branch=main"]);
        git(
            &path,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        fs::create_dir_all(path.join(".work/items")).unwrap();
        let project = discover(Some(&path)).unwrap();
        Self { path, project }
    }
    fn storage(&self) -> Storage {
        Storage::new(self.project.clone())
    }
    fn root(&self) -> PathBuf {
        self.project.git_common_dir.join("work")
    }
    fn request(&self) -> RecreateRequest {
        let status = self.storage().inspect().unwrap();
        RecreateRequest {
            expected_store_id: status
                .retained_store_id
                .or_else(|| status.metadata.as_ref().map(|m| m.store_id.clone())),
            expected_generation: status.metadata.map(|m| m.recovery_generation),
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: true,
        }
    }
    fn cli(&self, args: &[&str]) -> (i32, Value) {
        cli(&self.path, args)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn cli(path: &Path, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_work"))
        .current_dir(path)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}
#[test]
fn fresh_reads_do_not_write_and_initialization_is_shared_and_idempotent() {
    let f = Fixture::new();
    let storage = f.storage();
    let initial = storage.inspect().unwrap();
    assert_eq!(initial.state, StorageState::Uninitialized);
    assert!(!initial.coordination_available);
    assert!(initial.storage_warning.is_none());
    let (code, result) = f.cli(&["storage", "inspect"]);
    assert_eq!(code, 0);
    assert_eq!(result["result"]["storage"]["state"], "uninitialized");
    assert!(!f.root().exists());
    assert!(!initial.identity_path.exists());
    let outcome = storage.initialize().unwrap();
    assert!(outcome.changed);
    assert!(outcome.storage.coordination_available);
    let metadata = outcome.storage.metadata.clone().unwrap();
    let lock_meta = fs::metadata(outcome.storage.lock_path).unwrap();
    assert_eq!(lock_meta.mode() & 0o777, 0o600);
    assert!(!storage.initialize().unwrap().changed);
    assert_eq!(storage.inspect().unwrap().metadata, Some(metadata.clone()));
    let linked = f.path.join("linked");
    git(
        &f.path,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let other = Storage::new(discover(Some(&linked)).unwrap())
        .inspect()
        .unwrap();
    assert_eq!(other.metadata, Some(metadata));
    assert_eq!(other.path, f.root());
    assert!(!f.root().join("work.db").exists());
}
#[test]
fn lost_lock_inspection_supplies_exact_expectations_to_real_recreation_caller() {
    let f = Fixture::new();
    let initial = f.storage().initialize().unwrap();
    fs::remove_file(f.root().join("coordination.lock")).unwrap();
    let (status, value) = f.cli(&["storage", "inspect"]);
    assert_eq!(status, 0);
    let data = &value["result"]["storage"];
    assert_eq!(data["coordination_available"], false);
    assert_eq!(data["storage_warning"]["code"], "storage_missing");
    let id = data["metadata"]["store_id"].as_str().unwrap();
    let generation = data["metadata"]["recovery_generation"].as_str().unwrap();
    let (code, outcome) = f.cli(&[
        "storage",
        "recreate",
        "--expected-store-id",
        id,
        "--expected-generation",
        generation,
        "--executors-stopped",
        "--acknowledge-loss",
        "--all-clients-stopped",
    ]);
    assert_eq!(code, 0, "{outcome}");
    assert_eq!(outcome["result"]["storage"]["coordination_available"], true);
    assert_eq!(outcome["result"]["storage"]["metadata"]["store_id"], id);
    assert_ne!(
        outcome["result"]["storage"]["metadata"]["recovery_generation"],
        initial.storage.metadata.unwrap().recovery_generation
    );
}
#[test]
fn completed_receipt_cannot_recreate_a_deleted_entity_folder() {
    let f = Fixture::new();
    let initial = f.storage().initialize().unwrap();
    let generation = initial.storage.metadata.unwrap().recovery_generation;
    fs::remove_dir(f.root().join("claims")).unwrap();
    let error = f
        .storage()
        .recover(
            initial.operation_id.as_deref().unwrap(),
            RecoverRequest::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, StorageErrorCode::StorageMissing);
    assert_eq!(error.publication, Publication::NotPublished);
    assert!(!f.root().join("claims").exists());
    let inspection = f.storage().inspect().unwrap();
    assert!(!inspection.coordination_available);
    assert_eq!(inspection.metadata.unwrap().recovery_generation, generation);
}
#[test]
fn future_versions_with_extra_fields_cannot_be_downgraded_even_with_a_lost_lock() {
    for witness in [false, true] {
        let f = Fixture::new();
        f.storage().initialize().unwrap();
        let path = if witness {
            f.project.git_common_dir.join("work.identity.yaml")
        } else {
            f.root().join("store.yaml")
        };
        let future=b"format_version: 2\nstore_id: \"00000000000040008000000000000000\"\nnew_future_field: true\n";
        fs::write(&path, future).unwrap();
        fs::remove_file(f.root().join("coordination.lock")).unwrap();
        let error = f.storage().recreate(f.request()).unwrap_err();
        assert_eq!(error.code, StorageErrorCode::UnsupportedFormat);
        assert_eq!(fs::read(&path).unwrap(), future);
        assert!(!f.root().join("coordination.lock").exists());
    }
}
#[test]
fn recreation_preserves_opaque_entities_receipts_and_intact_lock_inode() {
    let f = Fixture::new();
    let old = f.storage().initialize().unwrap();
    let old_id = old.operation_id.unwrap();
    let old_generation = old.storage.metadata.unwrap().recovery_generation;
    let lock = fs::metadata(f.root().join("coordination.lock")).unwrap();
    let opaque = b"not a claim format\0\xff\n";
    fs::write(f.root().join("claims/opaque.yaml"), opaque).unwrap();
    fs::create_dir(f.root().join("runs/some-opaque-run")).unwrap();
    fs::write(
        f.root().join("runs/some-opaque-run/any.data"),
        b"execution bytes",
    )
    .unwrap();
    let outcome = f.storage().recreate(f.request()).unwrap();
    let archive = outcome.loss.unwrap().prior_state_path.unwrap();
    assert_eq!(
        fs::read(archive.join("prior/claims/opaque.yaml")).unwrap(),
        opaque
    );
    assert_eq!(
        fs::read(archive.join("prior/runs/some-opaque-run/any.data")).unwrap(),
        b"execution bytes"
    );
    assert!(
        archive
            .join("operations")
            .join(&old_id)
            .join("operation.yaml")
            .is_file()
    );
    assert_eq!(fs::read_dir(f.root().join("claims")).unwrap().count(), 0);
    let current = fs::metadata(f.root().join("coordination.lock")).unwrap();
    assert_eq!((lock.dev(), lock.ino()), (current.dev(), current.ino()));
    assert_ne!(
        outcome.storage.metadata.unwrap().recovery_generation,
        old_generation
    );
    assert_eq!(
        f.storage()
            .recover(&old_id, RecoverRequest::default())
            .unwrap_err()
            .code,
        StorageErrorCode::Conflict
    );
}
#[test]
fn loss_requires_explicit_affirmations_and_never_resets_on_reads_or_init() {
    let f = Fixture::new();
    let old = f.storage().initialize().unwrap();
    fs::remove_dir_all(f.root()).unwrap();
    let status = f.storage().inspect().unwrap();
    assert_eq!(status.state, StorageState::RecoveryRequired);
    assert!(!f.root().exists());
    assert_eq!(
        status.retained_store_id,
        Some(old.storage.metadata.as_ref().unwrap().store_id.clone())
    );
    assert_eq!(
        f.storage().initialize().unwrap_err().code,
        StorageErrorCode::RecoveryRequired
    );
    assert!(!f.root().exists());
    assert_eq!(
        f.storage()
            .recreate(RecreateRequest::default())
            .unwrap_err()
            .code,
        StorageErrorCode::InvalidArgument
    );
    assert!(!f.root().exists());
    let new = f.storage().recreate(f.request()).unwrap();
    assert_ne!(
        new.storage.metadata.as_ref().unwrap().recovery_generation,
        old.storage.metadata.unwrap().recovery_generation
    );
}
#[test]
fn unsafe_paths_permissions_duplicate_fields_and_future_witnesses_are_distinct() {
    let f = Fixture::new();
    f.storage().initialize().unwrap();
    let metadata = f.root().join("store.yaml");
    let original = fs::read(&metadata).unwrap();
    let mut duplicate = original.clone();
    duplicate.extend_from_slice(b"format_version: 1\n");
    fs::write(&metadata, duplicate).unwrap();
    assert_eq!(
        f.storage().inspect().unwrap().storage_warning.unwrap().code,
        StorageErrorCode::InvalidFormat
    );
    fs::write(&metadata, &original).unwrap();
    fs::set_permissions(&metadata, fs::Permissions::from_mode(0o0)).unwrap();
    assert_eq!(
        f.storage().inspect().unwrap().storage_warning.unwrap().code,
        StorageErrorCode::PermissionDenied
    );
    fs::set_permissions(&metadata, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&metadata).unwrap();
    std::os::unix::fs::symlink(f.path.join("external"), &metadata).unwrap();
    assert_eq!(
        f.storage().inspect().unwrap().storage_warning.unwrap().code,
        StorageErrorCode::UnsafePath
    );
    assert!(!f.path.join("external").exists());
    fs::remove_file(&metadata).unwrap();
    fs::write(&metadata, &original).unwrap();
    fs::hard_link(&metadata, f.path.join("hardlink")).unwrap();
    assert_eq!(
        f.storage().inspect().unwrap().storage_warning.unwrap().code,
        StorageErrorCode::UnsafePath
    );
}
#[test]
fn unknown_operation_is_reported_and_only_explicitly_retained_without_execution() {
    let f = Fixture::new();
    f.storage().initialize().unwrap();
    let id = "11111111000040008000000000000000";
    fs::create_dir(f.root().join("operations").join(id)).unwrap();
    let bytes = b"format_version: 9\nkind: futuristic\npayload: unsupported\n";
    fs::write(
        f.root().join("operations").join(id).join("operation.yaml"),
        bytes,
    )
    .unwrap();
    let status = f.storage().inspect().unwrap();
    assert!(!status.coordination_available);
    assert!(!status.pending_operations[0].supported);
    let outcome = f.storage().recreate(f.request()).unwrap();
    let archive = outcome.loss.unwrap().prior_state_path.unwrap();
    assert_eq!(
        fs::read(archive.join("operations").join(id).join("operation.yaml")).unwrap(),
        bytes
    );
}
#[test]
fn independent_clones_get_separate_store_identity() {
    let a = Fixture::new();
    let b = Fixture::new();
    assert_ne!(
        a.storage()
            .initialize()
            .unwrap()
            .storage
            .metadata
            .unwrap()
            .store_id,
        b.storage()
            .initialize()
            .unwrap()
            .storage
            .metadata
            .unwrap()
            .store_id
    );
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}
#[test]
fn storage_lock_child() {
    let Some(path) = std::env::var_os("WORK_TEST_STORAGE_LOCK") else {
        return;
    };
    let root = PathBuf::from(path);
    let lock = fs::OpenOptions::new()
        .read(true)
        .open(root.join("coordination.lock"))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    fs::write(root.join("child.ready"), b"ready").unwrap();
    loop {
        std::thread::park();
    }
}
#[test]
fn independent_process_contention_and_termination_release_same_inode() {
    let f = Fixture::new();
    f.storage().initialize().unwrap();
    let inode = fs::metadata(f.root().join("coordination.lock"))
        .unwrap()
        .ino();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "storage_lock_child", "--nocapture"])
        .env("WORK_TEST_STORAGE_LOCK", f.root())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut guard = ChildGuard(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root().join("child.ready").exists() {
        assert!(Instant::now() < deadline, "lock child did not become ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (code, value) = f.cli(&["storage", "inspect"]);
    assert_eq!(code, 0);
    assert_eq!(
        value["result"]["storage"]["storage_warning"]["code"],
        "storage_busy"
    );
    let (code, value) = f.cli(&["storage", "init"]);
    assert_eq!(code, 5);
    assert_eq!(value["error"]["code"], "storage_busy");
    guard.0.kill().unwrap();
    guard.0.wait().unwrap();
    assert!(!f.storage().initialize().unwrap().changed);
    assert_eq!(
        fs::metadata(f.root().join("coordination.lock"))
            .unwrap()
            .ino(),
        inode
    );
}
struct Mcp {
    child: Child,
    input: std::process::ChildStdin,
    output: BufReader<std::process::ChildStdout>,
    id: u64,
}
impl Mcp {
    fn new(path: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
            .arg("mcp")
            .current_dir(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            input,
            output,
            id: 0,
        };
        client.rpc("initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"storage-test","version":"1"}}));
        writeln!(
            client.input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        client.input.flush().unwrap();
        client
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name":name,"arguments":args}))["result"].clone()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
#[test]
fn real_persistent_mcp_and_cli_reload_divergent_selected_views_without_rewriting_entities() {
    let f = Fixture::new();
    f.storage().initialize().unwrap();
    let id = "aaaaaaaa000040008000000000000000";
    let write = |root: &Path, state: &str, body: &str| {
        fs::write(
            root.join(".work/items").join(format!("{id}.md")),
            format!(
                "---\nformat_version: 1\nid: \"{id}\"\ntitle: Fixture\nstate: {state}\n---\n{body}"
            ),
        )
        .unwrap()
    };
    write(&f.path, "open", "control");
    git(&f.path, &["add", ".work"]);
    git(
        &f.path,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "items",
        ],
    );
    let linked = f.path.join("linked");
    git(
        &f.path,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    write(&linked, "done", "feature");
    let opaque = b"opaque\xff entity";
    fs::write(f.root().join("claims/fixture.yaml"), opaque).unwrap();
    let mut client = Mcp::new(&f.path);
    let result = client.call("item_inspect", json!({"id":id,"worktree":linked}));
    assert_eq!(result["structuredContent"]["item"]["state"], "done");
    assert_eq!(result["structuredContent"]["item"]["body"], "feature");
    write(&linked, "open", "edited uncommitted");
    let result = client.call("item_inspect", json!({"id":id,"worktree":linked}));
    assert_eq!(
        result["structuredContent"]["item"]["body"],
        "edited uncommitted"
    );
    let control = f.cli(&["item", "inspect", id]).1;
    assert_eq!(control["result"]["item"]["body"], "control");
    assert_eq!(control["result"]["item"]["state"], "open");
    let inspected = client.call("storage_inspect", json!({"worktree":linked}));
    assert_eq!(
        inspected["structuredContent"],
        f.cli(&["storage", "inspect"]).1["result"]
    );
    assert_eq!(
        fs::read(f.root().join("claims/fixture.yaml")).unwrap(),
        opaque
    );
    git(&f.path, &["switch", "-c", "storage-view"]);
    write(&f.path, "done", "different committed branch");
    git(&f.path, &["add", ".work/items"]);
    git(
        &f.path,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "branch-specific item",
        ],
    );
    let switched = client.call("item_inspect", json!({"id":id}));
    assert_eq!(
        switched["structuredContent"]["item"]["body"],
        "different committed branch"
    );
    assert_eq!(switched["structuredContent"]["item"]["state"], "done");
    git(&f.path, &["switch", "main"]);
    let restored = client.call("item_inspect", json!({"id":id}));
    assert_eq!(restored["structuredContent"]["item"]["body"], "control");
    assert_eq!(restored["structuredContent"]["item"]["state"], "open");
    assert_eq!(
        fs::read(f.root().join("claims/fixture.yaml")).unwrap(),
        opaque
    );
    drop(client);
    git(
        &f.path,
        &["worktree", "remove", "--force", linked.to_str().unwrap()],
    );
    assert_eq!(
        fs::read(f.root().join("claims/fixture.yaml")).unwrap(),
        opaque
    );
}

#[test]
fn storage_recreation_racer() {
    let Some(path) = std::env::var_os("WORK_STORAGE_RACE_PROJECT") else {
        return;
    };
    let path = PathBuf::from(path);
    let role = std::env::var("WORK_STORAGE_RACE_ROLE").unwrap();
    let id = std::env::var("WORK_STORAGE_RACE_ID").unwrap();
    let generation = std::env::var("WORK_STORAGE_RACE_GENERATION").unwrap();
    fs::write(path.join(format!("race-ready-{role}")), b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.join("race-start").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = if role == "cli" {
        cli(
            &path,
            &[
                "storage",
                "recreate",
                "--expected-store-id",
                &id,
                "--expected-generation",
                &generation,
                "--executors-stopped",
                "--acknowledge-loss",
            ],
        )
        .1
    } else {
        let mut client = Mcp::new(&path);
        let response=client.call("storage_recreate",json!({"expected_store_id":id,"expected_generation":generation,"executors_stopped":true,"acknowledge_loss":true}));
        if response["isError"] == true {
            json!({"ok":false,"error":response["structuredContent"]["error"]})
        } else {
            json!({"ok":true,"result":response["structuredContent"]})
        }
    };
    fs::write(
        path.join(format!("race-result-{role}.json")),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
}
#[test]
fn two_real_callers_with_same_expected_generation_publish_at_most_one_reset() {
    let f = Fixture::new();
    let initial = f.storage().initialize().unwrap();
    let meta = initial.storage.metadata.unwrap();
    let lock = fs::metadata(f.root().join("coordination.lock")).unwrap();
    let mut children = Vec::new();
    for role in ["cli", "mcp"] {
        children.push(ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "storage_recreation_racer", "--nocapture"])
                .env("WORK_STORAGE_RACE_PROJECT", &f.path)
                .env("WORK_STORAGE_RACE_ROLE", role)
                .env("WORK_STORAGE_RACE_ID", &meta.store_id)
                .env("WORK_STORAGE_RACE_GENERATION", &meta.recovery_generation)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while ["cli", "mcp"]
        .iter()
        .any(|role| !f.path.join(format!("race-ready-{role}")).exists())
    {
        assert!(
            Instant::now() < deadline,
            "race callers did not become ready"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::write(f.path.join("race-start"), b"start").unwrap();
    let mut outcomes = Vec::new();
    for (role, child) in ["cli", "mcp"].iter().zip(&mut children) {
        while child.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "race caller stuck");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.0.wait().unwrap().success(), "{role} child failed");
        outcomes.push(
            serde_json::from_slice::<Value>(
                &fs::read(f.path.join(format!("race-result-{role}.json"))).unwrap(),
            )
            .unwrap(),
        );
    }
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| result["ok"] == true)
            .count(),
        1,
        "{outcomes:?}"
    );
    let winner = outcomes.iter().find(|result| result["ok"] == true).unwrap();
    let loser = outcomes
        .iter()
        .find(|result| result["ok"] == false)
        .unwrap();
    assert!(
        matches!(
            loser["error"]["code"].as_str(),
            Some("conflict" | "storage_busy")
        ),
        "{loser}"
    );
    let after = f.storage().inspect().unwrap();
    assert!(after.coordination_available);
    assert_eq!(
        after.metadata.as_ref().unwrap().recovery_generation,
        winner["result"]["storage"]["metadata"]["recovery_generation"]
            .as_str()
            .unwrap()
    );
    assert_ne!(
        after.metadata.unwrap().recovery_generation,
        meta.recovery_generation
    );
    let current = fs::metadata(f.root().join("coordination.lock")).unwrap();
    assert_eq!((lock.dev(), lock.ino()), (current.dev(), current.ino()));
    assert_eq!(
        fs::read_dir(f.root().join("operations")).unwrap().count(),
        1
    );
}

#[test]
fn completed_receipt_cannot_replace_lost_lock_through_real_cli() {
    let f = Fixture::new();
    let initial = f.storage().initialize().unwrap();
    let metadata = initial.storage.metadata.unwrap();
    let id = initial.operation_id.as_deref().unwrap();
    let lock_path = f.root().join("coordination.lock");
    fs::remove_file(&lock_path).unwrap();
    let (code, value) = f.cli(&["storage", "recover", id, "--all-clients-stopped"]);
    assert_eq!(code, 4, "{value}");
    assert_eq!(value["error"]["code"], "storage_missing");
    assert!(!lock_path.exists());
    let status = f.storage().inspect().unwrap();
    assert!(!status.coordination_available);
    assert_eq!(status.metadata.as_ref(), Some(&metadata));
    let (code, result) = f.cli(&[
        "storage",
        "recreate",
        "--expected-store-id",
        &metadata.store_id,
        "--expected-generation",
        &metadata.recovery_generation,
        "--executors-stopped",
        "--acknowledge-loss",
        "--all-clients-stopped",
    ]);
    assert_eq!(code, 0, "{result}");
    assert_ne!(
        result["result"]["storage"]["metadata"]["recovery_generation"],
        metadata.recovery_generation
    );
    assert!(lock_path.is_file());
}

#[test]
fn real_cli_initialization_and_recreation_use_private_modes_under_restrictive_child_umask() {
    fn child(f: &Fixture, args: &[&str]) -> Value {
        let output = Command::new("sh")
            .args([
                "-c",
                "umask 0777; exec \"$WORK_STORAGE_UMASK_BIN\" \"$@\"",
                "storage-umask-test",
            ])
            .args(args)
            .env("WORK_STORAGE_UMASK_BIN", env!("CARGO_BIN_EXE_work"))
            .current_dir(&f.path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn private_tree(path: &Path) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let expected = if metadata.is_dir() { 0o700 } else { 0o600 };
        assert_eq!(metadata.mode() & 0o7777, expected, "{}", path.display());
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                private_tree(&entry.unwrap().path());
            }
        }
    }
    let f = Fixture::new();
    let initial = child(&f, &["--json", "storage", "init"]);
    assert_eq!(initial["result"]["storage"]["coordination_available"], true);
    private_tree(&f.root());
    private_tree(&f.project.git_common_dir.join("work.identity.yaml"));
    let metadata = f.storage().inspect().unwrap().metadata.unwrap();
    let lock = fs::metadata(f.root().join("coordination.lock")).unwrap();
    let opaque = b"opaque entity bytes\0\xff";
    let entity = f.root().join("claims/opaque");
    fs::write(&entity, opaque).unwrap();
    fs::set_permissions(&entity, fs::Permissions::from_mode(0o600)).unwrap();
    let repeated = child(&f, &["--json", "storage", "init"]);
    assert_eq!(repeated["result"]["changed"], false);
    let recreated = child(
        &f,
        &[
            "--json",
            "storage",
            "recreate",
            "--expected-store-id",
            &metadata.store_id,
            "--expected-generation",
            &metadata.recovery_generation,
            "--executors-stopped",
            "--acknowledge-loss",
        ],
    );
    assert_eq!(
        recreated["result"]["storage"]["coordination_available"],
        true
    );
    assert_ne!(
        recreated["result"]["storage"]["metadata"]["recovery_generation"],
        metadata.recovery_generation
    );
    let after_lock = fs::metadata(f.root().join("coordination.lock")).unwrap();
    assert_eq!(
        (after_lock.dev(), after_lock.ino()),
        (lock.dev(), lock.ino())
    );
    let id = recreated["result"]["operation_id"].as_str().unwrap();
    assert_eq!(
        fs::read(
            f.root()
                .join("recovery")
                .join(id)
                .join("prior/claims/opaque")
        )
        .unwrap(),
        opaque
    );
    private_tree(&f.root());
    private_tree(&f.project.git_common_dir.join("work.identity.yaml"));
}

#[test]
fn real_cli_and_mcp_require_a_complete_receipt_matching_live_generation_without_writing() {
    fn snapshot(path: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        let mut files = std::collections::BTreeMap::new();
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                files.extend(snapshot(&entry.path()));
            } else {
                files.insert(entry.path(), fs::read(entry.path()).unwrap());
            }
        }
        files
    }
    for mode in [
        "generation",
        "empty",
        "obsolete_only",
        "matching_and_obsolete",
    ] {
        let f = Fixture::new();
        let (code, initial) = f.cli(&["storage", "init"]);
        assert_eq!(code, 0);
        let old_id = initial["result"]["operation_id"].as_str().unwrap();
        if mode.starts_with("obsolete") || mode == "matching_and_obsolete" {
            let data = &initial["result"]["storage"]["metadata"];
            let (code, result) = f.cli(&[
                "storage",
                "recreate",
                "--expected-store-id",
                data["store_id"].as_str().unwrap(),
                "--expected-generation",
                data["recovery_generation"].as_str().unwrap(),
                "--executors-stopped",
                "--acknowledge-loss",
            ]);
            assert_eq!(code, 0, "{result}");
            let current_id = result["result"]["operation_id"].as_str().unwrap();
            let old = f
                .root()
                .join("recovery")
                .join(current_id)
                .join("operations")
                .join(old_id);
            let copied = f.root().join("operations").join(old_id);
            fs::create_dir(&copied).unwrap();
            for entry in fs::read_dir(&old).unwrap() {
                let entry = entry.unwrap();
                fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
            }
            if mode == "obsolete_only" {
                fs::remove_dir_all(f.root().join("operations").join(current_id)).unwrap();
            }
        } else if mode == "empty" {
            fs::remove_dir_all(f.root().join("operations").join(old_id)).unwrap();
        } else {
            let path = f.root().join("store.yaml");
            let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value["recovery_generation"] = json!("00000000000040008000000000000000");
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let before = snapshot(&f.root());
        let witness = fs::read(f.project.git_common_dir.join("work.identity.yaml")).unwrap();
        let (code, cli) = f.cli(&["storage", "inspect"]);
        assert_eq!(code, 0);
        let mut mcp = Mcp::new(&f.path);
        let response = mcp.call("storage_inspect", json!({}));
        assert_eq!(response["structuredContent"], cli["result"]);
        let healthy = mode == "matching_and_obsolete";
        let status = &cli["result"]["storage"];
        assert_eq!(status["coordination_available"], healthy, "{mode}");
        if healthy {
            assert!(status["storage_warning"].is_null());
        } else {
            assert_eq!(status["storage_warning"]["code"], "storage_corrupt");
            assert_eq!(
                status["storage_warning"]["path"],
                f.root().join("operations").to_str().unwrap()
            );
            let (code, refusal) = f.cli(&["storage", "init"]);
            assert_eq!(code, 4, "{refusal}");
            assert_eq!(refusal["error"]["code"], "storage_corrupt");
        }
        assert_eq!(snapshot(&f.root()), before);
        assert_eq!(
            fs::read(f.project.git_common_dir.join("work.identity.yaml")).unwrap(),
            witness
        );
    }
}

#[test]
fn real_callers_report_malformed_pending_intent_and_explicitly_retain_it_without_replay() {
    for damaged in ["context.yaml", "store.yaml", "identity.yaml"] {
        let f = Fixture::new();
        let (code, initial) = f.cli(&["storage", "init"]);
        assert_eq!(code, 0);
        let data = &initial["result"]["storage"]["metadata"];
        let (code, recreated) = f.cli(&[
            "storage",
            "recreate",
            "--expected-store-id",
            data["store_id"].as_str().unwrap(),
            "--expected-generation",
            data["recovery_generation"].as_str().unwrap(),
            "--executors-stopped",
            "--acknowledge-loss",
        ]);
        assert_eq!(code, 0);
        let id = recreated["result"]["operation_id"].as_str().unwrap();
        let metadata = &recreated["result"]["storage"]["metadata"];
        let dir = f.root().join("operations").join(id);
        // Simulate a malformed pending header/component, using a real published
        // foundation layout. Actual interruption phases are covered by core tests.
        let header_path = dir.join("operation.yaml");
        let mut header: Value = serde_json::from_slice(&fs::read(&header_path).unwrap()).unwrap();
        header["phase"] = json!("prepared");
        fs::write(&header_path, serde_json::to_vec(&header).unwrap()).unwrap();
        let component = dir.join(damaged);
        if damaged == "context.yaml" {
            fs::write(&component, b"malformed retained context").unwrap();
        } else {
            let mut value: Value = serde_json::from_slice(&fs::read(&component).unwrap()).unwrap();
            value[if damaged == "store.yaml" {
                "recovery_generation"
            } else {
                "store_id"
            }] = json!("00000000000040008000000000000000");
            fs::write(&component, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let bytes = fs::read(&component).unwrap();
        let receipt = fs::read(&header_path).unwrap();
        let (code, status) = f.cli(&["storage", "inspect"]);
        assert_eq!(code, 0);
        let mut mcp = Mcp::new(&f.path);
        let response = mcp.call("storage_inspect", json!({}));
        assert_eq!(response["structuredContent"], status["result"]);
        assert_eq!(status["result"]["storage"]["coordination_available"], false);
        let pending = status["result"]["storage"]["pending_operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|pending| pending["id"] == id)
            .unwrap();
        assert_eq!(pending["supported"], false);
        let (code, refusal) = f.cli(&[
            "storage",
            "recover",
            id,
            "--executors-stopped",
            "--acknowledge-loss",
        ]);
        assert_eq!(code, 4);
        let expected = if damaged == "context.yaml" {
            "invalid_format"
        } else {
            "storage_corrupt"
        };
        assert_eq!(refusal["error"]["code"], expected);
        let refusal = mcp.call(
            "storage_recover",
            json!({"operation_id":id,"executors_stopped":true,"acknowledge_loss":true}),
        );
        assert_eq!(refusal["isError"], true);
        assert_eq!(refusal["structuredContent"]["error"]["code"], expected);
        assert_eq!(fs::read(&component).unwrap(), bytes);
        let (code, reset) = f.cli(&[
            "storage",
            "recreate",
            "--expected-store-id",
            metadata["store_id"].as_str().unwrap(),
            "--expected-generation",
            metadata["recovery_generation"].as_str().unwrap(),
            "--executors-stopped",
            "--acknowledge-loss",
        ]);
        assert_eq!(code, 0, "{reset}");
        assert_eq!(reset["result"]["storage"]["coordination_available"], true);
        assert_ne!(
            reset["result"]["storage"]["metadata"]["recovery_generation"],
            metadata["recovery_generation"]
        );
        let new_id = reset["result"]["operation_id"].as_str().unwrap();
        let archive = f
            .root()
            .join("recovery")
            .join(new_id)
            .join("operations")
            .join(id);
        assert_eq!(fs::read(archive.join(damaged)).unwrap(), bytes);
        assert_eq!(fs::read(archive.join("operation.yaml")).unwrap(), receipt);
    }
}

#[test]
fn real_cli_and_mcp_retry_complete_receipts_without_rewriting_bytes_or_generation() {
    for kind in ["initialize", "recreate"] {
        let f = Fixture::new();
        let (code, mut outcome) = f.cli(&["storage", "init"]);
        assert_eq!(code, 0);
        if kind == "recreate" {
            let meta = &outcome["result"]["storage"]["metadata"];
            let (code, recreated) = f.cli(&[
                "storage",
                "recreate",
                "--expected-store-id",
                meta["store_id"].as_str().unwrap(),
                "--expected-generation",
                meta["recovery_generation"].as_str().unwrap(),
                "--executors-stopped",
                "--acknowledge-loss",
            ]);
            assert_eq!(code, 0);
            outcome = recreated;
        }
        let id = outcome["result"]["operation_id"].as_str().unwrap();
        let dir = f.root().join("operations").join(id);
        let receipt = dir.join("operation.yaml");
        let parsed: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
        // Valid alternate formatting must be synced in place, not normalized.
        let bytes = serde_json::to_vec(&parsed).unwrap();
        fs::write(&receipt, &bytes).unwrap();
        let before = fs::metadata(&receipt).unwrap();
        let count = fs::read_dir(&dir).unwrap().count();
        let meta = outcome["result"]["storage"]["metadata"].clone();
        let (code, retry) = f.cli(&[
            "storage",
            "recover",
            id,
            "--executors-stopped",
            "--acknowledge-loss",
        ]);
        assert_eq!(code, 0, "{retry}");
        assert_eq!(retry["result"]["storage"]["metadata"], meta);
        assert_eq!(retry["result"]["operation_id"], id);
        let mut client = Mcp::new(&f.path);
        let retry = client.call(
            "storage_recover",
            json!({"operation_id":id,"executors_stopped":true,"acknowledge_loss":true}),
        );
        assert_ne!(retry["isError"], true, "{retry}");
        assert_eq!(retry["structuredContent"]["storage"]["metadata"], meta);
        assert_eq!(retry["structuredContent"]["operation_id"], id);
        let after = fs::metadata(&receipt).unwrap();
        assert_eq!(
            (
                after.dev(),
                after.ino(),
                after.mode(),
                after.mtime(),
                after.mtime_nsec()
            ),
            (
                before.dev(),
                before.ino(),
                before.mode(),
                before.mtime(),
                before.mtime_nsec()
            )
        );
        assert_eq!(fs::read(&receipt).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), count);
    }
}

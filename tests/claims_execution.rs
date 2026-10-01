//! Regressions for accepted coordinator review findings, on disposable repos.
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use work::core::{
    claims::{ClaimCandidate, ClaimOutcome, ClaimStore},
    context::ResolvedView,
    coordination::{CoordinationGuard, SessionIdentity},
    graph::ItemGraph,
    items::ItemStore,
    project::{Project, discover},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-claims-execution-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let output = Command::new("git")
            .args(["init", "-q"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success());
        fs::create_dir_all(path.join(".work/items")).unwrap();
        let project = discover(Some(&path)).unwrap();
        Self { path, project }
    }
    fn cli(&self, args: &[&str]) -> Value {
        cli(&self.path, args)
    }
    fn item(&self) -> String {
        ok(self.cli(&["item", "create", "--title", "original"]))["item"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn init(&self) {
        ok(self.cli(&["storage", "init"]));
    }
    fn root(&self) -> PathBuf {
        self.project.git_common_dir.join("work")
    }
    fn candidate(&self, id: &str) -> ClaimCandidate {
        let store = ItemStore::load(&self.project).unwrap();
        ClaimCandidate {
            header: store.resolve(id).unwrap().header.clone().unwrap(),
            evaluation: ItemGraph::from_store(&store).evaluate(id).unwrap(),
            workspace_id: None,
            run_id: None,
            session_record_id: None,
        }
    }
    fn claim(&self, id: &str) -> Value {
        self.cli(&[
            "claim",
            "acquire",
            id,
            "--actor",
            "worker",
            "--session-namespace",
            "provider",
            "--session-id",
            "session",
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
fn cli(path: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("--json")
        .arg("--worktree")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn ok(value: Value) -> Value {
    assert_eq!(value["ok"], true, "{value}");
    value["result"].clone()
}
fn session() -> SessionIdentity {
    SessionIdentity {
        namespace: "provider".into(),
        id: "session".into(),
    }
}

#[test]
fn claim_acquisition_preserves_cli_and_mcp_lookup_errors_and_valid_references() {
    let f = Fixture::new();
    let ids = [
        "12345678000040008000000000000000",
        "12345678000040008000000000000001",
        "abcdef12000040008000000000000000",
    ];
    for id in ids {
        let generated = f.item();
        let path = f.path.join(format!(".work/items/{generated}.md"));
        let raw = fs::read_to_string(&path).unwrap().replace(&generated, id);
        fs::rename(&path, f.path.join(format!(".work/items/{id}.md"))).unwrap();
        fs::write(f.path.join(format!(".work/items/{id}.md")), raw).unwrap();
    }
    f.init();
    let cases = [
        ("not-an-id", "invalid_argument", 2),
        ("w-", "invalid_argument", 2),
        ("ABCDEF12", "invalid_argument", 2),
        ("00000000000000000000000000000000", "invalid_argument", 2),
        ("w-abcdef12000030008000000000000000", "invalid_argument", 2),
        ("abcdef12000040007000000000000000", "invalid_argument", 2),
        ("12345678", "ambiguous_id", 3),
        ("w-12345678", "ambiguous_id", 3),
        ("eeeeeeee", "not_found", 3),
        ("w-eeeeeeee", "not_found", 3),
        ("eeeeeeee000040008000000000000000", "not_found", 3),
    ];
    for (input, code, exit) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_work"))
            .arg("--json")
            .arg("--worktree")
            .arg(&f.path)
            .args([
                "claim",
                "acquire",
                input,
                "--actor",
                "worker",
                "--session-namespace",
                "provider",
                "--session-id",
                "session",
            ])
            .output()
            .unwrap();
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(exit), "{input}: {response}");
        assert_eq!(response["error"]["code"], code, "{input}: {response}");
        let response = mcp(
            &f.path,
            "claim_acquire",
            serde_json::json!({"item":input,"actor":"worker","session":{"namespace":"provider","id":"session"}}),
        );
        assert_eq!(response["error"]["code"], code, "{input}: {response}");
        if code == "ambiguous_id" {
            let message = response["error"]["message"].as_str().unwrap();
            assert!(message.contains(ids[0]) && message.contains(ids[1]));
        }
    }
    for input in [
        ids[2],
        "w-abcdef12000040008000000000000000",
        "abcdef12",
        "w-abcdef12",
    ] {
        let result = ok(f.claim(input));
        assert_eq!(result["claim"]["item_id"], ids[2]);
        let claim_id = result["claim"]["id"].as_str().unwrap();
        ok(f.cli(&[
            "claim",
            "release",
            claim_id,
            "--session-namespace",
            "provider",
            "--session-id",
            "session",
        ]));
        let result = mcp(
            &f.path,
            "claim_acquire",
            serde_json::json!({"item":input,"actor":"worker","session":{"namespace":"provider","id":"session"}}),
        );
        assert_eq!(result["claim"]["item_id"], ids[2]);
        let claim_id = result["claim"]["id"].as_str().unwrap();
        ok(f.cli(&[
            "claim",
            "release",
            claim_id,
            "--session-namespace",
            "provider",
            "--session-id",
            "session",
        ]));
    }
}

#[test]
fn actual_uninitialized_cli_completion_changes_use_original_durable_operation() {
    let f = Fixture::new();
    let id = f.item();
    let children = ok(f.cli(&["item", "update", &id, "--completion", "children"]));
    assert_eq!(children["item"]["completion"], "children");
    assert_eq!(children["item"]["state"], Value::Null);
    let manual = ok(f.cli(&["item", "update", &id, "--completion", "manual"]));
    assert_eq!(manual["item"]["completion"], "manual");
    assert_eq!(manual["item"]["state"], "open");
    ok(f.cli(&["item", "close", &id]));
    assert_eq!(ok(f.cli(&["item", "reopen", &id]))["item"]["state"], "open");
    assert!(!f.root().exists());
}

#[test]
fn independent_cli_field_updates_preserve_each_others_changes_without_storage() {
    let f = Fixture::new();
    let id = f.item();
    for round in 0..16 {
        ok(f.cli(&[
            "item",
            "update",
            &id,
            "--title",
            "original",
            "--priority",
            "2",
        ]));
        let gate = f.path.join(format!("gate-{round}"));
        let mut children = Vec::new();
        for field in ["title", "priority"] {
            let ready = f.path.join(format!("ready-{round}-{field}"));
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "field_update_process_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("WORK_CLAIMS_EXEC_CHECKOUT", &f.path)
                .env("WORK_CLAIMS_EXEC_ITEM", &id)
                .env("WORK_CLAIMS_EXEC_FIELD", field)
                .env("WORK_CLAIMS_EXEC_GATE", &gate)
                .env("WORK_CLAIMS_EXEC_READY", &ready)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            children.push((child, ready));
        }
        wait_until(|| children.iter().all(|(_, path)| path.exists()));
        fs::write(gate, b"go").unwrap();
        for (child, _) in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let final_item = ok(f.cli(&["item", "inspect", &id]));
        assert_eq!(
            final_item["item"]["title"], "concurrent title",
            "round {round}"
        );
        assert_eq!(final_item["item"]["priority"], 0, "round {round}");
    }
    assert!(!f.root().exists());
}
fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore]
fn field_update_process_helper() {
    let Some(path) = std::env::var_os("WORK_CLAIMS_EXEC_CHECKOUT") else {
        return;
    };
    let id = std::env::var("WORK_CLAIMS_EXEC_ITEM").unwrap();
    let field = std::env::var("WORK_CLAIMS_EXEC_FIELD").unwrap();
    let gate = PathBuf::from(std::env::var_os("WORK_CLAIMS_EXEC_GATE").unwrap());
    fs::write(
        PathBuf::from(std::env::var_os("WORK_CLAIMS_EXEC_READY").unwrap()),
        b"ready",
    )
    .unwrap();
    wait_until(|| gate.exists());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let change = if field == "title" {
            ["--title", "concurrent title"]
        } else {
            ["--priority", "0"]
        };
        let result = cli(
            Path::new(&path),
            &["item", "update", &id, change[0], change[1]],
        );
        if result["ok"] == true {
            break;
        }
        let changed_during_lookup = result["error"]["code"] == "invalid_source"
            && result["error"]["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| {
                    !diagnostics.is_empty()
                        && diagnostics
                            .iter()
                            .all(|d| d["message"] == "item changed while reading")
                });
        assert!(
            result["error"]["code"] == "conflict" || changed_during_lookup,
            "{result}"
        );
        assert!(Instant::now() < deadline, "{result}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn resolved_list_keeps_healthy_items_with_invalid_misnamed_source() {
    let f = Fixture::new();
    let healthy = f.item();
    let invalid = f.item();
    fs::rename(
        f.path.join(format!(".work/items/{invalid}.md")),
        f.path.join(".work/items/misnamed.md"),
    )
    .unwrap();
    f.init();
    let listed = ok(f.cli(&["item", "list"]));
    assert_eq!(listed["items"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["items"][0]["id"], healthy);
    assert!(
        !ok(f.cli(&["item", "diagnose"]))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn malformed_claims_and_invalid_owner_inputs_create_no_workspace_or_binding() {
    for case in ["malformed", "actor", "session", "owned"] {
        let f = Fixture::new();
        let id = f.item();
        f.init();
        if case == "malformed" {
            fs::write(f.root().join("claims/broken.yaml"), b"broken").unwrap();
        }
        if case == "owned" {
            ClaimStore::acquire(
                &CoordinationGuard::acquire(&f.project, true).unwrap(),
                &f.candidate(&id),
                "worker",
                &session(),
            )
            .unwrap();
        }
        let actor = if case == "actor" { "" } else { "worker" };
        let owner = if case == "session" { "" } else { "session" };
        let error = f.cli(&[
            "claim",
            "acquire",
            &id,
            "--actor",
            actor,
            "--session-namespace",
            "provider",
            "--session-id",
            owner,
        ]);
        assert_eq!(error["ok"], false, "{case}: {error}");
        assert_eq!(
            fs::read_dir(f.root().join("workspaces")).unwrap().count(),
            0,
            "{case}"
        );
    }
}

#[test]
fn not_ready_returns_item_and_evaluated_prerequisite_completed_and_aggregate_blockers() {
    let f = Fixture::new();
    let blocked = f.item();
    let prerequisite = f.item();
    let done = f.item();
    let aggregate = f.item();
    ok(f.cli(&["relation", "add", "depends_on", &blocked, &prerequisite]));
    ok(f.cli(&["item", "close", &done]));
    ok(f.cli(&["item", "update", &aggregate, "--completion", "children"]));
    f.init();
    for (id, kind) in [
        (&blocked, "prerequisite"),
        (&done, "completed"),
        (&aggregate, "aggregate"),
    ] {
        let error = f.claim(id);
        assert_eq!(error["error"]["code"], "not_ready", "{error}");
        assert_eq!(error["error"]["item_id"], *id);
        assert!(
            error["error"]["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["kind"] == kind),
            "{error}"
        );
        if kind == "prerequisite" {
            assert_eq!(error["error"]["blockers"][0]["id"], prerequisite);
        }
    }
    assert_eq!(
        fs::read_dir(f.root().join("workspaces")).unwrap().count(),
        0
    );
}

#[test]
fn cycle_requests_preserve_invalid_candidate_before_and_after_initialization() {
    for initialized in [false, true] {
        let f = Fixture::new();
        let one = f.item();
        let two = f.item();
        ok(f.cli(&["relation", "add", "depends_on", &one, &two]));
        if initialized {
            f.init();
        }
        let before = fs::read(f.path.join(format!(".work/items/{two}.md"))).unwrap();
        let error = f.cli(&["relation", "add", "depends_on", &two, &one]);
        assert_eq!(error["error"]["code"], "invalid_candidate", "{error}");
        assert_eq!(
            fs::read(f.path.join(format!(".work/items/{two}.md"))).unwrap(),
            before
        );
        assert!(
            ok(f.cli(&["item", "diagnose"]))["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn acquire_checked_detects_source_substitution_after_claim_scan_before_publication() {
    let f = Fixture::new();
    let id = f.item();
    f.init();
    let guard = CoordinationGuard::acquire(&f.project, true).unwrap();
    let view = ResolvedView::load(&guard).unwrap();
    let candidate = f.candidate(&id);
    let path = f.path.join(format!(".work/items/{id}.md"));
    let error = ClaimStore::acquire_checked(&guard, &candidate, "worker", &session(), || {
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("state: open", "state: done");
        fs::write(&path, text).unwrap();
        view.recheck()
    })
    .unwrap_err();
    assert_eq!(error.code, "conflict");
    assert!(ClaimStore::current(&guard, &id).unwrap().is_none());
    assert!(ClaimStore::list(&guard, None, false).unwrap().is_empty());
}

#[test]
fn reassign_checked_rechecks_both_boundaries_and_reports_ending_if_second_check_fails() {
    for fail_at in [1, 2] {
        let f = Fixture::new();
        let id = f.item();
        f.init();
        let guard = CoordinationGuard::acquire(&f.project, true).unwrap();
        let candidate = f.candidate(&id);
        let old = ClaimStore::acquire(&guard, &candidate, "worker", &session())
            .unwrap()
            .claim;
        let view = ResolvedView::load(&guard).unwrap();
        let path = f.path.join(format!(".work/items/{id}.md"));
        let mut checks = 0;
        let error = ClaimStore::reassign_checked(
            &guard,
            &old.id,
            &candidate,
            "replacement",
            &session(),
            "executor stopped",
            true,
            || {
                checks += 1;
                if checks == fail_at {
                    let replacement = f.path.join("replacement.md");
                    fs::write(&replacement, fs::read(&path).unwrap()).unwrap();
                    fs::rename(replacement, &path).unwrap();
                }
                view.recheck()
            },
        )
        .unwrap_err();
        assert_eq!(checks, fail_at);
        assert_eq!(error.code, "conflict");
        let record = ClaimStore::inspect(&guard, &old.id).unwrap();
        assert_eq!(record.current, fail_at == 1);
        if fail_at == 2 {
            assert_eq!(record.ending.unwrap().outcome, ClaimOutcome::Reassigned);
            assert_eq!(
                error.details["partial"]["created"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(error.details["partial"]["created"][0]["id"], old.id);
            assert!(ClaimStore::current(&guard, &id).unwrap().is_none());
        } else {
            assert!(record.ending.is_none());
        }
    }
}

#[test]
fn batch_list_matches_single_inspection_and_sorts_overlaid_sources_by_full_id() {
    let f = Fixture::new();
    let mut ids: Vec<_> = (0..6).map(|_| f.item()).collect();
    ids.sort();
    for (id, priority) in [
        (&ids[0], "1"),
        (&ids[1], "1"),
        (&ids[2], "1"),
        (&ids[3], "0"),
    ] {
        ok(f.cli(&["item", "update", id, "--priority", priority]));
    }
    ok(f.cli(&["relation", "add", "depends_on", &ids[4], &ids[3]]));
    ok(f.cli(&["item", "close", &ids[5]]));
    f.init();
    // Overlay the lowest ID, which used to append it after all unbound sources.
    let ended = ok(f.claim(&ids[0]));
    let ended_id = ended["claim"]["id"].as_str().unwrap();
    ok(f.cli(&[
        "claim",
        "release",
        ended_id,
        "--session-namespace",
        "provider",
        "--session-id",
        "session",
    ]));
    ok(f.claim(&ids[2]));
    let list = ok(f.cli(&["item", "list"]));
    let listed = list["items"].as_array().unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        ids
    );
    for item in listed {
        assert_eq!(
            *item,
            ok(f.cli(&["item", "inspect", item["id"].as_str().unwrap()]))["item"]
        );
    }
    let ready = ok(f.cli(&["item", "ready"]));
    assert_eq!(
        ready["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![ids[3].as_str(), ids[0].as_str(), ids[1].as_str()]
    );
}

#[test]
fn batch_list_keeps_ownership_warnings_and_readiness_refuses_malformed_claims() {
    let f = Fixture::new();
    let one = f.item();
    let two = f.item();
    f.init();
    fs::write(f.root().join("claims/broken.yaml"), b"broken").unwrap();
    let list = ok(f.cli(&["item", "list"]));
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    for id in [one, two] {
        let single = ok(f.cli(&["item", "inspect", &id]))["item"].clone();
        assert_eq!(single["ownership_warning"]["code"], "invalid_format");
        assert_eq!(single["executable"], false);
        assert_eq!(
            *list["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == id)
                .unwrap(),
            single
        );
    }
    assert_eq!(f.cli(&["item", "ready"])["error"]["code"], "invalid_format");
}

#[test]
fn close_permission_error_retains_last_ending_publication_and_saved_item_partial() {
    let f = Fixture::new();
    let id = f.item();
    f.init();
    let claim = ok(f.claim(&id));
    let claim_id = claim["claim"]["id"].as_str().unwrap();
    let authorization =
        serde_json::json!({"claim_id":claim_id,"session":session().to_json()}).to_string();
    let claims_path = f.root().join("claims");
    let prior_mode = fs::metadata(&claims_path).unwrap().permissions();
    fs::set_permissions(&claims_path, fs::Permissions::from_mode(0o500)).unwrap();
    let error = f.cli(&["item", "close", &id, "--authorize", &authorization]);
    // Restore before assertions, so the disposable repo is removable on failure.
    fs::set_permissions(&claims_path, prior_mode).unwrap();
    assert_eq!(error["error"]["code"], "permission_denied", "{error}");
    assert_eq!(error["error"]["publication"], "not_published", "{error}");
    assert_eq!(error["error"]["errno"], 13);
    assert!(
        error["error"]["path"]
            .as_str()
            .unwrap()
            .contains("/claims/")
    );
    let item = serde_json::json!({"id":id,"path":f.path.join(format!(".work/items/{id}.md")).to_str().unwrap()});
    assert_eq!(error["error"]["published_item"], item);
    assert_eq!(
        error["error"]["partial"]["updated"],
        serde_json::json!([item])
    );
    let saved = fs::read(f.path.join(format!(".work/items/{id}.md"))).unwrap();
    assert_eq!(
        ok(f.cli(&["item", "inspect", &id]))["item"]["state"],
        "done"
    );
    assert_eq!(ok(f.cli(&["claim", "inspect", claim_id]))["current"], true);
    assert!(
        !f.root()
            .join(format!("claims/{claim_id}.end.yaml"))
            .exists()
    );
    ok(f.cli(&["item", "close", &id, "--authorize", &authorization]));
    assert_eq!(
        fs::read(f.path.join(format!(".work/items/{id}.md"))).unwrap(),
        saved
    );
    assert_eq!(
        ok(f.cli(&["claim", "inspect", claim_id]))["ending"]["outcome"],
        "completed"
    );
}

#[test]
#[ignore]
fn coordination_lock_process_helper() {
    let Some(path) = std::env::var_os("WORK_CLAIMS_LOCK_CHECKOUT") else {
        return;
    };
    let project = discover(Some(Path::new(&path))).unwrap();
    let _guard = CoordinationGuard::acquire(&project, true).unwrap();
    fs::write(
        PathBuf::from(std::env::var_os("WORK_CLAIMS_LOCK_READY").unwrap()),
        b"ready",
    )
    .unwrap();
    let release = PathBuf::from(std::env::var_os("WORK_CLAIMS_LOCK_RELEASE").unwrap());
    wait_until(|| release.exists());
}
fn mcp(path: &Path, name: &str, arguments: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("mcp")
        .current_dir(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin,"{}",serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    writeln!(stdin,"{}",serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}})).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    result["result"]["structuredContent"].clone()
}
#[test]
fn unreadable_metadata_preserves_cli_mcp_write_refusal_and_read_warning() {
    let f = Fixture::new();
    let id = f.item();
    f.init();
    let path = f.root().join("store.yaml");
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    let metadata = fs::read(&path).unwrap();
    let item_path = f.path.join(format!(".work/items/{id}.md"));
    let original = fs::read(&item_path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let baseline = f.cli(&["storage", "init"]);
    assert_eq!(baseline["error"]["code"], "permission_denied");
    for (args, tool, arguments) in [
        (
            vec!["item", "update", &id, "--title", "refused"],
            "item_update",
            serde_json::json!({"id":id,"title":"refused"}),
        ),
        (
            vec![
                "claim",
                "acquire",
                &id,
                "--actor",
                "worker",
                "--session-namespace",
                "provider",
                "--session-id",
                "session",
            ],
            "claim_acquire",
            serde_json::json!({"item":id,"actor":"worker","session":{"namespace":"provider","id":"session"}}),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_work"))
            .arg("--json")
            .arg("--worktree")
            .arg(&f.path)
            .args(args)
            .output()
            .unwrap();
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(1), "{response}");
        let mcp_response = mcp(&f.path, tool, arguments);
        for error in [&response["error"], &mcp_response["error"]] {
            assert_eq!(error["code"], "permission_denied", "{error}");
            assert_eq!(error["path"], path.to_str().unwrap());
            assert_eq!(error["errno"], baseline["error"]["errno"]);
            assert_eq!(error["publication"], "not_published");
            assert!(!error["message"].as_str().unwrap().contains("recovery"));
        }
        assert_eq!(fs::read(&item_path).unwrap(), original);
    }
    let read = f.cli(&["item", "list"]);
    assert_eq!(read["ok"], true, "{read}");
    let mcp_read = mcp(&f.path, "item_list", serde_json::json!({}));
    for result in [&read["result"], &mcp_read] {
        assert_eq!(result["items"].as_array().unwrap().len(), 1, "{result}");
        assert_eq!(result["items"][0]["id"], id);
        assert_eq!(result["storage_warning"]["code"], "permission_denied");
        assert_eq!(result["storage_warning"]["path"], path.to_str().unwrap());
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), metadata);
    assert_eq!(fs::read(&item_path).unwrap(), original);
    assert_eq!(fs::read_dir(f.root().join("claims")).unwrap().count(), 0);
    assert_eq!(
        fs::read_dir(f.root().join("workspaces")).unwrap().count(),
        0
    );
}

#[test]
fn independent_lock_holder_preserves_cli_mcp_mutation_contention_and_read_warnings() {
    let f = Fixture::new();
    let id = f.item();
    f.init();
    let before = fs::read(f.path.join(format!(".work/items/{id}.md"))).unwrap();
    let ready = f.path.join("lock-ready");
    let release = f.path.join("lock-release");
    let holder = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "coordination_lock_process_helper",
            "--ignored",
            "--nocapture",
        ])
        .env("WORK_CLAIMS_LOCK_CHECKOUT", &f.path)
        .env("WORK_CLAIMS_LOCK_READY", &ready)
        .env("WORK_CLAIMS_LOCK_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until(|| ready.exists());
    let cli_output = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("--json")
        .arg("--worktree")
        .arg(&f.path)
        .args(["item", "update", &id, "--title", "blocked by lock"])
        .output()
        .unwrap();
    let cli_error: Value = serde_json::from_slice(&cli_output.stdout).unwrap();
    let mcp_error = mcp(
        &f.path,
        "item_update",
        serde_json::json!({"id":id,"title":"blocked by lock"}),
    );
    let claim_error = f.claim(&id);
    let read = f.cli(&["item", "list"]);
    fs::write(release, b"release").unwrap();
    let holder_output = holder.wait_with_output().unwrap();
    assert!(
        holder_output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&holder_output.stdout),
        String::from_utf8_lossy(&holder_output.stderr)
    );
    assert_eq!(cli_output.status.code(), Some(5), "{cli_error}");
    let path = f.root().join("coordination.lock");
    for error in [&cli_error["error"], &mcp_error["error"]] {
        assert_eq!(error["code"], "storage_busy", "{error}");
        assert_eq!(error["path"], path.to_str().unwrap());
        assert_eq!(error["diagnostics"][0]["code"], "storage_busy");
        assert_eq!(error["diagnostics"][0]["path"], error["path"]);
        assert!(!error["message"].as_str().unwrap().contains("recovery"));
    }
    assert_eq!(claim_error["error"]["code"], cli_error["error"]["code"]);
    assert_eq!(claim_error["error"]["path"], cli_error["error"]["path"]);
    assert_eq!(read["ok"], true);
    assert_eq!(read["result"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(read["result"]["storage_warning"]["code"], "storage_busy");
    assert_eq!(
        fs::read(f.path.join(format!(".work/items/{id}.md"))).unwrap(),
        before
    );
    assert_eq!(
        ok(f.cli(&["item", "update", &id, "--title", "after lock release"]))["item"]["title"],
        "after lock release"
    );
}

#[test]
fn invalid_candidate_inputs_keep_argument_errors_before_and_after_storage_init() {
    for initialized in [false, true] {
        let f = Fixture::new();
        let id = f.item();
        if initialized {
            f.init();
        }
        let item_path = f.path.join(format!(".work/items/{id}.md"));
        let original = fs::read(&item_path).unwrap();
        for args in [
            vec!["item", "update", &id, "--title", ""],
            vec!["item", "create", "--title", ""],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_work"))
                .args(["--json", "--worktree"])
                .arg(&f.path)
                .args(args)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2));
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["error"]["code"], "invalid_argument", "{value}");
        }
        for (tool, args) in [
            ("item_update", serde_json::json!({"id":id,"title":""})),
            ("item_create", serde_json::json!({"title":""})),
        ] {
            let value = mcp(&f.path, tool, args);
            assert_eq!(value["error"]["code"], "invalid_argument", "{value}");
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
            .args(["--json", "--worktree"])
            .arg(&f.path)
            .args(["item", "create", "--title", "valid", "--body", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&[0xff, 0xfe])
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["code"], "invalid_argument", "{value}");
        assert_eq!(fs::read(&item_path).unwrap(), original);
        assert_eq!(ItemStore::load(&f.project).unwrap().files.len(), 1);
    }
}

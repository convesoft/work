//! Regressions for accepted coordinator review findings, on disposable repos.
use serde_json::Value;
use std::{
    fs,
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

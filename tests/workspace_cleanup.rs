//! Real CLI/MCP cleanup, using only disposable repositories. Git deletion is
//! performed by this test harness, never by a Work command.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn command(root: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_work"));
    c.arg("--json").arg("--worktree").arg(root).args(args);
    c
}
fn cli(root: &Path, args: &[&str]) -> Value {
    let o = command(root, args).output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&o.stderr)));
    assert_eq!(o.status.success(), v["ok"] == true, "{v}");
    v
}
fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}
fn mcp(root: &Path, name: &str, mut args: Value) -> Value {
    args["worktree"] = json!(root);
    let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("mcp")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    drop(input);
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).lines().last().unwrap()).unwrap();
    v["result"]["structuredContent"].clone()
}
fn good(v: Value) -> Value {
    assert!(v["error"].is_null(), "{v}");
    v
}
struct Fixture {
    dir: PathBuf,
    root: PathBuf,
    target: PathBuf,
    item: String,
    other: String,
    control: String,
    wid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "work-cleanup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = dir.join("controller");
        let target = dir.join("target");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "--initial-branch=main"]);
        fs::create_dir_all(root.join(".work/items")).unwrap();
        ok(cli(&root, &["storage", "init"]));
        let create = || {
            ok(cli(
                &root,
                &[
                    "item",
                    "create",
                    "--title",
                    "cleanup fixture",
                    "--body",
                    "opaque retained result",
                ],
            ))["item"]["id"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let item = create();
        let other = create();
        git(&root, &["add", ".work/items"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        git(
            &root,
            &[
                "worktree",
                "add",
                "-qb",
                "feature",
                target.to_str().unwrap(),
            ],
        );
        let register = |p: &Path| {
            ok(cli(&root, &["workspace", "register", p.to_str().unwrap()]))["workspace"]["id"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let control = register(&root);
        let wid = register(&target);
        Self {
            dir,
            root,
            target,
            item,
            other,
            control,
            wid,
        }
    }
    fn begin(&self) -> Value {
        cli(
            &self.root,
            &[
                "workspace",
                "cleanup",
                "begin",
                &self.wid,
                "--item",
                &self.item,
                "--controller-workspace",
                &self.control,
            ],
        )
    }
    fn report(&self) -> Value {
        cli(
            &self.root,
            &["workspace", "cleanup", "report", &self.wid, "--removed"],
        )
    }
    fn inspect(&self) -> Value {
        ok(cli(&self.root, &["workspace", "inspect", &self.wid]))
    }
    fn record(&self) -> PathBuf {
        self.root
            .join(format!(".git/work/workspaces/{}.yaml", self.wid))
    }
    fn remove(&self) {
        git(
            &self.root,
            &[
                "worktree",
                "remove",
                "--force",
                self.target.to_str().unwrap(),
            ],
        );
    }
    fn bind(&self, item: &str, wid: &str) {
        ok(cli(&self.root, &["workspace", "bind", item, wid]));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn closing_failure_retry_external_removal_and_restart_keep_real_context() {
    let f = Fixture::new();
    f.bind(&f.other, &f.wid);
    let acquired = good(mcp(
        &f.root,
        "claim_acquire",
        json!({"item":f.other,"actor":"worker","session":{"namespace":"test","id":"worker"}}),
    ));
    let cid = acquired["claim"]["id"].as_str().unwrap();
    assert_eq!(f.begin()["error"]["code"], "workspace_busy");
    good(mcp(
        &f.root,
        "claim_release",
        json!({"claim_id":cid,"session":{"namespace":"test","id":"worker"}}),
    ));
    ok(cli(&f.target, &["item", "close", &f.other]));
    // Even completed target progress needs an explicit surviving source.
    assert_eq!(f.begin()["error"]["code"], "workspace_busy");
    let result_path = format!(".work/items/{}.md", f.other);
    fs::copy(f.target.join(&result_path), f.root.join(&result_path)).unwrap();
    f.bind(&f.other, &f.control);
    let h = good(mcp(
        &f.root,
        "handoff_create",
        json!({"from_items":[f.other],"to_items":[f.item],"body":"handoff α\n\nopaque retained context\n","workspace_id":f.wid}),
    ));
    let hp = f.root.join(format!(
        ".git/work/handoffs/{}.md",
        h["handoff"]["id"].as_str().unwrap()
    ));
    let handoff_bytes = fs::read(&hp).unwrap();
    let claim_path = f.root.join(format!(".git/work/claims/{cid}.yaml"));
    let claim_bytes = fs::read(&claim_path).unwrap();
    let item_bytes = fs::read(f.root.join(&result_path)).unwrap();
    let owned_cleanup = good(mcp(
        &f.root,
        "claim_acquire",
        json!({"item":f.item,"actor":"controller","session":{"namespace":"test","id":"controller"}}),
    ));
    let start = ok(f.begin());
    assert_eq!(start["workspace"]["state"], "closing");
    assert_eq!(start["workspace"]["cleanup"]["item_id"], f.item);
    assert!(f.target.is_dir()); // Work did not physically delete anything.
    assert_eq!(f.report()["error"]["code"], "workspace_busy");
    let failure = "external Git failed\nretain α / context";
    let failed = good(mcp(
        &f.root,
        "workspace_cleanup_report",
        json!({"workspace_id":f.wid,"removed":false,"failure":failure}),
    ));
    assert_eq!(failed["workspace"]["cleanup"]["failure"], failure);
    assert_eq!(
        ok(cli(
            &f.root,
            &[
                "workspace",
                "cleanup",
                "report",
                &f.wid,
                "--failure",
                failure
            ]
        ))["changed"],
        false
    );
    let retry = ok(f.begin());
    assert_eq!(retry["changed"], false);
    assert_eq!(retry["workspace"], failed["workspace"]);
    assert_eq!(f.inspect()["workspace"], failed["workspace"]);
    // Restart after begin, before physical deletion: context is still present.
    let controller = good(mcp(
        &f.root,
        "workspace_inspect",
        json!({"workspace_id":f.control}),
    ));
    assert!(
        controller["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["kind"] == "cleanup_controller" && u["id"] == f.wid)
    );
    f.remove(); // External harness; no Work lock is held across this operation.
    // Restart after physical deletion, before reporting: closing remains usable.
    assert_eq!(f.inspect()["workspace"]["state"], "closing");
    assert_eq!(ok(f.begin())["changed"], false);
    let removed = good(mcp(
        &f.root,
        "workspace_cleanup_report",
        json!({"workspace_id":f.wid,"removed":true}),
    ));
    assert_eq!(
        removed,
        json!({"workspace_id":f.wid,"removed":true,"changed":true})
    );
    assert!(!f.record().exists());
    // Restart after deletion: not-found is the contract's completion evidence.
    assert_eq!(
        cli(&f.root, &["workspace", "inspect", &f.wid])["error"]["code"],
        "not_found"
    );
    assert_eq!(f.report()["error"]["code"], "not_found");
    assert_eq!(fs::read(hp).unwrap(), handoff_bytes);
    assert_eq!(fs::read(claim_path).unwrap(), claim_bytes);
    assert_eq!(fs::read(f.root.join(result_path)).unwrap(), item_bytes);
    let cleanup = ok(cli(&f.root, &["item", "inspect", &f.item]))["item"].clone();
    assert_eq!(cleanup["state"], "open");
    assert_eq!(cleanup["claim"]["id"], owned_cleanup["claim"]["id"]);
    assert_eq!(
        cleanup["incoming_handoffs"][0]["body"],
        "handoff α\n\nopaque retained context\n"
    );
}

#[test]
fn closing_refuses_new_bindings_defaults_claims_and_controller_cleanup() {
    let f = Fixture::new();
    ok(f.begin());
    assert_eq!(
        cli(&f.root, &["workspace", "bind", &f.other, &f.wid])["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        good(mcp(
            &f.root,
            "workspace_inspect",
            json!({"workspace_id":f.wid})
        ))["workspace"]["state"],
        "closing"
    );
    assert_eq!(
        mcp(
            &f.root,
            "claim_acquire",
            json!({"item":f.other,"actor":"late","session":{"namespace":"test","id":"late"}})
        )["error"],
        Value::Null
    );
    // Acquiring from the controller works; target assignment does not.
    assert_eq!(
        cli(
            &f.target,
            &[
                "claim",
                "acquire",
                &f.item,
                "--actor",
                "late",
                "--session-namespace",
                "test",
                "--session-id",
                "late"
            ]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(&f.root, &["run", "start", &f.item, "--workspace", &f.wid])["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(
            &f.root,
            &["run", "start", &f.item, "--output-workspace", &f.wid]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(
            &f.root,
            &["workspace", "register", f.target.to_str().unwrap()]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        mcp(
            &f.target,
            "claim_next",
            json!({"root":f.item,"actor":"late","session":{"namespace":"test","id":"late"}})
        )["error"]["code"],
        "workspace_busy"
    );
    // The controller cannot itself be closed while its dependent cleanup lives.
    assert_eq!(
        cli(
            &f.root,
            &[
                "workspace",
                "cleanup",
                "begin",
                &f.control,
                "--item",
                &f.item,
                "--controller-workspace",
                &f.wid
            ]
        )["error"]["code"],
        "workspace_busy"
    );
}

#[test]
fn cancel_requires_original_path_and_repository_then_allows_reuse() {
    let f = Fixture::new();
    ok(f.begin());
    let cancelled = good(mcp(
        &f.root,
        "workspace_cleanup_cancel",
        json!({"workspace_id":f.wid}),
    ));
    assert_eq!(cancelled["workspace"]["state"], "open");
    assert!(cancelled["workspace"]["cleanup"].is_null());
    f.bind(&f.other, &f.wid);
    ok(cli(&f.root, &["workspace", "unbind", &f.other]));
    ok(f.begin());
    f.remove();
    assert_eq!(
        cli(&f.root, &["workspace", "cleanup", "cancel", &f.wid])["error"]["code"],
        "source_unavailable"
    );
    fs::create_dir(&f.target).unwrap();
    git(&f.target, &["init", "-q", "--initial-branch=main"]);
    assert_eq!(
        mcp(
            &f.root,
            "workspace_cleanup_cancel",
            json!({"workspace_id":f.wid})
        )["error"]["code"],
        "source_unavailable"
    );
    assert_eq!(f.inspect()["workspace"]["state"], "closing");
    assert_eq!(f.report()["error"]["code"], "workspace_busy");
}

#[test]
fn current_run_defaults_outputs_roots_and_outside_members_block_cleanup() {
    for kind in ["default", "output", "root", "member"] {
        let f = Fixture::new();
        let root = if kind == "root" { &f.other } else { &f.item };
        if kind == "root" {
            f.bind(root, &f.wid);
        }
        let default = if kind == "default" {
            &f.wid
        } else {
            &f.control
        };
        let output = if kind == "output" { &f.wid } else { &f.control };
        let run = ok(cli(
            &f.root,
            &[
                "run",
                "start",
                root,
                "--workspace",
                default,
                "--output-workspace",
                output,
            ],
        ))["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        good(mcp(
            &f.root,
            "session_set",
            json!({"run_id":run,"name":"worker","session":{"namespace":"test","id":"reuse"}}),
        ));
        if kind == "member" {
            f.bind(&f.other, &f.wid);
            ok(cli(&f.root, &["run", "attach", &run, &f.other]));
        }
        let blocked = f.begin();
        assert_eq!(
            blocked["error"]["code"], "workspace_busy",
            "{kind}: {blocked}"
        );
        assert_eq!(f.inspect()["workspace"]["state"], "open");
        if kind == "root" || kind == "member" {
            f.bind(&f.other, &f.control);
            ok(f.begin());
            f.remove();
            ok(f.report());
            let preserved = ok(cli(&f.root, &["run", "inspect", &run]));
            assert_eq!(preserved["run"]["phase"], "active");
            assert_eq!(
                good(mcp(&f.root, "session_list", json!({"run_id":run})))["sessions"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        } else {
            // Run finalization is a successor; only use a valid terminal fixture.
            let p = f.root.join(format!(".git/work/runs/{run}/run.yaml"));
            let mut v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            v["phase"] = json!("disposed");
            v["ended_at"] = json!("2026-10-01T10:30:00Z");
            v["cleanup"] = json!({"kind":"discard","finalize":true,"item_ids":[],"session_ids":[],"started_at":"2026-10-01T10:30:00Z"});
            fs::write(p, serde_json::to_vec(&v).unwrap()).unwrap();
            ok(f.begin());
        }
    }
}

#[test]
fn gates_reject_wrong_controller_unexecutable_items_and_missing_sources() {
    let f = Fixture::new();
    assert_eq!(
        mcp(
            &f.root,
            "workspace_cleanup_begin",
            json!({"workspace_id":f.wid,"item":f.item,"controller_workspace_id":f.wid})
        )["error"]["code"],
        "workspace_busy"
    );
    ok(cli(
        &f.root,
        &["relation", "add", "depends_on", &f.item, &f.other],
    ));
    assert_eq!(f.begin()["error"]["code"], "not_ready");
    ok(cli(
        &f.root,
        &["relation", "remove", "depends_on", &f.item, &f.other],
    ));
    ok(cli(&f.root, &["item", "close", &f.item]));
    assert_eq!(f.begin()["error"]["code"], "not_ready");
    ok(cli(&f.root, &["item", "reopen", &f.item]));
    f.bind(&f.item, &f.wid);
    assert_eq!(f.begin()["error"]["code"], "workspace_busy");
    f.bind(&f.item, &f.control);
    f.bind(&f.other, &f.wid);
    fs::remove_file(f.target.join(format!(".work/items/{}.md", f.other))).unwrap();
    assert_eq!(f.begin()["error"]["code"], "invalid_source");
    assert_eq!(f.inspect()["users"][0]["effective_done"], Value::Null);
    f.bind(&f.other, &f.control); // Explicit source repair, never inferred.
    ok(f.begin());
}

#[test]
fn observed_success_refuses_path_substitutes_and_new_users_without_deleting_context() {
    let f = Fixture::new();
    ok(f.begin());
    f.remove();
    std::os::unix::fs::symlink(f.dir.join("absent"), &f.target).unwrap();
    assert_eq!(f.report()["error"]["code"], "workspace_busy");
    fs::remove_file(&f.target).unwrap();
    // Simulate a raw editor outside the cooperating assignment guarantee.
    f.bind(&f.other, &f.control);
    let binding = f
        .root
        .join(format!(".git/work/workspaces/items/{}.yaml", f.other));
    let mut v: Value = serde_json::from_slice(&fs::read(&binding).unwrap()).unwrap();
    v["workspace_id"] = json!(f.wid);
    fs::write(&binding, serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(f.report()["error"]["code"], "invalid_source");
    assert!(f.record().exists());
    assert!(binding.exists());
    let failed = good(mcp(
        &f.root,
        "workspace_cleanup_report",
        json!({"workspace_id":f.wid,"removed":false,"failure":"source repair required"}),
    ));
    assert_eq!(
        failed["workspace"]["cleanup"]["failure"],
        "source repair required"
    );
    f.bind(&f.other, &f.control);
    ok(f.report());
}

#[test]
fn input_validation_and_no_implicit_cleanup() {
    let f = Fixture::new();
    for args in [
        vec!["workspace", "cleanup", "report", &f.wid],
        vec![
            "workspace",
            "cleanup",
            "report",
            &f.wid,
            "--removed",
            "--failure",
            "x",
        ],
        vec![
            "workspace",
            "cleanup",
            "report",
            &f.wid,
            "--removed",
            "--removed",
        ],
        vec!["workspace", "cleanup", "begin", &f.wid, "--item", &f.item],
    ] {
        assert_eq!(cli(&f.root, &args)["error"]["code"], "invalid_argument");
    }
    for args in [
        json!({"workspace_id":f.wid,"removed":false}),
        json!({"workspace_id":f.wid,"removed":"true"}),
        json!({"workspace_id":f.wid,"removed":true,"failure":"x"}),
        json!({"workspace_id":f.wid,"removed":false,"failure":null}),
    ] {
        assert_eq!(
            mcp(&f.root, "workspace_cleanup_report", args)["error"]["code"],
            "invalid_argument"
        );
    }
    assert_eq!(f.report()["error"]["code"], "workspace_busy");
    assert_eq!(
        mcp(
            &f.root,
            "workspace_cleanup_cancel",
            json!({"workspace_id":"w-short"})
        )["error"]["code"],
        "invalid_argument"
    );
    f.bind(&f.other, &f.wid);
    ok(cli(&f.target, &["item", "close", &f.other]));
    assert!(f.target.is_dir());
    assert!(f.record().is_file());
}

#[test]
fn independent_process_race_closing_vs_assignment_has_one_safe_outcome() {
    for _ in 0..4 {
        let f = Fixture::new();
        let begin = command(
            &f.root,
            &[
                "workspace",
                "cleanup",
                "begin",
                &f.wid,
                "--item",
                &f.item,
                "--controller-workspace",
                &f.control,
            ],
        )
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
        let acquire = command(
            &f.target,
            &[
                "claim",
                "acquire",
                &f.other,
                "--actor",
                "racer",
                "--session-namespace",
                "test",
                "--session-id",
                "racer",
            ],
        )
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
        let a: Value = serde_json::from_slice(&begin.wait_with_output().unwrap().stdout).unwrap();
        let b: Value = serde_json::from_slice(&acquire.wait_with_output().unwrap().stdout).unwrap();
        assert!(!(a["ok"] == true && b["ok"] == true), "{a} / {b}");
        for v in [&a, &b] {
            if v["ok"] != true {
                assert!(
                    ["storage_busy", "workspace_busy"]
                        .contains(&v["error"]["code"].as_str().unwrap()),
                    "{v}"
                );
            }
        }
        if a["ok"] == true {
            assert_eq!(f.inspect()["workspace"]["state"], "closing");
        }
        if b["ok"] == true {
            assert_eq!(f.begin()["error"]["code"], "workspace_busy");
        }
        assert!(f.target.is_dir());
    }
}

#[test]
fn closing_blocks_material_creation_updates_repair_and_expansion_without_writes() {
    let f = Fixture::new();
    fs::create_dir_all(f.target.join(".work/templates")).unwrap();
    fs::write(
        f.target.join(".work/templates/plan.yaml"),
        "format_version: 2\nname: plan\nitems: [{key: a, title: NewMaterial}]\n",
    )
    .unwrap();
    let source = f.target.join(format!(".work/items/{}.md", f.other));
    let original = fs::read(&source).unwrap();
    ok(f.begin());
    let before = fs::read_dir(f.target.join(".work/items")).unwrap().count();
    assert_eq!(
        cli(&f.target, &["item", "create", "--title", "late work"])["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        mcp(&f.target, "item_create", json!({"title":"late MCP work"}))["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(
            &f.target,
            &["item", "update", &f.other, "--title", "late update"]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        mcp(&f.target, "item_close", json!({"id":f.other}))["error"]["code"],
        "workspace_busy"
    );
    let raw_hex: String = original.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        mcp(
            &f.target,
            "item_repair",
            json!({"id":f.other,"raw_hex":raw_hex})
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(&f.target, &["template", "expand", "plan"])["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        fs::read_dir(f.target.join(".work/items")).unwrap().count(),
        before
    );
    assert_eq!(fs::read(source).unwrap(), original);
    assert!(
        good(mcp(&f.root, "claim_list", json!({"current_only":true})))["claims"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.remove();
    ok(f.report());
}

#[test]
fn same_context_retry_remains_readable_after_item_closure_or_missing_source() {
    for missing in [false, true] {
        let f = Fixture::new();
        let source = f.root.join(format!(".work/items/{}.md", f.item));
        let original = fs::read(&source).unwrap();
        // Delivered claim release keeps the controller material binding.
        let c = good(mcp(
            &f.root,
            "claim_acquire",
            json!({"item":f.item,"actor":"controller","session":{"namespace":"test","id":"retry"}}),
        ));
        good(mcp(
            &f.root,
            "claim_release",
            json!({"claim_id":c["claim"]["id"],"session":{"namespace":"test","id":"retry"}}),
        ));
        let saved = ok(f.begin())["workspace"].clone();
        if missing {
            fs::remove_file(f.root.join(format!(".work/items/{}.md", f.item))).unwrap();
        } else {
            ok(cli(&f.root, &["item", "close", &f.item]));
        }
        assert_eq!(ok(f.begin()), json!({"workspace":saved,"changed":false}));
        assert_eq!(
            mcp(
                &f.root,
                "workspace_cleanup_begin",
                json!({"workspace_id":f.wid,"item":f.other,"controller_workspace_id":f.control})
            )["error"]["code"],
            "workspace_busy"
        );
        f.remove();
        assert_eq!(
            good(mcp(
                &f.root,
                "workspace_cleanup_begin",
                json!({"workspace_id":f.wid,"item":f.item,"controller_workspace_id":f.control})
            )),
            json!({"workspace":saved,"changed":false})
        );
        if missing {
            assert_eq!(f.report()["error"]["code"], "invalid_source");
            assert!(f.record().exists());
            fs::write(source, original).unwrap();
        }
        ok(f.report());
    }
}

#[test]
fn nested_controller_or_material_checkout_cannot_be_treated_as_surviving() {
    let f = Fixture::new();
    let nested = f.target.join("controller");
    git(
        &f.root,
        &["worktree", "add", "-qb", "nested", nested.to_str().unwrap()],
    );
    let nid = ok(cli(
        &f.root,
        &["workspace", "register", nested.to_str().unwrap()],
    ))["workspace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.bind(&f.item, &nid);
    let source = nested.join(format!(".work/items/{}.md", f.item));
    let original = fs::read(&source).unwrap();
    assert_eq!(
        cli(
            &nested,
            &[
                "workspace",
                "cleanup",
                "begin",
                &f.wid,
                "--item",
                &f.item,
                "--controller-workspace",
                &nid
            ]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        mcp(
            &f.root,
            "workspace_cleanup_begin",
            json!({"workspace_id":f.wid,"item":f.item,"controller_workspace_id":nid})
        )["error"]["code"],
        "workspace_busy"
    );
    // A distinct controller outside the target does not make other nested
    // authoritative sources survive the target's physical removal.
    f.bind(&f.item, &f.control);
    f.bind(&f.other, &nid);
    let blocked = f.begin();
    assert_eq!(blocked["error"]["code"], "workspace_busy");
    assert!(
        blocked["error"]["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["kind"] == "nested_workspace" && b["id"] == nid)
    );
    assert_eq!(f.inspect()["workspace"]["state"], "open");
    assert_eq!(fs::read(source).unwrap(), original);
    assert!(nested.is_dir());
}

#[test]
fn closing_ancestor_refuses_registration_and_claim_setup_in_new_nested_checkout() {
    let f = Fixture::new();
    ok(f.begin());
    let nested = f.target.join("new-child");
    git(
        &f.root,
        &["worktree", "add", "-qb", "nested", nested.to_str().unwrap()],
    );
    assert_eq!(
        mcp(&f.root, "workspace_register", json!({"path":nested}))["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(
            &nested,
            &[
                "claim",
                "acquire",
                &f.other,
                "--actor",
                "late",
                "--session-namespace",
                "test",
                "--session-id",
                "late"
            ]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(&nested, &["run", "start", &f.other])["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        good(mcp(&f.root, "workspace_list", json!({})))["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(f.inspect()["workspace"]["state"], "closing");
}

#[test]
fn target_containing_repository_shared_storage_cannot_be_closed() {
    let f = Fixture::new();
    f.bind(&f.item, &f.wid);
    let blocked = cli(
        &f.target,
        &[
            "workspace",
            "cleanup",
            "begin",
            &f.control,
            "--item",
            &f.item,
            "--controller-workspace",
            &f.wid,
        ],
    );
    assert_eq!(blocked["error"]["code"], "workspace_busy");
    assert!(
        blocked["error"]["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["kind"] == "shared_storage")
    );
    assert_eq!(
        good(mcp(
            &f.root,
            "workspace_inspect",
            json!({"workspace_id":f.control})
        ))["workspace"]["state"],
        "open"
    );
}

#[test]
fn shared_lock_contention_and_return_do_not_hold_lock_during_external_removal() {
    let f = Fixture::new();
    let lock = fs::File::open(f.root.join(".git/work/coordination.lock")).unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    assert_eq!(f.begin()["error"]["code"], "storage_busy");
    drop(lock);
    ok(f.begin());
    let lock = fs::File::open(f.root.join(".git/work/coordination.lock")).unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    f.remove(); // Kernel lock demonstrably not retained by the Work process.
    drop(lock);
    ok(f.report());
}

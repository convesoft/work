//! Context acceptance uses only disposable repos and real CLI/MCP processes.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "work-context-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "--initial-branch=main"]);
        fs::create_dir_all(root.join(".work/items")).unwrap();
        ok(cli(&root, &["storage", "init"]));
        Self { root }
    }
    fn item(&self) -> String {
        ok(cli(
            &self.root,
            &["item", "create", "--title", "context", "--body", "original"],
        ))["item"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn linked(&self) -> PathBuf {
        git(&self.root, &["add", ".work/items"]);
        git(
            &self.root,
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
        let linked = self.root.join("linked");
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                linked.to_str().unwrap(),
            ],
        );
        linked
    }
    fn register(&self, path: &Path) -> String {
        ok(cli(
            &self.root,
            &["workspace", "register", path.to_str().unwrap()],
        ))["workspace"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn run(&self, root: &str) -> String {
        ok(cli(&self.root, &["run", "start", root]))["run"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn terminal(&self, run: &str) {
        // Finalization is a successor. Load valid terminal fixture metadata;
        // never simulate its mutation interface or claim its acceptance.
        let path = self.root.join(format!(".git/work/runs/{run}/run.yaml"));
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v["phase"] = json!("disposed");
        v["ended_at"] = json!("2026-10-01T10:30:00Z");
        v["cleanup"] = json!({"kind":"discard","finalize":true,"item_ids":[],"session_ids":[],"started_at":"2026-10-01T10:30:00Z"});
        fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn cli(root: &Path, args: &[&str]) -> Value {
    let o = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("--json")
        .arg("--worktree")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&o.stderr)))
}
fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}
fn mcp(root: &Path, name: &str, mut args: Value) -> Value {
    args["worktree"] = json!(root.to_str().unwrap());
    let mut c = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("mcp")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = c.stdin.take().unwrap();
    writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    drop(stdin);
    let o = c.wait_with_output().unwrap();
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).lines().last().unwrap()).unwrap();
    v["result"]["structuredContent"].clone()
}
fn session(id: &str) -> Value {
    json!({"namespace":"external / α", "id":id})
}
fn set(f: &Fixture, run: &str, name: &str, id: &str) -> Value {
    let v = mcp(
        &f.root,
        "session_set",
        json!({"run_id":run,"name":name,"session":session(id)}),
    );
    assert!(v["error"].is_null(), "{v}");
    v
}
fn claim(f: &Fixture, item: &str, external: &str, named: &str) -> Value {
    let v = mcp(
        &f.root,
        "claim_acquire",
        json!({"item":item,"actor":"worker", "session":session(external),"session_record_id":named}),
    );
    assert!(v["error"].is_null(), "{v}");
    v
}
fn release(f: &Fixture, claim: &Value, external: &str) {
    let v = mcp(
        &f.root,
        "claim_release",
        json!({"claim_id":claim["claim"]["id"],"session":session(external)}),
    );
    assert!(v["error"].is_null(), "{v}");
}
#[test]
fn named_rebind_remove_keeps_captured_ownership_and_workspace_across_restart() {
    let f = Fixture::new();
    let root = f.item();
    let a = f.item();
    let b = f.item();
    let run = f.run(&root);
    ok(cli(&f.root, &["run", "attach", &run, &a, &b]));
    let first = set(&f, &run, "Worker α", "first");
    let sid = first["session_record"]["id"].as_str().unwrap();
    let unchanged = set(&f, &run, "Worker α", "first");
    assert_eq!(unchanged["changed"], false);
    let captured = claim(&f, &a, "first", sid);
    let raw = fs::read(f.root.join(format!(
        ".git/work/claims/{}.yaml",
        captured["claim"]["id"].as_str().unwrap()
    )))
    .unwrap();
    let rebound = ok(cli(
        &f.root,
        &[
            "session",
            "set",
            &run,
            "Worker α",
            "--namespace",
            "external / α",
            "--session-id",
            "second",
            "--availability",
            "unavailable",
            "--observed-at",
            "2026-10-01T11:00:00+01:00",
        ],
    ));
    assert_eq!(rebound["session_record"]["id"], sid);
    assert_eq!(
        rebound["session_record"]["availability"]["state"],
        "unavailable"
    );
    assert_eq!(
        mcp(&f.root, "session_list", json!({"run_id":run})),
        ok(cli(&f.root, &["session", "list", &run]))
    );
    let wrong = mcp(
        &f.root,
        "claim_acquire",
        json!({"item":b,"actor":"worker", "session":session("first"),"session_record_id":sid}),
    );
    assert_eq!(wrong["error"]["code"], "invalid_argument");
    let second = ok(cli(
        &f.root,
        &[
            "claim",
            "acquire",
            &b,
            "--actor",
            "worker",
            "--session-namespace",
            "external / α",
            "--session-id",
            "second",
            "--session-record",
            sid,
        ],
    ));
    assert_eq!(second["claim"]["session_record_id"], sid);
    assert_eq!(
        second["claim"]["workspace_id"],
        captured["claim"]["workspace_id"]
    );
    let wid = captured["claim"]["workspace_id"].as_str().unwrap();
    let users = ok(cli(&f.root, &["workspace", "inspect", wid]));
    assert_eq!(
        users["users"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|u| u["kind"] == "claim")
            .count(),
        2
    );
    assert_eq!(
        mcp(
            &f.root,
            "claim_release",
            json!({"claim_id":captured["claim"]["id"],"session":session("second")})
        )["error"]["code"],
        "stale_claim"
    );
    assert_eq!(
        ok(cli(&f.root, &["session", "remove", &run, "Worker α"]))["changed"],
        true
    );
    assert_eq!(
        mcp(
            &f.root,
            "session_remove",
            json!({"run_id":run,"name":"Worker α"})
        )["changed"],
        false
    );
    assert_eq!(
        fs::read(f.root.join(format!(
            ".git/work/claims/{}.yaml",
            captured["claim"]["id"].as_str().unwrap()
        )))
        .unwrap(),
        raw
    );
    release(&f, &captured, "first");
    release(&f, &second, "second");
    assert!(f.root.is_dir());
    assert_eq!(
        ok(cli(&f.root, &["workspace", "inspect", wid]))["bindings"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn workspace_reuse_rebind_unbind_and_cleanup_user_evidence() {
    let f = Fixture::new();
    let root = f.item();
    let item = f.item();
    let linked = f.linked();
    let registered = ok(cli(
        &f.root,
        &[
            "workspace",
            "register",
            linked.to_str().unwrap(),
            "--branch",
            "observed-feature",
            "--commit",
            "opaque-revision",
        ],
    ));
    let wid = registered["workspace"]["id"].as_str().unwrap();
    let same = mcp(
        &f.root,
        "workspace_register",
        json!({"path":linked.join(".").to_str().unwrap(),"branch":"ignored-new-observation"}),
    );
    assert_eq!(same["changed"], false);
    assert_eq!(same["workspace"], registered["workspace"]);
    assert_eq!(
        mcp(&f.root, "workspace_list", json!({})),
        ok(cli(&f.root, &["workspace", "list"]))
    );
    let bound = ok(cli(&f.root, &["workspace", "bind", &item, wid]));
    assert_eq!(bound["changed"], true);
    assert_eq!(
        mcp(
            &f.root,
            "workspace_bind",
            json!({"item":item,"workspace_id":wid})
        )["changed"],
        false
    );
    let file = linked.join(format!(".work/items/{item}.md"));
    let original = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        original.replace("original", "uncommitted linked progress"),
    )
    .unwrap();
    assert!(
        ok(cli(&f.root, &["item", "inspect", &item]))["item"]["body"]
            .as_str()
            .unwrap()
            .contains("uncommitted linked progress")
    );
    let named_root_workspace = f.register(&f.root);
    let run = f.run(&root);
    ok(cli(&f.root, &["run", "attach", &run, &item]));
    let record = set(&f, &run, "persistent", "one");
    let sid = record["session_record"]["id"].as_str().unwrap();
    let owned = claim(&f, &item, "one", sid);
    assert_eq!(owned["claim"]["workspace_id"], wid); // material override beats default
    assert_eq!(
        cli(
            &f.root,
            &["workspace", "bind", &item, &named_root_workspace]
        )["error"]["code"],
        "claim_conflict"
    );
    release(&f, &owned, "one");
    ok(cli(&f.root, &["item", "close", &item]));
    assert_eq!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"][0]["id"],
        sid
    );
    assert_eq!(
        cli(&f.root, &["workspace", "unbind", &item])["error"]["code"],
        "run_conflict"
    );
    let before = mcp(&f.root, "workspace_inspect", json!({"workspace_id":wid}));
    assert!(
        before["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["kind"] == "run_material")
    );
    // External merge/relocation is explicit. Work must not copy item progress.
    fs::copy(&file, f.root.join(format!(".work/items/{item}.md"))).unwrap();
    fs::remove_file(&file).unwrap(); // old source missing: full-ID rebind can repair it
    assert_eq!(
        ok(cli(
            &f.root,
            &["workspace", "bind", &item, &named_root_workspace]
        ))["changed"],
        true
    );
    let after = mcp(&f.root, "workspace_inspect", json!({"workspace_id":wid}));
    assert!(after["users"].as_array().unwrap().is_empty());
    assert!(linked.exists()); // eligibility evidence is not physical cleanup
    // Build a terminal fixture with execution context already removed; the
    // actual finalization/deletion ordering belongs to the successor item.
    ok(cli(&f.root, &["session", "remove", &run, "persistent"]));
    f.terminal(&run);
    assert!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        mcp(
            &f.root,
            "session_remove",
            json!({"run_id":run,"name":"persistent"})
        )["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        mcp(&f.root, "workspace_unbind", json!({"item":item}))["changed"],
        true
    );
    assert_eq!(
        ok(cli(&f.root, &["workspace", "unbind", &item]))["changed"],
        false
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &item]))["item"]["state"],
        "done"
    );
    assert!(
        ok(cli(
            &f.root,
            &["workspace", "inspect", &named_root_workspace]
        ))["users"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["kind"] == "binding")
    );
}
#[test]
fn session_scope_strict_inputs_and_corruption_never_become_empty_context() {
    let f = Fixture::new();
    let root = f.item();
    let other = f.item();
    let run = f.run(&root);
    let run2 = f.run(&other);
    for args in [
        vec!["session", "set", &run, "incomplete", "--namespace", "n"],
        vec!["session", "set", &run, "incomplete", "--session-id", "s"],
        vec![
            "session",
            "set",
            &run,
            "",
            "--namespace",
            "n",
            "--session-id",
            "s",
        ],
        vec![
            "session",
            "set",
            &run,
            "bad-observation",
            "--namespace",
            "n",
            "--session-id",
            "s",
            "--availability",
            "unknown",
        ],
    ] {
        assert_eq!(cli(&f.root, &args)["error"]["code"], "invalid_argument");
    }
    let a = set(&f, &run, "Name", "a");
    let b = set(&f, &run2, "Name", "b");
    assert_ne!(a["session_record"]["id"], b["session_record"]["id"]);
    set(&f, &run, "name", "case sensitive");
    assert_eq!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let unregistered = f.item();
    for (item, sid, external, code) in [
        (
            &root,
            b["session_record"]["id"].as_str().unwrap(),
            "b",
            "not_found",
        ),
        (
            &unregistered,
            a["session_record"]["id"].as_str().unwrap(),
            "a",
            "invalid_argument",
        ),
    ] {
        assert_eq!(
            mcp(
                &f.root,
                "claim_acquire",
                json!({"item":item,"actor":"worker","session":session(external),"session_record_id":sid})
            )["error"]["code"],
            code
        );
    }
    for availability in [
        json!({"state":"alive","observed_at":"2026-01-01T00:00:00Z"}),
        json!({"state":"unknown","observed_at":"2026-02-30T00:00:00Z"}),
        json!({"state":"unknown"}),
        json!({"state":"unknown","observed_at":"2026-01-01T00:00:00Z","extra":"bad"}),
        Value::Null,
    ] {
        assert_eq!(
            mcp(
                &f.root,
                "session_set",
                json!({"run_id":run,"name":"bad","session":session("a"),"availability":availability})
            )["error"]["code"],
            "invalid_argument"
        );
    }
    let owned = claim(&f, &root, "a", a["session_record"]["id"].as_str().unwrap());
    let path = f.root.join(format!(
        ".git/work/runs/{run}/sessions/{}.yaml",
        a["session_record"]["id"].as_str().unwrap()
    ));
    let original = fs::read(&path).unwrap();
    for mutation in [
        "unknown",
        "version",
        "identity",
        "duplicate-name",
        "bad-session",
        "null",
    ] {
        let mut v: Value = serde_json::from_slice(&original).unwrap();
        match mutation {
            "unknown" => v["extra"] = json!("bad"),
            "version" => v["format_version"] = json!(2),
            "identity" => v["run_id"] = json!(run2),
            "duplicate-name" => v["name"] = json!("name"),
            "bad-session" => v["session"]["id"] = json!(""),
            "null" => v["availability"] = Value::Null,
            _ => unreachable!(),
        }
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(cli(&f.root, &["session", "list", &run])["error"].is_object());
        assert!(cli(&f.root, &["item", "ready"])["error"].is_object());
    }
    release(&f, &owned, "a"); // stopping ownership still works with damaged context
    fs::write(&path, original).unwrap();
    assert_eq!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn concurrent_set_same_name_has_one_record_and_one_create() {
    let f = Fixture::new();
    let root = f.item();
    let run = f.run(&root);
    let r = f.root.clone();
    let run2 = run.clone();
    let t = std::thread::spawn(move || {
        mcp(
            &r,
            "session_set",
            json!({"run_id":run2,"name":"worker","session":session("same")}),
        )
    });
    let args = [
        "session",
        "set",
        &run,
        "worker",
        "--namespace",
        "external / α",
        "--session-id",
        "same",
    ];
    let mut cli_result = cli(&f.root, &args);
    let mut mcp_result = t.join().unwrap();
    // The foundation deliberately reports contention rather than waiting.
    // Retry the loser after both independent processes have stopped.
    if cli_result["ok"] != true {
        assert_eq!(cli_result["error"]["code"], "storage_busy");
        cli_result = cli(&f.root, &args);
    }
    if mcp_result["error"].is_object() {
        assert_eq!(mcp_result["error"]["code"], "storage_busy");
        mcp_result = mcp(
            &f.root,
            "session_set",
            json!({"run_id":run,"name":"worker","session":session("same")}),
        );
    }
    let cli_result = ok(cli_result);
    assert!(mcp_result["error"].is_null(), "{mcp_result}");
    assert_eq!(cli_result["session_record"], mcp_result["session_record"]);
    assert_ne!(cli_result["changed"], mcp_result["changed"]);
    assert_eq!(
        ok(crate::cli(&f.root, &["session", "list", &run]))["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn wisp_uses_run_default_without_material_binding_and_names_survive_completion() {
    let f = Fixture::new();
    let root = f.item();
    let linked = f.linked();
    let wid = f.register(&linked);
    let run = ok(cli(&f.root, &["run", "start", &root, "--workspace", &wid]))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::create_dir_all(f.root.join(".work/templates")).unwrap();
    fs::write(
        f.root.join(".work/templates/temp.yaml"),
        "format_version: 2\nname: temp\nitems: [{key: task, title: Task, persistence: wisp}]\n",
    )
    .unwrap();
    let expansion = ok(cli(&f.root, &["template", "expand", "temp", "--run", &run]));
    let item = expansion["items"][0]["id"].as_str().unwrap();
    let named = set(&f, &run, "worker", "wisp");
    let owned = claim(
        &f,
        item,
        "wisp",
        named["session_record"]["id"].as_str().unwrap(),
    );
    assert_eq!(owned["claim"]["workspace_id"], wid);
    assert_eq!(owned["claim"]["run_id"], run);
    assert!(
        !f.root
            .join(format!(".git/work/workspaces/items/{item}.yaml"))
            .exists()
    );
    let authorization = json!([{"claim_id":owned["claim"]["id"],"session":session("wisp")}]);
    assert!(
        mcp(
            &f.root,
            "item_close",
            json!({"id":item,"authorization":authorization})
        )["error"]
            .is_null()
    );
    assert_eq!(
        ok(cli(&f.root, &["run", "inspect", &run]))["finished"],
        true
    );
    assert_eq!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"][0]["id"],
        named["session_record"]["id"]
    );
    let inspection = mcp(&f.root, "workspace_inspect", json!({"workspace_id":wid}));
    assert!(inspection["bindings"].as_array().unwrap().is_empty());
    assert!(
        inspection["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["kind"] == "run_default")
    );
    assert!(
        inspection["users"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["kind"] != "claim")
    );
    // Valid frozen manifest fixture owned by the finalization successor.
    let path = f.root.join(format!(".git/work/runs/{run}/run.yaml"));
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["phase"] = json!("discarding");
    value["cleanup"] = json!({"kind":"discard","finalize":true,"item_ids":[item],"session_ids":[named["session_record"]["id"]],"started_at":"2026-10-01T10:30:00Z"});
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        mcp(
            &f.root,
            "session_set",
            json!({"run_id":run,"name":"worker","session":session("new")})
        )["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        cli(&f.root, &["session", "remove", &run, "worker"])["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        ok(cli(&f.root, &["session", "list", &run]))["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn closing_and_missing_workspaces_remain_inspectable_but_refuse_assignments() {
    let f = Fixture::new();
    let item = f.item();
    let linked = f.linked();
    let wid = f.register(&linked);
    ok(cli(&f.root, &["workspace", "bind", &item, &wid]));
    let unrelated = Fixture::new();
    assert_eq!(
        cli(
            &f.root,
            &["workspace", "register", unrelated.root.to_str().unwrap()]
        )["error"]["code"],
        "invalid_argument"
    );
    let record = f.root.join(format!(".git/work/workspaces/{wid}.yaml"));
    let mut value: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    value["state"] = json!("closing");
    fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        cli(&f.root, &["workspace", "bind", &item, &wid])["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        cli(
            &f.root,
            &[
                "claim",
                "acquire",
                &item,
                "--actor",
                "worker",
                "--session-namespace",
                "n",
                "--session-id",
                "s"
            ]
        )["error"]["code"],
        "workspace_busy"
    );
    assert_eq!(
        ok(cli(&f.root, &["workspace", "inspect", &wid]))["workspace"]["state"],
        "closing"
    );
    value["state"] = json!("open");
    fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::rename(&linked, f.root.join("externally-moved")).unwrap();
    let inspection = ok(cli(&f.root, &["workspace", "inspect", &wid]));
    assert!(inspection["users"][0]["effective_done"].is_null());
    assert!(
        !inspection["users"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(cli(&f.root, &["item", "inspect", &item])["error"].is_object());
    // Explicit removal of the location reference restores selected-view semantics.
    assert_eq!(
        ok(cli(&f.root, &["workspace", "unbind", &item]))["changed"],
        true
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &item]))["item"]["body"],
        "original"
    );
    assert_eq!(
        ok(cli(&f.root, &["workspace", "list"]))["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn multiple_missing_current_run_sources_can_be_rebound_one_at_a_time() {
    let f = Fixture::new();
    let root = f.item();
    let a = f.item();
    let b = f.item();
    let linked = f.linked();
    let old = f.register(&linked);
    let surviving = f.register(&f.root);
    for item in [&a, &b] {
        ok(cli(&f.root, &["workspace", "bind", item, &old]));
        let source = linked.join(format!(".work/items/{item}.md"));
        let raw = fs::read_to_string(&source)
            .unwrap()
            .replace("original", "externally retained progress");
        fs::write(&source, &raw).unwrap();
        // External integration retained each whole material file first.
        fs::write(f.root.join(format!(".work/items/{item}.md")), raw).unwrap();
    }
    let run = f.run(&root);
    ok(cli(&f.root, &["run", "attach", &run, &a, &b]));
    let manifest = f.root.join(format!(".git/work/runs/{run}/run.yaml"));
    let run_before = fs::read(&manifest).unwrap();
    fs::rename(&linked, f.root.join("moved-checkout")).unwrap();
    assert_eq!(
        cli(&f.root, &["workspace", "unbind", &a])["error"]["code"],
        "run_conflict"
    );
    assert_eq!(
        ok(cli(&f.root, &["workspace", "bind", &a, &surviving]))["changed"],
        true
    );
    // B is still unavailable: location repair must not hide its diagnostic or
    // make the incomplete graph dispatchable while allowing the next repair.
    assert!(cli(&f.root, &["item", "ready"])["error"].is_object());
    let partial = mcp(&f.root, "workspace_inspect", json!({"workspace_id":old}));
    assert_eq!(partial["bindings"].as_array().unwrap().len(), 1);
    assert!(
        partial["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["kind"] == "binding" && u["id"] == b && u["effective_done"].is_null())
    );
    assert_eq!(
        mcp(
            &f.root,
            "workspace_bind",
            json!({"item":b,"workspace_id":surviving})
        )["changed"],
        true
    );
    ok(cli(&f.root, &["item", "ready"]));
    for item in [&a, &b] {
        let inspection = ok(cli(&f.root, &["item", "inspect", item]));
        assert_eq!(inspection["item"]["body"], "externally retained progress");
        assert_eq!(
            inspection["item"]["source_worktree"],
            f.root.to_str().unwrap()
        );
    }
    assert_eq!(fs::read(&manifest).unwrap(), run_before);
    assert!(
        mcp(&f.root, "workspace_inspect", json!({"workspace_id":old}))["users"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

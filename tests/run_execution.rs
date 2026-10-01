//! Real CLI/MCP ownership and material-source routing acceptance.
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
            "work-execution-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "--initial-branch=main"]);
        fs::create_dir_all(root.join(".work/items")).unwrap();
        Self { root }
    }
    fn item(&self) -> String {
        ok(cli(
            &self.root,
            &[
                "item",
                "create",
                "--title",
                "original",
                "--body",
                "original body",
            ],
        ))["item"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn init(&self) {
        ok(cli(&self.root, &["storage", "init"]));
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
        let path = self.root.join("linked");
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                path.to_str().unwrap(),
            ],
        );
        path
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
    serde_json::from_slice(&o.stdout).unwrap_or_else(|_| {
        panic!(
            "{} {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    })
}
fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}
fn mcp(root: &Path, name: &str, args: Value) -> Value {
    let mut c = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("mcp")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = c.stdin.take().unwrap();
    writeln!(stdin,"{}",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    writeln!(stdin,"{}",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    drop(stdin);
    let o = c.wait_with_output().unwrap();
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).lines().last().unwrap()).unwrap();
    v["result"]["structuredContent"].clone()
}
fn session() -> Value {
    json!({"namespace":"codex","id":"session / opaque:α"})
}
fn acquire(root: &Path, id: &str) -> Value {
    ok(cli(
        root,
        &[
            "claim",
            "acquire",
            id,
            "--actor",
            "worker",
            "--session-namespace",
            "codex",
            "--session-id",
            "session / opaque:α",
        ],
    ))
}

fn template(f: &Fixture, name: &str, source: &str) {
    fs::create_dir_all(f.root.join(".work/templates")).unwrap();
    fs::write(f.root.join(format!(".work/templates/{name}.yaml")), source).unwrap();
}
#[test]
fn rootless_planning_and_readonly_mixed_preview_through_cli_mcp() {
    let f = Fixture::new();
    template(
        &f,
        "plan",
        "format_version: 2\nname: plan\nitems: [{key: a, title: Alpha}, {key: b, title: Beta}]\nedges: [{from: 'local:b', kind: depends_on, to: 'local:a'}]\n",
    );
    let preview = ok(cli(&f.root, &["template", "preview", "plan"]));
    assert_eq!(preview["preview"]["root"], Value::Null);
    assert!(!f.root.join(".git/work").exists());
    assert_eq!(
        mcp(&f.root, "template_preview", json!({"name":"plan"})),
        preview
    );
    assert!(cli(&f.root, &["template", "expand", "plan"])["error"].is_object());
    f.init();
    let result = mcp(&f.root, "template_expand", json!({"name":"plan"}));
    assert!(result["error"].is_null(), "{result}");
    assert!(result["run_id"].is_null());
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let a = items[0]["id"].as_str().unwrap();
    let b = items[1]["id"].as_str().unwrap();
    assert!(f.root.join(format!(".work/items/{a}.md")).is_file());
    let inspected = ok(cli(&f.root, &["item", "inspect", b]));
    assert_eq!(inspected["item"]["depends_on"], json!([a]));
    assert_eq!(
        fs::read_dir(f.root.join(".git/work/runs")).unwrap().count(),
        0
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "ready"]))["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn mixed_expansion_claimed_wisp_completion_and_derived_finish_survive_restart() {
    let f = Fixture::new();
    let root = f.item();
    f.init();
    let started = ok(cli(&f.root, &["run", "start", &root]));
    let run = started["run"]["id"].as_str().unwrap();
    let repeated = mcp(&f.root, "run_start", json!({"root":root}));
    assert_eq!(repeated["changed"], false);
    assert_eq!(repeated["run"]["id"], run);
    template(
        &f,
        "mixed",
        "format_version: 2\nname: mixed\nitems: [{key: a, title: Material}, {key: b, title: Wisp, persistence: wisp}]\nedges: [{from: 'local:a', kind: depends_on, to: 'local:b'}]\n",
    );
    let preview = ok(cli(
        &f.root,
        &["template", "preview", "mixed", "--run", run],
    ));
    assert_eq!(preview["preview"]["items"]["b"]["persistence"], "wisp");
    assert_eq!(
        mcp(
            &f.root,
            "template_preview",
            json!({"name":"mixed","run_id":run})
        ),
        preview
    );
    let result = ok(cli(&f.root, &["template", "expand", "mixed", "--run", run]));
    let material = result["items"][0]["id"].as_str().unwrap();
    let wisp = result["items"][1]["id"].as_str().unwrap();
    assert!(f.root.join(format!(".work/items/{material}.md")).is_file());
    assert!(
        f.root
            .join(format!(".git/work/runs/{run}/items/{wisp}.md"))
            .is_file()
    );
    assert!(!f.root.join(format!(".work/items/{wisp}.md")).exists());
    let linked = f.linked();
    assert_eq!(
        mcp(&linked, "item_inspect", json!({"id":wisp}))["item"]["persistence"],
        "wisp"
    );
    let state = mcp(&f.root, "run_inspect", json!({"run_id":run}));
    assert_eq!(state["finished"], false);
    assert_eq!(state["run"]["material_items"], json!([material]));
    let claimed = acquire(&f.root, wisp);
    let pair = json!({"claim_id":claimed["claim"]["id"],"session":session()});
    assert_eq!(claimed["item"]["persistence"], "wisp");
    assert_eq!(claimed["claim"]["run_id"], run);
    let closed = mcp(
        &f.root,
        "item_close",
        json!({"id":wisp,"authorization":[pair]}),
    );
    assert_eq!(closed["item"]["state"], "done", "{closed}");
    ok(cli(&f.root, &["item", "close", material]));
    assert_eq!(
        mcp(&f.root, "run_inspect", json!({"run_id":run}))["finished"],
        true
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &root]))["item"]["state"],
        "open"
    );
    ok(cli(&f.root, &["item", "reopen", wisp]));
    assert_eq!(
        mcp(&f.root, "run_inspect", json!({"run_id":run}))["finished"],
        false
    );
    let detached = mcp(
        &f.root,
        "run_detach",
        json!({"run_id":run,"items":[material]}),
    );
    assert_eq!(detached["changed"], true, "{detached}");
    assert_eq!(
        ok(cli(&f.root, &["run", "attach", run, material]))["changed"],
        true
    );
    assert_eq!(
        mcp(
            &f.root,
            "run_attach",
            json!({"run_id":run,"items":[material]})
        )["changed"],
        false
    );
}

#[test]
fn terminal_fixture_keeps_material_binding_and_uncommitted_worktree_state() {
    let f = Fixture::new();
    let root = f.item();
    let member = f.item();
    let linked = f.linked();
    f.init();
    let relative = format!(".work/items/{member}.md");
    let original = fs::read(f.root.join(&relative)).unwrap();
    fs::write(
        linked.join(&relative),
        String::from_utf8(original.clone())
            .unwrap()
            .replace("original body", "uncommitted feature body"),
    )
    .unwrap();
    let claim = acquire(&linked, &member);
    ok(cli(
        &f.root,
        &[
            "claim",
            "release",
            claim["claim"]["id"].as_str().unwrap(),
            "--session-namespace",
            "codex",
            "--session-id",
            "session / opaque:α",
        ],
    ));
    let started = ok(cli(&f.root, &["run", "start", &root]));
    let run = started["run"]["id"].as_str().unwrap();
    ok(cli(&f.root, &["run", "attach", run, &member]));
    ok(cli(&f.root, &["item", "close", &member]));
    let binding = f
        .root
        .join(format!(".git/work/workspaces/items/{member}.yaml"));
    let binding_bytes = fs::read(&binding).unwrap();
    // Terminal transition is supplied as a future-finalization fixture, not a command in this slice.
    let manifest = f.root.join(format!(".git/work/runs/{run}/run.yaml"));
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["phase"] = json!("disposed");
    value["ended_at"] = json!("2026-10-01T11:00:00Z");
    value["cleanup"] = json!({"kind":"discard","finalize":true,"item_ids":[],"session_ids":[],"started_at":"2026-10-01T10:30:00Z"});
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let item = mcp(&f.root, "item_inspect", json!({"id":member}));
    assert_eq!(item["item"]["body"], "uncommitted feature body");
    assert_eq!(item["item"]["state"], "done");
    assert_eq!(item["item"]["source_worktree"], linked.to_str().unwrap());
    assert!(item["item"]["run_id"].is_null());
    assert_eq!(fs::read(binding).unwrap(), binding_bytes);
    assert_eq!(fs::read(f.root.join(relative)).unwrap(), original);
    let next = ok(cli(&f.root, &["run", "start", &root]));
    assert_ne!(next["run"]["id"], run);
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &root]))["item"]["state"],
        "open"
    );
}

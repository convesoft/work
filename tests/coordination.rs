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
#[test]
fn bound_whole_file_is_read_and_mutated_from_main_without_touching_main_item() {
    let f = Fixture::new();
    let id = f.item();
    let linked = f.linked();
    let rel = format!(".work/items/{id}.md");
    let main = fs::read(f.root.join(&rel)).unwrap();
    let text = String::from_utf8(main.clone())
        .unwrap()
        .replace("original body", "feature body")
        .replace("original", "feature title");
    fs::write(linked.join(&rel), text).unwrap();
    f.init();
    let a = acquire(&linked, &id);
    let claim = a["claim"]["id"].as_str().unwrap();
    let pair = json!({"claim_id":claim,"session":session()});
    let read = ok(cli(&f.root, &["item", "inspect", &id]));
    assert_eq!(read["item"]["body"], "feature body");
    assert_eq!(read["item"]["source_worktree"], linked.to_str().unwrap());
    assert_eq!(read["item"]["claim"]["id"], claim);
    assert_eq!(
        mcp(&f.root, "item_inspect", json!({"id":id}))["item"],
        read["item"]
    );
    assert!(
        ok(cli(&f.root, &["item", "ready"]))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        cli(&f.root, &["item", "close", &id])["error"]["code"],
        "claim_conflict"
    );
    let update = mcp(
        &f.root,
        "item_update",
        json!({"id":id,"title":"changed from main","authorization":[pair.clone()]}),
    );
    assert_eq!(update["item"]["title"], "changed from main", "{update}");
    let branch = ok(cli(
        &f.root,
        &["item", "inspect", &id, "--view", "checkout"],
    ));
    assert_eq!(branch["item"]["body"], "original body");
    let closed = ok(cli(
        &f.root,
        &[
            "item",
            "close",
            &id,
            "--authorize",
            &pair.to_string(),
            "--reason",
            "done",
        ],
    ));
    assert_eq!(closed["item"]["state"], "done");
    assert_eq!(fs::read(f.root.join(&rel)).unwrap(), main);
    assert_eq!(
        ok(cli(&f.root, &["claim", "inspect", claim]))["ending"]["outcome"],
        "completed"
    );
    assert_eq!(
        mcp(
            &f.root,
            "item_reopen",
            json!({"id":id,"authorization":[pair]})
        )["error"]["code"],
        "stale_claim"
    );
}
#[test]
fn actual_claim_calls_refuse_uninitialized_and_corrupt_storage_while_reads_warn() {
    let f = Fixture::new();
    let id = f.item();
    let args = json!({"item":id,"actor":"a","session":session()});
    let before = mcp(&f.root, "claim_acquire", args.clone());
    assert_eq!(before["error"]["code"], "storage_missing");
    assert!(!f.root.join(".git/work").exists());
    f.init();
    fs::write(f.root.join(".git/work/store.yaml"), "broken: [").unwrap();
    let denied = mcp(&f.root, "claim_acquire", args);
    assert_eq!(denied["error"]["code"], "recovery_required");
    let read = ok(cli(&f.root, &["item", "ready"]));
    assert!(read["storage_warning"].is_object());
    assert_eq!(read["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        cli(
            &f.root,
            &[
                "claim",
                "acquire",
                &id,
                "--actor",
                "a",
                "--session-namespace",
                "codex",
                "--session-id",
                "x"
            ]
        )["error"]["code"],
        denied["error"]["code"]
    );
}
#[test]
fn cli_and_mcp_compete_for_one_owner_then_release_without_item_source() {
    let f = Fixture::new();
    let id = f.item();
    f.init();
    let root = f.root.clone();
    let id2 = id.clone();
    let a = std::thread::spawn(move || {
        cli(
            &root,
            &[
                "claim",
                "acquire",
                &id2,
                "--actor",
                "cli",
                "--session-namespace",
                "codex",
                "--session-id",
                "session / opaque:α",
            ],
        )
    });
    let b = mcp(
        &f.root,
        "claim_acquire",
        json!({"item":id,"actor":"mcp","session":session()}),
    );
    let a = a.join().unwrap();
    assert_eq!(
        usize::from(a["ok"] == true) + usize::from(b["claim"].is_object()),
        1,
        "{a} {b}"
    );
    let owner = ok(cli(&f.root, &["claim", "list", "--current"]));
    let claim = owner["claims"][0]["claim"]["id"].as_str().unwrap();
    assert_eq!(owner["claims"].as_array().unwrap().len(), 1);
    fs::remove_file(f.root.join(format!(".work/items/{id}.md"))).unwrap();
    let released = mcp(
        &f.root,
        "claim_release",
        json!({"claim_id":claim,"session":session()}),
    );
    assert_eq!(released["ending"]["outcome"], "released", "{released}");
    assert_eq!(
        mcp(
            &f.root,
            "claim_release",
            json!({"claim_id":claim,"session":session()})
        )["changed"],
        false
    );
}

#[test]
fn checkout_view_survives_missing_bound_source_and_malformed_workspace() {
    let f = Fixture::new();
    let id = f.item();
    let linked = f.linked();
    f.init();
    acquire(&linked, &id);
    fs::remove_file(linked.join(format!(".work/items/{id}.md"))).unwrap();
    let missing = cli(&f.root, &["item", "inspect", &id]);
    assert_eq!(missing["error"]["code"], "invalid_source");
    let source = linked.join(format!(".work/items/{id}.md"));
    assert!(
        missing["error"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["path"] == source.to_str().unwrap()),
        "{missing}"
    );
    assert_eq!(
        mcp(&f.root, "item_inspect", json!({"id":id}))["error"],
        missing["error"]
    );
    for damaged in [false, true] {
        if damaged {
            let dir = f.root.join(".git/work/workspaces");
            let p = fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "yaml"))
                .unwrap();
            fs::write(p, "broken: [").unwrap();
        }
        let read = ok(cli(
            &f.root,
            &["item", "inspect", &id, "--view", "checkout"],
        ));
        assert_eq!(read["item"]["body"], "original body");
        assert_eq!(
            mcp(&f.root, "item_inspect", json!({"id":id,"view":"checkout"}))["item"],
            read["item"]
        );
        for (verb, tool) in [("list", "item_list"), ("ready", "item_ready")] {
            let values = ok(cli(&f.root, &["item", verb, "--view", "checkout"]));
            assert_eq!(values["items"].as_array().unwrap().len(), 1);
            assert_eq!(
                mcp(&f.root, tool, json!({"view":"checkout"}))["items"],
                values["items"]
            );
        }
    }
}

#[test]
fn option_values_that_resemble_new_flags_remain_literal() {
    let f = Fixture::new();
    for initialized in [false, true] {
        if initialized {
            f.init();
        }
        let created = ok(cli(
            &f.root,
            &[
                "item",
                "create",
                "--title",
                "--view",
                "--body",
                "--authorize",
                "--label",
                "--view",
                "--model",
                "--authorize",
            ],
        ));
        let id = created["item"]["id"].as_str().unwrap();
        assert_eq!(created["item"]["title"], "--view");
        assert_eq!(created["item"]["body"], "--authorize");
        assert_eq!(created["item"]["labels"], json!(["--view"]));
        assert_eq!(created["item"]["model"], "--authorize");
        let updated = ok(cli(
            &f.root,
            &[
                "item",
                "update",
                id,
                "--title",
                "--authorize",
                "--label",
                "--authorize",
            ],
        ));
        assert_eq!(updated["item"]["title"], "--authorize");
        let closed = ok(cli(&f.root, &["item", "close", id, "--reason", "--view"]));
        assert_eq!(closed["item"]["close_reason"], "--view");
    }
}

#[test]
fn bound_items_remain_authoritative_when_selected_catalog_is_absent() {
    for remove_work in [false, true] {
        let f = Fixture::new();
        let id = f.item();
        let linked = f.linked();
        f.init();
        let acquisition = acquire(&linked, &id);
        let released = mcp(
            &f.root,
            "claim_release",
            json!({"claim_id":acquisition["claim"]["id"],"session":session()}),
        );
        assert!(released.get("error").is_none(), "{released}");
        git(&f.root, &["rm", "-q", &format!(".work/items/{id}.md")]);
        assert!(!f.root.join(".work/items").exists());
        if remove_work {
            fs::remove_dir_all(f.root.join(".work")).unwrap();
        }
        let read = ok(cli(&f.root, &["item", "inspect", &id]));
        assert_eq!(read["item"]["source_worktree"], linked.to_str().unwrap());
        assert_eq!(
            mcp(&f.root, "item_inspect", json!({"id":id}))["item"],
            read["item"]
        );
        for (verb, tool) in [("list", "item_list"), ("ready", "item_ready")] {
            let result = ok(cli(&f.root, &["item", verb]));
            assert_eq!(result["items"].as_array().unwrap().len(), 1, "{result}");
            assert_eq!(mcp(&f.root, tool, json!({}))["items"], result["items"]);
        }
        let updated = mcp(
            &f.root,
            "item_update",
            json!({"id":id,"title":"bound source updated"}),
        );
        assert_eq!(
            updated["item"]["title"], "bound source updated",
            "{updated}"
        );
        let a = acquire(&f.root, &id);
        assert_eq!(a["claim"]["item_id"], id);
        assert!(!f.root.join(".work/items").exists());
    }
}
#[test]
fn selected_catalog_symlink_and_permission_errors_are_not_empty_views() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let id = f.item();
    f.init();
    let items = f.root.join(".work/items");
    let retained = f.root.join("retained-items");
    fs::rename(&items, &retained).unwrap();
    symlink(&retained, &items).unwrap();
    assert_eq!(cli(&f.root, &["item", "list"])["ok"], false);
    assert!(mcp(&f.root, "item_list", json!({})).get("error").is_some());
    fs::remove_file(&items).unwrap();
    symlink(f.root.join("missing"), &items).unwrap();
    assert_eq!(cli(&f.root, &["item", "list"])["ok"], false);
    fs::remove_file(&items).unwrap();
    fs::rename(&retained, &items).unwrap();
    fs::set_permissions(&items, fs::Permissions::from_mode(0o000)).unwrap();
    let denied = cli(&f.root, &["item", "inspect", &id]);
    let mcp_denied = mcp(&f.root, "item_inspect", json!({"id":id}));
    fs::set_permissions(&items, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(denied["error"]["code"], "permission_denied", "{denied}");
    assert_eq!(
        mcp_denied["error"]["code"], "permission_denied",
        "{mcp_denied}"
    );
}

#[test]
fn malformed_bound_sources_remain_diagnostics_independent_of_binding_order() {
    for corrupt_first in [true, false] {
        let f = Fixture::new();
        let mut ids = [f.item(), f.item()];
        ids.sort();
        let linked = f.linked();
        f.init();
        for (index, id) in ids.iter().enumerate() {
            let root = if index == 0 { &linked } else { &f.root };
            let claim = acquire(root, id);
            let release = mcp(
                root,
                "claim_release",
                json!({"claim_id":claim["claim"]["id"],"session":session()}),
            );
            assert!(release.get("error").is_none(), "{release}");
        }
        let corrupt = if corrupt_first { 0 } else { 1 };
        let healthy = 1 - corrupt;
        let root = if corrupt == 0 { &linked } else { &f.root };
        let path = root.join(format!(".work/items/{}.md", ids[corrupt]));
        let malformed = fs::read_to_string(&path)
            .unwrap()
            .replace(&ids[corrupt], &ids[healthy]);
        fs::write(&path, &malformed).unwrap();
        let diagnostics = ok(cli(&f.root, &["item", "diagnose"]));
        assert!(
            diagnostics["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["path"] == path.to_str().unwrap()),
            "{diagnostics}"
        );
        assert_eq!(
            mcp(&f.root, "item_diagnose", json!({}))["diagnostics"],
            diagnostics["diagnostics"]
        );
        assert_eq!(
            cli(&f.root, &["item", "ready"])["error"]["code"],
            "invalid_source"
        );
        let claimed = mcp(
            &f.root,
            "claim_acquire",
            json!({"item":ids[healthy],"actor":"worker","session":session()}),
        );
        assert_eq!(claimed["error"]["code"], "invalid_source", "{claimed}");
        assert_eq!(
            cli(
                &f.root,
                &["item", "update", &ids[healthy], "--title", "refused"]
            )["error"]["code"],
            "ambiguous_id"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), malformed);
    }
}

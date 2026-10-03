//! Real CLI/MCP finalization with disposable linked checkouts.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use work::core::coordination::{decode_path, new_id};
struct Fixture {
    root: PathBuf,
    root_id: String,
    run: String,
    wisps: Vec<String>,
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
fn cli(root: &Path, args: &[&str]) -> Value {
    let o = Command::new(env!("CARGO_BIN_EXE_work"))
        .args(["--json", "--worktree"])
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|_| panic!("{:?}", o));
    assert_eq!(o.status.success(), v["ok"] == true, "{v}");
    v
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
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("work-finalization-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        git(&root, &["init", "-q", "--initial-branch=main"]);
        let root_id = ok(cli(
            &root,
            &[
                "item",
                "create",
                "--title",
                "root",
                "--body",
                "\r\nRoot opaque body\n",
            ],
        ))["item"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        ok(cli(&root, &["storage", "init"]));
        let run = ok(cli(&root, &["run", "start", &root_id]))["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        fs::create_dir_all(root.join(".work/templates")).unwrap();
        fs::write(root.join(".work/templates/steps.yaml"),"format_version: 2\nname: steps\nitems: [{key: a, title: A, persistence: wisp},{key: b, title: B, persistence: wisp}]\n").unwrap();
        let expansion = ok(cli(&root, &["template", "expand", "steps", "--run", &run]));
        let wisps = expansion["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().into())
            .collect();
        Self {
            root,
            root_id,
            run,
            wisps,
        }
    }
    fn finish(&self) {
        for id in &self.wisps {
            ok(cli(&self.root, &["item", "close", id]));
        }
    }
    fn linked(&self, path: &Path) {
        git(&self.root, &["add", ".work"]);
        git(
            &self.root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "output",
                path.to_str().unwrap(),
            ],
        );
    }
    fn session(&self) -> String {
        ok(cli(
            &self.root,
            &[
                "session",
                "set",
                &self.run,
                "worker",
                "--namespace",
                "test",
                "--session-id",
                "external",
            ],
        ))["session_record"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn squash_keeps_root_exact_routes_output_and_repeats_across_transports() {
    let f = Fixture::new();
    f.finish();
    let session = f.session();
    let bytes = fs::read(f.root.join(format!(".work/items/{}.md", f.root_id))).unwrap();
    assert_eq!(
        ok(cli(&f.root, &["run", "inspect", &f.run]))["finished"],
        true
    );
    for id in &f.wisps {
        assert!(
            f.root
                .join(format!(".git/work/runs/{}/items/{id}.md", f.run))
                .is_file()
        );
    }
    let body = "\r\n# Digest α\n---\nopaque\n\n";
    let result = mcp(
        &f.root,
        "run_squash",
        json!({"run_id":f.run,"summary":body}),
    );
    assert_eq!(result["phase"], "finalized", "{result}");
    assert_eq!(result["changed"], true);
    assert!(
        !f.root
            .join(format!(".git/work/runs/{}/sessions/{session}.yaml", f.run))
            .exists()
    );
    let root = ok(cli(&f.root, &["item", "inspect", &f.root_id]));
    assert_eq!(root["item"]["state"], "open");
    assert_eq!(root["item"]["digests"][0]["body"], body);
    assert_eq!(
        bytes,
        fs::read(f.root.join(format!(".work/items/{}.md", f.root_id))).unwrap()
    );
    assert_eq!(
        ok(cli(&f.root, &["run", "squash", &f.run, "--summary", body]))["changed"],
        false
    );
    assert_eq!(
        mcp(
            &f.root,
            "run_squash",
            json!({"run_id":f.run,"summary":"different"})
        )["error"]["code"],
        "run_conflict"
    );
    let later = ok(cli(&f.root, &["run", "start", &f.root_id]));
    assert_ne!(later["run"]["id"], f.run);
    assert_eq!(
        ok(cli(&f.root, &["item", "list"]))["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn material_members_historical_claims_and_existing_digests_survive_later_discard() {
    let f = Fixture::new();
    f.finish();
    ok(cli(
        &f.root,
        &["run", "squash", &f.run, "--summary", "retained first"],
    ));
    let digest = fs::read(
        f.root
            .join(format!(".work/digests/{}/{}.md", f.root_id, f.run)),
    )
    .unwrap();
    let member = ok(cli(
        &f.root,
        &[
            "item",
            "create",
            "--title",
            "material",
            "--body",
            "material progress",
        ],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let run = ok(cli(&f.root, &["run", "start", &f.root_id]))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(&f.root, &["run", "attach", &run, &member]));
    let claim = ok(cli(
        &f.root,
        &[
            "claim",
            "acquire",
            &member,
            "--actor",
            "material",
            "--session-namespace",
            "test",
            "--session-id",
            "one",
        ],
    ));
    assert_eq!(
        cli(&f.root, &["run", "discard", &run, "--all"])["error"]["code"],
        "claim_conflict"
    );
    ok(cli(
        &f.root,
        &[
            "claim",
            "release",
            claim["claim"]["id"].as_str().unwrap(),
            "--session-namespace",
            "test",
            "--session-id",
            "one",
        ],
    ));
    let material = fs::read(f.root.join(format!(".work/items/{member}.md"))).unwrap();
    ok(cli(&f.root, &["run", "discard", &run, "--all"]));
    assert_eq!(
        material,
        fs::read(f.root.join(format!(".work/items/{member}.md"))).unwrap()
    );
    assert_eq!(
        digest,
        fs::read(
            f.root
                .join(format!(".work/digests/{}/{}.md", f.root_id, f.run))
        )
        .unwrap()
    );
    assert!(
        f.root
            .join(format!(".git/work/workspaces/items/{member}.yaml"))
            .is_file()
    );
    assert!(
        f.root
            .join(format!(
                ".git/work/claims/{}.yaml",
                claim["claim"]["id"].as_str().unwrap()
            ))
            .is_file()
    );
    let abbreviated = format!("w-{}", &f.root_id[..8]);
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &abbreviated]))["item"]["digests"][0]["body"],
        "retained first"
    );
    // A fresh clone has no shared store, but the Git-tracked extension survives.
    git(&f.root, &["add", ".work"]);
    git(
        &f.root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "retained material",
        ],
    );
    let clone = f.root.join("clone");
    let output = Command::new("git")
        .args(["clone", "-q"])
        .arg(&f.root)
        .arg(&clone)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        ok(cli(&clone, &["item", "inspect", &f.root_id]))["item"]["digests"][0]["body"],
        "retained first"
    );
    for operation in ["list", "ready"] {
        let result = ok(cli(&clone, &["item", operation]));
        let root = result["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == f.root_id)
            .unwrap();
        assert_eq!(root["digests"][0]["body"], "retained first");
        let batch = mcp(
            &clone,
            if operation == "list" {
                "item_list"
            } else {
                "item_ready"
            },
            json!({}),
        );
        let root = batch["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == f.root_id)
            .unwrap();
        assert_eq!(root["digests"][0]["body"], "retained first");
    }
}
#[test]
fn pending_squash_freezes_material_member_edits_but_not_the_root() {
    let f = Fixture::new();
    f.finish();
    let member = ok(cli(&f.root, &["item", "create", "--title", "material"]))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(&f.root, &["run", "attach", &f.run, &member]));
    ok(cli(&f.root, &["item", "close", &member]));
    let digest = f
        .root
        .join(format!(".work/digests/{}/{}.md", f.root_id, f.run));
    fs::create_dir_all(digest.parent().unwrap()).unwrap();
    fs::write(
        &digest,
        format!(
            "---\nformat_version: 1\nroot_item_id: {}\nrun_id: {}\n---\noriginal",
            f.root_id, f.run
        ),
    )
    .unwrap();
    assert_eq!(
        cli(
            &f.root,
            &["run", "squash", &f.run, "--summary", "different"]
        )["error"]["code"],
        "run_conflict"
    );
    assert_eq!(
        cli(&f.root, &["item", "reopen", &member])["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        mcp(
            &f.root,
            "item_update",
            json!({"id":member,"title":"blocked"})
        )["error"]["code"],
        "run_not_current"
    );
    // Root lifecycle remains explicitly caller-controlled.
    ok(cli(&f.root, &["item", "close", &f.root_id]));
    ok(cli(&f.root, &["item", "reopen", &f.root_id]));
    assert_eq!(
        ok(cli(
            &f.root,
            &["run", "squash", &f.run, "--summary", "original"]
        ))["phase"],
        "finalized"
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &member]))["item"]["state"],
        "done"
    );
}

#[test]
fn squashing_blocks_outside_completion_changes_creation_templates_repair_and_rebinding() {
    let f = Fixture::new();
    f.finish();
    let aggregate = ok(cli(
        &f.root,
        &[
            "item",
            "create",
            "--title",
            "aggregate",
            "--completion",
            "children",
        ],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let descendant = ok(cli(
        &f.root,
        &[
            "item",
            "create",
            "--title",
            "outside child",
            "--parent",
            &aggregate,
        ],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(&f.root, &["item", "close", &descendant]));
    ok(cli(
        &f.root,
        &["relation", "add", "parent", &f.wisps[0], &aggregate],
    ));
    ok(cli(&f.root, &["run", "attach", &f.run, &aggregate]));
    let linked = f.root.join("linked");
    f.linked(&linked);
    let workspace = ok(cli(
        &f.root,
        &["workspace", "register", linked.to_str().unwrap()],
    ))["workspace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let digest = f
        .root
        .join(format!(".work/digests/{}/{}.md", f.root_id, f.run));
    fs::create_dir_all(digest.parent().unwrap()).unwrap();
    fs::write(
        &digest,
        format!(
            "---\nformat_version: 1\nroot_item_id: {}\nrun_id: {}\n---\noriginal",
            f.root_id, f.run
        ),
    )
    .unwrap();
    assert_eq!(
        cli(
            &f.root,
            &["run", "squash", &f.run, "--summary", "different"]
        )["error"]["code"],
        "run_conflict"
    );
    assert_eq!(
        cli(&f.root, &["item", "reopen", &descendant])["error"]["code"],
        "run_not_current"
    );
    ok(cli(
        &f.root,
        &[
            "item",
            "update",
            &descendant,
            "--title",
            "unrelated title is allowed",
        ],
    ));
    assert_eq!(
        cli(
            &f.root,
            &["relation", "remove", "parent", &descendant, &aggregate]
        )["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        cli(
            &f.root,
            &[
                "item",
                "create",
                "--title",
                "blocked child",
                "--parent",
                &aggregate
            ]
        )["error"]["code"],
        "run_not_current"
    );
    fs::write(f.root.join(".work/templates/blocked.yaml"),"format_version: 2\nname: blocked\nexisting: [aggregate]\nitems: [{key: child, title: Child}]\nedges: [{from: 'local:child', kind: parent, to: 'existing:aggregate'}]\n").unwrap();
    assert_eq!(
        mcp(
            &f.root,
            "template_expand",
            json!({"name":"blocked","existing":{"aggregate":aggregate}})
        )["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        cli(&f.root, &["workspace", "bind", &aggregate, &workspace])["error"]["code"],
        "run_not_current"
    );
    let bytes = fs::read(f.root.join(format!(".work/items/{aggregate}.md"))).unwrap();
    let raw_hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        mcp(
            &f.root,
            "item_repair",
            json!({"id":aggregate,"raw_hex":raw_hex})
        )["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        ok(cli(
            &f.root,
            &["run", "squash", &f.run, "--summary", "original"]
        ))["phase"],
        "finalized"
    );
}

#[test]
fn symmetric_related_links_block_cleanup_regardless_of_which_endpoint_authored_them() {
    for target_owns_relation in [false, true] {
        let f = Fixture::new();
        let (source, target) = if target_owns_relation {
            (&f.wisps[0], &f.root_id)
        } else {
            (&f.root_id, &f.wisps[0])
        };
        ok(cli(
            &f.root,
            &["relation", "add", "related", source, target],
        ));
        assert_eq!(
            cli(&f.root, &["run", "discard", &f.run, "--all"])["error"]["code"],
            "reference_blocked"
        );
        f.finish();
        assert_eq!(
            mcp(
                &f.root,
                "run_squash",
                json!({"run_id":f.run,"summary":"blocked"})
            )["error"]["code"],
            "reference_blocked"
        );
        ok(cli(
            &f.root,
            &["relation", "remove", "related", source, target],
        ));
        ok(cli(&f.root, &["run", "discard", &f.run, "--all"]));
    }
}
#[test]
fn other_run_cleanup_cannot_remove_children_of_a_squashing_member() {
    let f = Fixture::new();
    f.finish();
    let aggregate = ok(cli(
        &f.root,
        &[
            "item",
            "create",
            "--title",
            "aggregate",
            "--completion",
            "children",
        ],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(&f.root, &["run", "attach", &f.run, &aggregate]));
    let root_b = ok(cli(&f.root, &["item", "create", "--title", "other root"]))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let run_b = ok(cli(&f.root, &["run", "start", &root_b]))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let expanded = ok(cli(
        &f.root,
        &["template", "expand", "steps", "--run", &run_b],
    ));
    let children: Vec<_> = expanded["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    for id in &children {
        ok(cli(&f.root, &["item", "close", id]));
    }
    ok(cli(
        &f.root,
        &["relation", "add", "parent", &children[0], &aggregate],
    ));
    let digest = f
        .root
        .join(format!(".work/digests/{}/{}.md", f.root_id, f.run));
    fs::create_dir_all(digest.parent().unwrap()).unwrap();
    fs::write(
        &digest,
        format!(
            "---\nformat_version: 1\nroot_item_id: {}\nrun_id: {}\n---\noriginal",
            f.root_id, f.run
        ),
    )
    .unwrap();
    assert_eq!(
        cli(
            &f.root,
            &["run", "squash", &f.run, "--summary", "different"]
        )["error"]["code"],
        "run_conflict"
    );
    assert_eq!(
        cli(&f.root, &["run", "discard", &run_b, "--all"])["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        mcp(
            &f.root,
            "run_squash",
            json!({"run_id":run_b,"summary":"blocked"})
        )["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        ok(cli(&f.root, &["run", "inspect", &run_b]))["run"]["phase"],
        "active"
    );
    ok(cli(
        &f.root,
        &["run", "squash", &f.run, "--summary", "original"],
    ));
    ok(cli(&f.root, &["run", "discard", &run_b, "--all"]));
}
#[test]
fn unbinding_cannot_expose_unfinished_outside_children_during_squash() {
    let f = Fixture::new();
    f.finish();
    let aggregate = ok(cli(
        &f.root,
        &[
            "item",
            "create",
            "--title",
            "aggregate",
            "--completion",
            "children",
        ],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let child = ok(cli(
        &f.root,
        &[
            "item",
            "create",
            "--title",
            "outside child",
            "--parent",
            &aggregate,
        ],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(&f.root, &["run", "attach", &f.run, &aggregate]));
    let linked = f.root.join("linked");
    f.linked(&linked);
    ok(cli(&linked, &["item", "close", &child]));
    let workspace = ok(cli(
        &f.root,
        &["workspace", "register", linked.to_str().unwrap()],
    ))["workspace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(&f.root, &["workspace", "bind", &child, &workspace]));
    let digest = f
        .root
        .join(format!(".work/digests/{}/{}.md", f.root_id, f.run));
    fs::create_dir_all(digest.parent().unwrap()).unwrap();
    fs::write(
        &digest,
        format!(
            "---\nformat_version: 1\nroot_item_id: {}\nrun_id: {}\n---\noriginal",
            f.root_id, f.run
        ),
    )
    .unwrap();
    assert_eq!(
        cli(
            &f.root,
            &["run", "squash", &f.run, "--summary", "different"]
        )["error"]["code"],
        "run_conflict"
    );
    assert_eq!(
        cli(&f.root, &["workspace", "unbind", &child])["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        mcp(&f.root, "workspace_unbind", json!({"item":child}))["error"]["code"],
        "run_not_current"
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &child]))["item"]["state"],
        "done"
    );
    ok(cli(
        &f.root,
        &["run", "squash", &f.run, "--summary", "original"],
    ));
    ok(cli(&f.root, &["workspace", "unbind", &child]));
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &child]))["item"]["state"],
        "open"
    );
}

#[test]
fn empty_and_unfinished_runs_are_not_squashable_but_disposable() {
    let f = Fixture::new();
    assert_eq!(
        cli(&f.root, &["run", "squash", &f.run, "--summary", ""])["error"]["code"],
        "run_conflict"
    );
    assert_eq!(
        ok(cli(&f.root, &["run", "inspect", &f.run]))["run"]["phase"],
        "active"
    );
    ok(cli(&f.root, &["run", "discard", &f.run, "--all"]));
    let run = ok(cli(&f.root, &["run", "start", &f.root_id]))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        mcp(&f.root, "run_squash", json!({"run_id":run,"summary":""}))["error"]["code"],
        "run_conflict"
    );
    ok(cli(&f.root, &["run", "discard", &run, "--all"]));
}
#[test]
fn selected_then_full_discard_abandons_work_preserves_material_and_context() {
    let f = Fixture::new();
    let session = f.session();
    let original = fs::read(f.root.join(format!(".work/items/{}.md", f.root_id))).unwrap();
    let first = ok(cli(
        &f.root,
        &["run", "discard", &f.run, "--item", &f.wisps[0]],
    ));
    assert_eq!(first["phase"], "active");
    assert_eq!(
        cli(&f.root, &["run", "discard", &f.run, "--item", &f.wisps[0]])["error"]["code"],
        "not_found"
    );
    assert!(
        f.root
            .join(format!(".git/work/runs/{}/sessions/{session}.yaml", f.run))
            .exists()
    );
    assert_eq!(
        cli(&f.root, &["run", "discard", &f.run, "--item", &f.root_id])["error"]["code"],
        "invalid_argument"
    );
    let result = mcp(&f.root, "run_discard", json!({"run_id":f.run,"all":true}));
    assert_eq!(result["phase"], "disposed", "{result}");
    assert_eq!(
        ok(cli(&f.root, &["run", "discard", &f.run, "--all"]))["changed"],
        false
    );
    assert!(!f.root.join(".work/digests").exists());
    assert_eq!(
        original,
        fs::read(f.root.join(format!(".work/items/{}.md", f.root_id))).unwrap()
    );
    assert!(
        f.root
            .join(format!(".git/work/workspaces/items/{}.yaml", f.root_id))
            .is_file()
    );
    assert!(
        ok(cli(&f.root, &["run", "list"]))["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(cli(&f.root, &["run", "list", "--all"]))["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn claims_outside_item_references_and_cross_run_handoffs_block() {
    let f = Fixture::new();
    let claim = ok(cli(
        &f.root,
        &[
            "claim",
            "acquire",
            &f.wisps[0],
            "--actor",
            "test",
            "--session-namespace",
            "test",
            "--session-id",
            "one",
        ],
    ));
    assert_eq!(
        cli(&f.root, &["run", "discard", &f.run, "--all"])["error"]["code"],
        "claim_conflict"
    );
    ok(cli(
        &f.root,
        &[
            "claim",
            "release",
            claim["claim"]["id"].as_str().unwrap(),
            "--session-namespace",
            "test",
            "--session-id",
            "one",
        ],
    ));
    ok(cli(
        &f.root,
        &["relation", "add", "related", &f.root_id, &f.wisps[0]],
    ));
    let blocked = mcp(&f.root, "run_discard", json!({"run_id":f.run,"all":true}));
    assert_eq!(blocked["error"]["code"], "reference_blocked", "{blocked}");
    ok(cli(
        &f.root,
        &["relation", "remove", "related", &f.root_id, &f.wisps[0]],
    ));
    let h = mcp(
        &f.root,
        "handoff_create",
        json!({"from_items":[f.wisps[0]],"to_items":[f.root_id],"body":"needed outside"}),
    );
    assert!(h["handoff"]["id"].is_string(), "{h}");
    assert_eq!(
        cli(&f.root, &["run", "discard", &f.run, "--all"])["error"]["code"],
        "reference_blocked"
    );
    ok(cli(&f.root, &["item", "close", &f.root_id]));
    ok(cli(&f.root, &["run", "discard", &f.run, "--all"]));
    assert!(
        mcp(&f.root, "handoff_list", json!({}))["handoffs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn internal_edges_and_abandoned_internal_handoffs_do_not_block() {
    let f = Fixture::new();
    ok(cli(
        &f.root,
        &["relation", "add", "depends_on", &f.wisps[0], &f.wisps[1]],
    ));
    assert_eq!(
        cli(&f.root, &["run", "discard", &f.run, "--item", &f.wisps[1]])["error"]["code"],
        "reference_blocked"
    );
    let h = mcp(
        &f.root,
        "handoff_create",
        json!({"from_items":[f.wisps[0]],"to_items":[f.wisps[1]],"body":"abandoned but retained"}),
    );
    ok(cli(&f.root, &["run", "discard", &f.run, "--all"]));
    assert_eq!(
        mcp(
            &f.root,
            "handoff_inspect",
            json!({"handoff_id":h["handoff"]["id"]})
        )["handoff"]["body"],
        "abandoned but retained"
    );
    assert_eq!(
        mcp(&f.root, "handoff_prune", json!({}))["retained"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn mixed_handoff_receivers_allow_cleanup_after_internal_edges_become_dangling() {
    let f = Fixture::new();
    let mut ids = f.wisps.clone();
    ids.sort();
    ok(cli(
        &f.root,
        &["relation", "add", "depends_on", &ids[1], &ids[0]],
    ));
    ok(cli(&f.root, &["item", "close", &f.root_id]));
    let h = mcp(
        &f.root,
        "handoff_create",
        json!({"from_items":[ids[0]],"to_items":[ids[1],f.root_id],"body":"unfinished internal, completed outside"}),
    );
    assert!(h["handoff"]["id"].is_string(), "{h}");
    assert_eq!(
        ok(cli(&f.root, &["run", "discard", &f.run, "--all"]))["phase"],
        "disposed"
    );
    assert_eq!(
        mcp(&f.root, "handoff_list", json!({}))["handoffs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn truncated_digest_returns_structured_errors_instead_of_panicking() {
    let f = Fixture::new();
    f.finish();
    let result = ok(cli(
        &f.root,
        &["run", "squash", &f.run, "--summary", "original"],
    ));
    let digest = decode_path(result["digest_path"].as_str().unwrap()).unwrap();
    fs::write(&digest, b"---").unwrap();
    for args in [
        vec!["item", "inspect", f.root_id.as_str()],
        vec!["item", "list"],
        vec!["run", "squash", f.run.as_str(), "--summary", "original"],
    ] {
        assert_eq!(cli(&f.root, &args)["error"]["code"], "invalid_format");
    }
    assert_eq!(
        mcp(&f.root, "item_inspect", json!({"id":f.root_id}))["error"]["code"],
        "invalid_format"
    );
    assert_eq!(
        mcp(
            &f.root,
            "run_squash",
            json!({"run_id":f.run,"summary":"original"})
        )["error"]["code"],
        "invalid_format"
    );
    assert_eq!(fs::read(digest).unwrap(), b"---");
}

fn linked_output(cross_fs: bool) {
    let f = Fixture::new();
    f.finish();
    let linked = if cross_fs {
        PathBuf::from("/dev/shm").join(format!("work-digest-{}", new_id().unwrap()))
    } else {
        f.root.join("linked")
    };
    f.linked(&linked);
    let wid = ok(cli(
        &f.root,
        &["workspace", "register", linked.to_str().unwrap()],
    ))["workspace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Output is captured at start, so use a fresh run after disposing the old one.
    ok(cli(&f.root, &["run", "discard", &f.run, "--all"]));
    let run = ok(cli(
        &f.root,
        &["run", "start", &f.root_id, "--output-workspace", &wid],
    ))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(cli(
        &f.root,
        &["template", "expand", "steps", "--run", &run],
    ));
    let members = ok(cli(&f.root, &["run", "inspect", &run]))["members"]
        .as_array()
        .unwrap()
        .clone();
    for id in members {
        ok(cli(&f.root, &["item", "close", id.as_str().unwrap()]));
    }
    assert_eq!(
        cli(&f.root, &["run", "squash", &run, "--summary", "surviving"])["error"]["code"],
        "run_conflict"
    );
    ok(cli(&f.root, &["workspace", "bind", &f.root_id, &wid]));
    let result = mcp(
        &f.root,
        "run_squash",
        json!({"run_id":run,"summary":"surviving"}),
    );
    assert_eq!(result["phase"], "finalized", "{result}");
    assert!(
        decode_path(result["digest_path"].as_str().unwrap())
            .unwrap()
            .starts_with(&linked)
    );
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &f.root_id]))["item"]["digests"][0]["body"],
        "surviving"
    );
    assert_eq!(
        ok(cli(
            &linked,
            &["run", "squash", &run, "--summary", "surviving"]
        ))["changed"],
        false
    );
    assert!(!f.root.join(format!(".work/digests/{}", f.root_id)).exists());
    if cross_fs {
        fs::remove_dir_all(&linked).unwrap();
    }
}
#[test]
fn linked_surviving_output() {
    linked_output(false);
}
#[test]
fn cross_filesystem_output_when_available() {
    use std::os::unix::fs::MetadataExt;
    if !Path::new("/dev/shm").is_dir()
        || fs::metadata("/dev/shm").unwrap().dev()
            == fs::metadata(std::env::temp_dir()).unwrap().dev()
    {
        eprintln!("cross-filesystem fixture unavailable");
        return;
    }
    linked_output(true);
}
#[test]
fn strict_adapter_inputs_and_summary_stdin() {
    let f = Fixture::new();
    for args in [
        json!({"run_id":f.run,"all":false}),
        json!({"run_id":f.run,"all":true,"items":[f.wisps[0]]}),
        json!({"run_id":f.run,"items":[]}),
        json!({"run_id":f.run}),
        json!({"run_id":f.run,"all":true,"summary":"no"}),
    ] {
        assert_eq!(
            mcp(&f.root, "run_discard", args)["error"]["code"],
            "invalid_argument"
        );
    }
    f.finish();
    let mut c = Command::new(env!("CARGO_BIN_EXE_work"))
        .args(["--json", "--worktree"])
        .arg(&f.root)
        .args(["run", "squash", &f.run, "--summary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(b"\nstdin verbatim\r\n")
        .unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success());
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &f.root_id]))["item"]["digests"][0]["body"],
        "\nstdin verbatim\r\n"
    );
}

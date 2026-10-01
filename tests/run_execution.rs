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
    cli_status(root, args).1
}
fn cli_status(root: &Path, args: &[&str]) -> (i32, Value) {
    let o = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("--json")
        .arg("--worktree")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&o.stdout).unwrap_or_else(|_| {
        panic!(
            "{} {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    });
    (o.status.code().unwrap(), value)
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

/// Pending cleanup is reserved fixture data; no finalization verb is performed.
fn freeze_run(f: &Fixture, run: &str, phase: &str, wisp: &str) {
    let path = f.root.join(format!(".git/work/runs/{run}/run.yaml"));
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["phase"] = json!(phase);
    manifest["cleanup"] = json!({"kind":if phase=="squashing" {"squash"} else {"discard"},"finalize":phase=="squashing","item_ids":[wisp],"session_ids":[],"started_at":"2026-10-01T10:30:00Z"});
    fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
fn operational_files(f: &Fixture) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn collect(path: &Path, files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(&path, files);
            } else {
                files.insert(path.clone(), fs::read(path).unwrap());
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    collect(&f.root.join(".git/work"), &mut files);
    if f.root.join(".work/items").exists() {
        collect(&f.root.join(".work/items"), &mut files);
    }
    files
}

#[test]
fn frozen_existing_wisp_expansion_refuses_before_any_setup_through_cli_and_mcp() {
    for phase in ["squashing", "discarding"] {
        let f = Fixture::new();
        let root = f.item();
        f.init();
        let started = ok(cli(&f.root, &["run", "start", &root]));
        let run = started["run"]["id"].as_str().unwrap();
        template(
            &f,
            "seed",
            "format_version: 2\nname: seed\nitems: [{key: a, title: Seed, persistence: wisp}]\n",
        );
        let expanded = ok(cli(&f.root, &["template", "expand", "seed", "--run", run]));
        let wisp = expanded["items"][0]["id"].as_str().unwrap();
        freeze_run(&f, run, phase, wisp);
        template(
            &f,
            "extend",
            "format_version: 2\nname: extend\nexisting: [seed]\nitems: [{key: a, title: NewMaterial}]\nedges: [{from: 'existing:seed', kind: depends_on, to: 'local:a'}]\n",
        );
        let before = operational_files(&f);
        let binding = format!("seed={wisp}");
        // There is deliberately no target run: the existing source's owner is
        // the frozen run. Rejecting only --run would miss this preflight case.
        let failed = cli(
            &f.root,
            &["template", "expand", "extend", "--existing", &binding],
        );
        assert_eq!(failed["error"]["code"], "run_not_current", "{failed}");
        assert_eq!(operational_files(&f), before);
        let failed = mcp(
            &f.root,
            "template_expand",
            json!({"name":"extend","existing":{"seed":wisp}}),
        );
        assert_eq!(failed["error"]["code"], "run_not_current", "{failed}");
        assert_eq!(operational_files(&f), before);
        assert_eq!(
            ok(cli(&f.root, &["item", "inspect", wisp]))["item"]["body"],
            ""
        );
    }
}

#[test]
fn frozen_run_root_material_and_wisps_match_ready_vs_acquire_in_cli_and_mcp() {
    for phase in ["squashing", "discarding"] {
        let f = Fixture::new();
        let root = f.item();
        let member = f.item();
        let outsider = f.item();
        f.init();
        let started = ok(cli(&f.root, &["run", "start", &root]));
        let run = started["run"]["id"].as_str().unwrap();
        ok(cli(&f.root, &["run", "attach", run, &member]));
        template(
            &f,
            "seed",
            "format_version: 2\nname: seed\nitems: [{key: a, title: Seed, persistence: wisp}]\n",
        );
        let expanded = ok(cli(&f.root, &["template", "expand", "seed", "--run", run]));
        let wisp = expanded["items"][0]["id"].as_str().unwrap();
        // Retain one current owner when cleanup is frozen; inspection must still
        // report the acquisition rather than silently hiding ownership.
        let owned = acquire(&f.root, wisp);
        freeze_run(&f, run, phase, wisp);
        let ready = ok(cli(&f.root, &["item", "ready"]));
        let mcp_ready = mcp(&f.root, "item_ready", json!({}));
        assert_eq!(ready, mcp_ready);
        let ids: Vec<_> = ready["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![outsider.as_str()]);
        let listed = ok(cli(&f.root, &["item", "list"]));
        assert_eq!(listed, mcp(&f.root, "item_list", json!({})));
        for id in [&root, &member, wisp] {
            let inspected = ok(cli(&f.root, &["item", "inspect", id]));
            assert_eq!(inspected["item"]["executable"], false, "{inspected}");
            assert_eq!(inspected, mcp(&f.root, "item_inspect", json!({"id":id})));
            assert_eq!(
                listed["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item["id"] == id)
                    .unwrap()["executable"],
                false
            );
            let failed = cli(
                &f.root,
                &[
                    "claim",
                    "acquire",
                    id,
                    "--actor",
                    "worker",
                    "--session-namespace",
                    "codex",
                    "--session-id",
                    "another session",
                ],
            );
            assert_eq!(failed["error"]["code"], "run_not_current", "{failed}");
            let failed = mcp(
                &f.root,
                "claim_acquire",
                json!({"item":id,"actor":"worker","session":session()}),
            );
            assert_eq!(failed["error"]["code"], "run_not_current", "{failed}");
        }
        let inspected = ok(cli(&f.root, &["item", "inspect", wisp]));
        assert_eq!(inspected["item"]["claim"]["id"], owned["claim"]["id"]);
        assert_eq!(inspected["item"]["claim"]["session"], session());
        assert_eq!(inspected["item"]["persistence"], "wisp");
        assert_eq!(inspected["item"]["run_id"], run);
    }
}

const LOOKUP_ROOT: &str = "11111111000040008000000000000000";
const LOOKUP_MEMBER: &str = "22222222000040008000000000000000";
fn lookup_fixture() -> Fixture {
    let f = Fixture::new();
    for id in [
        LOOKUP_ROOT,
        "11111112000040008000000000000000",
        LOOKUP_MEMBER,
        "22222223000040008000000000000000",
    ] {
        fs::write(
            f.root.join(format!(".work/items/{id}.md")),
            format!("---\nformat_version: 1\nid: '{id}'\ntitle: Lookup fixture\ncompletion: manual\nstate: open\npriority: 2\n---\nFixture body\n"),
        ).unwrap();
    }
    f.init();
    f
}
fn lookup_failures(ambiguous: &str) -> Vec<(String, &'static str, i32)> {
    let mut failures = Vec::new();
    for input in [
        "invalid",
        "w-",
        "DEADBEEF",
        "dead/beef",
        "00000000000000000000000000000000",
        "11111111000010008000000000000000",
        "11111111000040007000000000000000",
    ] {
        for input in [input.to_owned(), format!("w-{input}")] {
            failures.push((input, "invalid_argument", 2));
        }
    }
    for input in [ambiguous, "99999999", "99999999000040008000000000000000"] {
        for reference in [input.to_owned(), format!("w-{input}")] {
            failures.push((
                reference,
                if input == ambiguous {
                    "ambiguous_id"
                } else {
                    "not_found"
                },
                3,
            ));
        }
    }
    failures
}
#[test]
fn run_start_lookup_errors_and_reference_forms_match_cli_and_mcp_without_invalid_writes() {
    let f = lookup_fixture();
    let before = operational_files(&f);
    for (input, code, exit) in lookup_failures("1111111") {
        let (status, failed) = cli_status(&f.root, &["run", "start", &input]);
        assert_eq!(status, exit, "{input}: {failed}");
        assert_eq!(failed["error"]["code"], code, "{input}: {failed}");
        assert_eq!(operational_files(&f), before, "CLI {input}");
        let failed = mcp(&f.root, "run_start", json!({"root":input}));
        assert_eq!(failed["error"]["code"], code, "{input}: {failed}");
        assert_eq!(operational_files(&f), before, "MCP {input}");
    }
    let mut run = None;
    for reference in [
        LOOKUP_ROOT.to_owned(),
        format!("w-{LOOKUP_ROOT}"),
        "11111111".into(),
        "w-11111111".into(),
    ] {
        let (status, result) = cli_status(&f.root, &["run", "start", &reference]);
        assert_eq!(status, 0, "{result}");
        let result = ok(result);
        assert_eq!(result["run"]["root_item_id"], LOOKUP_ROOT);
        let current = result["run"]["id"].as_str().unwrap().to_owned();
        if let Some(run) = &run {
            assert_eq!(run, &current);
        }
        run = Some(current.clone());
        let result = mcp(&f.root, "run_start", json!({"root":reference}));
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["run"]["root_item_id"], LOOKUP_ROOT);
        assert_eq!(result["run"]["id"], current);
    }
}
#[test]
fn membership_validates_all_references_before_setup_with_cli_mcp_lookup_parity() {
    let f = lookup_fixture();
    let result = ok(cli(&f.root, &["run", "start", LOOKUP_ROOT]));
    let run = result["run"]["id"].as_str().unwrap();
    let before = operational_files(&f);
    for verb in ["attach", "detach"] {
        for (input, code, exit) in lookup_failures("2222222") {
            // Resolve a valid unbound member first: a later bad argument must
            // still refuse before registering any member binding.
            let (status, failed) = cli_status(&f.root, &["run", verb, run, LOOKUP_MEMBER, &input]);
            assert_eq!(status, exit, "{verb} {input}: {failed}");
            assert_eq!(failed["error"]["code"], code, "{verb} {input}: {failed}");
            assert_eq!(operational_files(&f), before, "CLI {verb} {input}");
            let failed = mcp(
                &f.root,
                &format!("run_{verb}"),
                json!({"run_id":run,"items":[LOOKUP_MEMBER,input]}),
            );
            assert_eq!(failed["error"]["code"], code, "{verb} {input}: {failed}");
            assert_eq!(operational_files(&f), before, "MCP {verb} {input}");
        }
    }
    for reference in [
        LOOKUP_MEMBER.to_owned(),
        format!("w-{LOOKUP_MEMBER}"),
        "22222222".into(),
        "w-22222222".into(),
    ] {
        for transport in ["cli", "mcp"] {
            for verb in ["attach", "detach"] {
                let result = if transport == "cli" {
                    let (status, result) = cli_status(&f.root, &["run", verb, run, &reference]);
                    assert_eq!(status, 0, "{result}");
                    ok(result)
                } else {
                    mcp(
                        &f.root,
                        &format!("run_{verb}"),
                        json!({"run_id":run,"items":[reference]}),
                    )
                };
                assert!(result["error"].is_null(), "{result}");
                assert_eq!(result["changed"], true, "{result}");
                assert_eq!(
                    result["run"]["material_items"],
                    if verb == "attach" {
                        json!([LOOKUP_MEMBER])
                    } else {
                        json!([])
                    }
                );
            }
        }
    }
}

#[test]
fn frozen_target_expansion_is_lifecycle_conflict_while_preview_stays_read_only() {
    for phase in ["squashing", "discarding"] {
        let f = Fixture::new();
        let root = f.item();
        f.init();
        let started = ok(cli(&f.root, &["run", "start", &root]));
        let run = started["run"]["id"].as_str().unwrap();
        template(
            &f,
            "seed",
            "format_version: 2\nname: seed\nitems: [{key: a, title: Seed, persistence: wisp}]\n",
        );
        template(
            &f,
            "mixed",
            "format_version: 2\nname: mixed\nitems: [{key: a, title: Material}, {key: b, title: Wisp, persistence: wisp}]\nedges: [{from: 'local:b', kind: depends_on, to: 'local:a'}]\n",
        );
        // Malformed expansion input on an active target keeps its input code.
        let before = operational_files(&f);
        let (exit, failed) = cli_status(
            &f.root,
            &[
                "template", "expand", "mixed", "--run", run, "--root", "invalid",
            ],
        );
        assert_eq!(exit, 2, "{failed}");
        assert_eq!(failed["error"]["code"], "invalid_argument");
        let failed = mcp(
            &f.root,
            "template_expand",
            json!({"name":"mixed","run_id":run,"root":"invalid"}),
        );
        assert_eq!(failed["error"]["code"], "invalid_argument", "{failed}");
        assert_eq!(operational_files(&f), before);

        let seeded = ok(cli(&f.root, &["template", "expand", "seed", "--run", run]));
        let wisp = seeded["items"][0]["id"].as_str().unwrap();
        freeze_run(&f, run, phase, wisp);
        let before = operational_files(&f);
        let manifest = f.root.join(format!(".git/work/runs/{run}/run.yaml"));
        let (exit, failed) = cli_status(&f.root, &["template", "expand", "mixed", "--run", run]);
        assert_eq!(exit, 5, "{failed}");
        let failed_mcp = mcp(
            &f.root,
            "template_expand",
            json!({"name":"mixed","run_id":run}),
        );
        for failed in [&failed, &failed_mcp] {
            assert_eq!(failed["error"]["code"], "run_not_current", "{failed}");
            assert_eq!(failed["error"]["run_id"], run);
            assert_eq!(failed["error"]["phase"], phase);
            assert_eq!(failed["error"]["path"], manifest.to_str().unwrap());
            assert_eq!(failed["error"]["publication"], "not_published");
            // Refusal precedes expansion ID allocation and setup.
            assert!(failed["error"]["key_ids"].is_null());
        }
        assert_eq!(operational_files(&f), before);
        let preview = ok(cli(
            &f.root,
            &["template", "preview", "mixed", "--run", run],
        ));
        assert_eq!(
            preview,
            mcp(
                &f.root,
                "template_preview",
                json!({"name":"mixed","run_id":run})
            )
        );
        assert_eq!(preview["preview"]["items"].as_object().unwrap().len(), 2);
        assert_eq!(operational_files(&f), before);
        assert_eq!(
            ok(cli(&f.root, &["run", "inspect", run]))["run"]["phase"],
            phase
        );

        // An invalid target reference remains an argument failure even when
        // another current run is frozen.
        let (exit, failed) = cli_status(
            &f.root,
            &["template", "expand", "mixed", "--run", "invalid"],
        );
        assert_eq!(exit, 2, "{failed}");
        assert_eq!(failed["error"]["code"], "invalid_argument");
        let failed = mcp(
            &f.root,
            "template_expand",
            json!({"name":"mixed","run_id":"invalid"}),
        );
        assert_eq!(failed["error"]["code"], "invalid_argument", "{failed}");
        assert_eq!(operational_files(&f), before);
    }
}

#[test]
fn fresh_template_only_clone_previews_without_writes_and_expands_materials_in_cli_and_mcp() {
    for transport in ["cli", "mcp"] {
        let origin = Fixture::new();
        template(
            &origin,
            "plan",
            "format_version: 2\nname: plan\nitems: [{key: a, title: Alpha}, {key: b, title: Beta}]\nedges: [{from: 'local:b', kind: depends_on, to: 'local:a'}]\n",
        );
        git(&origin.root, &["add", ".work/templates"]);
        git(
            &origin.root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "template-only",
            ],
        );
        let f = Fixture {
            root: origin.root.join("fresh"),
        };
        git(
            &origin.root,
            &[
                "clone",
                "-q",
                origin.root.to_str().unwrap(),
                f.root.to_str().unwrap(),
            ],
        );
        assert!(!f.root.join(".work/items").exists());
        f.init();
        let before = operational_files(&f);
        let preview = ok(cli(&f.root, &["template", "preview", "plan"]));
        assert_eq!(
            preview,
            mcp(&f.root, "template_preview", json!({"name":"plan"}))
        );
        assert_eq!(operational_files(&f), before);
        assert!(!f.root.join(".work/items").exists());
        assert!(!f.root.join(".work/operations.lock").exists());
        let (exit, failed) = cli_status(
            &f.root,
            &["template", "expand", "plan", "--param", "unknown=value"],
        );
        assert_eq!(exit, 2, "{failed}");
        assert_eq!(failed["error"]["code"], "invalid_argument");
        let failed = mcp(
            &f.root,
            "template_expand",
            json!({"name":"plan","parameters":{"unknown":"value"}}),
        );
        assert_eq!(failed["error"]["code"], "invalid_argument", "{failed}");
        assert_eq!(operational_files(&f), before);
        assert!(!f.root.join(".work/items").exists());
        assert!(!f.root.join(".work/operations.lock").exists());
        let result = if transport == "cli" {
            ok(cli(&f.root, &["template", "expand", "plan"]))
        } else {
            mcp(&f.root, "template_expand", json!({"name":"plan"}))
        };
        assert!(result["error"].is_null(), "{result}");
        assert!(result["run_id"].is_null());
        assert_eq!(result["changed"], true);
        let items = result["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        let a = items[0]["id"].as_str().unwrap();
        let b = items[1]["id"].as_str().unwrap();
        assert_ne!(a, b);
        for item in items {
            let id = item["id"].as_str().unwrap();
            assert!(work::core::coordination::valid_id(id));
            assert_eq!(item["persistence"], "material");
            let path = f.root.join(format!(".work/items/{id}.md"));
            assert_eq!(item["path"], path.to_str().unwrap());
            assert!(path.is_file());
            let binding: Value = serde_json::from_slice(
                &fs::read(f.root.join(format!(".git/work/workspaces/items/{id}.yaml"))).unwrap(),
            )
            .unwrap();
            assert_eq!(binding["item_id"], id);
            let workspace_id = binding["workspace_id"].as_str().unwrap();
            assert!(
                f.root
                    .join(format!(".git/work/workspaces/{workspace_id}.yaml"))
                    .is_file()
            );
            let inspected = ok(cli(&f.root, &["item", "inspect", id]));
            assert_eq!(
                inspected["item"]["source_worktree"],
                f.root.to_str().unwrap()
            );
            assert!(inspected["item"]["run_id"].is_null());
            assert_eq!(inspected, mcp(&f.root, "item_inspect", json!({"id":id})));
        }
        assert_eq!(
            ok(cli(&f.root, &["item", "inspect", b]))["item"]["depends_on"],
            json!([a])
        );
        assert_eq!(
            ok(cli(&f.root, &["item", "ready"]))["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            fs::read_dir(f.root.join(".git/work/runs")).unwrap().count(),
            0
        );
    }
}

#[test]
fn wisp_update_returns_actual_previous_inode_and_noop_has_no_recovery_in_cli_and_mcp() {
    for transport in ["cli", "mcp"] {
        let f = Fixture::new();
        let root = f.item();
        f.init();
        let started = ok(cli(&f.root, &["run", "start", &root]));
        let run = started["run"]["id"].as_str().unwrap();
        template(
            &f,
            "seed",
            "format_version: 2\nname: seed\nitems: [{key: a, title: Original, persistence: wisp}]\n",
        );
        let expanded = ok(cli(&f.root, &["template", "expand", "seed", "--run", run]));
        let id = expanded["items"][0]["id"].as_str().unwrap();
        let path = f.root.join(format!(".git/work/runs/{run}/items/{id}.md"));
        let original = fs::read(&path).unwrap();
        let mut outside_editor = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let update = || {
            if transport == "cli" {
                ok(cli(&f.root, &["item", "update", id, "--title", "Saved"]))
            } else {
                mcp(&f.root, "item_update", json!({"id":id,"title":"Saved"}))
            }
        };
        let result = update();
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["item"]["title"], "Saved");
        let recovery = work::core::coordination::decode_path(
            result["item"]["recovery_path"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(recovery.parent(), path.parent());
        assert!(
            recovery
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".storage-")
        );
        assert_eq!(fs::read(&recovery).unwrap(), original);
        let saved = fs::read(&path).unwrap();
        outside_editor.write_all(b"late editor bytes").unwrap();
        outside_editor.sync_all().unwrap();
        assert_eq!(
            fs::read(&recovery).unwrap(),
            [original, b"late editor bytes".to_vec()].concat()
        );
        assert_eq!(fs::read(&path).unwrap(), saved);
        let before = operational_files(&f);
        let noop = update();
        assert!(noop["error"].is_null(), "{noop}");
        assert!(noop["item"]["recovery_path"].is_null());
        assert_eq!(operational_files(&f), before);
    }
}

#[test]
fn expansion_refuses_missing_bound_source_without_recreating_its_catalog() {
    let f = Fixture::new();
    f.item();
    f.init();
    let linked = f.linked();
    template(
        &f,
        "plan",
        "format_version: 2\nname: plan\nitems: [{key: a, title: Initial}]\n",
    );
    // Linked checkout material publication records its authoritative binding.
    template(
        &f,
        "extend",
        "format_version: 2\nname: extend\nexisting: [seed]\nitems: [{key: a, title: New}]\nedges: [{from: 'existing:seed', kind: depends_on, to: 'local:a'}]\n",
    );
    fs::create_dir_all(linked.join(".work/templates")).unwrap();
    fs::copy(
        f.root.join(".work/templates/plan.yaml"),
        linked.join(".work/templates/plan.yaml"),
    )
    .unwrap();
    let initial = ok(cli(&linked, &["template", "expand", "plan"]));
    let id = initial["items"][0]["id"].as_str().unwrap();
    fs::remove_dir_all(linked.join(".work/items")).unwrap();
    let before = operational_files(&f);
    let binding = format!("seed={id}");
    let failed = cli(
        &f.root,
        &["template", "expand", "extend", "--existing", &binding],
    );
    assert_eq!(failed["error"]["code"], "invalid_source", "{failed}");
    let failed = mcp(
        &f.root,
        "template_expand",
        json!({"name":"extend","existing":{"seed":id}}),
    );
    assert_eq!(failed["error"]["code"], "invalid_source", "{failed}");
    assert!(!linked.join(".work/items").exists());
    assert_eq!(operational_files(&f), before);
}

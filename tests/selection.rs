//! Real CLI/MCP selection and acquisition on disposable repositories only.
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};
use work::core::{coordination::new_id, project::discover};

const ROOT: &str = "10000000000040008000000000000000";
const A: &str = "20000000000040008000000000000000";
const B: &str = "30000000000040008000000000000000";
const C: &str = "40000000000040008000000000000000";
const D: &str = "50000000000040008000000000000000";
const MISSING: &str = "eeeeeeee000040008000000000000000";
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("work-selection-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        git(&root, &["init", "-q"]);
        Self { root }
    }
    fn item(&self, id: &str, priority: u8, extra: &str) {
        fs::write(self.root.join(format!(".work/items/{id}.md")), format!("---\nformat_version: 1\nid: '{id}'\ntitle: Selection fixture\ncompletion: manual\nstate: open\npriority: {priority}\n{extra}---\nOpaque body\n")).unwrap();
    }
    fn init(&self) {
        ok(cli(&self.root, &["storage", "init"]));
    }
    fn shared(&self) -> PathBuf {
        discover(Some(&self.root))
            .unwrap()
            .git_common_dir
            .join("work")
    }
    fn linked(&self) -> PathBuf {
        git(&self.root, &["add", ".work/items"]);
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
        let path = self.root.join("linked");
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-qb",
                "fixture-linked",
                path.to_str().unwrap(),
            ],
        );
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn spawn_cli(root: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_work"))
        .args(["--json", "--worktree"])
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}
fn cli(root: &Path, args: &[&str]) -> Value {
    response(spawn_cli(root, args).wait_with_output().unwrap())
}
fn response(o: Output) -> Value {
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
fn session() -> Value {
    json!({"namespace":"fixture / α","id":"same opaque session"})
}
fn next_args<'a>(filters: &'a [&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "claim",
        "next",
        "--actor",
        "fixture",
        "--session-namespace",
        "fixture / α",
        "--session-id",
        "same opaque session",
    ];
    args.extend_from_slice(filters);
    args
}
fn spawn_mcp(server_root: &Path, name: &str, args: Value) -> Child {
    let mut c = Command::new(env!("CARGO_BIN_EXE_work"))
        .arg("mcp")
        .current_dir(server_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = c.stdin.take().unwrap();
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    drop(input);
    c
}
fn mcp_response(o: Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).lines().last().unwrap()).unwrap();
    let r = &v["result"];
    if r["isError"] == true {
        json!({"ok":false,"error":r["structuredContent"]["error"]})
    } else {
        json!({"ok":true,"result":r["structuredContent"]})
    }
}
fn mcp(root: &Path, name: &str, args: Value) -> Value {
    mcp_response(spawn_mcp(root, name, args).wait_with_output().unwrap())
}
fn next_mcp_args() -> Value {
    json!({"actor":"fixture","session":session()})
}
fn ids(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().into())
        .collect()
}
fn release(root: &Path, result: &Value) {
    ok(mcp(
        root,
        "claim_release",
        json!({"claim_id":result["claim"]["id"],"session":session()}),
    ));
}

#[test]
fn filters_share_full_graph_exact_labels_and_cli_mcp_ordering() {
    let f = Fixture::new();
    f.item(ROOT, 2, "labels: [inherited-only]\n");
    f.item(A, 1, &format!("parent: '{ROOT}'\nlabels: [alpha, beta]\n"));
    f.item(B, 1, &format!("parent: '{ROOT}'\nlabels: [alpha, beta]\n"));
    let grandchild = "60000000000040008000000000000000";
    f.item(grandchild, 2, &format!("parent: '{B}'\n"));
    ok(cli(&f.root, &["item", "close", grandchild]));
    f.item(C, 0, "labels: [alpha, beta]\n");
    f.item(
        D,
        0,
        &format!("parent: '{ROOT}'\nlabels: [alpha, beta]\ndepends_on: ['{C}']\n"),
    );
    f.init();
    let flags = [
        "--root",
        "w-10000000",
        "--label",
        "alpha",
        "--label",
        "beta",
        "--priority-max",
        "1",
        "--persistence",
        "material",
    ];
    let filters = json!({"root":"w-10000000","labels_all":["alpha","beta"],"priority_max":1,"persistence":"material"});
    for verb in ["list", "ready"] {
        let mut args = vec!["item", verb];
        args.extend_from_slice(&flags);
        let actual = ok(cli(&f.root, &args));
        assert_eq!(
            actual,
            ok(mcp(&f.root, &format!("item_{verb}"), filters.clone()))
        );
        assert_eq!(
            ids(&actual),
            if verb == "list" {
                vec![A, B, D]
            } else {
                vec![A, B]
            }
        );
        if verb == "list" {
            let blocked = &actual["items"][2];
            assert_eq!(blocked["blockers"][0]["kind"], "prerequisite");
            assert_eq!(blocked["blockers"][0]["id"], C); // Outside scope still gates.
        }
    }
    let selected = ok(cli(&f.root, &next_args(&flags)));
    assert_eq!(selected["item"]["id"], A);
    let mut request = filters.clone();
    request["actor"] = json!("fixture");
    request["session"] = session();
    let second = ok(mcp(&f.root, "claim_next", request.clone()));
    assert_eq!(second["item"]["id"], B);
    assert_ne!(selected["claim"]["id"], second["claim"]["id"]);
    let claimed = ok(mcp(&f.root, "item_list", filters.clone()));
    assert!(
        claimed["items"][0]["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["kind"] == "claim" && b["id"] == selected["claim"]["id"])
    );
    assert_eq!(
        ok(mcp(&f.root, "claim_next", request)),
        json!({"claim":null,"item":null,"changed":false})
    );
    assert_eq!(
        ok(cli(&f.root, &next_args(&flags))),
        json!({"claim":null,"item":null,"changed":false})
    );
    // Priority overrides ID: C sorts before lower-priority ROOT when unscoped.
    assert_eq!(ok(cli(&f.root, &next_args(&[])))["item"]["id"], C);
    assert!(
        ids(&ok(cli(
            &f.root,
            &["item", "list", "--root", ROOT, "--label", "inherited-only"]
        )))
        .iter()
        .all(|id| id == ROOT)
    );
    assert!(ids(&ok(cli(&f.root, &["item", "ready", "--label", "Alpha"]))).is_empty());
}

#[test]
fn all_priority_levels_then_canonical_ids_and_empty_do_not_create_setup() {
    let f = Fixture::new();
    for (id, p) in [(ROOT, 4), (A, 2), (B, 1), (C, 3), (D, 0)] {
        f.item(id, p, "");
    }
    f.init();
    let before = fs::read_dir(f.shared().join("workspaces")).unwrap().count();
    assert_eq!(
        ok(cli(&f.root, &next_args(&["--label", "absent"]))),
        json!({"claim":null,"item":null,"changed":false})
    );
    assert_eq!(
        fs::read_dir(f.shared().join("workspaces")).unwrap().count(),
        before
    );
    assert_eq!(
        ids(&ok(cli(&f.root, &["item", "ready"]))),
        vec![D, B, A, C, ROOT]
    );
    for id in [D, B, A, C, ROOT] {
        assert_eq!(ok(cli(&f.root, &next_args(&[])))["item"]["id"], id);
    }
    assert_eq!(
        ok(mcp(&f.root, "claim_next", next_mcp_args()))["changed"],
        false
    );
}

#[test]
fn reference_errors_and_checkout_run_rejection_match_transports() {
    let f = Fixture::new();
    f.item(ROOT, 2, "");
    f.init();
    for (flags, filters, code) in [
        (
            vec!["--root", "garbage"],
            json!({"root":"garbage"}),
            "invalid_argument",
        ),
        (
            vec!["--root", MISSING],
            json!({"root":MISSING}),
            "not_found",
        ),
        (
            vec!["--run", "bad"],
            json!({"run_id":"bad"}),
            "invalid_argument",
        ),
        (
            vec!["--run", MISSING],
            json!({"run_id":MISSING}),
            "not_found",
        ),
        (
            vec!["--priority-max", "5"],
            json!({"priority_max":5}),
            "invalid_argument",
        ),
        (
            vec!["--persistence", "disk"],
            json!({"persistence":"disk"}),
            "invalid_argument",
        ),
    ] {
        for verb in ["list", "ready", "next"] {
            let mut args = if verb == "next" {
                next_args(&[])
            } else {
                vec!["item", verb]
            };
            args.extend_from_slice(&flags);
            let mut input = filters.clone();
            if verb == "next" {
                input["actor"] = json!("fixture");
                input["session"] = session();
            }
            let name = if verb == "next" {
                "claim_next".into()
            } else {
                format!("item_{verb}")
            };
            assert_eq!(cli(&f.root, &args)["error"]["code"], code);
            assert_eq!(mcp(&f.root, &name, input)["error"]["code"], code);
        }
    }
    for verb in ["list", "ready"] {
        assert_eq!(
            cli(
                &f.root,
                &["item", verb, "--view", "checkout", "--run", MISSING]
            )["error"]["code"],
            "invalid_argument"
        );
        assert_eq!(
            mcp(
                &f.root,
                &format!("item_{verb}"),
                json!({"view":"checkout","run_id":MISSING})
            )["error"]["code"],
            "invalid_argument"
        );
        assert_eq!(
            ids(&ok(cli(
                &f.root,
                &[
                    "item",
                    verb,
                    "--view",
                    "checkout",
                    "--root",
                    ROOT,
                    "--persistence",
                    "material"
                ]
            ))),
            vec![ROOT]
        );
        assert!(
            ids(&ok(cli(
                &f.root,
                &["item", verb, "--view", "checkout", "--persistence", "wisp"]
            )))
            .is_empty()
        );
    }
    for flags in [
        vec!["--root", ROOT, "--root", ROOT],
        vec!["--priority-max", "1", "--priority-max", "1"],
    ] {
        assert_eq!(
            cli(&f.root, &next_args(&flags))["error"]["code"],
            "invalid_argument"
        );
    }
    for input in [
        json!({"labels_all":[1]}),
        json!({"priority_max":null}),
        json!({"root":null}),
    ] {
        assert_eq!(
            mcp(&f.root, "item_list", input)["error"]["code"],
            "invalid_argument"
        );
    }
    // Use an authored terminal run fixture: no disposal operation is implemented here.
    let run = ok(cli(&f.root, &["run", "start", ROOT]))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let manifest = f.shared().join(format!("runs/{run}/run.yaml"));
    let mut record: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    record["phase"] = json!("disposed");
    record["ended_at"] = json!("2026-10-01T00:00:00Z");
    record["cleanup"] = json!({"kind":"discard","finalize":true,"item_ids":[],"session_ids":[],"started_at":"2026-10-01T00:00:00Z"});
    fs::write(manifest, serde_json::to_vec(&record).unwrap()).unwrap();
    for verb in ["list", "ready", "next"] {
        let mut args = if verb == "next" {
            next_args(&[])
        } else {
            vec!["item", verb]
        };
        args.extend(["--run", &run]);
        let mut input = json!({"run_id":run});
        if verb == "next" {
            input["actor"] = json!("fixture");
            input["session"] = session();
        }
        let name = if verb == "next" {
            "claim_next".into()
        } else {
            format!("item_{verb}")
        };
        let response = cli(&f.root, &args);
        assert_eq!(response["error"]["code"], "run_not_current", "{response}");
        assert_eq!(
            mcp(&f.root, &name, input)["error"]["code"],
            "run_not_current"
        );
    }
}

#[test]
fn unavailable_or_corrupt_ownership_is_never_empty_dispatch() {
    let f = Fixture::new();
    f.item(ROOT, 2, "");
    assert_eq!(
        cli(&f.root, &next_args(&[]))["error"]["code"],
        "storage_missing"
    );
    assert_eq!(
        mcp(&f.root, "claim_next", next_mcp_args())["error"]["code"],
        "storage_missing"
    );
    assert_eq!(
        mcp(&f.root, "item_list", json!({"run_id":MISSING}))["error"]["code"],
        "recovery_required"
    );
    assert_eq!(
        ids(&ok(cli(&f.root, &["item", "ready", "--root", ROOT]))),
        vec![ROOT]
    );
    f.init();
    fs::write(f.shared().join("claims/unexpected"), b"invalid").unwrap();
    for filters in [vec![], vec!["--label", "absent"]] {
        assert_eq!(
            cli(&f.root, &next_args(&filters))["error"]["code"],
            "invalid_format"
        );
    }
    assert_eq!(
        mcp(&f.root, "claim_next", next_mcp_args())["error"]["code"],
        "invalid_format"
    );
    assert_eq!(
        mcp(&f.root, "item_ready", json!({"labels_all":["absent"]}))["error"]["code"],
        "invalid_format"
    );
    let list = ok(mcp(&f.root, "item_list", json!({"root":ROOT})));
    assert_eq!(
        list["items"][0]["ownership_warning"]["code"],
        "invalid_format"
    );
}

#[test]
fn current_run_members_material_wisps_and_root_scope_intersect() {
    let f = Fixture::new();
    f.item(ROOT, 2, "");
    f.init();
    let run = ok(cli(&f.root, &["run", "start", ROOT]))["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::create_dir_all(f.root.join(".work/templates")).unwrap();
    fs::write(f.root.join(".work/templates/mixed.yaml"), "format_version: 2\nname: mixed\nitems: [{key: m, title: Material, priority: 1}, {key: w, title: Wisp, persistence: wisp, priority: 0}]\nedges: [{from: 'local:m', kind: parent, to: 'root'}, {from: 'local:w', kind: parent, to: 'root'}]\n").unwrap();
    let expanded = ok(cli(
        &f.root,
        &["template", "expand", "mixed", "--run", &run],
    ));
    let material = expanded["items"][0]["id"].as_str().unwrap();
    let wisp = expanded["items"][1]["id"].as_str().unwrap();
    let members = ok(cli(&f.root, &["item", "list", "--run", &run]));
    assert_eq!(
        members,
        ok(mcp(&f.root, "item_list", json!({"run_id":run})))
    );
    assert_eq!(
        ids(&members).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([material.to_owned(), wisp.to_owned()])
    );
    assert_eq!(
        ids(&ok(cli(
            &f.root,
            &["item", "ready", "--root", ROOT, "--run", &run]
        ))),
        vec![wisp, material]
    );
    assert_eq!(
        ids(&ok(mcp(
            &f.root,
            "item_list",
            json!({"run_id":run,"persistence":"wisp"})
        ))),
        vec![wisp]
    );
    let claimed = ok(mcp(
        &f.root,
        "claim_next",
        json!({"run_id":run,"root":ROOT,"persistence":"wisp","actor":"fixture","session":session()}),
    ));
    assert_eq!(claimed["item"]["id"], wisp);
    assert_eq!(claimed["claim"]["run_id"], run);
    let second = ok(cli(&f.root, &next_args(&["--run", &run])));
    assert_eq!(second["item"]["id"], material);
    assert_eq!(
        ok(cli(&f.root, &next_args(&["--run", &run])))["changed"],
        false
    );
    release(&f.root, &claimed);
    release(&f.root, &second);
}

#[test]
fn selected_checkout_and_bound_whole_file_control_all_filters_and_selection() {
    let f = Fixture::new();
    f.item(ROOT, 2, "labels: [main]\n");
    f.item(A, 2, "");
    let linked = f.linked();
    let path = linked.join(format!(".work/items/{ROOT}.md"));
    let raw = fs::read_to_string(&path)
        .unwrap()
        .replace("labels: [main]", "labels: [branch]")
        .replace("priority: 2", "priority: 0");
    fs::write(&path, raw).unwrap();
    f.init();
    let args = next_args(&["--root", ROOT, "--label", "branch", "--priority-max", "0"]);
    assert_eq!(ok(cli(&f.root, &args))["changed"], false);
    let claimed = ok(cli(&linked, &args));
    assert_eq!(claimed["item"]["source_worktree"], linked.to_str().unwrap());
    release(&f.root, &claimed);
    // The binding survives release. MCP selects linked source despite server cwd/main selector.
    let listed = ok(mcp(
        &f.root,
        "item_list",
        json!({"root":ROOT,"labels_all":["branch"],"priority_max":0,"worktree":f.root}),
    ));
    assert_eq!(listed["items"][0]["priority"], 0);
    assert_eq!(
        listed["items"][0]["source_worktree"],
        linked.to_str().unwrap()
    );
    assert_eq!(
        ids(&ok(cli(
            &f.root,
            &["item", "list", "--view", "checkout", "--label", "main"]
        ))),
        vec![ROOT]
    );
    let second = ok(mcp(
        &linked,
        "claim_next",
        json!({"worktree":f.root,"root":ROOT,"labels_all":["branch"],"actor":"fixture","session":session()}),
    ));
    assert_eq!(second["item"]["id"], ROOT);
    assert_ne!(second["claim"]["id"], claimed["claim"]["id"]);
    assert_eq!(ok(cli(&linked, &args))["changed"], false);
}

#[test]
fn independent_cli_mcp_linked_worktree_races_never_repeat_coordination_identity() {
    for count in [1, 5] {
        let f = Fixture::new();
        for id in [ROOT, A, B, C, D].into_iter().take(count) {
            f.item(id, 2, "");
        }
        let linked = f.linked();
        f.init();
        // No notification mechanism or persistent process state: every caller reloads files.
        let mut children = Vec::new();
        for index in 0..8 {
            let selected = if index % 2 == 0 { &f.root } else { &linked };
            if index % 3 == 0 {
                children.push((false, spawn_cli(selected, &next_args(&[]))));
            } else {
                let mut args = next_mcp_args();
                args["worktree"] = json!(selected);
                children.push((true, spawn_mcp(&f.root, "claim_next", args)));
            }
        }
        let mut owned = BTreeSet::new();
        let mut acquisitions = BTreeSet::new();
        for (is_mcp, child) in children {
            let o = child.wait_with_output().unwrap();
            let v = if is_mcp { mcp_response(o) } else { response(o) };
            if v["ok"] == false {
                assert_eq!(v["error"]["code"], "storage_busy", "{v}");
                continue;
            }
            let r = ok(v);
            if r["changed"] == true {
                assert!(
                    owned.insert(r["claim"]["item_id"].as_str().unwrap().to_owned()),
                    "duplicate {r}"
                );
                assert!(acquisitions.insert(r["claim"]["id"].as_str().unwrap().to_owned()));
            } else {
                assert_eq!(r, json!({"claim":null,"item":null,"changed":false}));
            }
        }
        // Retry contention with new independent processes until the valid scope is exhausted.
        loop {
            let r = ok(mcp(&linked, "claim_next", next_mcp_args()));
            if r["changed"] == false {
                break;
            }
            assert!(owned.insert(r["claim"]["item_id"].as_str().unwrap().to_owned()));
            assert!(acquisitions.insert(r["claim"]["id"].as_str().unwrap().to_owned()));
        }
        assert_eq!(owned.len(), count);
        assert_eq!(
            ok(cli(&f.root, &["claim", "list", "--current"]))["claims"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
        assert_eq!(
            ok(cli(&f.root, &next_args(&[]))),
            json!({"claim":null,"item":null,"changed":false})
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn source_change_while_waiting_on_checkout_lock_refuses_without_retry_or_claim() {
    use rustix::fs::{FlockOperation, flock};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    for is_mcp in [false, true] {
        let f = Fixture::new();
        f.item(ROOT, 1, "");
        f.item(A, 2, "");
        f.init();
        let path = f.root.join(".work/operations.lock");
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        flock(&lock, FlockOperation::LockExclusive).unwrap();
        let child = if is_mcp {
            spawn_mcp(&f.root, "claim_next", next_mcp_args())
        } else {
            spawn_cli(&f.root, &next_args(&[]))
        };
        // Observe the real child opening its checkout lock after the graph snapshot.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let opened = fs::read_dir(format!("/proc/{}/fd", child.id()))
                .unwrap()
                .flatten()
                .any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target == path));
            if opened {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "child never reached checkout lock"
            );
            thread::sleep(Duration::from_millis(2));
        }
        // While blocked on the checkout lock the shared lock must already exclude others.
        assert_eq!(
            mcp(&f.root, "claim_next", next_mcp_args())["error"]["code"],
            "storage_busy"
        );
        f.item(A, 0, ""); // Outside the selected candidate, but changes selection order.
        flock(&lock, FlockOperation::Unlock).unwrap();
        let o = child.wait_with_output().unwrap();
        let response = if is_mcp { mcp_response(o) } else { response(o) };
        assert_eq!(response["error"]["code"], "conflict", "{response}");
        assert_eq!(response["error"]["publication"], "not_published");
        assert!(
            ok(cli(&f.root, &["claim", "list"]))["claims"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            !f.shared()
                .join(format!("workspaces/items/{ROOT}.yaml"))
                .exists()
        );
        assert_eq!(ok(cli(&f.root, &next_args(&[])))["item"]["id"], A);
    }
}

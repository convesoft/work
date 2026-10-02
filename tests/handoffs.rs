//! Real transports over disposable repositories; no live coordination writes.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use work::core::coordination::new_id;
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("work-handoffs-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        git(&root, &["init", "-q", "--initial-branch=main"]);
        Self { root }
    }
    fn item(&self) -> String {
        ok(cli(
            &self.root,
            &[
                "item",
                "create",
                "--title",
                "task",
                "--body",
                "\r\nopaque source Ω\n---\n",
            ],
        ))["item"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn init(&self) {
        ok(cli(&self.root, &["storage", "init"]));
    }
    fn path(&self, id: &str) -> PathBuf {
        self.root.join(format!(".git/work/handoffs/{id}.md"))
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
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
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
fn command(root: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_work"));
    c.arg("--json").arg("--worktree").arg(root).args(args);
    c
}
fn cli(root: &Path, args: &[&str]) -> Value {
    let o = command(root, args).output().unwrap();
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&o.stderr)))
}
fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}
struct Mcp {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}
impl Mcp {
    fn new(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
            .arg("mcp")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut m = Self {
            child,
            input,
            output,
            next: 1,
        };
        m.rpc("initialize", json!({"protocolVersion":"2025-06-18"}));
        m
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()["result"].clone()
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        let v = self.rpc("tools/call", json!({"name":name,"arguments":args}));
        assert!(v["structuredContent"].is_object(), "{v}");
        v["structuredContent"].clone()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn create(root: &Path, from: &str, to: &str, body: &str) -> Value {
    ok(cli(
        root,
        &[
            "handoff", "create", "--from", from, "--to", to, "--body", body,
        ],
    ))["handoff"]
        .clone()
}
fn claim(root: &Path, item: &str) -> Value {
    ok(cli(
        root,
        &[
            "claim",
            "acquire",
            item,
            "--actor",
            "worker",
            "--session-namespace",
            "opaque",
            "--session-id",
            "session / Ω",
        ],
    ))
}
fn authorization(claim: &Value) -> Value {
    json!({"claim_id":claim["claim"]["id"],"session":{"namespace":"opaque","id":"session / Ω"}})
}
#[test]
fn cli_mcp_restart_deliver_verbatim_many_sources_receivers_and_same_item_context() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    let c = f.item();
    f.init();
    let mut m = Mcp::new(&f.root);
    let body = "\r\n# Not interpreted\n- [ ] caller text Ω\r\n---\nno final newline";
    let h=m.call("handoff_create",json!({"from_items":[b,a],"to_items":[c,a],"body":body,"session":{"namespace":"external","id":"context"}}))["handoff"].clone();
    assert_eq!(h["body"], body);
    let id = h["id"].as_str().unwrap();
    assert!(f.path(id).is_file());
    assert!(fs::read(f.path(id)).unwrap().ends_with(body.as_bytes()));
    let second = create(&f.root, &b, &a, "another source body");
    let item = m.call("item_inspect", json!({"id":a}))["item"].clone();
    assert_eq!(item["body"], "\r\nopaque source Ω\n---\n");
    assert_eq!(item["incoming_handoffs"].as_array().unwrap().len(), 2);
    let claimed = claim(&f.root, &a);
    assert_eq!(
        claimed["item"]["incoming_handoffs"],
        item["incoming_handoffs"]
    );
    ok(cli(
        &f.root,
        &[
            "claim",
            "release",
            claimed["claim"]["id"].as_str().unwrap(),
            "--session-namespace",
            "opaque",
            "--session-id",
            "session / Ω",
        ],
    ));
    drop(m);
    let mut restarted = Mcp::new(&f.root);
    assert_eq!(
        restarted.call("handoff_inspect", json!({"handoff_id":id}))["handoff"],
        h
    );
    assert_eq!(
        ok(cli(&f.root, &["handoff", "list", "--to", &a]))["handoffs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        restarted.call("handoff_list", json!({"to_item":a})),
        ok(cli(&f.root, &["handoff", "list", "--to", &a]))
    );
    assert!(f.path(second["id"].as_str().unwrap()).is_file());
}
#[test]
fn receiver_replacement_preserves_body_and_retention_until_all_cancelled() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    let c = f.item();
    f.init();
    let h = create(&f.root, &a, &b, "\n\r\nopaque\n---\nΩ");
    let id = h["id"].as_str().unwrap();
    let mut m = Mcp::new(&f.root);
    let changed = m.call(
        "handoff_receivers",
        json!({"handoff_id":id,"to_items":[c,b]}),
    );
    assert_eq!(changed["changed"], true);
    assert_eq!(changed["handoff"]["body"], h["body"]);
    assert_eq!(
        ok(cli(&f.root, &["handoff", "receivers", id, "--to", &b, &c]))["changed"],
        false
    );
    ok(cli(&f.root, &["item", "close", &a]));
    assert!(f.path(id).exists());
    assert_eq!(m.call("handoff_prune", json!({}))["retained"][0]["id"], id);
    ok(cli(
        &f.root,
        &["item", "close", &b, "--reason", "cancelled externally"],
    ));
    assert!(f.path(id).exists());
    let last = m.call("item_close", json!({"id":c,"reason":"cancelled"}));
    assert!(last["error"].is_null(), "{last}");
    assert!(!f.path(id).exists());
    ok(cli(&f.root, &["item", "reopen", &b]));
    assert_eq!(m.call("handoff_list", json!({}))["handoffs"], json!([]));
    assert_eq!(
        m.call("handoff_inspect", json!({"handoff_id":id}))["error"]["code"],
        "not_found"
    );
}
#[test]
fn claimed_sources_authorize_all_sources_but_receivers_need_no_pair_and_release_reassign_retain() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    let c = f.item();
    f.init();
    let ca = claim(&f.root, &a);
    let cb = claim(&f.root, &b);
    let _cc = claim(&f.root, &c);
    let mut m = Mcp::new(&f.root);
    let input = json!({"from_items":[a,b],"to_items":[c],"body":"retained","authorization":[authorization(&ca)]});
    assert_eq!(
        m.call("handoff_create", input)["error"]["code"],
        "claim_conflict"
    );
    assert_eq!(m.call("handoff_list", json!({}))["handoffs"], json!([]));
    let h=m.call("handoff_create",json!({"from_items":[a,b],"to_items":[c],"body":"retained","authorization":[authorization(&ca),authorization(&cb)]}))["handoff"].clone();
    let id = h["id"].as_str().unwrap();
    let reassign=m.call("claim_reassign",json!({"claim_id":ca["claim"]["id"],"actor":"controller","session":{"namespace":"new","id":"new"},"reason":"executor stopped","executors_stopped":true}));
    assert!(reassign["error"].is_null(), "{reassign}");
    assert!(f.path(id).exists());
    assert_eq!(m.call("handoff_create",json!({"from_items":[a],"to_items":[c],"body":"stale","authorization":[authorization(&ca)]}))["error"]["code"],"stale_claim");
    let closed = m.call(
        "item_close",
        json!({"id":b,"authorization":[authorization(&cb)]}),
    );
    assert!(closed["error"].is_null(), "{closed}");
    assert!(f.path(id).exists());
    // Receiver metadata updates do not edit or require ownership of any item.
    assert_eq!(
        m.call(
            "handoff_receivers",
            json!({"handoff_id":id,"to_items":[a,c]})
        )["changed"],
        true
    );
}
#[test]
fn close_with_context_cli_and_mcp_save_then_complete_and_do_not_deduplicate() {
    for via_mcp in [false, true] {
        let f = Fixture::new();
        let a = f.item();
        let b = f.item();
        f.init();
        let owner = claim(&f.root, &a);
        let auth = authorization(&owner);
        let auth_text = auth.to_string();
        let input = json!({"from_items":[a],"to_items":[b],"body":"saved before close Ω"});
        let result = if via_mcp {
            Mcp::new(&f.root).call(
                "item_close",
                json!({"id":a,"authorization":[auth],"handoffs":[input]}),
            )
        } else {
            ok(cli(
                &f.root,
                &[
                    "item",
                    "close",
                    &a,
                    "--authorize",
                    &auth_text,
                    "--handoff",
                    &input.to_string(),
                ],
            ))
        };
        assert!(result["error"].is_null(), "{result}");
        let saved = result["item"]["saved_handoffs"][0]["id"].as_str().unwrap();
        assert!(f.path(saved).exists());
        assert_eq!(result["item"]["state"], "done");
        let ending = ok(cli(
            &f.root,
            &["claim", "inspect", owner["claim"]["id"].as_str().unwrap()],
        ));
        assert_eq!(ending["ending"]["outcome"], "completed");
        assert_eq!(
            claim(&f.root, &b)["item"]["incoming_handoffs"][0]["body"],
            "saved before close Ω"
        );
        // Ordinary create has no implicit deduplication or workflow receipt.
        create(&f.root, &a, &b, "saved before close Ω");
        assert_eq!(
            ok(cli(&f.root, &["handoff", "list"]))["handoffs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
#[test]
fn missing_or_invalid_receivers_retain_with_diagnostics_without_stale_checkout_fallback() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    f.init();
    let linked = f.linked();
    let owner = claim(&linked, &b);
    let h = create(&f.root, &a, &b, "bound receiver context");
    let id = h["id"].as_str().unwrap();
    ok(cli(
        &linked,
        &[
            "claim",
            "release",
            owner["claim"]["id"].as_str().unwrap(),
            "--session-namespace",
            "opaque",
            "--session-id",
            "session / Ω",
        ],
    ));
    // Main's copy is done, but the explicitly bound receiver file is absent.
    let path = f.root.join(format!(".work/items/{b}.md"));
    let bytes = fs::read_to_string(&path).unwrap();
    fs::write(path, bytes.replace("state: open", "state: done")).unwrap();
    fs::remove_file(linked.join(format!(".work/items/{b}.md"))).unwrap();
    let result = ok(cli(&f.root, &["handoff", "prune", id]));
    assert_eq!(result["deleted"], json!([]));
    assert!(
        !result["retained"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(f.path(id).exists());
    assert_eq!(
        Mcp::new(&f.root).call("handoff_prune", json!({"ids":[id]})),
        result
    );
}
#[test]
fn concurrent_receiver_changes_and_pruning_serializes_exact_sets_across_linked_processes() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    let c = f.item();
    f.init();
    let linked = f.linked();
    let h = create(&f.root, &a, &b, "concurrent body");
    let id = h["id"].as_str().unwrap();
    let mut children = Vec::new();
    for n in 0..12 {
        let root = if n % 2 == 0 { &f.root } else { &linked };
        let args = if n % 3 == 0 {
            vec!["handoff", "prune", id]
        } else if n % 3 == 1 {
            vec!["handoff", "receivers", id, "--to", &b, &c]
        } else {
            vec!["handoff", "receivers", id, "--to", &c]
        };
        children.push(command(root, &args).stdout(Stdio::piped()).spawn().unwrap());
    }
    let mut successes = 0;
    for child in children {
        let o = child.wait_with_output().unwrap();
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        if v["ok"] == true {
            successes += 1;
        } else {
            assert_eq!(v["error"]["code"], "storage_busy", "{v}");
        }
    }
    assert!(successes > 0);
    let h = ok(cli(&f.root, &["handoff", "inspect", id]))["handoff"].clone();
    assert_eq!(h["body"], "concurrent body");
    let receivers = h["to_items"].as_array().unwrap();
    assert!(receivers == &vec![json!(c)] || receivers == &vec![json!(b)] || receivers.len() == 2);
    assert!(f.path(id).exists());
}
#[test]
fn cross_run_wisps_deliver_and_retain_after_source_run_finishes() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    f.init();
    let mut m = Mcp::new(&f.root);
    let ra = m.call("run_start", json!({"root":a}));
    assert!(ra["error"].is_null(), "{ra}");
    let rb = m.call("run_start", json!({"root":b}));
    assert!(rb["error"].is_null(), "{rb}");
    fs::create_dir_all(f.root.join(".work/templates")).unwrap();
    fs::write(f.root.join(".work/templates/wisp.yaml"),"format_version: 2\nname: wisp\nitems: [{key: task, title: Task, persistence: wisp}]\nedges: []\n").unwrap();
    let source = m.call(
        "template_expand",
        json!({"name":"wisp","run_id":ra["run"]["id"]}),
    )["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let receiver = m.call(
        "template_expand",
        json!({"name":"wisp","run_id":rb["run"]["id"]}),
    )["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let h = create(&f.root, &source, &receiver, "outside source run context");
    let id = h["id"].as_str().unwrap();
    let closed = m.call("item_close", json!({"id":source}));
    assert!(closed["error"].is_null(), "{closed}");
    assert!(f.path(id).exists());
    assert_eq!(
        m.call("run_inspect", json!({"run_id":ra["run"]["id"]}))["finished"],
        true
    );
    assert_eq!(
        claim(&f.root, &receiver)["item"]["incoming_handoffs"][0]["id"],
        id
    );
    assert_eq!(m.call("handoff_prune", json!({}))["retained"][0]["id"], id);
    // No run cleanup is implemented here. The independent record exposes both
    // run-owned endpoints for the finalization successor's reference guard.
    assert_eq!(h["from_items"], json!([source]));
    assert_eq!(h["to_items"], json!([receiver]));
}
#[test]
fn derived_aggregate_receivers_resolve_without_changing_graph_edges() {
    let f = Fixture::new();
    let source = f.item();
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
        &["item", "create", "--title", "child", "--parent", &aggregate],
    ))["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.init();
    let h = create(&f.root, &source, &aggregate, "aggregate context");
    let id = h["id"].as_str().unwrap();
    assert_eq!(
        ok(cli(&f.root, &["handoff", "prune"]))["retained"][0]["id"],
        id
    );
    ok(cli(&f.root, &["item", "close", &child]));
    assert!(!f.path(id).exists());
    assert_eq!(
        ok(cli(&f.root, &["item", "inspect", &aggregate]))["item"]["relations"]["children"],
        json!([child])
    );
}
#[test]
fn retarget_to_done_receiver_and_prune_race_never_deletes_an_unresolved_receiver_set() {
    let f = Fixture::new();
    let source = f.item();
    let done = f.item();
    let open = f.item();
    f.init();
    ok(cli(&f.root, &["item", "close", &done]));
    let h = create(&f.root, &source, &open, "racing retention");
    let id = h["id"].as_str().unwrap();
    let update = command(&f.root, &["handoff", "receivers", id, "--to", &done])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let prune = command(&f.root, &["handoff", "prune", id])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    for c in [update, prune] {
        let o = c.wait_with_output().unwrap();
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(
            v["ok"] == true || v["error"]["code"] == "storage_busy",
            "{v}"
        );
    }
    if f.path(id).exists() {
        let h = ok(cli(&f.root, &["handoff", "inspect", id]))["handoff"].clone();
        if h["to_items"] == json!([open]) {
            assert_eq!(
                ok(cli(&f.root, &["handoff", "prune", id]))["deleted"],
                json!([])
            );
        } else {
            assert_eq!(h["to_items"], json!([done]));
        }
    }
    // A completed retarget is the only way deletion was eligible in this race.
    else {
        assert_eq!(
            ok(cli(&f.root, &["item", "inspect", &open]))["item"]["state"],
            "open"
        );
    }
}
#[test]
fn stdin_exact_body_malformed_envelopes_and_storage_recreation_are_visible() {
    let f = Fixture::new();
    let a = f.item();
    let b = f.item();
    assert!(
        cli(
            &f.root,
            &["handoff", "create", "--from", &a, "--to", &b, "--body", "x"]
        )["error"]
            .is_object()
    );
    f.init();
    let mut child = command(
        &f.root,
        &["handoff", "create", "--from", &a, "--to", &b, "--body", "-"],
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()
    .unwrap();
    let body = "\r\n---\nopaque without final newline Ω";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    let h = ok(serde_json::from_slice(&o.stdout).unwrap())["handoff"].clone();
    let id = h["id"].as_str().unwrap();
    assert_eq!(h["body"], body);
    let original = fs::read(f.path(id)).unwrap();
    let mut m = Mcp::new(&f.root);
    for source in [
        b"---\nformat_version: 2\nfuture: true\n---\nbody".as_slice(),
        b"---\nformat_version: 1\nformat_version: 1\n---\nbody".as_slice(),
    ] {
        fs::write(f.path(id), source).unwrap();
        let result = m.call("handoff_inspect", json!({"handoff_id":id}));
        assert!(
            matches!(
                result["error"]["code"].as_str(),
                Some("invalid_format" | "unsupported_format")
            ),
            "{result}"
        );
        let item = m.call("item_inspect", json!({"id":b}));
        assert!(item["item"]["incoming_handoffs"].is_null());
        assert!(item["item"]["handoff_warning"].is_object());
    }
    fs::write(f.path(id), &original).unwrap();
    let status = ok(cli(&f.root, &["storage", "inspect"]));
    let metadata = &status["storage"]["metadata"];
    let reset=m.call("storage_recreate",json!({"expected_store_id":metadata["store_id"],"expected_generation":metadata["recovery_generation"],"executors_stopped":true,"acknowledge_loss":true}));
    assert!(reset["error"].is_null(), "{reset}");
    assert_eq!(m.call("handoff_list", json!({}))["handoffs"], json!([]));
    let recovery = f.root.join(".git/work/recovery");
    assert!(fs::read_dir(&recovery).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .join(format!("prior/handoffs/{id}.md"))
            .is_file()
    }));
    fs::write(f.path(id), original).unwrap();
    assert_eq!(
        m.call("handoff_inspect", json!({"handoff_id":id}))["error"]["code"],
        "identity_mismatch"
    );
}

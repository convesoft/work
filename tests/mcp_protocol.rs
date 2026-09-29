use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-mcp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "--initial-branch=main"]);
        git(
            &path,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        fs::create_dir_all(path.join(".work/items")).unwrap();
        Self(path)
    }
    fn write(&self, id: &str, extra: &str) {
        fs::write(self.0.join(".work/items").join(format!("{id}.md")),format!("---\nformat_version: 1\nid: \"{id}\"\ntitle: Fixture\nstate: open\n{extra}---\nBody\n")).unwrap();
    }
    fn cli(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_work"))
            .current_dir(&self.0)
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}
impl Client {
    fn new(path: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
            .current_dir(path)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        assert_eq!(client.request("ping", json!({}))["result"], json!({}));
        let initialized = client.request("initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"work-test-client","version":"1"}}));
        assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
        assert!(initialized["result"]["capabilities"]["tools"].is_object());
        client.notify("notifications/initialized", json!({}));
        client
    }
    fn notify(&mut self, method: &str, params: Value) {
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","method":method,"params":params})
        )
        .unwrap();
        self.stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "MCP server closed before response");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], id);
        value
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        let response = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert!(response.get("error").is_none(), "{response}");
        let result = &response["result"];
        assert_eq!(
            serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
            result["structuredContent"]
        );
        result.clone()
    }
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let result = self.call(name, args);
        assert_eq!(result["isError"], false, "{result}");
        result["structuredContent"].clone()
    }
    fn error(&mut self, name: &str, args: Value, code: &str) -> Value {
        let result = self.call(name, args);
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(
            result["structuredContent"]["error"]["code"], code,
            "{result}"
        );
        result["structuredContent"]["error"].clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        drop(self.stdin.flush());
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

#[test]
fn protocol_client_runs_durable_loop_and_matches_cli_results() {
    let f = Fixture::new();
    let mut client = Client::new(&f.0);
    let tools = client.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(tools.len(), 19);
    for name in [
        "item_create",
        "item_ready",
        "relation_add",
        "item_close",
        "item_repair",
        "storage_inspect",
        "storage_rebuild",
        "storage_backup",
        "storage_migrate",
        "storage_restore",
        "storage_recreate",
    ] {
        assert!(tools.iter().any(|tool| tool["name"] == name));
    }
    assert!(
        tools
            .iter()
            .find(|tool| tool["name"] == "item_ready")
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .contains("does not claim")
    );
    assert!(
        tools
            .iter()
            .find(|tool| tool["name"] == "relation_add")
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .contains("informational")
    );
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0}))["items"],
        json!([])
    );
    let a = client.ok(
        "item_create",
        json!({"title":"Prepare","body":"opaque body\n","worktree":f.0}),
    )["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = client.ok(
        "item_create",
        json!({"title":"Deliver","labels":["release"],"worktree":f.0}),
    )["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let related = client.ok(
        "relation_add",
        json!({"kind":"depends_on","source":b,"target":a,"worktree":f.0}),
    );
    assert_eq!(related["item"]["relations"]["depends_on"], json!([a]));
    assert_eq!(
        client.ok("item_inspect", json!({"id":b,"worktree":f.0})),
        f.cli(&["item", "inspect", &b])["result"]
    );
    assert_eq!(
        client.ok("item_list", json!({"worktree":f.0})),
        f.cli(&["item", "list"])["result"]
    );
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0})),
        f.cli(&["item", "ready"])["result"]
    );
    client.ok(
        "item_update",
        json!({"id":b,"title":"Ship","priority":1,"model":"test","worktree":f.0}),
    );
    assert_eq!(
        client.ok("item_inspect", json!({"id":b,"worktree":f.0}))["item"]["body"],
        ""
    );
    client.ok(
        "item_close",
        json!({"id":a,"reason":"finished","worktree":f.0}),
    );
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0}))["items"][0]["id"],
        b
    );
    client.ok("item_close", json!({"id":b,"worktree":f.0}));
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0}))["items"],
        json!([])
    );
    client.ok("item_reopen", json!({"id":a,"worktree":f.0}));
    client.ok(
        "relation_remove",
        json!({"kind":"depends_on","source":b,"target":a,"worktree":f.0}),
    );
    assert_eq!(
        client.ok("item_inspect_raw", json!({"id":a,"worktree":f.0})),
        f.cli(&["item", "inspect", &a, "--raw"])["result"]
    );
}

#[test]
fn protocol_schema_rejects_null_wrong_types_and_source_errors() {
    let f = Fixture::new();
    let mut client = Client::new(&f.0);
    let before = fs::read_dir(f.0.join(".work/items")).unwrap().count();
    for args in [
        json!({"title":null}),
        json!({"title":42}),
        json!({"title":"Task","priority":"1"}),
        json!({"title":"Task","body":null}),
        json!({"title":"Task","unknown":true}),
        json!({"title":" "}),
        json!({"title":"Task","labels":["bad\nlabel"]}),
    ] {
        client.error("item_create", args, "invalid_argument");
    }
    client.error(
        "item_repair",
        json!({"id":"11111111111141118111111111111111","raw_hex":"aéb"}),
        "invalid_argument",
    );
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
    assert_eq!(
        fs::read_dir(f.0.join(".work/items")).unwrap().count(),
        before
    );
    let schema = client.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "item_create")
        .unwrap()
        .clone();
    assert_eq!(schema["inputSchema"]["required"], json!(["title"]));
    assert_eq!(schema["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        schema["inputSchema"]["properties"]["title"]["type"],
        "string"
    );
    let missing = "11111111111141118111111111111111";
    assert_eq!(
        client.error("item_inspect", json!({"id":missing}), "not_found")["code"],
        f.cli(&["item", "inspect", missing])["error"]["code"]
    );
    let malformed = "22222222222242228222222222222222";
    f.write(malformed, "bad_key: true\n");
    assert_eq!(
        client.error("item_ready", json!({}), "invalid_source")["code"],
        f.cli(&["item", "ready"])["error"]["code"]
    );
    assert!(
        !client.ok("item_diagnose", json!({}))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let raw = client.ok("item_inspect_raw", json!({"id":malformed}));
    assert!(!raw["source"]["raw_hex"].as_str().unwrap().is_empty());
    let fixed = format!(
        "---\nformat_version: 1\nid: \"{malformed}\"\ntitle: Fixed\nstate: open\n---\nBody\n"
    );
    let raw_hex: String = fixed
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    client.ok("item_repair", json!({"id":malformed,"raw_hex":raw_hex}));
    assert_eq!(
        client.ok("item_ready", json!({}))["items"][0]["id"],
        malformed
    );
}

#[test]
fn malformed_rpc_and_noop_update_do_not_silently_succeed() {
    let fixture = Fixture::new();
    let id = "44444444444444448444444444444444";
    fixture.write(id, "");
    let path = fixture.0.join(".work/items").join(format!("{id}.md"));
    let before = fs::read(&path).unwrap();
    let entries = fs::read_dir(fixture.0.join(".work/items")).unwrap().count();
    let mut client = Client::new(&fixture.0);
    client.error(
        "item_update",
        json!({"id":id,"clear_parent":false,"clear_model":false}),
        "invalid_argument",
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read_dir(fixture.0.join(".work/items")).unwrap().count(),
        entries
    );
    writeln!(client.stdin, "{}", json!({"jsonrpc":"2.0"})).unwrap();
    client.stdin.flush().unwrap();
    let mut line = String::new();
    client.stdout.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], Value::Null);
    assert_eq!(response["error"]["code"], -32600);
    for bad_id in [json!({}), json!([]), json!(true)] {
        writeln!(client.stdin,"{}",json!({"jsonrpc":"2.0","id":bad_id,"method":"tools/call","params":{"name":"item_close","arguments":{"id":id}}})).unwrap();
        client.stdin.flush().unwrap();
        let mut line = String::new();
        client.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], -32600);
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read_dir(fixture.0.join(".work/items")).unwrap().count(),
        entries
    );
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
}

#[test]
fn equivalent_cli_and_mcp_mutations_publish_identical_sources() {
    let cli_fixture = Fixture::new();
    let mcp_fixture = Fixture::new();
    let a = "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa";
    let b = "bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb";
    for fixture in [&cli_fixture, &mcp_fixture] {
        fixture.write(a, "");
        fixture.write(b, "");
    }
    let mut client = Client::new(&mcp_fixture.0);
    let cli_added = cli_fixture.cli(&["relation", "add", "depends_on", b, a]);
    let mcp_added = client.ok(
        "relation_add",
        json!({"kind":"depends_on","source":b,"target":a}),
    );
    assert_eq!(
        cli_added["result"]["item"]["relations"],
        mcp_added["item"]["relations"]
    );
    for id in [a, b] {
        assert_eq!(
            fs::read(cli_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap(),
            fs::read(mcp_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap()
        );
    }
    cli_fixture.cli(&["item", "close", a, "--reason", "finished"]);
    client.ok("item_close", json!({"id":a,"reason":"finished"}));
    assert_eq!(
        cli_fixture.cli(&["item", "ready"])["result"]["items"][0]["id"],
        client.ok("item_ready", json!({}))["items"][0]["id"]
    );
    cli_fixture.cli(&["item", "reopen", a]);
    client.ok("item_reopen", json!({"id":a}));
    cli_fixture.cli(&["relation", "remove", "depends_on", b, a]);
    client.ok(
        "relation_remove",
        json!({"kind":"depends_on","source":b,"target":a}),
    );
    for id in [a, b] {
        assert_eq!(
            fs::read(cli_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap(),
            fs::read(mcp_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap()
        );
    }
    assert_eq!(
        cli_fixture.cli(&["item", "list"])["result"]["items"]
            .as_array()
            .unwrap()
            .len(),
        client.ok("item_list", json!({}))["items"]
            .as_array()
            .unwrap()
            .len()
    );
}

#[test]
fn protocol_client_reads_isolated_bootstrap_backlog() {
    let fixture = Fixture::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(".work/items");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        fs::copy(
            entry.path(),
            fixture.0.join(".work/items").join(entry.file_name()),
        )
        .unwrap();
    }
    let mcp_id = "04368710988a405d8e96f85d3bae6b01";
    let path = fixture.0.join(".work/items").join(format!("{mcp_id}.md"));
    let source = fs::read_to_string(&path).unwrap();
    fs::write(&path, source.replacen("state: done", "state: open", 1)).unwrap();
    let mut client = Client::new(&fixture.0);
    let ready = client.ok("item_ready", json!({}));
    assert_eq!(ready, fixture.cli(&["item", "ready"])["result"]);
    assert!(
        ready["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == mcp_id)
    );
    assert_eq!(
        client.ok("item_inspect", json!({"id":mcp_id}))["item"]["body"],
        fixture.cli(&["item", "inspect", mcp_id])["result"]["item"]["body"]
    );
}

#[test]
fn explicit_worktree_selects_other_durable_view() {
    let control = Fixture::new();
    let selected = std::env::temp_dir().join(format!(
        "work-mcp-linked-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    git(
        &control.0,
        &[
            "worktree",
            "add",
            "-b",
            "feature",
            selected.to_str().unwrap(),
        ],
    );
    fs::create_dir_all(selected.join(".work/items")).unwrap();
    let id = "33333333333343338333333333333333";
    fs::write(
        selected.join(".work/items").join(format!("{id}.md")),
        format!("---\nformat_version: 1\nid: \"{id}\"\ntitle: Linked\nstate: open\n---\nBody\n"),
    )
    .unwrap();
    let mut client = Client::new(&control.0);
    assert_eq!(client.ok("item_list", json!({}))["items"], json!([]));
    assert_eq!(
        client.ok("item_list", json!({"worktree":selected}))["items"][0]["id"],
        id
    );
    assert_eq!(
        client.ok("discover", json!({"worktree":selected}))["worktree_root"],
        selected.to_str().unwrap()
    );
    client.error("item_list", json!({"worktree":null}), "invalid_argument");
    client.error(
        "item_list",
        json!({"worktree":"/definitely/not/a/repository"}),
        "io",
    );
    assert_eq!(
        client.ok("item_inspect", json!({"worktree":selected,"id":id}))["item"]["body"],
        "Body\n"
    );
    drop(client);
    git(
        &control.0,
        &["worktree", "remove", "--force", selected.to_str().unwrap()],
    );
}

#[test]
fn storage_recovery_warning_and_rebuild_match_cli_and_mcp() {
    let fixture = Fixture::new();
    let id = "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa";
    fixture.write(id, "");
    let mut client = Client::new(&fixture.0);
    assert_eq!(
        client.ok("storage_inspect", json!({}))["storage"]["status"],
        "uninitialized"
    );
    assert_eq!(
        client.ok("item_ready", json!({})),
        fixture.cli(&["item", "ready"])["result"]
    );
    let inspection = client.ok("storage_inspect", json!({}));
    assert_eq!(inspection["storage"]["status"], "ready");
    let database = PathBuf::from(inspection["storage"]["database_path"].as_str().unwrap());
    let rebuilt = client.ok("storage_rebuild", json!({}));
    let cli_rebuilt = fixture.cli(&["storage", "rebuild"])["result"].clone();
    assert_eq!(rebuilt["view_root"], cli_rebuilt["view_root"]);
    assert_eq!(rebuilt["changed_files"], cli_rebuilt["changed_files"]);
    fs::remove_file(&database).unwrap();
    assert_eq!(
        client.ok("storage_inspect", json!({}))["storage"]["status"],
        "missing_database"
    );
    let ready = client.ok("item_ready", json!({}));
    assert_eq!(ready, fixture.cli(&["item", "ready"])["result"]);
    assert_eq!(ready["items"][0]["id"], id);
    assert_eq!(ready["storage_warning"]["code"], "missing_database");
    assert_eq!(
        client.ok("storage_recreate", json!({}))["lost_coordination"],
        true
    );
    assert!(
        client
            .ok("item_ready", json!({}))
            .get("storage_warning")
            .is_none()
    );
}

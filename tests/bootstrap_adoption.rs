//! Adoption checks against disposable Git copies of the authored bootstrap backlog.
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

const STEPS: [&str; 7] = [
    "87b8795f934049c2acebaac42e81664d",
    "0fac69ec66004e088ce2b22cc0119bd2",
    "13ca14c6e5c14b24a5dea551c343bbeb",
    "cec96d7e174d47c1ab3117968b0bba0f",
    "393f86fafaf54449944d27da1af0a24b",
    "04368710988a405d8e96f85d3bae6b01",
    "e1121992e0574b4b9b163b85e17ec601",
];
const AGGREGATE: &str = "933a6d82674c4a23ab45e810b8319bd2";
const RELEASE: &str = "74c22e40cbc249229f86e355a663a942";
static NEXT: AtomicU64 = AtomicU64::new(0);

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "work-adoption-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "--initial-branch=main"]);
        fs::create_dir_all(root.join(".work/items")).unwrap();
        for entry in
            fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".work/items")).unwrap()
        {
            let entry = entry.unwrap();
            fs::copy(
                entry.path(),
                root.join(".work/items").join(entry.file_name()),
            )
            .unwrap();
        }
        // Model the handoff to item 7 independently of live backlog progress.
        for (index, id) in STEPS.iter().enumerate() {
            let path = root.join(".work/items").join(format!("{id}.md"));
            let source = fs::read_to_string(&path).unwrap();
            let state = if index == STEPS.len() - 1 {
                "open"
            } else {
                "done"
            };
            fs::write(path, with_fixture_state(&source, state)).unwrap();
        }
        git(&root, &["add", ".work/items"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );
        Self(root)
    }
    fn file(&self, id: &str) -> PathBuf {
        self.0.join(".work/items").join(format!("{id}.md"))
    }
    fn originals(&self) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(self.0.join(".work/items"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }
}

fn with_fixture_state(source: &str, state: &str) -> String {
    let (header, body) = source.split_once("\n---\n").unwrap();
    assert!(header.lines().any(|line| line.starts_with("state: ")));
    let header = header
        .lines()
        .filter(|line| state != "open" || !line.starts_with("close_reason:"))
        .map(|line| {
            if line.starts_with("state: ") {
                format!("state: {state}")
            } else {
                (*line).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header}\n---\n{body}")
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn cli(root: &Path, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_work"))
        .current_dir(root)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.iter().filter(|&&b| b == b'\n').count(), 1);
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}
struct Mcp {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: u64,
}
impl Mcp {
    fn new(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
            .current_dir(root)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut this = Self {
            child,
            input,
            output,
            id: 0,
        };
        this.request("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"adoption-test","version":"1"}}));
        this
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "MCP server exited");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], self.id);
        value
    }
    fn call(&mut self, name: &str, args: Value) -> (bool, Value) {
        let response = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert!(response.get("error").is_none(), "{response}");
        let result = &response["result"];
        assert_eq!(
            serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
            result["structuredContent"]
        );
        (
            result["isError"] == false,
            result["structuredContent"].clone(),
        )
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

enum Surface {
    Cli,
    Mcp(Mcp),
}
impl Surface {
    fn new(root: &Path, mcp: bool) -> Self {
        if mcp {
            Self::Mcp(Mcp::new(root))
        } else {
            Self::Cli
        }
    }
    fn call(&mut self, root: &Path, cli_args: &[&str], name: &str, args: Value) -> (bool, Value) {
        match self {
            Self::Cli => {
                let (status, v) = cli(root, cli_args);
                (
                    status == 0,
                    if status == 0 {
                        v["result"].clone()
                    } else {
                        v["error"].clone()
                    },
                )
            }
            Self::Mcp(mcp) => {
                let (ok, value) = mcp.call(name, args);
                (ok, if ok { value } else { value["error"].clone() })
            }
        }
    }
    fn ok(&mut self, root: &Path, cli_args: &[&str], name: &str, args: Value) -> Value {
        let (ok, value) = self.call(root, cli_args, name, args);
        assert!(ok, "{name}: {value}");
        value
    }
    fn error(
        &mut self,
        root: &Path,
        cli_args: &[&str],
        name: &str,
        args: Value,
        code: &str,
    ) -> Value {
        let (ok, value) = self.call(root, cli_args, name, args);
        assert!(!ok, "{name}: {value}");
        assert_eq!(value["code"], code, "{value}");
        value
    }
    fn inspect(&mut self, root: &Path, id: &str) -> Value {
        self.ok(
            root,
            &["item", "inspect", id],
            "item_inspect",
            json!({"id":id}),
        )["item"]
            .clone()
    }
    fn ready(&mut self, root: &Path) -> Vec<String> {
        self.ok(root, &["item", "ready"], "item_ready", json!({}))["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_owned())
            .collect()
    }
    fn close(&mut self, root: &Path, id: &str) {
        self.ok(root, &["item", "close", id], "item_close", json!({"id":id}));
    }
    fn reopen(&mut self, root: &Path, id: &str) {
        self.ok(
            root,
            &["item", "reopen", id],
            "item_reopen",
            json!({"id":id}),
        );
    }
}
fn body(bytes: &[u8]) -> &[u8] {
    let first = bytes.windows(5).position(|v| v == b"\n---\n").unwrap() + 5;
    &bytes[first..]
}

#[test]
fn copied_handoff_remains_open_after_live_item_completion() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".work/items")
            .join(format!("{}.md", STEPS[6])),
    )
    .unwrap();
    let completed = with_fixture_state(&with_fixture_state(&source, "open"), "done").replacen(
        "state: done\n",
        "state: done\nclose_reason: \"merged\"\n",
        1,
    );
    let open = with_fixture_state(&completed, "open");
    assert!(open.contains("\nstate: open\n"));
    assert!(!open.contains("close_reason:"));
    assert_eq!(body(open.as_bytes()), body(source.as_bytes()));
}

#[test]
fn real_backlog_adopts_and_advances_on_both_surfaces() {
    for via_mcp in [false, true] {
        let fixture = Fixture::new();
        let before = fixture.originals();
        let mut surface = Surface::new(&fixture.0, via_mcp);
        let listed = surface.ok(&fixture.0, &["item", "list"], "item_list", json!({}));
        assert_eq!(listed["items"].as_array().unwrap().len(), before.len());
        assert_eq!(surface.ready(&fixture.0), vec![STEPS[6]]);
        assert_eq!(surface.inspect(&fixture.0, AGGREGATE)["state"], Value::Null);
        let aggregate = surface.inspect(&fixture.0, AGGREGATE);
        let children = aggregate["relations"]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(children.len(), STEPS.len());
        for id in STEPS {
            assert!(children.contains(&id));
        }
        assert_eq!(
            surface.inspect(&fixture.0, RELEASE)["relations"]["depends_on"][0],
            AGGREGATE
        );
        assert_eq!(
            surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
            false
        );
        for (i, id) in STEPS.iter().enumerate() {
            let item = surface.inspect(&fixture.0, id);
            assert_eq!(item["id"], *id);
            assert_eq!(item["parent"], AGGREGATE);
            if i > 0 {
                assert_eq!(item["relations"]["depends_on"][0], STEPS[i - 1]);
            }
            assert_eq!(
                item["body"].as_str().unwrap().as_bytes(),
                body(&before[&format!("{id}.md")])
            );
        }
        for id in &STEPS[..6] {
            surface.reopen(&fixture.0, id);
        }
        for (i, id) in STEPS.iter().enumerate() {
            assert_eq!(surface.ready(&fixture.0), vec![*id]);
            assert_eq!(
                surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
                false
            );
            surface.close(&fixture.0, id);
            if i < 6 {
                assert_eq!(surface.ready(&fixture.0), vec![STEPS[i + 1]]);
            }
        }
        assert_eq!(
            surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
            true
        );
        assert_eq!(surface.ready(&fixture.0), vec![RELEASE]);
        surface.reopen(&fixture.0, STEPS[0]);
        assert_eq!(
            surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
            false
        );
        assert_eq!(surface.inspect(&fixture.0, STEPS[1])["state"], "done");
        assert_eq!(surface.inspect(&fixture.0, STEPS[6])["state"], "done");
        assert!(surface.ready(&fixture.0).contains(&STEPS[0].to_owned()));
        assert!(!surface.ready(&fixture.0).contains(&RELEASE.to_owned()));
        for (name, bytes) in before {
            assert_eq!(
                body(&fs::read(fixture.0.join(".work/items").join(name)).unwrap()),
                body(&bytes)
            );
        }
    }
}

#[test]
fn roundtrip_restart_and_selected_linked_view() {
    let fixture = Fixture::new();
    let before = fixture.originals();
    let mut mcp = Mcp::new(&fixture.0);
    let created = mcp.call(
        "item_create",
        json!({"title":"Adoption round trip","body":"line one\n## acceptance\n- [ ] byte exact\n"}),
    );
    assert!(created.0, "{}", created.1);
    let id = created.1["item"]["id"].as_str().unwrap().to_owned();
    assert!(
        mcp.call(
            "relation_add",
            json!({"kind":"depends_on","source":id,"target":STEPS[6]})
        )
        .0
    );
    assert!(
        mcp.call("item_close", json!({"id":id,"reason":"recorded outcome"}))
            .0
    );
    assert!(mcp.call("item_reopen", json!({"id":id})).0);
    drop(mcp);
    let (status, reopened) = cli(&fixture.0, &["item", "inspect", &id]);
    assert_eq!(status, 0);
    assert_eq!(
        reopened["result"]["item"]["relations"]["depends_on"][0],
        STEPS[6]
    );
    assert_eq!(
        reopened["result"]["item"]["body"],
        "line one\n## acceptance\n- [ ] byte exact\n"
    );
    assert_eq!(reopened["result"]["item"]["state"], "open");
    assert_eq!(reopened["result"]["item"]["close_reason"], Value::Null);
    for (name, bytes) in before {
        assert_eq!(
            fs::read(fixture.0.join(".work/items").join(name)).unwrap(),
            bytes
        );
    }

    let linked = fixture.0.join("linked");
    git(
        &fixture.0,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    // The linked view starts at the committed authored backlog, without the uncommitted new item.
    assert!(!linked.join(".work/items").join(format!("{id}.md")).exists());
    let (status, control) = cli(&fixture.0, &["item", "list"]);
    assert_eq!(status, 0);
    let (status, selected) = cli(
        &fixture.0,
        &["--worktree", linked.to_str().unwrap(), "item", "list"],
    );
    assert_eq!(status, 0);
    assert_eq!(
        control["result"]["items"].as_array().unwrap().len(),
        selected["result"]["items"].as_array().unwrap().len() + 1
    );
    let mut mcp = Mcp::new(&fixture.0);
    assert_eq!(
        mcp.call("item_list", json!({"worktree":linked})).1,
        selected["result"]
    );
    assert_eq!(mcp.call("item_list", json!({})).1, control["result"]);
    assert!(
        mcp.call("item_close", json!({"id":STEPS[6],"worktree":linked}))
            .0
    );
    assert_eq!(
        mcp.call("item_inspect", json!({"id":STEPS[6],"worktree":linked}))
            .1["item"]["state"],
        "done"
    );
    assert_eq!(
        mcp.call("item_inspect", json!({"id":STEPS[6]})).1["item"]["state"],
        "open"
    );
}

#[test]
fn copied_backlog_reports_invalid_header_ambiguous_id_and_graph() {
    for via_mcp in [false, true] {
        let fixture = Fixture::new();
        let mut surface = Surface::new(&fixture.0, via_mcp);
        let original = fs::read(fixture.file(STEPS[6])).unwrap();
        fs::write(fixture.file(STEPS[6]), b"bad header\n").unwrap();
        let failure = surface.error(
            &fixture.0,
            &["item", "ready"],
            "item_ready",
            json!({}),
            "invalid_source",
        );
        assert_eq!(failure["diagnostics"][0]["line"], 1);
        assert!(
            failure["diagnostics"][0]["path"]
                .as_str()
                .unwrap()
                .ends_with(&format!(
                    "{}/.work/items/{}.md",
                    fixture.0.display(),
                    STEPS[6]
                ))
        );
        let diagnose = surface.ok(
            &fixture.0,
            &["item", "diagnose"],
            "item_diagnose",
            json!({}),
        );
        assert!(!diagnose["diagnostics"].as_array().unwrap().is_empty());
        fs::write(fixture.file(STEPS[6]), original).unwrap();
        let a = "abcdef12000040008000000000000001";
        let b = "abcdef12000040008000000000000002";
        for id in [a, b] {
            fs::write(fixture.file(id), format!("---\nformat_version: 1\nid: \"{id}\"\ntitle: Ambiguous\nstate: open\n---\nBody\n")).unwrap();
        }
        surface.error(
            &fixture.0,
            &["item", "inspect", "w-abcdef12"],
            "item_inspect",
            json!({"id":"w-abcdef12"}),
            "ambiguous_id",
        );
        fs::remove_file(fixture.file(a)).unwrap();
        fs::remove_file(fixture.file(b)).unwrap();
        let before_candidate = fs::read(fixture.file(STEPS[0])).unwrap();
        let failure = surface.error(
            &fixture.0,
            &["relation", "add", "depends_on", STEPS[0], STEPS[1]],
            "relation_add",
            json!({"kind":"depends_on","source":STEPS[0],"target":STEPS[1]}),
            "invalid_candidate",
        );
        assert!(!failure["diagnostics"].as_array().unwrap().is_empty());
        assert_eq!(fs::read(fixture.file(STEPS[0])).unwrap(), before_candidate);
        let invalid = fs::read_to_string(fixture.file(STEPS[0]))
            .unwrap()
            .replacen(
                "---\n\n",
                &format!("depends_on:\n  - \"{}\"\n---\n\n", STEPS[1]),
                1,
            );
        fs::write(fixture.file(STEPS[0]), invalid).unwrap();
        let failure = surface.error(
            &fixture.0,
            &["item", "ready"],
            "item_ready",
            json!({}),
            "invalid_source",
        );
        assert!(!failure["diagnostics"].as_array().unwrap().is_empty());
        assert!(
            !surface.ok(
                &fixture.0,
                &["item", "diagnose"],
                "item_diagnose",
                json!({})
            )["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

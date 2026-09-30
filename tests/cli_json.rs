use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-cli-{}-{}",
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
    fn call(&self, args: &[&str]) -> (i32, Value) {
        call(&self.0, args, None)
    }
    fn write(&self, id: &str, extra: &str) {
        fs::write(self.0.join(".work/items").join(format!("{id}.md")),format!("---\nformat_version: 1\nid: \"{id}\"\ntitle: Fixture\nstate: open\n{extra}---\nBody\n")).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn call(cwd: &Path, args: &[&str], stdin: Option<&[u8]>) -> (i32, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_work"));
    command
        .current_dir(cwd)
        .arg("--json")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.stdout.iter().filter(|&&b| b == b'\n').count(), 1);
    (output.status.code().unwrap(), value)
}
fn ok((status, value): (i32, Value)) -> Value {
    assert_eq!(status, 0, "{value}");
    assert_eq!(value["ok"], true);
    value["result"].clone()
}
fn error((status, value): (i32, Value), expected_status: i32, code: &str) -> Value {
    assert_eq!(status, expected_status, "{value}");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], code);
    value["error"].clone()
}

fn human(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_work"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn command_help_explains_operations_without_a_git_checkout() {
    let outside = std::env::temp_dir();
    for (args, detail) in [
        (vec!["item", "--help"], "item COMMAND"),
        (
            vec!["item", "ready", "--help"],
            "eligibility, not ownership",
        ),
        (vec!["item", "create", "--help"], "generated full ID"),
        (
            vec!["relation", "add", "--help"],
            "dependent to prerequisite",
        ),
        (vec!["discover", "--help"], "Git working checkout"),
        (vec!["mcp", "--help"], "MCP tools over stdio"),
    ] {
        let help = ok(call(&outside, &args, None))["help"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(help.contains(detail), "{help}");
    }
}

#[test]
fn durable_loop_and_machine_readable_errors() {
    let f = Fixture::new();
    let help = ok(f.call(&["--help"]));
    assert!(help["help"].as_str().unwrap().contains("item create"));
    assert!(!help["help"].as_str().unwrap().contains("claim"));
    assert!(
        ok(f.call(&["item", "ready"]))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let first = ok(call(
        &f.0,
        &["item", "create", "--title", "Prepare", "--body", "-"],
        Some(b"opaque body\n"),
    ))["item"]
        .clone();
    let a = first["id"].as_str().unwrap().to_owned();
    assert_eq!(first["body"], "opaque body\n");
    assert_eq!(first["display_id"], format!("w-{}", &a[..8]));
    let item_path = f.0.join(".work/items").join(format!("{a}.md"));
    let before = fs::read(&item_path).unwrap();
    let entries_before = fs::read_dir(f.0.join(".work/items")).unwrap().count();
    error(f.call(&["item", "update", &a]), 2, "usage");
    assert_eq!(fs::read(item_path).unwrap(), before);
    assert_eq!(
        fs::read_dir(f.0.join(".work/items")).unwrap().count(),
        entries_before
    );
    let second =
        ok(f.call(&["item", "create", "--title", "Deliver", "--label", "release"]))["item"].clone();
    let b = second["id"].as_str().unwrap().to_owned();
    ok(f.call(&["relation", "add", "depends_on", &b, &a]));
    let waiting = ok(f.call(&["item", "inspect", &format!("w-{}", &b[..8])]))["item"].clone();
    assert_eq!(waiting["executable"], false);
    assert_eq!(waiting["relations"]["depends_on"][0], a);
    let ready = ok(f.call(&["item", "ready"]));
    assert_eq!(ready["items"].as_array().unwrap().len(), 1);
    assert_eq!(ready["items"][0]["id"], a);
    ok(f.call(&[
        "item",
        "update",
        &b,
        "--title",
        "Ship",
        "--priority",
        "1",
        "--model",
        "test",
    ]));
    assert_eq!(
        ok(f.call(&["item", "list"]))["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    ok(f.call(&["item", "close", &a, "--reason", "finished"]));
    assert_eq!(ok(f.call(&["item", "ready"]))["items"][0]["id"], b);
    ok(f.call(&["item", "close", &b]));
    assert!(
        ok(f.call(&["item", "ready"]))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ok(f.call(&["item", "reopen", &a]));
    assert_eq!(
        ok(f.call(&["item", "inspect", &b]))["item"]["state"],
        "done"
    );
    error(
        f.call(&["item", "inspect", "ffffffff000040008000000000000000"]),
        3,
        "not_found",
    );
    for id in [
        "00000000000000000000000000000000",
        "w-00000000000000000000000000000000",
    ] {
        error(f.call(&["item", "inspect", id]), 2, "invalid_argument");
        error(f.call(&["item", "close", id]), 2, "invalid_argument");
    }
    error(
        f.call(&["item", "update", &a, "--priority", "bad"]),
        2,
        "invalid_argument",
    );
    error(f.call(&["claim", "next"]), 2, "usage");
    error(
        call(&std::env::temp_dir(), &["claim", "next"], None),
        2,
        "usage",
    );
    error(
        call(&std::env::temp_dir(), &["item", "create", "--title"], None),
        2,
        "usage",
    );
    error(
        call(
            &std::env::temp_dir(),
            &["item", "create", "--bad", "x"],
            None,
        ),
        2,
        "usage",
    );
    let (status, stdout, stderr) = human(&f.0, &["item", "create", "--title", "--json"]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.starts_with("w-"));
    assert!(!stdout.starts_with('{'));
}

#[test]
fn invalid_commands_fail_before_waiting_for_stdin() {
    let f = Fixture::new();
    for args in [
        vec!["--json", "item", "create", "--body", "-"],
        vec![
            "--json",
            "item",
            "create",
            "--title",
            "Task",
            "--body",
            "-",
            "--priority",
            "5",
        ],
        vec!["--json", "item", "repair", "nope", "--source", "-"],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_work"))
            .current_dir(&f.0)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("invalid command waited for stdin EOF");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            response["error"]["code"] == "usage" || response["error"]["code"] == "invalid_argument"
        );
    }
}

#[test]
fn explicit_worktree_selects_durable_view_without_branch_switch() {
    let f = Fixture::new();
    let linked = f.0.join("linked");
    git(
        &f.0,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    fs::create_dir_all(linked.join(".work/items")).unwrap();
    let branch = git(&f.0, &["branch", "--show-current"]);
    let linked_path = linked.to_str().unwrap();
    let created = ok(f.call(&[
        "--worktree",
        linked_path,
        "item",
        "create",
        "--title",
        "Linked",
    ]))["item"]
        .clone();
    assert!(
        linked
            .join(".work/items")
            .join(format!("{}.md", created["id"].as_str().unwrap()))
            .exists()
    );
    assert!(
        ok(f.call(&["item", "list"]))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(f.call(&["--worktree", linked_path, "item", "list"]))["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(git(&f.0, &["branch", "--show-current"]), branch);
}

#[test]
fn invalid_source_graph_and_ambiguous_prefix_are_distinct() {
    let f = Fixture::new();
    let a = "abcdef12000040008000000000000001";
    let b = "abcdef12000040008000000000000002";
    f.write(a, "");
    f.write(b, "");
    let listed = ok(f.call(&["item", "list"]));
    assert_eq!(listed["items"][0]["display_id"], format!("w-{a}"));
    assert_eq!(listed["items"][1]["display_id"], format!("w-{b}"));
    error(
        f.call(&["item", "inspect", "w-abcdef12"]),
        3,
        "ambiguous_id",
    );
    ok(f.call(&["relation", "add", "depends_on", b, a]));
    error(
        f.call(&["relation", "add", "depends_on", a, b]),
        4,
        "invalid_candidate",
    );
    fs::write(
        f.0.join(".work/items").join(format!("{b}.md")),
        b"bad source",
    )
    .unwrap();
    let diagnostics = ok(f.call(&["item", "diagnose"]));
    assert!(!diagnostics["diagnostics"].as_array().unwrap().is_empty());
    error(f.call(&["item", "inspect", b]), 4, "invalid_source");
    error(
        f.call(&["item", "inspect", &format!("w-{b}")]),
        4,
        "invalid_source",
    );
    let raw = ok(f.call(&["item", "inspect", b, "--raw"]));
    assert_eq!(raw["source"]["raw_hex"], "62616420736f75726365");
    let failure = error(f.call(&["item", "ready"]), 4, "invalid_source");
    assert!(!failure["diagnostics"].as_array().unwrap().is_empty());
    let repaired =
        format!("---\nformat_version: 1\nid: \"{b}\"\ntitle: Repaired\nstate: open\n---\nBody\n");
    let result = ok(call(
        &f.0,
        &["item", "repair", b, "--source", "-"],
        Some(repaired.as_bytes()),
    ));
    assert_eq!(result["source"]["id"], b);
    assert!(
        ok(f.call(&["item", "diagnose"]))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn storage_reads_do_not_initialize_and_warnings_reach_human_output() {
    let f = Fixture::new();
    let id = "00000000000040008000000000000001";
    f.write(id, "completion: manual\n");
    let fresh = ok(f.call(&["storage", "inspect"]));
    assert_eq!(fresh["storage"]["state"], "uninitialized");
    assert_eq!(fresh["storage"]["coordination_available"], false);
    assert!(fresh["storage"]["storage_warning"].is_null());
    for args in [
        vec!["item", "list"],
        vec!["item", "ready"],
        vec!["item", "diagnose"],
        vec!["item", "inspect", id],
        vec!["item", "inspect", id, "--raw"],
    ] {
        let read = ok(f.call(&args));
        assert_eq!(read["storage"]["state"], "uninitialized");
        assert!(read["storage_warning"].is_null());
    }
    assert!(!f.0.join(".git/work").exists());
    assert!(!f.0.join(".git/work.identity.yaml").exists());
    ok(f.call(&["storage", "init"]));
    fs::remove_file(f.0.join(".git/work/store.yaml")).unwrap();
    for args in [
        vec!["item", "list"],
        vec!["item", "ready"],
        vec!["item", "diagnose"],
        vec!["item", "inspect", id],
        vec!["item", "inspect", id, "--raw"],
    ] {
        let read = ok(f.call(&args));
        assert_eq!(read["storage"]["coordination_available"], false);
        assert_eq!(read["storage_warning"]["code"], "storage_missing");
        let (status, _, stderr) = human(&f.0, &args);
        assert_eq!(status, 0);
        assert!(stderr.contains("storage_missing"), "{stderr}");
    }
    assert!(!f.0.join(".git/work/store.yaml").exists());
}

#[test]
fn storage_usage_rejects_invalid_inputs_before_discovery() {
    let outside = std::env::temp_dir();
    for args in [
        vec!["storage", "recreate"],
        vec![
            "storage",
            "recreate",
            "--executors-stopped",
            "--acknowledge-loss",
            "--expected-generation",
            "bad",
        ],
        vec![
            "storage",
            "recreate",
            "--executors-stopped",
            "--executors-stopped",
            "--acknowledge-loss",
        ],
        vec!["storage", "recover", "../escape"],
        vec!["storage", "inspect", "--acknowledge-loss"],
        vec!["storage", "init", "unexpected"],
    ] {
        error(call(&outside, &args, None), 2, "invalid_argument");
    }
    error(call(&outside, &["storage", "restore"], None), 2, "usage");
    let (status, stdout, stderr) = human(&outside, &["storage", "--help"]);
    assert_eq!(status, 0);
    assert!(stdout.contains("executors-stopped"));
    assert!(stderr.is_empty());
}

#[test]
fn storage_contention_is_distinct_and_does_not_hide_file_readiness() {
    use rustix::fs::{FlockOperation, flock};
    let f = Fixture::new();
    f.write("00000000000040008000000000000001", "completion: manual\n");
    ok(f.call(&["storage", "init"]));
    let lock = fs::File::open(f.0.join(".git/work/coordination.lock")).unwrap();
    flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
    let ready = ok(f.call(&["item", "ready"]));
    assert_eq!(ready["items"].as_array().unwrap().len(), 1);
    assert_eq!(ready["storage_warning"]["code"], "storage_busy");
    error(f.call(&["storage", "init"]), 5, "storage_busy");
    drop(lock);
    assert_eq!(
        ok(f.call(&["storage", "inspect"]))["storage"]["coordination_available"],
        true
    );
}

#[test]
fn duplicate_worktree_selectors_refuse_storage_mutations_before_discovery() {
    let first = Fixture::new();
    let second = Fixture::new();
    let absent = first.0.join("does-not-exist");
    for selected in [&first.0, &absent] {
        for command in [
            vec!["storage", "init"],
            vec![
                "storage",
                "recreate",
                "--executors-stopped",
                "--acknowledge-loss",
            ],
            vec!["storage", "recover", "00000000000040008000000000000001"],
        ] {
            let mut args = vec![
                "--worktree",
                selected.to_str().unwrap(),
                "--worktree",
                second.0.to_str().unwrap(),
            ];
            args.extend(command);
            let (status, response) = first.call(&args);
            assert_eq!(status, 2);
            assert_eq!(response["error"]["code"], "invalid_argument");
            assert_eq!(response["error"]["publication"], "not_published");
            assert!(
                response["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("duplicate")
            );
            for checkout in [&first.0, &second.0] {
                assert!(!checkout.join(".git/work").exists());
                assert!(!checkout.join(".git/work.identity.yaml").exists());
            }
            assert!(!absent.exists());
        }
    }
}

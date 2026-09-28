use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use work::core::project::{DiscoveryError, discover};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-discovery-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Git may mark a linked worktree as active. Removing this disposable
        // fixture does not touch the repository running the tests.
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn discovers_regular_and_linked_worktrees_without_switching_checkout() {
    let fixture = Fixture::new();
    let checkout = fixture.0.join("checkout");
    let linked = fixture.0.join("linked");
    fs::create_dir(&checkout).unwrap();
    fixture.git(&checkout, &["init", "--initial-branch=main"]);
    fixture.git(
        &checkout,
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
    fs::create_dir(checkout.join("nested")).unwrap();
    fixture.git(
        &checkout,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );

    let original_head = fixture.git(&checkout, &["rev-parse", "HEAD"]);
    let original_branch = fixture.git(&checkout, &["branch", "--show-current"]);
    let main = discover(Some(&checkout.join("nested"))).unwrap();
    let other = discover(Some(&linked)).unwrap();

    assert_eq!(main.worktree_root, checkout);
    assert_eq!(other.worktree_root, linked);
    assert_eq!(main.git_common_dir, other.git_common_dir);
    assert!(linked.join(".git").is_file());
    assert_eq!(
        fixture.git(&checkout, &["rev-parse", "HEAD"]),
        original_head
    );
    assert_eq!(
        fixture.git(&checkout, &["branch", "--show-current"]),
        original_branch
    );
    assert!(!main.git_common_dir.join("work").exists());
}

#[test]
fn rejects_non_git_and_bare_repositories() {
    let fixture = Fixture::new();
    let plain = fixture.0.join("plain");
    let bare = fixture.0.join("bare.git");
    fs::create_dir(&plain).unwrap();
    assert!(matches!(
        discover(Some(&plain)),
        Err(DiscoveryError::UnsupportedProject(_))
    ));
    fixture.git(&fixture.0, &["init", "--bare", bare.to_str().unwrap()]);
    assert!(matches!(
        discover(Some(&bare)),
        Err(DiscoveryError::UnsupportedProject(_))
    ));
    assert!(!bare.join("work").exists());
}

#[test]
fn preserves_newlines_and_non_utf8_bytes_in_checkout_paths() {
    let fixture = Fixture::new();
    for name in [
        OsString::from("line\nbreak"),
        OsString::from("trailing-line\n"),
        OsString::from_vec(vec![b'n', b'o', b'n', b'-', 0xff]),
    ] {
        let checkout = fixture.0.join(name);
        fs::create_dir(&checkout).unwrap();
        fixture.git(&checkout, &["init", "--initial-branch=main"]);
        let project = discover(Some(&checkout)).unwrap();
        assert_eq!(project.worktree_root, checkout);
        assert_eq!(project.git_common_dir, checkout.join(".git"));
    }
}

#[test]
fn cli_discovery_outputs_distinct_single_line_values_for_unusual_paths() {
    let fixture = Fixture::new();
    let mut outputs = Vec::new();
    for byte in [0xff, 0xfe] {
        let checkout = fixture.0.join(OsString::from_vec(vec![
            b'p', b'a', b't', b'h', byte, b'\n', b'%',
        ]));
        fs::create_dir(&checkout).unwrap();
        fixture.git(&checkout, &["init", "--initial-branch=main"]);
        let output = Command::new(env!("CARGO_BIN_EXE_work"))
            .arg("discover")
            .arg(&checkout)
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = stdout.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("worktree_root="));
        assert!(lines[1].starts_with("git_common_dir="));
        assert!(stdout.contains(&format!("%{byte:02X}%0A%25")));
        outputs.push(stdout);
    }
    assert_ne!(outputs[0], outputs[1]);
}

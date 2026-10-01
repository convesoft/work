//! Claims acceptance uses disposable repositories, including independent
//! test processes sharing one repository's foundation through linked worktrees.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use work::core::claims::{
    Claim, ClaimAuthorization, ClaimCandidate, ClaimEnding, ClaimOutcome, ClaimStore,
};
use work::core::coordination::{CoordinationGuard, SessionIdentity, yaml_bytes};
use work::core::graph::ItemGraph;
use work::core::items::ItemStore;
use work::core::project::{Project, discover};
use work::core::storage::{RecreateRequest, Storage};

const ITEM: &str = "11111111111141118111111111111111";
const OTHER: &str = "22222222222242228222222222222222";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    project: Project,
}
impl Fixture {
    fn new(initialize: bool) -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-claims-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "--initial-branch=main"]);
        fs::create_dir_all(path.join(".work/items")).unwrap();
        for id in [ITEM, OTHER] {
            fs::write(path.join(format!(".work/items/{id}.md")),format!(
                "---\nformat_version: 1\nid: \"{id}\"\ntitle: Claim test\ncompletion: manual\nstate: open\npriority: 2\n---\nOpaque body\n")).unwrap();
        }
        git(&path, &["add", ".work"]);
        git(
            &path,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "init",
            ],
        );
        let project = discover(Some(&path)).unwrap();
        if initialize {
            Storage::new(project.clone()).initialize().unwrap();
        }
        Self { path, project }
    }
    fn guard(&self, exclusive: bool) -> CoordinationGuard {
        CoordinationGuard::acquire(&self.project, exclusive).unwrap()
    }
    fn root(&self) -> PathBuf {
        self.project.git_common_dir.join("work")
    }
    fn candidate(&self, id: &str) -> ClaimCandidate {
        candidate(&self.project, id)
    }
    fn acquire(&self, id: &str) -> Claim {
        ClaimStore::acquire(&self.guard(true), &self.candidate(id), "worker", &session())
            .unwrap()
            .claim
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
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
fn candidate(project: &Project, id: &str) -> ClaimCandidate {
    let items = ItemStore::load(project).unwrap();
    let graph = ItemGraph::from_store(&items);
    assert!(graph.is_valid(), "{:?}", graph.diagnostics());
    ClaimCandidate {
        header: items.resolve(id).unwrap().header.clone().unwrap(),
        evaluation: graph.evaluate(id).unwrap(),
        workspace_id: None,
        run_id: None,
        session_record_id: None,
    }
}
fn session() -> SessionIdentity {
    SessionIdentity {
        namespace: "Provider/opaque:namespace".into(),
        id: "Case Sensitive : /\nSession".into(),
    }
}
fn pair(claim: &Claim) -> ClaimAuthorization {
    ClaimAuthorization {
        claim_id: claim.id.clone(),
        session: claim.session.clone(),
    }
}
fn claim_value() -> Value {
    json!({"format_version":1,"store_id":ITEM,"recovery_generation":OTHER,"id":ITEM,
        "item_id":OTHER,"actor":"worker","session":session().to_json(),
        "acquired_at":"2026-10-01T12:34:56Z"})
}
fn ending_value() -> Value {
    json!({"format_version":1,"store_id":ITEM,"recovery_generation":OTHER,"claim_id":ITEM,
        "item_id":OTHER,"actor":"worker","ended_at":"2026-10-01T12:34:56Z",
        "reason":"","outcome":"released"})
}

#[test]
fn exact_envelopes_reject_invalid_version_keys_types_identity_and_calendar() {
    let value = claim_value();
    let parsed = Claim::parse(&yaml_bytes(&value)).unwrap();
    assert_eq!(parsed.to_json(), value);
    assert_eq!(
        ClaimEnding::parse(&yaml_bytes(&ending_value()))
            .unwrap()
            .to_json(),
        ending_value()
    );
    let mut future = json!({"format_version":2,"future_field":true});
    assert_eq!(
        Claim::parse(&yaml_bytes(&future)).unwrap_err().code,
        "unsupported_format"
    );
    future["format_version"] = json!("1");
    assert_eq!(
        Claim::parse(&yaml_bytes(&future)).unwrap_err().code,
        "invalid_format"
    );
    for (key, replacement) in [
        ("id", json!("w-11111111")),
        ("actor", json!("")),
        ("session", json!({"namespace":"x","id":""})),
        (
            "session",
            json!({"namespace":"x","id":"x","token":"forbidden"}),
        ),
        ("workspace_id", Value::Null),
        ("workspace_id", json!("not-an-id")),
        ("acquired_at", json!("2025-02-29T00:00:00Z")),
        ("acquired_at", json!("2026-10-01T24:00:00Z")),
        ("acquired_at", json!("2026-10-01T00:00:00.Z")),
        ("unknown", json!(true)),
    ] {
        let mut bad = value.clone();
        bad[key] = replacement;
        assert_eq!(
            Claim::parse(&yaml_bytes(&bad)).unwrap_err().code,
            "invalid_format",
            "{key}"
        );
    }
    for timestamp in ["2024-02-29T23:59:59.001Z", "2026-10-01T12:34:56+00:00"] {
        let mut good = value.clone();
        good["acquired_at"] = json!(timestamp);
        Claim::parse(&yaml_bytes(&good)).unwrap();
    }
    for (key, replacement) in [
        ("outcome", json!("expired")),
        ("recovery", json!(false)),
        ("reason", Value::Null),
        ("actor", json!("")),
    ] {
        let mut bad = ending_value();
        bad[key] = replacement;
        assert_eq!(
            ClaimEnding::parse(&yaml_bytes(&bad)).unwrap_err().code,
            "invalid_format"
        );
    }
    let mut bad = ending_value();
    bad["recovery"] = json!(true);
    assert_eq!(
        ClaimEnding::parse(&yaml_bytes(&bad)).unwrap_err().code,
        "invalid_format"
    );
    for raw in [
        b"format_version: 1\nformat_version: 1\n".as_slice(),
        b"---\nformat_version: 1\n---\nformat_version: 1\n",
        b"x: &anchor thing\ny: *anchor\n",
        b"x: !tag thing\n",
    ] {
        assert!(Claim::parse(raw).is_err());
    }
}

#[test]
fn acquisition_is_immutable_release_is_idempotent_and_same_session_returns_with_new_id() {
    let f = Fixture::new(true);
    let first = f.acquire(ITEM);
    let path = f.root().join(format!("claims/{}.yaml", first.id));
    let bytes = fs::read(&path).unwrap();
    let guard = f.guard(true);
    assert_eq!(
        ClaimStore::acquire(&guard, &f.candidate(ITEM), "worker", &session())
            .unwrap_err()
            .code,
        "claim_conflict"
    );
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[]).unwrap_err().code,
        "claim_conflict"
    );
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[pair(&first)])
            .unwrap()
            .unwrap(),
        first
    );
    let mut wrong = session();
    wrong.namespace.make_ascii_lowercase();
    assert_eq!(
        ClaimStore::release(&guard, &first.id, &wrong, "")
            .unwrap_err()
            .code,
        "stale_claim"
    );
    let released = ClaimStore::release(&guard, &first.id, &session(), "opaque reason").unwrap();
    assert!(released.changed);
    assert!(
        !ClaimStore::release(&guard, &first.id, &session(), "ignored retry reason")
            .unwrap()
            .changed
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[pair(&first)])
            .unwrap_err()
            .code,
        "stale_claim"
    );
    let next = ClaimStore::acquire(&guard, &f.candidate(ITEM), "worker", &session())
        .unwrap()
        .claim;
    assert_ne!(first.id, next.id);
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[pair(&first)])
            .unwrap_err()
            .code,
        "stale_claim"
    );
    assert_eq!(
        ClaimStore::list(&guard, Some(ITEM), false).unwrap().len(),
        2
    );
    assert_eq!(ClaimStore::list(&guard, Some(ITEM), true).unwrap().len(), 1);
    drop(guard);
    let reopened = discover(Some(&f.path)).unwrap();
    assert_eq!(
        ClaimStore::current(&CoordinationGuard::acquire(&reopened, false).unwrap(), ITEM).unwrap(),
        Some(next)
    );
}

#[test]
fn completion_hook_ends_current_pair_and_rejects_old_release() {
    let f = Fixture::new(true);
    let claim = f.acquire(ITEM);
    // Simulate the coordinator's persisted close before its ending. Ownership
    // stays current across that interruption and the owner's close retry ends it.
    let path = f.path.join(format!(".work/items/{ITEM}.md"));
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("state: open", "state: done");
    fs::write(&path, source).unwrap();
    let guard = f.guard(true);
    assert!(ClaimStore::current(&guard, ITEM).unwrap().is_some());
    assert_eq!(
        ClaimStore::acquire(&guard, &f.candidate(ITEM), "worker", &session())
            .unwrap_err()
            .code,
        "not_ready"
    );
    let ending = ClaimStore::completed(&guard, &claim.id, &session(), "").unwrap();
    assert_eq!(ending.ending.outcome, ClaimOutcome::Completed);
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[pair(&claim)])
            .unwrap_err()
            .code,
        "stale_claim"
    );
    assert_eq!(
        ClaimStore::release(&guard, &claim.id, &session(), "")
            .unwrap_err()
            .code,
        "stale_claim"
    );
}

#[test]
fn release_and_controller_recovery_do_not_require_item_sources_or_graph() {
    let f = Fixture::new(true);
    let one = f.acquire(ITEM);
    let two = f.acquire(OTHER);
    fs::remove_dir_all(f.path.join(".work/items")).unwrap();
    let guard = f.guard(true);
    ClaimStore::release(&guard, &one.id, &session(), "").unwrap();
    assert_eq!(
        ClaimStore::recover(&guard, &two.id, "controller", "stopped", false)
            .unwrap_err()
            .code,
        "invalid_argument"
    );
    assert_eq!(
        ClaimStore::recover(&guard, &two.id, "controller", "", true)
            .unwrap_err()
            .code,
        "invalid_argument"
    );
    let recovered =
        ClaimStore::recover(&guard, &two.id, "controller", "executor stopped", true).unwrap();
    assert!(recovered.ending.recovery);
    assert_eq!(recovered.ending.actor, "controller");
    assert_eq!(
        ClaimStore::release(&guard, &two.id, &session(), "")
            .unwrap_err()
            .code,
        "stale_claim"
    );
    assert_eq!(
        ClaimStore::authorize(&guard, OTHER, &[pair(&two)])
            .unwrap_err()
            .code,
        "stale_claim"
    );
}

#[test]
fn reassign_validates_candidate_before_ending_and_old_pairs_become_stale() {
    let f = Fixture::new(true);
    let first = f.acquire(ITEM);
    let mut blocked = f.candidate(ITEM);
    blocked
        .evaluation
        .blockers
        .push(work::core::graph::Blocker::Prerequisite {
            id: OTHER.into(),
            inherited_from: None,
        });
    blocked.evaluation.executable = false;
    let guard = f.guard(true);
    assert_eq!(
        ClaimStore::reassign(
            &guard,
            &first.id,
            &blocked,
            "new worker",
            &session(),
            "stopped",
            true
        )
        .unwrap_err()
        .code,
        "not_ready"
    );
    assert!(ClaimStore::inspect(&guard, &first.id).unwrap().current);
    let next = ClaimStore::reassign(
        &guard,
        &first.id,
        &f.candidate(ITEM),
        "new worker",
        &session(),
        "stopped",
        true,
    )
    .unwrap();
    assert_ne!(first.id, next.claim.id);
    assert_eq!(next.previous_claim_id, first.id);
    let historical = ClaimStore::inspect(&guard, &first.id).unwrap();
    assert_eq!(historical.ending.unwrap().outcome, ClaimOutcome::Reassigned);
    assert_eq!(
        ClaimStore::reassign(
            &guard,
            &first.id,
            &f.candidate(ITEM),
            "new worker",
            &session(),
            "stopped",
            true
        )
        .unwrap_err()
        .code,
        "stale_claim"
    );
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[pair(&first)])
            .unwrap_err()
            .code,
        "stale_claim"
    );
}

#[test]
fn retained_audit_context_does_not_require_live_run_workspace_or_session_records() {
    let f = Fixture::new(true);
    let mut candidate = f.candidate(ITEM);
    candidate.run_id = Some(OTHER.into());
    candidate.workspace_id = Some(OTHER.into());
    candidate.session_record_id = Some(OTHER.into());
    let guard = f.guard(true);
    let claim = ClaimStore::acquire(&guard, &candidate, "worker", &session())
        .unwrap()
        .claim;
    ClaimStore::release(&guard, &claim.id, &session(), "").unwrap();
    assert_eq!(ClaimStore::inspect(&guard, &claim.id).unwrap().claim, claim);
}

#[test]
fn malformed_unknown_dangling_and_mismatched_endings_never_appear_unclaimed() {
    for case in [
        "malformed",
        "unknown",
        "dangling",
        "mismatch",
        "future",
        "old-generation",
    ] {
        let f = Fixture::new(true);
        let claim = f.acquire(ITEM);
        let path = f.root().join(format!("claims/{}.end.yaml", claim.id));
        let ending = ClaimEnding {
            store_id: claim.store_id.clone(),
            recovery_generation: claim.recovery_generation.clone(),
            claim_id: claim.id.clone(),
            item_id: ITEM.into(),
            ended_at: "2026-10-01T01:00:00Z".into(),
            actor: "worker".into(),
            reason: "".into(),
            outcome: ClaimOutcome::Released,
            recovery: false,
        };
        let mut value = ending.to_json();
        match case {
            "malformed" => {
                fs::write(path, b"invalid").unwrap();
            }
            "unknown" => {
                fs::write(f.root().join("claims/unknown.txt"), b"unknown").unwrap();
            }
            "dangling" => {
                fs::remove_file(f.root().join(format!("claims/{}.yaml", claim.id))).unwrap();
                fs::write(path, yaml_bytes(&value)).unwrap();
            }
            "mismatch" => {
                value["item_id"] = json!(OTHER);
                fs::write(path, yaml_bytes(&value)).unwrap();
            }
            "future" => {
                value["format_version"] = json!(2);
                fs::write(path, yaml_bytes(&value)).unwrap();
            }
            "old-generation" => {
                value["recovery_generation"] = json!(OTHER);
                fs::write(path, yaml_bytes(&value)).unwrap();
            }
            _ => unreachable!(),
        }
        let error = ClaimStore::acquire(&f.guard(true), &f.candidate(OTHER), "worker", &session())
            .unwrap_err();
        assert_eq!(
            error.code,
            if case == "future" {
                "unsupported_format"
            } else {
                "invalid_format"
            },
            "{case}"
        );
        assert!(error.path.is_some(), "{case}");
    }
}

#[test]
fn conflicting_acquisitions_are_reported_with_all_ids_without_timestamp_selection() {
    let f = Fixture::new(true);
    let first = f.acquire(ITEM);
    let mut second = first.clone();
    second.id = OTHER.into();
    second.acquired_at = "1900-01-01T00:00:00Z".into();
    fs::write(
        f.root().join(format!("claims/{}.yaml", second.id)),
        yaml_bytes(&second.to_json()),
    )
    .unwrap();
    let guard = f.guard(true);
    let error = ClaimStore::current(&guard, ITEM).unwrap_err();
    assert_eq!(error.code, "claim_conflict");
    let ids = error.details["claim_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&json!(first.id)) && ids.contains(&json!(second.id)));
    assert_eq!(ClaimStore::list(&guard, Some(ITEM), true).unwrap().len(), 2);
    assert_eq!(
        ClaimStore::release(&guard, &first.id, &session(), "")
            .unwrap_err()
            .code,
        "claim_conflict"
    );
}

#[test]
fn storage_recreation_fences_former_pairs_and_unavailable_foundation_is_refused() {
    let fresh = Fixture::new(false);
    assert!(CoordinationGuard::acquire(&fresh.project, true).is_err());
    assert!(!fresh.root().exists());
    let f = Fixture::new(true);
    let claim = f.acquire(ITEM);
    let storage = Storage::new(f.project.clone());
    let metadata = storage.inspect().unwrap().metadata.unwrap();
    fs::remove_dir_all(f.root().join("claims")).unwrap();
    assert!(CoordinationGuard::acquire(&f.project, true).is_err());
    storage
        .recreate(RecreateRequest {
            expected_store_id: Some(metadata.store_id),
            expected_generation: Some(metadata.recovery_generation),
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: true,
        })
        .unwrap();
    let guard = f.guard(true);
    assert_eq!(
        ClaimStore::authorize(&guard, ITEM, &[pair(&claim)])
            .unwrap_err()
            .code,
        "stale_claim"
    );
    assert!(ClaimStore::current(&guard, ITEM).unwrap().is_none());
    let new = ClaimStore::acquire(&guard, &f.candidate(ITEM), "worker", &session())
        .unwrap()
        .claim;
    assert_ne!(new.recovery_generation, claim.recovery_generation);
    assert_ne!(new.id, claim.id);
}

#[test]
fn reassignment_crash_gap_remains_inspectable_and_requires_explicit_new_acquisition() {
    let f = Fixture::new(true);
    let claim = f.acquire(ITEM);
    let ending = ClaimEnding {
        store_id: claim.store_id.clone(),
        recovery_generation: claim.recovery_generation.clone(),
        claim_id: claim.id.clone(),
        item_id: ITEM.into(),
        ended_at: "2026-10-01T01:00:00Z".into(),
        actor: "controller".into(),
        reason: "stopped".into(),
        outcome: ClaimOutcome::Reassigned,
        recovery: true,
    };
    // Persisted first half of reassignment; simulate restart before acquisition.
    fs::write(
        f.root().join(format!("claims/{}.end.yaml", claim.id)),
        yaml_bytes(&ending.to_json()),
    )
    .unwrap();
    let guard = f.guard(true);
    assert!(ClaimStore::current(&guard, ITEM).unwrap().is_none());
    assert!(!ClaimStore::inspect(&guard, &claim.id).unwrap().current);
    assert_eq!(
        ClaimStore::reassign(
            &guard,
            &claim.id,
            &f.candidate(ITEM),
            "worker",
            &session(),
            "stopped",
            true
        )
        .unwrap_err()
        .code,
        "stale_claim"
    );
    let next = ClaimStore::acquire(&guard, &f.candidate(ITEM), "worker", &session())
        .unwrap()
        .claim;
    assert_ne!(next.id, claim.id);
}

// Test processes race through the actual held-lock core, never a copied mutex.
#[test]
#[ignore]
fn claim_process_helper() {
    let Some(checkout) = std::env::var_os("WORK_CLAIMS_TEST_CHECKOUT") else {
        return;
    };
    let project = discover(Some(Path::new(&checkout))).unwrap();
    let gate = PathBuf::from(std::env::var_os("WORK_CLAIMS_TEST_GATE").unwrap());
    let ready = PathBuf::from(std::env::var_os("WORK_CLAIMS_TEST_READY").unwrap());
    let output = PathBuf::from(std::env::var_os("WORK_CLAIMS_TEST_RESULT").unwrap());
    fs::write(ready, b"ready").unwrap();
    wait_until(|| gate.exists());
    let item = std::env::var("WORK_CLAIMS_TEST_ITEM").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match CoordinationGuard::acquire(&project, true) {
            Ok(guard) => {
                let result =
                    ClaimStore::acquire(&guard, &candidate(&project, &item), "process", &session());
                let value = match result {
                    Ok(result) => json!({"claim":result.claim.to_json()}),
                    Err(error) => json!({"code":error.code,"details":error.details}),
                };
                fs::write(output, yaml_bytes(&value)).unwrap();
                // Exit without unwinding while the guard is still held. The OS
                // releases the lock; the emitted claim must survive the process.
                if std::env::var_os("WORK_CLAIMS_TEST_ABRUPT_EXIT").is_some() {
                    std::process::exit(0);
                }
                break;
            }
            Err(error) if error.code == "storage_busy" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("{error}"),
        }
    }
}
fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "child gate timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn independent_linked_worktree_processes_exclude_same_item_but_allow_distinct_items() {
    for distinct in [false, true] {
        let f = Fixture::new(true);
        let linked = f.path.join("linked");
        git(
            &f.path,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        let gate = f.path.join("gate");
        let mut children = Vec::new();
        for (index, checkout) in [&f.path, &linked].into_iter().enumerate() {
            let ready = f.path.join(format!("ready-{index}"));
            let result = f.path.join(format!("result-{index}"));
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "claim_process_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("WORK_CLAIMS_TEST_CHECKOUT", checkout)
                .env("WORK_CLAIMS_TEST_GATE", &gate)
                .env("WORK_CLAIMS_TEST_READY", &ready)
                .env("WORK_CLAIMS_TEST_RESULT", &result)
                .env("WORK_CLAIMS_TEST_ABRUPT_EXIT", "1")
                .env(
                    "WORK_CLAIMS_TEST_ITEM",
                    if distinct && index == 1 { OTHER } else { ITEM },
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            children.push((child, ready, result));
        }
        wait_until(|| children.iter().all(|(_, ready, _)| ready.exists()));
        fs::write(&gate, b"go").unwrap();
        let mut results = Vec::new();
        for (child, _, path) in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            results.push(serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap());
        }
        assert_eq!(
            results.iter().filter(|r| r.get("claim").is_some()).count(),
            if distinct { 2 } else { 1 }
        );
        if !distinct {
            assert_eq!(
                results
                    .iter()
                    .filter(|r| r["code"] == "claim_conflict")
                    .count(),
                1
            );
        }
        assert_eq!(
            ClaimStore::list(&f.guard(false), None, true).unwrap().len(),
            if distinct { 2 } else { 1 }
        );
    }
}

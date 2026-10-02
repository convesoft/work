use super::*;
use crate::core::{
    claims::{ClaimAuthorization, ClaimStore},
    execution::ExecutionOperations,
    operations::MetadataChange,
    project::discover,
    storage::{Storage, files},
};
use std::{fs, process::Command};
struct Fixture {
    root: PathBuf,
    ops: ExecutionOperations,
    from: String,
    to: String,
    claim: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("work-handoff-faults-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let mut ops = ExecutionOperations::new(discover(Some(&root)).unwrap());
        let from = ops
            .create(
                "source".into(),
                b"opaque source".to_vec(),
                MetadataChange::default(),
            )
            .unwrap()
            .file
            .header
            .unwrap()
            .id;
        let to = ops
            .create(
                "receiver".into(),
                b"opaque receiver".to_vec(),
                MetadataChange::default(),
            )
            .unwrap()
            .file
            .header
            .unwrap()
            .id;
        Storage::new(ops.project.clone()).initialize().unwrap();
        let session = SessionIdentity {
            namespace: "test".into(),
            id: "owner".into(),
        };
        let claim = ops.acquire(&from, "worker", &session).unwrap().0["id"]
            .as_str()
            .unwrap()
            .to_owned();
        ops.authorization.push(ClaimAuthorization {
            claim_id: claim.clone(),
            session,
        });
        Self {
            root,
            ops,
            from,
            to,
            claim,
        }
    }
    fn input(&self) -> HandoffInput {
        HandoffInput {
            from_items: vec![self.from.clone()],
            to_items: vec![self.to.clone()],
            body: "\r\n# caller Ω\n---\nverbatim".into(),
            session: None,
            workspace_id: None,
        }
    }
    fn guard(&self) -> CoordinationGuard {
        CoordinationGuard::acquire(&self.ops.project, false).unwrap()
    }
    fn closed(&self) -> bool {
        self.ops
            .inspect(&self.from)
            .unwrap()
            .file
            .header
            .unwrap()
            .state
            == Some(crate::core::items::ManualState::Done)
    }
    fn current(&self) -> bool {
        ClaimStore::inspect(&self.guard(), &self.claim)
            .unwrap()
            .current
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn save_failure_never_closes_or_ends_and_uncertain_publication_is_inspectable_after_restart() {
    for point in ["before_publication", "publication_sync"] {
        let f = Fixture::new();
        let failure = files::fail_next(point);
        let error = f
            .ops
            .close_with_handoffs(&f.from, None, &[f.input()])
            .unwrap_err();
        drop(failure);
        assert_eq!(error.code, "io");
        assert!(!f.closed());
        assert!(f.current());
        let id = error.details["handoff_id"].as_str().unwrap();
        if point == "publication_sync" {
            assert_eq!(error.details["publication"], "possible");
            assert_eq!(
                error.details["partial"]["uncertain_paths"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            let restarted = ExecutionOperations::new(f.ops.project.clone());
            assert_eq!(
                restarted.handoff_inspect(id).unwrap()["handoff"]["body"],
                f.input().body
            );
        } else {
            assert_eq!(error.details["publication"], "not_published");
            assert_eq!(f.ops.handoff_inspect(id).unwrap_err().code, "not_found");
        }
    }
}
#[test]
fn interruption_after_saving_and_after_item_publication_retains_context_and_current_owner() {
    for point in ["handoffs_saved", "checkout_after_publish"] {
        let f = Fixture::new();
        let failure = files::fail_next(point);
        let error = f
            .ops
            .close_with_handoffs(&f.from, None, &[f.input(), f.input()])
            .unwrap_err();
        drop(failure);
        assert_eq!(error.details["saved_handoffs"].as_array().unwrap().len(), 2);
        assert!(f.current());
        assert_eq!(f.closed(), point == "checkout_after_publish");
        assert_eq!(
            f.ops.handoff_list(None).unwrap()["handoffs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        // Retry only completion, not already-saved notes. Same owner completes.
        f.ops.close(&f.from, None).unwrap();
        assert!(!f.current());
        assert_eq!(
            f.ops.handoff_list(None).unwrap()["handoffs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
#[test]
fn later_handoff_save_failure_reports_earlier_ids_and_never_closes_source() {
    let f = Fixture::new();
    let hook = files::on_nth("before_publication", 2, || {
        std::mem::forget(files::fail_next("before_publication"));
    });
    let error = f
        .ops
        .close_with_handoffs(&f.from, None, &[f.input(), f.input()])
        .unwrap_err();
    drop(hook);
    assert_eq!(error.details["saved_handoffs"].as_array().unwrap().len(), 1);
    assert_eq!(
        error.details["partial"]["created"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!f.closed());
    assert!(f.current());
}
#[test]
fn ending_failure_is_after_saved_context_and_item_and_same_owner_can_finish_without_resubmission() {
    let f = Fixture::new();
    // First entity publication is context, second is the completed ending.
    let hook = files::on_nth("before_publication", 2, || {
        std::mem::forget(files::fail_next("before_publication"));
    });
    let error = f
        .ops
        .close_with_handoffs(&f.from, None, &[f.input()])
        .unwrap_err();
    drop(hook);
    assert!(f.closed());
    assert!(f.current());
    assert_eq!(error.details["saved_handoffs"].as_array().unwrap().len(), 1);
    assert_eq!(error.details["partial"]["updated"][0]["id"], f.from);
    f.ops.close(&f.from, None).unwrap();
    assert!(!f.current());
    assert_eq!(
        f.ops.handoff_list(None).unwrap()["handoffs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn pruning_failure_reports_saved_completion_and_remaining_context_without_rollback() {
    for point in ["entity_before_delete", "entity_delete_sync"] {
        let mut f = Fixture::new();
        f.ops.handoff_create(&f.input()).unwrap();
        f.ops.close(&f.from, None).unwrap();
        assert!(!f.current());
        f.ops.authorization.clear();
        let hook = files::fail_next(point);
        let error = f.ops.close(&f.to, None).unwrap_err();
        drop(hook);
        assert_eq!(error.code, "io");
        assert_eq!(error.details["partial"]["updated"][0]["id"], f.to);
        assert!(
            error.details["remaining_handoffs"]
                .as_array()
                .unwrap()
                .len()
                == 1
        );
        assert_eq!(
            f.ops.inspect(&f.to).unwrap().file.header.unwrap().state,
            Some(crate::core::items::ManualState::Done)
        );
        if point == "entity_delete_sync" {
            assert_eq!(error.details["publication"], "possible");
            assert_eq!(
                error.details["partial"]["uncertain_paths"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
        f.ops.handoff_prune(None).unwrap();
        assert_eq!(f.ops.handoff_list(None).unwrap()["handoffs"], json!([]));
    }
}
#[test]
fn invalid_close_inputs_are_prevalidated_before_any_context_is_saved() {
    let f = Fixture::new();
    let mut bad = f.input();
    bad.to_items.push(new_id().unwrap());
    assert_eq!(
        f.ops
            .close_with_handoffs(&f.from, None, &[f.input(), bad])
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(f.ops.handoff_list(None).unwrap()["handoffs"], json!([]));
    assert!(!f.closed());
    assert!(f.current());
}
#[test]
fn malformed_frontmatter_and_headers_refuse_without_panics_or_receiver_edits() {
    let f = Fixture::new();
    let h = f.ops.handoff_create(&f.input()).unwrap();
    let id = h["handoff"]["id"].as_str().unwrap();
    let path = decode_path(h["handoff"]["path"].as_str().unwrap()).unwrap();
    let original = fs::read(&path).unwrap();
    for raw in [
        b"---".as_slice(),
        b"---\n---\nbody".as_slice(),
        b"---\nformat_version: 7\nextra: true\n---\nbody".as_slice(),
    ] {
        fs::write(&path, raw).unwrap();
        assert!(f.ops.handoff_inspect(id).is_err());
        assert!(
            f.ops
                .handoff_receivers(id, std::slice::from_ref(&f.to))
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
    fs::write(&path, original).unwrap();
    for (key, value) in [
        ("created_at", json!("2026-02-30T00:00:00Z")),
        ("unknown", json!(true)),
        ("to_items", json!([f.to, f.to])),
        ("session", json!({"namespace":"","id":"x"})),
        ("workspace_id", json!("bad")),
    ] {
        let g = f.guard();
        let mut record = HandoffStore::inspect(&g, id).unwrap();
        drop(g);
        record.header[key] = value;
        fs::write(&path, encode(&record.header, &record.body)).unwrap();
        assert!(f.ops.handoff_inspect(id).is_err());
        fs::write(&path, &record.source.raw).unwrap();
    }
}

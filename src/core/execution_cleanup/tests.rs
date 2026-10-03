use super::*;
use crate::core::{
    operations::MetadataChange,
    storage::{Storage, files},
};
use std::{fs, process::Command};
struct Fixture {
    dir: PathBuf,
    root: PathBuf,
    target: PathBuf,
    ops: ExecutionOperations,
    item: String,
    control: String,
    wid: String,
}
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("work-cleanup-core-{}", new_id().unwrap()));
        let root = dir.join("controller");
        let target = dir.join("target");
        fs::create_dir_all(root.join(".work/items")).unwrap();
        git(&root, &["init", "-q", "--initial-branch=main"]);
        let ops = ExecutionOperations::new(discover(Some(&root)).unwrap());
        let item = ops
            .create(
                "cleanup".into(),
                b"retained".to_vec(),
                MetadataChange::default(),
            )
            .unwrap()
            .file
            .header
            .unwrap()
            .id;
        git(&root, &["add", ".work/items"]);
        git(
            &root,
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
        git(
            &root,
            &[
                "worktree",
                "add",
                "-qb",
                "feature",
                target.to_str().unwrap(),
            ],
        );
        Storage::new(ops.project.clone()).initialize().unwrap();
        let control = ops.workspace_register(&root, None, None).unwrap()["workspace"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let wid = ops.workspace_register(&target, None, None).unwrap()["workspace"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            dir,
            root,
            target,
            ops,
            item,
            control,
            wid,
        }
    }
    fn begin(&self) -> ExecutionResult<Value> {
        self.ops
            .workspace_cleanup_begin(&self.wid, &self.item, &self.control)
    }
    fn report(&self) -> ExecutionResult<Value> {
        self.ops.workspace_cleanup_report(&self.wid, true, None)
    }
    fn record(&self) -> PathBuf {
        self.ops
            .project
            .git_common_dir
            .join("work")
            .join(path(&self.wid))
    }
    fn remove(&self) {
        git(
            &self.root,
            &[
                "worktree",
                "remove",
                "--force",
                self.target.to_str().unwrap(),
            ],
        );
    }
    fn state(&self) -> Value {
        self.ops.workspace_inspect(&self.wid).unwrap()["workspace"].clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn begin_publication_failures_preserve_inspectable_state_and_same_context_retry() {
    for point in ["before_publication", "after_directory_sync"] {
        let f = Fixture::new();
        let old = fs::read(f.record()).unwrap();
        let hook = files::fail_next(point);
        let e = f.begin().unwrap_err();
        drop(hook);
        assert_eq!(e.code, "io");
        assert!(f.target.is_dir());
        if point == "before_publication" {
            assert_eq!(e.details["publication"], "not_published");
            assert!(
                e.details["partial"]["updated"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(fs::read(f.record()).unwrap(), old);
            assert!(f.begin().unwrap()["changed"].as_bool().unwrap());
        } else {
            assert_eq!(e.details["publication"], "possible");
            assert_eq!(
                e.details["partial"]["uncertain_paths"],
                json!([encode_path(&f.record())])
            );
            let saved = f.state();
            assert_eq!(saved["state"], "closing");
            let retry = f.begin().unwrap();
            assert_eq!(retry["changed"], false);
            assert_eq!(retry["workspace"], saved);
        }
    }
}

#[test]
fn report_interruption_before_and_after_record_deletion_has_observable_recovery() {
    for point in ["entity_before_delete", "entity_delete_sync"] {
        let f = Fixture::new();
        f.begin().unwrap();
        f.remove();
        let hook = files::fail_next(point);
        let e = f.report().unwrap_err();
        drop(hook);
        assert_eq!(e.code, "io");
        assert!(!f.target.exists());
        if point == "entity_before_delete" {
            assert!(f.record().exists());
            assert_eq!(f.state()["state"], "closing");
            assert_eq!(f.report().unwrap()["removed"], true);
        } else {
            assert_eq!(e.details["publication"], "possible");
            assert_eq!(
                e.details["partial"]["uncertain_paths"],
                json!([encode_path(&f.record())])
            );
            assert!(!f.record().exists());
            assert_eq!(
                f.ops.workspace_inspect(&f.wid).unwrap_err().code,
                "not_found"
            );
            assert_eq!(f.report().unwrap_err().code, "not_found");
        }
        assert_eq!(
            f.ops.inspect(&f.item).unwrap().file.header.unwrap().state,
            Some(crate::core::items::ManualState::Open)
        );
    }
}

#[test]
fn cancel_and_failure_publication_errors_do_not_hide_saved_context() {
    for cancel in [false, true] {
        let f = Fixture::new();
        f.begin().unwrap();
        let hook = files::fail_next("after_directory_sync");
        let e = if cancel {
            f.ops.workspace_cleanup_cancel(&f.wid)
        } else {
            f.ops
                .workspace_cleanup_report(&f.wid, false, Some("external failure"))
        }
        .unwrap_err();
        drop(hook);
        assert_eq!(e.details["publication"], "possible");
        assert_eq!(
            e.details["partial"]["uncertain_paths"],
            json!([encode_path(&f.record())])
        );
        if cancel {
            assert_eq!(f.state()["state"], "open");
            assert!(f.state()["cleanup"].is_null());
        } else {
            assert_eq!(f.state()["cleanup"]["failure"], "external failure");
        }
        assert!(f.target.is_dir());
    }
}

#[test]
fn held_lock_source_checks_detect_same_byte_entity_and_material_substitution() {
    for kind in [
        "workspace",
        "binding",
        "claim",
        "handoff",
        "material",
        "run",
    ] {
        let f = Fixture::new();
        let session = SessionIdentity {
            namespace: "test".into(),
            id: "owner".into(),
        };
        let (claim, _) = f.ops.acquire(&f.item, "controller", &session).unwrap();
        let source = match kind {
            "workspace" => f.record(),
            "binding" => f
                .ops
                .project
                .git_common_dir
                .join(format!("work/workspaces/items/{}.yaml", f.item)),
            "claim" => f.ops.project.git_common_dir.join(format!(
                "work/claims/{}.yaml",
                claim["id"].as_str().unwrap()
            )),
            "handoff" => {
                let input = crate::core::handoffs::HandoffInput {
                    from_items: vec![f.item.clone()],
                    to_items: vec![f.item.clone()],
                    body: "opaque".into(),
                    session: None,
                    workspace_id: Some(f.wid.clone()),
                };
                let mut ops = ExecutionOperations::new(f.ops.project.clone());
                ops.authorization
                    .push(crate::core::claims::ClaimAuthorization {
                        claim_id: claim["id"].as_str().unwrap().into(),
                        session: session.clone(),
                    });
                let h = ops.handoff_create(&input).unwrap();
                f.ops.project.git_common_dir.join(format!(
                    "work/handoffs/{}.md",
                    h["handoff"]["id"].as_str().unwrap()
                ))
            }
            "material" => f.root.join(format!(".work/items/{}.md", f.item)),
            "run" => {
                let run = f
                    .ops
                    .run_start(&f.item, Some(f.control.clone()), Some(f.control.clone()))
                    .unwrap();
                f.ops.project.git_common_dir.join(format!(
                    "work/runs/{}/run.yaml",
                    run["run"]["id"].as_str().unwrap()
                ))
            }
            _ => unreachable!(),
        };
        let hook = files::on_next("checkout_verify", move || {
            let stage = source.with_extension("swap");
            fs::write(&stage, fs::read(&source).unwrap()).unwrap();
            fs::rename(stage, source).unwrap();
        });
        let e = f.begin().unwrap_err();
        drop(hook);
        assert_eq!(e.code, "conflict", "{kind}: {e}");
        assert_eq!(f.state()["state"], "open");
    }
}

#[test]
fn malformed_shared_context_blocks_removal_but_failure_is_still_recordable() {
    for dir in ["claims", "handoffs", "runs"] {
        let f = Fixture::new();
        f.begin().unwrap();
        let bad = f
            .ops
            .project
            .git_common_dir
            .join("work")
            .join(dir)
            .join("unexpected");
        fs::write(&bad, b"not an entity").unwrap();
        f.remove();
        assert_eq!(f.report().unwrap_err().code, "invalid_format");
        assert_eq!(f.begin().unwrap()["changed"], false);
        let failed = f
            .ops
            .workspace_cleanup_report(&f.wid, false, Some("damaged shared context"))
            .unwrap();
        assert_eq!(
            failed["workspace"]["cleanup"]["failure"],
            "damaged shared context"
        );
        fs::remove_file(bad).unwrap();
        f.report().unwrap();
    }
}

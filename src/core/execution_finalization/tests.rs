use super::*;
use crate::core::{
    context::ContextStore,
    coordination::SessionIdentity,
    operations::{DurableOperations, default_header},
    runs::RunStore,
    storage::{Storage, files},
};
use std::{fs, process::Command};
struct Fixture {
    root: PathBuf,
    ops: ExecutionOperations,
    root_id: String,
    run: String,
    ids: Vec<String>,
}
impl Fixture {
    fn new(done: bool) -> Self {
        let root =
            std::env::temp_dir().join(format!("work-finalization-fault-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        let root_id = DurableOperations::new(&root)
            .create(
                "root".into(),
                b"root untouched".to_vec(),
                Default::default(),
            )
            .unwrap()
            .file
            .header
            .unwrap()
            .id;
        let project = crate::core::project::discover(Some(&root)).unwrap();
        Storage::new(project.clone()).initialize().unwrap();
        let ops = ExecutionOperations::new(project);
        let run: String = ops.run_start(&root_id, None, None).unwrap()["run"]["id"]
            .as_str()
            .unwrap()
            .into();
        let mut ids = vec![new_id().unwrap(), new_id().unwrap(), new_id().unwrap()];
        ids.sort();
        {
            let g = CoordinationGuard::acquire(&ops.project, true).unwrap();
            for (i, id) in ids.iter().enumerate() {
                let mut h = default_header(id.clone(), format!("wisp{i}"));
                // Later files reference the first deleted ID. A restart must
                // continue despite dangling edges wholly inside the frozen set.
                if i > 0 {
                    h.depends_on.push(ids[i - 1].clone());
                }
                RunStore::create_wisp(&g, &run, &h, b"wisp body").unwrap();
            }
        }
        if done {
            for id in &ids {
                ops.close(id, None).unwrap();
            }
        }
        ops.session_set(
            &run,
            "worker",
            &SessionIdentity {
                namespace: "test".into(),
                id: "one".into(),
            },
            None,
        )
        .unwrap();
        Self {
            root,
            ops,
            root_id,
            run,
            ids,
        }
    }
    fn manifest(&self) -> Value {
        self.ops.run_inspect(&self.run).unwrap()["run"].clone()
    }
    fn restart(&self) -> ExecutionOperations {
        ExecutionOperations::new(crate::core::project::discover(Some(&self.root)).unwrap())
    }
    fn digest(&self) -> PathBuf {
        crate::core::digests::path(&self.root, &self.root_id, &self.run)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn pending_squash_retains_before_delete_and_requires_matching_digest() {
    let f = Fixture::new(true);
    let hook = files::fail_next("entity_before_delete");
    let error = f.ops.run_squash(&f.run, "verbatim\r\n").unwrap_err();
    drop(hook);
    assert_eq!(error.code, "io");
    assert_eq!(
        error.details["remaining_items"].as_array().unwrap().len(),
        3
    );
    assert_eq!(f.manifest()["phase"], "squashing");
    assert!(f.digest().is_file());
    assert_eq!(
        f.restart()
            .run_squash(&f.run, "different")
            .unwrap_err()
            .code,
        "run_conflict"
    );
    assert_eq!(
        f.restart().run_discard(&f.run, true, &[]).unwrap_err().code,
        "run_conflict"
    );
    assert_eq!(
        f.restart().run_squash(&f.run, "verbatim\r\n").unwrap()["phase"],
        "finalized"
    );
    assert_eq!(
        f.restart().run_squash(&f.run, "verbatim\r\n").unwrap()["changed"],
        false
    );
}
#[test]
fn interruption_before_digest_freezes_without_deleting() {
    let f = Fixture::new(true);
    let hook = files::fail_next("finalization_before_digest");
    let error = f.ops.run_squash(&f.run, "resupplied").unwrap_err();
    drop(hook);
    assert_eq!(error.code, "io");
    assert!(!f.digest().exists());
    assert_eq!(f.manifest()["phase"], "squashing");
    assert_eq!(
        f.manifest()["cleanup"]["item_ids"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        f.ops
            .run_membership(&f.run, std::slice::from_ref(&f.root_id), true)
            .unwrap_err()
            .code,
        "run_not_current"
    );
    let session = SessionIdentity {
        namespace: "test".into(),
        id: "one".into(),
    };
    assert_eq!(
        f.ops.acquire(&f.ids[0], "test", &session).unwrap_err().code,
        "run_not_current"
    );
    assert_eq!(
        f.ops
            .session_set(&f.run, "new", &session, None)
            .unwrap_err()
            .code,
        "run_not_current"
    );
    assert_eq!(
        f.restart().run_squash(&f.run, "resupplied").unwrap()["phase"],
        "finalized"
    );
}
#[test]
fn partial_discard_retry_uses_recorded_set_and_keeps_internal_dangling_edges_inspectable() {
    let f = Fixture::new(false);
    let hook = files::fail_next("finalization_after_delete");
    let error = f.ops.run_discard(&f.run, true, &[]).unwrap_err();
    drop(hook);
    assert_eq!(
        error.details["partial"]["deleted"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        error.details["remaining_items"].as_array().unwrap().len(),
        2
    );
    assert_eq!(f.manifest()["phase"], "discarding");
    assert_eq!(
        f.restart()
            .run_discard(&f.run, false, &f.ids)
            .unwrap_err()
            .code,
        "run_conflict"
    );
    assert_eq!(
        f.restart().run_discard(&f.run, true, &[]).unwrap()["phase"],
        "disposed"
    );
    assert!(!f.digest().exists());
}
#[test]
fn subset_pending_retry_exact_set_and_post_delete_sync_error() {
    let f = Fixture::new(false);
    let hook = files::fail_next("entity_delete_sync");
    let error = f.ops.run_discard(&f.run, false, &f.ids).unwrap_err();
    drop(hook);
    assert_eq!(error.details["publication"], "possible");
    assert_eq!(
        error.details["partial"]["deleted"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(f.manifest()["cleanup"]["finalize"], false);
    assert_eq!(
        f.ops
            .run_discard(&f.run, false, &f.ids[1..])
            .unwrap_err()
            .code,
        "run_conflict"
    );
    let result = f.restart().run_discard(&f.run, false, &f.ids).unwrap();
    assert_eq!(result["phase"], "active");
    assert!(f.manifest().get("cleanup").is_none());
    assert_eq!(
        f.ops.session_list(&f.run).unwrap()["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn digest_publication_error_and_terminal_manifest_error_preserve_restart_evidence() {
    for point in [
        "finalization_digest_retained",
        "finalization_before_terminal",
    ] {
        let f = Fixture::new(true);
        let original = fs::read(f.root.join(format!(".work/items/{}.md", f.root_id))).unwrap();
        let hook = files::fail_next(point);
        let error = f.ops.run_squash(&f.run, "summary").unwrap_err();
        drop(hook);
        assert_eq!(error.code, "io");
        assert!(f.digest().exists());
        assert_eq!(f.manifest()["phase"], "squashing");
        assert_eq!(
            f.restart().run_squash(&f.run, "summary").unwrap()["phase"],
            "finalized"
        );
        assert_eq!(
            original,
            fs::read(f.root.join(format!(".work/items/{}.md", f.root_id))).unwrap()
        );
    }
}
#[test]
fn frozen_material_aggregate_survives_partial_child_cleanup_without_reconstruction() {
    use crate::core::{items::Completion, operations::MetadataChange};
    let f = Fixture::new(true);
    let aggregate = f
        .ops
        .create(
            "material aggregate".into(),
            b"unchanged material bytes".to_vec(),
            MetadataChange {
                completion: Some(Completion::Children),
                ..Default::default()
            },
        )
        .unwrap()
        .file
        .header
        .unwrap()
        .id;
    f.ops
        .update(
            &f.ids[0],
            MetadataChange {
                parent: Some(Some(aggregate.clone())),
                ..Default::default()
            },
        )
        .unwrap();
    f.ops
        .run_membership(&f.run, std::slice::from_ref(&aggregate), true)
        .unwrap();
    assert_eq!(f.ops.run_inspect(&f.run).unwrap()["finished"], true);
    let source = f.root.join(format!(".work/items/{aggregate}.md"));
    let original = fs::read(&source).unwrap();
    let hook = files::fail_next("finalization_after_delete");
    f.ops
        .run_squash(&f.run, "retained aggregate outcome")
        .unwrap_err();
    drop(hook);
    assert_eq!(
        f.restart()
            .run_squash(&f.run, "retained aggregate outcome")
            .unwrap()["phase"],
        "finalized"
    );
    assert_eq!(fs::read(source).unwrap(), original);
    assert_eq!(
        f.ops.run_inspect(&f.run).unwrap()["members"],
        json!([aggregate])
    );
}

#[test]
fn pending_subset_retry_resolves_original_prefixes_including_deleted_items() {
    let f = Fixture::new(false);
    let prefixes: Vec<_> = f.ids.iter().map(|id| format!("w-{}", &id[..8])).collect();
    let hook = files::fail_next("finalization_after_delete");
    let error = f.ops.run_discard(&f.run, false, &prefixes).unwrap_err();
    drop(hook);
    assert_eq!(
        error.details["remaining_items"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        f.restart().run_discard(&f.run, false, &prefixes).unwrap()["phase"],
        "active"
    );
    assert_eq!(
        f.restart()
            .run_discard(&f.run, false, &prefixes)
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn interrupted_digest_publication_is_uncertain_but_never_deletes_wisps() {
    let f = Fixture::new(true);
    let inner = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured = inner.clone();
    // First publication is the frozen manifest, second is the digest.
    let hook = files::on_nth("after_publication", 2, move || {
        *captured.borrow_mut() = Some(files::fail_next("publication_sync"));
    });
    let error = f.ops.run_squash(&f.run, "publication").unwrap_err();
    drop(hook);
    drop(inner);
    assert_eq!(error.code, "io");
    assert_eq!(error.details["publication"], "possible");
    assert_eq!(
        error.details["partial"]["uncertain_paths"],
        json!([encode_path(&f.digest())])
    );
    assert_eq!(
        error.details["remaining_items"].as_array().unwrap().len(),
        3
    );
    assert!(f.digest().is_file());
    assert_eq!(
        f.restart().run_squash(&f.run, "publication").unwrap()["phase"],
        "finalized"
    );
}

#[test]
fn outside_edit_after_digest_prevents_cleanup_and_material_bindings_survive_discard() {
    let f = Fixture::new(true);
    let root_path = f.root.join(format!(".work/items/{}.md", f.root_id));
    let root = fs::read(&root_path).unwrap();
    let p = root_path.clone();
    let hook = files::on_next("finalization_digest_retained", move || {
        fs::write(p, b"external conflict").unwrap();
    });
    assert_eq!(
        f.ops.run_squash(&f.run, "summary").unwrap_err().code,
        "conflict"
    );
    drop(hook);
    assert_eq!(f.manifest()["phase"], "squashing");
    assert_eq!(
        f.manifest()["cleanup"]["item_ids"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    fs::write(root_path, root).unwrap();
    f.restart().run_squash(&f.run, "summary").unwrap();
    let g = CoordinationGuard::acquire(&f.ops.project, false).unwrap();
    assert!(
        ContextStore::load(&g)
            .unwrap()
            .bindings
            .contains_key(&f.root_id)
    );
}
#[test]
#[ignore = "subprocess-only crash entry"]
fn crash_child() {
    let root = PathBuf::from(std::env::var("WORK_FINALIZATION_CRASH_ROOT").unwrap());
    let run = std::env::var("WORK_FINALIZATION_CRASH_RUN").unwrap();
    let kind = std::env::var("WORK_FINALIZATION_CRASH_KIND").unwrap();
    let ops = ExecutionOperations::new(crate::core::project::discover(Some(&root)).unwrap());
    let _hook = files::on_next("finalization_after_delete", || std::process::exit(77));
    if kind == "squash" {
        ops.run_squash(&run, "crash summary").unwrap();
    } else {
        ops.run_discard(&run, true, &[]).unwrap();
    }
    panic!("crash hook was not reached");
}
#[test]
fn actual_process_exit_releases_lock_and_restart_continues_cleanup() {
    for kind in ["squash", "discard"] {
        let f = Fixture::new(kind == "squash");
        let o = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "core::execution_finalization::tests::crash_child",
                "--nocapture",
            ])
            .env("WORK_FINALIZATION_CRASH_ROOT", &f.root)
            .env("WORK_FINALIZATION_CRASH_RUN", &f.run)
            .env("WORK_FINALIZATION_CRASH_KIND", kind)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(77), "{:?}", o);
        assert_eq!(f.digest().exists(), kind == "squash");
        let restart = f.restart();
        if kind == "squash" {
            assert_eq!(
                restart.run_squash(&f.run, "crash summary").unwrap()["phase"],
                "finalized"
            );
        } else {
            assert_eq!(
                restart.run_discard(&f.run, true, &[]).unwrap()["phase"],
                "disposed"
            );
        }
    }
}

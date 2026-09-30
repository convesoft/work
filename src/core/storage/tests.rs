//! Fault injection is private to core tests; no production switch or protocol backdoor.
use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    storage: Storage,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "work-storage-faults-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let common = root.join("git-common");
        fs::create_dir(&common).unwrap();
        let storage = Storage::new(Project {
            worktree_root: root.clone(),
            git_common_dir: common,
        });
        Self { root, storage }
    }
    fn request(&self) -> RecreateRequest {
        let status = self.storage.inspect().unwrap();
        RecreateRequest {
            expected_store_id: status
                .retained_store_id
                .or_else(|| status.metadata.as_ref().map(|m| m.store_id.clone())),
            expected_generation: status.metadata.map(|m| m.recovery_generation),
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: true,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn injected() -> StorageError {
    StorageError::new(
        StorageErrorCode::Io,
        "injected storage boundary failure",
        None,
    )
}
#[test]
fn interruption_at_each_initialization_boundary_preserves_identity_or_requires_explicit_reset() {
    for point in [
        "root_lock",
        "stage_store",
        "stage_identity",
        "context",
        "prepared",
        "live_folders",
        "witness",
        "metadata",
        "committed",
        "complete",
    ] {
        let f = Fixture::new();
        let error = f
            .storage
            .initialize_inner(&mut |step| {
                if step == point {
                    Err(injected())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let before = f.storage.inspect().unwrap();
        if matches!(
            point,
            "prepared" | "live_folders" | "witness" | "metadata" | "committed" | "complete"
        ) {
            let id = error.operation_id.as_deref().unwrap();
            let dir = f
                .storage
                .project
                .git_common_dir
                .join("work/operations")
                .join(id);
            let intended =
                format::metadata(&fs::read(dir.join("store.yaml")).unwrap(), &dir).unwrap();
            if point != "complete" {
                assert!(!before.coordination_available, "{point}");
            }
            let result = f.storage.recover(id, RecoverRequest::default()).unwrap();
            assert_eq!(result.storage.metadata, Some(intended), "{point}");
            assert!(result.storage.coordination_available);
            assert_eq!(
                f.storage
                    .recover(id, RecoverRequest::default())
                    .unwrap()
                    .storage,
                result.storage
            );
        } else {
            assert!(!before.coordination_available);
            assert!(f.storage.initialize().is_err());
            let result = f.storage.recreate(f.request()).unwrap();
            assert!(result.storage.coordination_available, "{point}");
        }
    }
}
#[test]
fn interruption_at_each_recreation_archive_and_publication_boundary_reuses_generation() {
    for point in [
        "prepared",
        "archive_folder:claims",
        "archive_folder:handoffs",
        "archive_folder:runs",
        "archive_folder:workspaces",
        "archived",
        "live_folders",
        "witness",
        "metadata",
        "committed",
        "complete",
    ] {
        let f = Fixture::new();
        let initial = f.storage.initialize().unwrap();
        let initial_id = initial.operation_id.unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        for name in LIVE {
            fs::write(
                root.join(name).join("opaque.data"),
                format!("{name}:\0 retained\n"),
            )
            .unwrap();
        }
        let error = f
            .storage
            .recreate_inner(f.request(), &mut |step| {
                if step == point {
                    Err(injected())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let id = error.operation_id.as_deref().unwrap();
        let dir = root.join("operations").join(id);
        let intended = format::metadata(&fs::read(dir.join("store.yaml")).unwrap(), &dir).unwrap();
        if point != "complete" {
            assert!(
                !f.storage.inspect().unwrap().coordination_available,
                "{point}"
            );
        }
        assert_eq!(
            f.storage
                .recover(id, RecoverRequest::default())
                .unwrap_err()
                .code,
            StorageErrorCode::InvalidArgument
        );
        let result = f
            .storage
            .recover(
                id,
                RecoverRequest {
                    executors_stopped: true,
                    acknowledge_loss: true,
                    all_clients_stopped: false,
                },
            )
            .unwrap();
        assert_eq!(result.storage.metadata, Some(intended), "{point}");
        let archive = root.join("recovery").join(id);
        for name in LIVE {
            assert_eq!(
                fs::read(archive.join("prior").join(name).join("opaque.data")).unwrap(),
                format!("{name}:\0 retained\n").as_bytes(),
                "{point}"
            );
            assert_eq!(fs::read_dir(root.join(name)).unwrap().count(), 0);
        }
        assert!(
            archive
                .join("operations")
                .join(initial_id)
                .join("operation.yaml")
                .is_file()
        );
    }
}
#[test]
fn changing_captured_source_or_directory_before_archival_refuses_without_overwrite() {
    for directory in [false, true] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        let request = f.request();
        let changed = b"unexpected external source\n";
        let error = f
            .storage
            .recreate_inner(request, &mut |step| {
                if step == "prepared" {
                    if directory {
                        fs::rename(root.join("claims"), f.root.join("retained-claims")).unwrap();
                        fs::create_dir(root.join("claims")).unwrap();
                    } else {
                        fs::write(root.join("store.yaml"), changed).unwrap();
                    }
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.code, StorageErrorCode::Conflict);
        if !directory {
            assert_eq!(fs::read(root.join("store.yaml")).unwrap(), changed);
        }
        assert!(!f.storage.inspect().unwrap().coordination_available);
    }
}
#[test]
fn replacing_archived_directory_or_conflicting_both_locations_refuses_recovery() {
    let f = Fixture::new();
    f.storage.initialize().unwrap();
    let root = f.storage.project.git_common_dir.join("work");
    let error = f
        .storage
        .recreate_inner(f.request(), &mut |step| {
            if step == "archive_folder:claims" {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    fs::create_dir(root.join("claims")).unwrap();
    let result = f
        .storage
        .recover(
            error.operation_id.as_deref().unwrap(),
            RecoverRequest {
                executors_stopped: true,
                acknowledge_loss: true,
                all_clients_stopped: false,
            },
        )
        .unwrap_err();
    assert_eq!(result.code, StorageErrorCode::Conflict);
    assert!(root.join("claims").is_dir());
}
#[test]
fn completed_recreation_missing_retained_archive_does_not_create_replacement_paths() {
    for target in ["archive", "prior", "operations", "prior/claims"] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let outcome = f.storage.recreate(f.request()).unwrap();
        let id = outcome.operation_id.as_deref().unwrap();
        let archive = outcome.loss.unwrap().prior_state_path.unwrap();
        let path = if target == "archive" {
            archive.clone()
        } else {
            archive.join(target)
        };
        fs::remove_dir_all(&path).unwrap();
        let error = f
            .storage
            .recover(
                id,
                RecoverRequest {
                    executors_stopped: true,
                    acknowledge_loss: true,
                    all_clients_stopped: false,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, StorageErrorCode::StorageMissing, "{target}");
        assert!(!path.exists(), "{target}");
    }
}
#[test]
fn strict_context_rejects_unknown_keys_wrong_snapshots_and_timestamp_bounds() {
    let f = Fixture::new();
    f.storage.initialize().unwrap();
    let error = f
        .storage
        .recreate_inner(f.request(), &mut |step| {
            if step == "prepared" {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let id = error.operation_id.as_deref().unwrap();
    let dir = f
        .storage
        .project
        .git_common_dir
        .join("work/operations")
        .join(id);
    let operation =
        format::operation(&fs::read(dir.join("operation.yaml")).unwrap(), &dir).unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("context.yaml")).unwrap()).unwrap();
    for kind in [
        "unknown",
        "size",
        "nano",
        "mode",
        "negative",
        "folder_keys",
        "tag",
    ] {
        let mut value = original.clone();
        match kind {
            "unknown" => {
                value["unknown"] = serde_json::json!(true);
            }
            "size" => {
                value["store"]["size"] = serde_json::json!(1);
            }
            "nano" => {
                value["store"]["mtime"][1] = serde_json::json!(1_000_000_000);
            }
            "mode" => {
                value["store"]["mode"] = serde_json::json!(u64::MAX);
            }
            "negative" => {
                value["identity"]["ino"] = serde_json::json!(-1);
            }
            "folder_keys" => {
                value["folders"]["wrong"] = serde_json::json!({"dev":1,"ino":1});
            }
            "tag" => {
                value["store"]["raw_hex"] = serde_json::json!("ZZ");
            }
            _ => unreachable!(),
        }
        assert_eq!(
            format::context(&format::bytes(&value), &operation, &dir)
                .unwrap_err()
                .code,
            StorageErrorCode::InvalidFormat,
            "{kind}"
        );
    }
}
#[test]
fn same_directory_file_exchange_retains_bytes_and_open_inode_writes() {
    use std::io::Write;
    let f = Fixture::new();
    let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
    common.stage("source.yaml", b"old bytes").unwrap();
    let before = common.read("source.yaml").unwrap();
    let mut open_old = fs::OpenOptions::new()
        .append(true)
        .open(common.path.join("source.yaml"))
        .unwrap();
    let retained = common
        .publish(
            "source.yaml",
            b"new bytes",
            Some(&before),
            ".storage-test-stage",
        )
        .unwrap()
        .unwrap();
    open_old.write_all(b" external open-handle edit").unwrap();
    open_old.sync_all().unwrap();
    assert_eq!(
        fs::read(retained).unwrap(),
        b"old bytes external open-handle edit"
    );
    assert_eq!(common.read("source.yaml").unwrap().raw, b"new bytes");
}
#[test]
fn path_substitution_keeps_descriptor_bound_source_untouched() {
    let f = Fixture::new();
    let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
    let directory = common.create("held").unwrap();
    directory.stage("source.yaml", b"safe").unwrap();
    let snapshot = directory.read("source.yaml").unwrap();
    fs::rename(&directory.path, common.path.join("old-held")).unwrap();
    fs::create_dir(&directory.path).unwrap();
    let error = directory
        .publish(
            "source.yaml",
            b"replacement",
            Some(&snapshot),
            ".storage-test-stage",
        )
        .unwrap_err();
    assert_eq!(error.code, StorageErrorCode::Conflict);
    assert_eq!(
        fs::read(common.path.join("old-held/source.yaml")).unwrap(),
        b"safe"
    );
    assert!(!directory.path.join("source.yaml").exists());
}

#[test]
fn staged_write_sync_and_post_publication_io_failures_preserve_inspectable_sources() {
    for point in [
        "stage_write",
        "stage_sync",
        "before_publication",
        "after_publication",
        "publication_sync",
        "after_directory_sync",
    ] {
        let f = Fixture::new();
        let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
        common.stage("source.yaml", b"old bytes").unwrap();
        let source = common.read("source.yaml").unwrap();
        let _guard = files::fail_next(point);
        let error = common
            .publish(
                "source.yaml",
                b"new bytes",
                Some(&source),
                ".storage-fault-stage",
            )
            .unwrap_err();
        assert_eq!(error.code, StorageErrorCode::Io, "{point}");
        assert_eq!(error.errno, Some(5));
        let after_publication = matches!(
            point,
            "after_publication" | "publication_sync" | "after_directory_sync"
        );
        if after_publication {
            assert_eq!(error.publication, Publication::Possible, "{point}");
            assert_eq!(common.read("source.yaml").unwrap().raw, b"new bytes");
            assert_eq!(
                common.read(".storage-fault-stage").unwrap().raw,
                b"old bytes"
            );
            assert_eq!(
                error.recovery_paths,
                vec![common.path.join(".storage-fault-stage")]
            );
        } else {
            assert_eq!(error.publication, Publication::NotPublished, "{point}");
            assert_eq!(common.read("source.yaml").unwrap().raw, b"old bytes");
        }
    }
}

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

#[test]
fn recovery_retries_failed_publication_stage_sync_before_rename() {
    for point in ["live_folders", "witness"] {
        let f = Fixture::new();
        let mut failure = None;
        let error = f
            .storage
            .initialize_inner(&mut |step| {
                if step == point {
                    failure = Some(files::fail_next("stage_sync"));
                }
                Ok(())
            })
            .unwrap_err();
        drop(failure);
        let id = error.operation_id.as_deref().unwrap();
        let stage = error.path.unwrap();
        let raw = fs::read(&stage).unwrap();
        for _ in 0..2 {
            let _guard = files::fail_next("retry_stage_sync");
            let retry = f
                .storage
                .recover(id, RecoverRequest::default())
                .unwrap_err();
            assert_eq!(retry.code, StorageErrorCode::Io);
            assert_eq!(retry.errno, Some(5));
            assert_eq!(retry.path.as_ref(), Some(&stage));
            assert_eq!(fs::read(&stage).unwrap(), raw);
            assert!(!f.storage.inspect().unwrap().coordination_available);
        }
        assert!(
            f.storage
                .recover(id, RecoverRequest::default())
                .unwrap()
                .storage
                .coordination_available
        );
    }
}

#[test]
fn recovery_retries_both_archive_parent_syncs_after_completed_rename() {
    for point in ["archive_source_sync", "archive_destination_sync"] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        fs::write(root.join("claims/opaque"), b"retained").unwrap();
        let error = {
            let _guard = files::fail_next(point);
            f.storage.recreate(f.request()).unwrap_err()
        };
        let id = error.operation_id.as_deref().unwrap();
        let archive = root.join("recovery").join(id).join("prior/claims/opaque");
        assert!(!root.join("claims").exists());
        assert_eq!(fs::read(&archive).unwrap(), b"retained");
        let request = || RecoverRequest {
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: false,
        };
        for again in ["archive_source_sync", "archive_destination_sync"] {
            let _guard = files::fail_next(again);
            let retry = f.storage.recover(id, request()).unwrap_err();
            assert_eq!(retry.code, StorageErrorCode::Io);
            assert_eq!(retry.errno, Some(5));
            assert_eq!(fs::read(&archive).unwrap(), b"retained");
            assert!(!root.join("claims").exists());
        }
        assert!(
            f.storage
                .recover(id, request())
                .unwrap()
                .storage
                .coordination_available
        );
    }
}

#[test]
fn incomplete_initialization_can_explicitly_recover_a_missing_lock() {
    let f = Fixture::new();
    let error = f
        .storage
        .initialize_inner(&mut |step| {
            if step == "prepared" {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    fs::remove_file(
        f.storage
            .project
            .git_common_dir
            .join("work/coordination.lock"),
    )
    .unwrap();
    let result = f
        .storage
        .recover(
            error.operation_id.as_deref().unwrap(),
            RecoverRequest {
                all_clients_stopped: true,
                ..RecoverRequest::default()
            },
        )
        .unwrap();
    assert!(result.storage.coordination_available);
}

#[test]
fn source_resync_refuses_fifo_and_replacement_after_validated_stat_without_hanging() {
    // Run the deliberate FIFO race in a killable child: a blocking-open regression
    // must fail promptly instead of hanging the entire test process and its lock.
    if std::env::var_os("WORK_STORAGE_REOPEN_CHILD").is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "core::storage::tests::source_resync_refuses_fifo_and_replacement_after_validated_stat_without_hanging", "--nocapture"])
            .env("WORK_STORAGE_REOPEN_CHILD", "1")
            .spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("source resync blocked on a substituted FIFO");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        return;
    }
    for kind in ["fifo", "replacement", "hardlink", "symlink"] {
        let f = Fixture::new();
        let outcome = f.storage.initialize().unwrap();
        let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
        let receipt = common
            .child("work")
            .unwrap()
            .child("operations")
            .unwrap()
            .child(outcome.operation_id.as_deref().unwrap())
            .unwrap();
        let operation = format::operation(
            &receipt.read("operation.yaml").unwrap().raw,
            &receipt.path.join("operation.yaml"),
        )
        .unwrap();
        let source = common.read("work.identity.yaml").unwrap();
        let path = common.path.join("work.identity.yaml");
        let retained = common.path.join("retained-witness");
        let target = common.path.join("unexpected");
        fs::write(&target, b"unexpected source").unwrap();
        let changed = path.clone();
        let kept = retained.clone();
        let other = target.clone();
        let _guard = files::on_nth("source_open", 2, move || {
            fs::rename(&changed, &kept).unwrap();
            match kind {
                "fifo" => rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &changed,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::from_bits_truncate(0o600),
                    0,
                )
                .unwrap(),
                "replacement" => fs::write(&changed, b"unexpected replacement").unwrap(),
                "hardlink" => fs::hard_link(&other, &changed).unwrap(),
                "symlink" => std::os::unix::fs::symlink(&other, &changed).unwrap(),
                _ => unreachable!(),
            }
        });
        // The first open is publish_source's matching-current read. Substitute
        // after the resync reopen's stat/type check, immediately before openat.
        let error = publish_source(
            &common,
            "work.identity.yaml",
            &source.raw,
            &None,
            &operation,
            &mut Vec::new(),
            &mut Publication::NotPublished,
        )
        .unwrap_err();
        assert!(
            matches!(
                error.code,
                StorageErrorCode::Conflict | StorageErrorCode::UnsafePath
            ),
            "{kind}: {error}"
        );
        assert_eq!(fs::read(&retained).unwrap(), source.raw);
        assert_eq!(fs::read(&target).unwrap(), b"unexpected source");
        if kind == "replacement" {
            assert_eq!(fs::read(&path).unwrap(), b"unexpected replacement");
        }
    }
}

#[test]
fn new_directory_permission_recovery_rejects_substituted_entry() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
    let held = common.create("new-private").unwrap();
    let target = common.create("unrelated").unwrap();
    fs::set_permissions(&target.path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&held.path, common.path.join("retained-private")).unwrap();
    std::os::unix::fs::symlink(&target.path, &held.path).unwrap();
    assert!(
        common
            .restore_new_directory_mode("new-private", &held.identity)
            .is_err()
    );
    assert_eq!(fs::metadata(&target.path).unwrap().mode() & 0o777, 0o755);
}

#[cfg(target_os = "linux")]
#[test]
fn new_directory_permission_recovery_chmods_only_pinned_inode_after_substitution() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
    let held = common.create("new-private").unwrap();
    let target = common.create("unrelated").unwrap();
    fs::set_permissions(&held.path, fs::Permissions::from_mode(0o0)).unwrap();
    fs::set_permissions(&target.path, fs::Permissions::from_mode(0o755)).unwrap();
    let path = held.path.clone();
    let retained = common.path.join("retained-private");
    let moved = retained.clone();
    let other = target.path.clone();
    let _guard = files::on_next("directory_mode", move || {
        fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
    });
    assert_eq!(
        common
            .restore_new_directory_mode("new-private", &held.identity)
            .unwrap_err()
            .code,
        StorageErrorCode::Conflict
    );
    assert_eq!(fs::metadata(&target.path).unwrap().mode() & 0o777, 0o755);
    assert_eq!(fs::metadata(&retained).unwrap().mode() & 0o777, 0o700);
}

#[test]
fn healthy_committed_recreation_resumes_by_id_but_damaged_terminal_structure_requires_fresh_generation()
 {
    for damage in ["healthy", "lock", "claims", "archive", "retained_claims"] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let error = f
            .storage
            .recreate_inner(f.request(), &mut |step| {
                if step == "committed" {
                    Err(injected())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let id = error.operation_id.as_deref().unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        let generation = f
            .storage
            .inspect()
            .unwrap()
            .metadata
            .unwrap()
            .recovery_generation;
        let request = || RecoverRequest {
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: true,
        };
        if damage == "healthy" {
            let before = fs::read(root.join("operations").join(id).join("operation.yaml")).unwrap();
            assert_eq!(
                f.storage.recreate(f.request()).unwrap_err().code,
                StorageErrorCode::RecoveryRequired
            );
            assert_eq!(
                fs::read(root.join("operations").join(id).join("operation.yaml")).unwrap(),
                before
            );
            let result = f.storage.recover(id, request()).unwrap();
            assert_eq!(
                result.storage.metadata.unwrap().recovery_generation,
                generation
            );
            continue;
        }
        let damaged = match damage {
            "lock" => root.join("coordination.lock"),
            "claims" => root.join("claims"),
            "archive" => root.join("recovery").join(id),
            "retained_claims" => root.join("recovery").join(id).join("prior/claims"),
            _ => unreachable!(),
        };
        if damage == "lock" {
            fs::remove_file(&damaged).unwrap();
        } else {
            fs::remove_dir_all(&damaged).unwrap();
        }
        assert_eq!(
            f.storage.recover(id, request()).unwrap_err().code,
            StorageErrorCode::StorageMissing
        );
        assert!(!damaged.exists(), "{damage}");
        let result = f.storage.recreate(f.request()).unwrap();
        assert!(result.storage.coordination_available, "{damage}");
        assert_ne!(
            result.storage.metadata.unwrap().recovery_generation,
            generation,
            "{damage}"
        );
        assert!(
            result
                .loss
                .unwrap()
                .prior_state_path
                .unwrap()
                .join("operations")
                .join(id)
                .join("operation.yaml")
                .is_file()
        );
    }
}

#[test]
fn committed_recreation_missing_terminal_metadata_or_witness_requires_explicit_fresh_generation() {
    for name in ["store.yaml", "work.identity.yaml"] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let error = f
            .storage
            .recreate_inner(f.request(), &mut |step| {
                if step == "committed" {
                    Err(injected())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let id = error.operation_id.as_deref().unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        let generation = f
            .storage
            .inspect()
            .unwrap()
            .metadata
            .unwrap()
            .recovery_generation;
        let damaged = if name == "store.yaml" {
            root.join(name)
        } else {
            f.storage.project.git_common_dir.join(name)
        };
        fs::remove_file(&damaged).unwrap();
        let request = RecoverRequest {
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: false,
        };
        let refusal = f.storage.recover(id, request).unwrap_err();
        assert!(matches!(
            refusal.code,
            StorageErrorCode::Conflict | StorageErrorCode::StorageMissing
        ));
        assert!(!damaged.exists());
        let result = f.storage.recreate(f.request()).unwrap();
        assert!(result.storage.coordination_available);
        assert_ne!(
            result.storage.metadata.unwrap().recovery_generation,
            generation
        );
        assert!(
            result
                .loss
                .unwrap()
                .prior_state_path
                .unwrap()
                .join("operations")
                .join(id)
                .join("operation.yaml")
                .is_file()
        );
    }
}

#[test]
fn terminal_replay_never_recreates_a_live_folder_deleted_after_structure_validation() {
    for kind in ["initialize", "recreate"] {
        for phase in ["committed", "complete"] {
            let f = Fixture::new();
            let mut stop = |step: &str| {
                if step == phase {
                    Err(injected())
                } else {
                    Ok(())
                }
            };
            let error = if kind == "initialize" {
                f.storage.initialize_inner(&mut stop).unwrap_err()
            } else {
                f.storage.initialize().unwrap();
                f.storage
                    .recreate_inner(f.request(), &mut stop)
                    .unwrap_err()
            };
            let id = error.operation_id.as_deref().unwrap();
            let root = f.storage.project.git_common_dir.join("work");
            let metadata = fs::read(root.join("store.yaml")).unwrap();
            let receipt =
                fs::read(root.join("operations").join(id).join("operation.yaml")).unwrap();
            let request = RecoverRequest {
                executors_stopped: true,
                acknowledge_loss: true,
                all_clients_stopped: false,
            };
            let error = f
                .storage
                .recover_inner(id, request, &mut |step| {
                    if step == "terminal_structure" {
                        fs::remove_dir(root.join("claims")).unwrap();
                    }
                    Ok(())
                })
                .unwrap_err();
            assert_eq!(
                error.code,
                StorageErrorCode::StorageMissing,
                "{kind}/{phase}"
            );
            assert_eq!(error.publication, Publication::NotPublished);
            assert!(!root.join("claims").exists());
            assert_eq!(fs::read(root.join("store.yaml")).unwrap(), metadata);
            assert_eq!(
                fs::read(root.join("operations").join(id).join("operation.yaml")).unwrap(),
                receipt
            );
            assert!(!f.storage.inspect().unwrap().coordination_available);
        }
    }
}

#[test]
fn malformed_pending_recreation_components_are_unsupported_and_retained_by_explicit_recreation() {
    for phase in ["prepared", "archived"] {
        for damaged in [
            "context.yaml",
            "store.yaml",
            "identity.yaml",
            "missing_context",
            "mismatched_stage",
        ] {
            let f = Fixture::new();
            f.storage.initialize().unwrap();
            let error = f
                .storage
                .recreate_inner(f.request(), &mut |step| {
                    if step == phase {
                        Err(injected())
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            let id = error.operation_id.as_deref().unwrap();
            let root = f.storage.project.git_common_dir.join("work");
            let dir = root.join("operations").join(id);
            let generation = f
                .storage
                .inspect()
                .unwrap()
                .metadata
                .unwrap()
                .recovery_generation;
            let name = match damaged {
                "missing_context" => "context.yaml",
                "mismatched_stage" => "store.yaml",
                name => name,
            };
            let path = dir.join(name);
            if damaged == "missing_context" {
                fs::remove_file(&path).unwrap();
            } else if damaged == "mismatched_stage" {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                value["recovery_generation"] =
                    serde_json::json!("00000000000040008000000000000000");
                fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            } else {
                fs::write(&path, b"malformed retained bytes\0\xff").unwrap();
            }
            let retained = fs::read(&path).ok();
            let header = fs::read(dir.join("operation.yaml")).unwrap();
            let status = f.storage.inspect().unwrap();
            assert!(!status.coordination_available);
            assert!(
                !status
                    .pending_operations
                    .iter()
                    .find(|op| op.id.as_deref() == Some(id))
                    .unwrap()
                    .supported
            );
            let request = RecoverRequest {
                executors_stopped: true,
                acknowledge_loss: true,
                all_clients_stopped: false,
            };
            assert!(f.storage.recover(id, request).is_err());
            let result = f.storage.recreate(f.request()).unwrap();
            assert!(result.storage.coordination_available, "{phase}/{damaged}");
            assert_ne!(
                result.storage.metadata.unwrap().recovery_generation,
                generation
            );
            let archived = result
                .loss
                .unwrap()
                .prior_state_path
                .unwrap()
                .join("operations")
                .join(id);
            assert_eq!(fs::read(archived.join("operation.yaml")).unwrap(), header);
            assert_eq!(fs::read(archived.join(name)).ok(), retained);
        }
    }
}

#[test]
fn pending_intent_guard_refuses_unreadable_or_unsafe_components_without_archival() {
    use std::os::unix::fs::PermissionsExt;
    for kind in ["permissions", "symlink", "fifo"] {
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
        let root = f.storage.project.git_common_dir.join("work");
        let dir = root.join("operations").join(id);
        let context = dir.join("context.yaml");
        let header = fs::read(dir.join("operation.yaml")).unwrap();
        match kind {
            "permissions" => {
                fs::set_permissions(&context, fs::Permissions::from_mode(0o000)).unwrap()
            }
            "symlink" => {
                fs::remove_file(&context).unwrap();
                std::os::unix::fs::symlink(root.join("store.yaml"), &context).unwrap();
            }
            "fifo" => {
                fs::remove_file(&context).unwrap();
                rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &context,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::from_bits_truncate(0o600),
                    0,
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let error = f.storage.recreate(f.request()).unwrap_err();
        assert_eq!(
            error.code,
            if kind == "permissions" {
                StorageErrorCode::PermissionDenied
            } else {
                StorageErrorCode::UnsafePath
            }
        );
        assert_eq!(fs::read(dir.join("operation.yaml")).unwrap(), header);
        assert!(root.join("claims").is_dir());
        if kind == "permissions" {
            fs::set_permissions(&context, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

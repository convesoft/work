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
        "missing_health",
        "wrong_health",
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
            "missing_health" => {
                value
                    .as_object_mut()
                    .unwrap()
                    .remove("prior_missing_or_damaged");
            }
            "wrong_health" => {
                value["prior_missing_or_damaged"] = serde_json::json!("false");
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

#[test]
fn completed_receipt_recovery_retries_file_and_operation_directory_sync_without_republication() {
    use std::os::unix::fs::MetadataExt;
    for kind in ["initialize", "recreate"] {
        let f = Fixture::new();
        if kind == "recreate" {
            f.storage.initialize().unwrap();
        }
        let mut failure = None;
        let mut stop = |step: &str| {
            if step == "committed" {
                failure = Some(files::fail_next("publication_sync"));
            }
            Ok(())
        };
        let error = if kind == "initialize" {
            f.storage.initialize_inner(&mut stop).unwrap_err()
        } else {
            f.storage
                .recreate_inner(f.request(), &mut stop)
                .unwrap_err()
        };
        drop(failure);
        let id = error.operation_id.as_deref().unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        let dir = root.join("operations").join(id);
        let path = dir.join("operation.yaml");
        let receipt = fs::read(&path).unwrap();
        assert_eq!(
            format::operation(&receipt, &path).unwrap().phase,
            "complete"
        );
        let inode = fs::metadata(&path).unwrap().ino();
        let entries = fs::read_dir(&dir).unwrap().count();
        let metadata = fs::read(root.join("store.yaml")).unwrap();
        let request = || RecoverRequest {
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: false,
        };
        for point in [
            "receipt_file_sync",
            "receipt_file_sync",
            "receipt_directory_sync",
            "receipt_directory_sync",
        ] {
            let _guard = files::fail_next(point);
            let refusal = f.storage.recover(id, request()).unwrap_err();
            assert_eq!(refusal.code, StorageErrorCode::Io, "{kind}/{point}");
            assert_eq!(refusal.errno, Some(5));
            assert_eq!(
                refusal.path.as_ref(),
                Some(if point == "receipt_file_sync" {
                    &path
                } else {
                    &dir
                })
            );
            assert_eq!(fs::read(&path).unwrap(), receipt);
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
            assert_eq!(fs::read_dir(&dir).unwrap().count(), entries);
            assert_eq!(fs::read(root.join("store.yaml")).unwrap(), metadata);
        }
        let result = f.storage.recover(id, request()).unwrap();
        assert!(result.storage.coordination_available);
        assert_eq!(result.operation_id.as_deref(), Some(id));
        assert_eq!(fs::read(&path).unwrap(), receipt);
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(fs::read(root.join("store.yaml")).unwrap(), metadata);
    }
}

#[test]
fn storage_initializer_child() {
    let Some(path) = std::env::var_os("WORK_INIT_RACE_CHECKOUT") else {
        return;
    };
    use std::os::unix::fs::MetadataExt;
    let path = PathBuf::from(path);
    let storage = Storage::new(crate::core::project::discover(Some(&path)).unwrap());
    let result = storage.initialize().unwrap();
    assert!(result.changed);
    let metadata = result.storage.metadata.unwrap();
    let root = fs::metadata(&result.storage.path).unwrap();
    let lock = fs::metadata(&result.storage.lock_path).unwrap();
    fs::write(
        path.join("winner-state.json"),
        serde_json::to_vec(&serde_json::json!({
            "store_id":metadata.store_id,"generation":metadata.recovery_generation,
            "root_dev":root.dev(),"root_ino":root.ino(),"lock_dev":lock.dev(),"lock_ino":lock.ino()
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn independent_initializer_winning_absence_to_root_create_race_is_rechecked_under_existing_lock() {
    use std::os::unix::fs::MetadataExt;
    let mut f = Fixture::new();
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&f.root)
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success());
    f.storage = Storage::new(crate::core::project::discover(Some(&f.root)).unwrap());
    let result = f
        .storage
        .initialize_inner(&mut |step| {
            if step == "before_root_create" {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "core::storage::tests::storage_initializer_child",
                        "--nocapture",
                    ])
                    .env("WORK_INIT_RACE_CHECKOUT", &f.root)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
        .unwrap();
    assert!(!result.changed);
    assert!(result.operation_id.is_none());
    let expected: serde_json::Value =
        serde_json::from_slice(&fs::read(f.root.join("winner-state.json")).unwrap()).unwrap();
    let metadata = result.storage.metadata.unwrap();
    assert_eq!(expected["store_id"], metadata.store_id);
    assert_eq!(expected["generation"], metadata.recovery_generation);
    let root = fs::metadata(&result.storage.path).unwrap();
    let lock = fs::metadata(&result.storage.lock_path).unwrap();
    assert_eq!(expected["root_dev"], root.dev());
    assert_eq!(expected["root_ino"], root.ino());
    assert_eq!(expected["lock_dev"], lock.dev());
    assert_eq!(expected["lock_ino"], lock.ino());
    assert_eq!(
        fs::read_dir(result.storage.path.join("operations"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn root_create_race_never_adopts_unlocked_remnants_or_unsafe_replacements() {
    for kind in ["remnant", "symlink"] {
        let f = Fixture::new();
        let root = f.storage.project.git_common_dir.join("work");
        let result = f
            .storage
            .initialize_inner(&mut |step| {
                if step == "before_root_create" {
                    if kind == "remnant" {
                        fs::create_dir(&root).unwrap();
                    } else {
                        std::os::unix::fs::symlink(&f.root, &root).unwrap();
                    }
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(
            result.code,
            if kind == "remnant" {
                StorageErrorCode::RecoveryRequired
            } else {
                StorageErrorCode::UnsafePath
            }
        );
        assert!(!root.join("coordination.lock").exists());
        assert!(
            !f.storage
                .project
                .git_common_dir
                .join("work.identity.yaml")
                .exists()
        );
    }
}

#[test]
fn unchanged_intended_witness_rejects_same_bytes_with_changed_fingerprint_at_both_boundaries() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for point in ["prepared", "live_folders"] {
        for change in ["rewrite", "inode", "mode"] {
            let f = Fixture::new();
            f.storage.initialize().unwrap();
            let common = f.storage.project.git_common_dir.clone();
            let witness = common.join("work.identity.yaml");
            let raw = fs::read(&witness).unwrap();
            let prior = files::Directory::open(&common)
                .unwrap()
                .read("work.identity.yaml")
                .unwrap();
            let metadata = fs::read(common.join("work/store.yaml")).unwrap();
            let mut changed = None;
            let error = f
                .storage
                .recreate_inner(f.request(), &mut |step| {
                    if step == point {
                        match change {
                            "rewrite" => {
                                fs::write(&witness, &raw).unwrap();
                                // A known timestamp change makes this case independent
                                // of timestamp resolution or scheduler timing.
                                let times = rustix::fs::Timestamps {
                                    last_access: rustix::fs::Timespec {
                                        tv_sec: prior.mtime.0 - 1,
                                        tv_nsec: 0,
                                    },
                                    last_modification: rustix::fs::Timespec {
                                        tv_sec: prior.mtime.0 - 1,
                                        tv_nsec: 0,
                                    },
                                };
                                rustix::fs::utimensat(
                                    rustix::fs::CWD,
                                    &witness,
                                    &times,
                                    rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
                                )
                                .unwrap();
                            }
                            "inode" => {
                                let replacement = common.join("replacement-witness");
                                fs::write(&replacement, &raw).unwrap();
                                fs::set_permissions(
                                    &replacement,
                                    fs::Permissions::from_mode(0o600),
                                )
                                .unwrap();
                                fs::rename(&replacement, &witness).unwrap();
                            }
                            "mode" => {
                                fs::set_permissions(&witness, fs::Permissions::from_mode(0o640))
                                    .unwrap()
                            }
                            _ => unreachable!(),
                        }
                        changed = Some(
                            files::Directory::open(&common)
                                .unwrap()
                                .read("work.identity.yaml")
                                .unwrap(),
                        );
                    }
                    Ok(())
                })
                .unwrap_err();
            assert_eq!(error.code, StorageErrorCode::Conflict, "{point}/{change}");
            assert_eq!(error.path.as_ref(), Some(&witness));
            let unexpected = changed.unwrap();
            assert_ne!(unexpected, prior);
            assert_eq!(
                files::Directory::open(&common)
                    .unwrap()
                    .read("work.identity.yaml")
                    .unwrap(),
                unexpected
            );
            assert_eq!(fs::read(&witness).unwrap(), raw);
            assert_eq!(fs::read(common.join("work/store.yaml")).unwrap(), metadata);
            if change == "inode" {
                assert_ne!(fs::metadata(&witness).unwrap().ino(), prior.identity.ino);
            }
            assert!(!f.storage.inspect().unwrap().coordination_available);
        }
    }
}

#[test]
fn retained_exchange_fingerprint_checks_mtime_on_initial_and_existing_stage_publication() {
    use std::fs::FileTimes;
    use std::time::{Duration, UNIX_EPOCH};
    for retry in [false, true] {
        for tamper in [false, true] {
            let f = Fixture::new();
            let initialized = f.storage.initialize().unwrap();
            let id = initialized.operation_id.unwrap();
            let receipt = f
                .storage
                .project
                .git_common_dir
                .join("work/operations")
                .join(&id);
            let operation =
                format::operation(&fs::read(receipt.join("operation.yaml")).unwrap(), &receipt)
                    .unwrap();
            let dir = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
            dir.stage("store.yaml", b"old bytes").unwrap();
            let before = dir.read("store.yaml").unwrap();
            let stage = format!(".storage-{id}-store");
            if retry {
                dir.stage(&stage, b"new bytes").unwrap();
            }
            let retained = dir.path.join(&stage);
            let changed = retained.clone();
            let old = before.raw.clone();
            let mtime = before.mtime.0;
            let _guard = tamper.then(|| {
                files::on_next(
                    if retry {
                        "retry_after_publication"
                    } else {
                        "after_publication"
                    },
                    move || {
                        fs::write(&changed, old).unwrap();
                        let file = fs::OpenOptions::new().write(true).open(changed).unwrap();
                        file.set_times(
                            FileTimes::new()
                                .set_modified(UNIX_EPOCH + Duration::from_secs(mtime as u64 + 10)),
                        )
                        .unwrap();
                        file.sync_all().unwrap();
                    },
                )
            });
            let mut publication = Publication::NotPublished;
            let result = if retry {
                publish_source(
                    &dir,
                    "store.yaml",
                    b"new bytes",
                    &Some(before.clone()),
                    &operation,
                    &mut Vec::new(),
                    &mut publication,
                )
            } else {
                dir.publish("store.yaml", b"new bytes", Some(&before), &stage)
                    .map(|_| ())
            };
            if tamper {
                assert_eq!(
                    result.unwrap_err().code,
                    StorageErrorCode::Conflict,
                    "retry={retry}"
                );
                assert_ne!(dir.read(&stage).unwrap().mtime, before.mtime);
            } else {
                result.unwrap();
                assert!(before.same_content_identity(&dir.read(&stage).unwrap()));
            }
            assert_eq!(fs::read(retained).unwrap(), b"old bytes");
            assert_eq!(dir.read("store.yaml").unwrap().raw, b"new bytes");
        }
    }
}

#[test]
fn interrupted_exchange_requires_intact_retained_source_before_successful_retry() {
    for damage in ["missing", "bytes", "substitute", "mtime", "healthy"] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let error = f
            .storage
            .recreate_inner(f.request(), &mut |step| {
                if step == "metadata" {
                    Err(injected())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let id = error.operation_id.as_deref().unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        let stage = root.join(format!(".storage-{id}-store"));
        let current = fs::read(root.join("store.yaml")).unwrap();
        let old = fs::read(&stage).unwrap();
        match damage {
            "missing" => fs::remove_file(&stage).unwrap(),
            "bytes" => fs::write(&stage, b"unexpected retained bytes").unwrap(),
            "substitute" => {
                fs::rename(&stage, root.join("held-prior-source")).unwrap();
                fs::write(&stage, &old).unwrap();
            }
            "mtime" => {
                let source = files::Directory::open(&root)
                    .unwrap()
                    .read(stage.file_name().unwrap().to_str().unwrap())
                    .unwrap();
                fs::File::open(&stage)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(
                        std::time::UNIX_EPOCH
                            + std::time::Duration::from_secs(source.mtime.0 as u64 + 10),
                    ))
                    .unwrap();
            }
            "healthy" => {}
            _ => unreachable!(),
        }
        let retained_after_damage = fs::read(&stage).ok();
        let result = f.storage.recover(
            id,
            RecoverRequest {
                executors_stopped: true,
                acknowledge_loss: true,
                all_clients_stopped: false,
            },
        );
        if damage == "healthy" {
            assert!(result.unwrap().storage.coordination_available);
        } else {
            assert!(
                matches!(
                    result.unwrap_err().code,
                    StorageErrorCode::StorageMissing | StorageErrorCode::Conflict
                ),
                "{damage}"
            );
            assert!(!f.storage.inspect().unwrap().coordination_available);
        }
        assert_eq!(fs::read(root.join("store.yaml")).unwrap(), current);
        assert_eq!(fs::read(&stage).ok(), retained_after_damage);
    }
}

#[test]
fn prior_health_is_persisted_before_repair_and_returned_on_same_id_recovery() {
    for damage in [
        "healthy",
        "root",
        "lock",
        "operations",
        "recovery",
        "archive",
        "receipt",
        "live",
        "metadata",
    ] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let earlier = f.storage.recreate(f.request()).unwrap();
        assert!(!earlier.loss.unwrap().missing_or_damaged);
        let root = f.storage.project.git_common_dir.join("work");
        let earlier_id = earlier.operation_id.unwrap();
        match damage {
            "healthy" => {}
            "root" => fs::remove_dir_all(&root).unwrap(),
            "lock" => fs::remove_file(root.join("coordination.lock")).unwrap(),
            "operations" | "recovery" | "live" => {
                fs::remove_dir_all(root.join(if damage == "live" { "claims" } else { damage }))
                    .unwrap()
            }
            "archive" => {
                fs::remove_dir_all(root.join("recovery").join(&earlier_id).join("prior/claims"))
                    .unwrap()
            }
            "receipt" => fs::write(
                root.join("operations")
                    .join(&earlier_id)
                    .join("operation.yaml"),
                b"invalid receipt",
            )
            .unwrap(),
            "metadata" => fs::remove_file(root.join("store.yaml")).unwrap(),
            _ => unreachable!(),
        }
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
        let context_path = root.join("operations").join(id).join("context.yaml");
        let raw = fs::read(&context_path).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(
            value["prior_missing_or_damaged"],
            damage != "healthy",
            "{damage}"
        );
        let request = RecoverRequest {
            executors_stopped: true,
            acknowledge_loss: true,
            all_clients_stopped: true,
        };
        let result = f.storage.recover(id, request.clone()).unwrap();
        assert_eq!(
            result.loss.unwrap().missing_or_damaged,
            damage != "healthy",
            "{damage}"
        );
        assert_eq!(
            f.storage
                .recover(id, request)
                .unwrap()
                .loss
                .unwrap()
                .missing_or_damaged,
            damage != "healthy",
            "{damage}"
        );
        assert_eq!(fs::read(context_path).unwrap(), raw);
    }
}

#[test]
fn initialization_context_requires_false_prior_health() {
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
    let id = error.operation_id.as_deref().unwrap();
    let dir = f
        .storage
        .project
        .git_common_dir
        .join("work/operations")
        .join(id);
    let op = format::operation(&fs::read(dir.join("operation.yaml")).unwrap(), &dir).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("context.yaml")).unwrap()).unwrap();
    value["prior_missing_or_damaged"] = serde_json::json!(true);
    assert_eq!(
        format::context(&format::bytes(&value), &op, &dir)
            .unwrap_err()
            .code,
        StorageErrorCode::InvalidFormat
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn permission_recovery_never_chmods_a_substituted_real_directory() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    let common = files::Directory::open(&f.storage.project.git_common_dir).unwrap();
    let original = common.create("new-private").unwrap();
    fs::rename(&original.path, common.path.join("retained-private")).unwrap();
    fs::create_dir(&original.path).unwrap();
    fs::set_permissions(&original.path, fs::Permissions::from_mode(0o755)).unwrap();
    let error = common
        .restore_new_directory_mode("new-private", &original.identity)
        .unwrap_err();
    assert_eq!(error.code, StorageErrorCode::PermissionDenied);
    assert_eq!(fs::metadata(&original.path).unwrap().mode() & 0o777, 0o755);
}

#[test]
fn prior_health_retains_incomplete_supported_initialization_as_damage() {
    let f = Fixture::new();
    let incomplete = f
        .storage
        .initialize_inner(&mut |step| {
            if step == "prepared" {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let before = f.storage.inspect().unwrap();
    assert!(
        before
            .pending_operations
            .iter()
            .any(|op| op.supported && op.kind.as_deref() == Some("initialize"))
    );
    let outcome = f.storage.recreate(f.request()).unwrap();
    assert!(outcome.loss.as_ref().unwrap().missing_or_damaged);
    let id = outcome.operation_id.unwrap();
    let archived = outcome
        .loss
        .unwrap()
        .prior_state_path
        .unwrap()
        .join("operations")
        .join(incomplete.operation_id.unwrap());
    assert!(archived.join("operation.yaml").is_file());
    let recovered = f
        .storage
        .recover(
            &id,
            RecoverRequest {
                executors_stopped: true,
                acknowledge_loss: true,
                all_clients_stopped: false,
            },
        )
        .unwrap();
    assert!(recovered.loss.unwrap().missing_or_damaged);
}

#[test]
fn matching_archive_uncertainty_refuses_reset_but_missing_archive_can_be_acknowledged() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for damage in ["unsafe", "unreadable", "conflict", "missing"] {
        let f = Fixture::new();
        f.storage.initialize().unwrap();
        let previous = f.storage.recreate(f.request()).unwrap();
        let root = f.storage.project.git_common_dir.join("work");
        let archive = previous.loss.unwrap().prior_state_path.unwrap();
        let path = archive.join("prior/claims");
        let retained = archive.join("held-claims");
        fs::write(path.join("opaque"), b"retained claim bytes").unwrap();
        match damage {
            "unsafe" => {
                fs::rename(&path, &retained).unwrap();
                std::os::unix::fs::symlink(&retained, &path).unwrap();
            }
            "unreadable" => fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap(),
            "conflict" => {
                fs::rename(&path, &retained).unwrap();
                fs::create_dir(&path).unwrap();
                fs::write(path.join("replacement"), b"unrelated bytes").unwrap();
            }
            "missing" => fs::remove_dir_all(&path).unwrap(),
            _ => unreachable!(),
        }
        let operations_before = files::Directory::open(&root.join("operations"))
            .unwrap()
            .names()
            .unwrap();
        let metadata = fs::read(root.join("store.yaml")).unwrap();
        let lock = fs::metadata(root.join("coordination.lock")).unwrap();
        let result = f.storage.recreate(f.request());
        if damage == "missing" {
            assert!(result.unwrap().loss.unwrap().missing_or_damaged);
            assert!(!path.exists());
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.publication, Publication::NotPublished);
            assert_eq!(
                error.code,
                match damage {
                    "unsafe" => StorageErrorCode::UnsafePath,
                    "unreadable" => StorageErrorCode::PermissionDenied,
                    "conflict" => StorageErrorCode::Conflict,
                    _ => unreachable!(),
                }
            );
            assert_eq!(error.path, Some(path.clone()));
            if damage == "unreadable" {
                assert_eq!(error.errno, Some(13));
                assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0);
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            assert_eq!(fs::read(root.join("store.yaml")).unwrap(), metadata);
            assert_eq!(
                files::Directory::open(&root.join("operations"))
                    .unwrap()
                    .names()
                    .unwrap(),
                operations_before
            );
            let opaque = if damage == "unreadable" {
                path.join("opaque")
            } else {
                retained.join("opaque")
            };
            assert_eq!(fs::read(opaque).unwrap(), b"retained claim bytes");
        }
        let after = fs::metadata(root.join("coordination.lock")).unwrap();
        assert_eq!((after.dev(), after.ino()), (lock.dev(), lock.ino()));
    }
}

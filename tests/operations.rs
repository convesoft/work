use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use work::core::items::{Completion, ItemStore, ManualState};
use work::core::operations::{DurableOperations, MetadataChange, OperationError, RelationKind};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id(n: u32) -> String {
    format!("{n:08x}000040008000000000000000")
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-operations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join(".work/items")).unwrap();
        Self(path)
    }
    fn ops(&self) -> DurableOperations {
        DurableOperations::new(&self.0)
    }
    fn path(&self, id: &str) -> PathBuf {
        self.0.join(".work/items").join(format!("{id}.md"))
    }
    fn write(&self, n: u32, extra: &str, body: &[u8]) {
        let id = id(n);
        let mut raw = format!(
            "---\nformat_version: 1\nid: \"{id}\"\ntitle: Item {n}\nstate: open\n{extra}---\n"
        )
        .into_bytes();
        raw.extend_from_slice(body);
        fs::write(self.path(&id), raw).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn creates_updates_and_preserves_opaque_body_bytes() {
    let f = Fixture::new();
    let ops = f.ops();
    let body = b"\r\n## Arbitrary\r\n- [ ] keep verbatim".to_vec();
    let created = ops
        .create("New item".into(), body.clone(), MetadataChange::default())
        .unwrap();
    let id = created.file.header.as_ref().unwrap().id.clone();
    assert_eq!(id.len(), 32);
    assert_eq!(id.as_bytes()[12], b'4');
    assert!(matches!(id.as_bytes()[16], b'8' | b'9' | b'a' | b'b'));
    assert_eq!(created.file.body.as_deref(), Some(body.as_slice()));
    let changed = ops
        .update(
            &id,
            MetadataChange {
                title: Some("Updated".into()),
                priority: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(changed.file.body.as_deref(), Some(body.as_slice()));
    assert_eq!(changed.file.header.unwrap().priority, 0);
    assert_eq!(ops.list().unwrap().len(), 1);
    assert!(ItemStore::load_from_root(&f.0).unwrap().is_valid());
}

#[test]
fn replacement_retains_source_for_writes_through_an_open_handle() {
    let f = Fixture::new();
    f.write(1, "", b"Original body");
    let path = f.path(&id(1));
    let old_bytes = fs::read(&path).unwrap();
    let mut open_old = fs::OpenOptions::new().append(true).open(&path).unwrap();
    let result = f
        .ops()
        .update(
            &id(1),
            MetadataChange {
                title: Some("Updated".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let recovery = result.recovery_path.unwrap();
    open_old.write_all(b" later edit").unwrap();
    open_old.sync_all().unwrap();
    let mut expected = old_bytes;
    expected.extend_from_slice(b" later edit");
    assert_eq!(fs::read(&recovery).unwrap(), expected);
    assert!(!fs::read(&path).unwrap().ends_with(b" later edit"));
    assert!(ItemStore::load_from_root(&f.0).unwrap().is_valid());
}

#[test]
fn relations_close_and_reopen_recompute_without_cascading() {
    let f = Fixture::new();
    f.write(1, "", b"A");
    f.write(2, "", b"B");
    let ops = f.ops();
    ops.relation_add(&id(2), RelationKind::DependsOn, &id(1))
        .unwrap();
    let before = ops.inspect(&id(2)).unwrap();
    assert!(!before.evaluation.unwrap().executable);
    assert_eq!(before.relations.depends_on, vec![id(1)]);
    assert_eq!(ops.inspect(&id(1)).unwrap().relations.blocks, vec![id(2)]);
    ops.close(&id(1), Some("opaque: cancelled".into())).unwrap();
    assert!(ops.inspect(&id(2)).unwrap().evaluation.unwrap().executable);
    ops.close(&id(2), None).unwrap();
    ops.reopen(&id(1)).unwrap();
    assert_eq!(
        ops.inspect(&id(2)).unwrap().file.header.unwrap().state,
        Some(ManualState::Done)
    );
    assert_eq!(
        ops.inspect(&id(1))
            .unwrap()
            .file
            .header
            .unwrap()
            .close_reason,
        None
    );
    ops.relation_remove(&id(2), RelationKind::DependsOn, &id(1))
        .unwrap();
    assert!(ops.inspect(&id(2)).unwrap().relations.depends_on.is_empty());
}

#[test]
fn symmetric_related_is_stored_once_and_removable_from_either_end() {
    let f = Fixture::new();
    f.write(1, "", b"A");
    f.write(2, "", b"B");
    let ops = f.ops();
    ops.relation_add(&id(1), RelationKind::Related, &id(2))
        .unwrap();
    assert_eq!(ops.inspect(&id(2)).unwrap().relations.related, vec![id(1)]);
    assert!(matches!(
        ops.relation_add(&id(2), RelationKind::Related, &id(1)),
        Err(OperationError::AlreadyExists(_))
    ));
    let stored = fs::read_to_string(f.path(&id(2))).unwrap();
    assert!(!stored.contains("related:"));
    ops.relation_remove(&id(2), RelationKind::Related, &id(1))
        .unwrap();
    assert!(ops.inspect(&id(1)).unwrap().relations.related.is_empty());
}

#[test]
fn rejected_cycle_and_bad_metadata_preserve_original_files() {
    let f = Fixture::new();
    f.write(1, "", b"Body one");
    f.write(2, "", b"Body two");
    let ops = f.ops();
    ops.relation_add(&id(2), RelationKind::DependsOn, &id(1))
        .unwrap();
    let originals = [
        fs::read(f.path(&id(1))).unwrap(),
        fs::read(f.path(&id(2))).unwrap(),
    ];
    assert!(matches!(
        ops.relation_add(&id(1), RelationKind::DependsOn, &id(2)),
        Err(OperationError::InvalidCandidate(_))
    ));
    assert!(matches!(
        ops.update(
            &id(1),
            MetadataChange {
                title: Some("\ninvalid".into()),
                ..Default::default()
            }
        ),
        Err(OperationError::InvalidArgument(_))
    ));
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), originals[0]);
    assert_eq!(fs::read(f.path(&id(2))).unwrap(), originals[1]);
}

#[test]
fn invalid_graph_can_be_inspected_and_repaired_explicitly() {
    let f = Fixture::new();
    f.write(
        1,
        "depends_on: [\"00000003000040008000000000000000\"]\n",
        b"Body",
    );
    let ops = f.ops();
    assert!(matches!(
        ops.close(&id(1), None),
        Err(OperationError::InvalidSource(_))
    ));
    assert_eq!(
        ops.inspect_raw(&id(1)).unwrap().file.body.as_deref(),
        Some(&b"Body"[..])
    );
    assert!(
        !ops.inspect_raw(&id(1))
            .unwrap()
            .graph_diagnostics
            .is_empty()
    );
    let repaired = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Repaired\nstate: open\n---\nBody",
        id(1)
    );
    ops.repair(&id(1), repaired.into_bytes()).unwrap();
    assert!(ops.inspect(&id(1)).unwrap().graph_diagnostics.is_empty());
}

#[test]
fn repair_rejects_a_healthy_item() {
    let f = Fixture::new();
    f.write(1, "", b"Body");
    let ops = f.ops();
    let original = fs::read(f.path(&id(1))).unwrap();
    let replacement = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Invalid graph\nstate: open\ndepends_on: [\"{}\"]\n---\nBody",
        id(1),
        id(2)
    );
    assert!(matches!(
        ops.repair(&id(1), replacement.into_bytes()),
        Err(OperationError::InvalidArgument(_))
    ));
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), original);
}

#[test]
fn repair_cannot_add_graph_errors_to_an_invalid_view() {
    let f = Fixture::new();
    f.write(1, &format!("depends_on: [\"{}\"]\n", id(3)), b"Body");
    let ops = f.ops();
    let original = fs::read(f.path(&id(1))).unwrap();
    let replacement = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Different bad edge\nstate: open\ndepends_on: [\"{}\"]\n---\nBody",
        id(1),
        id(4)
    );
    assert!(matches!(
        ops.repair(&id(1), replacement.into_bytes()),
        Err(OperationError::InvalidCandidate(_))
    ));
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), original);
}

#[test]
fn invalid_supplied_fields_return_invalid_argument_without_writes() {
    let f = Fixture::new();
    f.write(1, "", b"Body");
    let ops = f.ops();
    let original = fs::read(f.path(&id(1))).unwrap();
    assert!(matches!(
        ops.update(
            &id(1),
            MetadataChange {
                priority: Some(5),
                ..Default::default()
            }
        ),
        Err(OperationError::InvalidArgument(_))
    ));
    assert!(matches!(
        ops.close(&id(1), Some(" ".into())),
        Err(OperationError::InvalidArgument(_))
    ));
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), original);
}

#[test]
fn replacement_preserves_restrictive_file_mode() {
    let f = Fixture::new();
    f.write(1, "", b"private body");
    let path = f.path(&id(1));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    f.ops()
        .update(
            &id(1),
            MetadataChange {
                title: Some("Updated".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn replacement_preserves_special_permission_bits() {
    let f = Fixture::new();
    f.write(1, "", b"Body");
    let path = f.path(&id(1));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o2750)).unwrap();
    f.ops()
        .update(
            &id(1),
            MetadataChange {
                title: Some("Updated".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o7777,
        0o2750
    );
}

#[test]
fn list_skips_malformed_files_while_raw_inspection_remains_available() {
    let f = Fixture::new();
    f.write(1, "", b"Valid");
    f.write(2, "", b"Invalid filename");
    let wrong = f.0.join(".work/items/wrong.md");
    fs::rename(f.path(&id(2)), &wrong).unwrap();
    let ops = f.ops();
    let listed = ops.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].file.header.as_ref().unwrap().id, id(1));
    let raw = ops.inspect_raw(&id(2)).unwrap();
    assert_eq!(raw.file.path, wrong);
    assert!(!raw.file.diagnostics.is_empty());
    assert!(!ItemStore::load_from_root(&f.0).unwrap().is_valid());
}

#[test]
fn repair_canonicalizes_a_misnamed_item_without_discarding_source() {
    let f = Fixture::new();
    f.write(1, "", b"Body");
    let wrong = f.0.join(".work/items/wrong.md");
    fs::rename(f.path(&id(1)), &wrong).unwrap();
    let original = fs::read(&wrong).unwrap();
    let repaired = f.ops().repair(&id(1), original.clone()).unwrap();
    assert_eq!(repaired.file.path, f.path(&id(1)));
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), original);
    assert!(!wrong.exists());
    assert!(ItemStore::load_from_root(&f.0).unwrap().is_valid());
    assert!(
        fs::read_dir(f.0.join(".work/items"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".operation-"))
    );
}

#[test]
fn interrupted_filename_repair_can_be_inspected_and_resumed() {
    let f = Fixture::new();
    f.write(1, "", b"Old body");
    let wrong = f.0.join(".work/items/wrong.md");
    fs::rename(f.path(&id(1)), &wrong).unwrap();
    let canonical = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Repaired\nstate: open\n---\nNew body",
        id(1)
    )
    .into_bytes();
    fs::write(f.path(&id(1)), &canonical).unwrap();
    let ops = f.ops();
    assert_eq!(ops.inspect_raw(&id(1)).unwrap().file.path, wrong);
    assert!(matches!(
        ops.repair(&id(1), fs::read(&wrong).unwrap()),
        Err(OperationError::Conflict(_))
    ));
    assert!(wrong.exists());
    let result = ops.repair(&id(1), canonical.clone()).unwrap();
    assert_eq!(result.file.path, f.path(&id(1)));
    assert!(result.graph_diagnostics.is_empty());
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), canonical);
    assert!(!wrong.exists());
    assert!(ItemStore::load_from_root(&f.0).unwrap().is_valid());
}

#[test]
fn repair_rejects_identity_rewrite_with_another_claim() {
    let f = Fixture::new();
    f.write(1, "", b"Body one");
    let wrong = f.0.join(".work/items/wrong.md");
    fs::rename(f.path(&id(1)), &wrong).unwrap();
    f.write(2, "", b"Body two");
    fs::rename(f.path(&id(2)), f.path(&id(1))).unwrap();
    let original_canonical = fs::read(f.path(&id(1))).unwrap();
    let original_wrong = fs::read(&wrong).unwrap();
    assert!(matches!(
        f.ops().repair(&id(1), original_wrong.clone()),
        Err(OperationError::InvalidArgument(_))
    ));
    assert_eq!(fs::read(f.path(&id(1))).unwrap(), original_canonical);
    assert_eq!(fs::read(wrong).unwrap(), original_wrong);
}

#[test]
fn repair_uses_the_parsed_identity_for_a_misnamed_source() {
    let f = Fixture::new();
    f.write(2, "", b"Body");
    let misplaced = f.path(&id(1));
    fs::rename(f.path(&id(2)), &misplaced).unwrap();
    let raw = fs::read(&misplaced).unwrap();
    let replacement_for_one = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Wrong identity\nstate: open\n---\nBody",
        id(1)
    );
    assert!(matches!(
        f.ops().repair(&id(1), replacement_for_one.into_bytes()),
        Err(OperationError::InvalidArgument(_))
    ));
    assert_eq!(fs::read(&misplaced).unwrap(), raw);
    let repaired = f.ops().repair(&id(2), raw.clone()).unwrap();
    assert_eq!(repaired.file.path, f.path(&id(2)));
    assert_eq!(fs::read(f.path(&id(2))).unwrap(), raw);
    assert!(!misplaced.exists());
}

#[test]
fn repair_replaces_a_symlink_without_following_it() {
    let f = Fixture::new();
    let outside = f.0.join("outside.md");
    fs::write(&outside, b"outside").unwrap();
    std::os::unix::fs::symlink(&outside, f.path(&id(1))).unwrap();
    let replacement = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Repaired\nstate: open\n---\nBody",
        id(1)
    );
    let repaired = f.ops().repair(&id(1), replacement.into_bytes()).unwrap();
    assert!(repaired.graph_diagnostics.is_empty());
    assert!(
        fs::symlink_metadata(f.path(&id(1)))
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(fs::read(outside).unwrap(), b"outside");
}

#[test]
fn repair_returns_remaining_graph_diagnostics() {
    let f = Fixture::new();
    f.write(1, &format!("depends_on: [\"{}\"]\n", id(3)), b"One");
    f.write(2, &format!("depends_on: [\"{}\"]\n", id(4)), b"Two");
    let replacement = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Repaired\nstate: open\n---\nOne",
        id(1)
    );
    let repaired = f.ops().repair(&id(1), replacement.into_bytes()).unwrap();
    assert_eq!(repaired.file.header.unwrap().id, id(1));
    assert_eq!(repaired.graph_diagnostics.len(), 1);
    assert!(repaired.graph_diagnostics[0].message.contains(&id(4)));
}

#[test]
fn repair_can_remove_an_edge_authored_by_a_cycle_participant() {
    let f = Fixture::new();
    f.write(1, "", b"Parent");
    f.write(
        2,
        &format!("parent: \"{}\"\ndepends_on: [\"{}\"]\n", id(1), id(1)),
        b"Child",
    );
    let ops = f.ops();
    let before = ops.inspect_raw(&id(2)).unwrap();
    assert!(
        before
            .graph_diagnostics
            .iter()
            .any(|d| d.message.contains("combined lifecycle deadlock") && d.path == f.path(&id(1)))
    );
    let replacement = format!(
        "---\nformat_version: 1\nid: \"{}\"\ntitle: Child\nstate: open\nparent: \"{}\"\n---\nChild",
        id(2),
        id(1)
    );
    let repaired = ops.repair(&id(2), replacement.into_bytes()).unwrap();
    assert!(repaired.graph_diagnostics.is_empty());
    assert!(ItemStore::load_from_root(&f.0).unwrap().is_valid());
}

#[test]
fn creates_files_readable_under_restrictive_umask() {
    if std::env::var_os("WORK_UMASK_TEST_CHILD").is_some() {
        let root = PathBuf::from(std::env::var_os("WORK_UMASK_TEST_ROOT").unwrap());
        let ops = DurableOperations::new(root.clone());
        let created = ops
            .create("New".into(), b"Body".to_vec(), MetadataChange::default())
            .unwrap();
        let id = created.file.header.unwrap().id;
        assert_eq!(
            fs::metadata(root.join(".work/items").join(format!("{id}.md")))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        ops.update(
            &id,
            MetadataChange {
                title: Some("Updated".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            fs::metadata(root.join(".work/operations.lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        return;
    }
    let f = Fixture::new();
    let result = Command::new("sh")
        .arg("-c")
        .arg("umask 0777; exec \"$WORK_UMASK_TEST_BIN\" --exact creates_files_readable_under_restrictive_umask --nocapture")
        .env("WORK_UMASK_TEST_BIN", std::env::current_exe().unwrap())
        .env("WORK_UMASK_TEST_ROOT", &f.0)
        .env("WORK_UMASK_TEST_CHILD", "1")
        .output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn unchanged_completion_policy_does_not_reopen_completed_item() {
    let f = Fixture::new();
    f.write(1, "", b"Body");
    let ops = f.ops();
    ops.close(&id(1), Some("Finished".into())).unwrap();
    let updated = ops
        .update(
            &id(1),
            MetadataChange {
                completion: Some(Completion::Manual),
                title: Some("Retitled".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let header = updated.file.header.unwrap();
    assert_eq!(header.state, Some(ManualState::Done));
    assert_eq!(header.close_reason.as_deref(), Some("Finished"));
}

#[test]
fn aggregate_rejects_close_and_cannot_cache_state() {
    let f = Fixture::new();
    let ops = f.ops();
    let aggregate = ops
        .create(
            "Aggregate".into(),
            b"".to_vec(),
            MetadataChange {
                completion: Some(Completion::Children),
                ..Default::default()
            },
        )
        .unwrap();
    let id = &aggregate.file.header.as_ref().unwrap().id;
    let before = fs::read(f.path(id)).unwrap();
    assert!(matches!(
        ops.close(id, None),
        Err(OperationError::InvalidArgument(_))
    ));
    assert_eq!(fs::read(f.path(id)).unwrap(), before);
    assert_eq!(aggregate.file.header.unwrap().state, None);
}

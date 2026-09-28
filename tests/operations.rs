use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
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

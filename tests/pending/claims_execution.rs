//! Preserved downstream public consumer tests. See tests/MIGRATION.md.
use super::*;

#[test]
#[ignore = "PENDING 6bf6198a: preserved public close/reopen/repair consumer; migrate to beta.2, not a hosting pass"]
fn actual_uninitialized_cli_completion_changes_use_original_durable_operation() {
    let f = Fixture::new();
    let id = f.item();
    let children = ok(f.cli(&["item", "update", &id, "--completion", "children"]));
    assert_eq!(children["item"]["completion"], "children");
    assert_eq!(children["item"]["state"], Value::Null);
    let manual = ok(f.cli(&["item", "update", &id, "--completion", "manual"]));
    assert_eq!(manual["item"]["completion"], "manual");
    assert_eq!(manual["item"]["state"], "open");
    ok(f.cli(&["item", "close", &id]));
    assert_eq!(ok(f.cli(&["item", "reopen", &id]))["item"]["state"], "open");
    assert!(!f.root().exists());
}

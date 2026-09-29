use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use work::core::graph::{Blocker, GraphError, ItemGraph};
use work::core::items::ItemStore;

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id(n: u32) -> String {
    format!("{n:08x}000040008000000000000000")
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-graph-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join(".work/items")).unwrap();
        Self(path)
    }
    fn write(&self, n: u32, extra: &str) {
        let id = id(n);
        let content =
            format!("---\nformat_version: 1\nid: \"{id}\"\ntitle: Item {n}\n{extra}---\nBody\n");
        fs::write(self.0.join(".work/items").join(format!("{id}.md")), content).unwrap();
    }
    fn graph(&self) -> ItemGraph {
        ItemGraph::from_store(&ItemStore::load_from_root(&self.0).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn manual(state: &str, more: &str) -> String {
    format!("state: {state}\n{more}")
}
fn aggregate(more: &str) -> String {
    format!("completion: children\n{more}")
}
fn parent(n: u32) -> String {
    format!("parent: \"{}\"\n", id(n))
}
fn depends(n: u32) -> String {
    format!("depends_on: [\"{}\"]\n", id(n))
}
fn ready(graph: &ItemGraph) -> Vec<String> {
    graph
        .ready()
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect()
}

fn set_fixture_state(path: &Path, state: &str) {
    let source = fs::read_to_string(path).unwrap();
    let (header, body) = source.split_once("\n---\n").unwrap();
    let header = if header.contains("\nstate: done") {
        header.replacen("\nstate: done", &format!("\nstate: {state}"), 1)
    } else {
        header.replacen("\nstate: open", &format!("\nstate: {state}"), 1)
    };
    fs::write(path, format!("{header}\n---\n{body}")).unwrap();
}

#[test]
fn nested_delivery_waits_for_manual_parent_and_derives_aggregate_completion() {
    let f = Fixture::new();
    // P gates C; A1/A2 belong to manual A, and deployment B depends on A.
    f.write(1, &manual("open", "")); // P
    f.write(2, &aggregate(&depends(1))); // C
    f.write(3, &manual("open", &parent(2))); // A
    f.write(4, &manual("open", &parent(3))); // A1
    f.write(5, &manual("open", &parent(3))); // A2
    f.write(6, &manual("open", &format!("{}{}", parent(2), depends(3)))); // B
    let graph = f.graph();
    assert!(graph.is_valid(), "{:?}", graph.diagnostics());
    assert_eq!(ready(&graph), vec![id(1)]);
    let a1 = graph.evaluate(&id(4)).unwrap();
    assert_eq!(
        a1.blockers,
        [Blocker::Prerequisite {
            id: id(1),
            inherited_from: Some(id(2))
        }]
    );
    f.write(1, &manual("done", ""));
    let graph = f.graph();
    assert_eq!(ready(&graph), vec![id(4), id(5)]);
    assert!(
        graph
            .evaluate(&id(3))
            .unwrap()
            .blockers
            .contains(&Blocker::Child(id(4)))
    );
    f.write(4, &manual("done", &parent(3)));
    f.write(5, &manual("done", &parent(3)));
    let graph = f.graph();
    assert_eq!(ready(&graph), vec![id(3)]);
    assert!(!graph.evaluate(&id(2)).unwrap().effective_done);
    f.write(3, &manual("done", &parent(2)));
    assert_eq!(ready(&f.graph()), vec![id(6)]);
    f.write(6, &manual("done", &format!("{}{}", parent(2), depends(3))));
    let graph = f.graph();
    assert!(graph.evaluate(&id(2)).unwrap().effective_done);
    assert_eq!(
        graph.evaluate(&id(2)).unwrap().blockers,
        [Blocker::Aggregate]
    );
    assert!(ready(&graph).is_empty());
    // A retained manual parent stays done when a child reopens.
    f.write(4, &manual("open", &parent(3)));
    let graph = f.graph();
    assert!(graph.evaluate(&id(2)).unwrap().effective_done);
    assert!(graph.evaluate(&id(3)).unwrap().effective_done);
    assert_eq!(ready(&graph), vec![id(4)]);
    // Reopening a direct required child recalculates the aggregate.
    f.write(6, &manual("open", &format!("{}{}", parent(2), depends(3))));
    assert!(!f.graph().evaluate(&id(2)).unwrap().effective_done);
}

#[test]
fn empty_and_nested_aggregates_recalculate_when_children_change() {
    let f = Fixture::new();
    f.write(1, &aggregate(""));
    assert!(!f.graph().evaluate(&id(1)).unwrap().effective_done);
    f.write(2, &aggregate(&parent(1)));
    f.write(3, &manual("done", &parent(2)));
    assert!(f.graph().evaluate(&id(1)).unwrap().effective_done);
    f.write(4, &manual("open", &parent(2)));
    assert!(!f.graph().evaluate(&id(1)).unwrap().effective_done);
    assert_eq!(ready(&f.graph()), vec![id(4)]);
}

#[test]
fn informational_edges_have_symmetric_navigation_without_scheduling() {
    let f = Fixture::new();
    f.write(
        1,
        &manual(
            "open",
            &format!(
                "related: [\"{}\"]\ndiscovered_from: [\"{}\"]\n",
                id(2),
                id(2)
            ),
        ),
    );
    f.write(2, &manual("open", ""));
    let graph = f.graph();
    assert_eq!(ready(&graph), vec![id(1), id(2)]);
    assert_eq!(graph.relations(&id(1)).unwrap().related, [id(2)]);
    assert_eq!(graph.relations(&id(2)).unwrap().related, [id(1)]);
    assert_eq!(graph.relations(&id(2)).unwrap().discovered_by, [id(1)]);
    f.write(
        1,
        &manual(
            "open",
            &format!(
                "related: [\"{}\"]\ndiscovered_from: [\"{}\"]\n{}",
                id(2),
                id(2),
                depends(2)
            ),
        ),
    );
    assert_eq!(ready(&f.graph()), vec![id(2)]);
    assert_eq!(f.graph().relations(&id(2)).unwrap().blocks, [id(1)]);
}

#[test]
fn invalid_graph_refuses_all_readiness_but_keeps_inspection() {
    let cases = [
        (
            manual("open", &parent(1)),
            manual("open", ""),
            "self-relation parent",
        ),
        (
            manual("open", &depends(1)),
            manual("open", ""),
            "self-relation depends_on",
        ),
        (
            manual("open", &format!("related: [\"{}\"]\n", id(1))),
            manual("open", ""),
            "self-relation related",
        ),
        (
            manual("open", &format!("discovered_from: [\"{}\"]\n", id(1))),
            manual("open", ""),
            "self-relation discovered_from",
        ),
        (
            manual("open", &depends(2)),
            manual("open", &depends(1)),
            "dependency cycle",
        ),
        (
            manual("open", &parent(2)),
            manual("open", &parent(1)),
            "hierarchy cycle",
        ),
        (
            manual("open", &parent(2)),
            manual("open", &depends(1)),
            "combined lifecycle deadlock",
        ),
        (
            manual("open", &format!("{}{}", parent(2), depends(2))),
            manual("open", ""),
            "combined lifecycle deadlock",
        ),
        (
            manual("open", &depends(3)),
            manual("open", ""),
            "unresolved depends_on",
        ),
        (
            manual("open", &parent(3)),
            manual("open", ""),
            "unresolved parent",
        ),
        (
            manual("open", &format!("related: [\"{}\"]\n", id(3))),
            manual("open", ""),
            "unresolved related",
        ),
        (
            manual("open", &format!("discovered_from: [\"{}\"]\n", id(3))),
            manual("open", ""),
            "unresolved discovered_from",
        ),
        (
            manual("open", &format!("related: [\"{}\"]\n", id(2))),
            manual("open", &format!("related: [\"{}\"]\n", id(1))),
            "duplicate semantic related",
        ),
    ];
    for (left, right, expected) in cases {
        let f = Fixture::new();
        f.write(1, &left);
        f.write(2, &right);
        let graph = f.graph();
        assert!(
            graph
                .diagnostics()
                .iter()
                .any(|d| d.message.contains(expected)),
            "{expected}: {:?}",
            graph.diagnostics()
        );
        assert!(matches!(graph.ready(), Err(GraphError::Invalid(_))));
        assert!(matches!(
            graph.evaluate(&id(2)),
            Err(GraphError::Invalid(_))
        ));
        assert!(graph.relations(&id(1)).is_ok());
        let valid = Fixture::new();
        valid.write(9, &manual("open", ""));
        assert_eq!(ready(&valid.graph()), vec![id(9)]);
    }
}

#[test]
fn parent_field_cannot_author_multiple_parents() {
    let f = Fixture::new();
    f.write(
        1,
        &manual("open", &format!("parent: [\"{}\", \"{}\"]\n", id(2), id(3))),
    );
    f.write(2, &manual("open", ""));
    f.write(3, &manual("open", ""));
    let graph = f.graph();
    assert!(
        graph
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("parent must be a string"))
    );
    assert!(matches!(graph.ready(), Err(GraphError::Invalid(_))));
}

#[test]
fn detects_long_mixed_deadlock_and_orders_ready_by_priority_then_id() {
    let f = Fixture::new();
    f.write(1, &manual("open", &parent(2)));
    f.write(2, &aggregate(""));
    f.write(3, &manual("open", &depends(2)));
    f.write(4, &manual("open", &format!("{}{}", parent(2), depends(3))));
    // The aggregate waits for child 4, child 4 waits for 3, and 3 waits for aggregate.
    assert!(
        f.graph()
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("combined lifecycle deadlock"))
    );
    f.write(3, &manual("open", "priority: 0\n"));
    f.write(4, &manual("open", &format!("{}priority: 2\n", parent(2))));
    f.write(5, &manual("open", "priority: 0\n"));
    assert_eq!(ready(&f.graph()), vec![id(3), id(5), id(1), id(4)]);
}

#[test]
fn explanations_order_blockers_by_priority_then_canonical_id() {
    let f = Fixture::new();
    f.write(
        1,
        &manual(
            "open",
            &format!("depends_on: [\"{}\", \"{}\"]\n", id(3), id(2)),
        ),
    );
    f.write(2, &manual("open", "priority: 1\n"));
    f.write(3, &manual("open", "priority: 1\n"));
    f.write(4, &manual("open", &format!("{}priority: 0\n", parent(1))));
    let graph = f.graph();
    assert_eq!(
        graph.evaluate(&id(1)).unwrap().blockers,
        [
            Blocker::Child(id(4)),
            Blocker::Prerequisite {
                id: id(2),
                inherited_from: None
            },
            Blocker::Prerequisite {
                id: id(3),
                inherited_from: None
            },
        ]
    );
}

#[test]
fn bootstrap_fixture_exposes_only_next_manual_child() {
    let f = Fixture::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(".work/items");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        fs::copy(
            entry.path(),
            f.0.join(".work/items").join(entry.file_name()),
        )
        .unwrap();
    }
    let foundation = "87b8795f934049c2acebaac42e81664d";
    let loader = "0fac69ec66004e088ce2b22cc0119bd2";
    let graph_item = "13ca14c6e5c14b24a5dea551c343bbeb";
    let operations_item = "cec96d7e174d47c1ab3117968b0bba0f";
    let cli_item = "393f86fafaf54449944d27da1af0a24b";
    // Fixture starts at the item-2-to-item-3 handoff regardless of live progress.
    let loader_path = f.0.join(".work/items").join(format!("{loader}.md"));
    let graph_path = f.0.join(".work/items").join(format!("{graph_item}.md"));
    let operations_path =
        f.0.join(".work/items")
            .join(format!("{operations_item}.md"));
    let cli_path = f.0.join(".work/items").join(format!("{cli_item}.md"));
    // Normalize copied progress so this fixture models the item-2 handoff.
    set_fixture_state(&loader_path, "open");
    set_fixture_state(&graph_path, "open");
    set_fixture_state(&operations_path, "open");
    set_fixture_state(&cli_path, "open");
    let graph = f.graph();
    assert!(graph.is_valid(), "{:?}", graph.diagnostics());
    assert_eq!(ready(&graph), vec![loader.to_owned()]);
    assert_eq!(
        graph.relations(foundation).unwrap().blocks,
        [loader.to_owned()]
    );
    set_fixture_state(&loader_path, "done");
    assert_eq!(ready(&f.graph()), vec![graph_item.to_owned()]);
}

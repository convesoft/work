//! Reactivated ordinary material consumers over authoritative beta.2 routing.
use super::*;

#[test]
fn durable_loop_and_machine_readable_errors() {
    let f = Fixture::new();
    let help = ok(f.call(&["--help"]));
    assert!(help["help"].as_str().unwrap().contains("item create"));
    assert!(help["help"].as_str().unwrap().contains("item close"));
    assert!(
        ok(f.call(&["item", "ready"]))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let first = ok(call(
        &f.0,
        &["item", "create", "--title", "Prepare", "--body", "-"],
        Some(b"opaque body\n"),
    ))["item"]
        .clone();
    let a = first["id"].as_str().unwrap().to_owned();
    assert_eq!(first["body"], "opaque body\n");
    assert_eq!(first["display_id"], format!("w-{}", &a[..8]));
    let item_path = f.0.join(".work/items").join(format!("{a}.md"));
    let before = fs::read(&item_path).unwrap();
    let entries_before = fs::read_dir(f.0.join(".work/items")).unwrap().count();
    error(f.call(&["item", "update", &a]), 2, "invalid_argument");
    assert_eq!(fs::read(item_path).unwrap(), before);
    assert_eq!(
        fs::read_dir(f.0.join(".work/items")).unwrap().count(),
        entries_before
    );
    let second =
        ok(f.call(&["item", "create", "--title", "Deliver", "--label", "release"]))["item"].clone();
    let b = second["id"].as_str().unwrap().to_owned();
    ok(f.call(&["relation", "add", "depends_on", &b, &a]));
    let waiting = ok(f.call(&["item", "inspect", &format!("w-{}", &b[..8])]))["item"].clone();
    assert_eq!(waiting["executable"], false);
    assert_eq!(waiting["relations"]["depends_on"][0], a);
    let ready = ok(f.call(&["item", "ready"]));
    assert_eq!(ready["items"].as_array().unwrap().len(), 1);
    assert_eq!(ready["items"][0]["id"], a);
    ok(f.call(&[
        "item",
        "update",
        &b,
        "--title",
        "Ship",
        "--priority",
        "1",
        "--model",
        "test",
    ]));
    assert_eq!(
        ok(f.call(&["item", "list"]))["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    ok(f.call(&["item", "close", &a, "--reason", "finished"]));
    assert_eq!(ok(f.call(&["item", "ready"]))["items"][0]["id"], b);
    ok(f.call(&["item", "close", &b]));
    assert!(
        ok(f.call(&["item", "ready"]))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ok(f.call(&["item", "reopen", &a]));
    assert_eq!(
        ok(f.call(&["item", "inspect", &b]))["item"]["state"],
        "done"
    );
    error(
        f.call(&["item", "inspect", "ffffffff000040008000000000000000"]),
        3,
        "not_found",
    );
    for id in [
        "00000000000000000000000000000000",
        "w-00000000000000000000000000000000",
    ] {
        error(f.call(&["item", "inspect", id]), 2, "invalid_argument");
        error(f.call(&["item", "close", id]), 2, "invalid_argument");
    }
    error(
        f.call(&["item", "update", &a, "--priority", "bad"]),
        2,
        "invalid_argument",
    );
    error(f.call(&["claim", "next"]), 2, "usage");
    error(
        call(&std::env::temp_dir(), &["claim", "next"], None),
        2,
        "usage",
    );
    error(
        call(&std::env::temp_dir(), &["item", "create", "--title"], None),
        2,
        "usage",
    );
    error(
        call(
            &std::env::temp_dir(),
            &["item", "create", "--bad", "x"],
            None,
        ),
        2,
        "usage",
    );
    let (status, stdout, stderr) = human(&f.0, &["item", "create", "--title", "--json"]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.starts_with('{'));
    assert!(stdout.contains("--json"));
}

#[test]
fn invalid_source_graph_and_ambiguous_prefix_are_distinct() {
    let f = Fixture::new();
    let a = "abcdef12000040008000000000000001";
    let b = "abcdef12000040008000000000000002";
    f.write(a, "");
    f.write(b, "");
    let listed = ok(f.call(&["item", "list"]));
    assert_eq!(listed["items"][0]["display_id"], format!("w-{a}"));
    assert_eq!(listed["items"][1]["display_id"], format!("w-{b}"));
    error(
        f.call(&["item", "inspect", "w-abcdef12"]),
        3,
        "ambiguous_id",
    );
    ok(f.call(&["relation", "add", "depends_on", b, a]));
    error(
        f.call(&["relation", "add", "depends_on", a, b]),
        4,
        "invalid_candidate",
    );
    fs::write(
        f.0.join(".work/items").join(format!("{b}.md")),
        b"bad source",
    )
    .unwrap();
    let diagnostics = ok(f.call(&["item", "diagnose"]));
    assert!(!diagnostics["diagnostics"].as_array().unwrap().is_empty());
    error(f.call(&["item", "inspect", b]), 4, "invalid_source");
    error(
        f.call(&["item", "inspect", &format!("w-{b}")]),
        4,
        "invalid_source",
    );
    let raw = ok(f.call(&["item", "inspect", b, "--raw"]));
    assert_eq!(raw["source"]["raw_hex"], "62616420736f75726365");
    let failure = error(f.call(&["item", "ready"]), 4, "invalid_source");
    assert!(!failure["diagnostics"].as_array().unwrap().is_empty());
    let repaired =
        format!("---\nformat_version: 1\nid: \"{b}\"\ntitle: Repaired\nstate: open\n---\nBody\n");
    let result = ok(call(
        &f.0,
        &["item", "repair", b, "--source", "-"],
        Some(repaired.as_bytes()),
    ));
    assert_eq!(result["source"]["id"], b);
    assert!(
        ok(f.call(&["item", "diagnose"]))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

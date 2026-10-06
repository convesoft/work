//! Reactivated ordinary material consumers over authoritative beta.2 routing.
use super::*;

#[test]
fn protocol_client_runs_durable_loop_and_matches_cli_results() {
    let f = Fixture::new();
    let mut client = Client::new(&f.0);
    let tools = client.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    let expected = [
        "discover",
        "init",
        "status",
        "scope_list",
        "scope_inspect",
        "scope_host",
        "item_create",
        "item_list",
        "item_inspect",
        "item_inspect_raw",
        "item_diagnose",
        "item_ready",
        "item_update",
        "item_close",
        "item_reopen",
        "item_repair",
        "relation_add",
        "relation_remove",
    ];
    assert_eq!(tools.len(), expected.len());
    for name in expected {
        assert!(tools.iter().any(|t| t["name"] == name));
    }
    for name in ["template_list", "claim_acquire", "run_start"] {
        assert!(!tools.iter().any(|t| t["name"] == name));
    }
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0}))["items"],
        json!([])
    );
    let a = client.ok(
        "item_create",
        json!({"title":"Prepare","body":"opaque body\n","worktree":f.0}),
    )["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = client.ok(
        "item_create",
        json!({"title":"Deliver","labels":["release"],"worktree":f.0}),
    )["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let related = client.ok(
        "relation_add",
        json!({"kind":"depends_on","source":b,"target":a,"worktree":f.0}),
    );
    assert_eq!(related["item"]["relations"]["depends_on"], json!([a]));
    assert_eq!(
        client.ok("item_inspect", json!({"id":b,"worktree":f.0})),
        f.cli(&["item", "inspect", &b])["result"]
    );
    assert_eq!(
        client.ok("item_list", json!({"worktree":f.0})),
        f.cli(&["item", "list"])["result"]
    );
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0})),
        f.cli(&["item", "ready"])["result"]
    );
    client.ok(
        "item_update",
        json!({"id":b,"title":"Ship","priority":1,"model":"test","worktree":f.0}),
    );
    assert_eq!(
        client.ok("item_inspect", json!({"id":b,"worktree":f.0}))["item"]["body"],
        ""
    );
    client.ok(
        "item_close",
        json!({"id":a,"reason":"finished","worktree":f.0}),
    );
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0}))["items"][0]["id"],
        b
    );
    client.ok("item_close", json!({"id":b,"worktree":f.0}));
    assert_eq!(
        client.ok("item_ready", json!({"worktree":f.0}))["items"],
        json!([])
    );
    client.ok("item_reopen", json!({"id":a,"worktree":f.0}));
    client.ok(
        "relation_remove",
        json!({"kind":"depends_on","source":b,"target":a,"worktree":f.0}),
    );
    assert_eq!(
        client.ok("item_inspect_raw", json!({"id":a,"worktree":f.0})),
        f.cli(&["item", "inspect", &a, "--raw"])["result"]
    );
}

#[test]
fn protocol_schema_rejects_null_wrong_types_and_source_errors() {
    let f = Fixture::new();
    let mut client = Client::new(&f.0);
    let before = fs::read_dir(f.0.join(".work/items")).unwrap().count();
    for args in [
        json!({"title":null}),
        json!({"title":42}),
        json!({"title":"Task","priority":"1"}),
        json!({"title":"Task","body":null}),
        json!({"title":"Task","unknown":true}),
        json!({"title":" "}),
        json!({"title":"Task","labels":["bad\nlabel"]}),
    ] {
        client.error("item_create", args, "invalid_argument");
    }
    client.error(
        "item_repair",
        json!({"id":"11111111111141118111111111111111","raw_hex":"aéb"}),
        "invalid_argument",
    );
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
    assert_eq!(
        fs::read_dir(f.0.join(".work/items")).unwrap().count(),
        before
    );
    let schema = client.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "item_create")
        .unwrap()
        .clone();
    assert_eq!(schema["inputSchema"]["required"], json!(["title"]));
    assert_eq!(schema["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        schema["inputSchema"]["properties"]["title"]["type"],
        "string"
    );
    let missing = "11111111111141118111111111111111";
    assert_eq!(
        client.error("item_inspect", json!({"id":missing}), "not_found")["code"],
        f.cli(&["item", "inspect", missing])["error"]["code"]
    );
    let malformed = "22222222222242228222222222222222";
    f.write(malformed, "bad_key: true\n");
    assert_eq!(
        client.error("item_ready", json!({}), "invalid_source")["code"],
        f.cli(&["item", "ready"])["error"]["code"]
    );
    assert!(
        !client.ok("item_diagnose", json!({}))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let raw = client.ok("item_inspect_raw", json!({"id":malformed}));
    assert!(!raw["source"]["raw_hex"].as_str().unwrap().is_empty());
    let fixed = format!(
        "---\nformat_version: 1\nid: \"{malformed}\"\ntitle: Fixed\nstate: open\n---\nBody\n"
    );
    let raw_hex: String = fixed
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    client.ok("item_repair", json!({"id":malformed,"raw_hex":raw_hex}));
    assert_eq!(
        client.ok("item_ready", json!({}))["items"][0]["id"],
        malformed
    );
}

#[test]
fn equivalent_cli_and_mcp_mutations_publish_identical_sources() {
    let cli_fixture = Fixture::new();
    let mcp_fixture = Fixture::new();
    let a = "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa";
    let b = "bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb";
    for fixture in [&cli_fixture, &mcp_fixture] {
        fixture.write(a, "");
        fixture.write(b, "");
    }
    let mut client = Client::new(&mcp_fixture.0);
    let cli_added = cli_fixture.cli(&["relation", "add", "depends_on", b, a]);
    let mcp_added = client.ok(
        "relation_add",
        json!({"kind":"depends_on","source":b,"target":a}),
    );
    assert_eq!(
        cli_added["result"]["item"]["relations"],
        mcp_added["item"]["relations"]
    );
    for id in [a, b] {
        assert_eq!(
            fs::read(cli_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap(),
            fs::read(mcp_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap()
        );
    }
    cli_fixture.cli(&["item", "close", a, "--reason", "finished"]);
    client.ok("item_close", json!({"id":a,"reason":"finished"}));
    assert_eq!(
        cli_fixture.cli(&["item", "ready"])["result"]["items"][0]["id"],
        client.ok("item_ready", json!({}))["items"][0]["id"]
    );
    cli_fixture.cli(&["item", "reopen", a]);
    client.ok("item_reopen", json!({"id":a}));
    cli_fixture.cli(&["relation", "remove", "depends_on", b, a]);
    client.ok(
        "relation_remove",
        json!({"kind":"depends_on","source":b,"target":a}),
    );
    for id in [a, b] {
        assert_eq!(
            fs::read(cli_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap(),
            fs::read(mcp_fixture.0.join(".work/items").join(format!("{id}.md"))).unwrap()
        );
    }
    assert_eq!(
        cli_fixture.cli(&["item", "list"])["result"]["items"]
            .as_array()
            .unwrap()
            .len(),
        client.ok("item_list", json!({}))["items"]
            .as_array()
            .unwrap()
            .len()
    );
}

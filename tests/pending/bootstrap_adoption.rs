//! Reactivated ordinary material consumers over authoritative beta.2 routing.
use super::*;

#[test]
fn real_backlog_adopts_and_advances_on_both_surfaces() {
    for via_mcp in [false, true] {
        let fixture = Fixture::new();
        fs::write(
            fixture.file(UNRELATED),
            format!("---\nformat_version: 1\nid: \"{UNRELATED}\"\ntitle: Independent work\nstate: open\n---\nBody\n"),
        )
        .unwrap();
        let before = fixture.originals();
        let mut surface = Surface::new(&fixture.0, via_mcp);
        let listed = surface.ok(&fixture.0, &["item", "list"], "item_list", json!({}));
        assert_eq!(listed["items"].as_array().unwrap().len(), before.len());
        assert!(surface.ready(&fixture.0).contains(&UNRELATED.to_owned()));
        assert_eq!(bootstrap_ready(&mut surface, &fixture.0), vec![STEPS[6]]);
        assert_eq!(surface.inspect(&fixture.0, AGGREGATE)["state"], Value::Null);
        let aggregate = surface.inspect(&fixture.0, AGGREGATE);
        let children = aggregate["relations"]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(children.len(), STEPS.len());
        for id in STEPS {
            assert!(children.contains(&id));
        }
        assert_eq!(
            surface.inspect(&fixture.0, RELEASE)["relations"]["depends_on"][0],
            AGGREGATE
        );
        assert_eq!(surface.inspect(&fixture.0, RELEASE)["state"], "open");
        assert_eq!(
            surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
            false
        );
        for (i, id) in STEPS.iter().enumerate() {
            let item = surface.inspect(&fixture.0, id);
            assert_eq!(item["id"], *id);
            assert_eq!(item["parent"], AGGREGATE);
            if i > 0 {
                assert_eq!(item["relations"]["depends_on"][0], STEPS[i - 1]);
            }
            assert_eq!(
                item["body"].as_str().unwrap().as_bytes(),
                body(&before[&format!("{id}.md")])
            );
        }
        for id in &STEPS[..6] {
            surface.reopen(&fixture.0, id);
        }
        for (i, id) in STEPS.iter().enumerate() {
            assert_eq!(bootstrap_ready(&mut surface, &fixture.0), vec![*id]);
            assert_eq!(
                surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
                false
            );
            surface.close(&fixture.0, id);
            if i < 6 {
                assert_eq!(
                    bootstrap_ready(&mut surface, &fixture.0),
                    vec![STEPS[i + 1]]
                );
            }
        }
        assert_eq!(
            surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
            true
        );
        assert_eq!(bootstrap_ready(&mut surface, &fixture.0), vec![RELEASE]);
        surface.reopen(&fixture.0, STEPS[0]);
        assert_eq!(
            surface.inspect(&fixture.0, AGGREGATE)["effective_done"],
            false
        );
        assert_eq!(surface.inspect(&fixture.0, STEPS[1])["state"], "done");
        assert_eq!(surface.inspect(&fixture.0, STEPS[6])["state"], "done");
        assert!(surface.ready(&fixture.0).contains(&STEPS[0].to_owned()));
        assert!(!surface.ready(&fixture.0).contains(&RELEASE.to_owned()));
        for (name, bytes) in before {
            assert_eq!(
                body(&fs::read(fixture.0.join(".work/items").join(name)).unwrap()),
                body(&bytes)
            );
        }
    }
}

#[test]
fn roundtrip_restart_and_selected_linked_view() {
    let fixture = Fixture::new();
    let before = fixture.originals();
    let mut mcp = Mcp::new(&fixture.0);
    let created = mcp.call(
        "item_create",
        json!({"title":"Adoption round trip","body":"line one\n## acceptance\n- [ ] byte exact\n"}),
    );
    assert!(created.0, "{}", created.1);
    let id = created.1["item"]["id"].as_str().unwrap().to_owned();
    assert!(
        mcp.call(
            "relation_add",
            json!({"kind":"depends_on","source":id,"target":STEPS[6]})
        )
        .0
    );
    assert!(
        mcp.call("item_close", json!({"id":id,"reason":"recorded outcome"}))
            .0
    );
    assert!(mcp.call("item_reopen", json!({"id":id})).0);
    drop(mcp);
    let (status, reopened) = cli(&fixture.0, &["item", "inspect", &id]);
    assert_eq!(status, 0);
    assert_eq!(
        reopened["result"]["item"]["relations"]["depends_on"][0],
        STEPS[6]
    );
    assert_eq!(
        reopened["result"]["item"]["body"],
        "line one\n## acceptance\n- [ ] byte exact\n"
    );
    assert_eq!(reopened["result"]["item"]["state"], "open");
    assert_eq!(reopened["result"]["item"]["close_reason"], Value::Null);
    for (name, bytes) in before {
        assert_eq!(
            fs::read(fixture.0.join(".work/items").join(name)).unwrap(),
            bytes
        );
    }

    let linked = fixture.0.join("linked");
    git(
        &fixture.0,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    // The linked view starts at the committed authored backlog, without the uncommitted new item.
    assert!(!linked.join(".work/items").join(format!("{id}.md")).exists());
    let (status, control) = cli(&fixture.0, &["item", "list"]);
    assert_eq!(status, 0);
    let (status, selected) = cli(
        &fixture.0,
        &["--worktree", linked.to_str().unwrap(), "item", "list"],
    );
    assert_eq!(status, 0);
    assert_eq!(
        control["result"]["items"].as_array().unwrap().len(),
        selected["result"]["items"].as_array().unwrap().len()
    );
    let mut mcp = Mcp::new(&fixture.0);
    assert_eq!(
        mcp.call("item_list", json!({"worktree":linked})).1,
        selected["result"]
    );
    assert_eq!(mcp.call("item_list", json!({})).1, control["result"]);
    assert!(
        mcp.call("item_close", json!({"id":STEPS[6],"worktree":linked}))
            .0
    );
    assert_eq!(
        mcp.call("item_inspect", json!({"id":STEPS[6],"worktree":linked}))
            .1["item"]["state"],
        "done"
    );
    assert_eq!(
        mcp.call("item_inspect", json!({"id":STEPS[6]})).1["item"]["state"],
        "done"
    );
}

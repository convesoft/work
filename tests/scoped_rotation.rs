//! Internal preparation guards; no public rotation/ending success is claimed.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use work::core::{coordination::new_id, project::discover, scope_runs::prepare_rotation};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("work-rotation-{}", new_id().unwrap()));
        std::fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--initial-branch=main"])
                .current_dir(&root)
                .output()
                .unwrap()
                .status
                .success()
        );
        Self(root)
    }
    fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_work"))
            .arg("--json")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }
    fn write(&self, path: &str, value: Value) {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![];
    for p in std::fs::read_dir(root).unwrap() {
        let p = p.unwrap().path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push((p.clone(), std::fs::read(p).unwrap()));
        }
    }
    out.sort();
    out
}
#[test]
fn rotation_prepare_is_readonly_and_requires_explicit_abandonment_of_unfinished_root_wisps() {
    let f = Fixture::new();
    let project = discover(Some(&f.0)).unwrap();
    let before = files(&f.0);
    assert_eq!(
        prepare_rotation(&project, false).unwrap_err().code,
        "coordination_unavailable"
    );
    assert_eq!(files(&f.0), before);
    let rid = f.cli(&["init"])["execution"]["root_run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.write(".work/templates/root.yaml",json!({"format_version":2,"name":"root","items":[{"key":"w","title":"Wisp","persistence":"wisp"}],"edges":[]}));
    let w = f.cli(&["template", "expand", "root"])["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = files(&f.0);
    assert_eq!(
        prepare_rotation(&project, false).unwrap_err().code,
        "run_unfinished"
    );
    let plan = prepare_rotation(&project, true).unwrap();
    assert_eq!(plan["old_run_id"], rid);
    assert_eq!(plan["target_phase"], "abandoned");
    assert!(!plan["protected_files"].as_array().unwrap().is_empty());
    assert_eq!(files(&f.0), before);
    f.cli(&["item", "close", &w]);
    let before = files(&f.0);
    assert_eq!(
        prepare_rotation(&project, false).unwrap()["target_phase"],
        "finalized"
    );
    assert_eq!(files(&f.0), before);
}
#[test]
fn rotation_prepare_refuses_descendants_and_cross_run_root_session_claims() {
    let f = Fixture::new();
    let project = discover(Some(&f.0)).unwrap();
    let rid = f.cli(&["init"])["execution"]["root_run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let scope = f.cli(&["item", "create", "--title", "Child"])["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let child = f.cli(&["run", "start", &scope])["run"]["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = files(&f.0);
    assert_eq!(
        prepare_rotation(&project, true).unwrap_err().code,
        "descendant_run_active"
    );
    assert_eq!(files(&f.0), before);
    let session = new_id().unwrap();
    let claim = new_id().unwrap();
    let identity = json!({"namespace":"fixture","id":"identity"});
    f.write(&format!(".work/scopes/root/runs/{rid}/sessions/{session}.yaml"),json!({"format_version":2,"id":session,"name":"root-owner","actor":"Actor","identity":identity,"registered_at":"2026-01-01T00:00:00Z","state":"active"}));
    f.write(&format!(".work/scopes/{scope}/runs/{child}/claims/{claim}.yaml"),json!({"format_version":2,"id":claim,"item_id":scope,"actor":"Actor","identity":identity,"acquired_at":"2026-01-01T00:00:00Z","named_session":{"run_id":rid,"session_record_id":session}}));
    let before = files(&f.0);
    assert_eq!(
        prepare_rotation(&project, true).unwrap_err().code,
        "claims_present"
    );
    assert_eq!(files(&f.0), before);
    // Ending fixture proves drained read guards only, not delivery of ending APIs.
    std::fs::remove_file(f.0.join(format!(
        ".work/scopes/{scope}/runs/{child}/claims/{claim}.yaml"
    )))
    .unwrap();
    f.write(&format!(".work/scopes/{scope}/runs/{child}/run.yaml"),json!({"format_version":2,"id":child,"created_at":"2026-01-01T00:00:00Z","phase":"abandoned","ended_at":"2026-01-02T00:00:00Z"}));
    let before = files(&f.0);
    assert_eq!(
        prepare_rotation(&project, false).unwrap()["old_run_id"],
        rid
    );
    assert_eq!(files(&f.0), before);
}

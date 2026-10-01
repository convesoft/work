//! Run-core acceptance on disposable repositories. Shared routing/adapter tests
//! belong to main; terminal transitions below are fixtures, not implemented verbs.
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use work::core::coordination::{CoordinationGuard, ExecutionError, yaml_bytes};
use work::core::items::{ItemStore, ManualState};
use work::core::project::{Project, discover};
use work::core::runs::{RunManifest, RunPhase, RunSnapshot, RunStore};
use work::core::storage::Storage;

static NEXT: AtomicU64 = AtomicU64::new(0);
const ROOT: &str = "d66b0ba51d2c4a7aa15de40cb3c9d507";
const MEMBER: &str = "d66b0ba59d2c4a7aa15de40cb3c9d507";
const OTHER: &str = "d66b0ba58d2c4a7aa15de40cb3c9d507";
const WISP: &str = "22222222000040008000000000000000";
const PRIOR: &str = "11111111000040008000000000000000";

struct Fixture {
    path: PathBuf,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-runs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let result = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["init", "--initial-branch=main"])
            .output()
            .unwrap();
        assert!(result.status.success());
        fs::create_dir_all(path.join(".work/items")).unwrap();
        let project = discover(Some(&path)).unwrap();
        Storage::new(project.clone()).initialize().unwrap();
        let fixture = Self { path, project };
        fixture.item(ROOT, "open", "");
        fixture.item(MEMBER, "done", "");
        fixture.item(OTHER, "open", "");
        fixture
    }
    fn item(&self, id: &str, state: &str, extra: &str) {
        fs::write(self.path.join(".work/items").join(format!("{id}.md")),format!("---\nformat_version: 1\nid: \"{id}\"\ntitle: \"Fixture\"\nstate: {state}\n{extra}---\nOpaque body\n")).unwrap();
    }
    fn root(&self) -> PathBuf {
        self.project.git_common_dir.join("work")
    }
    fn guard(&self) -> CoordinationGuard {
        CoordinationGuard::acquire(&self.project, true).unwrap()
    }
    fn view(&self) -> ItemStore {
        ItemStore::load_from_root(&self.path).unwrap()
    }
    fn terminal(&self, phase: &str) {
        let guard = self.guard();
        let dir = PathBuf::from("runs").join(PRIOR);
        guard.ensure_dir(&dir.join("items")).unwrap();
        guard.ensure_dir(&dir.join("sessions")).unwrap();
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/runs")
                .join(format!("{phase}.yaml")),
        )
        .unwrap()
        .replace("STORE_ID", &guard.metadata.store_id)
        .replace("GENERATION", &guard.metadata.recovery_generation);
        guard
            .create(&dir.join("run.yaml"), source.as_bytes())
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
fn materials(view: &ItemStore) -> BTreeSet<String> {
    view.files
        .iter()
        .filter_map(|f| f.header.as_ref().map(|h| h.id.clone()))
        .collect()
}
fn snapshot<'a>(
    view: &'a ItemStore,
    material: &'a BTreeSet<String>,
    claims: &'a BTreeSet<String>,
) -> RunSnapshot<'a> {
    RunSnapshot {
        view,
        material_ids: material,
        active_claims: claims,
    }
}
fn error<T: std::fmt::Debug>(result: Result<T, ExecutionError>, code: &str) {
    assert_eq!(result.unwrap_err().code, code);
}

#[test]
fn empty_finished_current_and_membership_never_close_root() {
    let f = Fixture::new();
    let view = f.view();
    let material = materials(&view);
    let claims = BTreeSet::new();
    let snapshot = snapshot(&view, &material, &claims);
    let guard = f.guard();
    let original = fs::read(f.path.join(".work/items").join(format!("{ROOT}.md"))).unwrap();
    let started = RunStore::start(&guard, &snapshot, ROOT, None, None).unwrap();
    assert!(started.changed);
    assert!(!started.finished);
    let id = &started.run.id;
    let attached = RunStore::attach(&guard, &snapshot, id, &[MEMBER.into()]).unwrap();
    assert!(attached.finished);
    assert!(attached.changed);
    assert_eq!(attached.run.phase, RunPhase::Active);
    assert!(
        !RunStore::attach(&guard, &snapshot, id, &[MEMBER.into()])
            .unwrap()
            .changed
    );
    let again = RunStore::start(&guard, &snapshot, ROOT, None, None).unwrap();
    assert!(!again.changed);
    assert!(again.finished);
    assert_eq!(&again.run.id, id);
    assert_eq!(
        fs::read(f.path.join(".work/items").join(format!("{ROOT}.md"))).unwrap(),
        original
    );
    let claimed = BTreeSet::from([MEMBER.into()]);
    assert!(
        !RunStore::load(&guard)
            .unwrap()
            .get(id)
            .unwrap()
            .finished(&snapshot_fn(&view, &material, &claimed))
            .unwrap()
    );
    error(
        RunStore::detach(
            &guard,
            &snapshot_fn(&view, &material, &claimed),
            id,
            &[MEMBER.into()],
        ),
        "claim_conflict",
    );
    // Repeated attach of a claimed already-member changes nothing.
    assert!(
        !RunStore::attach(
            &guard,
            &snapshot_fn(&view, &material, &claimed),
            id,
            &[MEMBER.into()]
        )
        .unwrap()
        .changed
    );
    f.item(MEMBER, "open", "");
    let reopened = f.view();
    assert!(
        !RunStore::load(&guard)
            .unwrap()
            .get(id)
            .unwrap()
            .finished(&snapshot_fn(&reopened, &material, &claims))
            .unwrap()
    );
    assert!(
        !RunStore::detach(&guard, &snapshot, id, &[MEMBER.into()])
            .unwrap()
            .finished
    );
    assert!(
        !RunStore::detach(&guard, &snapshot, id, &[MEMBER.into()])
            .unwrap()
            .changed
    );
}
fn snapshot_fn<'a>(
    v: &'a ItemStore,
    m: &'a BTreeSet<String>,
    c: &'a BTreeSet<String>,
) -> RunSnapshot<'a> {
    snapshot(v, m, c)
}

#[test]
fn material_exclusion_workspace_conflict_and_claim_checks() {
    let f = Fixture::new();
    let view = f.view();
    let m = materials(&view);
    let c = BTreeSet::new();
    let s = snapshot(&view, &m, &c);
    let g = f.guard();
    let a = RunStore::start(&g, &s, ROOT, Some(WISP.into()), Some(PRIOR.into())).unwrap();
    error(
        RunStore::start(&g, &s, ROOT, Some(PRIOR.into()), None),
        "run_conflict",
    );
    RunStore::attach(&g, &s, &a.run.id, &[MEMBER.into()]).unwrap();
    error(RunStore::start(&g, &s, MEMBER, None, None), "run_conflict");
    let b = RunStore::start(&g, &s, OTHER, None, None).unwrap();
    error(
        RunStore::attach(&g, &s, &b.run.id, &[MEMBER.into()]),
        "run_conflict",
    );
    error(
        RunStore::attach(&g, &s, &a.run.id, &[OTHER.into()]),
        "run_conflict",
    );
    error(
        RunStore::attach(&g, &s, &a.run.id, &[ROOT.into()]),
        "invalid_argument",
    );
    error(RunStore::attach(&g, &s, &a.run.id, &[]), "invalid_argument");
    let active = BTreeSet::from([OTHER.into()]);
    error(
        RunStore::attach(
            &g,
            &snapshot(&view, &m, &active),
            &a.run.id,
            &[OTHER.into()],
        ),
        "run_conflict",
    );
    error(
        RunStore::attach(&g, &s, &a.run.id, &[WISP.into()]),
        "invalid_argument",
    );
}

#[test]
fn terminal_fixtures_allow_fresh_start_without_resurrection() {
    for phase in ["finalized", "disposed"] {
        let f = Fixture::new();
        f.terminal(phase);
        let view = f.view();
        let m = materials(&view);
        let c = BTreeSet::new();
        let s = snapshot(&view, &m, &c);
        let g = f.guard();
        let store = RunStore::load(&g).unwrap();
        assert!(store.current_for_root(ROOT).is_none());
        error(
            RunStore::attach(&g, &s, PRIOR, &[MEMBER.into()]),
            "run_not_current",
        );
        let next = RunStore::start(&g, &s, ROOT, None, None).unwrap();
        assert_ne!(next.run.id, PRIOR);
        assert_eq!(RunStore::load(&g).unwrap().records.len(), 2);
        assert!(
            RunStore::load(&g)
                .unwrap()
                .get(PRIOR)
                .unwrap()
                .wisps
                .is_empty()
        );
    }
}

#[test]
fn manifest_validation_is_strict_and_future_versions_short_circuit() {
    let f = Fixture::new();
    let v = f.view();
    let m = materials(&v);
    let c = BTreeSet::new();
    let g = f.guard();
    let run = RunStore::start(&g, &snapshot(&v, &m, &c), ROOT, None, None)
        .unwrap()
        .run;
    let source = run.to_json();
    for (key, bad) in [
        ("phase", json!("finished")),
        ("created_at", json!("2026-02-30T10:00:00Z")),
        ("ended_at", json!("2026-10-01T10:00:00Z")),
        ("material_items", json!([MEMBER, MEMBER])),
        ("material_items", json!([ROOT])),
        ("default_workspace_id", json!(null)),
        ("unexpected", json!(true)),
        ("phase", json!("disposed")),
    ] {
        let mut value = source.clone();
        value[key] = bad;
        error(
            RunManifest::parse(&yaml_bytes(&value), &g.metadata, &run.id),
            "invalid_format",
        );
    }
    let mut future = json!({"format_version":2,"anything":true});
    error(
        RunManifest::parse(&yaml_bytes(&future), &g.metadata, &run.id),
        "unsupported_format",
    );
    future = source.clone();
    future["store_id"] = json!(WISP);
    error(
        RunManifest::parse(&yaml_bytes(&future), &g.metadata, &run.id),
        "identity_mismatch",
    );
    error(
        RunManifest::parse(&yaml_bytes(&source), &g.metadata, PRIOR),
        "invalid_format",
    );
    for raw in [
        b"format_version: 1\nformat_version: 1\n".as_slice(),
        b"format_version: !!int 1\n",
        b"format_version: &a 1\n",
        b"format_version: 1\n---\n{}\n",
        b"format_version: 1\nphase: *a\n",
    ] {
        error(
            RunManifest::parse(raw, &g.metadata, &run.id),
            "invalid_format",
        );
    }
}

#[test]
fn strict_wisps_restart_sources_and_orphan_directories() {
    let f = Fixture::new();
    let v = f.view();
    let m = materials(&v);
    let c = BTreeSet::new();
    let g = f.guard();
    let run = RunStore::start(&g, &snapshot(&v, &m, &c), ROOT, None, None)
        .unwrap()
        .run;
    let mut header = v.resolve(MEMBER).unwrap().header.clone().unwrap();
    header.id = WISP.into();
    let path = RunStore::create_wisp(&g, &run.id, &header, b"\r\nVerbatim \xc3\xa9\n").unwrap();
    let loaded = RunStore::load(&g).unwrap();
    let record = loaded.get(&run.id).unwrap();
    assert_eq!(record.members(), BTreeSet::from([WISP.into()]));
    assert_eq!(
        record.wisps[0].body.as_deref(),
        Some(b"\r\nVerbatim \xc3\xa9\n".as_slice())
    );
    let mut resolved = v.clone();
    resolved.files.extend(record.wisps.clone());
    assert!(record.finished(&snapshot(&resolved, &m, &c)).unwrap());
    header.state = Some(ManualState::Open);
    RunStore::replace_wisp(
        &g,
        &run.id,
        &record.wisp_sources[WISP],
        &header,
        b"New body",
    )
    .unwrap();
    let current = RunStore::load(&g).unwrap();
    assert_eq!(
        current.get(&run.id).unwrap().wisps[0].body.as_deref(),
        Some(b"New body".as_slice())
    );
    error(
        RunStore::replace_wisp(
            &g,
            &run.id,
            &record.wisp_sources[WISP],
            &header,
            b"Stale body",
        ),
        "conflict",
    );
    drop(g);
    let restarted = f.guard();
    assert_eq!(
        RunStore::load(&restarted)
            .unwrap()
            .get(&run.id)
            .unwrap()
            .wisps[0]
            .path,
        path
    );
    fs::write(&path, b"malformed").unwrap();
    error(RunStore::load(&restarted), "invalid_format");
    fs::remove_file(&path).unwrap();
    restarted
        .ensure_dir(&PathBuf::from("runs").join(PRIOR))
        .unwrap();
    let orphan = RunStore::load(&restarted).unwrap_err();
    assert_eq!(orphan.code, "invalid_format");
    assert_eq!(orphan.path, Some(f.root().join("runs").join(PRIOR)));
}

#[test]
fn run_start_process_child() {
    let Some(path) = std::env::var_os("WORK_RUN_RACE_REPO") else {
        return;
    };
    let project = discover(Some(Path::new(&path))).unwrap();
    let gate = PathBuf::from(std::env::var_os("WORK_RUN_RACE_GATE").unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gate.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let guard = loop {
        match CoordinationGuard::acquire(&project, true) {
            Ok(guard) => break guard,
            Err(e) if e.code == "storage_busy" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) => panic!("{e}"),
        }
    };
    let view = ItemStore::load(&project).unwrap();
    let m = materials(&view);
    let c = BTreeSet::new();
    let result = RunStore::start(&guard, &snapshot(&view, &m, &c), ROOT, None, None).unwrap();
    fs::write(
        PathBuf::from(std::env::var_os("WORK_RUN_RACE_OUT").unwrap()),
        &result.run.id,
    )
    .unwrap();
}

#[test]
fn independent_process_starts_choose_one_current_run() {
    let f = Fixture::new();
    let gate = f.path.join("gate");
    let mut children = Vec::new();
    for i in 0..2 {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "run_start_process_child", "--nocapture"])
            .env("WORK_RUN_RACE_REPO", &f.path)
            .env("WORK_RUN_RACE_GATE", &gate)
            .env("WORK_RUN_RACE_OUT", f.path.join(format!("out{i}")))
            .spawn()
            .unwrap();
        children.push(child);
    }
    fs::write(&gate, b"go").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(
        fs::read(f.path.join("out0")).unwrap(),
        fs::read(f.path.join("out1")).unwrap()
    );
    assert_eq!(RunStore::load(&f.guard()).unwrap().records.len(), 1);
}

#[test]
fn mixed_expansion_destinations_partial_failure_cleanup_and_fresh_retry() {
    use std::collections::BTreeMap;
    use work::core::graph::ItemGraph;
    use work::core::templates::{Persistence, PreviewRequest, TemplateCatalog};
    let f = Fixture::new();
    fs::create_dir_all(f.path.join(".work/templates")).unwrap();
    fs::write(f.path.join(".work/templates/mixed.yaml"),"format_version: 2\nname: mixed\nitems:\n- {key: e, title: Last}\n- {key: d, title: SecondWisp, persistence: wisp}\n- {key: c, title: Third}\n- {key: b, title: FirstWisp, persistence: wisp}\n- {key: a, title: First}\nedges:\n- {from: 'local:a', kind: depends_on, to: 'local:e'}\n- {from: 'local:b', kind: parent, to: 'local:c'}\n- {from: 'local:d', kind: depends_on, to: 'local:a'}\n").unwrap();
    let g = f.guard();
    let view = f.view();
    let m = materials(&view);
    let c = BTreeSet::new();
    let run = RunStore::start(&g, &snapshot(&view, &m, &c), ROOT, None, None)
        .unwrap()
        .run;
    let request = PreviewRequest {
        root: None,
        parameters: BTreeMap::new(),
        existing: BTreeMap::new(),
    };
    let catalog = TemplateCatalog::load_from_root(&f.path).unwrap();
    let plan = catalog
        .plan_expansion("mixed", &request, &view, &f.path, Some(&run), g.root_path())
        .unwrap();
    assert_eq!(plan.items.len(), 5);
    assert_eq!(plan.run_id.as_deref(), Some(run.id.as_str()));
    let root_before = fs::read(f.path.join(".work/items").join(format!("{ROOT}.md"))).unwrap();
    let mut created = Vec::new();
    // Main owns material bindings/publication. This fixture exercises only the
    // returned plan and real run-side guarded publication, with a failure at 4.
    for item in plan.items.iter().take(3) {
        match item.persistence {
            Persistence::Material => fs::write(&item.path, &item.raw).unwrap(),
            Persistence::Wisp => assert_eq!(
                RunStore::create_wisp(&g, &run.id, &item.header, &item.body).unwrap(),
                item.path
            ),
        }
        created.push(item.id.clone());
    }
    let partial = plan.partial_error(
        ExecutionError::new("io", "injected fourth-write failure"),
        &created,
        &[],
        &[],
    );
    assert_eq!(partial.code, "io");
    assert_eq!(
        partial.details["partial"]["created"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(partial.details["key_ids"].as_object().unwrap().len(), 5);
    assert_eq!(partial.details["publication"], "not_published");
    for item in &plan.items[..3] {
        assert!(item.path.exists());
    }
    for item in &plan.items[3..] {
        assert!(!item.path.exists());
    }
    let loaded = RunStore::load(&g).unwrap();
    let record = loaded.get(&run.id).unwrap();
    assert_eq!(
        record.members(),
        BTreeSet::from([plan.key_ids["b"].clone()])
    );
    let mut partial_view = f.view();
    partial_view.files.extend(record.wisps.clone());
    assert!(!ItemGraph::from_store(&partial_view).is_valid());
    // Explicit caller cleanup, without a rollback engine or application record.
    for item in &plan.items[..3] {
        if item.persistence == Persistence::Wisp {
            g.delete(
                &PathBuf::from("runs")
                    .join(&run.id)
                    .join("items")
                    .join(format!("{}.md", item.id)),
                &record.wisp_sources[&item.id],
            )
            .unwrap();
        } else {
            fs::remove_file(&item.path).unwrap();
        }
    }
    let retry = catalog
        .plan_expansion(
            "mixed",
            &request,
            &f.view(),
            &f.path,
            Some(&run),
            g.root_path(),
        )
        .unwrap();
    assert_ne!(retry.key_ids, plan.key_ids);
    for item in &retry.items {
        match item.persistence {
            Persistence::Material => fs::write(&item.path, &item.raw).unwrap(),
            Persistence::Wisp => assert_eq!(
                RunStore::create_wisp(&g, &run.id, &item.header, &item.body).unwrap(),
                item.path
            ),
        }
    }
    let material_view = f.view();
    let material = materials(&material_view);
    let mut resolved = material_view.clone();
    resolved.files.extend(
        RunStore::load(&g)
            .unwrap()
            .get(&run.id)
            .unwrap()
            .wisps
            .clone(),
    );
    assert!(ItemGraph::from_store(&resolved).is_valid());
    let ids = retry
        .items
        .iter()
        .filter(|i| i.persistence == Persistence::Material)
        .map(|i| i.id.clone())
        .collect::<Vec<_>>();
    let attached =
        RunStore::attach(&g, &snapshot(&resolved, &material, &c), &run.id, &ids).unwrap();
    assert_eq!(attached.run.material_items.len(), 3);
    assert_eq!(
        RunStore::load(&g)
            .unwrap()
            .get(&run.id)
            .unwrap()
            .members()
            .len(),
        5
    );
    assert_eq!(
        fs::read(f.path.join(".work/items").join(format!("{ROOT}.md"))).unwrap(),
        root_before
    );
    let mut mismatch = request.clone();
    mismatch.root = Some(OTHER.into());
    assert!(
        catalog
            .plan_expansion(
                "mixed",
                &mismatch,
                &resolved,
                &f.path,
                Some(&run),
                g.root_path()
            )
            .is_err()
    );
    let mut terminal = run.clone();
    terminal.phase = RunPhase::Disposed;
    assert!(
        catalog
            .plan_expansion(
                "mixed",
                &request,
                &resolved,
                &f.path,
                Some(&terminal),
                g.root_path()
            )
            .is_err()
    );
    drop(g);
    let restarted = f.guard();
    assert_eq!(
        RunStore::load(&restarted)
            .unwrap()
            .get(&run.id)
            .unwrap()
            .members()
            .len(),
        5
    );
}

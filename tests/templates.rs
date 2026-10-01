use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use work::core::items::{Completion, ItemStore, ManualState};
use work::core::templates::{HintSource, PreviewRequest, TemplateCatalog, TemplateError};

static NEXT: AtomicU64 = AtomicU64::new(0);
const ROOT: &str = "d66b0ba51d2c4a7aa15de40cb3c9d507";
const REVIEW: &str = "d66b0ba59d2c4a7aa15de40cb3c9d507";
const DELIVERY: &str = "d66b0ba58d2c4a7aa15de40cb3c9d507";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "work-template-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        fs::create_dir_all(root.join(".work/templates")).unwrap();
        Self(root)
    }
    fn template(&self, name: &str, source: &str) {
        fs::write(
            self.0.join(".work/templates").join(format!("{name}.yaml")),
            source,
        )
        .unwrap();
    }
    fn copy_template(&self, name: &str) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".work/templates")
            .join(format!("{name}.yaml"));
        fs::copy(
            source,
            self.0.join(".work/templates").join(format!("{name}.yaml")),
        )
        .unwrap();
    }
    fn item(&self, id: &str, extra: &str) {
        fs::write(
            self.0.join(".work/items").join(format!("{id}.md")),
            format!(
                "---\nformat_version: 1\nid: \"{id}\"\ntitle: \"Existing\"\nstate: open\n{extra}---\nOpaque existing body\n"
            ),
        )
        .unwrap();
    }
    fn catalog(&self) -> TemplateCatalog {
        TemplateCatalog::load_from_root(&self.0).unwrap()
    }
    fn view(&self) -> ItemStore {
        ItemStore::load_from_root(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn request(parameters: &[(&str, &str)], existing: &[(&str, &str)]) -> PreviewRequest {
    PreviewRequest {
        root: Some(ROOT.into()),
        parameters: parameters
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        existing: existing
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
    }
}

fn error_text(error: TemplateError) -> String {
    let mut text = error.to_string();
    for diagnostic in error.diagnostics() {
        text.push_str(&diagnostic.message);
    }
    text
}

#[test]
fn discovers_and_validates_versioned_repo_templates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let catalog = TemplateCatalog::load_from_root(root).unwrap();
    assert_eq!(catalog.files.len(), 2);
    assert!(catalog.files.iter().all(|f| f.diagnostics.is_empty()));
    assert_eq!(catalog.validate("review_cycle").unwrap().items.len(), 3);
    assert_eq!(catalog.validate("review_fixes").unwrap().edges.len(), 8);
}

#[test]
fn preview_is_symbolic_deterministic_read_only_and_preserves_rendered_body() {
    let f = Fixture::new();
    f.copy_template("review_cycle");
    f.item(ROOT, "");
    let original = fs::read(f.0.join(".work/items").join(format!("{ROOT}.md"))).unwrap();
    let catalog = f.catalog();
    let view = f.view();
    let input = request(&[("change", "search indexing")], &[]);
    let first = catalog.preview("review_cycle", &input, &view).unwrap();
    let second = catalog.preview("review_cycle", &input, &view).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.items.len(), 3);
    assert_eq!(first.edges.len(), 5);
    assert_eq!(first.items[0].key, "deliver");
    assert_eq!(first.items[1].key, "implement");
    assert_eq!(first.items[1].title, "Implement search indexing");
    assert_eq!(
        first.items[1].body,
        "Complete the change and record the result.\n"
    );
    assert_eq!(first.items[1].completion, Completion::Manual);
    assert_eq!(first.items[1].state, Some(ManualState::Open));
    assert_eq!(first.items[1].model.as_deref(), Some("gpt-6-sol"));
    assert_eq!(
        first.items[1].model_source,
        Some(HintSource::TemplateDefault)
    );
    assert!(first.edges.iter().any(|edge| {
        edge.from.reference == "local:review"
            && edge.to.reference == "local:implement"
            && edge.to.existing_id.is_none()
    }));
    assert!(first.edges.iter().any(|edge| {
        edge.from.reference == "local:implement"
            && edge.to.reference == "root"
            && edge.to.existing_id.as_deref() == Some(ROOT)
    }));
    assert_eq!(
        fs::read(f.0.join(".work/items").join(format!("{ROOT}.md"))).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(f.0.join(".work/items")).unwrap().count(), 1);
    assert!(!f.0.join(".work/work.db").exists());
}

#[test]
fn supporting_template_binds_existing_items_and_distinguishes_hint_sources() {
    let f = Fixture::new();
    f.copy_template("review_fixes");
    f.item(ROOT, "");
    f.item(REVIEW, &format!("parent: \"{ROOT}\"\n"));
    f.item(DELIVERY, &format!("parent: \"{ROOT}\"\n"));
    let preview = f
        .catalog()
        .preview(
            "review_fixes",
            &request(
                &[("change", "parser"), ("finding", "lost bytes")],
                &[("review", REVIEW), ("delivery", DELIVERY)],
            ),
            &f.view(),
        )
        .unwrap();
    assert_eq!(preview.items.len(), 3);
    assert_eq!(preview.edges.len(), 8);
    let fix = preview.items.iter().find(|item| item.key == "fix").unwrap();
    assert_eq!(fix.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(fix.model_source, Some(HintSource::Item));
    assert_eq!(fix.thinking_source, Some(HintSource::TemplateDefault));
    assert!(preview.edges.iter().any(|edge| {
        edge.from.reference == "existing:delivery"
            && edge.from.existing_id.as_deref() == Some(DELIVERY)
            && edge.to.reference == "local:review_again"
    }));
}

#[test]
fn rejects_schema_tokens_and_references_before_preview() {
    let base = "format_version: 1\nname: bad\nparameters: [change]\nitems:\n  - key: task\n    title: \"Do {{change}}\"\n";
    for (source, expected) in [
        (
            base.replace("format_version: 1", "format_version: 3"),
            "format_version",
        ),
        (format!("{base}unknown: yes\n"), "unknown template field"),
        (
            base.replace("name: bad", "name: bad\nname: bad"),
            "duplicate YAML key",
        ),
        (base.replace("{{change}}", "{{missing}}"), "undeclared"),
        (base.replace("{{change}}", "{{change"), "unmatched"),
        (base.replace("{{change}}", "plain"), "unused parameter"),
        (
            "format_version: 1\nname: bad\nparameters: [change]\ndefaults:\n  model: '{{change}}'\nitems:\n  - key: task\n    title: Task\n    model: explicit\n".into(),
            "unused parameter",
        ),
        (
            "format_version: 1\nname: bad\nparameters: [change]\ndefaults:\n  thinking: '{{change}}'\nitems:\n  - key: task\n    title: Task\n    thinking: explicit\n".into(),
            "unused parameter",
        ),
        (
            format!(
                "{base}edges:\n  - {{from: \"local:task\", kind: depends_on, to: \"local:missing\"}}\n"
            ),
            "unknown local reference",
        ),
    ] {
        let f = Fixture::new();
        f.template("bad", &source);
        let error = f.catalog().validate("bad").unwrap_err();
        assert!(error_text(error).contains(expected), "{expected}: {source}");
    }
}

#[test]
fn rejects_bad_arguments_rendered_fields_and_duplicate_or_cyclic_edges() {
    let f = Fixture::new();
    f.copy_template("review_cycle");
    f.item(ROOT, "");
    let catalog = f.catalog();
    let view = f.view();
    for input in [
        request(&[], &[]),
        request(&[("change", "a"), ("extra", "b")], &[]),
        request(&[("change", "line\nbreak")], &[]),
    ] {
        assert_eq!(
            catalog
                .preview("review_cycle", &input, &view)
                .unwrap_err()
                .code(),
            "invalid_argument"
        );
    }
    let bad = Fixture::new();
    bad.item(ROOT, "");
    bad.template(
        "bad",
        "format_version: 1\nname: bad\nitems:\n  - key: a\n    title: A\n  - key: b\n    title: B\nedges:\n  - {from: 'local:a', kind: depends_on, to: 'local:b'}\n  - {from: 'local:b', kind: depends_on, to: 'local:a'}\n",
    );
    let cycle = bad
        .catalog()
        .preview("bad", &request(&[], &[]), &bad.view())
        .unwrap_err();
    assert_eq!(cycle.code(), "invalid_candidate");
    assert!(error_text(cycle).contains("local:a"));
    bad.template(
        "bad",
        "format_version: 1\nname: bad\nitems:\n  - key: a\n    title: A\nedges:\n  - {from: 'local:a', kind: parent, to: root}\n  - {from: 'local:a', kind: parent, to: root}\n",
    );
    assert_eq!(
        bad.catalog().validate("bad").unwrap_err().code(),
        "invalid_template"
    );
}

#[test]
fn invalid_selected_graph_blocks_preview_and_existing_bindings_must_resolve() {
    let f = Fixture::new();
    f.copy_template("review_cycle");
    f.item(ROOT, &format!("depends_on: [\"{REVIEW}\"]\n"));
    let error = f
        .catalog()
        .preview(
            "review_cycle",
            &request(&[("change", "parser")], &[]),
            &f.view(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_source");
    assert!(error_text(error).contains("unresolved depends_on"));

    let f = Fixture::new();
    f.copy_template("review_fixes");
    f.item(ROOT, "");
    f.item(REVIEW, "");
    let error = f
        .catalog()
        .preview(
            "review_fixes",
            &request(
                &[("change", "parser"), ("finding", "loss")],
                &[("review", REVIEW), ("delivery", DELIVERY)],
            ),
            &f.view(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "invalid_argument");
    assert!(error_text(error).contains("delivery"));
}

#[test]
fn parameter_values_are_inserted_once_and_omitted_body_is_empty() {
    let f = Fixture::new();
    f.item(ROOT, "");
    f.template(
        "literal",
        "format_version: 1\nname: literal\nparameters: [value]\nitems:\n  - key: task\n    title: \"Do {{value}}\"\n",
    );
    let result = f
        .catalog()
        .preview(
            "literal",
            &request(&[("value", "{{untouched}}")], &[]),
            &f.view(),
        )
        .unwrap();
    assert_eq!(result.items[0].title, "Do {{untouched}}");
    assert_eq!(result.items[0].body, "");
    assert_eq!(
        result.parameters,
        BTreeMap::from([("value".into(), "{{untouched}}".into())])
    );
}

#[test]
fn version_two_rootless_material_planning_is_read_only_and_defaults_unchanged() {
    use work::core::templates::Persistence;
    let f = Fixture::new();
    f.template("initial", "format_version: 2\nname: initial\nitems:\n- {key: b, title: Beta}\n- {key: a, title: Alpha, persistence: material}\nedges:\n- {from: 'local:b', kind: depends_on, to: 'local:a'}\n");
    let mut input = request(&[], &[]);
    input.root = None;
    let catalog = f.catalog();
    let view = f.view();
    let preview = catalog.preview("initial", &input, &view).unwrap();
    assert_eq!(preview.root, None);
    assert!(
        preview
            .items
            .iter()
            .all(|i| i.persistence == Persistence::Material)
    );
    assert_eq!(preview, catalog.preview("initial", &input, &view).unwrap());
    let shared = f.0.join("disposable-shared");
    let plan = catalog
        .plan_expansion("initial", &input, &view, &f.0, None, &shared)
        .unwrap();
    assert!(plan.run_id.is_none());
    assert!(plan.updated.is_empty());
    assert_eq!(plan.key_ids.len(), 2);
    assert_eq!(
        plan.items
            .iter()
            .map(|i| i.key.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        plan.items[1].header.depends_on,
        vec![plan.items[0].id.clone()]
    );
    for item in &plan.items {
        assert_eq!(
            item.path,
            f.0.join(".work/items").join(format!("{}.md", item.id))
        );
        assert!(!item.path.exists());
    }
    assert_eq!(fs::read_dir(f.0.join(".work/items")).unwrap().count(), 0);
    assert!(!shared.exists());
    let retry = catalog
        .plan_expansion("initial", &input, &view, &f.0, None, &shared)
        .unwrap();
    assert_ne!(retry.key_ids, plan.key_ids);
    f.template(
        "legacy",
        "format_version: 1\nname: legacy\nitems: [{key: a, title: Alpha}]\n",
    );
    assert_eq!(
        f.catalog().preview("legacy", &input, &view).unwrap().items[0].persistence,
        Persistence::Material
    );
    f.template(
        "bad",
        "format_version: 1\nname: bad\nitems: [{key: a, title: Alpha, persistence: material}]\n",
    );
    assert!(f.catalog().validate("bad").is_err());
}

#[test]
fn persistence_is_structural_and_wisp_preview_needs_no_run() {
    use work::core::templates::Persistence;
    let f = Fixture::new();
    let view = f.view();
    let mut input = request(&[], &[]);
    input.root = None;
    f.template("mixed","format_version: 2\nname: mixed\nitems:\n- {key: a, title: Material}\n- {key: b, title: Wisp, persistence: wisp}\nedges:\n- {from: 'local:b', kind: parent, to: 'local:a'}\n");
    let catalog = f.catalog();
    let preview = catalog.preview("mixed", &input, &view).unwrap();
    assert_eq!(preview.items[1].persistence, Persistence::Wisp);
    let error = catalog
        .plan_expansion("mixed", &input, &view, &f.0, None, &f.0.join("shared"))
        .unwrap_err();
    assert!(error_text(error).contains("explicit current run"));
    for invalid in ["'{{kind}}'", "null", "1", "temporary"] {
        f.template("bad",&format!("format_version: 2\nname: bad\nitems: [{{key: a, title: Alpha, persistence: {invalid}}}]\n"));
        assert!(f.catalog().validate("bad").is_err());
    }
    f.template("needs_root","format_version: 2\nname: needs_root\nitems: [{key: a, title: Alpha}]\nedges: [{from: 'local:a', kind: parent, to: root}]\n");
    assert!(
        error_text(
            f.catalog()
                .preview("needs_root", &input, &view)
                .unwrap_err()
        )
        .contains("root is required")
    );
}

#[test]
fn planner_keeps_existing_source_authority_and_validates_complete_graph() {
    let f = Fixture::new();
    f.item(ROOT, "");
    f.item(REVIEW, "");
    let view = f.view();
    let original = view.resolve(REVIEW).unwrap();
    let raw = original.raw.clone();
    let body = original.body.clone();
    f.template("extend","format_version: 2\nname: extend\nexisting: [review]\nitems: [{key: a, title: Alpha}]\nedges: [{from: 'existing:review', kind: depends_on, to: 'local:a'}]\n");
    let input = request(&[], &[("review", REVIEW)]);
    let plan = f
        .catalog()
        .plan_expansion("extend", &input, &view, &f.0, None, &f.0.join("shared"))
        .unwrap();
    assert_eq!(plan.updated.len(), 1);
    assert_eq!(plan.updated[0].raw, raw);
    assert_eq!(plan.updated[0].body, body);
    assert_eq!(plan.updated[0].path, original.path);
    assert_eq!(
        plan.updated[0].header.as_ref().unwrap().depends_on,
        vec![plan.items[0].id.clone()]
    );
    f.template("cycle","format_version: 2\nname: cycle\nitems: [{key: a, title: Alpha}, {key: b, title: Beta}]\nedges:\n- {from: 'local:a', kind: depends_on, to: 'local:b'}\n- {from: 'local:b', kind: depends_on, to: 'local:a'}\n");
    assert!(matches!(
        f.catalog().plan_expansion(
            "cycle",
            &request(&[], &[]),
            &view,
            &f.0,
            None,
            &f.0.join("shared")
        ),
        Err(TemplateError::InvalidCandidate(_))
    ));
    assert_eq!(fs::read_dir(f.0.join(".work/items")).unwrap().count(), 2);
}

#[test]
fn reverse_related_duplicate_is_refused_before_any_expansion() {
    let f = Fixture::new();
    f.item(ROOT, "");
    f.item(REVIEW, &format!("related: [\"{ROOT}\"]\n"));
    f.template("duplicate","format_version: 2\nname: duplicate\nexisting: [review]\nitems: [{key: a, title: Alpha}]\nedges: [{from: root, kind: related, to: 'existing:review'}]\n");
    assert!(matches!(
        f.catalog()
            .preview("duplicate", &request(&[], &[("review", REVIEW)]), &f.view()),
        Err(TemplateError::InvalidCandidate(_))
    ));
}

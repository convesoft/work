use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use work::core::items::{Completion, ItemStore, LookupError, ManualState};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
const ID: &str = "d66b0ba51d2c4a7aa15de40cb3c9d507";
const OTHER_ID: &str = "d66b0ba59d2c4a7aa15de40cb3c9d507";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "work-items-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join(".work/items")).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, content: impl AsRef<[u8]>) {
        fs::write(self.0.join(".work/items").join(name), content).unwrap();
    }
    fn load(&self) -> ItemStore {
        ItemStore::load_from_root(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn item(id: &str, extra: &str, body: &str) -> String {
    format!(
        "---\nformat_version: 1\nid: \"{id}\"\ntitle: \"Example\"\nstate: open\n{extra}---\n{body}"
    )
}
fn errors(store: &ItemStore) -> String {
    store
        .diagnostics()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn loads_isolated_copy_of_real_bootstrap_items_without_changing_bytes() {
    let fixture = Fixture::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(".work/items");
    let mut originals = Vec::new();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let raw = fs::read(entry.path()).unwrap();
        fixture.write(name.to_str().unwrap(), &raw);
        originals.push((name, raw));
    }
    let store = fixture.load();
    assert!(store.is_valid(), "{}", errors(&store));
    assert_eq!(store.files.len(), originals.len());
    assert_eq!(store.files.len(), 9);
    for file in &store.files {
        let header = file.header.as_ref().unwrap();
        assert_eq!(file.path.file_stem().unwrap().to_str().unwrap(), header.id);
        assert_eq!(file.raw, fs::read(&file.path).unwrap());
        assert!(!file.body.as_ref().unwrap().is_empty());
        assert!(matches!(store.resolve(&header.id), Ok(found) if found.path == file.path));
        for target in header
            .parent
            .iter()
            .chain(&header.depends_on)
            .chain(&header.related)
            .chain(&header.discovered_from)
        {
            assert!(store.resolve(target).is_ok(), "unresolved {target}");
        }
    }
    let aggregate = store
        .resolve("w-933a6d82")
        .unwrap()
        .header
        .as_ref()
        .unwrap();
    assert_eq!(aggregate.completion, Completion::Children);
    assert_eq!(aggregate.state, None);
    let foundation = store
        .resolve("w-87b8795f")
        .unwrap()
        .header
        .as_ref()
        .unwrap();
    assert_eq!(foundation.state, Some(ManualState::Done));
    for (name, raw) in originals {
        assert_eq!(
            fs::read(fixture.0.join(".work/items").join(name)).unwrap(),
            raw
        );
    }
    assert!(!fixture.0.join(".work/work.db").exists());
}

#[test]
fn preserves_lf_crlf_body_blanks_and_final_newline_choices() {
    let fixture = Fixture::new();
    fixture.write(
        &format!("{ID}.md"),
        item(ID, "", "\n\nBody without final newline"),
    );
    let crlf = format!(
        "---\r\nformat_version: 1\r\nid: \"{OTHER_ID}\"\r\ntitle: \"Example\"\r\nstate: done\r\nclose_reason: \"Finished\"\r\n---\r\n\r\nBody\r\n"
    );
    fixture.write(&format!("{OTHER_ID}.md"), crlf.as_bytes());
    let store = fixture.load();
    assert!(store.is_valid(), "{}", errors(&store));
    assert_eq!(
        store.resolve(ID).unwrap().body.as_deref(),
        Some(&b"\n\nBody without final newline"[..])
    );
    assert_eq!(
        store.resolve(OTHER_ID).unwrap().body.as_deref(),
        Some(&b"\r\nBody\r\n"[..])
    );
}

#[test]
fn applies_defaults_and_optional_header_fields() {
    let fixture = Fixture::new();
    let extra = format!(
        "parent: \"{OTHER_ID}\"\ndepends_on: [\"{OTHER_ID}\"]\nrelated: []\ndiscovered_from: []\nlabels: [\"Exact Case\", \"other\"]\nmodel: \"provider/model\"\nthinking: \"high\"\n"
    );
    fixture.write(&format!("{ID}.md"), item(ID, &extra, "body"));
    let store = fixture.load();
    let header = store.resolve(ID).unwrap().header.as_ref().unwrap();
    assert_eq!(header.completion, Completion::Manual);
    assert_eq!(header.priority, 2);
    assert_eq!(header.parent.as_deref(), Some(OTHER_ID));
    assert_eq!(header.depends_on, [OTHER_ID]);
    assert_eq!(header.labels, ["Exact Case", "other"]);
    assert_eq!(header.model.as_deref(), Some("provider/model"));
    assert_eq!(header.thinking.as_deref(), Some("high"));
}

#[test]
fn resolves_only_unambiguous_prefixes() {
    let fixture = Fixture::new();
    fixture.write(&format!("{ID}.md"), item(ID, "", ""));
    fixture.write(&format!("{OTHER_ID}.md"), item(OTHER_ID, "", ""));
    let store = fixture.load();
    assert!(
        matches!(store.resolve("w-d66b0ba5"), Err(LookupError::Ambiguous(ids)) if ids.len() == 2)
    );
    assert!(store.resolve("w-d66b0ba51").is_ok());
    assert!(store.resolve(OTHER_ID).is_ok());
    assert!(matches!(
        store.resolve("w-unknown"),
        Err(LookupError::InvalidInput)
    ));
    assert!(matches!(
        store.resolve("w-aaaaaaaa"),
        Err(LookupError::NotFound)
    ));
}

#[test]
fn reports_invalid_headers_and_retains_original_file_for_inspection() {
    let cases = [
        ("format_version: 2", "unsupported format_version"),
        ("format_version: +1", "format_version must be an integer"),
        ("id: 123", "id must be a string"),
        ("id: \"d66b0ba51d2c3a7aa15de40cb3c9d507\"", "UUIDv4"),
        ("title: 123", "title must be a string"),
        ("title: \" \"", "nonblank"),
        ("state: null", "state must be a string"),
        ("priority: \"2\"", "priority must be an integer"),
        ("priority: 5", "priority must be an integer"),
        ("priority: 1_000", "priority must be an integer"),
        ("mystery: true", "unknown header key"),
        ("labels: [x, x]", "duplicate value"),
        ("depends_on: [123]", "string IDs"),
        (
            "state: done\nclose_reason: null",
            "close_reason must be a string",
        ),
        ("state: open\nclose_reason: reason", "close_reason requires"),
        ("title: &name Example", "anchors and tags"),
        ("title: !custom Example", "anchors and tags"),
        ("title: [Example]", "title must be a string"),
    ];
    for (replacement, expected) in cases {
        let fixture = Fixture::new();
        let original = item(ID, "", "\nOpaque body");
        let base = match replacement.split_once(':').unwrap().0 {
            "format_version" => "format_version: 1",
            "id" => "id: \"d66b0ba51d2c4a7aa15de40cb3c9d507\"",
            "title" => "title: \"Example\"",
            "state" => "state: open",
            "priority" | "mystery" | "labels" | "depends_on" => "state: open",
            _ => "state: open",
        };
        let content = if replacement.starts_with("state: done\n")
            || replacement.starts_with("state: open\n")
        {
            original.replace("state: open", replacement)
        } else if matches!(
            replacement.split_once(':').unwrap().0,
            "priority" | "mystery" | "labels" | "depends_on" | "close_reason"
        ) {
            original.replace(base, &format!("{base}\n{replacement}"))
        } else {
            original.replace(base, replacement)
        };
        fixture.write(&format!("{ID}.md"), &content);
        let store = fixture.load();
        assert!(!store.is_valid(), "accepted {replacement}");
        assert!(
            errors(&store).contains(expected),
            "{replacement}: {}",
            errors(&store)
        );
        assert_eq!(store.files[0].raw, content.as_bytes());
        assert_eq!(store.files[0].body.as_deref(), Some(&b"\nOpaque body"[..]));
    }
}

#[test]
fn reports_duplicate_yaml_keys_forbidden_constructs_and_framing_errors() {
    let cases = [
        (
            format!(
                "---\nformat_version: 1\nid: \"{ID}\"\ntitle: A\ntitle: B\nstate: open\n---\nbody"
            ),
            "duplicate YAML key",
        ),
        (
            format!(
                "---\nformat_version: 1\nid: \"{ID}\"\ntitle: A\nstate: open\nlabels: &list [x]\n---\nbody"
            ),
            "anchors and tags",
        ),
        (
            format!(
                "---\nformat_version: 1\nid: \"{ID}\"\ntitle: A\nstate: open\n<<: {{title: B}}\n---\nbody"
            ),
            "merge keys",
        ),
        (
            format!(
                "---\nformat_version: 1\nid: \"{ID}\"\ntitle: A\nstate: open\nlabels: [*missing]\n---\nbody"
            ),
            "invalid YAML",
        ),
        (
            format!("---\nformat_version: 1\nid: \"{ID}\"\ntitle: A\nstate: open\nbody"),
            "missing closing",
        ),
        (
            format!(
                "--- # comment\nformat_version: 1\nid: \"{ID}\"\ntitle: A\nstate: open\n---\nbody"
            ),
            "first line",
        ),
        (
            format!(
                "---\n%YAML 1.1\nformat_version: 1\nid: \"{ID}\"\ntitle: A\nstate: open\n---\nbody"
            ),
            "directives are forbidden",
        ),
    ];
    for (content, expected) in cases {
        let fixture = Fixture::new();
        fixture.write(&format!("{ID}.md"), content);
        let store = fixture.load();
        assert!(errors(&store).contains(expected), "{}", errors(&store));
    }
    let fixture = Fixture::new();
    let mut invalid_utf8 = item(ID, "", "body").into_bytes();
    invalid_utf8.push(0xff);
    fixture.write(&format!("{ID}.md"), invalid_utf8);
    assert!(errors(&fixture.load()).contains("invalid UTF-8"));
}

#[test]
fn reports_filename_mismatches_and_duplicate_id_claims() {
    let fixture = Fixture::new();
    fixture.write(&format!("{ID}.md"), item(ID, "", ""));
    fixture.write("wrong.md", item(ID, "", ""));
    let store = fixture.load();
    assert_eq!(store.files.len(), 2);
    assert!(errors(&store).contains("filename must be"));
    assert!(errors(&store).contains("duplicate item ID"));
    assert!(matches!(store.resolve(ID), Err(LookupError::Ambiguous(_))));
}

#[test]
fn keeps_unexpected_files_visible_as_diagnostics() {
    let fixture = Fixture::new();
    fixture.write(&format!("{ID}.txt"), item(ID, "", "body"));
    let store = fixture.load();
    assert_eq!(store.files.len(), 1);
    assert_eq!(store.files[0].body.as_deref(), Some(&b"body"[..]));
    assert!(errors(&store).contains("filename must be"));
    assert!(matches!(store.resolve(ID), Err(LookupError::Invalid(_))));
}

#[test]
fn enforces_aggregate_state_omission() {
    let fixture = Fixture::new();
    fixture.write(&format!("{ID}.md"), item(ID, "completion: children\n", ""));
    assert!(errors(&fixture.load()).contains("must omit state"));
    let fixture = Fixture::new();
    let aggregate = format!(
        "---\nformat_version: 1\nid: \"{ID}\"\ntitle: Aggregate\ncompletion: children\n---\n"
    );
    fixture.write(&format!("{ID}.md"), aggregate);
    assert!(fixture.load().is_valid());
}

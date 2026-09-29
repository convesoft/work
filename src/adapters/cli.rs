//! CLI presentation over shared durable operations.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work::core::graph::{Blocker, ItemGraph};
use work::core::items::{Completion, Diagnostic, ItemStore, LookupError, ManualState};
use work::core::operations::{
    DurableOperations, Inspection, MetadataChange, OperationError, RawInspection, RelationKind,
};
use work::core::project::{DiscoveryError, Project, discover};

struct CliError {
    code: &'static str,
    message: String,
    diagnostics: Vec<Diagnostic>,
    published_item: Option<Value>,
    previous_source_path: Option<PathBuf>,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            diagnostics: vec![],
            published_item: None,
            previous_source_path: None,
        }
    }
    fn exit(&self) -> i32 {
        match self.code {
            "usage" | "invalid_argument" => 2,
            "not_found" | "ambiguous_id" => 3,
            "invalid_source" | "invalid_candidate" => 4,
            "conflict" | "already_exists" => 5,
            _ => 1,
        }
    }
    fn value(&self) -> Value {
        let mut v = json!({"code":self.code,"message":self.message});
        if !self.diagnostics.is_empty() {
            v["diagnostics"] = json!(self.diagnostics.iter().map(diagnostic).collect::<Vec<_>>());
        }
        if let Some(p) = &self.published_item {
            v["published_item"] = p.clone();
        }
        if let Some(p) = &self.previous_source_path {
            v["previous_source_path"] = json!(encode_path(p));
        }
        v
    }
}

impl From<OperationError> for CliError {
    fn from(error: OperationError) -> Self {
        let diagnostics = match &error {
            OperationError::InvalidSource(d) | OperationError::InvalidCandidate(d) => d.clone(),
            OperationError::Published { cause, .. } => match cause.as_ref() {
                OperationError::InvalidSource(d) | OperationError::InvalidCandidate(d) => d.clone(),
                _ => vec![],
            },
            _ => vec![],
        };
        Self {
            code: error.code(),
            message: error.to_string(),
            diagnostics,
            published_item: error
                .published_item()
                .map(|(id, path)| json!({"id":id,"path":encode_path(path)})),
            previous_source_path: error.previous_source_path().map(Path::to_path_buf),
        }
    }
}
impl From<DiscoveryError> for CliError {
    fn from(error: DiscoveryError) -> Self {
        let code = if matches!(error, DiscoveryError::UnsupportedProject(_)) {
            "unsupported_project"
        } else {
            "io"
        };
        Self::new(code, error.to_string())
    }
}

pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<_> = args.collect();
    let json_mode = leading_json_mode(&args);
    match dispatch(&args) {
        Ok(value) => {
            if json_mode {
                println!("{}", json!({"ok":true,"result":value}));
            } else {
                print_human(&value);
            }
            0
        }
        Err(error) => {
            if json_mode {
                println!("{}", json!({"ok":false,"error":error.value()}));
            } else {
                eprintln!("work: {}: {}", error.code, error.message);
            }
            error.exit()
        }
    }
}

fn leading_json_mode(args: &[OsString]) -> bool {
    let mut at = 0;
    let mut json = false;
    while let Some(arg) = args.get(at) {
        if arg == "--json" {
            json = true;
            at += 1;
        } else if arg == "--worktree" {
            at += 2;
        } else {
            break;
        }
    }
    json
}

fn dispatch(args: &[OsString]) -> Result<Value, CliError> {
    let mut at = 0;
    let mut selected: Option<&Path> = None;
    while let Some(arg) = args.get(at) {
        if arg == "--json" {
            at += 1;
        } else if arg == "--worktree" {
            selected = Some(Path::new(
                args.get(at + 1)
                    .ok_or_else(|| usage("missing --worktree path"))?,
            ));
            at += 2;
        } else {
            break;
        }
    }
    let command = args.get(at).ok_or_else(|| usage("missing command"))?;
    if command == "--help" || command == "help" {
        if at + 1 != args.len() || selected.is_some() {
            return Err(usage("work --help takes no arguments"));
        }
        return Ok(json!({"help": HELP}));
    }
    if command == "--version" {
        if at + 1 != args.len() || selected.is_some() {
            return Err(usage("work --version takes no arguments"));
        }
        return Ok(json!({"version":env!("CARGO_PKG_VERSION")}));
    }
    if command == "discover" {
        if at + 2 < args.len() {
            return Err(usage("work discover [PATH]"));
        }
        let project = discover(args.get(at + 1).map(Path::new).or(selected))?;
        return Ok(
            json!({"worktree_root":encode_path(&project.worktree_root),"git_common_dir":encode_path(&project.git_common_dir)}),
        );
    }
    let words: Vec<String> = args[at..]
        .iter()
        .map(|a| {
            a.to_str()
                .map(str::to_owned)
                .ok_or_else(|| CliError::new("invalid_argument", "command arguments must be UTF-8"))
        })
        .collect::<Result<_, _>>()?;
    validate_command_shape(&words)?;
    let project = discover(selected)?;
    let ops = DurableOperations::new(&project.worktree_root);
    match words.as_slice() {
        [noun, verb, tail @ ..] if noun == "item" => item_command(&project, &ops, verb, tail),
        [noun, verb, tail @ ..] if noun == "relation" => {
            relation_command(&project, &ops, verb, tail)
        }
        _ => Err(usage("expected item or relation command")),
    }
}

fn validate_command_shape(words: &[String]) -> Result<(), CliError> {
    match words {
        [noun, verb, tail @ ..] if noun == "item" => match verb.as_str() {
            "list" | "ready" | "diagnose" if tail.is_empty() => Ok(()),
            "inspect" if tail.len() == 1 || (tail.len() == 2 && tail[1] == "--raw") => Ok(()),
            "create" => Ok(()),
            "update" if tail.len() >= 2 => Ok(()),
            "close" | "reopen" if !tail.is_empty() => Ok(()),
            "repair" if tail.len() == 3 && tail[1] == "--source" && tail[2] == "-" => Ok(()),
            _ => Err(usage("unknown item command or arguments")),
        },
        [noun, verb, tail @ ..] if noun == "relation" => {
            if (verb == "add" || verb == "remove") && tail.len() == 3 {
                Ok(())
            } else {
                Err(usage("relation add|remove KIND SOURCE TARGET"))
            }
        }
        _ => Err(usage("expected item or relation command")),
    }
}

fn item_command(
    project: &Project,
    ops: &DurableOperations,
    verb: &str,
    args: &[String],
) -> Result<Value, CliError> {
    match verb {
        "list" if args.is_empty() => Ok(json!({"items":items_value(project,&ops.list()?)?})),
        "ready" if args.is_empty() => Ok(json!({"items":items_value(project,&ops.ready()?)?})),
        "diagnose" if args.is_empty() => {
            let store = ItemStore::load(project).map_err(io_error)?;
            let graph = ItemGraph::from_store(&store);
            Ok(
                json!({"diagnostics":graph.diagnostics().iter().map(diagnostic).collect::<Vec<_>>()}),
            )
        }
        "inspect" if args.len() == 1 => {
            let candidate = args[0].strip_prefix("w-").unwrap_or(&args[0]);
            let id = match resolve(project, &args[0]) {
                Ok(id) => id,
                Err(error) if error.code == "not_found" && candidate.len() == 32 => {
                    if let Ok(raw) = ops.inspect_raw(candidate) {
                        return Err(invalid_source(raw.file.diagnostics));
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            Ok(json!({"item":one_item_value(project,&ops.inspect(&id)?)?}))
        }
        "inspect" if args.len() == 2 && args[1] == "--raw" => {
            let raw = ops.inspect_raw(&args[0])?;
            Ok(json!({"source":source_value(&args[0],&raw)}))
        }
        "create" => {
            let input = parse_metadata(project, args, true, true)?;
            let title = input
                .title
                .ok_or_else(|| usage("item create requires --title"))?;
            let item = ops.create(title, input.body.unwrap_or_default(), input.change)?;
            Ok(json!({"item":one_item_value(project,&item)?}))
        }
        "update" if args.len() >= 2 => {
            let id = resolve(project, &args[0])?;
            let input = parse_metadata(project, &args[1..], true, false)?;
            let mut change = input.change;
            change.title = input.title;
            Ok(json!({"item":one_item_value(project,&ops.update(&id,change)?)?}))
        }
        "close" if !args.is_empty() => {
            let id = resolve(project, &args[0])?;
            let reason = match &args[1..] {
                [] => None,
                [flag, value] if flag == "--reason" => Some(value.clone()),
                _ => return Err(usage("item close ID [--reason TEXT]")),
            };
            Ok(json!({"item":one_item_value(project,&ops.close(&id,reason)?)?}))
        }
        "reopen" if args.len() == 1 => {
            let id = resolve(project, &args[0])?;
            Ok(json!({"item":one_item_value(project,&ops.reopen(&id)?)?}))
        }
        "repair" if args.len() == 3 && args[1] == "--source" && args[2] == "-" => {
            ops.inspect_raw(&args[0])?;
            let mut source = Vec::new();
            io::stdin().read_to_end(&mut source).map_err(io_error)?;
            let raw = ops.repair(&args[0], source)?;
            Ok(json!({"source":source_value(&args[0],&raw)}))
        }
        _ => Err(usage("unknown item command or arguments")),
    }
}
fn source_value(id: &str, raw: &RawInspection) -> Value {
    let mut hex = String::with_capacity(raw.file.raw.len() * 2);
    for byte in &raw.file.raw {
        write!(hex, "{byte:02x}").unwrap();
    }
    json!({"id":id,"path":encode_path(&raw.file.path),"raw_hex":hex,
        "diagnostics":raw.file.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
        "graph_diagnostics":raw.graph_diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
        "recovery_path":raw.recovery_path.as_deref().map(encode_path),
        "additional_recovery_paths":raw.additional_recovery_paths.iter().map(|p|encode_path(p)).collect::<Vec<_>>()})
}

fn relation_command(
    project: &Project,
    ops: &DurableOperations,
    verb: &str,
    args: &[String],
) -> Result<Value, CliError> {
    if args.len() != 3 {
        return Err(usage("relation add|remove KIND SOURCE TARGET"));
    }
    let kind = match args[0].as_str() {
        "parent" => RelationKind::Parent,
        "depends_on" => RelationKind::DependsOn,
        "related" => RelationKind::Related,
        "discovered_from" => RelationKind::DiscoveredFrom,
        _ => return Err(CliError::new("invalid_argument", "unknown relation kind")),
    };
    let source = resolve(project, &args[1])?;
    let target = resolve(project, &args[2])?;
    let item = match verb {
        "add" => ops.relation_add(&source, kind, &target)?,
        "remove" => ops.relation_remove(&source, kind, &target)?,
        _ => return Err(usage("relation add|remove KIND SOURCE TARGET")),
    };
    Ok(json!({"item":one_item_value(project,&item)?}))
}

struct MetadataInput {
    title: Option<String>,
    body: Option<Vec<u8>>,
    change: MetadataChange,
}
fn parse_metadata(
    project: &Project,
    args: &[String],
    title_allowed: bool,
    body_allowed: bool,
) -> Result<MetadataInput, CliError> {
    let mut input = MetadataInput {
        title: None,
        body: None,
        change: MetadataChange::default(),
    };
    let mut read_body_from_stdin = false;
    let mut at = 0;
    while at < args.len() {
        let flag = args[at].as_str();
        let clear = matches!(
            flag,
            "--clear-parent" | "--clear-labels" | "--clear-model" | "--clear-thinking"
        );
        let value = if clear {
            None
        } else {
            Some(
                args.get(at + 1)
                    .ok_or_else(|| usage(format!("missing value for {flag}")))?
                    .clone(),
            )
        };
        match flag {
            "--title" if title_allowed => input.title = value,
            "--body" if body_allowed => {
                let value = value.unwrap();
                read_body_from_stdin = value == "-";
                input.body = if read_body_from_stdin {
                    None
                } else {
                    Some(value.into_bytes())
                };
            }
            "--completion" => {
                input.change.completion = Some(match value.as_deref() {
                    Some("manual") => Completion::Manual,
                    Some("children") => Completion::Children,
                    _ => {
                        return Err(CliError::new(
                            "invalid_argument",
                            "completion must be manual or children",
                        ));
                    }
                })
            }
            "--priority" => {
                let priority = value
                    .unwrap()
                    .parse::<u8>()
                    .map_err(|_| CliError::new("invalid_argument", "priority must be 0..4"))?;
                if priority > 4 {
                    return Err(CliError::new("invalid_argument", "priority must be 0..4"));
                }
                input.change.priority = Some(priority);
            }
            "--parent" => input.change.parent = Some(Some(resolve(project, &value.unwrap())?)),
            "--clear-parent" => input.change.parent = Some(None),
            "--label" => input
                .change
                .labels
                .get_or_insert_with(Vec::new)
                .push(value.unwrap()),
            "--clear-labels" => input.change.labels = Some(vec![]),
            "--model" => input.change.model = Some(value),
            "--clear-model" => input.change.model = Some(None),
            "--thinking" => input.change.thinking = Some(value),
            "--clear-thinking" => input.change.thinking = Some(None),
            _ => return Err(usage(format!("unknown option {flag}"))),
        }
        at += if clear { 1 } else { 2 };
    }
    if body_allowed {
        let title = input
            .title
            .as_deref()
            .ok_or_else(|| usage("item create requires --title"))?;
        if !valid_line(title) {
            return Err(CliError::new(
                "invalid_argument",
                "title must be a nonblank single line",
            ));
        }
        if input
            .change
            .labels
            .as_ref()
            .is_some_and(|labels| labels.iter().any(|label| !valid_line(label)))
        {
            return Err(CliError::new(
                "invalid_argument",
                "labels must be nonblank single lines",
            ));
        }
        for (name, value) in [
            ("model", &input.change.model),
            ("thinking", &input.change.thinking),
        ] {
            if value
                .as_ref()
                .and_then(|v| v.as_deref())
                .is_some_and(|v| !valid_line(v))
            {
                return Err(CliError::new(
                    "invalid_argument",
                    format!("{name} must be a nonblank single line"),
                ));
            }
        }
    }
    if read_body_from_stdin {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes).map_err(io_error)?;
        input.body = Some(bytes);
    }
    Ok(input)
}

fn valid_line(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains(['\n', '\r'])
}

fn resolve(project: &Project, input: &str) -> Result<String, CliError> {
    let store = ItemStore::load(project).map_err(io_error)?;
    match store.resolve(input) {
        Ok(file) => Ok(file.header.as_ref().unwrap().id.clone()),
        Err(LookupError::InvalidInput) => Err(CliError::new(
            "invalid_argument",
            "item ID must be lowercase hexadecimal, optionally prefixed with w-",
        )),
        Err(LookupError::NotFound) => Err(CliError::new(
            "not_found",
            format!("item {input} was not found"),
        )),
        Err(LookupError::Ambiguous(ids)) => Err(CliError::new(
            "ambiguous_id",
            format!("item {input} matches {}", ids.join(", ")),
        )),
        Err(LookupError::Invalid(d)) => Err(invalid_source(d)),
    }
}
fn one_item_value(project: &Project, item: &Inspection) -> Result<Value, CliError> {
    let store = ItemStore::load(project).map_err(io_error)?;
    item_value(&display_prefixes(&store), item)
}

fn items_value(project: &Project, items: &[Inspection]) -> Result<Vec<Value>, CliError> {
    let store = ItemStore::load(project).map_err(io_error)?;
    let prefixes = display_prefixes(&store);
    items
        .iter()
        .map(|item| item_value(&prefixes, item))
        .collect()
}

fn display_prefixes(store: &ItemStore) -> BTreeMap<String, usize> {
    let mut ids: Vec<_> = store
        .files
        .iter()
        .filter_map(|file| file.header.as_ref().map(|header| header.id.clone()))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut prefixes = BTreeMap::new();
    for (index, id) in ids.iter().enumerate() {
        let neighbors = [
            index.checked_sub(1).and_then(|i| ids.get(i)),
            ids.get(index + 1),
        ];
        let shared = neighbors
            .into_iter()
            .flatten()
            .map(|other| {
                id.bytes()
                    .zip(other.bytes())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .max()
            .unwrap_or(0);
        prefixes.insert(id.clone(), (shared + 1).clamp(8, 32));
    }
    prefixes
}

fn item_value(prefixes: &BTreeMap<String, usize>, item: &Inspection) -> Result<Value, CliError> {
    let h = item
        .file
        .header
        .as_ref()
        .ok_or_else(|| CliError::new("invalid_source", "item header is invalid"))?;
    let prefix = prefixes.get(&h.id).copied().unwrap_or(8);
    let r = &item.relations;
    let e = &item.evaluation;
    let blockers: Vec<Value> = e
        .iter()
        .flat_map(|x| &x.blockers)
        .map(|b| match b {
            Blocker::Completed => json!({"kind":"completed"}),
            Blocker::Aggregate => json!({"kind":"aggregate"}),
            Blocker::Prerequisite { id, inherited_from } => {
                json!({"kind":"prerequisite","id":id,"inherited_from":inherited_from})
            }
            Blocker::Child(id) => json!({"kind":"child","id":id}),
        })
        .collect();
    Ok(json!({
        "id":h.id,"display_id":format!("w-{}",&h.id[..prefix]),"title":h.title,
        "completion":match h.completion {Completion::Manual=>"manual",Completion::Children=>"children"},
        "state":h.state.map(|s|match s {ManualState::Open=>"open",ManualState::Done=>"done"}),
        "effective_done":e.as_ref().map(|x|x.effective_done),"executable":e.as_ref().map(|x|x.executable),"blockers":blockers,
        "priority":h.priority,"parent":h.parent,"depends_on":h.depends_on,"related":h.related,
        "discovered_from":h.discovered_from,"labels":h.labels,"model":h.model,"thinking":h.thinking,
        "close_reason":h.close_reason,"body":item.file.body.as_ref().and_then(|b|std::str::from_utf8(b).ok()),
        "path":encode_path(&item.file.path),"relations":{"parent":r.parent,"children":r.children,"depends_on":r.depends_on,
            "blocks":r.blocks,"related":r.related,"discovered_from":r.discovered_from,"discovered_by":r.discovered_by},
        "graph_diagnostics":item.graph_diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
        "recovery_path":item.recovery_path.as_deref().map(encode_path)
    }))
}
fn invalid_source(diagnostics: Vec<Diagnostic>) -> CliError {
    CliError {
        code: "invalid_source",
        message: "selected source or graph is invalid".into(),
        diagnostics,
        published_item: None,
        previous_source_path: None,
    }
}
fn diagnostic(d: &Diagnostic) -> Value {
    json!({"path":encode_path(&d.path),"line":d.line,"message":d.message})
}
fn io_error(error: io::Error) -> CliError {
    CliError::new("io", error.to_string())
}
fn usage(message: impl Into<String>) -> CliError {
    CliError::new("usage", message)
}
fn print_human(value: &Value) {
    if let Some(help) = value.get("help").and_then(Value::as_str) {
        println!("{help}");
    } else if let Some(version) = value.get("version").and_then(Value::as_str) {
        println!("work {version}");
    } else if let Some(root) = value.get("worktree_root").and_then(Value::as_str) {
        println!("worktree_root={root}");
        println!(
            "git_common_dir={}",
            value["git_common_dir"].as_str().unwrap_or("")
        );
    } else if let Some(items) = value.get("items").and_then(Value::as_array) {
        for item in items {
            println!(
                "{} {}",
                item["display_id"].as_str().unwrap_or("?"),
                item["title"].as_str().unwrap_or("")
            );
        }
    } else if let Some(item) = value.get("item") {
        println!(
            "{} {}",
            item["display_id"].as_str().unwrap_or("?"),
            item["title"].as_str().unwrap_or("")
        );
        println!("{}", serde_json::to_string_pretty(item).unwrap());
    } else {
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    }
}
const HELP: &str = "Usage: work [--json] [--worktree PATH] COMMAND\n\
Commands: discover [PATH], item create|list|inspect|diagnose|ready|update|close|reopen|repair, relation add|remove\n\
Use --json for one structured result or error object. Run item create --title TEXT [--body TEXT|-]; item update ID with --title, --completion, --priority, --parent, --label, --model, or --thinking. Use item repair FULL_ID --source - for malformed source.";
// Preserve unusual Unix path bytes while keeping JSON paths single-line.
fn encode_path(path: &Path) -> String {
    let mut encoded = String::new();
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'.' | b'_' | b'-' | b'~') {
            encoded.push(*byte as char);
        } else {
            write!(encoded, "%{byte:02X}").unwrap();
        }
    }
    encoded
}

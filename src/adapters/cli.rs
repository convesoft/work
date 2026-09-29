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

pub(super) struct CliError {
    code: &'static str,
    message: String,
    diagnostics: Vec<Diagnostic>,
    published_item: Option<Value>,
    previous_source_path: Option<PathBuf>,
}

impl CliError {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
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
    pub(super) fn value(&self) -> Value {
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
        if at + 2 == args.len() && args[at + 1] == "--help" {
            return Ok(json!({"help":DISCOVER_HELP}));
        }
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
    if let Some(help) = command_help(&words) {
        return Ok(json!({"help":help}));
    }
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

fn command_help(words: &[String]) -> Option<&'static str> {
    let words: Vec<_> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["discover", "--help"] => Some(DISCOVER_HELP),
        ["mcp", "--help"] => Some(
            "Usage: work mcp\nServe durable item operations as MCP tools over stdio. Tools accept an optional worktree path; otherwise they use the server's checkout.",
        ),
        ["item", "--help"] => Some(
            "Usage: work [--json] [--worktree PATH] item COMMAND\nCommands: create, list, inspect, diagnose, ready, update, close, reopen, repair. Run work item COMMAND --help for details.",
        ),
        ["item", "create", "--help"] => Some(
            "Usage: work item create --title TEXT [--body TEXT|-] [METADATA OPTIONS]\nCreate a durable item with a generated full ID; completion is manual by default. Body text is opaque; --body - reads stdin. Metadata: --completion manual|children, --priority 0..4, --parent ID, --label TEXT (repeatable), --model TEXT, --thinking TEXT.",
        ),
        ["item", "list", "--help"] => Some(
            "Usage: work item list\nList valid durable items in canonical ID order, including recorded state, graph state, relations, and blockers. Use item diagnose for malformed files.",
        ),
        ["item", "inspect", "--help"] => Some(
            "Usage: work item inspect ID [--raw]\nInspect an item by full ID or unique lowercase prefix, optionally prefixed with w-. --raw requires a full ID and exposes original source bytes and diagnostics.",
        ),
        ["item", "diagnose", "--help"] => Some(
            "Usage: work item diagnose\nReport source and graph diagnostics for the selected checkout. Invalid graphs can still be inspected, but refuse readiness and structured mutations.",
        ),
        ["item", "ready", "--help"] => Some(
            "Usage: work item ready\nList open executable manual items whose lifecycle prerequisites are resolved, ordered by priority then ID. This reports eligibility, not ownership or execution.",
        ),
        ["item", "update", "--help"] => Some(
            "Usage: work item update ID OPTIONS\nEdit supplied header fields only; preserve the existing body. Options: --title TEXT, --completion manual|children, --priority 0..4, --parent ID, --clear-parent, --label TEXT (repeatable), --clear-labels, --model TEXT, --clear-model, --thinking TEXT, --clear-thinking.",
        ),
        ["item", "close", "--help"] => Some(
            "Usage: work item close ID [--reason TEXT]\nRecord a manual item as done with an optional opaque reason. Closing resolves its obligation; it does not close other manual items.",
        ),
        ["item", "reopen", "--help"] => Some(
            "Usage: work item reopen ID\nRecord a manual item as open and remove its close reason. Graph state is recomputed without reopening other manual items.",
        ),
        ["item", "repair", "--help"] => Some(
            "Usage: work item repair FULL_ID --source -\nReplace a diagnosed invalid source with complete version-1 item bytes from stdin. Inspect and diagnose first; repair rejects a healthy item.",
        ),
        ["relation", "--help"] => Some(
            "Usage: work relation add|remove KIND SOURCE TARGET\nKinds: parent (child to parent), depends_on (dependent to prerequisite), related (symmetric context), discovered_from (provenance). Run work relation add --help or remove --help for details.",
        ),
        ["relation", "add", "--help"] => Some(
            "Usage: work relation add KIND SOURCE TARGET\nAdd one full-ID edge. For parent, the edge points from child to parent; for depends_on, from dependent to prerequisite. related and discovered_from are informational and do not change readiness.",
        ),
        ["relation", "remove", "--help"] => Some(
            "Usage: work relation remove KIND SOURCE TARGET\nRemove an existing edge. SOURCE is the child for parent and the dependent for depends_on. related may be removed from either endpoint.",
        ),
        _ => None,
    }
}

fn validate_command_shape(words: &[String]) -> Result<(), CliError> {
    match words {
        [noun, verb, tail @ ..] if noun == "item" => match verb.as_str() {
            "list" | "ready" | "diagnose" if tail.is_empty() => Ok(()),
            "inspect" if tail.len() == 1 || (tail.len() == 2 && tail[1] == "--raw") => Ok(()),
            "create" => validate_metadata_syntax(tail, true),
            "update" if tail.len() >= 2 => validate_metadata_syntax(&tail[1..], false),
            "close" if tail.len() == 1 || (tail.len() == 3 && tail[1] == "--reason") => Ok(()),
            "reopen" if tail.len() == 1 => Ok(()),
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

fn validate_metadata_syntax(args: &[String], create: bool) -> Result<(), CliError> {
    let mut at = 0;
    let mut has_title = false;
    while at < args.len() {
        let flag = args[at].as_str();
        let clear = matches!(
            flag,
            "--clear-parent" | "--clear-labels" | "--clear-model" | "--clear-thinking"
        );
        let known = matches!(
            flag,
            "--title"
                | "--completion"
                | "--priority"
                | "--parent"
                | "--label"
                | "--model"
                | "--thinking"
        ) || (create && flag == "--body")
            || clear;
        if !known {
            return Err(usage(format!("unknown option {flag}")));
        }
        if !clear && args.get(at + 1).is_none() {
            return Err(usage(format!("missing value for {flag}")));
        }
        if flag == "--title" {
            has_title = true;
        }
        at += if clear { 1 } else { 2 };
    }
    if create && !has_title {
        return Err(usage("item create requires --title"));
    }
    Ok(())
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
            Ok(json!({"item":mutation_item_value(project,&item)?}))
        }
        "update" if args.len() >= 2 => {
            let id = resolve(project, &args[0])?;
            let input = parse_metadata(project, &args[1..], true, false)?;
            let mut change = input.change;
            change.title = input.title;
            Ok(json!({"item":mutation_item_value(project,&ops.update(&id,change)?)?}))
        }
        "close" if !args.is_empty() => {
            let id = resolve(project, &args[0])?;
            let reason = match &args[1..] {
                [] => None,
                [flag, value] if flag == "--reason" => Some(value.clone()),
                _ => return Err(usage("item close ID [--reason TEXT]")),
            };
            Ok(json!({"item":mutation_item_value(project,&ops.close(&id,reason)?)?}))
        }
        "reopen" if args.len() == 1 => {
            let id = resolve(project, &args[0])?;
            Ok(json!({"item":mutation_item_value(project,&ops.reopen(&id)?)?}))
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
pub(super) fn source_value(id: &str, raw: &RawInspection) -> Value {
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
    Ok(json!({"item":mutation_item_value(project,&item)?}))
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
        validate_create_fields(title, &input.change)?;
    }
    if read_body_from_stdin {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes).map_err(io_error)?;
        input.body = Some(bytes);
    }
    Ok(input)
}

pub(super) fn validate_create_fields(title: &str, change: &MetadataChange) -> Result<(), CliError> {
    if !valid_line(title) {
        return Err(CliError::new(
            "invalid_argument",
            "title must be a nonblank single line",
        ));
    }
    if change
        .labels
        .as_ref()
        .is_some_and(|labels| labels.iter().any(|label| !valid_line(label)))
    {
        return Err(CliError::new(
            "invalid_argument",
            "labels must be nonblank single lines",
        ));
    }
    for (name, value) in [("model", &change.model), ("thinking", &change.thinking)] {
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
    Ok(())
}

fn valid_line(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains(['\n', '\r'])
}

pub(super) fn resolve(project: &Project, input: &str) -> Result<String, CliError> {
    let candidate = input.strip_prefix("w-").unwrap_or(input);
    if candidate.len() == 32 && !valid_full_id(candidate) {
        return Err(CliError::new(
            "invalid_argument",
            "full item ID must be a lowercase UUIDv4",
        ));
    }
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

fn valid_full_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && bytes[12] == b'4'
        && matches!(bytes[16], b'8' | b'9' | b'a' | b'b')
}

pub(super) fn one_item_value(project: &Project, item: &Inspection) -> Result<Value, CliError> {
    let store = ItemStore::load(project).map_err(io_error)?;
    item_value(&display_prefixes(&store), item)
}

pub(super) fn mutation_item_value(project: &Project, item: &Inspection) -> Result<Value, CliError> {
    let header = item
        .file
        .header
        .as_ref()
        .expect("successful mutation has a header");
    one_item_value(project, item)
        .map_err(|error| with_published_item(error, &header.id, &item.file.path))
}

pub(super) fn items_value(project: &Project, items: &[Inspection]) -> Result<Vec<Value>, CliError> {
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
pub(super) fn invalid_source(diagnostics: Vec<Diagnostic>) -> CliError {
    CliError {
        code: "invalid_source",
        message: "selected source or graph is invalid".into(),
        diagnostics,
        published_item: None,
        previous_source_path: None,
    }
}
pub(super) fn diagnostic(d: &Diagnostic) -> Value {
    json!({"path":encode_path(&d.path),"line":d.line,"message":d.message})
}
fn io_error(error: io::Error) -> CliError {
    CliError::new("io", error.to_string())
}
fn with_published_item(mut error: CliError, id: &str, path: &Path) -> CliError {
    error.published_item = Some(json!({"id":id,"path":encode_path(path)}));
    error
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
const HELP: &str = "Usage: work [--json] [--worktree PATH] COMMAND | work mcp\n\
Commands: discover [PATH], item create|list|inspect|diagnose|ready|update|close|reopen|repair, relation add|remove; mcp starts a stdio server\n\
Use --json for one structured result or error object. Run work item --help or work relation --help for command details. Work tracks item state and graph readiness; it does not execute work or impose a workflow.";
const DISCOVER_HELP: &str = "Usage: work discover [PATH]\nResolve a Git working checkout and its shared Git common directory. Omit PATH to use the current directory.";
// Preserve unusual Unix path bytes while keeping JSON paths single-line.
pub(super) fn encode_path(path: &Path) -> String {
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

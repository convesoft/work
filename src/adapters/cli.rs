use work::core::execution::ExecutionOperations;
// CLI presentation over shared operations.

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
    Inspection, MetadataChange, OperationError, RawInspection, RelationKind,
};
use work::core::project::{DiscoveryError, Project, discover};
use work::core::templates::{
    PreviewRequest, TemplateCatalog, TemplateDefinition, TemplateError, TemplatePreview,
};

pub(super) struct CliError {
    code: &'static str,
    message: String,
    diagnostics: Vec<Diagnostic>,
    published_item: Option<Box<Value>>,
    previous_source_path: Option<PathBuf>,
    storage_details: Option<Box<Value>>,
}

impl CliError {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            diagnostics: vec![],
            published_item: None,
            previous_source_path: None,
            storage_details: None,
        }
    }
    pub(super) fn with_details(code: &'static str, message: String, details: Value) -> Self {
        let mut error = Self::new(code, message);
        error.storage_details = Some(Box::new(details));
        error
    }
    pub(super) fn storage_failure(mut self) -> Self {
        if self.storage_details.is_none() {
            self.storage_details = Some(Box::new(json!({"publication":"not_published"})));
        }
        self
    }
    fn exit(&self) -> i32 {
        match self.code {
            "usage" | "invalid_argument" => 2,
            "not_found" | "ambiguous_id" => 3,
            "invalid_source" | "invalid_candidate" | "invalid_format" | "unsupported_format"
            | "storage_missing" | "storage_corrupt" | "recovery_required" | "identity_mismatch" => {
                4
            }
            "conflict" | "already_exists" | "storage_busy" | "unsafe_path" | "claim_conflict"
            | "stale_claim" | "not_ready" | "run_conflict" | "run_not_current"
            | "workspace_busy" | "reference_blocked" | "source_unavailable" => 5,
            _ => 1,
        }
    }
    pub(super) fn value(&self) -> Value {
        let mut v = json!({"code":self.code,"message":self.message});
        if let Some(details) = self.storage_details.as_deref().and_then(Value::as_object) {
            v.as_object_mut().unwrap().extend(details.clone());
        }
        if !self.diagnostics.is_empty() {
            v["diagnostics"] = json!(self.diagnostics.iter().map(diagnostic).collect::<Vec<_>>());
        }
        if let Some(p) = self.published_item.as_deref() {
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
                .map(|(id, path)| Box::new(json!({"id":id,"path":encode_path(path)}))),
            previous_source_path: error.previous_source_path().map(Path::to_path_buf),
            storage_details: None,
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
impl From<TemplateError> for CliError {
    fn from(error: TemplateError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
            diagnostics: error.diagnostics().to_vec(),
            published_item: None,
            previous_source_path: None,
            storage_details: None,
        }
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
    let mut duplicate_selector = false;
    while let Some(arg) = args.get(at) {
        if arg == "--json" {
            at += 1;
        } else if arg == "--worktree" {
            duplicate_selector |= selected.is_some();
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
    if duplicate_selector {
        let error = CliError::new("invalid_argument", "duplicate --worktree selector");
        return Err(
            if command == "storage"
                && args
                    .get(at + 1)
                    .is_some_and(|verb| verb == "init" || verb == "recreate" || verb == "recover")
            {
                error.storage_failure()
            } else {
                error
            },
        );
    }
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
    let storage_mutation = command == "storage"
        && args
            .get(at + 1)
            .is_some_and(|verb| verb == "init" || verb == "recreate" || verb == "recover");
    let words: Vec<String> = args[at..]
        .iter()
        .map(|a| {
            a.to_str()
                .map(str::to_owned)
                .ok_or_else(|| CliError::new("invalid_argument", "command arguments must be UTF-8"))
        })
        .collect::<Result<_, _>>()
        .map_err(|error| {
            if storage_mutation {
                error.storage_failure()
            } else {
                error
            }
        })?;
    if let Some(help) = command_help(&words) {
        return Ok(json!({"help":help}));
    }
    if words.first().is_some_and(|noun| noun == "storage") {
        let result = (|| {
            let request = super::storage::from_cli(&words[1..])?;
            super::storage::execute(&discover(selected)?, request)
        })();
        return if words
            .get(1)
            .is_some_and(|verb| matches!(verb.as_str(), "init" | "recreate" | "recover"))
        {
            result.map_err(CliError::storage_failure)
        } else {
            result
        };
    }
    if words.first().is_some_and(|w| w == "claim") {
        if words.get(1).is_some_and(|w| w == "--help") {
            return Ok(json!({"help":super::claims::HELP}));
        }
        let (verb, fields) = super::claims::from_cli(&words[1..])?;
        return super::claims::execute(&discover(selected)?, &verb, &fields);
    }
    if words.first().is_some_and(|s| s == "run")
        || (words.first().is_some_and(|s| s == "template")
            && words
                .get(1)
                .is_some_and(|s| matches!(s.as_str(), "preview" | "expand")))
    {
        if words.get(1).is_some_and(|s| s == "--help") {
            return Ok(json!({"help":super::runs::HELP}));
        }
        let (name, fields) = super::runs::from_cli(&words[0], &words[1..])?;
        return super::runs::execute(&discover(selected)?, &name, &fields);
    }
    let mut words = words;
    let mut authorizations = Vec::new();
    let mut checkout_view = false;
    let mut i = 2;
    while i < words.len() {
        if words[i] == "--authorize" {
            let value = words
                .get(i + 1)
                .ok_or_else(|| usage("--authorize requires JSON"))?;
            authorizations.push(
                serde_json::from_str(value).map_err(|_| usage("invalid authorization JSON"))?,
            );
            words.drain(i..i + 2);
        } else if words[i] == "--view" {
            if words[0] != "item"
                || !matches!(words[1].as_str(), "list" | "ready" | "inspect" | "diagnose")
            {
                return Err(usage("--view is read-only"));
            }
            match words.get(i + 1).map(String::as_str) {
                Some("checkout") => checkout_view = true,
                Some("resolved") => {}
                _ => return Err(usage("--view resolved|checkout")),
            };
            words.drain(i..i + 2);
        } else if matches!(
            words[i].as_str(),
            "--title"
                | "--body"
                | "--completion"
                | "--priority"
                | "--parent"
                | "--label"
                | "--model"
                | "--thinking"
                | "--reason"
                | "--source"
                | "--root"
                | "--param"
                | "--existing"
        ) {
            // Existing option values are opaque text, even when they resemble
            // a newly added authorization/view option. The command parser
            // remains responsible for missing or invalid values.
            i += 2;
        } else {
            i += 1;
        }
    }
    super::claims::authorization(Some(&Value::Array(authorizations.clone())))?;
    validate_command_shape(&words)?;
    let project = discover(selected)?;
    let mut ops = ExecutionOperations::new(project.clone());
    ops.authorization =
        super::claims::authorization(Some(&serde_json::Value::Array(authorizations)))?;
    ops.checkout_view = checkout_view;
    let result = match words.as_slice() {
        [noun, verb, tail @ ..] if noun == "item" => item_command(&project, &ops, verb, tail),
        [noun, verb, tail @ ..] if noun == "relation" => {
            relation_command(&project, &ops, verb, tail)
        }
        [noun, verb, tail @ ..] if noun == "template" => template_command(&project, verb, tail),
        _ => Err(usage(
            "expected item, relation, template, or storage command",
        )),
    }?;
    if words[0] == "item" && matches!(words[1].as_str(), "list" | "ready" | "diagnose" | "inspect")
    {
        Ok(super::storage::attach_read(
            &project,
            result,
            ops.take_read_storage(),
        ))
    } else {
        Ok(result)
    }
}

fn command_help(words: &[String]) -> Option<&'static str> {
    let words: Vec<_> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["discover", "--help"] => Some(DISCOVER_HELP),
        ["storage", "--help"] => Some(super::storage::HELP),
        [
            "storage",
            "inspect" | "init" | "recreate" | "recover",
            "--help",
        ] => Some(super::storage::HELP),
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
            "Usage: work item inspect ID [--view resolved|checkout] [--raw]\nInspect an item by full ID or unique lowercase prefix, optionally prefixed with w-. --raw reads the physical selected checkout and requires a full ID and exposes original source bytes and diagnostics.",
        ),
        ["item", "diagnose", "--help"] => Some(
            "Usage: work item diagnose\nReport source and graph diagnostics for the selected checkout. Invalid graphs can still be inspected, but refuse readiness and structured mutations.",
        ),
        ["item", "ready", "--help"] => Some(
            "Usage: work item ready [--view resolved|checkout]\nList open executable manual items whose lifecycle prerequisites are resolved, ordered by priority then ID. Resolved readiness excludes claimed items; inspect shows their owner.",
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
        ["template", "--help"] => Some(
            "Usage: work template list|validate|preview|expand\nDiscover version-1/version-2 YAML templates and render a symbolic item graph without publishing a run or items.",
        ),
        ["template", "list", "--help"] => Some(
            "Usage: work template list\nList valid and invalid templates under .work/templates/.",
        ),
        ["template", "validate", "--help"] => Some(
            "Usage: work template validate NAME\nValidate one version-1/version-2 YAML template definition.",
        ),
        ["template", "expand", "--help"] => Some(
            "Usage: work template expand NAME [--run RUN_ID] [--root ITEM] [--param NAME=TEXT]... [--existing NAME=ID]... [--authorize JSON]...\nPublish validated material/wisp files; failures can leave partial results.",
        ),
        ["template", "preview", "--help"] => Some(
            "Usage: work template preview NAME [--root FULL_ID] [--run RUN_ID] [--param NAME=TEXT]... [--existing NAME=FULL_ID]...\nRender the exact symbolic graph for a selected checkout without creating items, runs, IDs, or claims.",
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
        [noun, verb, tail @ ..] if noun == "template" => match (verb.as_str(), tail) {
            ("list", []) | ("validate", [_]) => Ok(()),
            ("preview", [_, rest @ ..]) if rest.len().is_multiple_of(2) => Ok(()),
            _ => Err(usage(
                "template list|validate NAME|preview NAME [--root FULL_ID] [--run RUN_ID] [--param NAME=TEXT]... [--existing NAME=FULL_ID]...",
            )),
        },
        _ => Err(usage("expected item, relation, or template command")),
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
    ops: &ExecutionOperations,
    verb: &str,
    args: &[String],
) -> Result<Value, CliError> {
    match verb {
        "list" if args.is_empty() => Ok(json!({"items":items_view(ops,&ops.list()?)?})),
        "ready" if args.is_empty() => Ok(json!({"items":items_view(ops,&ops.ready()?)?})),
        "diagnose" if args.is_empty() => {
            let store = ops.view()?;
            let graph = ItemGraph::from_store(&store);
            Ok(
                json!({"diagnostics":graph.diagnostics().iter().map(diagnostic).collect::<Vec<_>>()}),
            )
        }
        "inspect" if args.len() == 1 => {
            let candidate = args[0].strip_prefix("w-").unwrap_or(&args[0]);
            let id = match resolve_view(ops, &args[0]) {
                Ok(id) => id,
                Err(error) if error.code == "not_found" && candidate.len() == 32 => {
                    if let Ok(raw) = ops.inspect_raw(candidate) {
                        return Err(invalid_source(raw.file.diagnostics));
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            Ok(json!({"item":one_item_view(ops,&ops.inspect(&id)?)?}))
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

fn template_definition_value(definition: &TemplateDefinition) -> Value {
    json!({
        "name":definition.name,
        "parameters":definition.parameters,
        "existing":definition.existing,
        "defaults":{"model":definition.defaults.model,"thinking":definition.defaults.thinking},
        "items":definition.items.iter().map(|item| json!({
            "key":item.key,"title":item.title,"body":item.body,"persistence":item.persistence.as_str(),
            "completion":match item.completion {Completion::Manual=>"manual",Completion::Children=>"children"},
            "priority":item.priority,"labels":item.labels,"model":item.model,"thinking":item.thinking,
        })).collect::<Vec<_>>(),
        "edges":definition.edges.iter().map(|edge| json!({
            "from":edge.from,"kind":edge.kind.as_str(),"to":edge.to,
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn template_preview_value(preview: &TemplatePreview) -> Value {
    json!({
        "name":preview.name,"root":preview.root,
        "parameters":preview.parameters,"existing":preview.existing,
        "items":preview.items.iter().map(|item| (item.key.clone(), json!({
            "key":item.key,"title":item.title,"body":item.body,"persistence":item.persistence.as_str(),
            "completion":match item.completion {Completion::Manual=>"manual",Completion::Children=>"children"},
            "state":item.state.map(|state|match state {ManualState::Open=>"open",ManualState::Done=>"done"}),
            "priority":item.priority,"labels":item.labels,
            "model":item.model,"model_source":item.model_source.as_ref().map(|source|source.as_str()),
            "thinking":item.thinking,"thinking_source":item.thinking_source.as_ref().map(|source|source.as_str()),
        }))).collect::<BTreeMap<_,_>>(),
        "edges":preview.edges.iter().map(|edge| json!({
            "from":{"reference":edge.from.reference,"existing_id":edge.from.existing_id},
            "kind":edge.kind.as_str(),
            "to":{"reference":edge.to.reference,"existing_id":edge.to.existing_id},
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn template_command(
    project: &Project,
    verb: &str,
    args: &[String],
) -> Result<Value, CliError> {
    let catalog = TemplateCatalog::load_from_root(&project.worktree_root).map_err(io_error)?;
    match (verb, args) {
        ("list", []) => Ok(json!({"templates":catalog.files.iter().map(|file|json!({
            "name":file.name,"path":encode_path(&file.path),"valid":file.definition.is_some(),
            "diagnostics":file.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()})),
        ("validate", [name]) => {
            let definition = catalog.validate(name)?;
            Ok(json!({"template":template_definition_value(definition)}))
        }
        ("preview", [name, tail @ ..]) => {
            let mut root = None;
            let mut parameters = BTreeMap::new();
            let mut existing = BTreeMap::new();
            let mut chunks = tail.chunks_exact(2);
            for pair in &mut chunks {
                match pair[0].as_str() {
                    "--root" if root.is_none() => root = Some(pair[1].clone()),
                    "--param" | "--existing" => {
                        let (key, value) = pair[1].split_once('=').ok_or_else(|| {
                            CliError::new(
                                "invalid_argument",
                                format!("{} expects NAME=VALUE", pair[0]),
                            )
                        })?;
                        let target = if pair[0] == "--param" {
                            &mut parameters
                        } else {
                            &mut existing
                        };
                        if target.insert(key.to_owned(), value.to_owned()).is_some() {
                            return Err(CliError::new(
                                "invalid_argument",
                                format!("duplicate {} binding {key}", pair[0]),
                            ));
                        }
                    }
                    _ => {
                        return Err(usage(
                            "template preview NAME [--root FULL_ID] [--run RUN_ID] [--param NAME=TEXT]... [--existing NAME=FULL_ID]...",
                        ));
                    }
                }
            }
            if !chunks.remainder().is_empty() {
                return Err(usage("template preview options require values"));
            }
            let request = PreviewRequest {
                root,
                parameters,
                existing,
            };
            let view = ItemStore::load(project).map_err(io_error)?;
            Ok(json!({"preview":template_preview_value(&catalog.preview(name,&request,&view)?)}))
        }
        _ => Err(usage(
            "template list|validate NAME|preview NAME [--root FULL_ID] [--run RUN_ID] [--param NAME=TEXT]... [--existing NAME=FULL_ID]...",
        )),
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
    ops: &ExecutionOperations,
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
    resolve_view(&ExecutionOperations::new(project.clone()), input)
}
pub(super) fn resolve_view(ops: &ExecutionOperations, input: &str) -> Result<String, CliError> {
    let candidate = input.strip_prefix("w-").unwrap_or(input);
    if candidate.len() == 32 && !valid_full_id(candidate) {
        return Err(CliError::new(
            "invalid_argument",
            "full item ID must be a lowercase UUIDv4",
        ));
    }
    let store = ops.view()?;
    match store.resolve(input) {
        Ok(file) => Ok(file.header.as_ref().unwrap().id.clone()),
        Err(LookupError::InvalidInput) => Err(CliError::new(
            "invalid_argument",
            "item ID must be lowercase hexadecimal, optionally prefixed with w-",
        )),
        Err(LookupError::NotFound) => {
            // An unavailable bound source has no parsed header. Its resolved
            // diagnostic still owns this full ID; a healthy checkout copy
            // must not replace that evidence.
            let diagnostics: Vec<_> = store
                .files
                .iter()
                .filter(|file| file.path.file_stem().and_then(|s| s.to_str()) == Some(candidate))
                .flat_map(|file| file.diagnostics.clone())
                .collect();
            if diagnostics.is_empty() {
                Err(CliError::new(
                    "not_found",
                    format!("item {input} was not found"),
                ))
            } else {
                Err(invalid_source(diagnostics))
            }
        }
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

/// Render a completed operation's returned snapshot without another filesystem
/// read. Full display IDs stay unambiguous without reloading the catalog.
pub(super) fn snapshot_item_value(item: &Inspection) -> Result<Value, CliError> {
    let header = item
        .file
        .header
        .as_ref()
        .ok_or_else(|| CliError::new("invalid_source", "item header is invalid"))?;
    item_value(&BTreeMap::from([(header.id.clone(), 32)]), item)
}

pub(super) fn one_item_value(project: &Project, item: &Inspection) -> Result<Value, CliError> {
    one_item_view(&ExecutionOperations::new(project.clone()), item)
}
pub(super) fn one_item_view(
    ops: &ExecutionOperations,
    item: &Inspection,
) -> Result<Value, CliError> {
    let store = ops.view()?;
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

pub(super) fn items_view(
    ops: &ExecutionOperations,
    items: &[Inspection],
) -> Result<Vec<Value>, CliError> {
    let store = ops.view()?;
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
    let mut value = json!({
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
    });
    if let Some(context) = item.context.as_object() {
        value.as_object_mut().unwrap().extend(context.clone());
    }
    if !value["claim"].is_null() {
        let claim = value["claim"].clone();
        value["blockers"].as_array_mut().unwrap().push(json!({"kind":"claim","id":claim["id"],"actor":claim["actor"],"session":claim["session"]}));
    }
    Ok(value)
}
pub(super) fn invalid_source(diagnostics: Vec<Diagnostic>) -> CliError {
    CliError {
        code: "invalid_source",
        message: "selected source or graph is invalid".into(),
        diagnostics,
        published_item: None,
        previous_source_path: None,
        storage_details: None,
    }
}
pub(super) fn diagnostic(d: &Diagnostic) -> Value {
    json!({"path":encode_path(&d.path),"line":d.line,"message":d.message})
}
fn io_error(error: io::Error) -> CliError {
    CliError::new("io", error.to_string())
}
fn with_published_item(mut error: CliError, id: &str, path: &Path) -> CliError {
    error.published_item = Some(Box::new(json!({"id":id,"path":encode_path(path)})));
    error
}
fn usage(message: impl Into<String>) -> CliError {
    CliError::new("usage", message)
}
fn print_human(value: &Value) {
    super::storage::print_warning(value);
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
    } else if value.get("run_id").is_some() && value.get("updated").is_some_and(Value::is_array) {
        // Expansion records have keys and publication paths, not inspected-item
        // titles. Show their complete mapping and affected existing files.
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    } else if let Some(items) = value.get("items").and_then(Value::as_array) {
        for item in items {
            println!(
                "{} {}",
                item["display_id"].as_str().unwrap_or("?"),
                item["title"].as_str().unwrap_or("")
            );
            if let Some(path) = item["path"].as_str() {
                println!("  source: {path}");
            }
            if let Some(w) = item.get("ownership_warning") {
                eprintln!("work: ownership warning: {}", w["message"]);
            }
            if let Some(actor) = item["claim"]["actor"].as_str() {
                println!(
                    "  claimed by {actor} ({})",
                    item["claim"]["id"].as_str().unwrap_or("")
                );
            }
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
Commands: discover [PATH], item create|list|inspect|diagnose|ready|update|close|reopen|repair, relation add|remove, template list|validate|preview|expand, claim acquire|inspect|list|release|recover|reassign, run start|inspect|list|attach|detach, storage inspect|init|recreate|recover; mcp starts a stdio server\n\
Use --json for one structured result or error object. Run work item --help, work relation --help, work template --help, work claim --help, or work storage --help for details. Work tracks item state and graph readiness; it does not execute work or impose a workflow.";
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

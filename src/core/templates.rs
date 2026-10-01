//! Read-only discovery, validation, and symbolic preview and planning of version-1/2 templates.
//!
//! Preview uses temporary graph identities only inside validation. Its public
//! result names prospective items by template-local keys; publication owns IDs.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, openat, statat};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use super::graph::ItemGraph;
use super::items::{Completion, Diagnostic, ItemFile, ItemHeader, ItemStore, ManualState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    InvalidArgument(String),
    NotFound(String),
    InvalidTemplate(Vec<Diagnostic>),
    InvalidSource(Vec<Diagnostic>),
    InvalidCandidate(Vec<Diagnostic>),
    Io(String),
}

impl TemplateError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "invalid_argument",
            Self::NotFound(_) => "not_found",
            Self::InvalidTemplate(_) => "invalid_template",
            Self::InvalidSource(_) => "invalid_source",
            Self::InvalidCandidate(_) => "invalid_candidate",
            Self::Io(_) => "io",
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            Self::InvalidTemplate(d) | Self::InvalidSource(d) | Self::InvalidCandidate(d) => d,
            _ => &[],
        }
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(m) | Self::NotFound(m) | Self::Io(m) => write!(f, "{m}"),
            Self::InvalidTemplate(d) | Self::InvalidSource(d) | Self::InvalidCandidate(d) => {
                write!(f, "{}: {} diagnostic(s)", self.code(), d.len())
            }
        }
    }
}

impl std::error::Error for TemplateError {}

#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub name: String,
    pub path: PathBuf,
    pub definition: Option<TemplateDefinition>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct TemplateCatalog {
    pub files: Vec<TemplateFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDefinition {
    pub name: String,
    pub parameters: Vec<String>,
    pub existing: Vec<String>,
    pub defaults: HintDefaults,
    pub items: Vec<TemplateItem>,
    pub edges: Vec<TemplateEdge>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HintDefaults {
    pub model: Option<String>,
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persistence {
    Material,
    Wisp,
}

impl Persistence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::Wisp => "wisp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateItem {
    pub persistence: Persistence,
    pub key: String,
    pub title: String,
    pub body: String,
    pub completion: Completion,
    pub priority: u8,
    pub labels: Vec<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TemplateRelationKind {
    Parent,
    DependsOn,
    Related,
    DiscoveredFrom,
}

impl TemplateRelationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::DependsOn => "depends_on",
            Self::Related => "related",
            Self::DiscoveredFrom => "discovered_from",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TemplateEdge {
    pub from: String,
    pub kind: TemplateRelationKind,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRequest {
    pub root: Option<String>,
    pub parameters: BTreeMap<String, String>,
    pub existing: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintSource {
    Item,
    TemplateDefault,
}

impl HintSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::TemplateDefault => "template_default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewItem {
    pub persistence: Persistence,
    pub key: String,
    pub title: String,
    pub body: String,
    pub completion: Completion,
    pub state: Option<ManualState>,
    pub priority: u8,
    pub labels: Vec<String>,
    pub model: Option<String>,
    pub model_source: Option<HintSource>,
    pub thinking: Option<String>,
    pub thinking_source: Option<HintSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEndpoint {
    pub reference: String,
    /// Populated only for an existing item, never for a prospective local key.
    pub existing_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEdge {
    pub from: PreviewEndpoint,
    pub kind: TemplateRelationKind,
    pub to: PreviewEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePreview {
    pub name: String,
    pub root: Option<String>,
    pub parameters: BTreeMap<String, String>,
    pub existing: BTreeMap<String, String>,
    pub items: Vec<PreviewItem>,
    pub edges: Vec<PreviewEdge>,
}

impl TemplateCatalog {
    /// A missing template directory is an empty catalog; malformed entries stay visible.
    pub fn load_from_root(root: &Path) -> io::Result<Self> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let root_fd = openat(CWD, root, flags, Mode::empty()).map_err(io::Error::from)?;
        let work_fd = openat(&root_fd, ".work", flags, Mode::empty()).map_err(io::Error::from)?;
        let templates_fd = match openat(&work_fd, "templates", flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(Self { files: Vec::new() }),
            Err(error) => return Err(io::Error::from(error)),
        };
        let mut names = Vec::new();
        for entry in Dir::read_from(&templates_fd).map_err(io::Error::from)? {
            let entry = entry.map_err(io::Error::from)?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." {
                names.push(OsString::from_vec(name.to_vec()));
            }
        }
        names.sort();
        let files = names
            .into_iter()
            .map(|name| load_template(&templates_fd, root.join(".work/templates"), &name))
            .collect();
        Ok(Self { files })
    }

    pub fn validate(&self, name: &str) -> Result<&TemplateDefinition, TemplateError> {
        if !identifier(name) {
            return Err(TemplateError::InvalidArgument(
                "template name must match [a-z][a-z0-9_]*".into(),
            ));
        }
        let file = self
            .files
            .iter()
            .find(|file| file.name == name)
            .ok_or_else(|| TemplateError::NotFound(format!("template {name:?} was not found")))?;
        file.definition
            .as_ref()
            .ok_or_else(|| TemplateError::InvalidTemplate(file.diagnostics.clone()))
    }

    /// `view` is the complete selected durable and retained-run item view.
    pub fn preview(
        &self,
        name: &str,
        request: &PreviewRequest,
        view: &ItemStore,
    ) -> Result<TemplatePreview, TemplateError> {
        let template = self.validate(name)?;
        let path = &self
            .files
            .iter()
            .find(|file| file.name == name)
            .expect("validated template has a file")
            .path;
        preview_definition(template, path, request, view)
    }
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn valid_id(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 32
        && b.iter()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && b[12] == b'4'
        && matches!(b[16], b'8' | b'9' | b'a' | b'b')
}

fn diagnostic(path: &Path, line: Option<usize>, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        path: path.to_owned(),
        line,
        message: message.into(),
    }
}

fn load_template(directory: &OwnedFd, base: PathBuf, name: &OsStr) -> TemplateFile {
    let path = base.join(name);
    let name_str = name.to_str().unwrap_or("");
    let stem = name_str.strip_suffix(".yaml").unwrap_or("");
    let mut file = TemplateFile {
        name: stem.to_owned(),
        path: path.clone(),
        definition: None,
        diagnostics: Vec::new(),
    };
    if !identifier(stem) {
        file.diagnostics.push(diagnostic(
            &path,
            None,
            "template filename must be <name>.yaml with [a-z][a-z0-9_]* name",
        ));
        return file;
    }
    let raw = match read_regular(directory, name) {
        Ok(raw) => raw,
        Err(error) => {
            file.diagnostics.push(diagnostic(&path, None, error));
            return file;
        }
    };
    let source = match std::str::from_utf8(&raw) {
        Ok(source) if !source.starts_with('\u{feff}') => source,
        Ok(_) => {
            file.diagnostics.push(diagnostic(
                &path,
                Some(1),
                "UTF-8 byte-order mark is forbidden",
            ));
            return file;
        }
        Err(error) => {
            file.diagnostics
                .push(diagnostic(&path, None, format!("invalid UTF-8: {error}")));
            return file;
        }
    };
    match parse_yaml(source).and_then(|node| decode_template(node, stem).map_err(|e| (None, e))) {
        Ok(definition) => file.definition = Some(definition),
        Err((line, message)) => file.diagnostics.push(diagnostic(&path, line, message)),
    }
    file
}

fn read_regular(directory: &OwnedFd, name: &OsStr) -> Result<Vec<u8>, String> {
    let stat = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("cannot inspect template: {error}"))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err("template path must be a regular file, not a symlink".into());
    }
    let fd = openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open template: {error}"))?;
    let mut source = File::from(fd);
    if !source
        .metadata()
        .map_err(|error| format!("cannot inspect opened template: {error}"))?
        .is_file()
    {
        return Err("opened template must be a regular file".into());
    }
    let mut raw = Vec::new();
    source
        .read_to_end(&mut raw)
        .map_err(|error| format!("cannot read template: {error}"))?;
    Ok(raw)
}

#[derive(Debug)]
enum Node {
    String(String),
    Integer(i64),
    Other,
    Sequence(Vec<Node>),
    Mapping(BTreeMap<String, Node>),
}

struct Events(Vec<(Event, Marker)>);
impl MarkedEventReceiver for Events {
    fn on_event(&mut self, event: Event, marker: Marker) {
        self.0.push((event, marker));
    }
}

fn parse_yaml(source: &str) -> Result<Node, (Option<usize>, String)> {
    if let Some((line, _)) = source
        .lines()
        .enumerate()
        .find(|(_, line)| line.starts_with('%'))
    {
        return Err((Some(line + 1), "YAML directives are forbidden".into()));
    }
    let mut events = Events(Vec::new());
    Parser::new_from_str(source)
        .load(&mut events, true)
        .map_err(|error| {
            (
                Some(error.marker().line()),
                format!("invalid YAML: {error}"),
            )
        })?;
    let mut cursor = 0;
    expect(&events.0, &mut cursor, Event::StreamStart)?;
    expect(&events.0, &mut cursor, Event::DocumentStart)?;
    let node = parse_node(&events.0, &mut cursor)?;
    expect(&events.0, &mut cursor, Event::DocumentEnd)?;
    expect(&events.0, &mut cursor, Event::StreamEnd)?;
    if cursor != events.0.len() {
        return Err((None, "additional YAML documents are forbidden".into()));
    }
    Ok(node)
}

fn expect(
    events: &[(Event, Marker)],
    cursor: &mut usize,
    expected: Event,
) -> Result<(), (Option<usize>, String)> {
    match events.get(*cursor) {
        Some((event, _)) if *event == expected => {
            *cursor += 1;
            Ok(())
        }
        Some((_, marker)) => Err((Some(marker.line()), format!("expected {expected:?}"))),
        None => Err((None, format!("expected {expected:?}"))),
    }
}

fn parse_node(
    events: &[(Event, Marker)],
    cursor: &mut usize,
) -> Result<Node, (Option<usize>, String)> {
    let (event, marker) = events
        .get(*cursor)
        .ok_or((None, "incomplete YAML document".into()))?;
    let line = Some(marker.line());
    *cursor += 1;
    match event {
        Event::Alias(_) => Err((line, "YAML aliases are forbidden".into())),
        Event::Scalar(value, style, anchor, tag) => {
            if *anchor != 0 {
                return Err((line, "YAML anchors are forbidden".into()));
            }
            match standard_tag(tag.as_ref()).map_err(|e| (line, e))? {
                Some("str") => return Ok(Node::String(value.clone())),
                Some("int") => {
                    return tagged_integer(value)
                        .map(Node::Integer)
                        .map_err(|message| (line, message));
                }
                Some("float" | "bool" | "null") => return Ok(Node::Other),
                Some(_) => return Err((line, "scalar has a collection YAML tag".into())),
                None => {}
            }
            if *style != TScalarStyle::Plain {
                return Ok(Node::String(value.clone()));
            }
            if value.is_empty()
                || matches!(
                    value.as_str(),
                    "null"
                        | "Null"
                        | "NULL"
                        | "~"
                        | "true"
                        | "True"
                        | "TRUE"
                        | "false"
                        | "False"
                        | "FALSE"
                )
            {
                return Ok(Node::Other);
            }
            if core_decimal_integer(value) {
                if json_integer(value)
                    && let Ok(integer) = value.parse::<i64>()
                {
                    return Ok(Node::Integer(integer));
                }
                return Ok(Node::Other);
            }
            if core_float(value)
                || value.strip_prefix("0x").is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                || value.strip_prefix("0o").is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
                })
            {
                return Ok(Node::Other);
            }
            Ok(Node::String(value.clone()))
        }
        Event::SequenceStart(anchor, tag) => {
            if *anchor != 0 || !matches!(standard_tag(tag.as_ref()), Ok(None | Some("seq"))) {
                return Err((line, "invalid YAML sequence tag or anchor".into()));
            }
            let mut values = Vec::new();
            while !matches!(events.get(*cursor), Some((Event::SequenceEnd, _))) {
                values.push(parse_node(events, cursor)?);
            }
            *cursor += 1;
            Ok(Node::Sequence(values))
        }
        Event::MappingStart(anchor, tag) => {
            if *anchor != 0 || !matches!(standard_tag(tag.as_ref()), Ok(None | Some("map"))) {
                return Err((line, "invalid YAML mapping tag or anchor".into()));
            }
            let mut values = BTreeMap::new();
            while !matches!(events.get(*cursor), Some((Event::MappingEnd, _))) {
                let key_line = events.get(*cursor).map(|(_, m)| m.line());
                let key = match parse_node(events, cursor)? {
                    Node::String(key) => key,
                    _ => return Err((key_line, "YAML mapping keys must be strings".into())),
                };
                if key == "<<" {
                    return Err((key_line, "YAML merge keys are forbidden".into()));
                }
                let value = parse_node(events, cursor)?;
                if values.insert(key.clone(), value).is_some() {
                    return Err((key_line, format!("duplicate YAML key {key:?}")));
                }
            }
            *cursor += 1;
            Ok(Node::Mapping(values))
        }
        _ => Err((line, "unexpected YAML event".into())),
    }
}

fn standard_tag(tag: Option<&Tag>) -> Result<Option<&str>, String> {
    let Some(tag) = tag else { return Ok(None) };
    let name = if tag.handle == "tag:yaml.org,2002:" {
        tag.suffix.as_str()
    } else if tag.handle.is_empty() {
        tag.suffix
            .strip_prefix("tag:yaml.org,2002:")
            .ok_or("custom YAML tags are forbidden")?
    } else {
        return Err("custom YAML tags are forbidden".into());
    };
    if matches!(
        name,
        "str" | "int" | "float" | "bool" | "null" | "seq" | "map"
    ) {
        Ok(Some(name))
    } else {
        Err(format!("unsupported YAML tag {name:?}"))
    }
}

fn json_integer(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    !unsigned.is_empty()
        && (unsigned == "0"
            || (!unsigned.starts_with('0') && unsigned.bytes().all(|b| b.is_ascii_digit())))
}

fn core_decimal_integer(value: &str) -> bool {
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value);
    !unsigned.is_empty() && unsigned.bytes().all(|b| b.is_ascii_digit())
}

fn tagged_integer(value: &str) -> Result<i64, String> {
    let parsed = if let Some(digits) = value.strip_prefix("0x") {
        i64::from_str_radix(digits, 16)
    } else if let Some(digits) = value.strip_prefix("0o") {
        i64::from_str_radix(digits, 8)
    } else if core_decimal_integer(value) {
        value.parse::<i64>()
    } else {
        return Err("tagged integer has invalid YAML 1.2 syntax".into());
    };
    parsed.map_err(|_| "tagged integer has invalid YAML 1.2 syntax or range".into())
}

fn core_float(value: &str) -> bool {
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value);
    if matches!(unsigned, ".inf" | ".Inf" | ".INF") {
        return true;
    }
    if matches!(value, ".nan" | ".NaN" | ".NAN") {
        return true;
    }
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (&unsigned[..index], Some(&unsigned[index + 1..])),
        None => (unsigned, None),
    };
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    let Some((whole, fractional)) = mantissa.split_once('.') else {
        return exponent.is_some()
            && !mantissa.is_empty()
            && mantissa.bytes().all(|b| b.is_ascii_digit());
    };
    ((!whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit()))
        || (whole.is_empty() && !fractional.is_empty()))
        && fractional.bytes().all(|b| b.is_ascii_digit())
}

fn mapping(node: Node, context: &str) -> Result<BTreeMap<String, Node>, String> {
    match node {
        Node::Mapping(fields) => Ok(fields),
        _ => Err(format!("{context} must be a YAML mapping")),
    }
}
fn required_string(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<String, String> {
    match fields.remove(key) {
        Some(Node::String(value)) => Ok(value),
        None => Err(format!("missing required key {key}")),
        _ => Err(format!("{key} must be a string")),
    }
}
fn optional_string(
    fields: &mut BTreeMap<String, Node>,
    key: &str,
) -> Result<Option<String>, String> {
    match fields.remove(key) {
        Some(Node::String(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(format!("{key} must be a string")),
    }
}
fn optional_sequence(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<Vec<Node>, String> {
    match fields.remove(key) {
        Some(Node::Sequence(values)) => Ok(values),
        None => Ok(Vec::new()),
        _ => Err(format!("{key} must be a sequence")),
    }
}
fn no_extra(fields: &BTreeMap<String, Node>, context: &str) -> Result<(), String> {
    if let Some(key) = fields.keys().next() {
        Err(format!("unknown {context} field {key:?}"))
    } else {
        Ok(())
    }
}
fn names(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut seen = BTreeSet::new();
    for value in optional_sequence(fields, key)? {
        let Node::String(value) = value else {
            return Err(format!("{key} must contain identifier strings"));
        };
        if !identifier(&value) || !seen.insert(value.clone()) {
            return Err(format!(
                "{key} contains invalid or duplicate name {value:?}"
            ));
        }
        values.push(value);
    }
    Ok(values)
}

fn decode_template(node: Node, file_stem: &str) -> Result<TemplateDefinition, String> {
    let mut fields = mapping(node, "template")?;
    let version = match fields.remove("format_version") {
        Some(Node::Integer(version @ 1..=2)) => version,
        Some(Node::Integer(version)) => {
            return Err(format!(
                "unsupported format_version {version}; expected 1 or 2"
            ));
        }
        None => return Err("missing required key format_version".into()),
        _ => return Err("format_version must be integer 1 or 2".into()),
    };
    let name = required_string(&mut fields, "name")?;
    if name != file_stem {
        return Err(format!(
            "name {name:?} must match filename stem {file_stem:?}"
        ));
    }
    let parameters = names(&mut fields, "parameters")?;
    let existing = names(&mut fields, "existing")?;
    let mut all_names = BTreeSet::new();
    for name in parameters.iter().chain(&existing) {
        if !all_names.insert(name) {
            return Err(format!("parameter and existing names overlap at {name:?}"));
        }
    }
    let defaults = match fields.remove("defaults") {
        None => HintDefaults::default(),
        Some(value) => {
            let mut hints = mapping(value, "defaults")?;
            let model = optional_string(&mut hints, "model")?;
            let thinking = optional_string(&mut hints, "thinking")?;
            no_extra(&hints, "defaults")?;
            HintDefaults { model, thinking }
        }
    };
    let item_nodes = optional_sequence(&mut fields, "items")?;
    if item_nodes.is_empty() {
        return Err("items must be a nonempty sequence".into());
    }
    let mut items = Vec::new();
    let mut local_names = BTreeSet::new();
    for node in item_nodes {
        let mut item = mapping(node, "item")?;
        let key = required_string(&mut item, "key")?;
        if !identifier(&key) || !local_names.insert(key.clone()) || all_names.contains(&key) {
            return Err(format!(
                "invalid, duplicate, or overlapping item key {key:?}"
            ));
        }
        let title = required_string(&mut item, "title")?;
        let body = optional_string(&mut item, "body")?.unwrap_or_default();
        let completion = match optional_string(&mut item, "completion")?.as_deref() {
            None | Some("manual") => Completion::Manual,
            Some("children") => Completion::Children,
            _ => {
                return Err(format!(
                    "item {key:?} completion must be manual or children"
                ));
            }
        };
        let priority = match item.remove("priority") {
            None => 2,
            Some(Node::Integer(value @ 0..=4)) => value as u8,
            _ => return Err(format!("item {key:?} priority must be integer 0..4")),
        };
        let labels = optional_sequence(&mut item, "labels")?
            .into_iter()
            .map(|value| match value {
                Node::String(value) => Ok(value),
                _ => Err(format!("item {key:?} labels must contain strings")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let model = optional_string(&mut item, "model")?;
        let thinking = optional_string(&mut item, "thinking")?;
        let persistence = if version == 2 {
            match optional_string(&mut item, "persistence")?.as_deref() {
                None | Some("material") => Persistence::Material,
                Some("wisp") => Persistence::Wisp,
                _ => return Err(format!("item {key:?} persistence must be material or wisp")),
            }
        } else {
            Persistence::Material
        };
        no_extra(&item, &format!("item {key:?}"))?;
        items.push(TemplateItem {
            persistence,
            key,
            title,
            body,
            completion,
            priority,
            labels,
            model,
            thinking,
        });
    }
    let mut edges = Vec::new();
    let mut symbolic_edges = BTreeSet::new();
    for node in optional_sequence(&mut fields, "edges")? {
        let mut edge = mapping(node, "edge")?;
        let from = required_string(&mut edge, "from")?;
        let kind = match required_string(&mut edge, "kind")?.as_str() {
            "parent" => TemplateRelationKind::Parent,
            "depends_on" => TemplateRelationKind::DependsOn,
            "related" => TemplateRelationKind::Related,
            "discovered_from" => TemplateRelationKind::DiscoveredFrom,
            other => return Err(format!("unknown relation kind {other:?}")),
        };
        let to = required_string(&mut edge, "to")?;
        no_extra(&edge, "edge")?;
        for endpoint in [&from, &to] {
            check_reference(endpoint, &local_names, &existing)?;
        }
        if from == to {
            return Err(format!("self-relation {} at {from}", kind.as_str()));
        }
        let semantic = if kind == TemplateRelationKind::Related && from > to {
            (kind.clone(), to.clone(), from.clone())
        } else {
            (kind.clone(), from.clone(), to.clone())
        };
        if !symbolic_edges.insert(semantic) {
            return Err(format!("duplicate semantic {} edge", kind.as_str()));
        }
        edges.push(TemplateEdge { from, kind, to });
    }
    no_extra(&fields, "template")?;
    let mut used = BTreeSet::new();
    let declared: BTreeSet<_> = parameters.iter().map(String::as_str).collect();
    for (default, inherited) in [
        (
            &defaults.model,
            items.iter().any(|item| item.model.is_none()),
        ),
        (
            &defaults.thinking,
            items.iter().any(|item| item.thinking.is_none()),
        ),
    ] {
        if let Some(text) = default {
            validate_template_line(text, "hint default")?;
            let mut default_uses = BTreeSet::new();
            scan_tokens(text, &declared, &mut default_uses)?;
            if inherited {
                used.extend(default_uses);
            }
        }
    }
    for item in &items {
        validate_template_line(&item.title, "title")?;
        let mut labels = BTreeSet::new();
        for label in &item.labels {
            validate_template_line(label, "label")?;
            if !labels.insert(label) {
                return Err(format!("item {:?} has duplicate label {label:?}", item.key));
            }
        }
        for text in item.model.iter().chain(&item.thinking) {
            validate_template_line(text, "hint")?;
        }
        for text in [&item.title, &item.body]
            .into_iter()
            .chain(item.labels.iter())
            .chain(item.model.iter())
            .chain(item.thinking.iter())
        {
            scan_tokens(text, &declared, &mut used)?;
        }
    }
    if let Some(unused) = parameters.iter().find(|name| !used.contains(name.as_str())) {
        return Err(format!("unused parameter declaration {unused:?}"));
    }
    items.sort_by(|a, b| a.key.cmp(&b.key));
    edges.sort();
    Ok(TemplateDefinition {
        name,
        parameters,
        existing,
        defaults,
        items,
        edges,
    })
}

fn validate_template_line(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains(['\r', '\n']) {
        Err(format!("{field} must be a nonblank single-line string"))
    } else {
        Ok(())
    }
}

fn check_reference(
    value: &str,
    locals: &BTreeSet<String>,
    existing: &[String],
) -> Result<(), String> {
    if value == "root" {
        return Ok(());
    }
    if let Some(key) = value.strip_prefix("local:") {
        if locals.contains(key) {
            return Ok(());
        }
        return Err(format!("unknown local reference {value:?}"));
    }
    if let Some(name) = value.strip_prefix("existing:") {
        if existing.iter().any(|value| value == name) {
            return Ok(());
        }
        return Err(format!("unknown existing reference {value:?}"));
    }
    Err(format!("invalid endpoint reference {value:?}"))
}

fn scan_tokens(
    text: &str,
    declared: &BTreeSet<&str>,
    used: &mut BTreeSet<String>,
) -> Result<(), String> {
    let mut rest = text;
    loop {
        let open = rest.find("{{");
        let close = rest.find("}}");
        match (open, close) {
            (None, None) => return Ok(()),
            (None, Some(_)) => return Err("unmatched }} token delimiter".into()),
            (Some(o), Some(c)) if c < o => return Err("unmatched }} token delimiter".into()),
            (Some(o), _) => {
                let start = o + 2;
                let Some(relative_end) = rest[start..].find("}}") else {
                    return Err("unmatched {{ token delimiter".into());
                };
                let end = start + relative_end;
                if rest[start..end].contains("{{") {
                    return Err("nested {{ token delimiter".into());
                }
                let name = &rest[start..end];
                if !identifier(name) || !declared.contains(name) {
                    return Err(format!("undeclared or malformed token {{{{{name}}}}}"));
                }
                used.insert(name.to_owned());
                rest = &rest[end + 2..];
            }
        }
    }
}

fn render(text: &str, values: &BTreeMap<String, String>) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let name_start = start + 2;
        let end = name_start + rest[name_start..].find("}}").expect("validated token");
        result.push_str(&values[&rest[name_start..end]]);
        rest = &rest[end + 2..];
    }
    result.push_str(rest);
    result
}

fn preview_definition(
    template: &TemplateDefinition,
    template_path: &Path,
    request: &PreviewRequest,
    view: &ItemStore,
) -> Result<TemplatePreview, TemplateError> {
    let graph = ItemGraph::from_store(view);
    if !graph.is_valid() {
        return Err(TemplateError::InvalidSource(graph.diagnostics().to_vec()));
    }
    let present: BTreeSet<_> = view
        .files
        .iter()
        .filter_map(|file| file.header.as_ref().map(|header| header.id.clone()))
        .collect();
    if let Some(root) = &request.root {
        if !valid_id(root) || !present.contains(root) {
            return Err(TemplateError::InvalidArgument(
                "root must resolve to a full UUIDv4 ID in the selected view".into(),
            ));
        }
    } else if template
        .edges
        .iter()
        .any(|e| e.from == "root" || e.to == "root")
    {
        return Err(TemplateError::InvalidArgument(
            "root is required by template edges".into(),
        ));
    }
    check_arguments(&template.parameters, &request.parameters, "parameter")?;
    check_arguments(&template.existing, &request.existing, "existing binding")?;
    for (name, id) in &request.existing {
        if !valid_id(id) || !present.contains(id) {
            return Err(TemplateError::InvalidArgument(format!(
                "existing binding {name:?} must resolve to a full ID in the selected view"
            )));
        }
    }
    let mut items = Vec::new();
    for item in &template.items {
        let title = render(&item.title, &request.parameters);
        let body = render(&item.body, &request.parameters);
        let labels = item
            .labels
            .iter()
            .map(|label| render(label, &request.parameters))
            .collect::<Vec<_>>();
        let (model, model_source) = render_hint(
            item.model.as_deref(),
            template.defaults.model.as_deref(),
            &request.parameters,
        );
        let (thinking, thinking_source) = render_hint(
            item.thinking.as_deref(),
            template.defaults.thinking.as_deref(),
            &request.parameters,
        );
        validate_line(&title, "title")?;
        let mut seen_labels = BTreeSet::new();
        for label in &labels {
            validate_line(label, "label")?;
            if !seen_labels.insert(label) {
                return Err(TemplateError::InvalidArgument(format!(
                    "item {:?} renders duplicate label {label:?}",
                    item.key
                )));
            }
        }
        for (field, value) in [("model", &model), ("thinking", &thinking)] {
            if let Some(value) = value {
                validate_line(value, field)?;
            }
        }
        items.push(PreviewItem {
            persistence: item.persistence,
            key: item.key.clone(),
            title,
            body,
            completion: item.completion,
            state: (item.completion == Completion::Manual).then_some(ManualState::Open),
            priority: item.priority,
            labels,
            model,
            model_source,
            thinking,
            thinking_source,
        });
    }

    // Assign deterministic, collision-free validation identities only in memory.
    // Neither these IDs nor source paths appear in a successful preview result.
    let mut serial = 1u32;
    let mut local_ids = BTreeMap::new();
    let mut occupied = present;
    for item in &items {
        let id = loop {
            let id = format!("{serial:08x}000040008000000000000000");
            serial = serial
                .checked_add(1)
                .ok_or_else(|| TemplateError::InvalidArgument("too many preview items".into()))?;
            if occupied.insert(id.clone()) {
                break id;
            }
        };
        local_ids.insert(item.key.clone(), id);
    }
    let (_, mut edges) = prospective_files(
        &items,
        &template.edges,
        template_path,
        request,
        view,
        &local_ids,
    )?;
    edges.sort_by(|a, b| {
        (&a.from.reference, &a.kind, &a.to.reference).cmp(&(
            &b.from.reference,
            &b.kind,
            &b.to.reference,
        ))
    });
    Ok(TemplatePreview {
        name: template.name.clone(),
        root: request.root.clone(),
        parameters: request.parameters.clone(),
        existing: request.existing.clone(),
        items,
        edges,
    })
}

fn check_arguments(
    expected: &[String],
    supplied: &BTreeMap<String, String>,
    kind: &str,
) -> Result<(), TemplateError> {
    let expected: BTreeSet<_> = expected.iter().map(String::as_str).collect();
    for name in expected.iter() {
        if !supplied.contains_key(*name) {
            return Err(TemplateError::InvalidArgument(format!(
                "missing {kind} {name:?}"
            )));
        }
    }
    for name in supplied.keys() {
        if !expected.contains(name.as_str()) {
            return Err(TemplateError::InvalidArgument(format!(
                "unknown {kind} {name:?}"
            )));
        }
    }
    Ok(())
}

fn validate_line(value: &str, field: &str) -> Result<(), TemplateError> {
    if value.trim().is_empty() || value.contains(['\r', '\n']) {
        Err(TemplateError::InvalidArgument(format!(
            "rendered {field} must be a nonblank single-line string"
        )))
    } else {
        Ok(())
    }
}

fn render_hint(
    explicit: Option<&str>,
    default: Option<&str>,
    parameters: &BTreeMap<String, String>,
) -> (Option<String>, Option<HintSource>) {
    match (explicit, default) {
        (Some(value), _) => (Some(render(value, parameters)), Some(HintSource::Item)),
        (None, Some(value)) => (
            Some(render(value, parameters)),
            Some(HintSource::TemplateDefault),
        ),
        (None, None) => (None, None),
    }
}

fn endpoint(reference: &str, request: &PreviewRequest) -> PreviewEndpoint {
    let existing_id = if reference == "root" {
        request.root.clone()
    } else {
        reference
            .strip_prefix("existing:")
            .map(|name| request.existing[name].clone())
    };
    PreviewEndpoint {
        reference: reference.into(),
        existing_id,
    }
}

fn prospective_files(
    items: &[PreviewItem],
    template_edges: &[TemplateEdge],
    template_path: &Path,
    request: &PreviewRequest,
    view: &ItemStore,
    local_ids: &BTreeMap<String, String>,
) -> Result<(Vec<ItemFile>, Vec<PreviewEdge>), TemplateError> {
    let mut files = view.files.clone();
    for item in items {
        let id = local_ids[&item.key].clone();
        files.push(ItemFile {
            path: template_path.to_owned(),
            raw: Vec::new(),
            header: Some(ItemHeader {
                id,
                title: item.title.clone(),
                completion: item.completion,
                state: item.state,
                priority: item.priority,
                parent: None,
                depends_on: Vec::new(),
                related: Vec::new(),
                discovered_from: Vec::new(),
                labels: item.labels.clone(),
                model: item.model.clone(),
                thinking: item.thinking.clone(),
                close_reason: None,
            }),
            body: Some(item.body.as_bytes().to_vec()),
            diagnostics: Vec::new(),
            fingerprint: None,
        });
    }
    let mut edges = Vec::new();
    let mut semantic = BTreeSet::new();
    for edge in template_edges {
        let from = endpoint(&edge.from, request);
        let to = endpoint(&edge.to, request);
        let source_id = from
            .existing_id
            .as_ref()
            .or_else(|| local_ids.get(edge.from.strip_prefix("local:").unwrap_or("")))
            .expect("validated source reference");
        let target_id = to
            .existing_id
            .as_ref()
            .or_else(|| local_ids.get(edge.to.strip_prefix("local:").unwrap_or("")))
            .expect("validated target reference");
        if source_id == target_id {
            return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                template_path,
                None,
                format!(
                    "self-relation {} from {} to {}",
                    edge.kind.as_str(),
                    edge.from,
                    edge.to
                ),
            )]));
        }
        let (a, b) = if edge.kind == TemplateRelationKind::Related && source_id > target_id {
            (target_id.clone(), source_id.clone())
        } else {
            (source_id.clone(), target_id.clone())
        };
        if !semantic.insert((edge.kind.clone(), a, b)) {
            return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                template_path,
                None,
                format!("duplicate semantic {} edge", edge.kind.as_str()),
            )]));
        }
        if edge.kind == TemplateRelationKind::Related
            && files.iter().any(|file| {
                file.header.as_ref().is_some_and(|h| {
                    (&h.id == source_id && h.related.contains(target_id))
                        || (&h.id == target_id && h.related.contains(source_id))
                })
            })
        {
            return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                template_path,
                None,
                "duplicate related edge",
            )]));
        }
        let source = files
            .iter_mut()
            .find(|file| file.header.as_ref().is_some_and(|h| &h.id == source_id))
            .expect("resolved source is in view");
        let header = source
            .header
            .as_mut()
            .expect("resolved source has a header");
        match edge.kind {
            TemplateRelationKind::Parent => {
                if header.parent.is_some() {
                    return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                        template_path,
                        None,
                        format!("item {} already has a parent", edge.from),
                    )]));
                }
                header.parent = Some(target_id.clone());
            }
            TemplateRelationKind::DependsOn => {
                if header.depends_on.contains(target_id) {
                    return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                        template_path,
                        None,
                        "duplicate depends_on edge",
                    )]));
                }
                header.depends_on.push(target_id.clone());
            }
            TemplateRelationKind::Related => {
                if header.related.contains(target_id) {
                    return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                        template_path,
                        None,
                        "duplicate related edge",
                    )]));
                }
                header.related.push(target_id.clone());
            }
            TemplateRelationKind::DiscoveredFrom => {
                if header.discovered_from.contains(target_id) {
                    return Err(TemplateError::InvalidCandidate(vec![diagnostic(
                        template_path,
                        None,
                        "duplicate discovered_from edge",
                    )]));
                }
                header.discovered_from.push(target_id.clone())
            }
        }
        edges.push(PreviewEdge {
            from,
            kind: edge.kind.clone(),
            to,
        });
    }
    let candidate = ItemGraph::from_store(&ItemStore::from_candidate_files(files.clone()));
    if !candidate.is_valid() {
        let mut diagnostics = candidate.diagnostics().to_vec();
        for diagnostic in &mut diagnostics {
            for (key, id) in local_ids {
                diagnostic.message = diagnostic.message.replace(id, &format!("local:{key}"));
            }
        }
        return Err(TemplateError::InvalidCandidate(diagnostics));
    }
    Ok((files, edges))
}

/// Generated files are in lexical template-key order. Existing changes retain
/// their original `raw` and fingerprint for source authorization/rechecking;
/// their header is the proposed header and their opaque body is unchanged.
#[derive(Debug, Clone)]
pub struct ExpansionPlan {
    pub key_ids: BTreeMap<String, String>,
    pub items: Vec<PlannedItem>,
    pub updated: Vec<ItemFile>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlannedItem {
    pub key: String,
    pub id: String,
    pub persistence: Persistence,
    pub path: PathBuf,
    pub header: ItemHeader,
    pub body: Vec<u8>,
    pub raw: Vec<u8>,
}

impl ExpansionPlan {
    /// Add only progress known by the coordinator. Preserve the last write's
    /// underlying publication/error evidence, including possible publication.
    /// No retry record is written and no files are removed.
    pub fn partial_error(
        &self,
        mut error: super::coordination::ExecutionError,
        created: &[String],
        updated: &[String],
        uncertain_paths: &[PathBuf],
    ) -> super::coordination::ExecutionError {
        use super::coordination::encode_path;
        use serde_json::json;
        error.details["key_ids"] = json!(self.key_ids);
        let mut partial = json!({
            "created":self.items.iter().filter(|item| created.contains(&item.id)).map(|item| json!({"id":item.id,"path":encode_path(&item.path)})).collect::<Vec<_>>(),
            "updated":self.updated.iter().filter(|file| file.header.as_ref().is_some_and(|h| updated.contains(&h.id))).map(|file| json!({"id":file.header.as_ref().unwrap().id,"path":encode_path(&file.path)})).collect::<Vec<_>>(),
            "deleted":[], "uncertain_paths":uncertain_paths.iter().map(|p| encode_path(p)).collect::<Vec<_>>()
        });
        if let Some(prior) = error.details.get("partial") {
            for key in ["created", "updated", "deleted", "uncertain_paths"] {
                if let Some(records) = prior[key].as_array() {
                    for record in records {
                        let target = partial[key].as_array_mut().unwrap();
                        if !target.contains(record) {
                            target.push(record.clone());
                        }
                    }
                }
            }
        }
        error.details["partial"] = partial;
        if error.details.get("publication").is_none() {
            error.details["publication"] = json!("not_published");
        }
        error
    }
}

impl TemplateCatalog {
    /// Read-only expansion planning. Main supplies the complete resolved view,
    /// authorizes/rechecks every `updated` source and publishes under shared then
    /// checkout locks. Bindings precede new material files; run membership is last.
    /// Run selection has already been loaded under the held coordination guard.
    pub fn plan_expansion(
        &self,
        name: &str,
        request: &PreviewRequest,
        view: &ItemStore,
        material_root: &Path,
        run: Option<&super::runs::RunManifest>,
        shared_root: &Path,
    ) -> Result<ExpansionPlan, TemplateError> {
        if !material_root.is_absolute() || !shared_root.is_absolute() {
            return Err(TemplateError::InvalidArgument(
                "expansion destinations must be absolute discovered paths".into(),
            ));
        }
        let mut request = request.clone();
        if let Some(run) = run {
            if run.phase != super::runs::RunPhase::Active {
                return Err(TemplateError::InvalidArgument(
                    "target run does not accept expansion".into(),
                ));
            }
            if request
                .root
                .as_ref()
                .is_some_and(|root| root != &run.root_item_id)
            {
                return Err(TemplateError::InvalidArgument(
                    "explicit root differs from target run root".into(),
                ));
            }
            request.root = Some(run.root_item_id.clone());
        }
        let preview = self.preview(name, &request, view)?;
        if run.is_none()
            && preview
                .items
                .iter()
                .any(|i| i.persistence == Persistence::Wisp)
        {
            return Err(TemplateError::InvalidArgument(
                "wisp creation requires an explicit current run".into(),
            ));
        }
        let template = self.validate(name)?;
        let path = &self
            .files
            .iter()
            .find(|f| f.name == name)
            .expect("validated template has file")
            .path;
        let mut occupied: BTreeSet<_> = view
            .files
            .iter()
            .filter_map(|f| f.header.as_ref().map(|h| h.id.clone()))
            .collect();
        let mut key_ids = BTreeMap::new();
        for item in &preview.items {
            let id = loop {
                let id = super::coordination::new_id()
                    .map_err(|error| TemplateError::Io(error.to_string()))?;
                if occupied.insert(id.clone()) {
                    break id;
                }
            };
            key_ids.insert(item.key.clone(), id);
        }
        let (files, _) = prospective_files(
            &preview.items,
            &template.edges,
            path,
            &request,
            view,
            &key_ids,
        )?;
        let mut items = Vec::new();
        for item in &preview.items {
            let id = &key_ids[&item.key];
            let candidate = files
                .iter()
                .find(|f| f.header.as_ref().is_some_and(|h| &h.id == id))
                .expect("generated candidate exists");
            let header = candidate.header.clone().expect("candidate header");
            let body = item.body.as_bytes().to_vec();
            let path = match item.persistence {
                Persistence::Material => material_root.join(".work/items").join(format!("{id}.md")),
                Persistence::Wisp => shared_root
                    .join("runs")
                    .join(&run.expect("wisp requires run").id)
                    .join("items")
                    .join(format!("{id}.md")),
            };
            let raw = super::operations::serialize(&header, &body);
            let parsed = super::items::parse_candidate(path.clone(), raw.clone());
            if !parsed.is_valid() {
                return Err(TemplateError::InvalidCandidate(parsed.diagnostics));
            }
            items.push(PlannedItem {
                key: item.key.clone(),
                id: id.clone(),
                persistence: item.persistence,
                path,
                header,
                body,
                raw,
            });
        }
        let mut updated = Vec::new();
        for original in &view.files {
            let Some(header) = &original.header else {
                continue;
            };
            let proposed = files
                .iter()
                .find(|f| f.header.as_ref().is_some_and(|h| h.id == header.id))
                .expect("existing source retained");
            if proposed.header != original.header {
                updated.push(proposed.clone());
            }
        }
        updated.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(ExpansionPlan {
            key_ids,
            items,
            updated,
            run_id: run.map(|r| r.id.clone()),
        })
    }
}

//! Read-only loading of version-1 durable items.
//!
//! Every Markdown file remains in `ItemStore::files`, including malformed ones.
//! Consumers can inspect its original bytes and diagnostics before repairing it.

use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, openat, statat};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use super::project::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemHeader {
    pub id: String,
    pub title: String,
    pub completion: Completion,
    pub state: Option<ManualState>,
    pub priority: u8,
    pub parent: Option<String>,
    pub depends_on: Vec<String>,
    pub related: Vec<String>,
    pub discovered_from: Vec<String>,
    pub labels: Vec<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Manual,
    Children,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualState {
    Open,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}", self.path.display(), self.message),
            None => write!(f, "{}: {}", self.path.display(), self.message),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ItemFile {
    pub path: PathBuf,
    /// Original file bytes, including the header and all line endings.
    pub raw: Vec<u8>,
    pub header: Option<ItemHeader>,
    /// Exact bytes after the closing frontmatter delimiter, when framing is valid.
    pub body: Option<Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
    pub(crate) fingerprint: Option<FileFingerprint>,
}

/// Identity and metadata observed while loading a directory entry. A link's
/// destination is part of its identity for publication conflict checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub size: u64,
    pub mtime: (i64, i64),
    pub ctime: (i64, i64),
    pub link_target: Option<PathBuf>,
}

impl FileFingerprint {
    pub fn capture(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        let link_target = if metadata.file_type().is_symlink() {
            Some(std::fs::read_link(path)?)
        } else {
            None
        };
        Ok(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            mode: metadata.mode(),
            size: metadata.size(),
            mtime: (metadata.mtime(), metadata.mtime_nsec()),
            ctime: (metadata.ctime(), metadata.ctime_nsec()),
            link_target,
        })
    }

    pub fn same_identity_and_mode(&self, other: &Self) -> bool {
        self.dev == other.dev
            && self.ino == other.ino
            && self.mode == other.mode
            && self.link_target == other.link_target
    }
}

impl ItemFile {
    pub fn is_valid(&self) -> bool {
        self.header.is_some() && self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ItemStore {
    pub files: Vec<ItemFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    InvalidInput,
    NotFound,
    Ambiguous(Vec<String>),
    Invalid(Vec<Diagnostic>),
}

impl ItemStore {
    /// Load directly from the selected checkout, with no database or writes.
    pub fn load(project: &Project) -> std::io::Result<Self> {
        Self::load_from_root(&project.worktree_root)
    }

    /// Useful for isolated copies of the authored backlog.
    pub fn load_from_root(root: &Path) -> std::io::Result<Self> {
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let root_fd = openat(CWD, root, directory_flags, Mode::empty()).map_err(io::Error::from)?;
        let work_fd = open_real_subdirectory(&root_fd, ".work", directory_flags)?;
        let items_fd = open_real_subdirectory(&work_fd, "items", directory_flags)?;
        Self::load_from_open_items(&items_fd, &root.join(".work/items"))
    }

    fn load_from_open_items(items_fd: &OwnedFd, dir_path: &Path) -> io::Result<Self> {
        let mut names = Vec::new();
        for entry in Dir::read_from(items_fd).map_err(io::Error::from)? {
            let entry = entry.map_err(io::Error::from)?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." && !name.starts_with(b".operation-") {
                names.push(OsString::from_vec(name.to_vec()));
            }
        }
        names.sort();
        let files: Vec<_> = names
            .into_iter()
            .map(|name| load_file(dir_path.join(&name), items_fd, &name))
            .collect();
        Ok(Self::from_candidate_files(files))
    }

    /// Rebuild identity diagnostics after replacing or moving candidate files.
    pub(crate) fn from_candidate_files(mut files: Vec<ItemFile>) -> Self {
        for file in &mut files {
            file.diagnostics
                .retain(|diagnostic| !diagnostic.message.starts_with("duplicate item ID "));
        }
        let mut by_id: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, file) in files.iter().enumerate() {
            if let Some(header) = &file.header {
                by_id.entry(header.id.clone()).or_default().push(index);
            }
        }
        for (id, indices) in by_id {
            if indices.len() > 1 {
                for index in indices {
                    let path = files[index].path.clone();
                    files[index].diagnostics.push(Diagnostic {
                        path,
                        line: None,
                        message: format!("duplicate item ID {id}"),
                    });
                }
            }
        }
        Self { files }
    }

    pub fn is_valid(&self) -> bool {
        self.files.iter().all(ItemFile::is_valid)
    }

    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.files.iter().flat_map(|file| &file.diagnostics)
    }

    /// Resolve a full ID or an unambiguous displayed `w-` prefix.
    pub fn resolve(&self, input: &str) -> Result<&ItemFile, LookupError> {
        let prefix = input.strip_prefix("w-").unwrap_or(input);
        if prefix.is_empty()
            || prefix.len() > 32
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(LookupError::InvalidInput);
        }
        let matches: Vec<_> = self
            .files
            .iter()
            .filter(|file| {
                file.header
                    .as_ref()
                    .is_some_and(|header| header.id.starts_with(prefix))
            })
            .collect();
        match matches.as_slice() {
            [] => Err(LookupError::NotFound),
            [file] if file.is_valid() => Ok(file),
            [file] => Err(LookupError::Invalid(file.diagnostics.clone())),
            _ => Err(LookupError::Ambiguous(
                matches
                    .iter()
                    .filter_map(|file| file.header.as_ref().map(|header| header.id.clone()))
                    .collect(),
            )),
        }
    }
}

fn open_real_subdirectory(parent: &OwnedFd, name: &str, flags: OFlags) -> io::Result<OwnedFd> {
    openat(parent, name, flags, Mode::empty()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{name} must be a real directory: {error}"),
        )
    })
}

fn load_file(path: PathBuf, directory: &OwnedFd, name: &OsStr) -> ItemFile {
    let before = FileFingerprint::capture(&path).ok();
    let mut file = match read_regular_item(directory, name) {
        Ok(raw) => parse_candidate(path, raw),
        Err(message) => ItemFile {
            path: path.clone(),
            raw: Vec::new(),
            header: None,
            body: None,
            diagnostics: vec![Diagnostic {
                path,
                line: None,
                message,
            }],
            fingerprint: None,
        },
    };
    if before != FileFingerprint::capture(&file.path).ok() {
        file.diagnostics.push(Diagnostic {
            path: file.path.clone(),
            line: None,
            message: "item changed while reading".into(),
        });
    }
    file.fingerprint = before;
    file
}

/// Parse candidate bytes with the same framing and header rules as a loaded file.
pub(crate) fn parse_candidate(path: PathBuf, raw: Vec<u8>) -> ItemFile {
    let mut file = ItemFile {
        path: path.clone(),
        raw,
        header: None,
        body: None,
        diagnostics: Vec::new(),
        fingerprint: None,
    };
    match parse_file(&file.raw, &path) {
        Ok((header, body)) => {
            let expected_name = format!("{}.md", header.id);
            if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
                file.diagnostics.push(Diagnostic {
                    path: path.clone(),
                    line: None,
                    message: format!("filename must be {expected_name}"),
                });
            }
            file.header = Some(header);
            file.body = Some(body);
        }
        Err((line, message, body)) => {
            file.body = body;
            file.diagnostics.push(Diagnostic {
                path,
                line,
                message,
            });
        }
    }
    file
}

fn read_regular_item(directory: &OwnedFd, name: &OsStr) -> Result<Vec<u8>, String> {
    let metadata = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("cannot inspect item: {error}"))?;
    let file_type = FileType::from_raw_mode(metadata.st_mode);
    if file_type == FileType::Symlink {
        return Err("item path must not be a symlink".into());
    }
    if file_type != FileType::RegularFile {
        return Err("item path must be a regular file".into());
    }
    // Hold the same directory through enumeration and reads. O_NOFOLLOW
    // rejects an entry replaced with a symlink after statat; O_NONBLOCK
    // prevents a replacement FIFO from blocking the loader during open.
    let source = openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open regular item without following links: {error}"))?;
    let mut source = File::from(source);
    if !source
        .metadata()
        .map_err(|error| format!("cannot inspect opened item: {error}"))?
        .is_file()
    {
        return Err("opened item must be a regular file".into());
    }
    let mut raw = Vec::new();
    source
        .read_to_end(&mut raw)
        .map_err(|error| format!("cannot read item: {error}"))?;
    Ok(raw)
}

type ParseError = (Option<usize>, String, Option<Vec<u8>>);

fn parse_file(raw: &[u8], _path: &Path) -> Result<(ItemHeader, Vec<u8>), ParseError> {
    let source = std::str::from_utf8(raw)
        .map_err(|error| (None, format!("invalid UTF-8: {error}"), None))?;
    if source.starts_with('\u{feff}') {
        return Err((Some(1), "UTF-8 byte-order mark is forbidden".into(), None));
    }
    let first_end = source.find('\n').ok_or((
        Some(1),
        "missing opening frontmatter delimiter".into(),
        None,
    ))? + 1;
    if trim_line(&source[..first_end]) != "---" {
        return Err((Some(1), "first line must be exactly ---".into(), None));
    }
    let mut offset = first_end;
    let mut close = None;
    let mut line_number = 2;
    while offset < source.len() {
        let end = source[offset..]
            .find('\n')
            .map(|n| offset + n + 1)
            .unwrap_or(source.len());
        if trim_line(&source[offset..end]) == "---" {
            close = Some((offset, end));
            break;
        }
        offset = end;
        line_number += 1;
    }
    let (header_end, body_start) = close.ok_or((
        Some(line_number),
        "missing closing frontmatter delimiter".into(),
        None,
    ))?;
    let body = raw[body_start..].to_vec();
    let header_source = &source[first_end..header_end];
    let node = parse_yaml(header_source)
        .map_err(|(line, message)| (line.map(|line| line + 1), message, Some(body.clone())))?;
    let header = decode_header(node).map_err(|message| (None, message, Some(body.clone())))?;
    Ok((header, body))
}

fn trim_line(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

#[derive(Debug)]
enum Node {
    String(String),
    Integer(i64),
    OtherScalar,
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
    expect_event(&events.0, &mut cursor, Event::StreamStart)?;
    expect_event(&events.0, &mut cursor, Event::DocumentStart)?;
    let node = parse_node(&events.0, &mut cursor)?;
    expect_event(&events.0, &mut cursor, Event::DocumentEnd)?;
    expect_event(&events.0, &mut cursor, Event::StreamEnd)?;
    if cursor != events.0.len() {
        return Err((None, "additional YAML documents are forbidden".into()));
    }
    Ok(node)
}

fn expect_event(
    events: &[(Event, Marker)],
    cursor: &mut usize,
    expected: Event,
) -> Result<(), (Option<usize>, String)> {
    match events.get(*cursor) {
        Some((event, _)) if *event == expected => {
            *cursor += 1;
            Ok(())
        }
        Some((_, marker)) => Err((
            Some(marker.line()),
            format!("expected {expected:?}; additional YAML documents or malformed header"),
        )),
        None => Err((None, format!("expected {expected:?}"))),
    }
}

fn parse_node(
    events: &[(Event, Marker)],
    cursor: &mut usize,
) -> Result<Node, (Option<usize>, String)> {
    let (event, marker) = events
        .get(*cursor)
        .ok_or((None, "incomplete YAML header".into()))?;
    let line = Some(marker.line());
    *cursor += 1;
    match event {
        Event::Alias(_) => Err((line, "YAML aliases are forbidden".into())),
        Event::Scalar(value, style, anchor, tag) => {
            if *anchor != 0 {
                return Err((line, "YAML anchors are forbidden".into()));
            }
            match standard_tag(tag.as_ref()).map_err(|message| (line, message))? {
                Some("str") => return Ok(Node::String(value.clone())),
                Some("int") => {
                    return tagged_integer(value)
                        .map(Node::Integer)
                        .map_err(|message| (line, message));
                }
                Some("float" | "bool" | "null") => return Ok(Node::OtherScalar),
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
                return Ok(Node::OtherScalar);
            }
            if core_decimal_integer(value) {
                if json_integer(value)
                    && let Ok(integer) = value.parse::<i64>()
                {
                    return Ok(Node::Integer(integer));
                }
                return Ok(Node::OtherScalar);
            }
            if core_float(value)
                || value.strip_prefix("0x").is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                || value.strip_prefix("0o").is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
                })
            {
                return Ok(Node::OtherScalar);
            }
            Ok(Node::String(value.clone()))
        }
        Event::SequenceStart(anchor, tag) => {
            if *anchor != 0 {
                return Err((line, "YAML anchors are forbidden".into()));
            }
            if !matches!(
                standard_tag(tag.as_ref()).map_err(|message| (line, message))?,
                None | Some("seq")
            ) {
                return Err((line, "sequence requires a sequence YAML tag".into()));
            }
            let mut values = Vec::new();
            while !matches!(events.get(*cursor), Some((Event::SequenceEnd, _))) {
                values.push(parse_node(events, cursor)?);
            }
            *cursor += 1;
            Ok(Node::Sequence(values))
        }
        Event::MappingStart(anchor, tag) => {
            if *anchor != 0 {
                return Err((line, "YAML anchors are forbidden".into()));
            }
            if !matches!(
                standard_tag(tag.as_ref()).map_err(|message| (line, message))?,
                None | Some("map")
            ) {
                return Err((line, "mapping requires a mapping YAML tag".into()));
            }
            let mut values = BTreeMap::new();
            while !matches!(events.get(*cursor), Some((Event::MappingEnd, _))) {
                let key_line = events.get(*cursor).map(|(_, marker)| marker.line());
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
    let Some(tag) = tag else {
        return Ok(None);
    };
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
            || (!unsigned.starts_with('0') && unsigned.bytes().all(|byte| byte.is_ascii_digit())))
}

fn core_decimal_integer(value: &str) -> bool {
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value);
    !unsigned.is_empty() && unsigned.bytes().all(|byte| byte.is_ascii_digit())
}

fn tagged_integer(value: &str) -> Result<i64, String> {
    let parsed = if let Some(digits) = value.strip_prefix("0x") {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("tagged integer has invalid YAML 1.2 syntax".into());
        }
        i64::from_str_radix(digits, 16)
    } else if let Some(digits) = value.strip_prefix("0o") {
        if digits.is_empty() || !digits.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
            return Err("tagged integer has invalid YAML 1.2 syntax".into());
        }
        i64::from_str_radix(digits, 8)
    } else if core_decimal_integer(value) {
        value.parse::<i64>()
    } else {
        return Err("tagged integer has invalid YAML 1.2 syntax".into());
    };
    parsed.map_err(|_| "tagged integer exceeds the supported range".into())
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
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
    }
    let Some((whole, fractional)) = mantissa.split_once('.') else {
        return exponent.is_some()
            && !mantissa.is_empty()
            && mantissa.bytes().all(|byte| byte.is_ascii_digit());
    };
    ((!whole.is_empty() && whole.bytes().all(|byte| byte.is_ascii_digit()))
        || (whole.is_empty() && !fractional.is_empty()))
        && fractional.bytes().all(|byte| byte.is_ascii_digit())
}

fn decode_header(node: Node) -> Result<ItemHeader, String> {
    let Node::Mapping(mut fields) = node else {
        return Err("header must be one YAML mapping".into());
    };
    let version = required_integer(&mut fields, "format_version")?;
    if version != 1 {
        return Err(format!("unsupported format_version {version}; expected 1"));
    }
    let id = required_string(&mut fields, "id")?;
    validate_id(&id, "id")?;
    let title = required_line(&mut fields, "title")?;
    let completion = match optional_string(&mut fields, "completion")?.as_deref() {
        None | Some("manual") => Completion::Manual,
        Some("children") => Completion::Children,
        Some(other) => {
            return Err(format!(
                "completion must be manual or children, got {other:?}"
            ));
        }
    };
    let state = match optional_string(&mut fields, "state")?.as_deref() {
        None => None,
        Some("open") => Some(ManualState::Open),
        Some("done") => Some(ManualState::Done),
        Some(other) => return Err(format!("state must be open or done, got {other:?}")),
    };
    let priority = match fields.remove("priority") {
        None => 2,
        Some(Node::Integer(value @ 0..=4)) => value as u8,
        _ => return Err("priority must be an integer from 0 to 4".into()),
    };
    let parent = optional_string(&mut fields, "parent")?;
    if let Some(id) = &parent {
        validate_id(id, "parent")?;
    }
    let depends_on = id_list(&mut fields, "depends_on")?;
    let related = id_list(&mut fields, "related")?;
    let discovered_from = id_list(&mut fields, "discovered_from")?;
    let labels = line_list(&mut fields, "labels")?;
    let model = optional_line(&mut fields, "model")?;
    let thinking = optional_line(&mut fields, "thinking")?;
    let close_reason = optional_line(&mut fields, "close_reason")?;
    if let Some(key) = fields.keys().next() {
        return Err(format!("unknown header key {key:?}"));
    }
    match completion {
        Completion::Manual => {
            if state.is_none() {
                return Err("manual item requires state".into());
            }
            if close_reason.is_some() && state != Some(ManualState::Done) {
                return Err("close_reason requires a done manual item".into());
            }
        }
        Completion::Children => {
            if state.is_some() || close_reason.is_some() {
                return Err("children aggregate must omit state and close_reason".into());
            }
        }
    }
    Ok(ItemHeader {
        id,
        title,
        completion,
        state,
        priority,
        parent,
        depends_on,
        related,
        discovered_from,
        labels,
        model,
        thinking,
        close_reason,
    })
}

fn required_integer(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<i64, String> {
    match fields.remove(key) {
        Some(Node::Integer(value)) => Ok(value),
        None => Err(format!("missing required key {key}")),
        _ => Err(format!("{key} must be an integer")),
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
fn line(value: String, key: &str) -> Result<String, String> {
    if value.trim().is_empty() || value.contains(['\r', '\n']) {
        Err(format!("{key} must be a nonblank single-line string"))
    } else {
        Ok(value)
    }
}
fn required_line(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<String, String> {
    line(required_string(fields, key)?, key)
}
fn optional_line(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<Option<String>, String> {
    optional_string(fields, key)?
        .map(|value| line(value, key))
        .transpose()
}
fn validate_id(value: &str, key: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() != 32
        || !bytes.iter().all(u8::is_ascii_hexdigit)
        || bytes.iter().any(u8::is_ascii_uppercase)
        || bytes[12] != b'4'
        || !matches!(bytes[16], b'8' | b'9' | b'a' | b'b')
    {
        Err(format!(
            "{key} must be a 32-character lowercase UUIDv4 identity"
        ))
    } else {
        Ok(())
    }
}
fn id_list(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<Vec<String>, String> {
    let values = match fields.remove(key) {
        None => return Ok(Vec::new()),
        Some(Node::Sequence(values)) => values,
        _ => return Err(format!("{key} must be a list of item IDs")),
    };
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let Node::String(value) = value else {
            return Err(format!("{key} must contain only string IDs"));
        };
        validate_id(&value, key)?;
        if !seen.insert(value.clone()) {
            return Err(format!("{key} contains duplicate ID {value}"));
        }
        result.push(value);
    }
    Ok(result)
}
fn line_list(fields: &mut BTreeMap<String, Node>, key: &str) -> Result<Vec<String>, String> {
    let values = match fields.remove(key) {
        None => return Ok(Vec::new()),
        Some(Node::Sequence(values)) => values,
        _ => return Err(format!("{key} must be a list of strings")),
    };
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let Node::String(value) = value else {
            return Err(format!("{key} must contain only strings"));
        };
        let value = line(value, key)?;
        if !seen.insert(value.clone()) {
            return Err(format!("{key} contains duplicate value {value:?}"));
        }
        result.push(value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn reads_from_held_directory_after_path_is_replaced() {
        let root = std::env::temp_dir().join(format!(
            "work-held-items-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let items_path = root.join(".work/items");
        let outside_path = root.join("outside");
        fs::create_dir_all(&items_path).unwrap();
        fs::create_dir(&outside_path).unwrap();
        let id = "d66b0ba51d2c4a7aa15de40cb3c9d507";
        let document = |body: &str| {
            format!(
                "---\nformat_version: 1\nid: \"{id}\"\ntitle: Example\nstate: open\n---\n{body}"
            )
        };
        fs::write(items_path.join(format!("{id}.md")), document("original")).unwrap();
        fs::write(outside_path.join(format!("{id}.md")), document("outside")).unwrap();

        let directory = openat(
            CWD,
            &items_path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap();
        fs::rename(&items_path, root.join(".work/held")).unwrap();
        symlink(&outside_path, &items_path).unwrap();

        let store = ItemStore::load_from_open_items(&directory, &items_path).unwrap();
        assert!(store.is_valid());
        assert_eq!(
            store.resolve(id).unwrap().body.as_deref(),
            Some(&b"original"[..])
        );
        fs::remove_dir_all(root).unwrap();
    }
}

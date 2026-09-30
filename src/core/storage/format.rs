//! Strict, small foundation formats. JSON output is a valid YAML subset.
use super::files::{Identity, Source};
use super::{StorageError, StorageErrorCode, StoreMetadata};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

pub(super) fn valid_id(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 32
        && b.iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        && b[12] == b'4'
        && matches!(b[16], b'8' | b'9' | b'a' | b'b')
}
struct Events(Vec<(Event, Marker)>);
impl MarkedEventReceiver for Events {
    fn on_event(&mut self, event: Event, marker: Marker) {
        self.0.push((event, marker));
    }
}
fn bad(path: &Path, message: impl Into<String>) -> StorageError {
    StorageError::new(
        StorageErrorCode::InvalidFormat,
        message,
        Some(path.to_owned()),
    )
}
pub(super) fn yaml(raw: &[u8], path: &Path) -> Result<Value, StorageError> {
    let text = std::str::from_utf8(raw).map_err(|_| bad(path, "foundation YAML must be UTF-8"))?;
    if text.lines().any(|line| line.starts_with('%')) {
        return Err(bad(path, "YAML directives are forbidden"));
    }
    let mut events = Events(Vec::new());
    Parser::new_from_str(text)
        .load(&mut events, true)
        .map_err(|e| {
            let mut error = bad(path, format!("invalid YAML: {e}"));
            error.diagnostics.push(super::StorageDiagnostic {
                code: StorageErrorCode::InvalidFormat,
                message: error.message.clone(),
                path: Some(path.to_owned()),
                line: Some(e.marker().line()),
            });
            error
        })?;
    let mut cursor = 0;
    expect(&events.0, &mut cursor, Event::StreamStart, path)?;
    expect(&events.0, &mut cursor, Event::DocumentStart, path)?;
    let result = node(&events.0, &mut cursor, path, 0)?;
    expect(&events.0, &mut cursor, Event::DocumentEnd, path)?;
    expect(&events.0, &mut cursor, Event::StreamEnd, path)?;
    if cursor != events.0.len() || !result.is_object() {
        return Err(bad(path, "one mapping document is required"));
    }
    Ok(result)
}
fn expect(
    events: &[(Event, Marker)],
    cursor: &mut usize,
    expected: Event,
    path: &Path,
) -> Result<(), StorageError> {
    if events
        .get(*cursor)
        .is_some_and(|(event, _)| *event == expected)
    {
        *cursor += 1;
        Ok(())
    } else {
        Err(bad(
            path,
            format!("expected {expected:?}; additional documents are forbidden"),
        ))
    }
}
fn node(
    events: &[(Event, Marker)],
    cursor: &mut usize,
    path: &Path,
    depth: usize,
) -> Result<Value, StorageError> {
    if depth > 16 {
        return Err(bad(
            path,
            "foundation YAML is nested beyond its defined formats",
        ));
    }
    let (event, _) = events
        .get(*cursor)
        .ok_or_else(|| bad(path, "incomplete YAML"))?;
    *cursor += 1;
    match event {
        Event::Scalar(value, style, anchor, tag) => {
            if *anchor != 0 || tag.is_some() {
                return Err(bad(path, "YAML anchors/tags are forbidden"));
            }
            if *style != TScalarStyle::Plain {
                return Ok(Value::String(value.clone()));
            }
            match value.as_str() {
                "null" | "Null" | "NULL" | "~" | "" => Ok(Value::Null),
                "true" | "True" | "TRUE" => Ok(Value::Bool(true)),
                "false" | "False" | "FALSE" => Ok(Value::Bool(false)),
                _ => {
                    if let Ok(number) = value.parse::<u64>() {
                        return Ok(json!(number));
                    }
                    if let Ok(number) = value.parse::<i64>() {
                        return Ok(json!(number));
                    }
                    Ok(Value::String(value.clone()))
                }
            }
        }
        Event::MappingStart(anchor, tag) => {
            if *anchor != 0 || tag.is_some() {
                return Err(bad(path, "YAML anchors/tags are forbidden"));
            }
            let mut map = Map::new();
            while !matches!(events.get(*cursor), Some((Event::MappingEnd, _))) {
                let key = node(events, cursor, path, depth + 1)?
                    .as_str()
                    .ok_or_else(|| bad(path, "mapping keys must be strings"))?
                    .to_owned();
                if key == "<<" {
                    return Err(bad(path, "YAML merge keys are forbidden"));
                }
                let value = node(events, cursor, path, depth + 1)?;
                if map.insert(key.clone(), value).is_some() {
                    return Err(bad(path, format!("duplicate field {key}")));
                }
            }
            *cursor += 1;
            Ok(Value::Object(map))
        }
        Event::SequenceStart(anchor, tag) => {
            if *anchor != 0 || tag.is_some() {
                return Err(bad(path, "YAML anchors/tags are forbidden"));
            }
            let mut sequence = Vec::new();
            while !matches!(events.get(*cursor), Some((Event::SequenceEnd, _))) {
                sequence.push(node(events, cursor, path, depth + 1)?);
            }
            *cursor += 1;
            Ok(Value::Array(sequence))
        }
        _ => Err(bad(path, "aliases or unexpected YAML events are forbidden")),
    }
}
fn fields<'a>(
    value: &'a Value,
    keys: &[&str],
    path: &Path,
) -> Result<&'a Map<String, Value>, StorageError> {
    let map = value
        .as_object()
        .ok_or_else(|| bad(path, "expected mapping"))?;
    if map.len() != keys.len() || keys.iter().any(|key| !map.contains_key(*key)) {
        return Err(bad(path, "unknown or missing fields"));
    }
    Ok(map)
}
fn version(map: &Map<String, Value>, path: &Path) -> Result<(), StorageError> {
    let value = map
        .get("format_version")
        .ok_or_else(|| bad(path, "missing format_version"))?;
    if value.as_u64() == Some(1) {
        return Ok(());
    }
    if value.is_i64() || value.is_u64() {
        return Err(StorageError::new(
            StorageErrorCode::UnsupportedFormat,
            "unsupported foundation format version",
            Some(path.to_owned()),
        ));
    }
    Err(bad(path, "format_version must be integer 1"))
}
fn string(map: &Map<String, Value>, key: &str, path: &Path) -> Result<String, StorageError> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad(path, format!("{key} must be a string")))
}
fn id(map: &Map<String, Value>, key: &str, path: &Path) -> Result<String, StorageError> {
    let value = string(map, key, path)?;
    if !valid_id(&value) {
        return Err(bad(path, format!("{key} must be a canonical UUIDv4")));
    }
    Ok(value)
}
pub(super) fn metadata(raw: &[u8], path: &Path) -> Result<StoreMetadata, StorageError> {
    let value = yaml(raw, path)?;
    version(value.as_object().expect("YAML mapping"), path)?;
    let map = fields(
        &value,
        &["format_version", "store_id", "recovery_generation"],
        path,
    )?;
    Ok(StoreMetadata {
        format_version: 1,
        store_id: id(map, "store_id", path)?,
        recovery_generation: id(map, "recovery_generation", path)?,
    })
}
pub(super) fn identity(raw: &[u8], path: &Path) -> Result<String, StorageError> {
    let value = yaml(raw, path)?;
    version(value.as_object().expect("YAML mapping"), path)?;
    let map = fields(&value, &["format_version", "store_id"], path)?;
    id(map, "store_id", path)
}
#[derive(Debug, Clone)]
pub(super) struct Operation {
    pub id: String,
    pub kind: String,
    pub store_id: String,
    pub previous_generation: Option<String>,
    pub next_generation: String,
    pub phase: String,
    pub prior_folders: Vec<String>,
    pub prior_operations: Vec<String>,
    pub executors_stopped: bool,
}
fn list(map: &Map<String, Value>, key: &str, path: &Path) -> Result<Vec<String>, StorageError> {
    let values = map
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| bad(path, format!("{key} must be a sequence")))?;
    let mut result = Vec::new();
    for value in values {
        result.push(
            value
                .as_str()
                .ok_or_else(|| bad(path, "list values must be strings"))?
                .to_owned(),
        );
    }
    if result.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(bad(path, "lists must be sorted and unique"));
    }
    Ok(result)
}
pub(super) fn operation(raw: &[u8], path: &Path) -> Result<Operation, StorageError> {
    let value = yaml(raw, path)?;
    let map = fields(
        &value,
        &[
            "format_version",
            "id",
            "kind",
            "store_id",
            "previous_generation",
            "next_generation",
            "phase",
            "prior_folders",
            "prior_operations",
            "executors_stopped",
        ],
        path,
    )?;
    version(map, path)?;
    let kind = string(map, "kind", path)?;
    let phase = string(map, "phase", path)?;
    if !matches!(kind.as_str(), "initialize" | "recreate")
        || !matches!(
            phase.as_str(),
            "prepared" | "archived" | "committed" | "complete"
        )
    {
        return Err(bad(path, "unsupported operation kind or phase"));
    }
    let previous_generation = if map["previous_generation"].is_null() {
        None
    } else {
        Some(id(map, "previous_generation", path)?)
    };
    let prior_folders = list(map, "prior_folders", path)?;
    let prior_operations = list(map, "prior_operations", path)?;
    if prior_folders
        .iter()
        .any(|name| !super::LIVE.contains(&name.as_str()))
        || prior_operations.iter().any(|id| !valid_id(id))
    {
        return Err(bad(path, "invalid prior directory identity"));
    }
    let executors_stopped = map["executors_stopped"]
        .as_bool()
        .ok_or_else(|| bad(path, "executors_stopped must be boolean"))?;
    if kind == "initialize"
        && (previous_generation.is_some()
            || !prior_folders.is_empty()
            || !prior_operations.is_empty()
            || executors_stopped
            || phase == "archived")
    {
        return Err(bad(path, "invalid initialization intent"));
    }
    if kind == "recreate" && !executors_stopped {
        return Err(bad(path, "recreation intent requires stopped executors"));
    }
    Ok(Operation {
        id: id(map, "id", path)?,
        kind,
        store_id: id(map, "store_id", path)?,
        previous_generation,
        next_generation: id(map, "next_generation", path)?,
        phase,
        prior_folders,
        prior_operations,
        executors_stopped,
    })
}
impl Operation {
    pub fn metadata(&self) -> StoreMetadata {
        StoreMetadata {
            format_version: 1,
            store_id: self.store_id.clone(),
            recovery_generation: self.next_generation.clone(),
        }
    }
    pub fn bytes(&self) -> Vec<u8> {
        bytes(
            &json!({"format_version":1,"id":self.id,"kind":self.kind,"store_id":self.store_id,
        "previous_generation":self.previous_generation,"next_generation":self.next_generation,"phase":self.phase,
        "prior_folders":self.prior_folders,"prior_operations":self.prior_operations,"executors_stopped":self.executors_stopped}),
        )
    }
    pub fn check_stages(
        &self,
        store: &[u8],
        witness: &[u8],
        path: &Path,
    ) -> Result<(), StorageError> {
        if metadata(store, &path.join("store.yaml"))? != self.metadata()
            || identity(witness, &path.join("identity.yaml"))? != self.store_id
        {
            return Err(StorageError::new(
                StorageErrorCode::StorageCorrupt,
                "staged identity/metadata do not match operation intent",
                Some(path.to_owned()),
            ));
        }
        Ok(())
    }
}
pub(super) fn metadata_bytes(meta: &StoreMetadata) -> Vec<u8> {
    bytes(
        &json!({"format_version":1,"store_id":meta.store_id,"recovery_generation":meta.recovery_generation}),
    )
}
pub(super) fn identity_bytes(id: &str) -> Vec<u8> {
    bytes(&json!({"format_version":1,"store_id":id}))
}
pub(super) fn bytes(value: &Value) -> Vec<u8> {
    let mut raw = serde_json::to_vec_pretty(value).expect("JSON foundation serialization");
    raw.push(b'\n');
    raw
}

#[derive(Debug, Clone)]
pub(super) struct Context {
    pub prior_missing_or_damaged: bool,
    pub store: Option<Source>,
    pub identity: Option<Source>,
    pub folders: BTreeMap<String, Identity>,
    pub operations: BTreeMap<String, Identity>,
}
impl Context {
    pub fn empty() -> Self {
        Self {
            prior_missing_or_damaged: false,
            store: None,
            identity: None,
            folders: BTreeMap::new(),
            operations: BTreeMap::new(),
        }
    }
    pub fn bytes(&self) -> Vec<u8> {
        bytes(
            &json!({"format_version":1,"store":source_value(&self.store),"identity":source_value(&self.identity),
        "folders":directory_values(&self.folders),"operations":directory_values(&self.operations),
        "prior_missing_or_damaged":self.prior_missing_or_damaged}),
        )
    }
}
fn hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02x}")).collect()
}
fn source_value(source: &Option<Source>) -> Value {
    match source {
        None => Value::Null,
        Some(s) => {
            json!({"raw_hex":hex(&s.raw),"dev":s.identity.dev,"ino":s.identity.ino,"mode":s.mode,"size":s.size,
        "mtime":[s.mtime.0,s.mtime.1],"ctime":[s.ctime.0,s.ctime.1]})
        }
    }
}
fn directory_values(values: &BTreeMap<String, Identity>) -> Value {
    Value::Object(
        values
            .iter()
            .map(|(key, id)| (key.clone(), json!({"dev":id.dev,"ino":id.ino})))
            .collect(),
    )
}
fn unsigned(map: &Map<String, Value>, key: &str, path: &Path) -> Result<u64, StorageError> {
    map[key]
        .as_u64()
        .ok_or_else(|| bad(path, format!("{key} must be unsigned integer")))
}
fn directory(value: &Value, path: &Path) -> Result<Identity, StorageError> {
    let m = fields(value, &["dev", "ino"], path)?;
    Ok(Identity {
        dev: unsigned(m, "dev", path)?,
        ino: unsigned(m, "ino", path)?,
    })
}
fn time(value: &Value, path: &Path) -> Result<(i64, i64), StorageError> {
    let pair = value
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or_else(|| bad(path, "timestamp requires seconds/nanoseconds pair"))?;
    let seconds = pair[0]
        .as_i64()
        .ok_or_else(|| bad(path, "seconds outside signed filesystem representation"))?;
    let nano = pair[1]
        .as_u64()
        .filter(|n| *n < 1_000_000_000)
        .ok_or_else(|| bad(path, "nanoseconds must be unsigned and below one billion"))?;
    Ok((seconds, nano as i64))
}
fn source(value: &Value, path: &Path) -> Result<Option<Source>, StorageError> {
    if value.is_null() {
        return Ok(None);
    }
    let m = fields(
        value,
        &["raw_hex", "dev", "ino", "mode", "size", "mtime", "ctime"],
        path,
    )?;
    let encoded = string(m, "raw_hex", path)?;
    if !encoded.len().is_multiple_of(2)
        || encoded
            .bytes()
            .any(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b))
    {
        return Err(bad(
            path,
            "raw_hex must be lowercase even-length hexadecimal",
        ));
    }
    let raw = encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("hex ASCII"), 16)
                .expect("validated hex")
        })
        .collect::<Vec<_>>();
    let mode = u32::try_from(unsigned(m, "mode", path)?)
        .map_err(|_| bad(path, "mode outside filesystem representation"))?;
    let size = unsigned(m, "size", path)?;
    if size != raw.len() as u64 || size > i64::MAX as u64 {
        return Err(bad(
            path,
            "source size differs from raw bytes or exceeds filesystem representation",
        ));
    }
    let result = Source {
        raw,
        identity: Identity {
            dev: unsigned(m, "dev", path)?,
            ino: unsigned(m, "ino", path)?,
        },
        mode,
        size,
        mtime: time(&m["mtime"], path)?,
        ctime: time(&m["ctime"], path)?,
    };
    if result.mode & 0o170000 != 0o100000 {
        return Err(bad(path, "retained source mode is not a regular file"));
    }
    Ok(Some(result))
}
pub(super) fn context(raw: &[u8], op: &Operation, path: &Path) -> Result<Context, StorageError> {
    let value = yaml(raw, path)?;
    let m = fields(
        &value,
        &[
            "format_version",
            "prior_missing_or_damaged",
            "store",
            "identity",
            "folders",
            "operations",
        ],
        path,
    )?;
    version(m, path)?;
    let decode =
        |value: &Value, keys: &[String]| -> Result<BTreeMap<String, Identity>, StorageError> {
            let map = value
                .as_object()
                .ok_or_else(|| bad(path, "directory context must be mapping"))?;
            if map.len() != keys.len() || keys.iter().any(|k| !map.contains_key(k)) {
                return Err(bad(
                    path,
                    "context directory keys differ from operation intent",
                ));
            }
            map.iter()
                .map(|(k, v)| Ok((k.clone(), directory(v, path)?)))
                .collect()
        };
    let context = Context {
        prior_missing_or_damaged: m["prior_missing_or_damaged"]
            .as_bool()
            .ok_or_else(|| bad(path, "prior_missing_or_damaged must be boolean"))?,
        store: source(&m["store"], path)?,
        identity: source(&m["identity"], path)?,
        folders: decode(&m["folders"], &op.prior_folders)?,
        operations: decode(&m["operations"], &op.prior_operations)?,
    };
    if op.kind == "initialize"
        && (context.prior_missing_or_damaged
            || context.store.is_some()
            || context.identity.is_some()
            || !context.folders.is_empty()
            || !context.operations.is_empty())
    {
        return Err(bad(path, "initialization source context must be empty"));
    }
    Ok(context)
}

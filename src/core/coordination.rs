//! Shared held-lock entity IO for the execution slices.
//! @mara implements DES-EXECUTION-IO
use super::operations::OperationError;
use super::project::Project;
use super::storage::files::{Directory, Locked, Source};
use super::storage::{Publication, Storage, StorageError, StoreMetadata};
use serde_json::{Value, json};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub type ExecutionResult<T> = Result<T, ExecutionError>;
#[derive(Debug, Clone)]
pub struct ExecutionError {
    pub code: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
    pub details: Value,
}
impl ExecutionError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            details: json!({}),
        }
    }
    pub fn at(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}
impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ExecutionError {}
impl From<StorageError> for ExecutionError {
    fn from(e: StorageError) -> Self {
        Self {
            code: e.code.code(),
            message: e.message,
            path: e.path,
            details: json!({
            "publication":match e.publication {Publication::NotPublished=>"not_published",Publication::Possible=>"possible",Publication::Published=>"published"},
            "operation_id":e.operation_id,"recovery_paths":e.recovery_paths.iter().map(|p|encode_path(p)).collect::<Vec<_>>(),"errno":e.errno,
            "diagnostics":e.diagnostics.iter().map(|d|json!({"code":d.code.code(),"message":d.message,"path":d.path.as_deref().map(encode_path),"line":d.line})).collect::<Vec<_>>()}),
        }
    }
}
impl From<OperationError> for ExecutionError {
    fn from(e: OperationError) -> Self {
        let mut result = Self::new(e.code(), e.to_string());
        if let Some((id, path)) = e.published_item() {
            result.details["published_item"] = json!({"id":id,"path":encode_path(path)});
        }
        if let Some(path) = e.previous_source_path() {
            result.details["previous_source_path"] = json!(encode_path(path));
        }
        if let OperationError::InvalidSource(d) | OperationError::InvalidCandidate(d) = &e {
            result.details["diagnostics"] = json!(
                d.iter()
                    .map(|d| json!({"path":encode_path(&d.path),"line":d.line,"message":d.message}))
                    .collect::<Vec<_>>()
            );
        }
        result
    }
}
impl From<std::io::Error> for ExecutionError {
    fn from(e: std::io::Error) -> Self {
        Self::new(
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                "permission_denied"
            } else {
                "io"
            },
            e.to_string(),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub namespace: String,
    pub id: String,
}
impl SessionIdentity {
    pub fn validate(&self) -> ExecutionResult<()> {
        if self.namespace.is_empty() || self.id.is_empty() {
            return Err(ExecutionError::new(
                "invalid_argument",
                "session namespace and id must be nonempty",
            ));
        }
        Ok(())
    }
    pub fn to_json(&self) -> Value {
        json!({"namespace":self.namespace,"id":self.id})
    }
    pub fn from_json(v: &Value) -> ExecutionResult<Self> {
        exact_keys(v, &["namespace", "id"], &[])?;
        let result = Self {
            namespace: string(v, "namespace")?,
            id: string(v, "id")?,
        };
        result.validate()?;
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct EntitySource {
    pub raw: Vec<u8>,
    source: Source,
}
pub struct CoordinationGuard {
    locked: Locked,
    project: Project,
    exclusive: bool,
    pub metadata: StoreMetadata,
}
impl CoordinationGuard {
    pub fn acquire(project: &Project, exclusive: bool) -> ExecutionResult<Self> {
        let common = Directory::open(&project.git_common_dir)?;
        let locked = Locked::open(common, exclusive, false)?;
        let storage = Storage::new(project.clone());
        let mut inspection = storage.empty_inspection();
        storage.inspect_locked(&locked, &mut inspection);
        if !inspection.coordination_available {
            let mut e = ExecutionError::new(
                "recovery_required",
                "coordination unavailable; inspect storage and perform explicit recovery",
            )
            .at(&inspection.path);
            e.details["diagnostics"]=json!(inspection.diagnostics.iter().map(|d|json!({"code":d.code.code(),"message":d.message,"path":d.path.as_deref().map(encode_path)})).collect::<Vec<_>>());
            return Err(e);
        }
        Ok(Self {
            metadata: inspection.metadata.expect("available storage has metadata"),
            locked,
            project: project.clone(),
            exclusive,
        })
    }
    pub fn project(&self) -> &Project {
        &self.project
    }
    pub fn root_path(&self) -> &Path {
        &self.locked.root.path
    }
    pub fn verify(&self) -> ExecutionResult<()> {
        self.locked.verify()?;
        Ok(())
    }
    fn writable(&self) -> ExecutionResult<()> {
        if !self.exclusive {
            Err(ExecutionError::new(
                "conflict",
                "exclusive coordination guard required",
            ))
        } else {
            self.verify()
        }
    }
    fn directory(&self, path: &Path, ensure: bool) -> ExecutionResult<Directory> {
        self.verify()?;
        let mut dir = self.locked.common.child("work")?;
        for part in path.components() {
            let Component::Normal(name) = part else {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "entity paths must be relative without dot or parent components",
                ));
            };
            let name = name.to_str().ok_or_else(|| {
                ExecutionError::new("invalid_argument", "entity paths must be UTF-8")
            })?;
            dir = if ensure {
                dir.ensure(name)?
            } else {
                dir.child(name)?
            };
        }
        Ok(dir)
    }
    fn parent(&self, path: &Path) -> ExecutionResult<(Directory, String)> {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ExecutionError::new("invalid_argument", "entity file name required"))?;
        if path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ExecutionError::new(
                "invalid_argument",
                "invalid relative entity path",
            ));
        }
        Ok((
            self.directory(path.parent().unwrap_or(Path::new("")), false)?,
            name.into(),
        ))
    }
    pub fn read(&self, path: &Path) -> ExecutionResult<EntitySource> {
        let (dir, name) = self.parent(path)?;
        let source = dir.read(&name)?;
        dir.verify()?;
        self.verify()?;
        Ok(EntitySource {
            raw: source.raw.clone(),
            source,
        })
    }
    pub fn recheck(&self, path: &Path, expected: &EntitySource) -> ExecutionResult<()> {
        let actual = self.read(path)?;
        if actual.source != expected.source {
            return Err(ExecutionError::new("conflict", "entity source changed")
                .at(self.root_path().join(path)));
        }
        Ok(())
    }
    pub fn optional(&self, path: &Path) -> ExecutionResult<Option<EntitySource>> {
        let (dir, name) = self.parent(path)?;
        let source = dir.optional(&name)?;
        dir.verify()?;
        self.verify()?;
        Ok(source.map(|source| EntitySource {
            raw: source.raw.clone(),
            source,
        }))
    }
    pub fn names(&self, path: &Path) -> ExecutionResult<Vec<String>> {
        let dir = self.directory(path, false)?;
        let mut names = Vec::new();
        for name in dir.names()? {
            let name = name.into_string().map_err(|_| {
                ExecutionError::new("invalid_format", "entity name is not UTF-8").at(&dir.path)
            })?;
            if !name.starts_with(".storage-") {
                names.push(name);
            }
        }
        dir.verify()?;
        self.verify()?;
        names.sort();
        Ok(names)
    }
    pub fn ensure_dir(&self, path: &Path) -> ExecutionResult<()> {
        self.writable()?;
        self.directory(path, true)?.verify()?;
        self.verify()
    }
    pub fn create(&self, path: &Path, raw: &[u8]) -> ExecutionResult<()> {
        self.publish(path, None, raw)
    }
    pub fn replace(&self, path: &Path, expected: &EntitySource, raw: &[u8]) -> ExecutionResult<()> {
        self.publish(path, Some(&expected.source), raw)
    }
    fn publish(&self, path: &Path, expected: Option<&Source>, raw: &[u8]) -> ExecutionResult<()> {
        self.writable()?;
        let (dir, name) = self.parent(path)?;
        let stage = format!(".storage-{}", new_id()?);
        dir.publish(&name, raw, expected, &stage)?;
        self.verify()
    }
    pub fn delete(&self, path: &Path, expected: &EntitySource) -> ExecutionResult<()> {
        self.writable()?;
        let (dir, name) = self.parent(path)?;
        if dir.read(&name)? != expected.source {
            return Err(
                ExecutionError::new("conflict", "entity changed before deletion")
                    .at(dir.path.join(name)),
            );
        }
        dir.verify()?;
        self.verify()?;
        rustix::fs::unlinkat(&dir.file, name.as_str(), rustix::fs::AtFlags::empty())
            .map_err(std::io::Error::from)?;
        dir.sync()
            .map_err(ExecutionError::from)
            .and_then(|_| dir.verify().map_err(ExecutionError::from))
            .and_then(|_| self.verify())
            .map_err(|e| {
                let mut e = e;
                e.details["publication"] = json!("possible");
                e.details["deleted_path"] = json!(encode_path(&dir.path.join(&name)));
                e
            })?;
        Ok(())
    }
}
pub fn new_id() -> ExecutionResult<String> {
    Ok(super::operations::new_id()?)
}
pub fn valid_id(s: &str) -> bool {
    super::storage::format::valid_id(s)
}
pub fn parse_yaml(raw: &[u8]) -> ExecutionResult<Value> {
    Ok(super::storage::format::yaml(raw, Path::new("entity"))?)
}
pub fn yaml_bytes(v: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(v).expect("JSON value serializable");
    bytes.push(b'\n');
    bytes
}
pub fn exact_keys(v: &Value, required: &[&str], optional: &[&str]) -> ExecutionResult<()> {
    let m = v
        .as_object()
        .ok_or_else(|| ExecutionError::new("invalid_format", "expected mapping"))?;
    if required.iter().any(|k| !m.contains_key(*k))
        || m.keys()
            .any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
        || m.values().any(Value::is_null)
    {
        return Err(ExecutionError::new(
            "invalid_format",
            "unknown, missing, or null fields",
        ));
    }
    Ok(())
}
pub fn string(v: &Value, k: &str) -> ExecutionResult<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ExecutionError::new("invalid_format", format!("{k} must be a string")))
}
pub fn now_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}
pub fn encode_path(path: &Path) -> String {
    use std::fmt::Write;
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            write!(s, "%{b:02X}").unwrap();
        }
    }
    s
}
pub fn decode_path(s: &str) -> ExecutionResult<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = Vec::new();
    let mut i = 0;
    let raw = s.as_bytes();
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = raw
                .get(i + 1..i + 3)
                .ok_or_else(|| ExecutionError::new("invalid_format", "invalid encoded path"))?;
            let text = std::str::from_utf8(hex)
                .map_err(|_| ExecutionError::new("invalid_format", "invalid encoded path"))?;
            bytes.push(
                u8::from_str_radix(text, 16)
                    .map_err(|_| ExecutionError::new("invalid_format", "invalid encoded path"))?,
            );
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    if !path.is_absolute() {
        return Err(ExecutionError::new(
            "invalid_format",
            "workspace path must be absolute",
        ));
    }
    Ok(path)
}

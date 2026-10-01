//! Shared held-lock entity IO for the execution slices.
//! @mara implements DES-EXECUTION-IO
use super::operations::OperationError;
use super::project::Project;
use super::storage::files::{Directory, Locked, Source};
use super::storage::{
    Publication, Storage, StorageError, StorageErrorCode, StorageInspection, StoreMetadata,
};
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
        result.details["publication"] = json!(if e.published_item().is_some() {
            "published"
        } else {
            "not_published"
        });
        if let Some((id, path)) = e.published_item() {
            let item = json!({"id":id,"path":encode_path(path)});
            result.details["published_item"] = item.clone();
            let mut partial = json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[]});
            let bucket = if e.previous_source_path().is_some() {
                "updated"
            } else {
                "created"
            };
            partial[bucket] = json!([item]);
            result.details["partial"] = partial;
        }
        if let Some(path) = e.previous_source_path() {
            result.details["previous_source_path"] = json!(encode_path(path));
        }
        let mut cause = &e;
        while let OperationError::Published { cause: nested, .. } = cause {
            cause = nested;
        }
        if let OperationError::InvalidSource(d) | OperationError::InvalidCandidate(d) = cause {
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
        Self::acquire_with_storage(project, exclusive).map_err(|failure| {
            let (error, _) = *failure;
            error
        })
    }
    pub(crate) fn acquire_with_storage(
        project: &Project,
        exclusive: bool,
    ) -> Result<Self, Box<(ExecutionError, StorageInspection)>> {
        let storage = Storage::new(project.clone());
        let unavailable = |error: StorageError| {
            let inspection = storage.unavailable_inspection(error.clone());
            Box::new((error.into(), inspection))
        };
        let common = Directory::open(&project.git_common_dir).map_err(unavailable)?;
        let locked = Locked::open(common, exclusive, false).map_err(unavailable)?;
        let mut inspection = storage.empty_inspection();
        let refusal = storage.inspect_locked_evidence(&locked, &mut inspection);
        if let Some(error) = refusal
            && matches!(
                error.code,
                StorageErrorCode::PermissionDenied | StorageErrorCode::Io
            )
        {
            return Err(Box::new((error.into(), inspection)));
        }
        if !inspection.coordination_available {
            let mut e = ExecutionError::new(
                "recovery_required",
                "coordination unavailable; inspect storage and perform explicit recovery",
            )
            .at(&inspection.path);
            e.details["diagnostics"]=json!(inspection.diagnostics.iter().map(|d|json!({"code":d.code.code(),"message":d.message,"path":d.path.as_deref().map(encode_path)})).collect::<Vec<_>>());
            return Err(Box::new((e, inspection)));
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
        self.publish(path, None, raw).map(|_| ())
    }
    pub fn replace(&self, path: &Path, expected: &EntitySource, raw: &[u8]) -> ExecutionResult<()> {
        self.replace_with_recovery(path, expected, raw).map(|_| ())
    }
    pub(crate) fn replace_with_recovery(
        &self,
        path: &Path,
        expected: &EntitySource,
        raw: &[u8],
    ) -> ExecutionResult<Option<PathBuf>> {
        self.publish(path, Some(&expected.source), raw)
    }
    fn publish(
        &self,
        path: &Path,
        expected: Option<&Source>,
        raw: &[u8],
    ) -> ExecutionResult<Option<PathBuf>> {
        self.writable()?;
        let (dir, name) = self.parent(path)?;
        let stage = format!(".storage-{}", new_id()?);
        let retained = dir.publish(&name, raw, expected, &stage)?;
        self.verify().map_err(|mut error| {
            // Publication and directory sync completed before this final lock check.
            error.details["publication"] = json!("published");
            error.details["published_path"] = json!(encode_path(&dir.path.join(&name)));
            if let Some(path) = &retained {
                error.details["previous_source_path"] = json!(encode_path(path));
                let mut paths = error.details["recovery_paths"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                paths.push(json!(encode_path(path)));
                error.details["recovery_paths"] = json!(paths);
            }
            error
        })?;
        Ok(retained)
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

#[cfg(test)]
mod publication_tests {
    use super::*;
    use crate::core::{
        claims::{ClaimAuthorization, ClaimStore},
        context::ContextStore,
        execution::ExecutionOperations,
        operations::MetadataChange,
        project::discover,
        storage::files,
    };
    use std::{fs, process::Command};
    struct Fixture {
        root: PathBuf,
        ops: ExecutionOperations,
        item: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "work-coordination-publication-{}",
                new_id().unwrap()
            ));
            fs::create_dir_all(root.join(".work/items")).unwrap();
            assert!(
                Command::new("git")
                    .args(["init", "-q"])
                    .arg(&root)
                    .status()
                    .unwrap()
                    .success()
            );
            let ops = ExecutionOperations::new(discover(Some(&root)).unwrap());
            let item = ops
                .create("task".into(), b"body".to_vec(), MetadataChange::default())
                .unwrap()
                .file
                .header
                .unwrap()
                .id;
            Storage::new(ops.project.clone()).initialize().unwrap();
            Self { root, ops, item }
        }
        fn session() -> SessionIdentity {
            SessionIdentity {
                namespace: "test".into(),
                id: "owner".into(),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn substitute_lock(path: PathBuf) {
        fs::rename(&path, path.with_extension("retained")).unwrap();
        fs::write(path, b"").unwrap();
    }
    #[test]
    fn final_guard_failure_reports_synced_creation_and_replacement() {
        for replace in [false, true] {
            let f = Fixture::new();
            let g = CoordinationGuard::acquire(&f.ops.project, true).unwrap();
            let path = Path::new("claims/test.yaml");
            let source = if replace {
                g.create(path, b"before").unwrap();
                Some(g.read(path).unwrap())
            } else {
                None
            };
            let lock = g.root_path().join("coordination.lock");
            let hook = files::on_next("after_directory_sync", move || substitute_lock(lock));
            let error = match &source {
                Some(source) => g.replace(path, source, b"after"),
                None => g.create(path, b"after"),
            }
            .unwrap_err();
            drop(hook);
            assert_eq!(error.code, "conflict");
            assert_eq!(error.details["publication"], "published");
            assert_eq!(
                error.details["published_path"],
                encode_path(&g.root_path().join(path))
            );
            assert_eq!(error.path, Some(g.root_path().join("coordination.lock")));
            assert_eq!(fs::read(g.root_path().join(path)).unwrap(), b"after");
            if replace {
                let retained = error.details["recovery_paths"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()
                    .as_str()
                    .unwrap();
                assert_eq!(fs::read(decode_path(retained).unwrap()).unwrap(), b"before");
            }
        }
    }
    #[test]
    fn acquisition_final_guard_failure_reports_installed_claim() {
        let f = Fixture::new();
        {
            let g = CoordinationGuard::acquire(&f.ops.project, true).unwrap();
            let workspace = ContextStore::register(&g, &f.root).unwrap();
            ContextStore::bind(&g, &f.item, &workspace).unwrap();
        }
        let lock = f.ops.project.git_common_dir.join("work/coordination.lock");
        let hook = files::on_next("after_directory_sync", move || substitute_lock(lock));
        let error = f
            .ops
            .acquire(&f.item, "actor", &Fixture::session())
            .unwrap_err();
        drop(hook);
        assert_eq!(error.details["publication"], "published");
        let created = error.details["partial"]["created"].as_array().unwrap();
        assert_eq!(created.len(), 1);
        let path = decode_path(created[0]["path"].as_str().unwrap()).unwrap();
        assert!(path.is_file());
        assert_eq!(error.details["published_path"], created[0]["path"]);
    }
    #[test]
    fn acquisition_reload_failure_keeps_installed_claim_evidence() {
        let f = Fixture::new();
        {
            let g = CoordinationGuard::acquire(&f.ops.project, true).unwrap();
            let workspace = ContextStore::register(&g, &f.root).unwrap();
            ContextStore::bind(&g, &f.item, &workspace).unwrap();
        }
        let invalid = f
            .ops
            .project
            .git_common_dir
            .join("work/workspaces/unexpected");
        let hook = files::on_next("after_directory_sync", move || {
            fs::write(invalid, b"invalid").unwrap();
        });
        let error = f
            .ops
            .acquire(&f.item, "actor", &Fixture::session())
            .unwrap_err();
        drop(hook);
        assert_eq!(error.code, "invalid_format");
        assert_eq!(error.details["publication"], "published");
        let created = error.details["partial"]["created"].as_array().unwrap();
        assert_eq!(created.len(), 1);
        assert!(
            decode_path(created[0]["path"].as_str().unwrap())
                .unwrap()
                .is_file()
        );
    }
    #[test]
    fn close_checkout_failure_keeps_saved_item_and_active_claim_evidence() {
        let mut f = Fixture::new();
        let claim = f
            .ops
            .acquire(&f.item, "actor", &Fixture::session())
            .unwrap()
            .0;
        f.ops.authorization.push(ClaimAuthorization {
            claim_id: claim["id"].as_str().unwrap().into(),
            session: Fixture::session(),
        });
        let original = fs::read(f.root.join(format!(".work/items/{}.md", f.item))).unwrap();
        let hook = files::fail_next("checkout_after_publish");
        let error = f.ops.close(&f.item, None).unwrap_err();
        drop(hook);
        assert_eq!(error.code, "io");
        assert_eq!(error.details["publication"], "published");
        assert_eq!(
            error.details["partial"]["updated"],
            json!([error.details["published_item"]])
        );
        assert_eq!(error.details["published_item"]["id"], f.item);
        assert_eq!(
            fs::read(decode_path(error.details["previous_source_path"].as_str().unwrap()).unwrap())
                .unwrap(),
            original
        );
        assert_eq!(
            f.ops.inspect(&f.item).unwrap().file.header.unwrap().state,
            Some(crate::core::items::ManualState::Done)
        );
        let g = CoordinationGuard::acquire(&f.ops.project, false).unwrap();
        assert!(ClaimStore::current(&g, &f.item).unwrap().is_some());
    }
}

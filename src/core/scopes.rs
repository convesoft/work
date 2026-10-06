//! Git-local beta.2 hosting. Maps and inventories are disposable read snapshots.
//! @mara implements DES-SCOPE-RESOLUTION
//! @mara implements DES-SCOPE-LAYOUT
use super::coordination::{
    ExecutionError as Error, ExecutionResult as Result, decode_path, new_id, valid_id,
};
use super::graph::ItemGraph;
use super::items::{ItemFile, ItemStore};
use super::project::{Checkout, Project};
use super::scope_wire::{self as wire, hash};
use super::storage::files::{Directory, Identity, Locked, Source};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub fn path(p: &Path) -> String {
    use std::fmt::Write;
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/._-".contains(&b) {
            out.push(b as char);
        } else {
            write!(out, "%{b:02X}").unwrap();
        }
    }
    out
}
pub fn diagnostic(
    code: &str,
    message: impl Into<String>,
    source: Option<&Path>,
    scope: Option<&str>,
) -> Value {
    json!({"code":code,"message":message.into(),"path":source.map(path),"line":null,"item_id":null,"run_id":null,"scope_id":scope})
}
pub fn partial() -> Value {
    json!({"created":[],"updated":[],"deleted":[],"uncertain_paths":[],"saved_handoffs":[],"allocated_items":[],"tree_changes":[]})
}
pub(crate) fn error_partial(mut error: Error, mut known: Value, operation: &str) -> Error {
    if let Some(previous) = error.details.get("partial") {
        for key in partial().as_object().unwrap().keys() {
            if let Some(entries) = previous[key].as_array() {
                for entry in entries {
                    if !known[key].as_array().unwrap().contains(entry) {
                        known[key].as_array_mut().unwrap().push(entry.clone());
                    }
                }
            }
        }
    }
    if error.details["publication"] == "possible"
        && let Some(p) = error.path.as_deref()
    {
        let p = json!(path(p));
        if !known["uncertain_paths"].as_array().unwrap().contains(&p) {
            known["uncertain_paths"].as_array_mut().unwrap().push(p);
        }
    }
    error.details["operation_id"] = json!(operation);
    error.details["partial"] = known;
    error
}
fn io(e: std::io::Error) -> Error {
    Error::new("io", e.to_string())
}
fn discovery(e: super::project::DiscoveryError) -> Error {
    Error::new("unsupported_project", e.to_string())
}
fn visible(names: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    use std::os::unix::ffi::OsStrExt;
    names
        .into_iter()
        .filter(|n| {
            !n.as_bytes().starts_with(b".stage-") && !n.as_bytes().starts_with(b".operation-")
        })
        .collect()
}

#[derive(Clone, Default)]
pub(crate) struct Capture {
    pub files: BTreeMap<PathBuf, Source>,
    dirs: BTreeMap<PathBuf, (Identity, Vec<std::ffi::OsString>)>,
    inventory: Option<(Project, Vec<Checkout>)>,
}
impl Capture {
    fn directory(&mut self, d: &Directory) -> Result<()> {
        let snapshot = (d.identity.clone(), visible(d.names()?));
        if let Some(before) = self.dirs.get(&d.path) {
            if before != &snapshot {
                return Err(Error::new("conflict", "directory changed during capture").at(&d.path));
            }
        } else {
            self.dirs.insert(d.path.clone(), snapshot);
        }
        Ok(())
    }
    pub(crate) fn file(&mut self, p: &Path) -> Result<Option<Source>> {
        let Some(parent) = p.parent() else {
            return Err(Error::new("unsafe_path", "missing parent").at(p));
        };
        let d = Directory::open(parent)?;
        self.directory(&d)?;
        let s = d.optional(
            p.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| Error::new("unsafe_path", "non-UTF-8 record filename").at(p))?,
        )?;
        if let Some(s) = &s {
            self.files.insert(p.to_owned(), s.clone());
        }
        Ok(s)
    }
    pub fn recheck(&self) -> Result<()> {
        if let Some((project, expected)) = &self.inventory
            && project.checkouts().map_err(discovery)? != *expected
        {
            return Err(Error::new("conflict", "Git worktree inventory changed"));
        }
        for (p, (id, names)) in &self.dirs {
            let d = Directory::open(p)?;
            if &d.identity != id || &visible(d.names()?) != names {
                return Err(Error::new("conflict", "captured directory changed").at(p));
            }
        }
        for (p, s) in &self.files {
            let d = Directory::open(p.parent().unwrap())?;
            if &d.read(p.file_name().unwrap().to_str().unwrap())? != s {
                return Err(Error::new("conflict", "captured source changed").at(p));
            }
        }
        Ok(())
    }
    pub(crate) fn recheck_except_material(&self, root: &Path) -> Result<()> {
        // The durable repair writer holds/checks this catalog itself while
        // archiving exact sources. Keep the rest of the routing/graph fenced.
        let catalog = root.join(".work/items");
        let mut outside = self.clone();
        outside.files.retain(|p, _| !p.starts_with(&catalog));
        outside.dirs.remove(&catalog);
        outside.recheck()
    }
    pub(crate) fn compare_prelock(&self, fresh: &Capture) -> Result<()> {
        for (p, s) in &self.files {
            if fresh.files.get(p) != Some(s) {
                return Err(Error::new(
                    "conflict",
                    "source identity/bytes changed acquiring locks",
                )
                .at(p));
            }
        }
        for p in fresh.files.keys() {
            if !self.files.contains_key(p) && p.file_name().is_none_or(|n| n != "coordination.lock")
            {
                return Err(Error::new("conflict", "source set grew while acquiring locks").at(p));
            }
        }
        for (p, (id, names)) in &self.dirs {
            if let Some((new_id, new_names)) = fresh.dirs.get(p) {
                let filter = |a: &Vec<std::ffi::OsString>| {
                    a.iter()
                        .filter(|n| {
                            !(p.file_name().is_some_and(|s| s == ".work")
                                && n.as_os_str() == "operations.lock"
                                || self.inventory.as_ref().is_some_and(|(project, _)| {
                                    p == &project.git_common_dir.join("work-beta2")
                                }) && n.as_os_str() == "coordination.lock"
                                || self
                                    .inventory
                                    .as_ref()
                                    .is_some_and(|(project, _)| p == &project.git_common_dir)
                                    && n.as_os_str() == "work-beta2")
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                };
                if id != new_id || filter(names) != filter(new_names) {
                    return Err(
                        Error::new("conflict", "source directory changed acquiring locks").at(p),
                    );
                }
            } else {
                return Err(Error::new("conflict", "source directory disappeared").at(p));
            }
        }
        Ok(())
    }
    pub(crate) fn advance(&mut self, p: &Path, after: Option<&[u8]>) -> Result<()> {
        // Only the protected path and its parent listing may change. Retain every
        // unrelated captured source/identity; never absorb a raw editor's bytes.
        let d = Directory::open(p.parent().unwrap())?;
        let name = p.file_name().unwrap().to_str().unwrap();
        let s = d.optional(name)?;
        if s.as_ref().map(|s| s.raw.as_slice()) != after {
            return Err(Error::new("conflict", "publication differs from planned bytes").at(p));
        }
        match s {
            Some(s) => {
                self.files.insert(p.to_owned(), s);
            }
            None => {
                self.files.remove(p);
            }
        }
        if let Some((id, names)) = self.dirs.get_mut(&d.path) {
            if *id != d.identity {
                return Err(Error::new("conflict", "publication directory substituted").at(&d.path));
            }
            names.retain(|n| n != p.file_name().unwrap());
            if after.is_some() {
                names.push(p.file_name().unwrap().to_owned());
                names.sort();
            }
        } else {
            self.directory(&d)?;
        }
        self.recheck()
    }
    pub(crate) fn ensure(&mut self, p: &Path) -> Result<Directory> {
        if let Ok(d) = Directory::open(p) {
            if let Some((id, names)) = self.dirs.get(p)
                && (id != &d.identity || names != &visible(d.names()?))
            {
                return Err(Error::new(
                    "conflict",
                    "directory changed while preparing publication",
                )
                .at(p));
            }
            self.directory(&d)?;
            return Ok(d);
        }
        let parent = p
            .parent()
            .ok_or_else(|| Error::new("unsafe_path", "no directory parent"))?;
        let d = self.ensure(parent)?;
        self.recheck()?;
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::new("unsafe_path", "invalid directory name"))?;
        let child = d.ensure(name)?;
        if let Some((id, names)) = self.dirs.get_mut(parent) {
            if *id != d.identity {
                return Err(Error::new("conflict", "directory identity changed").at(parent));
            }
            if !names.contains(&p.file_name().unwrap().to_owned()) {
                names.push(p.file_name().unwrap().to_owned());
                names.sort();
            }
        }
        self.directory(&child)?;
        self.recheck()?;
        Ok(child)
    }
    pub(crate) fn publish(&mut self, p: &Path, raw: &[u8]) -> Result<Option<PathBuf>> {
        self.publish_named(p, raw, &format!(".stage-{}", new_id()?))
    }
    pub(crate) fn publish_named(
        &mut self,
        p: &Path,
        raw: &[u8],
        stage: &str,
    ) -> Result<Option<PathBuf>> {
        self.recheck()?;
        let d = self.ensure(p.parent().unwrap())?;
        let old = self.files.get(p).cloned();
        let publication_key = if old.is_some() { "updated" } else { "created" };
        let recovery = d
            .publish(
                p.file_name().unwrap().to_str().unwrap(),
                raw,
                old.as_ref(),
                stage,
            )
            .map_err(|e| {
                let e: Error = e.into();
                if e.details["publication"] == "published" {
                    capture_published(e, p, None, publication_key)
                } else {
                    e
                }
            })?;
        self.advance(p, Some(raw))
            .map_err(|e| capture_published(e, p, recovery.as_deref(), publication_key))?;
        Ok(recovery)
    }
    pub(crate) fn delete(&mut self, p: &Path) -> Result<()> {
        self.recheck()?;
        let d = Directory::open(p.parent().unwrap())?;
        let name = p.file_name().unwrap().to_str().unwrap();
        let s = self
            .files
            .get(p)
            .ok_or_else(|| Error::new("conflict", "missing deletion snapshot"))?;
        if d.read(name)? != *s {
            return Err(Error::new("conflict", "deletion source changed").at(p));
        }
        rustix::fs::unlinkat(&d.file, name, rustix::fs::AtFlags::empty())
            .map_err(|e| io(e.into()))?;
        (|| -> Result<()> {
            #[cfg(test)]
            super::storage::files::inject("scope_after_unlink", p)?;
            d.sync()?;
            self.advance(p, None)
        })()
        .map_err(|e| capture_published(e, p, None, "deleted"))
    }
}
fn capture_published(mut e: Error, p: &Path, recovery: Option<&Path>, key: &str) -> Error {
    if let Some(cause) = &e.path {
        e.message = format!("{} (post-publication check: {})", e.message, path(cause));
    }
    e.path = Some(p.to_owned());
    e.details["publication"] = json!("published");
    let mut known = partial();
    known[key] = json!([{"id":null,"path":path(p)}]);
    e.details["partial"] = known;
    if let Some(r) = recovery {
        e.details["previous_source_path"] = json!(path(r));
    }
    e
}
#[derive(Clone)]
pub(crate) struct Physical {
    pub checkout: Checkout,
    pub material: ItemStore,
    pub anchors: BTreeMap<String, Value>,
    pub children: BTreeMap<String, Vec<String>>,
    pub records: BTreeMap<PathBuf, Value>,
    pub digests: BTreeMap<PathBuf, Value>,
    pub diagnostics: Vec<Value>,
    pub declaration: Option<Value>,
    pub pending: Vec<Value>,
    pub available: bool,
}
impl Physical {
    fn metadata(c: Checkout) -> Self {
        Self {
            available: c.available,
            checkout: c,
            material: ItemStore::from_candidate_files(vec![]),
            anchors: BTreeMap::new(),
            children: BTreeMap::new(),
            records: BTreeMap::new(),
            digests: BTreeMap::new(),
            diagnostics: vec![],
            declaration: None,
            pending: vec![],
        }
    }
    fn load(c: Checkout, cap: &mut Capture, read_declaration: bool) -> Self {
        let mut out = Self {
            checkout: c,
            material: ItemStore::from_candidate_files(vec![]),
            anchors: BTreeMap::new(),
            children: BTreeMap::new(),
            records: BTreeMap::new(),
            digests: BTreeMap::new(),
            diagnostics: vec![],
            declaration: None,
            pending: vec![],
            available: false,
        };
        let result = (|| -> Result<()> {
            if read_declaration
                && let Some(git) = &out.checkout.git_dir
                && let Some(s) = cap
                    .file(&git.join("work-beta2-hosts.yaml"))
                    .map_err(|mut e| {
                        if let Some(cause) = &e.path {
                            e.message = format!(
                                "{} (hosting declaration read via {})",
                                e.message,
                                path(cause)
                            );
                        }
                        e.path = Some(git.join("work-beta2-hosts.yaml"));
                        e
                    })?
            {
                match wire::envelope(
                    "HostDeclaration",
                    &s.raw,
                    &git.join("work-beta2-hosts.yaml"),
                ) {
                    Ok(declaration) => out.declaration = Some(declaration),
                    Err(e) => {
                        out.diagnostics
                            .push(diagnostic(e.code, e.message, e.path.as_deref(), None))
                    }
                }
            }
            if !out.checkout.available {
                return Err(Error::new(
                    "source_unavailable",
                    "working directory is not the inventoried Git checkout",
                )
                .at(&out.checkout.root));
            }
            let d = Directory::open(&out.checkout.root)?;
            cap.directory(&d)?;
            out.available = true;
            if !d.exists(".work")? {
                return Ok(());
            }
            let work = d.child(".work")?;
            cap.directory(&work)?;
            // Catalogs are independent scan roots. A failed material capture must
            // not hide the scope references needed to resolve another checkout.
            if let Err(e) = (|| -> Result<()> {
                if work.exists("items")? {
                    let items = work.child("items")?;
                    cap.directory(&items)?;
                    #[cfg(test)]
                    super::storage::files::inject("scope_before_material_load", &items.path)?;
                    out.material = ItemStore::load_from_root(&out.checkout.root).map_err(io)?;
                    #[cfg(test)]
                    super::storage::files::inject("scope_after_material_load", &items.path)?;
                    let loaded_names = visible(
                        out.material
                            .files
                            .iter()
                            .map(|f| f.path.file_name().unwrap().to_os_string())
                            .collect(),
                    );
                    if loaded_names != cap.dirs.get(&items.path).unwrap().1 {
                        return Err(Error::new(
                            "conflict",
                            "material catalog file set changed during capture",
                        )
                        .at(&items.path));
                    }
                    for file in &out.material.files {
                        // Malformed/symlink item diagnostics remain inspectable.
                        if file
                            .fingerprint
                            .as_ref()
                            .is_some_and(|f| f.mode & 0o170000 == 0o100000)
                        {
                            let captured = (|| -> Result<()> {
                                let s = cap.file(&file.path)?.ok_or_else(|| {
                                    Error::new("conflict", "material file vanished").at(&file.path)
                                })?;
                                let f = file.fingerprint.as_ref().unwrap();
                                if file.raw != s.raw
                                    || f.dev != s.identity.dev
                                    || f.ino != s.identity.ino
                                    || f.mode != s.mode
                                    || f.size != s.size
                                    || f.mtime != s.mtime
                                    || f.ctime != s.ctime
                                {
                                    return Err(Error::new(
                                        "conflict",
                                        "material changed while capturing source",
                                    )
                                    .at(&file.path));
                                }
                                Ok(())
                            })();
                            if let Err(e) = captured {
                                out.diagnostics.push(diagnostic(
                                    e.code,
                                    e.message,
                                    e.path.as_deref().or(Some(&file.path)),
                                    None,
                                ));
                            }
                        }
                    }
                }
                Ok(())
            })() {
                out.diagnostics.push(diagnostic(
                    e.code,
                    e.message,
                    e.path.as_deref().or(Some(&work.path)),
                    None,
                ));
            }
            if let Err(e) = (|| -> Result<()> {
                if work.exists("scopes")? {
                    out.walk(&work.child("scopes")?, cap)?;
                }
                Ok(())
            })() {
                out.diagnostics.push(diagnostic(
                    e.code,
                    e.message,
                    e.path.as_deref().or(Some(&work.path.join("scopes"))),
                    None,
                ));
            }
            if let Err(e) = (|| -> Result<()> {
                if work.exists("digests")? {
                    out.walk_digests(&work.child("digests")?, cap)?;
                }
                Ok(())
            })() {
                out.diagnostics.push(diagnostic(
                    e.code,
                    e.message,
                    e.path.as_deref().or(Some(&work.path.join("digests"))),
                    None,
                ));
            }
            Ok(())
        })();
        if let Err(e) = result {
            out.diagnostics.push(diagnostic(
                e.code,
                e.message,
                e.path.as_deref().or(Some(&out.checkout.root)),
                None,
            ));
        }
        out
    }
    fn walk_digests(&mut self, d: &Directory, cap: &mut Capture) -> Result<()> {
        cap.directory(d)?;
        for name in visible(d.names()?) {
            let p = d.path.join(&name);
            let kind = rustix::fs::statat(&d.file, &name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| io(e.into()))?
                .st_mode
                & 0o170000;
            if kind == 0o040000 {
                if let Err(e) = d
                    .child(&name)
                    .map_err(Error::from)
                    .and_then(|child| self.walk_digests(&child, cap))
                {
                    self.diagnostics.push(diagnostic(
                        e.code,
                        e.message,
                        e.path.as_deref().or(Some(&p)),
                        None,
                    ));
                }
                continue;
            }
            let parsed = (|| -> Result<Value> {
                let s = cap
                    .file(&p)?
                    .ok_or_else(|| Error::new("source_unavailable", "digest vanished").at(&p))?;
                let relative = p
                    .strip_prefix(self.checkout.root.join(".work/digests"))
                    .unwrap();
                let components: Vec<_> =
                    relative.iter().map(|s| s.to_str().unwrap_or("")).collect();
                let [root, file] = components.as_slice() else {
                    return Err(Error::new("invalid_format", "invalid digest path").at(&p));
                };
                let (header, body) = frontmatter(&s.raw)?;
                let header = wire::envelope("DigestHeader", header, &p)?;
                if header["root_scope_id"] != *root
                    || file.strip_suffix(".md") != header["run_id"].as_str()
                {
                    return Err(
                        Error::new("invalid_format", "digest path/identity mismatch").at(&p),
                    );
                }
                let body = std::str::from_utf8(body)
                    .map_err(|_| Error::new("invalid_format", "digest body is not UTF-8").at(&p))?;
                Ok(json!({"header":header,"body":body,"path":path(&p)}))
            })();
            match parsed {
                Ok(v) => {
                    self.digests.insert(p, v);
                }
                Err(e) => self.diagnostics.push(diagnostic(
                    e.code,
                    e.message,
                    e.path.as_deref().or(Some(&p)),
                    None,
                )),
            }
        }
        Ok(())
    }
    fn walk(&mut self, d: &Directory, cap: &mut Capture) -> Result<()> {
        cap.directory(d)?;
        let relative = d
            .path
            .strip_prefix(self.checkout.root.join(".work/scopes"))
            .unwrap();
        let parts = relative
            .iter()
            .map(|s| s.to_str().unwrap_or(""))
            .collect::<Vec<_>>();
        let valid_scope = parts.first().is_none_or(|s| *s == "root" || valid_id(s));
        let valid = valid_scope
            && match parts.as_slice() {
                [] | [_] => true,
                [_, collection] => matches!(*collection, "children" | "runs" | "progress"),
                [_, "runs", rid] => valid_id(rid),
                [_, "runs", rid, collection] => {
                    valid_id(rid)
                        && matches!(*collection, "items" | "sessions" | "claims" | "handoffs")
                }
                [_, "progress", kind] => matches!(
                    *kind,
                    "start" | "ending" | "rotation" | "promotion" | "disposal"
                ),
                _ => false,
            };
        if !valid {
            self.diagnostics.push(diagnostic(
                "invalid_format",
                "unknown scope directory layout",
                Some(&d.path),
                None,
            ));
            return Ok(());
        }
        if (parts.len() == 1 && !d.exists("scope.yaml")?)
            || (parts.len() == 3 && parts[1] == "runs" && !d.exists("run.yaml")?)
        {
            self.diagnostics.push(diagnostic(
                "source_unavailable",
                "orphaned scope/run directory lacks its manifest",
                Some(&d.path),
                parts.first().copied(),
            ));
        }
        for name in visible(d.names()?) {
            let p = d.path.join(&name);
            let kind = rustix::fs::statat(&d.file, &name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| io(e.into()))?
                .st_mode
                & 0o170000;
            if kind == 0o040000 {
                if let Err(e) = d
                    .child(&name)
                    .map_err(Error::from)
                    .and_then(|child| self.walk(&child, cap))
                {
                    self.diagnostics.push(diagnostic(
                        e.code,
                        e.message,
                        e.path.as_deref().or(Some(&p)),
                        None,
                    ));
                }
                continue;
            }
            let result = (|| -> Result<()> {
                let s = cap
                    .file(&p)?
                    .ok_or_else(|| Error::new("source_unavailable", "source vanished").at(&p))?;
                let rel = p
                    .strip_prefix(self.checkout.root.join(".work/scopes"))
                    .unwrap();
                let components: Vec<_> = rel.iter().map(|x| x.to_str().unwrap_or("")).collect();
                let sid = components[0];
                if sid != "root" && !valid_id(sid) {
                    return Err(Error::new("invalid_format", "invalid scope path").at(&p));
                }
                let (schema, expected, field) = match components.as_slice() {
                    [_, "scope.yaml"] => ("ScopeAnchor", Some(sid), "scope_id"),
                    [_, "children", f] if f.ends_with(".yaml") => {
                        ("ChildReference", Some(&f[..f.len() - 5]), "child_scope_id")
                    }
                    [_, "runs", r, "run.yaml"] if valid_id(r) => ("RunManifest", Some(*r), "id"),
                    [_, "runs", r, "sessions", f] if valid_id(r) && f.ends_with(".yaml") => {
                        ("NamedSession", Some(&f[..f.len() - 5]), "id")
                    }
                    [_, "runs", r, "claims", f] if valid_id(r) && f.ends_with(".end.yaml") => {
                        ("ClaimEnding", Some(&f[..f.len() - 9]), "claim_id")
                    }
                    [_, "runs", r, "claims", f] if valid_id(r) && f.ends_with(".yaml") => {
                        ("ClaimAcquisition", Some(&f[..f.len() - 5]), "id")
                    }
                    [_, "runs", r, "items", f] if valid_id(r) && f.ends_with(".md") => {
                        let item = super::items::parse_candidate(p.clone(), s.raw.clone());
                        if let Some(id) = item
                            .header
                            .as_ref()
                            .map(|h| h.id.as_str())
                            .or_else(|| f.strip_suffix(".md").filter(|id| valid_id(id)))
                        {
                            self.records.insert(p.clone(), json!({"wisp_id":id}));
                        }
                        if !item.is_valid() {
                            return Err(
                                Error::new("invalid_source", "invalid retained wisp").at(&p)
                            );
                        }
                        return Ok(());
                    }
                    [_, "runs", r, "handoffs", f] if valid_id(r) && f.ends_with(".md") => {
                        ("HandoffHeader", Some(&f[..f.len() - 3]), "id")
                    }
                    [_, "progress", kind, f] if f.ends_with(".yaml") => (
                        match *kind {
                            "start" => "StartProgress",
                            "ending" => "EndingProgress",
                            "rotation" => "RotationProgress",

                            "promotion" => "PromotionProgress",
                            "disposal" => "DisposalProgress",
                            _ => {
                                return Err(
                                    Error::new("invalid_format", "unknown progress kind").at(&p)
                                );
                            }
                        },
                        Some(&f[..f.len() - 5]),
                        "id",
                    ),
                    _ => {
                        return Err(
                            Error::new("invalid_format", "unknown scope entity path").at(&p)
                        );
                    }
                };
                let raw = if schema == "HandoffHeader" {
                    let (head, body) = frontmatter(&s.raw)?;
                    std::str::from_utf8(body).map_err(|_| {
                        Error::new("invalid_format", "handoff body is not UTF-8").at(&p)
                    })?;
                    head
                } else {
                    s.raw.as_slice()
                };
                let v = wire::envelope(schema, raw, &p)?;
                if expected.is_some_and(|id| v[field].as_str() != Some(id)) {
                    return Err(Error::new("invalid_format", "path/identity mismatch").at(&p));
                }
                if schema == "ScopeAnchor" {
                    self.anchors.insert(sid.to_owned(), v.clone());
                }
                if schema == "ChildReference" {
                    self.children
                        .entry(sid.to_owned())
                        .or_default()
                        .push(v[field].as_str().unwrap().to_owned());
                }
                self.records.insert(p.clone(), v);
                Ok(())
            })();
            if let Err(e) = result {
                if let Ok(relative) = p.strip_prefix(self.checkout.root.join(".work/scopes")) {
                    let parts = relative
                        .iter()
                        .map(|s| s.to_str().unwrap_or(""))
                        .collect::<Vec<_>>();
                    if parts.len() == 4 && parts[1] == "progress" {
                        self.pending.push(json!({"id":null,"kind":parts[2],"path":path(&p),"step":null,"supported":false}));
                    }
                }
                self.diagnostics.push(diagnostic(
                    e.code,
                    e.message,
                    e.path.as_deref().or(Some(&p)),
                    None,
                ));
            }
        }
        Ok(())
    }
}
pub(crate) fn frontmatter(raw: &[u8]) -> Result<(&[u8], &[u8])> {
    let mut off = 0;
    let mut start = None;
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let stripped = line
            .strip_suffix(b"\n")
            .unwrap_or(line)
            .strip_suffix(b"\r")
            .unwrap_or(line.strip_suffix(b"\n").unwrap_or(line));
        if off == 0 {
            if stripped != b"---" {
                break;
            }
            start = Some(line.len());
        } else if stripped == b"---" {
            return Ok((&raw[start.unwrap()..off], &raw[off + line.len()..]));
        }
        off += line.len();
    }
    Err(Error::new("invalid_format", "invalid frontmatter framing"))
}

#[derive(Clone)]
pub(crate) struct View {
    pub project: Project,
    pub physical: Vec<Physical>,
    pub capture: Capture,
    pub hosts: BTreeMap<String, Option<PathBuf>>,
    pub parents: BTreeMap<String, String>,
    pub anchors: BTreeMap<String, Value>,
    pub store: ItemStore,
    pub diagnostics: Vec<Value>,
    pub pending: Vec<Value>,
    pub physical_mode: bool,
}
impl View {
    pub fn load(project: &Project, physical_mode: bool) -> Result<Self> {
        let inventory = project.checkouts().map_err(discovery)?;
        let mut capture = Capture {
            inventory: Some((project.clone(), inventory.clone())),
            ..Capture::default()
        };
        let physical = inventory
            .into_iter()
            .map(|c| {
                if !physical_mode || c.root == project.worktree_root {
                    Physical::load(c, &mut capture, !physical_mode)
                } else {
                    Physical::metadata(c)
                }
            })
            .collect::<Vec<_>>();
        let mut v = Self {
            project: project.clone(),
            physical,
            capture,
            hosts: BTreeMap::new(),
            parents: BTreeMap::new(),
            anchors: BTreeMap::new(),
            store: ItemStore::from_candidate_files(vec![]),
            diagnostics: vec![],
            pending: vec![],
            physical_mode,
        };
        if !physical_mode {
            let common = Directory::open(&project.git_common_dir)?;
            v.capture.directory(&common)?;
            if common.exists("work")? {
                v.diagnostics.push(diagnostic(
                    "obsolete_execution",
                    "obsolete beta.1 execution store; no migration/dual runtime",
                    Some(&common.path.join("work")),
                    None,
                ));
            }
            if common.exists("work-beta2")? {
                let dir = match common.child("work-beta2") {
                    Ok(d) => Some(d),
                    Err(e) => {
                        v.diagnostics.push(diagnostic(
                            "coordination_unavailable",
                            e.message,
                            e.path.as_deref(),
                            None,
                        ));
                        None
                    }
                };
                if let Some(dir) = dir {
                    v.capture.directory(&dir)?;
                    if dir.exists("coordination.lock")?
                        && let Err(e) = v.capture.file(&dir.path.join("coordination.lock"))
                    {
                        v.diagnostics.push(diagnostic(
                            "coordination_unavailable",
                            e.message,
                            e.path.as_deref(),
                            None,
                        ));
                    }
                    if dir.exists("relocations")? {
                        let relocations = dir.child("relocations")?;
                        v.capture.directory(&relocations)?;
                        for name in visible(relocations.names()?) {
                            let p = relocations.path.join(&name);
                            let result = (|| -> Result<Value> {
                                let s = v.capture.file(&p)?.unwrap();
                                let x = wire::envelope("HostTransferProgress", &s.raw, &p)?;
                                if name.to_str()
                                    != Some(&format!("{}.yaml", x["id"].as_str().unwrap()))
                                {
                                    return Err(Error::new(
                                        "invalid_format",
                                        "progress ID mismatch",
                                    )
                                    .at(&p));
                                }
                                Ok(x)
                            })();
                            match result {Ok(plan)=>v.pending.push(json!({"id":plan["id"],"kind":"host_transfer","path":path(&p),"step":plan["step"],"supported":plan["mode"]=="host"})),Err(e)=>{v.diagnostics.push(diagnostic(e.code,e.message,Some(&p),None));v.pending.push(json!({"id":null,"kind":"host_transfer","path":path(&p),"step":null,"supported":false}));}}
                        }
                    }
                }
            }
        }
        if physical_mode {
            let selected = v
                .physical
                .iter()
                .find(|p| p.checkout.root == project.worktree_root)
                .ok_or_else(|| {
                    Error::new(
                        "source_unavailable",
                        "selected checkout absent from Git inventory",
                    )
                })?;
            let mut files = selected.material.files.clone();
            for (p, record) in &selected.records {
                if record.get("wisp_id").is_some()
                    && let Some(s) = v.capture.files.get(p)
                {
                    files.push(super::items::parse_candidate(p.clone(), s.raw.clone()));
                }
            }
            v.store = ItemStore::from_candidate_files(files);
            v.anchors = selected.anchors.clone();
            v.diagnostics.extend(selected.diagnostics.clone());
            // Preserve local interruption evidence, including inactive copies,
            // without resolving or authorizing execution in physical mode.
            v.pending.extend(selected.pending.iter().cloned());
            for (file, record) in &selected.records {
                if record.get("step").is_some() {
                    v.pending.push(json!({"id":record["id"],
                        "kind":file.parent().unwrap().file_name().unwrap().to_str(),
                        "path":path(file),"step":record["step"],
                        "supported":record.get("tree_change").is_some()}));
                }
            }
            for sid in v.anchors.keys() {
                v.hosts
                    .insert(sid.clone(), Some(project.worktree_root.clone()));
            }
            v.hosts
                .insert("root".into(), Some(project.worktree_root.clone()));
            for (p, kids) in &selected.children {
                for kid in kids {
                    v.parents.insert(kid.clone(), p.clone());
                }
            }
            return Ok(v);
        }
        for p in &v.physical {
            if let Some(g) = &p.checkout.git_dir {
                let declaration_path = path(&g.join("work-beta2-hosts.yaml"));
                v.diagnostics.extend(
                    p.diagnostics
                        .iter()
                        .filter(|d| d["path"] == declaration_path)
                        .cloned(),
                );
            }
        }
        let primary = v.physical[0].checkout.root.clone();
        v.visit("root", &primary, &mut BTreeSet::new());
        v.unreachable_declarations();
        v.resolve_material();
        v.physical_diagnostics();
        v.check_tree();
        v.validate_records();
        Ok(v)
    }
    pub(crate) fn declarations_unresolved(&self) -> bool {
        self.physical.iter().any(|p| {
            p.checkout.git_dir.as_ref().is_some_and(|g| {
                p.diagnostics
                    .iter()
                    .any(|d| d["path"] == path(&g.join("work-beta2-hosts.yaml")))
            })
        })
    }
    pub(crate) fn declared(&self, sid: &str) -> Vec<&Physical> {
        self.physical
            .iter()
            .filter(|p| {
                p.declaration.as_ref().is_some_and(|d| {
                    d["scope_ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|id| id == sid)
                })
            })
            .collect()
    }
    pub(crate) fn select_prospective(
        &mut self,
        sid: &str,
        destination: &Path,
        new_boundaries: &[Value],
    ) {
        for p in &mut self.physical {
            let mut ids = p
                .declaration
                .as_ref()
                .map(|v| v["scope_ids"].as_array().unwrap().clone())
                .unwrap_or_default();
            ids.retain(|v| v != sid);
            if p.checkout.root == destination {
                ids.push(json!(sid));
            }
            ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            p.declaration = Some(json!({"format_version":2,"scope_ids":ids}));
        }
        for boundary in new_boundaries {
            let checkout = decode_path(boundary["checkout"].as_str().unwrap()).unwrap();
            let id = boundary["scope_id"].as_str().unwrap();
            if let Some(p) = self
                .physical
                .iter_mut()
                .find(|p| p.checkout.root == checkout)
            {
                p.anchors
                    .insert(id.into(), boundary["anchor_after"].clone());
            }
            if let Some(parent) = boundary["parent_scope_id"].as_str() {
                let host = self
                    .hosts
                    .get(parent)
                    .and_then(|h| h.as_ref())
                    .cloned()
                    .unwrap_or_else(|| self.physical[0].checkout.root.clone());
                if let Some(p) = self.physical.iter_mut().find(|p| p.checkout.root == host) {
                    let kids = p.children.entry(parent.into()).or_default();
                    if !kids.iter().any(|c| c == id) {
                        kids.push(id.into());
                    }
                }
            }
        }
        self.hosts.clear();
        self.parents.clear();
        self.anchors.clear();
        self.diagnostics
            .retain(|d| d["code"] == "obsolete_execution");
        let primary = self.physical[0].checkout.root.clone();
        self.visit("root", &primary, &mut BTreeSet::new());
        self.unreachable_declarations();
        self.resolve_material();
        self.physical_diagnostics();
        self.check_tree();
        self.validate_records();
    }
    fn visit(&mut self, sid: &str, inherited: &Path, seen: &mut BTreeSet<String>) {
        if !seen.insert(sid.into()) {
            if self.declared(sid).is_empty()
                && self
                    .hosts
                    .get(sid)
                    .and_then(|h| h.as_deref())
                    .is_some_and(|old| old != inherited)
            {
                let mut pending = vec![sid.to_owned()];
                let mut fenced = BTreeSet::new();
                while let Some(id) = pending.pop() {
                    if !fenced.insert(id.clone()) {
                        continue;
                    }
                    self.hosts.insert(id.clone(), None);
                    self.anchors.remove(&id);
                    pending.extend(
                        self.parents
                            .iter()
                            .filter(|(_, parent)| *parent == &id)
                            .map(|(child, _)| child.clone())
                            .filter(|child| self.declared(child).is_empty())
                            .collect::<Vec<_>>(),
                    );
                }
                self.diagnostics.push(diagnostic(
                    "host_ambiguous",
                    "competing inherited hosting paths",
                    None,
                    Some(sid),
                ));
            }
            self.diagnostics.push(diagnostic(
                "invalid_source",
                "repeated/cyclic stored child reference",
                None,
                Some(sid),
            ));
            return;
        }
        let declarations = self.declared(sid);
        let host = if sid != "root" && self.declarations_unresolved() {
            None
        } else {
            match declarations.as_slice() {
                [] => Some(inherited.to_owned()),
                [p] => Some(p.checkout.root.clone()),
                _ => None,
            }
        };
        if host.is_none() {
            self.diagnostics.push(diagnostic(
                "host_ambiguous",
                "multiple dedicated host declarations",
                None,
                Some(sid),
            ));
        }
        self.hosts.insert(sid.into(), host.clone());
        let Some(host) = host else {
            return;
        };
        let Some(physical) = self.physical.iter().find(|p| p.checkout.root == host) else {
            return;
        };
        if !physical.available {
            self.diagnostics.push(diagnostic(
                "source_unavailable",
                "hosting checkout unavailable",
                Some(&host),
                Some(sid),
            ));
            return;
        }
        if let Some(anchor) = physical.anchors.get(sid) {
            self.anchors.insert(sid.into(), anchor.clone());
        } else if sid != "root" {
            self.diagnostics.push(diagnostic(
                "host_missing",
                "authoritative anchor is missing",
                Some(&host.join(format!(".work/scopes/{sid}/scope.yaml"))),
                Some(sid),
            ));
            return;
        }
        let children = physical.children.get(sid).cloned().unwrap_or_default();
        for child in children {
            self.parents.insert(child.clone(), sid.into());
            self.visit(&child, &host, seen);
        }
    }
    fn unreachable_declarations(&mut self) {
        let ids = self
            .physical
            .iter()
            .filter_map(|p| p.declaration.as_ref())
            .flat_map(|d| {
                d["scope_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap().to_owned())
            })
            .collect::<BTreeSet<_>>();
        for id in ids {
            if !self.hosts.contains_key(&id) {
                self.hosts.insert(id.clone(), None);
                self.diagnostics.push(diagnostic(
                    "source_unavailable",
                    "declared scope is unreachable through parent-local references",
                    None,
                    Some(&id),
                ));
            }
        }
    }
    fn resolve_material(&mut self) {
        if self.declarations_unresolved() {
            self.store = ItemStore::from_candidate_files(vec![]);
            return;
        }
        let mut ids = BTreeSet::new();
        for p in &self.physical {
            for f in &p.material.files {
                if let Some(h) = &f.header {
                    ids.insert(h.id.clone());
                } else if let Some(id) = f.path.file_stem().and_then(|x| x.to_str())
                    && valid_id(id)
                {
                    ids.insert(id.into());
                }
            }
        }
        let mut selected = BTreeMap::<String, ItemFile>::new();
        let wisp_files = self
            .records()
            .into_iter()
            .filter(|(_, v, _, _)| v.get("wisp_id").is_some())
            .filter_map(|(p, _, _, _)| {
                self.capture
                    .files
                    .get(&p)
                    .map(|s| super::items::parse_candidate(p, s.raw.clone()))
            })
            .collect::<Vec<_>>();
        let mut wisps = BTreeMap::new();
        let mut duplicate = BTreeSet::new();
        for f in &wisp_files {
            if let Some(h) = &f.header
                && wisps.insert(h.id.clone(), f.clone()).is_some()
            {
                duplicate.insert(h.id.clone());
            }
        }
        for id in duplicate {
            wisps.remove(&id);
            self.diagnostics.push(diagnostic(
                "claim_conflict",
                "duplicate run-owned wisp identity",
                None,
                Some(&id),
            ));
        }
        // Resolve candidate parent chains without combining their header/body.
        // Conflicting self-consistent ancestry remains a diagnostic, never a
        // branch/newest preference or a fallback to a plausible physical copy.
        let mut ambiguous = BTreeSet::new();
        for _ in 0..=ids.len() {
            let mut next = BTreeMap::new();
            ambiguous.clear();
            for id in &ids {
                let mut candidates = Vec::new();
                for physical in &self.physical {
                    for f in &physical.material.files {
                        if f.header
                            .as_ref()
                            .map(|h| h.id.as_str())
                            .or_else(|| f.path.file_stem().and_then(|s| s.to_str()))
                            != Some(id)
                        {
                            continue;
                        }
                        let host = if self.hosts.contains_key(id) {
                            self.hosts[id].clone()
                        } else if let Some(parent) =
                            f.header.as_ref().and_then(|h| h.parent.as_deref())
                        {
                            if self.hosts.contains_key(parent) {
                                self.hosts[parent].clone()
                            } else {
                                self.parent_host(parent, &selected, &wisps, &mut BTreeSet::new())
                                    .or_else(|| self.invalid_material_host(id, &selected, &wisps))
                            }
                        } else if f.header.is_none() {
                            self.invalid_material_host(id, &selected, &wisps)
                        } else {
                            self.hosts.get("root").cloned().flatten()
                        };
                        if host.as_ref() == Some(&physical.checkout.root) {
                            candidates.push(f.clone());
                        }
                    }
                }
                if candidates.len() == 1 {
                    next.insert(id.clone(), candidates.remove(0));
                } else if candidates.len() > 1 {
                    ambiguous.insert(id.clone());
                }
            }
            let stable = next.len() == selected.len()
                && next
                    .iter()
                    .all(|(id, f)| selected.get(id).is_some_and(|old| old.path == f.path));
            selected = next;
            if stable {
                break;
            }
        }
        for id in &ambiguous {
            self.diagnostics.push(diagnostic(
                "host_ambiguous",
                "conflicting material ancestry candidates",
                None,
                Some(id),
            ));
        }
        let mut required = self
            .hosts
            .keys()
            .filter(|id| id.as_str() != "root")
            .cloned()
            .collect::<BTreeSet<_>>();
        required.extend(ambiguous.iter().cloned());
        for f in selected.values().chain(wisp_files.iter()) {
            if let Some(h) = &f.header {
                required.extend(
                    h.parent
                        .iter()
                        .chain(&h.depends_on)
                        .chain(&h.related)
                        .chain(&h.discovered_from)
                        .filter(|id| ids.contains(*id))
                        .cloned(),
                );
            }
        }
        for p in &self.physical {
            if self
                .hosts
                .values()
                .any(|host| host.as_ref() == Some(&p.checkout.root))
            {
                for f in &p.material.files {
                    let id = f.header.as_ref().map(|h| h.id.as_str()).or_else(|| {
                        f.path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .filter(|id| valid_id(id))
                    });
                    if let Some(id) = id {
                        let host = if self.hosts.contains_key(id) {
                            self.hosts[id].clone()
                        } else if let Some(parent) =
                            f.header.as_ref().and_then(|h| h.parent.as_deref())
                        {
                            self.parent_host(parent, &selected, &wisps, &mut BTreeSet::new())
                                .or_else(|| self.invalid_material_host(id, &selected, &wisps))
                        } else if f.header.is_none() {
                            self.invalid_material_host(id, &selected, &wisps)
                        } else {
                            self.hosts.get("root").cloned().flatten()
                        };
                        if host.is_none() || host.as_ref() == Some(&p.checkout.root) {
                            if host.is_none() && f.header.is_none() {
                                self.diagnostics.push(diagnostic(
                                    "invalid_source",
                                    "unresolved malformed material header",
                                    Some(&f.path),
                                    Some(id),
                                ));
                            }
                            required.insert(id.to_owned());
                        }
                    }
                }
                for digest in p.digests.values() {
                    if let Some(id) = digest["header"]["root_scope_id"]
                        .as_str()
                        .filter(|id| *id != "root")
                    {
                        required.insert(id.to_owned());
                    }
                }
            }
        }
        for id in required {
            if !selected.contains_key(&id) && !wisps.contains_key(&id) {
                self.diagnostics.push(diagnostic(
                    "source_unavailable",
                    "cannot resolve whole authoritative material source",
                    None,
                    Some(&id),
                ));
            }
        }
        let mut files = selected.into_values().chain(wisp_files).collect::<Vec<_>>();
        for p in &self.physical {
            if self
                .hosts
                .values()
                .any(|h| h.as_ref() == Some(&p.checkout.root))
            {
                files.extend(
                    p.material
                        .files
                        .iter()
                        .filter(|f| {
                            f.header.is_none()
                                && !f
                                    .path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .is_some_and(valid_id)
                        })
                        .cloned(),
                );
            }
        }
        self.store = ItemStore::from_candidate_files(files);
    }
    fn physical_diagnostics(&mut self) {
        for p in &self.physical {
            for d in &p.diagnostics {
                let file = d["path"].as_str().and_then(|s| decode_path(s).ok());
                let scope = file
                    .as_ref()
                    .and_then(|f| f.strip_prefix(p.checkout.root.join(".work/scopes")).ok())
                    .and_then(|r| r.iter().next())
                    .and_then(|s| s.to_str())
                    .filter(|s| *s == "root" || valid_id(s));
                let digest = file
                    .as_ref()
                    .and_then(|f| f.strip_prefix(p.checkout.root.join(".work/digests")).ok())
                    .and_then(|r| r.iter().next())
                    .and_then(|s| s.to_str());
                let material_id = file.as_ref().and_then(|f| {
                    f.strip_prefix(p.checkout.root.join(".work/items")).ok()?;
                    p.material
                        .files
                        .iter()
                        .find(|candidate| &candidate.path == f)
                        .and_then(|candidate| candidate.header.as_ref())
                        .map(|h| h.id.clone())
                        .or_else(|| {
                            f.file_stem()
                                .and_then(|s| s.to_str())
                                .filter(|s| valid_id(s))
                                .map(str::to_owned)
                        })
                });
                let relevant = if let Some(sid) = scope {
                    self.source_host(sid).as_ref() == Some(&p.checkout.root)
                } else if let Some(id) = digest {
                    self.source_host(id).as_ref() == Some(&p.checkout.root)
                } else if let Some(id) = material_id {
                    self.source_host(&id).as_ref() == Some(&p.checkout.root)
                } else {
                    self.hosts
                        .values()
                        .any(|h| h.as_ref() == Some(&p.checkout.root))
                };
                if relevant {
                    self.diagnostics.push(d.clone());
                }
            }
        }
    }
    fn invalid_material_host(
        &self,
        id: &str,
        material: &BTreeMap<String, ItemFile>,
        wisps: &BTreeMap<String, ItemFile>,
    ) -> Option<PathBuf> {
        let hosts = self
            .physical
            .iter()
            .flat_map(|p| &p.material.files)
            .filter_map(|f| f.header.as_ref().filter(|h| h.id == id))
            .filter_map(|h| match h.parent.as_deref() {
                Some(parent) => self.parent_host(parent, material, wisps, &mut BTreeSet::new()),
                None => self.hosts.get("root").cloned().flatten(),
            })
            .collect::<BTreeSet<_>>();
        if hosts.len() == 1 {
            hosts.into_iter().next()
        } else if hosts.is_empty() && self.hosts.values().all(Option::is_some) {
            let known = self
                .hosts
                .values()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>();
            if known.len() == 1 {
                known.into_iter().next()
            } else {
                None
            }
        } else {
            None
        }
    }
    fn parent_host(
        &self,
        id: &str,
        material: &BTreeMap<String, ItemFile>,
        wisps: &BTreeMap<String, ItemFile>,
        seen: &mut BTreeSet<String>,
    ) -> Option<PathBuf> {
        if !seen.insert(id.into()) {
            return None;
        }
        if let Some(host) = self.hosts.get(id) {
            return host.clone();
        }
        let header = material
            .get(id)
            .or_else(|| wisps.get(id))?
            .header
            .as_ref()?;
        match header.parent.as_deref() {
            Some(parent) => self.parent_host(parent, material, wisps, seen),
            None => self.hosts.get("root").cloned().flatten(),
        }
    }
    pub fn nearest(&self, id: Option<&str>) -> String {
        let mut current = id.map(str::to_owned);
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id.clone()) {
                break;
            }
            if self.hosts.contains_key(&id) {
                return id;
            }
            current = self.store.files.iter().find_map(|f| {
                f.header
                    .as_ref()
                    .filter(|h| h.id == id)
                    .and_then(|h| h.parent.clone())
            });
        }
        "root".into()
    }
    pub(crate) fn raw_material(&self, id: &str) -> Result<ItemStore> {
        let valid = self
            .store
            .files
            .iter()
            .filter_map(|f| {
                f.header
                    .as_ref()
                    .filter(|h| self.store.resolve(&h.id).is_ok())
                    .map(|h| (h.id.clone(), f.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        if !self.physical_mode && self.declarations_unresolved() {
            return Err(Error::new(
                "source_unavailable",
                "hosting declarations unreadable/unresolved",
            ));
        }
        let valid = &valid;
        let candidates = self
            .physical
            .iter()
            .filter(|p| !self.physical_mode || p.checkout.root == self.project.worktree_root)
            .flat_map(|p| {
                p.material
                    .files
                    .iter()
                    .filter(move |f| {
                        f.header.as_ref().is_some_and(|h| h.id == id)
                            || f.path.file_stem().and_then(|s| s.to_str()) == Some(id)
                    })
                    .filter(move |f| {
                        self.physical_mode || {
                            let expected = if let Some(host) = self.hosts.get(id) {
                                host.clone()
                            } else if let Some(parent) =
                                f.header.as_ref().and_then(|h| h.parent.as_deref())
                            {
                                self.source_host(parent).or_else(|| {
                                    self.invalid_material_host(id, valid, &BTreeMap::new())
                                })
                            } else if f.header.is_none() {
                                self.invalid_material_host(id, valid, &BTreeMap::new())
                            } else {
                                self.hosts.get("root").cloned().flatten()
                            };
                            expected.as_ref() == Some(&p.checkout.root)
                        }
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        let hosts = candidates
            .iter()
            .map(|f| f.path.parent().unwrap().parent().unwrap().parent().unwrap())
            .collect::<BTreeSet<_>>();
        if hosts.len() > 1 {
            return Err(Error::new(
                "host_ambiguous",
                "raw material sources have conflicting authority",
            ));
        }
        Ok(ItemStore::from_candidate_files(candidates))
    }
    pub(crate) fn is_wisp(&self, file: &ItemFile) -> bool {
        self.physical.iter().filter(|p|file.path.starts_with(&p.checkout.root)).max_by_key(|p|p.checkout.root.components().count()).is_some_and(|p|file.path.strip_prefix(p.checkout.root.join(".work/scopes")).is_ok_and(|rel|{let parts=rel.iter().map(|s|s.to_str().unwrap_or("")).collect::<Vec<_>>();matches!(parts.as_slice(),[sid,"runs",run,"items",name] if (*sid=="root"||valid_id(sid))&&valid_id(run)&&name.ends_with(".md"))}))
    }
    pub fn source_host(&self, id: &str) -> Option<PathBuf> {
        if !self.physical_mode && id != "root" && self.declarations_unresolved() {
            return None;
        }
        if self.physical_mode {
            return Some(self.project.worktree_root.clone());
        }
        if !self.hosts.contains_key(id)
            && self.store.resolve(id).is_err()
            && self.physical.iter().any(|p| {
                p.material
                    .files
                    .iter()
                    .any(|f| f.header.as_ref().is_some_and(|h| h.id == id))
            })
        {
            return None;
        }
        self.hosts.get(&self.nearest(Some(id))).cloned().flatten()
    }
    pub fn scope_id(&self, input: &str) -> Result<String> {
        if input == "root" {
            return Ok(input.into());
        }
        wire::validate("ScopeRef", &json!(input))?;
        let q = input.strip_prefix("w-").unwrap_or(input);
        let mut ids: BTreeSet<String> = self
            .store
            .files
            .iter()
            .filter_map(|f| {
                f.header.as_ref().map(|h| h.id.clone()).or_else(|| {
                    f.path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .filter(|s| valid_id(s))
                        .map(str::to_owned)
                })
            })
            .chain(self.hosts.keys().filter(|s| s.as_str() != "root").cloned())
            .filter(|id| id.starts_with(q))
            .collect();
        for p in self
            .physical
            .iter()
            .filter(|p| !self.physical_mode || p.checkout.root == self.project.worktree_root)
        {
            ids.extend(
                p.material
                    .files
                    .iter()
                    .filter_map(|f| f.header.as_ref().map(|h| h.id.clone()))
                    .filter(|id| {
                        id.starts_with(q)
                            && (self.physical_mode
                                || valid_id(q)
                                || self.hosts.contains_key(id)
                                || self.diagnostics.iter().any(|d| {
                                    d["scope_id"] == id.as_str()
                                        && matches!(
                                            d["code"].as_str(),
                                            Some("host_ambiguous" | "source_unavailable")
                                        )
                                }))
                    }),
            );
            ids.extend(
                p.anchors
                    .keys()
                    .filter(|s| self.physical_mode && s.starts_with(q) && s.as_str() != "root")
                    .cloned(),
            );
        }
        match ids.len() {
            0 => Err(Error::new("not_found", "scope/item not found")),
            1 => Ok(ids.into_iter().next().unwrap()),
            _ => Err(Error::new("ambiguous_id", "ambiguous scope/item prefix")),
        }
    }
    pub(crate) fn material_tree_diagnostics(&self) -> Vec<Value> {
        let mut projected = self.clone();
        projected.diagnostics.clear();
        projected.check_tree();
        projected.diagnostics
    }
    fn check_tree(&mut self) {
        // Unreachable local content at its ancestry-derived authority is not
        // evidence of absence: retained ownership must block material writes.
        for p in &self.physical {
            let mut scopes = p.anchors.keys().cloned().collect::<BTreeSet<_>>();
            for path in p.records.keys() {
                if let Ok(rel) = path.strip_prefix(p.checkout.root.join(".work/scopes"))
                    && let Some(sid) = rel.iter().next().and_then(|s| s.to_str())
                {
                    scopes.insert(sid.into());
                }
            }
            for sid in scopes {
                if !self.hosts.contains_key(&sid)
                    && self.source_host(&sid).as_ref() == Some(&p.checkout.root)
                {
                    self.diagnostics.push(diagnostic(
                        "invalid_source",
                        "orphaned authoritative scope/run content lacks parent-local reference",
                        Some(&p.checkout.root.join(format!(".work/scopes/{sid}"))),
                        Some(&sid),
                    ));
                }
            }
        }
        let malformed = self
            .physical
            .iter()
            .flat_map(|p| {
                p.pending
                    .iter()
                    .filter(|pending| {
                        decode_path(pending["path"].as_str().unwrap())
                            .ok()
                            .and_then(|file| {
                                file.strip_prefix(p.checkout.root.join(".work/scopes"))
                                    .ok()
                                    .and_then(|r| {
                                        r.iter().next().and_then(|s| s.to_str()).map(str::to_owned)
                                    })
                            })
                            .is_some_and(|sid| {
                                self.source_host(&sid).as_ref() == Some(&p.checkout.root)
                                    || self.hosts.get(&sid) == Some(&None)
                            })
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        self.pending.extend(malformed);
        for (sid, parent) in &self.parents {
            let Some(root) = self
                .store
                .files
                .iter()
                .filter(|f| !self.is_wisp(f))
                .find_map(|f| {
                    f.header
                        .as_ref()
                        .filter(|h| h.id == *sid && f.diagnostics.is_empty())
                })
            else {
                self.diagnostics.push(diagnostic(
                    "invalid_source",
                    "stored boundary lacks material root",
                    None,
                    Some(sid),
                ));
                continue;
            };
            if self.nearest(root.parent.as_deref()) != *parent {
                self.diagnostics.push(diagnostic(
                    "invalid_source",
                    "child reference disagrees with authoritative material ancestry",
                    None,
                    Some(sid),
                ));
            }
        }
        for p in &self.physical {
            if let Some(d) = &p.declaration {
                for id in d["scope_ids"].as_array().unwrap() {
                    let sid = id.as_str().unwrap();
                    if !self.parents.contains_key(sid) {
                        self.diagnostics.push(diagnostic(
                            "host_missing",
                            "declared scope has no parent-local reference",
                            p.checkout.git_dir.as_deref(),
                            Some(sid),
                        ));
                    }
                }
            }
        }
    }
    fn validate_records(&mut self) {
        // Envelope and ownership validation is shared safety for hosting, not
        // evidence that execution/claim/session mutation workflows are delivered.
        let mut ids = BTreeSet::new();
        let mut claims = BTreeMap::<String, Vec<String>>::new();
        let mut anchors = self.anchors.clone();
        anchors
            .entry("root".into())
            .or_insert_with(|| json!({"execution":{"mode":"uninitialized"}}));
        for (sid, anchor) in &anchors {
            let Some(host) = self.hosts[sid].as_ref() else {
                continue;
            };
            let p = self
                .physical
                .iter()
                .find(|p| &p.checkout.root == host)
                .unwrap();
            let prefix = host.join(format!(".work/scopes/{sid}"));
            let mut runs = BTreeSet::new();
            for (path, v) in &p.records {
                if !path.starts_with(&prefix) {
                    continue;
                }
                if path.file_name().is_some_and(|n| n == "run.yaml") {
                    let rid = v["id"].as_str().unwrap();
                    runs.insert(rid.to_owned());
                    if !ids.insert(rid.to_owned()) {
                        self.diagnostics.push(diagnostic(
                            "invalid_source",
                            "duplicate authoritative run ID",
                            Some(path),
                            Some(sid),
                        ));
                    }
                    if v["phase"] == "active"
                        && (anchor["execution"]["mode"] != "boundary"
                            || anchor["execution"]["run_id"] != rid)
                    {
                        self.diagnostics.push(diagnostic(
                            "invalid_source",
                            "unselected active run",
                            Some(path),
                            Some(sid),
                        ));
                    }
                }
                if path
                    .strip_prefix(&prefix)
                    .is_ok_and(|r| r.iter().next().is_some_and(|c| c == "progress"))
                {
                    self.pending.push(json!({"id":v["id"],"kind":path.parent().unwrap().file_name().unwrap().to_str(),"path":self::path(path),"step":v["step"],"supported":v.get("tree_change").is_some() || (sid == "root" && v.get("new_run_id").is_some() && path.parent().is_some_and(|p| p.file_name().is_some_and(|n| n == "start")))}));
                }
                if let Some(item) = v["item_id"].as_str() {
                    let ending =
                        path.with_file_name(format!("{}.end.yaml", v["id"].as_str().unwrap()));
                    if !p.records.contains_key(&ending) {
                        claims
                            .entry(item.into())
                            .or_default()
                            .push(v["id"].as_str().unwrap().into());
                    }
                }
            }
            if anchor["execution"]["mode"] == "boundary"
                && !runs.contains(anchor["execution"]["run_id"].as_str().unwrap())
            {
                self.diagnostics.push(diagnostic(
                    "source_unavailable",
                    "selected run manifest missing",
                    Some(&prefix),
                    Some(sid),
                ));
            }
        }
        if let Some(anchor) = self.anchors.get("root")
            && anchor["execution"]["mode"] == "boundary"
        {
            let rid = anchor["execution"]["run_id"].as_str().unwrap();
            if let Some(host) = self.hosts.get("root").and_then(|h| h.as_ref())
                && let Some(physical) = self.physical.iter().find(|p| &p.checkout.root == host)
            {
                let manifest = host.join(format!(".work/scopes/root/runs/{rid}/run.yaml"));
                let rotating = physical.records.iter().any(|(p, v)| {
                    p.starts_with(host.join(".work/scopes/root/progress/rotation"))
                        && v["scope_id"] == "root"
                        && v["old_run_id"] == rid
                });
                if physical
                    .records
                    .get(&manifest)
                    .is_some_and(|m| m["phase"] != "active")
                    && !rotating
                {
                    self.diagnostics.push(diagnostic(
                        "invalid_source",
                        "virtual root selects a terminal run without matching pending rotation",
                        Some(&manifest),
                        Some("root"),
                    ));
                }
            }
        }
        let records = self.records();
        let mut acquisitions = BTreeMap::<String, usize>::new();
        for (_, v, _, _) in &records {
            if v.get("item_id").is_some() {
                *acquisitions
                    .entry(v["id"].as_str().unwrap().to_owned())
                    .or_default() += 1;
            }
        }
        for (p, v, s, _) in &records {
            if v.get("item_id").is_some() && acquisitions[v["id"].as_str().unwrap()] > 1 {
                self.diagnostics.push(diagnostic(
                    "claim_conflict",
                    "duplicate authoritative acquisition ID",
                    Some(p),
                    Some(s),
                ));
            }
        }
        let mut names = BTreeSet::new();
        for (path, v, sid, rid) in &records {
            let manifest = records.iter().any(|(p, m, _, r)| {
                r == rid
                    && p.file_name().is_some_and(|n| n == "run.yaml")
                    && m["id"] == rid.as_str()
            });
            if !manifest {
                self.diagnostics.push(diagnostic(
                    "source_unavailable",
                    "run-owned record lacks manifest",
                    Some(path),
                    Some(sid),
                ));
            }
            if let Some(wisp) = v["wisp_id"].as_str()
                && records.iter().any(|(p, m, _, r)| {
                    r == rid
                        && p.file_name().is_some_and(|n| n == "run.yaml")
                        && m["phase"] == "active"
                })
                && self.effective_run(wisp).0.as_deref() != Some(rid)
            {
                self.diagnostics.push(diagnostic(
                    "wisp_run_mismatch",
                    "active wisp ancestry disagrees with owning run",
                    Some(path),
                    Some(sid),
                ));
            }
            if let Some(name) = v["name"].as_str()
                && !names.insert((rid.clone(), name.to_owned()))
            {
                self.diagnostics.push(diagnostic(
                    "invalid_source",
                    "duplicate immutable name in run",
                    Some(path),
                    Some(sid),
                ));
            }
            if let Some(claim) = v["claim_id"].as_str()
                && !records
                    .iter()
                    .any(|(_, c, _, r)| r == rid && c["id"] == claim && c.get("item_id").is_some())
            {
                self.diagnostics.push(diagnostic(
                    "claim_conflict",
                    "ending lacks matching acquisition",
                    Some(path),
                    Some(sid),
                ));
            }
            let current = v.get("item_id").is_some()
                && !records
                    .iter()
                    .any(|(p, e, _, _)| p.parent() == path.parent() && e["claim_id"] == v["id"]);
            if current {
                let target = v["item_id"].as_str().unwrap();
                let run = records.iter().find(|(p, m, s, r)| {
                    s == sid
                        && r == rid
                        && p.file_name().is_some_and(|n| n == "run.yaml")
                        && m["id"] == rid.as_str()
                });
                if !run.is_some_and(|(_, m, _, _)| m["phase"] == "active")
                    || v["item_id"]
                        .as_str()
                        .is_none_or(|id| self.store.resolve(id).is_err())
                {
                    self.diagnostics.push(diagnostic(
                        "claim_conflict",
                        "current acquisition lacks active run/target",
                        Some(path),
                        Some(sid),
                    ));
                }
                let target_file = self.store.resolve(target).ok();
                let target_wisp = target_file.is_some_and(|f| self.is_wisp(f));
                let wisp_owner = target_file
                    .filter(|_| target_wisp)
                    .and_then(|f| f.path.parent()?.parent()?.file_name()?.to_str());
                if (target_wisp && wisp_owner != Some(rid.as_str()))
                    || (!target_wisp && self.effective_run(target).0.as_deref() != Some(rid))
                {
                    self.diagnostics.push(diagnostic(
                        "claim_conflict",
                        "current acquisition disagrees with effective run",
                        Some(path),
                        Some(sid),
                    ));
                }
            }
            if let Some(address) = v.get("named_session") {
                let named = records.iter().find(|(_, s, _, r)| {
                    s["id"] == address["session_record_id"]
                        && r == address["run_id"].as_str().unwrap()
                        && s.get("name").is_some()
                });
                let valid = named.is_some_and(|(session_path, s, _, _)| {
                    let owner_manifest = session_path
                        .parent()
                        .and_then(|p| p.parent())
                        .map(|p| p.join("run.yaml"));
                    s["actor"] == v["actor"]
                        && s["identity"] == v["identity"]
                        && (!current
                            || (s["state"] != "retired"
                                && records.iter().any(|(manifest, m, _, _)| {
                                    Some(manifest) == owner_manifest.as_ref()
                                        && m["phase"] == "active"
                                })))
                });
                if (current || named.is_some()) && !valid {
                    self.diagnostics.push(diagnostic(
                        "claim_conflict",
                        "claim named identity snapshot is missing/mismatched",
                        Some(path),
                        Some(sid),
                    ));
                }
            }
        }
        for (item, ids) in claims {
            if ids.len() > 1 {
                let mut d = diagnostic(
                    "claim_conflict",
                    "multiple unended acquisitions",
                    None,
                    None,
                );
                d["item_id"] = json!(item);

                self.diagnostics.push(d);
            }
        }
    }
    pub(crate) fn records(&self) -> Vec<(PathBuf, Value, String, String)> {
        let mut out = Vec::new();
        if self.physical_mode {
            let p = self
                .physical
                .iter()
                .find(|p| p.checkout.root == self.project.worktree_root)
                .unwrap();
            let base = p.checkout.root.join(".work/scopes");
            for (path, v) in &p.records {
                if let Ok(relative) = path.strip_prefix(&base) {
                    let parts = relative
                        .iter()
                        .filter_map(|s| s.to_str())
                        .collect::<Vec<_>>();
                    if parts.len() >= 4 && parts[1] == "runs" {
                        out.push((path.clone(), v.clone(), parts[0].into(), parts[2].into()));
                    }
                }
            }
            return out;
        }
        for sid in self.hosts.keys() {
            let Some(host) = self.hosts.get(sid).and_then(|h| h.as_ref()) else {
                continue;
            };
            let Some(p) = self.physical.iter().find(|p| &p.checkout.root == host) else {
                continue;
            };
            let base = host.join(format!(".work/scopes/{sid}/runs"));
            for (path, v) in &p.records {
                if let Ok(rel) = path.strip_prefix(&base) {
                    let run = rel.iter().next().unwrap().to_str().unwrap().to_owned();
                    out.push((path.clone(), v.clone(), sid.clone(), run));
                }
            }
        }
        out
    }
    pub fn execution(&self) -> Value {
        let lock = self
            .project
            .git_common_dir
            .join("work-beta2/coordination.lock");
        let available = Directory::open(lock.parent().unwrap())
            .ok()
            .and_then(|d| d.read("coordination.lock").ok())
            .is_some_and(|s| s.raw.is_empty())
            && !self
                .diagnostics
                .iter()
                .any(|d| d["code"] == "coordination_unavailable");
        let root_run = self
            .anchors
            .get("root")
            .and_then(|a| a["execution"]["run_id"].as_str());
        let root_active = root_run.is_some_and(|id| {
            self.records().iter().any(|(p, m, s, r)| {
                s == "root"
                    && r == id
                    && p.file_name().is_some_and(|n| n == "run.yaml")
                    && m["phase"] == "active"
            })
        });
        let state = if self.physical_mode {
            "unavailable"
        } else if !self.pending.is_empty() {
            "pending"
        } else if !self.diagnostics.is_empty()
            || (root_run.is_some() && (!available || !root_active))
        {
            "unavailable"
        } else if root_run.is_some() {
            "ready"
        } else {
            "uninitialized"
        };
        json!({"state":state,"coordination_available":available&&!self.physical_mode,"root_run_id":root_run,"pending":self.pending,"diagnostics":self.diagnostics})
    }
    pub fn wrap(&self, mut value: Value) -> Value {
        value["view"] = json!(if self.physical_mode {
            "physical"
        } else {
            "resolved"
        });
        value["authoritative"] = json!(!self.physical_mode);
        value["execution"] = self.execution();
        value["diagnostics"] = json!(self.diagnostics);
        value
    }
    pub fn require_healthy(&self) -> Result<()> {
        if !self.pending.is_empty() {
            return Err(Error::new(
                "transition_pending",
                "bounded transition pending",
            ));
        }
        if let Some(d) = self
            .diagnostics
            .iter()
            .find(|d| d["code"] == "coordination_unavailable")
            .or_else(|| self.diagnostics.first())
        {
            let code = match d["code"].as_str().unwrap_or("") {
                "host_ambiguous" => "host_ambiguous",
                "host_missing" => "host_missing",
                "source_unavailable" => "source_unavailable",
                "obsolete_execution" => "obsolete_execution",
                "coordination_unavailable" => "coordination_unavailable",
                "conflict" => "conflict",
                _ => "invalid_source",
            };
            let mut e = Error::new(
                code,
                "authoritative hosting/source diagnostics must be resolved",
            );
            e.details = json!({"diagnostics":self.diagnostics});
            return Err(e);
        }
        let g = ItemGraph::from_store(&self.store);
        if !g.diagnostics().is_empty() {
            let mut e = Error::new("invalid_source", "invalid authoritative graph");
            e.details = json!({"diagnostics":g.diagnostics().iter().map(|d|diagnostic("invalid_source",&d.message,Some(&d.path),None)).collect::<Vec<_>>()});
            return Err(e);
        }
        Ok(())
    }
    pub fn inventory(&self, sid: &str, host: &Path) -> Result<Value> {
        let p = self
            .physical
            .iter()
            .find(|p| p.checkout.root == host)
            .ok_or_else(|| Error::new("source_unavailable", "host not in inventory"))?;
        if !p.available {
            return Err(
                Error::new("source_unavailable", "candidate source unavailable/invalid").at(host),
            );
        }
        let candidate_wisps = p
            .records
            .iter()
            .filter(|(_, v)| v.get("wisp_id").is_some())
            .filter_map(|(p, _)| {
                self.capture
                    .files
                    .get(p)
                    .map(|s| super::items::parse_candidate(p.clone(), s.raw.clone()))
            })
            .collect::<Vec<_>>();
        let mut scopes = BTreeSet::from([sid.to_owned()]);
        for child in p
            .anchors
            .keys()
            .filter(|c| c.as_str() != "root" && c.as_str() != sid)
        {
            let mut current = Some(child.clone());
            let mut seen = BTreeSet::new();
            while let Some(id) = current {
                if !seen.insert(id.clone()) {
                    break;
                }
                if id == sid {
                    scopes.insert(child.clone());
                    break;
                }
                if !self.declared(&id).is_empty() {
                    break;
                }
                current = p
                    .material
                    .files
                    .iter()
                    .chain(candidate_wisps.iter())
                    .chain(self.store.files.iter().filter(|f| self.is_wisp(f)))
                    .find_map(|f| {
                        f.header
                            .as_ref()
                            .filter(|h| h.id == id)
                            .and_then(|h| h.parent.clone())
                    });
            }
        }
        let mut todo = scopes.iter().cloned().collect::<Vec<_>>();
        while let Some(parent) = todo.pop() {
            for child in p.children.get(&parent).into_iter().flatten() {
                if !self.declared(child).is_empty() {
                    continue;
                }
                if scopes.insert(child.clone()) {
                    todo.push(child.clone());
                }
            }
        }
        let mut files = BTreeMap::<String, String>::new();
        for f in &p.material.files {
            let Some(h) = &f.header else {
                let affected = &scopes;
                let known = f
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|id| self.store.resolve(id).ok())
                    .and_then(|f| f.header.as_ref())
                    .map(|h| self.nearest(Some(&h.id)));
                if known.is_some_and(|scope| !affected.contains(&scope)) {
                    continue;
                }
                return Err(
                    Error::new("invalid_source", "invalid candidate material source").at(&f.path),
                );
            };
            let mut current = Some(h.id.clone());
            let mut seen = BTreeSet::new();
            let mut included = sid == "root";
            while let Some(id) = current {
                if !seen.insert(id.clone()) {
                    return Err(
                        Error::new("invalid_source", "candidate ancestry cycle").at(&f.path)
                    );
                }
                if id == sid {
                    included = true;
                    break;
                }
                if id != sid && !self.declared(&id).is_empty() {
                    included = false;
                    break;
                }
                current = p
                    .material
                    .files
                    .iter()
                    .chain(candidate_wisps.iter())
                    .chain(self.store.files.iter().filter(|f| self.is_wisp(f)))
                    .find_map(|f| {
                        f.header
                            .as_ref()
                            .filter(|h| h.id == id)
                            .and_then(|h| h.parent.clone())
                    });
            }
            if included {
                let rel = f
                    .path
                    .strip_prefix(host)
                    .unwrap()
                    .to_str()
                    .ok_or_else(|| Error::new("unsafe_path", "non-UTF-8 entity path"))?;
                files.insert(rel.into(), hash(&f.raw));
            }
        }
        if sid != "root" && !files.contains_key(&format!(".work/items/{sid}.md")) {
            return Err(Error::new(
                "source_unavailable",
                "candidate material boundary root missing",
            )
            .at(host));
        }
        for (path, s) in &self.capture.files {
            if let Ok(rel) = path.strip_prefix(host) {
                let comps: Vec<_> = rel.iter().map(|x| x.to_str().unwrap_or("")).collect();
                let scope_file = comps.len() >= 3
                    && comps[..2] == [".work", "scopes"]
                    && scopes.contains(comps[2]);
                let digest_file = comps.len() == 4
                    && comps[..2] == [".work", "digests"]
                    && (files.contains_key(&format!(".work/items/{}.md", comps[2]))
                        || (sid == "root" && comps[2] == "root"));
                if scope_file || digest_file {
                    files.insert(rel.to_str().unwrap().into(), hash(&s.raw));
                }
            }
        }
        for d in &p.diagnostics {
            let relevant = d["path"]
                .as_str()
                .and_then(|s| decode_path(s).ok())
                .and_then(|file| file.strip_prefix(host).ok().map(Path::to_owned))
                .is_none_or(|rel| {
                    let components = rel
                        .iter()
                        .map(|s| s.to_str().unwrap_or(""))
                        .collect::<Vec<_>>();
                    match components.as_slice() {
                        [".work", "scopes", scope, ..] => scopes.contains(*scope),
                        [".work", "digests", root, ..] => {
                            files.contains_key(&format!(".work/items/{root}.md"))
                                || (sid == "root" && *root == "root")
                        }
                        [".work", "items", name] => {
                            files.contains_key(&format!(".work/items/{name}"))
                        }
                        _ => true,
                    }
                });
            if relevant {
                return Err(
                    Error::new("source_unavailable", "candidate scope content invalid").at(host),
                );
            }
        }
        let file_marks = files
            .into_iter()
            .map(|(path, sha256)| json!({"path":path,"sha256":sha256}))
            .collect::<Vec<_>>();
        let scope_ids = scopes.into_iter().collect::<Vec<_>>();
        let mut inventory = json!({"scope_ids":scope_ids,"files":file_marks});
        inventory["sha256"] = json!(wire::inventory_hash(&inventory));
        Ok(inventory)
    }
    pub fn scope(&self, sid: &str) -> Value {
        let stored = self.anchors.contains_key(sid);
        let parent = if sid == "root" {
            None
        } else if stored {
            self.parents.get(sid).cloned()
        } else {
            self.store.files.iter().find_map(|f| {
                f.header
                    .as_ref()
                    .filter(|h| h.id == sid)
                    .map(|h| self.nearest(h.parent.as_deref()))
            })
        };
        let host = if self.physical_mode && sid == "root" {
            Some(self.physical[0].checkout.root.clone())
        } else {
            self.source_host(sid)
        };
        let mut candidates = Vec::new();
        for p in &self.physical {
            if self.physical_mode && p.checkout.root != self.project.worktree_root {
                continue;
            }
            let declared = self
                .declared(sid)
                .iter()
                .any(|x| x.checkout.root == p.checkout.root);
            if !declared
                && sid != "root"
                && !p.material.files.iter().any(|f| {
                    f.header.as_ref().is_some_and(|h| h.id == sid)
                        || f.path.file_stem().and_then(|s| s.to_str()) == Some(sid)
                })
                && !p.anchors.contains_key(sid)
            {
                continue;
            }
            let inv = self.inventory(sid, &p.checkout.root);
            candidates.push(json!({"checkout":path(&p.checkout.root),"scope_id":sid,"declared":declared,"available":p.available,"inventory_sha256":inv.as_ref().ok().map(|i|i["sha256"].clone()),"declaration_sha256":p.checkout.git_dir.as_ref().and_then(|g|self.capture.files.get(&g.join("work-beta2-hosts.yaml"))).map(|s|hash(&s.raw)),"diagnostics":p.diagnostics}));
        }
        let inventory = if self.physical_mode {
            self.inventory(sid, &self.project.worktree_root).ok()
        } else {
            host.as_ref().and_then(|h| self.inventory(sid, h).ok())
        };
        json!({"scope_id":sid,"stored":stored,"parent_scope_id":parent,"children":self.parents.iter().filter(|(_,p)|p.as_str()==sid).map(|(s,_)|s.clone()).collect::<Vec<_>>(),"host":host.as_deref().map(path),"hosting_override":!self.declared(sid).is_empty(),"execution":self.anchors.get(sid).map(|a|a["execution"].clone()),"effective_run_id":self.effective_run(sid).0,"candidates":candidates,"inventory_sha256":inventory.map(|i|i["sha256"].clone()),"diagnostics":self.diagnostics})
    }
    pub fn effective_run(&self, id: &str) -> (Option<String>, &'static str) {
        let mut sid = self.nearest(Some(id));
        let mut seen = BTreeSet::new();
        while seen.insert(sid.clone()) {
            if let Some(a) = self.anchors.get(&sid) {
                let e = &a["execution"];
                if e["mode"] == "ended" {
                    return (None, "ended");
                }
                if e["mode"] == "boundary" {
                    let rid = e["run_id"].as_str().unwrap().to_owned();
                    let host = if self.physical_mode {
                        Some(&self.project.worktree_root)
                    } else {
                        self.hosts.get(&sid).and_then(|h| h.as_ref())
                    };
                    let manifest = host.and_then(|h| {
                        self.physical
                            .iter()
                            .find(|p| &p.checkout.root == h)
                            .and_then(|p| {
                                p.records
                                    .get(&h.join(format!(".work/scopes/{sid}/runs/{rid}/run.yaml")))
                            })
                    });
                    return (
                        Some(rid),
                        if manifest.is_some_and(|m| m["phase"] == "active") {
                            "active"
                        } else if sid == "root" || manifest.is_none() {
                            "unavailable"
                        } else {
                            "ended"
                        },
                    );
                }
            }
            if sid == "root" {
                break;
            }
            sid = self
                .parents
                .get(&sid)
                .cloned()
                .unwrap_or_else(|| "root".into());
        }
        (None, "uninitialized")
    }
}
pub(crate) fn read_view(project: &Project, physical: bool) -> Result<(View, Option<Locked>)> {
    if physical {
        return Ok((View::load(project, true)?, None));
    }
    let initial = View::load(project, false)?;
    let mut guard = match coordination(&initial, false, false) {
        Ok(g) => g,
        Err(e)
            if matches!(
                e.code,
                "coordination_unavailable" | "unsafe_path" | "permission_denied" | "io"
            ) =>
        {
            None
        }
        Err(e) => return Err(e),
    };
    #[cfg(test)]
    super::storage::files::inject("scope_read_after_coordination", &project.git_common_dir)?;
    let mut v = View::load(project, false)?;
    if guard.is_none() {
        match coordination(&v, false, false) {
            Ok(Some(g)) => {
                guard = Some(g);
                v = View::load(project, false)?;
            }
            Ok(None) => {}
            Err(e)
                if matches!(
                    e.code,
                    "coordination_unavailable" | "unsafe_path" | "permission_denied" | "io"
                ) =>
            {
                let fallback = project.git_common_dir.join("work-beta2/coordination.lock");
                let d = diagnostic(
                    "coordination_unavailable",
                    e.message,
                    Some(e.path.as_deref().unwrap_or(&fallback)),
                    None,
                );
                if !v.diagnostics.contains(&d) {
                    v.diagnostics.push(d);
                }
            }
            Err(e) => return Err(e),
        }
    }
    if let Some(g) = &guard {
        g.verify()?;
    }
    v.capture.recheck()?;
    Ok((v, guard))
}
pub(crate) fn coordination(v: &View, exclusive: bool, create: bool) -> Result<Option<Locked>> {
    let common = Directory::open(&v.project.git_common_dir)?;
    let existing =
        common.exists("work-beta2")? && common.child("work-beta2")?.exists("coordination.lock")?;
    if !existing {
        let persisted = v.physical.iter().any(|p| {
            let relevant = |sid: &str| {
                v.source_host(sid).as_ref() == Some(&p.checkout.root)
                    || v.hosts.get(sid) == Some(&None)
            };
            p.anchors.iter().any(|(sid, a)| {
                relevant(sid)
                    && matches!(a["execution"]["mode"].as_str(), Some("boundary" | "ended"))
            }) || p.diagnostics.iter().any(|d| {
                d["path"]
                    .as_str()
                    .and_then(|s| decode_path(s).ok())
                    .is_some_and(|file| {
                        file.strip_prefix(p.checkout.root.join(".work/scopes"))
                            .is_ok_and(|relative| {
                                let parts: Vec<_> = relative.iter().collect();
                                parts.first().and_then(|s| s.to_str()).is_some_and(relevant)
                                    && parts
                                        .get(1)
                                        .is_some_and(|s| *s == "runs" || *s == "progress")
                            })
                    })
            }) || p.records.keys().any(|file| {
                let relative = file.strip_prefix(p.checkout.root.join(".work/scopes"));
                relative.is_ok_and(|r| {
                    let parts = r.iter().collect::<Vec<_>>();
                    parts.first().and_then(|s| s.to_str()).is_some_and(relevant)
                        && parts
                            .get(1)
                            .is_some_and(|c| *c == "runs" || *c == "progress")
                })
            })
        }) || !v.pending.is_empty();
        if persisted {
            return Err(Error::new(
                "coordination_unavailable",
                "persisted execution/progress has lost coordination",
            ));
        }
        if !create {
            return Ok(None);
        }
        v.capture.recheck()?;
        common.ensure("work-beta2")?;
    }
    let lock = Locked::open_at(common, "work-beta2", exclusive, create)?;
    if !lock.root.read("coordination.lock")?.raw.is_empty() {
        return Err(Error::new(
            "coordination_unavailable",
            "coordination target must be empty",
        ));
    }
    Ok(Some(lock))
}

pub struct ScopeOperations {
    pub project: Project,
    pub physical: bool,
}
impl ScopeOperations {
    pub fn new(project: Project, physical: bool) -> Self {
        Self { project, physical }
    }
    pub fn read(&self, scope: Option<&str>) -> Result<Value> {
        let (v, guard) = read_view(&self.project, self.physical)?;
        if let Some(g) = &guard {
            g.verify()?;
        }
        let value = if let Some(scope) = scope {
            let sid = v.scope_id(scope)?;
            json!({"scope":v.scope(&sid)})
        } else {
            let ids: BTreeSet<_> = v.hosts.keys().chain(v.anchors.keys()).cloned().collect();
            json!({"scopes":ids.iter().map(|id|v.scope(id)).collect::<Vec<_>>()})
        };
        v.capture.recheck()?;
        Ok(v.wrap(value))
    }
    pub fn host(&self, mode: &str, input: &Value) -> Result<Value> {
        super::scope_transfer::transfer(self, input, mode)
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    struct Fixture(PathBuf, Project);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("work-scope-capture-{}", new_id().unwrap()));
            std::fs::create_dir_all(root.join(".work/items")).unwrap();
            for args in [
                vec!["init", "--initial-branch=main"],
                vec![
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "seed",
                ],
            ] {
                let r = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
            }
            let p = super::super::project::discover(Some(&root)).unwrap();
            Self(root, p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn bootstrap_reads_acquire_new_shared_coordination_before_consuming_view() {
        let f = Fixture::new();
        for operation in ["item_list", "scope_list", "scope_inspect"] {
            let common = f.1.git_common_dir.clone();
            let held = std::rc::Rc::new(std::cell::RefCell::new(None));
            let keep = held.clone();
            let hook =
                super::super::storage::files::on_next("scope_read_after_coordination", move || {
                    let directory = Directory::open(&common).unwrap();
                    directory.ensure("work-beta2").unwrap();
                    *keep.borrow_mut() =
                        Some(Locked::open_at(directory, "work-beta2", true, true).unwrap());
                });
            let result = match operation {
                "item_list" => super::super::scope_items::call(&f.1, operation, &json!({})),
                "scope_list" => ScopeOperations::new(f.1.clone(), false).read(None),
                _ => ScopeOperations::new(f.1.clone(), false).read(Some("root")),
            };
            assert_eq!(result.unwrap_err().code, "storage_busy");
            drop(hook);
            held.borrow_mut().take();
            std::fs::remove_dir_all(f.1.git_common_dir.join("work-beta2")).unwrap();
        }
    }
    #[test]
    fn recapture_cannot_absorb_a_new_unparsed_file() {
        let f = Fixture::new();
        let first = f.0.join(".work/items/aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa.md");
        let new = f.0.join(".work/items/bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb.md");
        std::fs::write(&first, b"first").unwrap();
        let mut cap = Capture::default();
        let dir = Directory::open(&f.0.join(".work/items")).unwrap();
        cap.directory(&dir).unwrap();
        std::fs::write(&new, b"new").unwrap();
        assert_eq!(cap.file(&first).unwrap_err().code, "conflict");
        assert_eq!(cap.recheck().unwrap_err().code, "conflict");
    }
    #[test]
    fn catalog_is_captured_before_material_enumeration() {
        let f = Fixture::new();
        let root = f.0.clone();
        let hook = super::super::storage::files::on_next("scope_before_material_load", move || {
            std::fs::write(
                root.join(".work/items/aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa.md"),
                b"new raw candidate",
            )
            .unwrap();
        });
        let view = View::load(&f.1, false).unwrap();
        assert_eq!(view.capture.recheck().unwrap_err().code, "conflict");
        drop(hook);
    }
    #[test]
    fn capture_reports_completed_unlink_before_post_write_failure() {
        let f = Fixture::new();
        let p = f.0.join(".work/items/aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa.md");
        std::fs::write(&p, b"old").unwrap();
        let mut capture = View::load(&f.1, false).unwrap().capture;

        let fault = super::super::storage::files::fail_next("scope_after_unlink");
        let e = capture.delete(&p).unwrap_err();
        drop(fault);
        assert!(!p.exists());
        assert_eq!(e.details["publication"], "published");
        assert_eq!(e.path.as_ref(), Some(&p));
        assert_eq!(e.details["partial"]["deleted"][0]["path"], path(&p));
    }
    #[test]
    fn capture_retains_published_recovery_when_unrelated_source_changes() {
        let f = Fixture::new();
        let target = f.0.join(".work/items/aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa.md");
        let other = f.0.join(".work/items/bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb.md");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&other, b"other").unwrap();
        let mut capture = View::load(&f.1, false).unwrap().capture;
        let hook = super::super::storage::files::on_next("after_directory_sync", move || {
            std::fs::write(other, b"changed").unwrap();
        });
        let e = capture.publish(&target, b"new").unwrap_err();
        drop(hook);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(e.details["publication"], "published");
        assert_eq!(e.path.as_ref(), Some(&target));
        assert_eq!(e.details["partial"]["updated"][0]["path"], path(&target));
        let retained = decode_path(e.details["previous_source_path"].as_str().unwrap()).unwrap();
        assert_eq!(std::fs::read(retained).unwrap(), b"old");
    }
}

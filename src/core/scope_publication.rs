//! Short guarded publication for the scoped run/template writers. No journal.
use super::coordination::{ExecutionError as Error, ExecutionResult as Result};
use super::scope_wire::hash;
use super::scopes::{Capture, partial, path};
use super::storage::files::Directory;
use super::storage::{StorageError, StorageErrorCode};
use serde_json::{Value, json};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub(crate) fn snapshot(c: &Capture, p: &Path) -> Value {
    c.files.get(p).map_or_else(
        || json!({"exists":false}),
        |s| json!({"exists":true,"sha256":hash(&s.raw)}),
    )
}
pub(crate) fn preflight(root: &Path, target: &Path) -> Result<()> {
    let mut d = Directory::open(root)?;
    for part in target
        .parent()
        .unwrap()
        .strip_prefix(root)
        .map_err(|_| Error::new("unsafe_path", "destination outside checkout"))?
    {
        if !d.exists(part)? {
            break;
        }
        d = d.child(part)?;
    }
    if d.file.metadata()?.mode() & 0o200 == 0 {
        return Err(
            Error::new("io", "publication directory lacks owner-write permission").at(&d.path),
        );
    }
    Ok(())
}
pub(crate) fn known(effects: &mut Value, key: &str, p: &Path, id: Option<&str>) {
    let entry = json!({"id":id,"path":path(p)});
    if !effects[key].as_array().unwrap().contains(&entry) {
        effects[key].as_array_mut().unwrap().push(entry);
    }
}
pub(crate) fn ensure(c: &mut Capture, p: &Path, effects: &mut Value) -> Result<()> {
    let mut missing = Vec::new();
    let mut cursor = p;
    while std::fs::symlink_metadata(cursor).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        missing.push(cursor.to_owned());
        cursor = cursor.parent().unwrap();
    }
    let result = c.ensure(p);
    for d in missing.iter().rev() {
        if Directory::open(d).is_ok() {
            known(effects, "created", d, None);
        }
    }
    result.map(|_| ())
}
pub(crate) fn attempted(mut e: Error, effects: &Value, last: Option<&Path>) -> Error {
    if e.details.get("publication").is_none()
        && let Some(p) = last
    {
        let entry = ["created", "updated", "deleted"].iter().find_map(|key| {
            effects[key]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["path"] == path(p))
        });
        e.details["publication"] = json!(if entry.is_some() {
            "published"
        } else {
            "not_published"
        });
        if let Some(entry) = entry {
            e.details["published_item"] = entry.clone();
        }
        if e.path.is_none() {
            e.path = Some(p.to_owned());
        }
    }
    merge(e, effects)
}
pub(crate) fn merge(mut e: Error, effects: &Value) -> Error {
    let mut all = effects.clone();
    if let Some(previous) = e.details.get("partial") {
        for key in partial().as_object().unwrap().keys() {
            if let Some(entries) = previous[key].as_array() {
                for entry in entries {
                    if !all[key].as_array().unwrap().contains(entry) {
                        all[key].as_array_mut().unwrap().push(entry.clone());
                    }
                }
            }
        }
    }
    if e.details["publication"] == "possible"
        && let Some(p) = &e.path
    {
        let p = json!(path(p));
        if !all["uncertain_paths"].as_array().unwrap().contains(&p) {
            all["uncertain_paths"].as_array_mut().unwrap().push(p);
        }
    }
    e.details["partial"] = all;
    e
}
/// Only frozen operation callers may reconcile their exact planned staging bytes.
/// Ordinary/template publication never gains stage-based replay or deduplication.
pub(crate) fn save_resuming(
    c: &mut Capture,
    p: &Path,
    raw: &[u8],
    stage: &str,
    effects: &mut Value,
    id: Option<&str>,
    fence: &impl Fn() -> Result<()>,
) -> Result<()> {
    if c.files.get(p).is_some_and(|s| s.raw == raw) {
        return Ok(());
    }
    ensure(c, p.parent().unwrap(), effects)?;
    c.recheck()?;
    fence()?;
    let directory = Directory::open(p.parent().unwrap())?;
    if directory.optional(stage)?.is_some() {
        cleanup(
            c,
            &directory.path.join(stage),
            &json!({"exists":true,"sha256":hash(raw)}),
            fence,
        )?;
    }
    save(c, p, raw, stage, effects, id, fence)
}
/// Stages are retained until the caller has verified its relevant graph/context.
#[allow(clippy::result_large_err)] // Shared descriptor publication preserves structured storage evidence.
pub(crate) fn save(
    c: &mut Capture,
    p: &Path,
    raw: &[u8],
    stage: &str,
    effects: &mut Value,
    id: Option<&str>,
    fence: &impl Fn() -> Result<()>,
) -> Result<()> {
    if c.files.get(p).is_some_and(|s| s.raw == raw) {
        return Ok(());
    }
    ensure(c, p.parent().unwrap(), effects)?;
    c.recheck()?;
    fence()?;
    let d = Directory::open(p.parent().unwrap())?;
    let before = c.files.get(p).cloned();
    let key = if before.is_some() {
        "updated"
    } else {
        "created"
    };
    let stage_path = d.path.join(stage);
    let result = d.publish_guarded(
        p.file_name().unwrap().to_str().unwrap(),
        raw,
        before.as_ref(),
        stage,
        || {
            c.recheck()
                .and_then(|_| fence())
                .map_err(|e| StorageError::new(StorageErrorCode::Conflict, e.to_string(), e.path))
        },
    );
    if let Err(e) = result {
        let mut e: Error = e.into();
        if std::fs::symlink_metadata(&stage_path).is_ok() {
            e.details["recovery_paths"] = json!([path(&stage_path)]);
        }
        if e.details["publication"] == "possible" {
            e.path = Some(p.to_owned());
        }
        return Err(merge(e, effects));
    }
    known(effects, key, p, id);
    c.advance(p, Some(raw))
        .and_then(|_| fence())
        .map_err(|mut e| {
            e.details["publication"] = json!("published");
            e.path = Some(p.to_owned());
            if before.is_some() {
                e.details["recovery_paths"] = json!([path(&stage_path)]);
            }
            merge(e, effects)
        })
}
pub(crate) fn cleanup(
    c: &Capture,
    p: &Path,
    expected: &Value,
    fence: &impl Fn() -> Result<()>,
) -> Result<()> {
    let result = (|| -> Result<()> {
        let d = Directory::open(p.parent().unwrap())?;
        let name = p.file_name().unwrap().to_str().unwrap();
        let Some(s) = d.optional(name)? else {
            return Ok(());
        };
        if expected["exists"] != true || expected["sha256"] != hash(&s.raw) {
            return Err(Error::new(
                "conflict",
                "artifact differs from the frozen expected bytes",
            )
            .at(p));
        }
        c.recheck()?;
        fence()?;
        d.verify()?;
        if d.read(name)? != s {
            return Err(Error::new("conflict", "artifact changed before cleanup").at(p));
        }
        rustix::fs::unlinkat(&d.file, name, rustix::fs::AtFlags::empty())
            .map_err(|e| Error::new("io", e.to_string()).at(p))?;
        d.sync()?;
        d.verify()?;
        fence()?;
        c.recheck()
    })();
    result.map_err(|mut e| {
        if std::fs::symlink_metadata(p).is_ok() {
            e.details["recovery_paths"] = json!([path(p)]);
            e.details["remaining_paths"] = json!([path(p)]);
        }
        e
    })
}
pub(crate) fn sync(c: &Capture, p: &Path) -> Result<()> {
    let d = Directory::open(p.parent().unwrap())?;
    d.sync_source(
        p.file_name().unwrap().to_str().unwrap(),
        c.files
            .get(p)
            .ok_or_else(|| Error::new("conflict", "protected file missing").at(p))?,
    )?;
    d.sync()?;
    d.verify()?;
    c.recheck()
}

//! Root-owned extension documents, never Work items.
//! @mara implements DES-FINALIZATION-API
use super::coordination::*;
use super::storage::files::Directory;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) fn path(root: &Path, item: &str, run: &str) -> PathBuf {
    root.join(".work/digests")
        .join(item)
        .join(format!("{run}.md"))
}
fn decode(raw: &[u8], item: &str, run: &str) -> ExecutionResult<String> {
    let text = std::str::from_utf8(raw)
        .map_err(|_| ExecutionError::new("invalid_format", "digest must be UTF-8"))?;
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(|l| l.trim_end_matches(['\r', '\n'])) != Some("---") {
        return Err(ExecutionError::new(
            "invalid_format",
            "missing digest frontmatter",
        ));
    }
    let start = text
        .find('\n')
        .ok_or_else(|| ExecutionError::new("invalid_format", "missing closing digest delimiter"))?
        + 1;
    let mut end = start;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let header = parse_yaml(&raw[start..end])?;
            if header["format_version"].as_u64() != Some(1) {
                return Err(ExecutionError::new(
                    "unsupported_format",
                    "unsupported digest format",
                ));
            }
            exact_keys(&header, &["format_version", "root_item_id", "run_id"], &[])?;
            if header["root_item_id"] != item || header["run_id"] != run {
                return Err(ExecutionError::new(
                    "identity_mismatch",
                    "digest root/run mismatch",
                ));
            }
            return Ok(text[end + line.len()..].into());
        }
        end += line.len();
    }
    Err(ExecutionError::new(
        "invalid_format",
        "missing closing digest delimiter",
    ))
}
fn directory(root: &Path, item: &str, create: bool) -> ExecutionResult<Option<Directory>> {
    let work = Directory::open(root)?.child(".work")?;
    let digests = if create {
        work.ensure("digests")?
    } else {
        if !work.exists("digests")? {
            return Ok(None);
        }
        work.child("digests")?
    };
    if create {
        Ok(Some(digests.ensure(item)?))
    } else {
        if !digests.exists(item)? {
            return Ok(None);
        }
        Ok(Some(digests.child(item)?))
    }
}
pub(crate) fn list(root: &Path, item: &str) -> ExecutionResult<Vec<Value>> {
    let Some(dir) = directory(root, item, false)? else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for name in dir.names()? {
        let name = name
            .to_str()
            .ok_or_else(|| ExecutionError::new("invalid_format", "non-UTF-8 digest entry"))?;
        if name.starts_with('.') {
            continue;
        }
        let run = name
            .strip_suffix(".md")
            .filter(|s| valid_id(s))
            .ok_or_else(|| {
                ExecutionError::new("invalid_format", "unexpected digest entry")
                    .at(dir.path.join(name))
            })?;
        let source = dir.read(name)?;
        let body = decode(&source.raw, item, run).map_err(|e| e.at(dir.path.join(name)))?;
        result.push(json!({"run_id":run,"path":encode_path(&dir.path.join(name)),"body":body}));
    }
    result.sort_by_key(|v| v["run_id"].as_str().unwrap().to_owned());
    Ok(result)
}
pub(crate) fn retain(
    root: &Path,
    item: &str,
    run: &str,
    body: &str,
    terminal: bool,
) -> ExecutionResult<bool> {
    let dir = directory(root, item, !terminal)?
        .ok_or_else(|| ExecutionError::new("source_unavailable", "retained digest is missing"))?;
    let name = format!("{run}.md");
    if let Some(source) = dir.optional(&name)? {
        if decode(&source.raw, item, run)? != body {
            return Err(ExecutionError::new(
                "run_conflict",
                "digest summary differs from retained body",
            )
            .at(dir.path.join(name)));
        }
        // A prior interrupted publication may have installed the file before sync.
        dir.sync_source(&name, &source)?.sync_all()?;
        dir.sync()?;
        dir.verify()?;
        return Ok(false);
    }
    if terminal {
        return Err(
            ExecutionError::new("source_unavailable", "terminal digest is missing")
                .at(dir.path.join(name)),
        );
    }
    let mut raw = b"---\n".to_vec();
    raw.extend(yaml_bytes(
        &json!({"format_version":1,"root_item_id":item,"run_id":run}),
    ));
    raw.extend(b"---\n");
    raw.extend(body.as_bytes());
    dir.publish(&name, &raw, None, &format!(".stage-{}", new_id()?))?;
    Ok(true)
}

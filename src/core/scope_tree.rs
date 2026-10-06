//! Frozen parent-local reference maintenance for material reparenting.
use super::coordination::{
    ExecutionError as Error, ExecutionResult as Result, new_id, now_timestamp, yaml_bytes,
};
use super::items::{ItemFile, ItemStore};
use super::operations::{CheckoutGuard, OperationError};
use super::scope_wire::{self as wire, hash};
use super::scopes::{View, partial, path};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(crate) fn changes(v: &mut View, candidate: &ItemStore) -> Result<Vec<Value>> {
    let old = v.store.clone();
    v.store = candidate.clone();
    let mut refs = Vec::new();
    for (sid, parent) in &v.parents {
        let root = v
            .store
            .resolve(sid)
            .map_err(|_| Error::new("invalid_candidate", "missing material boundary"))?
            .header
            .as_ref()
            .unwrap();
        let new = v.nearest(root.parent.as_deref());
        if new == *parent {
            continue;
        }
        for (p, create) in [(&new, true), (parent, false)] {
            let host = v
                .hosts
                .get(p)
                .and_then(|h| h.as_ref())
                .ok_or_else(|| Error::new("host_missing", "tree reference host unavailable"))?;
            let file = host.join(format!(".work/scopes/{p}/children/{sid}.yaml"));
            let before = match v.capture.files.get(&file) {
                Some(s) => json!({"exists":true,"sha256":hash(&s.raw)}),
                None => json!({"exists":false}),
            };
            let mut r = json!({"parent_scope_id":p,"child_scope_id":sid,"before":before});
            if create {
                r["after"] = json!({"format_version":2,"child_scope_id":sid});
            }
            refs.push(r);
        }
    }
    v.store = old;
    Ok(refs)
}
pub(crate) fn existing(v: &View, id: &str, continue_id: &str) -> Result<(PathBuf, Value)> {
    for p in &v.pending {
        let file = super::coordination::decode_path(p["path"].as_str().unwrap())?;
        if p["id"] != continue_id
            && !(p["id"].is_null()
                && file.file_stem().and_then(|s| s.to_str()) == Some(continue_id))
        {
            continue;
        }
        let source = v.capture.files.get(&file).ok_or_else(|| {
            Error::new("source_unavailable", "continuation bytes unavailable").at(&file)
        })?;
        let plan = wire::envelope("StartProgress", &source.raw, &file)?;
        if plan["tree_change"]["item_id"] != id {
            return Err(Error::new(
                "conflict",
                "continuation is not this material tree edit",
            ));
        }
        return Ok((file, plan));
    }
    Err(Error::new(
        "not_found",
        "material tree continuation not found",
    ))
}
pub(crate) fn prospective(v: &View, candidate: &ItemStore, refs: &[Value]) -> View {
    let mut shadow = v.clone();
    shadow.store = candidate.clone();
    apply_refs(&mut shadow, refs, false);
    rehost(&mut shadow);
    shadow
}
fn rehost(v: &mut View) {
    let mut hosts = std::collections::BTreeMap::new();
    hosts.insert("root".to_owned(), v.hosts.get("root").cloned().flatten());
    for _ in 0..=v.parents.len() {
        for (sid, parent) in &v.parents {
            if hosts.contains_key(sid) {
                continue;
            }
            let declarations = v.declared(sid);
            let host = match declarations.as_slice() {
                [p] => Some(p.checkout.root.clone()),
                [] => {
                    let Some(host) = hosts.get(parent) else {
                        continue;
                    };
                    host.clone()
                }
                _ => None,
            };
            hosts.insert(sid.clone(), host);
        }
    }
    for sid in v.hosts.keys() {
        hosts.entry(sid.clone()).or_insert(None);
    }
    v.hosts = hosts;
}
fn apply_refs(shadow: &mut View, refs: &[Value], before: bool) {
    for r in refs {
        let child = r["child_scope_id"].as_str().unwrap();
        let parent = r["parent_scope_id"].as_str().unwrap();
        let exists = if before {
            r["before"]["exists"] == true
        } else {
            r.get("after").is_some()
        };
        if exists {
            shadow.parents.insert(child.into(), parent.into());
        } else if shadow.parents.get(child).is_some_and(|p| p == parent) {
            shadow.parents.remove(child);
        }
    }
}
fn projected(v: &View, plan: &Value, before: bool) -> View {
    let mut shadow = v.clone();
    let id = plan["tree_change"]["item_id"].as_str().unwrap();
    let parent = plan["tree_change"][if before {
        "parent_before"
    } else {
        "parent_after"
    }]
    .as_str();
    if let Some(file) = shadow
        .store
        .files
        .iter_mut()
        .find(|f| f.header.as_ref().is_some_and(|h| h.id == id))
    {
        file.header.as_mut().unwrap().parent = parent.map(str::to_owned);
    }
    for r in plan["tree_references"].as_array().unwrap() {
        let child = r["child_scope_id"].as_str().unwrap();
        let parent = r["parent_scope_id"].as_str().unwrap();
        let exists = if before {
            r["before"]["exists"] == true
        } else {
            r.get("after").is_some()
        };
        if exists {
            shadow.parents.insert(child.into(), parent.into());
        } else if shadow.parents.get(child).is_some_and(|p| p == parent) {
            shadow.parents.remove(child);
        }
    }
    rehost(&mut shadow);
    shadow
}
fn descends(v: &View, id: &str, root: &str) -> bool {
    let mut current = Some(id.to_owned());
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if id == root {
            return true;
        }
        if !seen.insert(id.clone()) {
            break;
        }
        current = v.store.files.iter().find_map(|f| {
            f.header
                .as_ref()
                .filter(|h| h.id == id)
                .and_then(|h| h.parent.clone())
        });
    }
    false
}
pub(crate) fn publish(
    v: &View,
    old: &ItemFile,
    raw: &[u8],
    refs: Vec<Value>,
    operation: &str,
    continuation: Option<(PathBuf, Value)>,
) -> Result<Value> {
    let id = old.header.as_ref().unwrap().id.clone();
    let sid = v.nearest(Some(&id));
    let host = old
        .path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let (progress, mut plan) = if let Some(p) = continuation {
        p
    } else {
        let op = new_id()?;
        let anchor = v
            .hosts
            .get(&sid)
            .and_then(|h| h.as_ref())
            .unwrap()
            .join(format!(".work/scopes/{sid}/scope.yaml"));
        let before = match v.capture.files.get(&anchor) {
            Some(s) => json!({"exists":true,"sha256":hash(&s.raw)}),
            None => json!({"exists":false}),
        };
        let parent = v.nearest(old.header.as_ref().unwrap().parent.as_deref());
        let after_item = super::items::parse_candidate(old.path.clone(), raw.to_vec());
        let mut prospective = ItemStore::from_candidate_files(
            v.store
                .files
                .iter()
                .map(|f| {
                    if f.path == old.path {
                        after_item.clone()
                    } else {
                        f.clone()
                    }
                })
                .collect(),
        );
        let next_parent = after_item.header.as_ref().unwrap().parent.as_deref();
        let mut new_parent = next_parent.map(str::to_owned);
        let mut seen = BTreeSet::new();
        while new_parent
            .as_ref()
            .is_some_and(|p| !v.hosts.contains_key(p))
        {
            let p = new_parent.clone().unwrap();
            if !seen.insert(p.clone()) {
                break;
            }
            new_parent = prospective.files.iter().find_map(|f| {
                f.header
                    .as_ref()
                    .filter(|h| h.id == p)
                    .and_then(|h| h.parent.clone())
            });
        }
        prospective.files.clear();
        let plan = json!({"format_version":2,"id":op,"created_at":now_timestamp(),"scope_id":sid,"anchor_before":before,"tree_change":{"item_id":id,"operation":operation,"parent_before":old.header.as_ref().unwrap().parent,"parent_after":after_item.header.as_ref().unwrap().parent,"old_parent_scope_id":parent,"new_parent_scope_id":new_parent.unwrap_or_else(||"root".into()),"item_before":hash(&old.raw),"item_after":hash(raw),"step":0},"tree_references":refs,"step":0});
        wire::validate("StartProgress", &plan)?;
        (
            v.hosts[&sid]
                .as_ref()
                .unwrap()
                .join(format!(".work/scopes/{sid}/progress/start/{op}.yaml")),
            plan,
        )
    };
    let desired = super::items::parse_candidate(old.path.clone(), raw.to_vec())
        .header
        .unwrap()
        .parent;
    if plan["tree_change"]["operation"] != operation
        || plan["tree_change"]["parent_after"] != json!(desired)
        || (plan["tree_change"]["item_before"] == hash(&old.raw)
            && plan["tree_change"]["parent_before"] != json!(old.header.as_ref().unwrap().parent))
    {
        return Err(Error::new(
            "conflict",
            "tree operation/edge differs from frozen source",
        ));
    }
    if plan["tree_change"]["item_after"] != hash(raw) {
        return Err(Error::new(
            "conflict",
            "continued edit differs from frozen parent/header bytes",
        ));
    }
    let scope = plan["scope_id"].as_str().unwrap();
    let owner = v
        .hosts
        .get(scope)
        .and_then(|h| h.as_ref())
        .ok_or_else(|| Error::new("host_missing", "frozen tree owner unavailable"))?;
    let anchor = owner.join(format!(".work/scopes/{scope}/scope.yaml"));
    let current = match v.capture.files.get(&anchor) {
        Some(s) => json!({"exists":true,"sha256":hash(&s.raw)}),
        None => json!({"exists":false}),
    };
    if current != plan["anchor_before"] {
        return Err(Error::new("conflict", "frozen tree owner anchor changed").at(&anchor));
    }
    let mut before = projected(v, &plan, true);
    let after = projected(v, &plan, false);
    let required = changes(&mut before, &after.store)?;
    let frozen = plan["tree_references"].as_array().unwrap();
    if required.len() != frozen.len()
        || required.iter().any(|r| {
            !frozen.iter().any(|f| {
                f["parent_scope_id"] == r["parent_scope_id"]
                    && f["child_scope_id"] == r["child_scope_id"]
                    && f.get("after") == r.get("after")
            })
        })
    {
        return Err(Error::new(
            "conflict",
            "tree continuation no longer covers affected boundaries",
        )
        .at(&progress));
    }
    if before.nearest(Some(&id)) != scope {
        return Err(Error::new(
            "invalid_format",
            "tree owner disagrees with frozen old ancestry",
        ));
    }
    for r in plan["tree_references"].as_array().unwrap() {
        let child = r["child_scope_id"].as_str().unwrap();
        let parent = r["parent_scope_id"].as_str().unwrap();
        let old_root = before
            .store
            .resolve(child)
            .map_err(|_| Error::new("invalid_source", "frozen child missing"))?
            .header
            .as_ref()
            .unwrap();
        let new_root = after
            .store
            .resolve(child)
            .map_err(|_| Error::new("invalid_source", "projected child missing"))?
            .header
            .as_ref()
            .unwrap();
        if !descends(&after, child, &id)
            || (r["before"]["exists"] == true
                && before.nearest(old_root.parent.as_deref()) != parent)
            || (r.get("after").is_some() && after.nearest(new_root.parent.as_deref()) != parent)
        {
            return Err(Error::new(
                "conflict",
                "frozen tree reference is outside this reparenting",
            ));
        }
    }
    for d in &v.diagnostics {
        let transient = d["code"] == "invalid_source"
            && matches!(
                d["message"].as_str(),
                Some(
                    "repeated/cyclic stored child reference"
                        | "child reference disagrees with authoritative material ancestry"
                )
            )
            && plan["tree_references"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["child_scope_id"] == d["scope_id"]);
        if !transient {
            return Err(Error::new(
                "invalid_source",
                "unrelated source diagnostics prevent tree continuation",
            ));
        }
    }
    let records = v.records();
    for (p, c, _, _) in &records {
        if let Some(target) = c["item_id"].as_str()
            && !records
                .iter()
                .any(|(e, end, _, _)| e.parent() == p.parent() && end["claim_id"] == c["id"])
            && descends(&after, target, &id)
            && before.effective_run(target) != after.effective_run(target)
        {
            return Err(Error::new(
                "context_change_claimed",
                "continued reparenting changes a current claim's context",
            ));
        }
    }
    let mut steps = Vec::<(PathBuf, Value, Option<Vec<u8>>)>::new();
    for create in [true, false] {
        if !create {
            steps.push((
                old.path.clone(),
                json!({"exists":true,"sha256":plan["tree_change"]["item_before"]}),
                Some(raw.to_vec()),
            ));
        }
        for r in plan["tree_references"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r.get("after").is_some() == create)
        {
            let parent = r["parent_scope_id"].as_str().unwrap();
            let child = r["child_scope_id"].as_str().unwrap();
            let root = v
                .hosts
                .get(parent)
                .and_then(|h| h.as_ref())
                .ok_or_else(|| Error::new("host_missing", "frozen parent host unavailable"))?;
            if r.get("after").is_some_and(|a| a["child_scope_id"] != child) {
                return Err(Error::new("invalid_format", "frozen child ID mismatch"));
            }
            steps.push((
                root.join(format!(".work/scopes/{parent}/children/{child}.yaml")),
                r["before"].clone(),
                r.get("after").map(yaml_bytes),
            ));
        }
    }
    let step = plan["step"].as_u64().unwrap() as usize;
    if step != plan["tree_change"]["step"].as_u64().unwrap() as usize || step > steps.len() {
        return Err(Error::new(
            "invalid_format",
            "inconsistent tree progress step",
        ));
    }
    for (i, (p, before, after)) in steps.iter().enumerate() {
        let current = v.capture.files.get(p);
        let installed = current.map(|s| s.raw.as_slice()) == after.as_deref();
        let before_matches = if before["exists"] == false {
            current.is_none()
        } else {
            current.is_some_and(|s| hash(&s.raw) == before["sha256"])
        };
        if (i < step && !installed)
            || (i > step && !before_matches)
            || (i == step && !installed && !before_matches)
        {
            return Err(Error::new("conflict", "tree source differs from frozen step").at(p));
        }
    }
    let roots: BTreeSet<_> = steps
        .iter()
        .filter_map(|(p, _, _)| {
            v.physical
                .iter()
                .filter(|c| p.starts_with(&c.checkout.root))
                .max_by_key(|c| c.checkout.root.components().count())
                .map(|c| c.checkout.root.clone())
        })
        .collect();
    let mut others = Vec::new();
    let mut writer = None;
    for root in roots {
        let guard = CheckoutGuard::open(&root)?;
        if root == host {
            writer = Some(guard.into_writer()?);
        } else {
            others.push(guard);
        }
    }
    let mut capture = View::load(&v.project, false)?.capture;
    v.capture.compare_prelock(&capture)?;
    let mut writer = writer
        .ok_or_else(|| Error::new("host_missing", "material author checkout lock unavailable"))?;
    let mut known = partial();
    let creates = plan["tree_references"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r.get("after").is_some())
        .count();
    let phase = |completed: usize| {
        if completed >= steps.len() {
            3
        } else if completed > creates {
            2
        } else if completed == creates {
            1
        } else {
            0
        }
    };
    known["tree_changes"] = json!([{"item_id":id,"old_parent_scope_id":plan["tree_change"]["old_parent_scope_id"],"new_parent_scope_id":plan["tree_change"]["new_parent_scope_id"],"step":phase(step)}]);
    let op = plan["id"].as_str().unwrap().to_owned();
    let mut recovery = None;
    let result = (|| -> Result<Value> {
        if !capture.files.contains_key(&progress) {
            capture.publish(&progress, &yaml_bytes(&plan))?;
        }
        for (i, (p, _, after)) in steps.iter().enumerate().skip(step) {
            for g in &others {
                g.verify()?;
            }
            writer.verify()?;
            capture.recheck()?;
            if capture.files.get(p).map(|s| s.raw.as_slice()) != after.as_deref() {
                let exists = capture.files.contains_key(p);
                if p == &old.path {
                    recovery = writer.publish_raw_guarded(&id, raw, Some(old), || {
                        capture
                            .recheck()
                            .map_err(|e| OperationError::Conflict(e.to_string()))
                    })?;
                    known["updated"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"id":id,"path":path(p)}));
                    capture.advance(p, Some(raw))?;
                } else if let Some(raw) = after {
                    capture.publish(p, raw)?;
                } else {
                    capture.delete(p)?;
                }
                if p != &old.path {
                    known[if after.is_none() {
                        "deleted"
                    } else if exists {
                        "updated"
                    } else {
                        "created"
                    }]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":null,"path":path(p)}));
                }
            }
            known["tree_changes"][0]["step"] = json!(phase(i + 1));
            plan["step"] = json!(i + 1);
            plan["tree_change"]["step"] = json!(i + 1);
            #[cfg(test)]
            if p == &old.path {
                super::storage::files::inject("tree_after_material", p)
                    .map_err(|e| Error::new("io", e.to_string()))?;
            }
            capture.publish(&progress, &yaml_bytes(&plan))?;
        }
        capture.delete(&progress)?;
        let final_view = View::load(&v.project, false)?;
        final_view.require_healthy()?;
        known["tree_changes"] = json!([{"item_id":id,"old_parent_scope_id":plan["tree_change"]["old_parent_scope_id"],"new_parent_scope_id":plan["tree_change"]["new_parent_scope_id"],"step":3}]);
        Ok(final_view.wrap(json!({"item":super::scope_items::item(&final_view,&id,recovery.as_deref())?,"changed":true,"partial":known})))
    })();
    result.map_err(|mut e| {
        if let Some(p) = &recovery {
            e.details["previous_source_path"] = json!(path(p));
        }
        super::scopes::error_partial(e, known, &op)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_failure_after_material_reports_retained_source_and_publication() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let f =
            Fixture(std::env::temp_dir().join(format!("work-tree-recovery-{}", new_id().unwrap())));
        std::fs::create_dir_all(f.0.join(".work/items")).unwrap();
        let git = |args: &[&str]| {
            let r = std::process::Command::new("git")
                .arg("-C")
                .arg(&f.0)
                .args(args)
                .output()
                .unwrap();
            assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        };
        git(&["init", "--initial-branch=main"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "seed",
        ]);
        let a = "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb";
        let p = "cccccccccccc4ccc8ccccccccccccccc";
        for id in [a, b, p] {
            let mut h = super::super::operations::default_header(id.into(), id.into());
            if id == b {
                h.parent = Some(a.into());
            }
            std::fs::write(
                f.0.join(format!(".work/items/{id}.md")),
                super::super::operations::serialize(&h, b"opaque retained body"),
            )
            .unwrap();
        }
        let project = super::super::project::discover(Some(&f.0)).unwrap();
        let scopes = super::super::scopes::ScopeOperations::new(project.clone(), false);
        for id in [b, p] {
            let view = View::load(&project, false).unwrap();
            let inventory = view.inventory(id, &f.0).unwrap();
            scopes.host("host",&json!({"scope":id,"destination":path(&f.0),"expected_inventory":inventory["sha256"]})).unwrap();
        }
        let source = f.0.join(format!(".work/items/{a}.md"));
        let before = std::fs::read(&source).unwrap();
        let fault = super::super::storage::files::fail_next("tree_after_material");
        let error =
            super::super::scope_items::call(&project, "item_update", &json!({"id":a,"parent":p}))
                .unwrap_err();
        drop(fault);
        let recovery = super::super::coordination::decode_path(
            error.details["previous_source_path"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(std::fs::read(recovery).unwrap(), before);
        assert!(
            error.details["partial"]["updated"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == a)
        );
        assert!(
            View::load(&project, false)
                .unwrap()
                .pending
                .iter()
                .any(|v| v["id"] == error.details["operation_id"])
        );
        let resumed = super::super::scope_items::call(
            &project,
            "item_update",
            &json!({"id":a,"parent":p,"continue_id":error.details["operation_id"]}),
        )
        .unwrap();
        assert_eq!(resumed["partial"]["tree_changes"][0]["step"], 3);
        let fault = super::super::storage::files::fail_next("tree_after_material");
        let removal = super::super::scope_items::call(
            &project,
            "relation_remove",
            &json!({"source":a,"kind":"parent","target":p}),
        )
        .unwrap_err();
        drop(fault);
        let captured = View::load(&project, false).unwrap().capture;
        let mismatch=super::super::scope_items::call(&project,"relation_remove",&json!({"source":a,"kind":"parent","target":b,"continue_id":removal.details["operation_id"]})).unwrap_err();
        assert_eq!(mismatch.code, "conflict");
        captured.recheck().unwrap();
        let wrong_operation = super::super::scope_items::call(
            &project,
            "item_update",
            &json!({"id":a,"clear_parent":true,"continue_id":removal.details["operation_id"]}),
        )
        .unwrap_err();
        assert_eq!(wrong_operation.code, "conflict");
        captured.recheck().unwrap();
        let finished=super::super::scope_items::call(&project,"relation_remove",&json!({"source":a,"kind":"parent","target":p,"continue_id":removal.details["operation_id"]})).unwrap();
        assert_eq!(finished["partial"]["tree_changes"][0]["step"], 3);
    }
}

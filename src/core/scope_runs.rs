//! Scope-owned rounds and derived membership; no global registry or endings.
//! @mara implements DES-SCOPED-RUNS
use super::coordination::{
    ExecutionError as Error, ExecutionResult as Result, new_id, now_timestamp, yaml_bytes,
};
use super::graph::ItemGraph;
use super::items::ItemFile;
use super::operations::CheckoutGuard;
use super::project::Project;
use super::scope_publication as publication;
use super::scope_wire::{self as wire, hash};
use super::scopes::{View, coordination, partial, path, read_view};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(crate) fn descends(v: &View, id: &str, root: &str) -> bool {
    if root == "root" {
        return true;
    }
    let mut at = Some(id.to_owned());
    let mut seen = BTreeSet::new();
    while let Some(id) = at {
        if id == root {
            return true;
        }
        if !seen.insert(id.clone()) {
            break;
        }
        at = v.store.files.iter().find_map(|f| {
            f.header
                .as_ref()
                .filter(|h| h.id == id)
                .and_then(|h| h.parent.clone())
        });
    }
    false
}
pub(crate) fn owner(f: &ItemFile) -> Option<&str> {
    if f.path.parent()?.file_name()? != "items" {
        return None;
    }
    let run = f.path.parent()?.parent()?;
    (run.parent()?.file_name()? == "runs")
        .then(|| run.file_name()?.to_str())
        .flatten()
}
pub(crate) fn current_claims(v: &View) -> Vec<Value> {
    let records = v.records();
    records.iter().filter(|(p,c,_,_)| c.get("item_id").is_some() && !records.iter().any(|(e,end,_,_)| e.parent()==p.parent() && end["claim_id"]==c["id"]))
        .map(|(p,c,s,r)| json!({"claim":c,"ending":null,"run_id":r,"scope_id":s,"path":path(p),"current":true,"diagnostics":[]})).collect()
}
/// Read-only manifest inventory; an authoritative frozen start justifies its
/// saved manifest before the selecting boundary exists, but never activates it.
fn manifests(v: &View) -> Result<Vec<(PathBuf, Value, String, String)>> {
    let mut records: Vec<_> = v
        .records()
        .into_iter()
        .filter(|(p, _, _, _)| p.file_name().is_some_and(|n| n == "run.yaml"))
        .collect();
    for pending in &v.pending {
        if pending["supported"] != true || pending["kind"] != "start" {
            continue;
        }
        let progress = super::coordination::decode_path(pending["path"].as_str().unwrap())?;
        let Some(source) = v.capture.files.get(&progress) else {
            continue;
        };
        let plan = wire::envelope("StartProgress", &source.raw, &progress)?;
        let Some(rid) = plan["new_run_id"].as_str() else {
            continue;
        };
        let scope = progress
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let sid = scope.file_name().unwrap().to_str().unwrap();
        let home = scope.parent().unwrap().parent().unwrap().parent().unwrap();
        if plan["scope_id"] != sid || plan["id"] != progress.file_stem().unwrap().to_str().unwrap()
        {
            return Err(Error::new(
                "invalid_source",
                "pending start ownership differs from its path",
            )
            .at(&progress));
        }
        if v.source_host(sid).as_deref() != Some(home) {
            continue;
        }
        let p = scope.join(format!("runs/{rid}/run.yaml"));
        if records.iter().any(|(file, _, _, _)| file == &p) {
            continue;
        }
        if let Some(source) = v.capture.files.get(&p) {
            let m = wire::envelope("RunManifest", &source.raw, &p)?;
            if m["id"] != rid {
                return Err(Error::new(
                    "invalid_source",
                    "pending manifest identity differs from its home",
                )
                .at(&p));
            }
            records.push((p, m, sid.into(), rid.into()));
        }
    }
    Ok(records)
}
pub(crate) fn manifest(v: &View, rid: &str) -> Result<(PathBuf, Value, String)> {
    let records: Vec<_> = manifests(v)?
        .into_iter()
        .filter(|(p, m, _, r)| {
            r == rid && m["id"] == rid && p.file_name().is_some_and(|n| n == "run.yaml")
        })
        .collect();
    match records.as_slice() {
        [(p, m, s, _)] => Ok((p.clone(), m.clone(), s.clone())),
        [] => Err(Error::new("not_found", "run manifest not found")),
        _ => Err(Error::new("invalid_source", "ambiguous run identity")),
    }
}
pub(crate) fn active(v: &View, rid: &str) -> Result<(PathBuf, String)> {
    if v.execution()["state"] != "ready" {
        v.require_healthy()?;
        return Err(Error::new(
            "coordination_unavailable",
            "healthy explicit initialization is required",
        ));
    }
    v.require_healthy()?;
    let (p, m, s) = manifest(v, rid)?;
    if m["phase"] != "active"
        || v.anchors
            .get(&s)
            .is_none_or(|a| a["execution"]["mode"] != "boundary" || a["execution"]["run_id"] != rid)
    {
        return Err(Error::new(
            "run_not_current",
            "run is not the active selected round",
        ));
    }
    Ok((p, s))
}
pub(crate) fn run_view(v: &View, rid: &str) -> Result<Value> {
    let (p, m, sid) = manifest(v, rid)?;
    let selected = v
        .anchors
        .get(&sid)
        .is_some_and(|a| a["execution"]["mode"] == "boundary" && a["execution"]["run_id"] == rid);
    let current =
        selected && m["phase"] == "active" && !v.physical_mode && v.execution()["state"] == "ready";
    let mut members = BTreeSet::new();
    let mut required = BTreeSet::new();
    for f in &v.store.files {
        let Some(h) = &f.header else { continue };
        let historical =
            owner(f).is_some_and(|r| manifest(v, r).is_ok_and(|(_, m, _)| m["phase"] != "active"));
        if historical {
            continue;
        }
        if descends(v, &h.id, &sid) {
            if h.id != sid {
                required.insert(h.id.clone());
            }
            if v.effective_run(&h.id).0.as_deref() == Some(rid) && owner(f).is_none_or(|r| r == rid)
            {
                members.insert(h.id.clone());
            }
        }
    }
    let descendants: BTreeSet<_> = manifests(v)?
        .into_iter()
        .filter(|(p, m, s, r)| {
            p.file_name().is_some_and(|n| n == "run.yaml")
                && m.get("phase").is_some()
                && r != rid
                && s != &sid
                && descends(v, s, &sid)
        })
        .map(|(_, _, _, r)| r)
        .collect();
    let claims = current_claims(v);
    let current_claims: BTreeSet<_> = claims
        .iter()
        .filter(|c| {
            c["run_id"] == rid
                || c["claim"]["item_id"] == sid
                || c["claim"]["item_id"]
                    .as_str()
                    .is_some_and(|i| required.contains(i))
        })
        .map(|c| c["claim"]["id"].as_str().unwrap().to_owned())
        .collect();
    let session_claims: BTreeSet<_> = claims
        .iter()
        .filter(|c| c["claim"]["named_session"]["run_id"] == rid)
        .map(|c| c["claim"]["id"].as_str().unwrap().to_owned())
        .collect();
    let graph = ItemGraph::from_store(&v.store);
    let finished = if sid == "root" || m["phase"] != "active" || !current || !graph.is_valid() {
        None
    } else {
        Some(
            !required.is_empty()
                && required
                    .iter()
                    .all(|i| graph.evaluate(i).is_ok_and(|e| e.effective_done))
                && current_claims.is_empty()
                && session_claims.is_empty(),
        )
    };
    Ok(
        json!({"run":m,"scope_id":sid,"path":path(&p),"current":current,"members":members,"required_members":required,"descendant_runs":descendants,"finished":finished,"current_claims":current_claims,"session_claims":session_claims,"diagnostics":v.diagnostics}),
    )
}
pub(crate) fn require_current_wisp(v: &View, f: &ItemFile) -> Result<()> {
    let rid = owner(f).ok_or_else(|| Error::new("invalid_argument", "not a wisp"))?;
    active(v, rid)?;
    require_wisp_home(v, f)
}
/// Same-home checks during a validated frozen tree continuation, which is pending by definition.
pub(crate) fn require_wisp_home(v: &View, f: &ItemFile) -> Result<()> {
    let rid = owner(f).ok_or_else(|| Error::new("invalid_argument", "not a wisp"))?;
    let (_, m, sid) = manifest(v, rid)?;
    if m["phase"] != "active"
        || v.anchors
            .get(&sid)
            .is_none_or(|a| a["execution"]["run_id"] != rid)
    {
        return Err(Error::new(
            "run_not_current",
            "wisp owner is not the current active round",
        ));
    }
    if v.effective_run(&f.header.as_ref().unwrap().id).0.as_deref() != Some(rid) {
        return Err(Error::new(
            "wisp_run_mismatch",
            "wisp ancestry differs from its owner",
        ));
    }
    Ok(())
}
pub fn read(project: &Project, name: &str, a: &Value) -> Result<Value> {
    let (v, guard) = read_view(project, a["view"] == "physical")?;
    let out = if name == "run_inspect" {
        json!({"run":run_view(&v,a["run_id"].as_str().unwrap())?})
    } else {
        let scope = a["scope"].as_str().map(|s| v.scope_id(s)).transpose()?;
        let ids: BTreeSet<_> = manifests(&v)?
            .into_iter()
            .filter(|(p, m, s, _)| {
                p.file_name().is_some_and(|n| n == "run.yaml")
                    && (a["include_terminal"] == true || m["phase"] == "active")
                    && scope.as_deref().is_none_or(|root| descends(&v, s, root))
            })
            .map(|(_, _, _, r)| r)
            .collect();
        json!({"runs":ids.iter().map(|r|run_view(&v,r)).collect::<Result<Vec<_>>>()?})
    };
    v.capture.recheck()?;
    if let Some(g) = guard {
        g.verify()?;
    }
    Ok(v.wrap(out))
}
fn material_scope(v: &View, input: &str) -> Result<String> {
    if input == "root" {
        return Err(Error::new(
            "invalid_argument",
            "root uses init/rotation, not child start/inherit",
        ));
    }
    let sid = super::scope_items::resolve(v, input)?;
    let f = v
        .store
        .resolve(&sid)
        .map_err(|_| Error::new("invalid_source", "scope root must be valid material"))?;
    if v.is_wisp(f) {
        return Err(Error::new(
            "invalid_argument",
            "promote a wisp before establishing a boundary",
        ));
    }
    Ok(sid)
}
fn next_anchor(sid: &str, rid: &str) -> Value {
    json!({"format_version":2,"scope_id":sid,"execution":{"mode":"boundary","run_id":rid}})
}
fn new_manifest(plan: &Value) -> Value {
    json!({"format_version":2,"id":plan["new_run_id"],"created_at":plan["run_created_at"],"phase":"active"})
}
fn refs(v: &View, sid: &str) -> Result<Vec<Value>> {
    let parent = v.nearest(
        v.store
            .resolve(sid)
            .unwrap()
            .header
            .as_ref()
            .unwrap()
            .parent
            .as_deref(),
    );
    let mut edges = vec![];
    if !v.anchors.contains_key(sid) {
        edges.push((parent.clone(), sid.to_owned(), true));
        for (child, old) in &v.parents {
            if old == &parent && descends(v, child, sid) {
                edges.push((sid.into(), child.clone(), true));
                edges.push((old.clone(), child.clone(), false));
            }
        }
    }
    edges.sort_by(|a, b| (!a.2, &a.0, &a.1).cmp(&(!b.2, &b.0, &b.1)));
    edges.into_iter().map(|(p,c,create)| {
        let host=if p==sid {v.source_host(sid)} else {v.source_host(&p)}.ok_or_else(||Error::new("host_missing","reference home missing"))?;
        let file=host.join(format!(".work/scopes/{p}/children/{c}.yaml"));
        let mut edge=json!({"parent_scope_id":p,"child_scope_id":c,"before":publication::snapshot(&v.capture,&file)});
        if create {edge["after"]=json!({"format_version":2,"child_scope_id":c});}
        Ok(edge)
    }).collect()
}
fn projected(v: &View, plan: &Value, home: &Path, progress: &Path, after: bool) -> Result<View> {
    let sid = plan["scope_id"].as_str().unwrap();
    let rid = plan["new_run_id"].as_str().unwrap();
    let mut out = v.clone();
    let anchor = home.join(format!(".work/scopes/{sid}/scope.yaml"));
    let original = if publication::snapshot(&v.capture, &anchor) == plan["anchor_before"] {
        v.capture
            .files
            .get(&anchor)
            .map(|s| wire::envelope("ScopeAnchor", &s.raw, &anchor))
            .transpose()?
    } else {
        let stage = anchor
            .parent()
            .unwrap()
            .join(format!(".stage-{}-anchor", plan["id"].as_str().unwrap()));
        if plan["anchor_before"]["exists"] == false {
            None
        } else if let Ok(d) = super::storage::files::Directory::open(stage.parent().unwrap())
            && let Some(source) = d.optional(stage.file_name().unwrap().to_str().unwrap())?
        {
            if hash(&source.raw) != plan["anchor_before"]["sha256"] {
                return Err(Error::new("conflict", "original anchor recovery changed").at(stage));
            }
            Some(wire::envelope("ScopeAnchor", &source.raw, &stage)?)
        } else {
            Some(json!({"format_version":2,"scope_id":sid,"execution":{"mode":"inherit"}}))
        }
    };
    let current = if after {
        Some(next_anchor(sid, rid))
    } else {
        original
    };
    for p in &mut out.physical {
        p.records.remove(progress);
        p.pending.retain(|x| x["path"] != path(progress));
        if p.checkout.root == home {
            p.records.remove(&anchor);
            if let Some(a) = &current {
                p.anchors.insert(sid.into(), a.clone());
            } else {
                p.anchors.remove(sid);
            }
            let run = home.join(format!(".work/scopes/{sid}/runs/{rid}/run.yaml"));
            if after {
                p.records.insert(run, new_manifest(plan));
            } else {
                p.records.remove(&run);
            }
            p.diagnostics.retain(|d| {
                !(d["message"] == "orphaned scope/run directory lacks its manifest"
                    && (d["path"] == path(&home.join(format!(".work/scopes/{sid}")))
                        || d["path"] == path(&home.join(format!(".work/scopes/{sid}/runs/{rid}")))))
            });
        }
        for edge in plan["tree_references"].as_array().unwrap() {
            let parent = edge["parent_scope_id"].as_str().unwrap();
            let child = edge["child_scope_id"].as_str().unwrap();
            let parent_home = if parent == sid {
                Some(home.to_owned())
            } else {
                v.source_host(parent)
            };
            if parent_home.as_ref() != Some(&p.checkout.root) {
                continue;
            }
            p.records.remove(
                &p.checkout
                    .root
                    .join(format!(".work/scopes/{parent}/children/{child}.yaml")),
            );
            let kids = p.children.entry(parent.into()).or_default();
            kids.retain(|x| x != child);
            if if after {
                edge.get("after").is_some()
            } else {
                edge["before"]["exists"] == true
            } {
                kids.push(child.into());
                kids.sort();
            }
        }
    }
    out.rebuild();
    Ok(out)
}
type Publication = (PathBuf, Option<Vec<u8>>, Value);
fn steps(v: &View, plan: &Value, home: &Path) -> Result<Vec<Publication>> {
    let sid = plan["scope_id"].as_str().unwrap();
    let rid = plan["new_run_id"].as_str().unwrap();
    let mut result = vec![(
        home.join(format!(".work/scopes/{sid}/runs/{rid}/run.yaml")),
        Some(yaml_bytes(&new_manifest(plan))),
        json!({"exists":false}),
    )];
    for create in [true, false] {
        if !create {
            result.push((
                home.join(format!(".work/scopes/{sid}/scope.yaml")),
                Some(yaml_bytes(&next_anchor(sid, rid))),
                plan["anchor_before"].clone(),
            ));
        }
        for edge in plan["tree_references"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e.get("after").is_some() == create)
        {
            let parent = edge["parent_scope_id"].as_str().unwrap();
            let host = if parent == sid {
                home.to_owned()
            } else {
                v.source_host(parent)
                    .ok_or_else(|| Error::new("host_missing", "frozen parent home missing"))?
            };
            result.push((
                host.join(format!(
                    ".work/scopes/{parent}/children/{}.yaml",
                    edge["child_scope_id"].as_str().unwrap()
                )),
                edge.get("after").map(yaml_bytes),
                edge["before"].clone(),
            ));
        }
    }
    Ok(result)
}
fn verify(v: &View, plan: &Value, home: &Path, progress: &Path) -> Result<(View, View)> {
    let sid = plan["scope_id"].as_str().unwrap();
    let before = projected(v, plan, home, progress, false)?;
    let after = projected(v, plan, home, progress, true)?;
    before.require_healthy()?;
    super::scope_items::validate_context_change(&before, &after)?;
    after.require_healthy()?;
    if material_scope(&before, sid)? != sid {
        return Err(Error::new("conflict", "frozen material root changed"));
    }
    let parent = before.nearest(
        before
            .store
            .resolve(sid)
            .unwrap()
            .header
            .as_ref()
            .unwrap()
            .parent
            .as_deref(),
    );
    if plan["parent_scope_id"] != parent {
        return Err(Error::new("conflict", "start parent changed"));
    }
    let shape = |edges: &Value| {
        edges.as_array().unwrap().iter().map(|e|json!({"parent_scope_id":e["parent_scope_id"],"child_scope_id":e["child_scope_id"],"after":e.get("after")})).collect::<Vec<_>>()
    };
    if shape(&plan["tree_references"]) != shape(&json!(refs(&before, sid)?)) {
        return Err(Error::new("conflict", "start reference target set changed"));
    }
    let incoming = before
        .source_host(&parent)
        .unwrap()
        .join(format!(".work/scopes/{parent}/children/{sid}.yaml"));
    if plan["anchor_before"]["exists"] == true
        && publication::snapshot(&v.capture, &incoming) != plan["child_before"]
    {
        return Err(Error::new("conflict", "existing incoming reference changed").at(incoming));
    }
    if plan["anchor_before"]["exists"] == false
        && plan["tree_references"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["child_scope_id"] == sid && e["parent_scope_id"] == parent)
            .is_none_or(|e| e["before"] != plan["child_before"] || e["before"]["exists"] != false)
    {
        return Err(Error::new(
            "conflict",
            "incoming frozen reference disagrees with original snapshot",
        ));
    }
    if before.anchors.get(sid).is_some_and(|a| {
        a["execution"]["mode"] == "boundary"
            && manifest(&before, a["execution"]["run_id"].as_str().unwrap())
                .is_ok_and(|(_, m, _)| m["phase"] == "active")
    }) {
        return Err(Error::new(
            "run_conflict",
            "start cannot end an active round",
        ));
    }
    super::scope_items::validate_context_change(&before, &after)?;
    let publications = steps(&before, plan, home)?;
    let step = plan["step"].as_u64().unwrap() as usize;
    if step > publications.len() {
        return Err(Error::new("conflict", "start step exceeds frozen sequence"));
    }
    for (i, (p, raw, snapshot)) in publications.iter().enumerate() {
        let installed = match raw {
            Some(raw) => v.capture.files.get(p).is_some_and(|s| &s.raw == raw),
            None => !v.capture.files.contains_key(p),
        };
        let original = publication::snapshot(&v.capture, p) == *snapshot;
        if (i < step && !installed)
            || (i == step && !installed && !original)
            || (i > step && !original)
        {
            return Err(Error::new("conflict", "frozen start source changed").at(p));
        }
    }
    Ok((before, after))
}
pub fn start(project: &Project, a: &Value) -> Result<Value> {
    let mut v = View::load(project, false)?;
    let sid = material_scope(&v, a["scope"].as_str().unwrap())?;
    let home = v
        .source_host(&sid)
        .ok_or_else(|| Error::new("host_missing", "start home missing"))?;
    let continued = a["continue_id"].as_str();
    if continued.is_none() {
        v.require_healthy()?;
        if v.execution()["state"] != "ready" {
            return Err(Error::new(
                "coordination_unavailable",
                "explicit init required",
            ));
        }
    }
    let (progress, mut plan) = if let Some(id) = continued {
        let p = home.join(format!(".work/scopes/{sid}/progress/start/{id}.yaml"));
        let s = v
            .capture
            .files
            .get(&p)
            .ok_or_else(|| Error::new("not_found", "exact start continuation missing"))?;
        let plan = wire::envelope("StartProgress", &s.raw, &p)?;
        if plan["id"] != id || plan["scope_id"] != sid || plan.get("new_run_id").is_none() {
            return Err(Error::new("conflict", "not this start operation"));
        }
        (p, plan)
    } else {
        if let Some(anchor) = v.anchors.get(&sid)
            && anchor["execution"]["mode"] == "boundary"
        {
            let rid = anchor["execution"]["run_id"].as_str().unwrap();
            if manifest(&v, rid)?.1["phase"] == "active" {
                let (fresh, g) = read_view(project, false)?;
                v.capture.compare_prelock(&fresh.capture)?;
                active(&fresh, rid)?;
                if let Some(g) = g {
                    g.verify()?;
                }
                return Ok(fresh.wrap(
                    json!({"run":run_view(&fresh,rid)?,"changed":false,"partial":partial()}),
                ));
            }
        }
        let op = new_id()?;
        let rid = new_id()?;
        let parent = v.nearest(
            v.store
                .resolve(&sid)
                .unwrap()
                .header
                .as_ref()
                .unwrap()
                .parent
                .as_deref(),
        );
        let child = v
            .source_host(&parent)
            .unwrap()
            .join(format!(".work/scopes/{parent}/children/{sid}.yaml"));
        let plan = json!({"format_version":2,"id":op,"created_at":now_timestamp(),"scope_id":sid,"parent_scope_id":parent,"new_run_id":rid,"run_created_at":now_timestamp(),"anchor_before":publication::snapshot(&v.capture,&home.join(format!(".work/scopes/{sid}/scope.yaml"))),"child_before":publication::snapshot(&v.capture,&child),"tree_references":refs(&v,&sid)?,"step":0});
        (
            home.join(format!(".work/scopes/{sid}/progress/start/{op}.yaml")),
            plan,
        )
    };
    wire::validate("StartProgress", &plan)?;
    let op = plan["id"].as_str().unwrap().to_owned();
    let rid = plan["new_run_id"].as_str().unwrap().to_owned();
    let (before, _) = verify(&v, &plan, &home, &progress)?;
    let publications = steps(&before, &plan, &home)?;
    let mut roots = BTreeSet::from([home.clone()]);
    for (p, _, _) in &publications {
        roots.insert(
            p.ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == ".work"))
                .unwrap()
                .parent()
                .unwrap()
                .to_owned(),
        );
    }
    for root in &roots {
        publication::preflight(root, &root.join(".work/operations.lock"))?;
    }
    publication::preflight(&home, &progress)?;
    for (p, raw, _) in &publications {
        if raw.is_some() {
            let root = roots
                .iter()
                .filter(|r| p.starts_with(r))
                .max_by_key(|r| r.components().count())
                .unwrap();
            publication::preflight(root, p)?;
        }
    }
    let mut effects = partial();
    let mut last = None;
    let result = (|| -> Result<Value> {
        let guard = coordination(&v, true, false)?.ok_or_else(|| {
            Error::new(
                "coordination_unavailable",
                "initialized coordination missing",
            )
        })?;
        let fresh = View::load(project, false)?;
        v.capture.compare_prelock(&fresh.capture)?;
        v = fresh;
        let mut checkouts = Vec::new();
        for root in &roots {
            publication::ensure(&mut v.capture, &root.join(".work"), &mut effects)?;
            checkouts.push(CheckoutGuard::open(root)?);
        }
        let fresh = View::load(project, false)?;
        v.capture.compare_prelock(&fresh.capture)?;
        v = fresh;
        let fence = || -> Result<()> {
            guard.verify()?;
            for c in &checkouts {
                c.verify()?;
            }
            Ok(())
        };
        verify(&v, &plan, &home, &progress)?;
        if continued.is_none() {
            publication::save(
                &mut v.capture,
                &progress,
                &yaml_bytes(&plan),
                &format!(".stage-{op}-intent"),
                &mut effects,
                None,
                &fence,
            )?;
        }
        for (i, (p, raw, _)) in publications
            .iter()
            .enumerate()
            .skip(plan["step"].as_u64().unwrap() as usize)
        {
            fence()?;
            v.capture.recheck()?;
            let fresh = View::load(project, false)?;
            verify(&fresh, &plan, &home, &progress)?;
            last = Some(p.clone());
            if let Some(raw) = raw {
                publication::save_resuming(
                    &mut v.capture,
                    p,
                    raw,
                    &format!(
                        ".stage-{op}-{}",
                        if p.file_name().is_some_and(|n| n == "scope.yaml") {
                            "anchor".into()
                        } else {
                            format!("file-{i}")
                        }
                    ),
                    &mut effects,
                    None,
                    &fence,
                )?;
                if i == 0 {
                    for dir in ["items", "sessions", "claims", "handoffs"] {
                        publication::ensure(
                            &mut v.capture,
                            &p.parent().unwrap().join(dir),
                            &mut effects,
                        )?;
                    }
                }
            } else if v.capture.files.contains_key(p) {
                v.capture.delete(p)?;
                publication::known(&mut effects, "deleted", p, None);
            }
            plan["step"] = json!(i + 1);
            publication::save_resuming(
                &mut v.capture,
                &progress,
                &yaml_bytes(&plan),
                &format!(".stage-{op}-step-{i}"),
                &mut effects,
                None,
                &fence,
            )?;
        }
        let fresh = View::load(project, false)?;
        verify(&fresh, &plan, &home, &progress)?;
        v.capture.recheck()?;
        fence()?;
        for (p, raw, _) in &publications {
            if raw.is_some() {
                publication::sync(&v.capture, p)?;
            }
        }
        publication::sync(&v.capture, &progress)?;
        for (i, (p, raw, expected)) in publications.iter().enumerate() {
            if raw.is_some() {
                publication::cleanup(
                    &v.capture,
                    &p.parent().unwrap().join(format!(
                        ".stage-{op}-{}",
                        if p.file_name().is_some_and(|n| n == "scope.yaml") {
                            "anchor".into()
                        } else {
                            format!("file-{i}")
                        }
                    )),
                    expected,
                    &fence,
                )?;
            }
        }
        for i in 0..publications.len() {
            let mut old = plan.clone();
            old["step"] = json!(i);
            publication::cleanup(
                &v.capture,
                &progress
                    .parent()
                    .unwrap()
                    .join(format!(".stage-{op}-step-{i}")),
                &json!({"exists":true,"sha256":hash(&yaml_bytes(&old))}),
                &fence,
            )?;
        }
        fence()?;
        v.capture.recheck()?;
        let fresh = View::load(project, false)?;
        verify(&fresh, &plan, &home, &progress)?;
        v.capture.delete(&progress)?;
        publication::known(&mut effects, "deleted", &progress, None);
        let fresh = View::load(project, false)?;
        fresh.require_healthy()?;
        active(&fresh, &rid)?;
        v.capture.recheck()?;
        fresh.capture.recheck()?;
        fence()?;
        Ok(fresh.wrap(json!({"run":run_view(&fresh,&rid)?,"changed":true,"partial":effects})))
    })();
    result.map_err(|mut e| {
        e.details["operation_id"] = json!(op);
        publication::attempted(e, &effects, last.as_deref())
    })
}
pub fn inherit(project: &Project, a: &Value) -> Result<Value> {
    let mut v = View::load(project, false)?;
    v.require_healthy()?;
    let sid = material_scope(&v, a["scope"].as_str().unwrap())?;
    let home = v.source_host(&sid).unwrap();
    let mut after = v.clone();
    let anchor = json!({"format_version":2,"scope_id":sid,"execution":{"mode":"inherit"}});
    if let Some(own) = v.anchors.get(&sid)
        && own["execution"]["mode"] == "boundary"
        && manifest(&v, own["execution"]["run_id"].as_str().unwrap())?.1["phase"] == "active"
    {
        return Err(Error::new(
            "run_conflict",
            "inherit cannot end an active own round",
        ));
    }
    after.anchors.insert(sid.clone(), anchor.clone());
    let (rid, state) = after.effective_run(&sid);
    if state != "active" {
        return Err(Error::new("run_not_current", "no healthy inherited round"));
    }
    active(&v, rid.as_deref().unwrap())?;
    super::scope_items::validate_context_change(&v, &after)?;
    let changed = v
        .anchors
        .get(&sid)
        .is_some_and(|x| x["execution"]["mode"] != "inherit");
    let p = home.join(format!(".work/scopes/{sid}/scope.yaml"));
    let before = publication::snapshot(&v.capture, &p);
    let stage = format!(".stage-{}", new_id()?);
    let mut effects = partial();
    if changed {
        publication::preflight(&home, &p)?;
    }
    let guard = coordination(&v, true, false)?
        .ok_or_else(|| Error::new("coordination_unavailable", "explicit init required"))?;
    let fresh = View::load(project, false)?;
    v.capture.compare_prelock(&fresh.capture)?;
    v = fresh;
    let checkout = CheckoutGuard::open(&home)?;
    let fresh = View::load(project, false)?;
    v.capture.compare_prelock(&fresh.capture)?;
    v = fresh;
    let fence = || -> Result<()> {
        guard.verify()?;
        checkout.verify()?;
        Ok(())
    };
    let result = (|| -> Result<Value> {
        if changed {
            publication::save(
                &mut v.capture,
                &p,
                &yaml_bytes(&anchor),
                &stage,
                &mut effects,
                None,
                &fence,
            )?;
        }
        let fresh = View::load(project, false)?;
        fresh.require_healthy()?;
        if fresh.effective_run(&sid) != (rid.clone(), "active") {
            return Err(Error::new("conflict", "inherit postcondition changed"));
        }
        v.capture.recheck()?;
        fence()?;
        if changed {
            publication::sync(&v.capture, &p)?;
            publication::cleanup(
                &v.capture,
                &p.parent().unwrap().join(&stage),
                &before,
                &fence,
            )?;
        }
        fresh.capture.recheck()?;
        v.capture.recheck()?;
        fence()?;
        Ok(fresh.wrap(json!({"scope":fresh.scope(&sid),"changed":changed,"partial":effects})))
    })();
    result.map_err(|e| publication::attempted(e, &effects, Some(&p)))
}
pub(crate) fn edit_wisp(
    project: &Project,
    v: &View,
    old: &ItemFile,
    raw: &[u8],
    name: &str,
) -> Result<Value> {
    require_current_wisp(v, old)?;
    let home = old
        .path
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".work"))
        .unwrap()
        .parent()
        .unwrap();
    let checkout = CheckoutGuard::open(home)?;
    let mut fresh = View::load(project, false)?;
    v.capture.compare_prelock(&fresh.capture)?;
    let fence = || -> Result<()> {
        checkout.verify()?;
        Ok(())
    };
    let stage = format!(".stage-{}", new_id()?);
    let before = publication::snapshot(&fresh.capture, &old.path);
    let id = &old.header.as_ref().unwrap().id;
    let mut effects = partial();
    let changed = old.raw != raw;
    let result = (|| -> Result<Value> {
        if changed {
            publication::save(
                &mut fresh.capture,
                &old.path,
                raw,
                &stage,
                &mut effects,
                Some(id),
                &fence,
            )?;
        }
        let final_view = View::load(project, false)?;
        final_view.require_healthy()?;
        require_current_wisp(&final_view, final_view.store.resolve(id).unwrap())?;
        fresh.capture.recheck()?;
        fence()?;
        if changed {
            publication::sync(&fresh.capture, &old.path)?;
            publication::cleanup(
                &fresh.capture,
                &old.path.parent().unwrap().join(&stage),
                &before,
                &fence,
            )?;
        }
        final_view.capture.recheck()?;
        fresh.capture.recheck()?;
        fence()?;
        let mut out = json!({"item":super::scope_items::item(&final_view,id,None)?,"changed":changed,"partial":effects});
        if name == "item_close" {
            out["saved_handoffs"] = json!([]);
            out["ending"] = Value::Null;
        }
        Ok(final_view.wrap(out))
    })();
    result.map_err(|e| publication::attempted(e, &effects, Some(&old.path)))
}
/// Internal lifecycle foundation, not a public rotation/ending operation.
pub fn prepare_rotation(project: &Project, abandon_unfinished: bool) -> Result<Value> {
    let (v, g) = read_view(project, false)?;
    v.require_healthy()?;
    let rid = v.execution()["root_run_id"]
        .as_str()
        .ok_or_else(|| Error::new("coordination_unavailable", "explicit init required"))?
        .to_owned();
    active(&v, &rid)?;
    let view = run_view(&v, &rid)?;
    if !view["current_claims"].as_array().unwrap().is_empty()
        || !view["session_claims"].as_array().unwrap().is_empty()
    {
        return Err(Error::new(
            "claims_present",
            "root ownership must drain, including cross-run named claims",
        ));
    }
    for r in view["descendant_runs"].as_array().unwrap() {
        if manifest(&v, r.as_str().unwrap())?.1["phase"] == "active" {
            return Err(Error::new(
                "descendant_run_active",
                "descendants must end before rotation",
            ));
        }
    }
    let graph = ItemGraph::from_store(&v.store);
    let unfinished = v
        .store
        .files
        .iter()
        .filter(|f| owner(f) == Some(rid.as_str()))
        .any(|f| {
            !graph
                .evaluate(&f.header.as_ref().unwrap().id)
                .is_ok_and(|e| e.effective_done)
        });
    if unfinished && !abandon_unfinished {
        return Err(Error::new(
            "run_unfinished",
            "unfinished root wisps require explicit abandonment",
        ));
    }
    v.capture.recheck()?;
    if let Some(g) = g {
        g.verify()?;
    }
    Ok(
        json!({"old_run_id":rid,"target_phase":if unfinished {"abandoned"} else {"finalized"},"protected_files":v.capture.files.iter().map(|(p,s)|json!({"path":path(p),"sha256":hash(&s.raw)})).collect::<Vec<_>>()}),
    )
}

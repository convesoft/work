//! Material operations over the authoritative beta.2 scope view.
use super::coordination::{ExecutionError as Error, ExecutionResult as Result, new_id};
use super::graph::{Blocker, ItemGraph};
use super::items::{Completion, ItemFile, ManualState};
use super::operations::{self, CheckoutGuard, MetadataChange, OperationError};
use super::scopes::{View, coordination, diagnostic, frontmatter, partial, path};
use serde_json::{Value, json};
use std::path::Path;

fn display_id(v: &View, id: &str) -> String {
    let length = (8..=id.len())
        .find(|&n| resolve(v, &format!("w-{}", &id[..n])).is_ok_and(|resolved| resolved == id))
        .unwrap_or(id.len());
    format!("w-{}", &id[..length])
}

fn graph_diagnostics(v: &View) -> Vec<Value> {
    ItemGraph::from_store(&v.store)
        .diagnostics()
        .iter()
        .map(|d| diagnostic("invalid_source", &d.message, Some(&d.path), None))
        .collect()
}
fn claim(v: &View, id: &str) -> Option<Value> {
    if v.physical_mode || !v.diagnostics.is_empty() || !v.pending.is_empty() {
        return None;
    }
    acquisition(v, id)
}
// Inspection eligibility is not authorization. A bounded retry still fences
// the captured acquisition; its prospective view must validate before writes.
fn acquisition(v: &View, id: &str) -> Option<Value> {
    let records = v.records();
    let candidates = records
        .iter()
        .filter(|(p, c, _, _)| {
            c["item_id"] == id
                && !records.iter().any(|(e, ending, _, _)| {
                    e.parent() == p.parent() && ending["claim_id"] == c["id"]
                })
        })
        .collect::<Vec<_>>();
    let [(p, c, s, r)] = candidates.as_slice() else {
        return None;
    };
    if v.diagnostics.iter().any(|d| {
        d["code"] == "claim_conflict"
            && (d["item_id"] == id || d["path"] == path(p) || d["scope_id"] == s.as_str())
    }) {
        return None;
    }
    Some(
        json!({"claim":c,"ending":null,"run_id":r,"scope_id":s,"path":path(p),"current":true,"diagnostics":[]}),
    )
}
fn context(v: &View, file: &ItemFile) -> Value {
    let id = &file.header.as_ref().unwrap().id;
    let rel = file
        .path
        .strip_prefix(
            file.path
                .ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == ".work"))
                .unwrap(),
        )
        .unwrap();
    let parts: Vec<_> = rel.iter().map(|x| x.to_str().unwrap_or("")).collect();
    let wisp = parts.first() == Some(&"scopes");
    let owner = if wisp { Some(parts[3]) } else { None };
    let (mut effective, mut state) = v.effective_run(id);
    if wisp {
        effective = owner.map(str::to_owned);
        state = if v
            .records()
            .iter()
            .any(|(_, m, _, r)| Some(r.as_str()) == owner && m["phase"] == "active")
        {
            "active"
        } else {
            "ended"
        };
    }
    if v.physical_mode || !v.diagnostics.is_empty() || !v.pending.is_empty() {
        state = "unavailable";
    }
    let root = file
        .path
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".work"))
        .unwrap()
        .parent()
        .unwrap();
    json!({"scope_id":if wisp{parts[1].to_owned()}else{v.nearest(Some(id))},"owner_run_id":owner,"effective_run_id":effective,"execution_state":state,"source_worktree":path(root),"path":path(&file.path),"persistence":if wisp{"wisp"}else{"material"},"authoritative":!v.physical_mode})
}
pub(crate) fn item(v: &View, id: &str, recovery: Option<&Path>) -> Result<Value> {
    if let Some(error) = authority_error(v, id) {
        return Err(error);
    }
    let f = v
        .store
        .resolve(id)
        .map_err(|_| Error::new("invalid_source", "material source missing/invalid"))?;
    let h = f.header.as_ref().unwrap();
    let graph = ItemGraph::from_store(&v.store);
    let r = graph
        .relations(id)
        .map_err(|_| Error::new("invalid_source", "cannot inspect relations"))?;
    let eval = graph.evaluate(id).ok();
    let current = if v.physical_mode { None } else { claim(v, id) };
    let cx = context(v, f);
    let mut blockers = Vec::new();
    for b in eval.iter().flat_map(|e| &e.blockers) {
        let (kind, item, via) = match b {
            Blocker::Completed => ("not_open", None, None),
            Blocker::Aggregate => ("aggregate", None, None),
            Blocker::Child(i) => ("children", Some(i.as_str()), None),
            Blocker::Prerequisite { id, inherited_from } => (
                if inherited_from.is_some() {
                    "ancestor_depends_on"
                } else {
                    "depends_on"
                },
                Some(id.as_str()),
                inherited_from.as_deref(),
            ),
        };
        blockers.push(json!({"kind":kind,"item_id":item,"via":via,"claim_id":null}));
    }
    if let Some(c) = &current {
        blockers
            .push(json!({"kind":"claimed","item_id":id,"via":null,"claim_id":c["claim"]["id"]}));
    }
    if cx["execution_state"] == "ended" {
        blockers.push(json!({"kind":if cx["persistence"]=="wisp"{"historical_wisp"}else{"execution_unavailable"},"item_id":id,"via":null,"claim_id":null}));
    }
    let mut relations = Vec::new();
    for (kind, direction, ids) in [
        (
            "parent",
            "outgoing",
            r.parent.iter().cloned().collect::<Vec<_>>(),
        ),
        ("parent", "incoming", r.children.clone()),
        ("depends_on", "outgoing", r.depends_on.clone()),
        ("depends_on", "incoming", r.blocks.clone()),
        ("related", "symmetric", r.related.clone()),
        ("discovered_from", "outgoing", r.discovered_from.clone()),
        ("discovered_from", "incoming", r.discovered_by.clone()),
    ] {
        for other in ids {
            if let Ok(neighbor) = v.store.resolve(&other) {
                relations.push(json!({"kind":kind,"direction":direction,"source":if direction=="incoming"{other.as_str()}else{id},"target":if direction=="incoming"{id}else{other.as_str()},"neighbor":{"id":other,"title":neighbor.header.as_ref().unwrap().title,"effective_done":graph.evaluate(&other).ok().map(|e|e.effective_done)}}));
            }
        }
    }
    let mut incoming = Vec::new();
    for (p, h, sid, rid) in v.records() {
        if h["destinations"]
            .as_array()
            .is_some_and(|a| a.iter().any(|d| d["kind"] == "item" && d["item_id"] == id))
        {
            let raw = &v.capture.files[&p].raw;
            let (_, body) = frontmatter(raw)?;
            let body = std::str::from_utf8(body)
                .map_err(|_| Error::new("invalid_format", "handoff body is not UTF-8").at(&p))?;
            incoming.push(json!({"header":h,"body":body,"run_id":rid,"scope_id":sid,"path":path(&p),"diagnostics":[]}));
        }
    }
    Ok(
        json!({"id":id,"display_id":display_id(v,id),"title":h.title,"completion":if h.completion==Completion::Manual{"manual"}else{"children"},"state":h.state.map(|s|if s==ManualState::Open{"open"}else{"done"}),"effective_done":eval.as_ref().map(|e|e.effective_done),"executable":if !v.physical_mode&&v.diagnostics.is_empty()&&v.pending.is_empty(){eval.as_ref().map(|e|e.executable&&blockers.is_empty())}else{None},"blockers":blockers,"priority":h.priority,"parent":h.parent,"depends_on":h.depends_on,"related":h.related,"discovered_from":h.discovered_from,"labels":h.labels,"model":h.model,"thinking":h.thinking,"close_reason":h.close_reason,"body":std::str::from_utf8(f.body.as_ref().unwrap()).unwrap(),"path":path(&f.path),"relations":{"parent":r.parent,"children":r.children,"depends_on":r.depends_on,"blocks":r.blocks,"related":r.related,"discovered_from":r.discovered_from,"discovered_by":r.discovered_by},"graph_diagnostics":graph_diagnostics(v),"recovery_path":recovery.map(path),"context":cx,"relation_context":relations,"current_claim":current,"incoming_handoffs":if !v.physical_mode&&v.diagnostics.is_empty()&&v.pending.is_empty(){Some(incoming)}else{None},"digests":v.physical.iter().filter(|p|v.source_host(id).as_ref()==Some(&p.checkout.root)).flat_map(|p|p.digests.values().filter(|d|d["header"]["root_scope_id"]==id).cloned()).collect::<Vec<_>>(),"context_diagnostics":v.diagnostics}),
    )
}
fn authority_error(v: &View, reference: &str) -> Option<Error> {
    if v.physical_mode || v.diagnostics.is_empty() {
        return None;
    }
    let query = reference.strip_prefix("w-").unwrap_or(reference);
    let mut ancestry = std::collections::BTreeSet::new();
    for p in &v.physical {
        for f in &p.material.files {
            let id = f.header.as_ref().map(|h| h.id.as_str()).or_else(|| {
                f.path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|id| super::coordination::valid_id(id))
            });
            if id.is_some_and(|id| id.starts_with(query)) {
                ancestry.insert(id.unwrap().to_owned());
            }
        }
    }
    if ancestry.is_empty() || ancestry.iter().all(|id| v.store.resolve(id).is_ok()) {
        return None;
    }
    loop {
        let mut parents = std::collections::BTreeSet::new();
        for f in v
            .physical
            .iter()
            .flat_map(|p| &p.material.files)
            .chain(v.store.files.iter())
        {
            if let Some(h) = &f.header
                && ancestry.contains(&h.id)
                && let Some(parent) = &h.parent
            {
                parents.insert(parent.clone());
            }
        }
        let old = ancestry.len();
        ancestry.extend(parents);
        if ancestry.len() == old {
            break;
        }
    }
    let relevant = v
        .diagnostics
        .iter()
        .filter(|d| {
            d["scope_id"]
                .as_str()
                .is_some_and(|sid| ancestry.contains(sid))
        })
        .collect::<Vec<_>>();
    let diagnostic = ["host_ambiguous", "host_missing", "source_unavailable"]
        .into_iter()
        .find_map(|code| relevant.iter().find(|d| d["code"] == code).copied());
    if diagnostic.is_none() && !v.declarations_unresolved() {
        return None;
    }
    let code = match diagnostic.and_then(|d| d["code"].as_str()) {
        Some("host_ambiguous") => "host_ambiguous",
        Some("host_missing") => "host_missing",
        _ => "source_unavailable",
    };
    let sid = diagnostic
        .and_then(|d| d["scope_id"].as_str())
        .or_else(|| {
            ancestry
                .iter()
                .find(|id| v.hosts.contains_key(id.as_str()))
                .map(String::as_str)
        })
        .unwrap_or(query);
    let scope = v.scope(sid);
    let mut error = Error::new(code, "material hosting authority is unresolved");
    error.details = json!({"diagnostics":v.diagnostics,"candidates":scope["candidates"]});
    Some(error)
}
fn catalog_read_error(v: &View) -> Option<Error> {
    // A failed authoritative directory scan cannot prove an item is absent.
    // Do not let an unrelated inactive checkout obscure a genuine not-found.
    let diagnostic = v
        .physical
        .iter()
        .filter(|p| {
            if v.physical_mode {
                p.checkout.root == v.project.worktree_root
            } else {
                v.hosts
                    .values()
                    .flatten()
                    .any(|host| host == &p.checkout.root)
            }
        })
        .flat_map(|p| {
            let catalog = p.checkout.root.join(".work/items");
            let work = p.checkout.root.join(".work");
            p.diagnostics.iter().filter(move |d| {
                d["path"]
                    .as_str()
                    .is_some_and(|path_| path_ == path(&catalog) || path_ == path(&work))
            })
        })
        .next()?;
    let code = match diagnostic["code"].as_str() {
        Some("permission_denied") => "permission_denied",
        Some("unsafe_path") => "unsafe_path",
        Some("conflict") => "conflict",
        Some("io") => "io",
        _ => "source_unavailable",
    };
    let mut error = Error::new(code, "authoritative material catalog could not be read");
    error.path = diagnostic["path"]
        .as_str()
        .and_then(|p| super::coordination::decode_path(p).ok());
    error.details = json!({"diagnostics":v.diagnostics});
    Some(error)
}

pub(crate) fn resolve(v: &View, s: &str) -> Result<String> {
    super::scope_wire::validate("ItemRef", &json!(s))?;
    let query = s.strip_prefix("w-").unwrap_or(s);
    let mut ids = v
        .store
        .files
        .iter()
        .filter_map(|f| {
            f.header.as_ref().map(|h| h.id.clone()).or_else(|| {
                f.path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|id| super::coordination::valid_id(id))
                    .map(str::to_owned)
            })
        })
        .filter(|id| id.starts_with(query))
        .collect::<std::collections::BTreeSet<_>>();
    if super::coordination::valid_id(query) && v.hosts.contains_key(query) {
        ids.insert(query.to_owned());
    }
    let id = match ids.len() {
        1 => ids.into_iter().next().unwrap(),
        0 => {
            return Err(authority_error(v, s)
                .or_else(|| catalog_read_error(v))
                .unwrap_or_else(|| Error::new("not_found", "authoritative item not found")));
        }
        _ => {
            return Err(Error::new(
                "ambiguous_id",
                "ambiguous authoritative item prefix",
            ));
        }
    };
    if id == "root" {
        Err(Error::new("invalid_argument", "root is not an item"))
    } else {
        Ok(id)
    }
}
fn metadata(v: &View, a: &Value) -> Result<MetadataChange> {
    Ok(MetadataChange {
        title: a["title"].as_str().map(str::to_owned),
        completion: a["completion"].as_str().map(|s| {
            if s == "manual" {
                Completion::Manual
            } else {
                Completion::Children
            }
        }),
        priority: a["priority"].as_u64().map(|n| n as u8),
        parent: if let Some(p) = a["parent"].as_str() {
            Some(Some(resolve(v, p)?))
        } else if a["clear_parent"] == true {
            Some(None)
        } else {
            None
        },
        labels: a["labels"]
            .as_array()
            .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect()),
        model: if let Some(s) = a["model"].as_str() {
            Some(Some(s.into()))
        } else if a["clear_model"] == true {
            Some(None)
        } else {
            None
        },
        thinking: if let Some(s) = a["thinking"].as_str() {
            Some(Some(s.into()))
        } else if a["clear_thinking"] == true {
            Some(None)
        } else {
            None
        },
        ..Default::default()
    })
}
pub(crate) fn authorize(v: &View, id: &str, a: &Value) -> Result<()> {
    let current = acquisition(v, id);
    let mut forms = a["authorization"].as_array().cloned().unwrap_or_default();
    if a.get("session").is_some() || a.get("identity").is_some() {
        forms.push(a.clone());
    }
    if forms.len() > 1 {
        return Err(Error::new(
            "invalid_argument",
            "this one-source edit requires exactly its authorization",
        ));
    }
    let Some(owner) = current else {
        if !forms.is_empty() {
            return Err(Error::new("stale_claim", "unused authorization"));
        }
        return Ok(());
    };
    let c = &owner["claim"];
    let Some(auth) = forms.first() else {
        return Err(Error::new(
            "claim_conflict",
            "material source has current ownership",
        ));
    };
    if let Some(source) = auth["item"].as_str()
        && resolve(v, source)? != id
    {
        return Err(Error::new(
            "invalid_argument",
            "authorization does not address authored source",
        ));
    }
    if auth.get("actor").is_some_and(|actor| actor != &c["actor"]) {
        return Err(Error::new(
            "stale_claim",
            "actor does not match acquisition",
        ));
    }
    if auth.get("claim_id").is_some() && auth["claim_id"] != c["id"] {
        return Err(Error::new(
            "stale_claim",
            "claim ID does not fence current acquisition",
        ));
    }
    if let Some(selector) = auth["session"].as_str() {
        let (rid, name) = selector
            .split_once('/')
            .map(|(r, n)| (r.to_owned(), n))
            .unwrap_or_else(|| (v.effective_run(id).0.unwrap_or_default(), selector));
        let named = v
            .records()
            .into_iter()
            .find(|(_, n, _, r)| r == &rid && n["name"] == name);
        if !named.is_some_and(|(_, n, _, _)| {
            c["named_session"]["run_id"] == rid
                && c["named_session"]["session_record_id"] == n["id"]
                && c["actor"] == n["actor"]
                && c["identity"] == n["identity"]
        }) {
            return Err(Error::new(
                "stale_claim",
                "named identity does not own source",
            ));
        }
    } else if auth["identity"] != c["identity"] {
        return Err(Error::new("stale_claim", "identity does not own source"));
    }
    Ok(())
}
fn ordinary_completion_inputs(v: &View, a: &Value) -> Result<()> {
    if a["handoffs"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err(Error::new(
            "invalid_argument",
            "attached completion handoffs are not delivered; use the downstream handoffs implementation",
        ));
    }
    if a.get("session").is_some()
        || a.get("identity").is_some()
        || a.get("claim_id").is_some()
        || a["authorization"].as_array().is_some_and(|a| !a.is_empty())
    {
        return Err(Error::new(
            "invalid_argument",
            "ownership-aware completion/repair is not delivered",
        ));
    }
    if let Some(id) = a["id"].as_str()
        && acquisition(v, id).is_some()
    {
        return Err(Error::new(
            "claim_conflict",
            "ordinary material completion/repair cannot end or mutate current ownership",
        ));
    }
    Ok(())
}
// Material repair and ordinary tree edits must protect the same surviving
// authoritative sources and execution contexts. Inspection's eligibility
// masking must not hide a captured acquisition during repair preflight.
pub(crate) fn validate_context_change(v: &View, prospective: &View) -> Result<()> {
    for f in &v.store.files {
        if let Some(rid) = super::scope_runs::owner(f)
            && super::scope_runs::manifest(v, rid).is_ok_and(|(_, m, _)| m["phase"] == "active")
            && let Some(id) = f.header.as_ref().map(|h| &h.id)
            && prospective.effective_run(id).0.as_deref() != Some(rid)
        {
            return Err(Error::new(
                "wisp_run_mismatch",
                "ancestry would move a current wisp out of its one home",
            ));
        }
    }
    for f in &prospective.store.files {
        if f.path.parent().is_some_and(|p| p.ends_with(".work/items"))
            && let Some(ch) = &f.header
        {
            let expected = prospective.source_host(&ch.id);
            let original = v
                .store
                .files
                .iter()
                .find(|f| f.header.as_ref().is_some_and(|h| h.id == ch.id));
            if original.is_some_and(|f| {
                expected.as_ref().is_some_and(|root| {
                    f.path
                        .ancestors()
                        .find(|p| p.file_name().is_some_and(|n| n == ".work"))
                        .and_then(Path::parent)
                        != Some(root.as_path())
                })
            }) {
                return Err(Error::new(
                    "conflict",
                    "reparenting changes a surviving source host; transfer hosting first",
                ));
            }
        }
    }
    let records = v.records();
    for (p, record, _, rid) in &records {
        if let Some(wisp) = record["wisp_id"].as_str()
            && records.iter().any(|(p, m, _, r)| {
                r == rid && p.file_name().is_some_and(|n| n == "run.yaml") && m["phase"] == "active"
            })
            && prospective.effective_run(wisp).0.as_deref() != Some(rid.as_str())
        {
            return Err(Error::new(
                "wisp_run_mismatch",
                "reparenting changes an active wisp owner context",
            )
            .at(p));
        }
    }
    for f in &prospective.store.files {
        if let Some(ch) = &f.header
            && acquisition(v, &ch.id).is_some()
            && v.effective_run(&ch.id) != prospective.effective_run(&ch.id)
        {
            return Err(Error::new(
                "context_change_claimed",
                "reparenting changes a current claim's context",
            ));
        }
    }
    Ok(())
}
fn repair(project: &super::project::Project, mut v: View, a: &Value) -> Result<Value> {
    ordinary_completion_inputs(&v, a)?;
    let id = a["id"].as_str().unwrap();
    v.capture.recheck()?;
    let material = v
        .raw_material(id)
        .map_err(|e| authority_error(&v, id).unwrap_or(e))?;
    if material.files.is_empty()
        && let Some(e) = authority_error(&v, id).or_else(|| catalog_read_error(&v))
    {
        return Err(e);
    }
    // raw_material already proves that every candidate has one authoritative
    // checkout. Resolve that checkout before selecting canonical/old repair
    // sources, including an interrupted filename repair's two-file pair.
    let host = material
        .files
        .first()
        .map(|f| {
            f.path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        })
        .ok_or_else(|| Error::new("not_found", format!("item {id}")))?;
    let ops = operations::DurableOperations::new(&host);
    let source = ops.raw_source(&material, id)?.clone();
    if source.header.as_ref().is_some_and(|h| h.id != id) {
        return Err(Error::new(
            "invalid_argument",
            "repair cannot reinterpret a parsed immutable ID",
        ));
    }
    operations::source_mode(&source)?;
    let raw = a["raw_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    let target = host.join(format!(".work/items/{id}.md"));
    // A canonical-plus-misnamed pair continues a specific already-published
    // repair. Reject different bytes before preparing either lock namespace;
    // the writer repeats this same guard under the checkout lock.
    if source.path != target
        && let Some(canonical) = material.files.iter().find(|f| f.path == target)
        && canonical.raw != raw
    {
        return Err(Error::new(
            "conflict",
            "retry source differs from the already published canonical item",
        ));
    }
    let candidate = super::items::parse_candidate(target.clone(), raw.clone());
    if !candidate.is_valid() || candidate.header.as_ref().is_none_or(|h| h.id != id) {
        return Err(Error::new(
            "invalid_argument",
            "repair requires valid full source with the same immutable ID",
        ));
    }
    let before = ItemGraph::from_store(&v.store);
    if source.is_valid()
        && !before.is_cycle_member(id)
        && !before.diagnostics().iter().any(|d| d.path == source.path)
    {
        return Err(Error::new(
            "invalid_argument",
            "healthy source is not an explicit repair target",
        ));
    }
    let mut files = v.store.files.clone();
    files.retain(|f| {
        f.path != source.path
            && f.path != target
            && !(f.header.as_ref().is_some_and(|h| h.id == id)
                && f.path.starts_with(host.join(".work/items")))
    });
    files.push(candidate.clone());
    let candidate_store = super::items::ItemStore::from_candidate_files(files);
    let after = ItemGraph::from_store(&candidate_store);
    let new = after
        .diagnostics()
        .iter()
        .filter(|d| !before.diagnostics().contains(d))
        .collect::<Vec<_>>();
    if !new.is_empty() {
        let mut e = Error::new("invalid_candidate", "repair introduces graph diagnostics");
        e.details = json!({"diagnostics":new.iter().map(|d|diagnostic("invalid_candidate",&d.message,Some(&d.path),None)).collect::<Vec<_>>()});
        return Err(e);
    }
    let prospective = super::scope_tree::prospective(&v, &candidate_store, &[]);
    validate_context_change(&v, &prospective)?;
    if prospective.source_host(id).as_ref() != Some(&host) {
        return Err(Error::new(
            "conflict",
            "repair would change material authority; transfer hosting separately",
        ));
    }
    if !super::scope_tree::changes(&mut v, &candidate_store)?.is_empty() {
        return Err(Error::new(
            "conflict",
            "repair would change stored scope references; use the explicit tree operation",
        ));
    }
    let candidate_tree_diagnostics = prospective.material_tree_diagnostics();
    let mut infrastructure = v.clone();
    infrastructure.diagnostics.retain(|d| {
        // Discharge only this target's material-root diagnostic when the
        // candidate actually satisfies the stored tree. Execution/declaration
        // uncertainty, other boundaries and changed references still refuse.
        let restored_material_boundary = d["scope_id"] == id
            && d["message"] == "stored boundary lacks material root"
            && !candidate_tree_diagnostics.contains(d);
        let diagnosed_same_host_candidate = d["scope_id"] == id
            && matches!(
                d["message"].as_str(),
                Some(
                    "conflicting material ancestry candidates"
                        | "cannot resolve whole authoritative material source"
                )
            );
        !restored_material_boundary
            && !diagnosed_same_host_candidate
            && !d["path"].as_str().is_some_and(|p| {
                v.store
                    .files
                    .iter()
                    .any(|f| path(&f.path) == p && !v.is_wisp(f))
            })
    });
    // Explicit repair tolerates existing material graph diagnostics only. It
    // cannot acknowledge away execution, declaration or acquisition damage.
    infrastructure.store = super::items::ItemStore::from_candidate_files(vec![]);
    infrastructure.require_healthy()?;
    if acquisition(&v, id).is_some() {
        return Err(Error::new(
            "claim_conflict",
            "repair source is currently claimed",
        ));
    }
    CheckoutGuard::preflight_repair(&host)?;
    v.capture.recheck()?;
    let guard = coordination(&v, true, true)?.unwrap();
    let locked = View::load(project, false)?;
    v.capture.compare_prelock(&locked.capture)?;
    guard.verify()?;
    drop(CheckoutGuard::open(&host)?);
    let prepared = View::load(project, false)?;
    locked.capture.compare_prelock(&prepared.capture)?;
    let locked = prepared;
    let local = locked
        .physical
        .iter()
        .find(|p| p.checkout.root == host)
        .unwrap()
        .material
        .clone();
    let operation = operations::DurableOperations::new(&host);
    let recheck = || {
        guard
            .verify()
            .map_err(|e| OperationError::Conflict(e.to_string()))?;
        locked
            .capture
            .recheck_except_material(&host)
            .map_err(|e| OperationError::Conflict(e.to_string()))
    };
    let repaired = operation.repair_guarded(
        id,
        raw,
        (Some(&locked.store), Some(&local)),
        recheck,
        recheck,
    )?;
    let result = (|| -> Result<Value> {
        recheck()?;
        let final_view = View::load(project, false)?;
        guard.verify()?;
        final_view.capture.recheck()?;
        recheck()?;
        let source = json!({"id":id,"path":path(&repaired.file.path),"raw_hex":repaired.file.raw.iter().map(|b|format!("{b:02x}")).collect::<String>(),"diagnostics":repaired.file.diagnostics.iter().map(|d|diagnostic("invalid_source",&d.message,Some(&d.path),None)).collect::<Vec<_>>(),"graph_diagnostics":graph_diagnostics(&final_view),"recovery_path":repaired.recovery_path.as_deref().map(path),"additional_recovery_paths":repaired.additional_recovery_paths.iter().map(|p|path(p)).collect::<Vec<_>>()});
        let mut known = partial();
        known["updated"] = json!([{"id":id,"path":path(&target)}]);
        Ok(final_view.wrap(json!({"source":source,"changed":true,"partial":known})))
    })();
    result.map_err(|mut e| {
        e.details["publication"] = json!("published");
        e.details["published_item"] = json!({"id":id,"path":path(&target)});
        let mut known = partial();
        known["updated"] = json!([{"id":id,"path":path(&target)}]);
        e.details["partial"] = known;
        if let Some(p) = repaired.recovery_path {
            e.details["previous_source_path"] = json!(path(&p));
            e.details["recovery_paths"] = json!([path(&p)]);
        }
        e
    })
}

pub fn call(project: &super::project::Project, name: &str, a: &Value) -> Result<Value> {
    let physical = a["view"] == "physical";
    let mutation = matches!(
        name,
        "item_create"
            | "item_update"
            | "item_close"
            | "item_reopen"
            | "item_repair"
            | "relation_add"
            | "relation_remove"
    );
    if mutation && physical {
        return Err(Error::new(
            "invalid_argument",
            "physical-copy mode is read-only",
        ));
    }
    let (mut v, read_guard) = if mutation {
        (View::load(project, physical)?, None)
    } else {
        super::scopes::read_view(project, physical)?
    };
    if let Some(g) = &read_guard {
        g.verify()?;
    }
    if !mutation {
        if !physical
            && v.declarations_unresolved()
            && matches!(name, "item_inspect" | "item_inspect_raw")
        {
            v.require_healthy()?;
        }
        v.diagnostics.extend(graph_diagnostics(&v));
        let out = match name {
            "item_diagnose" => {
                json!({"diagnostics":v.diagnostics})
            }
            "item_inspect_raw" => {
                let id = a["id"].as_str().unwrap();
                let authority = authority_error(&v, id);
                let wisps = v
                    .store
                    .files
                    .iter()
                    .filter(|f| {
                        v.is_wisp(f)
                            && (f.path.file_stem().and_then(|s| s.to_str()) == Some(id)
                                || f.header.as_ref().is_some_and(|h| h.id == id))
                    })
                    .collect::<Vec<_>>();
                let material = v
                    .raw_material(id)
                    .map_err(|error| authority.clone().unwrap_or(error))?;
                if material.files.is_empty()
                    && wisps.is_empty()
                    && let Some(error) = authority.or_else(|| catalog_read_error(&v))
                {
                    return Err(error);
                }
                let f = if wisps.is_empty() {
                    let root = material
                        .files
                        .first()
                        .map(|f| f.path.parent().unwrap().parent().unwrap().parent().unwrap())
                        .unwrap_or(&project.worktree_root);
                    super::operations::DurableOperations::new(root).raw_source(&material, id)?
                } else {
                    if wisps.len() != 1 || !material.files.is_empty() {
                        return Err(Error::new(
                            "invalid_source",
                            "multiple raw sources claim item identity",
                        ));
                    }
                    wisps[0]
                };
                json!({"source":{"id":id,"path":path(&f.path),"raw_hex":f.raw.iter().map(|b|format!("{b:02x}")).collect::<String>(),"diagnostics":f.diagnostics.iter().map(|d|diagnostic("invalid_source",&d.message,Some(&d.path),None)).collect::<Vec<_>>(),"graph_diagnostics":graph_diagnostics(&v),"recovery_path":null,"additional_recovery_paths":[]}})
            }
            "item_inspect" => {
                json!({"item":item(&v,&resolve(&v,a["id"].as_str().unwrap())?,None)?})
            }
            "item_list" | "item_ready" => {
                if name == "item_ready" && !physical {
                    v.require_healthy()?;
                }
                if physical && a.get("run_id").is_some() {
                    return Err(Error::new(
                        "invalid_argument",
                        "physical item selection cannot select a run",
                    ));
                }
                if let Some(rid) = a["run_id"].as_str() {
                    let records = v.records();
                    let manifest = records
                        .iter()
                        .find(|(p, m, _, r)| {
                            r == rid
                                && m["id"] == rid
                                && p.file_name().is_some_and(|n| n == "run.yaml")
                        })
                        .ok_or_else(|| Error::new("not_found", "selected run does not exist"))?;
                    if manifest.1["phase"] != "active"
                        && (name == "item_ready" || a["include_terminal"] != true)
                    {
                        return Err(Error::new(
                            "run_not_current",
                            "selection requires an active run",
                        ));
                    }
                }
                let scope = a["scope"].as_str().map(|s| v.scope_id(s)).transpose()?;
                let root = a["root"].as_str().map(|s| resolve(&v, s)).transpose()?;
                let descends = |id: &str, root: &str| {
                    let mut cursor = Some(id.to_owned());
                    let mut seen = std::collections::BTreeSet::new();
                    while let Some(id) = cursor {
                        if id == root {
                            return true;
                        }
                        if !seen.insert(id.clone()) {
                            break;
                        }
                        cursor = v.store.files.iter().find_map(|f| {
                            f.header
                                .as_ref()
                                .filter(|h| h.id == id)
                                .and_then(|h| h.parent.clone())
                        });
                    }
                    false
                };
                let mut out = Vec::new();
                for f in v.store.files.iter().filter(|f| f.is_valid()) {
                    let h = f.header.as_ref().unwrap();
                    let i = item(&v, &h.id, None)?;
                    if root.as_deref().is_some_and(|r| !descends(&h.id, r))
                        || scope
                            .as_deref()
                            .is_some_and(|s| s != "root" && !descends(&h.id, s))
                        || a["priority_max"]
                            .as_u64()
                            .is_some_and(|n| u64::from(h.priority) > n)
                        || a["labels_all"].as_array().is_some_and(|labels| {
                            labels.iter().any(|l| !h.labels.iter().any(|h| l == h))
                        })
                        || a["persistence"]
                            .as_str()
                            .is_some_and(|p| i["context"]["persistence"] != p)
                        || a["run_id"]
                            .as_str()
                            .is_some_and(|r| i["context"]["effective_run_id"] != r)
                    {
                        continue;
                    }
                    if i["context"]["persistence"] == "wisp"
                        && (v.physical_mode
                            || v.records().iter().any(|(p, m, _, r)| {
                                p.file_name().is_some_and(|n| n == "run.yaml")
                                    && m["phase"] != "active"
                                    && i["context"]["owner_run_id"] == r.as_str()
                            }))
                        && a["include_terminal"] != true
                    {
                        continue;
                    }
                    if name == "item_ready" && i["executable"] != true {
                        continue;
                    }
                    out.push(i);
                }
                out.sort_by(|a, b| {
                    if name == "item_ready" {
                        a["priority"]
                            .as_u64()
                            .cmp(&b["priority"].as_u64())
                            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
                    } else {
                        a["id"].as_str().cmp(&b["id"].as_str())
                    }
                });
                json!({"items":out})
            }
            _ => return Err(Error::new("invalid_argument", "operation not delivered")),
        };
        v.capture.recheck()?;
        let mut wrapped = v.wrap(out.clone());
        if name == "item_diagnose" {
            wrapped["diagnostics"] = out["diagnostics"].clone();
        }
        return Ok(wrapped);
    }
    if name == "item_repair" {
        return repair(project, v, a);
    }
    if matches!(name, "item_close" | "item_reopen") {
        ordinary_completion_inputs(&v, a)?;
    }
    // Planning is initially unlocked. Reject an inconsistent captured catalog
    // before lookup/candidate errors can misreport a cooperating source change.
    v.capture.recheck()?;
    if a.get("continue_id").is_none() {
        v.require_healthy()?;
    }
    let creating = name == "item_create";
    let mut id = if creating {
        new_id()?
    } else {
        resolve(
            &v,
            a[if name.starts_with("relation_") {
                "source"
            } else {
                "id"
            }]
            .as_str()
            .unwrap(),
        )?
    };
    let continuation = a["continue_id"]
        .as_str()
        .map(|c| super::scope_tree::existing(&v, &id, c))
        .transpose()?;
    if let Some((_, plan)) = &continuation
        && (plan["tree_change"]["operation"] != name
            || (name.starts_with("relation_") && a["kind"] != "parent"))
    {
        return Err(Error::new(
            "conflict",
            "continuation operation differs from frozen tree edit",
        ));
    }
    let mut target = None;
    if name.starts_with("relation_") {
        target = Some(resolve(&v, a["target"].as_str().unwrap())?);
        if a["kind"] == "related"
            && name == "relation_remove"
            && !v
                .store
                .resolve(&id)
                .map_err(|_| Error::new("invalid_source", "related author source missing/invalid"))?
                .header
                .as_ref()
                .unwrap()
                .related
                .contains(target.as_ref().unwrap())
        {
            std::mem::swap(&mut id, target.as_mut().unwrap());
        }
    }
    if let Some((_, plan)) = &continuation
        && name.starts_with("relation_")
    {
        let expected = &plan["tree_change"][if name == "relation_remove" {
            "parent_before"
        } else {
            "parent_after"
        }];
        if target.as_deref() != expected.as_str() {
            return Err(Error::new(
                "conflict",
                "continued edge target differs from frozen tree edit",
            ));
        }
    }
    authorize(&v, &id, a)?;
    let old = if creating {
        None
    } else {
        Some(
            v.store
                .resolve(&id)
                .map_err(|_| Error::new("invalid_source", "authored source invalid"))?
                .clone(),
        )
    };
    if let Some(f) = old.as_ref().filter(|f| v.is_wisp(f)) {
        super::scope_runs::require_current_wisp(&v, f)?;
    }
    let mut h = if let Some(f) = &old {
        f.header.clone().unwrap()
    } else {
        operations::default_header(id.clone(), a["title"].as_str().unwrap().into())
    };
    let body = old
        .as_ref()
        .and_then(|f| f.body.clone())
        .unwrap_or_else(|| a["body"].as_str().unwrap_or("").as_bytes().to_vec());
    if name.starts_with("relation_") {
        let other = target.unwrap();
        if other == id {
            return Err(Error::new("invalid_argument", "self relationship"));
        }
        let add = name == "relation_add";
        if a["kind"] == "parent" {
            if add {
                if h.parent.is_some()
                    && !(continuation.is_some() && h.parent.as_deref() == Some(&other))
                {
                    return Err(Error::new("already_exists", "parent already set"));
                }
                h.parent = Some(other);
            } else if h.parent.as_deref() == Some(&other) || continuation.is_some() {
                h.parent = None;
            } else {
                return Err(Error::new("not_found", "parent relation"));
            }
        } else {
            let edges = match a["kind"].as_str().unwrap() {
                "depends_on" => &mut h.depends_on,
                "related" => &mut h.related,
                _ => &mut h.discovered_from,
            };
            if add {
                if edges.contains(&other)
                    || (a["kind"] == "related"
                        && v.store
                            .resolve(&other)
                            .ok()
                            .and_then(|f| f.header.as_ref())
                            .is_some_and(|h| h.related.contains(&id)))
                {
                    return Err(Error::new("already_exists", "relationship already present"));
                }
                edges.push(other);
            } else if let Some(i) = edges.iter().position(|s| s == &other) {
                edges.remove(i);
            } else {
                return Err(Error::new("not_found", "relationship absent"));
            }
        }
    } else if matches!(name, "item_close" | "item_reopen") {
        if h.completion != Completion::Manual {
            return Err(Error::new(
                "invalid_argument",
                "aggregate completion is derived; only manual items can close or reopen",
            ));
        }
        h.state = Some(if name == "item_close" {
            ManualState::Done
        } else {
            ManualState::Open
        });
        h.close_reason = if name == "item_close" {
            a["reason"].as_str().map(str::to_owned)
        } else {
            None
        };
    } else {
        h = operations::apply_change(h, metadata(&v, a)?);
    }
    let host = if creating {
        let sid = v.nearest(h.parent.as_deref());
        v.hosts[&sid].clone().unwrap()
    } else {
        v.source_host(&id).unwrap()
    };
    let file_path = old.as_ref().filter(|f| v.is_wisp(f)).map_or_else(
        || host.join(format!(".work/items/{id}.md")),
        |f| f.path.clone(),
    );
    let raw = operations::serialize(&h, &body);
    let candidate = operations::candidate_store(&v.store, &file_path, raw.clone())?;
    let graph = ItemGraph::from_store(&candidate);
    if !graph.diagnostics().is_empty() {
        let mut e = Error::new("invalid_candidate", "prospective graph invalid");
        e.details = json!({"diagnostics":graph.diagnostics().iter().map(|d|diagnostic("invalid_candidate",&d.message,Some(&d.path),None)).collect::<Vec<_>>()});
        return Err(e);
    }
    let before_parent = old
        .as_ref()
        .and_then(|f| f.header.as_ref())
        .and_then(|h| h.parent.clone());
    let tree_references = super::scope_tree::changes(&mut v, &candidate)?;
    if before_parent != h.parent {
        let prospective = super::scope_tree::prospective(&v, &candidate, &tree_references);
        validate_context_change(&v, &prospective)?;
    }
    v.capture.recheck()?;
    let guard = coordination(&v, true, true)?.unwrap();
    let mut locked = View::load(project, false)?;
    v.capture.compare_prelock(&locked.capture)?;
    guard.verify()?;
    if old.as_ref().is_some_and(|f| v.is_wisp(f)) {
        return super::scope_runs::edit_wisp(project, &locked, old.as_ref().unwrap(), &raw, name);
    }
    if !tree_references.is_empty() || continuation.is_some() {
        return super::scope_tree::publish(
            &locked,
            old.as_ref().unwrap(),
            &raw,
            tree_references,
            name,
            continuation,
        );
    }
    locked.require_healthy()?;
    if creating {
        locked.capture.ensure(&host.join(".work/items"))?;
    }
    let mut writer = CheckoutGuard::open(&host)?.into_writer()?;
    let current = View::load(project, false)?;
    locked.capture.compare_prelock(&current.capture)?;
    guard.verify()?;
    writer.verify()?;
    current.capture.recheck()?;
    let changed = old.as_ref().is_none_or(|f| f.raw != raw);
    let recovery = if changed {
        writer.publish_raw_guarded(&id, &raw, old.as_ref(), || {
            current
                .capture
                .recheck()
                .map_err(|e| OperationError::Conflict(e.to_string()))
        })?
    } else {
        None
    };
    let recheck = || -> Result<()> {
        guard.verify()?;
        current.capture.recheck_except_material(&host)?;
        writer.verify()?;
        Ok(())
    };
    let result = (|| -> Result<Value> {
        recheck()?;
        let final_view = View::load(project, false)?;
        final_view.require_healthy()?;
        final_view.capture.recheck()?;
        recheck()?;
        let mut inspected = item(&final_view, &id, recovery.as_deref())?;
        let mut surviving = recovery.clone();
        if matches!(name, "item_close" | "item_reopen")
            && let Some(p) = surviving.as_ref()
        {
            writer.cleanup_displaced(p, old.as_ref().unwrap(), || {
                recheck().map_err(|e| OperationError::Conflict(e.to_string()))
            })?;
            surviving = None;
        }
        recheck()?;
        final_view.capture.recheck()?;
        inspected["recovery_path"] = json!(surviving.as_deref().map(path));
        let mut out = json!({"item":inspected,"changed":changed,"partial":partial()});
        if name == "item_close" {
            out["saved_handoffs"] = json!([]);
            out["ending"] = Value::Null;
        }
        Ok(final_view.wrap(out))
    })();
    result.map_err(|mut e| {
        if changed {
            let entry = json!({"id":id,"path":path(&file_path)});
            let mut known = partial();
            known[if creating { "created" } else { "updated" }] = json!([entry.clone()]);
            e.details["partial"] = known;
            e.details["published_item"] = entry;
            e.details["publication"] = json!("published");
            if let Some(p) = &recovery
                && std::fs::symlink_metadata(p).is_ok()
            {
                e.details["previous_source_path"] = json!(path(p));
                e.details["recovery_paths"] = json!([path(p)]);
            }
        }
        e
    })
}

#[cfg(test)]
mod publication_fence_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn catalog_change_during_initial_capture_is_conflict_not_false_absence() {
        let fixture =
            Fixture(std::env::temp_dir().join(format!("work-catalog-fence-{}", new_id().unwrap())));
        fs::create_dir_all(&fixture.0).unwrap();
        git(&fixture.0, &["init", "--initial-branch=main"]);
        let project = super::super::project::discover(Some(&fixture.0)).unwrap();
        let id = call(&project, "item_create", &json!({"title":"captured source"})).unwrap()["item"]["id"].as_str().unwrap().to_owned();
        let source = fixture.0.join(format!(".work/items/{id}.md"));
        let retained = fixture.0.join("externally-retained.md");
        let before = fs::read(&source).unwrap();
        let source_hook = source.clone();
        let retained_hook = retained.clone();
        let hook = super::super::storage::files::on_next("scope_before_material_load", move || {
            fs::rename(source_hook, retained_hook).unwrap();
        });
        let error = call(
            &project,
            "item_update",
            &json!({"id":id,"title":"must not publish"}),
        )
        .unwrap_err();
        drop(hook);
        assert_eq!(error.code, "conflict");
        assert!(error.details.get("published_item").is_none());
        assert!(!source.exists());
        assert_eq!(fs::read(retained).unwrap(), before);
    }

    #[test]
    fn material_loader_must_cover_the_captured_catalog_even_if_names_are_restored() {
        let fixture =
            Fixture(std::env::temp_dir().join(format!("work-catalog-set-{}", new_id().unwrap())));
        fs::create_dir_all(&fixture.0).unwrap();
        git(&fixture.0, &["init", "--initial-branch=main"]);
        let project = super::super::project::discover(Some(&fixture.0)).unwrap();
        let id = call(
            &project,
            "item_create",
            &json!({"title":"must remain present"}),
        )
        .unwrap()["item"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let items = fixture.0.join(".work/items");
        let retained = fixture.0.join("held-items");
        let source = items.join(format!("{id}.md"));
        let before = fs::read(&source).unwrap();
        let hook = super::super::storage::files::on_next("scope_before_material_load", move || {
            fs::rename(&items, &retained).unwrap();
            fs::create_dir(&items).unwrap();
            let restore =
                super::super::storage::files::on_next("scope_after_material_load", move || {
                    fs::remove_dir(&items).unwrap();
                    fs::rename(&retained, &items).unwrap();
                });
            // The outer guard owns cleanup; consume this action at the next hook.
            std::mem::forget(restore);
        });
        let error = call(
            &project,
            "item_update",
            &json!({"id":id,"title":"must not publish"}),
        )
        .unwrap_err();
        drop(hook);
        assert_eq!(error.code, "conflict");
        assert!(error.details.get("published_item").is_none());
        assert_eq!(fs::read(source).unwrap(), before);
    }

    #[test]
    fn remote_authoritative_change_during_staging_refuses_material_publication() {
        let fixture =
            Fixture(std::env::temp_dir().join(format!("work-hosting-fence-{}", new_id().unwrap())));
        let primary = fixture.0.join("primary");
        fs::create_dir_all(&primary).unwrap();
        git(&primary, &["init", "--initial-branch=main"]);
        git(
            &primary,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "seed",
            ],
        );
        let project = super::super::project::discover(Some(&primary)).unwrap();
        let a = call(&project, "item_create", &json!({"title":"primary"})).unwrap()["item"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let b = call(&project, "item_create", &json!({"title":"remote"})).unwrap()["item"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        git(&primary, &["add", ".work/items"]);
        git(
            &primary,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "items",
            ],
        );
        let linked = fixture.0.join("linked");
        git(
            &primary,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        let ops = super::super::scopes::ScopeOperations::new(project.clone(), false);
        let inventory = ops.read(Some(&b)).unwrap()["scope"]["inventory_sha256"].clone();
        ops.host(
            "host",
            &json!({"scope":b,"destination":path(&linked),"expected_inventory":inventory}),
        )
        .unwrap();
        let target = primary.join(format!(".work/items/{a}.md"));
        let before = fs::read(&target).unwrap();
        let other = linked.join(format!(".work/items/{b}.md"));
        let external_bytes = [
            fs::read(&other).unwrap(),
            b"external uncommitted body".to_vec(),
        ]
        .concat();
        let replacement = external_bytes.clone();
        let other_path = other.clone();
        let hook = super::super::storage::files::on_next("checkout_after_stage", move || {
            fs::write(other_path, replacement).unwrap();
        });
        let error = call(
            &project,
            "item_update",
            &json!({"id":a,"title":"must not publish"}),
        )
        .unwrap_err();
        drop(hook);
        assert_eq!(error.code, "conflict");
        assert!(error.details.get("published_item").is_none());
        assert_eq!(fs::read(target).unwrap(), before);
        assert_eq!(fs::read(other).unwrap(), external_bytes);
        assert!(
            !fs::read_dir(primary.join(".work/items")).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".operation-"))
        );
    }
}

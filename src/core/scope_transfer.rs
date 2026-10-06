//! Bounded hosting establishment only; remediation is downstream.
use super::coordination::{
    ExecutionError as Error, ExecutionResult as Result, decode_path, new_id, now_timestamp,
    yaml_bytes,
};
use super::operations::CheckoutGuard;
use super::scope_wire::{self as wire, hash};
use super::scopes::{Capture, ScopeOperations, View, coordination, partial, path};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
type PublicationStep = (PathBuf, Value, Option<Vec<u8>>);

fn snapshot(c: &Capture, p: &Path) -> Value {
    match c.files.get(p) {
        Some(s) => json!({"exists":true,"sha256":hash(&s.raw)}),
        None => json!({"exists":false}),
    }
}
fn matches(c: &Capture, p: &Path, mark: &Value) -> bool {
    snapshot(c, p) == *mark
}
fn boundary(
    sid: &str,
    checkout: &Path,
    parent: Option<&str>,
    anchor: Value,
    c: &Capture,
    parent_host: Option<&Path>,
) -> Value {
    let mut b = json!({"scope_id":sid,"checkout":path(checkout),"anchor_before":snapshot(c,&checkout.join(format!(".work/scopes/{sid}/scope.yaml"))),"anchor_after":anchor});
    if let Some(parent) = parent {
        b["parent_scope_id"] = json!(parent);
        b["child_before"] = snapshot(
            c,
            &parent_host
                .unwrap()
                .join(format!(".work/scopes/{parent}/children/{sid}.yaml")),
        );
    }
    b
}
fn claims(v: &View, inventory: &Value) -> bool {
    let affected = inventory["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| {
            f["path"]
                .as_str()
                .and_then(|p| p.strip_prefix(".work/items/"))
                .and_then(|p| p.strip_suffix(".md"))
        })
        .collect::<Vec<_>>();
    let records = v.records();
    let inherited = records.iter().any(|(p, c, _, _)| {
        c["item_id"]
            .as_str()
            .is_some_and(|id| affected.contains(&id))
            && !records
                .iter()
                .any(|(e, end, _, _)| e.parent() == p.parent() && end["claim_id"] == c["id"])
    });
    inherited
        || inventory["files"].as_array().unwrap().iter().any(|f| {
            let rel = f["path"].as_str().unwrap();
            if !rel.contains("/claims/") || rel.ends_with(".end.yaml") {
                return false;
            }
            let ending = rel.strip_suffix(".yaml").unwrap().to_owned() + ".end.yaml";
            !inventory["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["path"] == ending)
        })
        || v.diagnostics.iter().any(|d| d["code"] == "claim_conflict")
}
fn compare(v: &View, inventory: &Value, host: &Path) -> Result<()> {
    for f in inventory["files"].as_array().unwrap() {
        let p = host.join(f["path"].as_str().unwrap());
        let Some(s) = v.capture.files.get(&p) else {
            return Err(Error::new(
                "source_unavailable",
                "required source/copy is unavailable; recover it before comparison",
            )
            .at(p));
        };
        if hash(&s.raw) != f["sha256"] {
            return Err(Error::new(
                "conflict",
                "copy differs from inspected authoritative inventory",
            )
            .at(p));
        }
    }
    Ok(())
}
fn compare_declarations(v: &View, plan: &Value) -> Result<()> {
    if v.declarations_unresolved() {
        return Err(Error::new(
            "source_unavailable",
            "hosting declaration authority is unresolved",
        ));
    }
    let affected = plan["inventory"]["scope_ids"].as_array().unwrap();
    for p in &v.physical {
        let Some(git) = &p.checkout.git_dir else {
            continue;
        };
        let target = git.join("work-beta2-hosts.yaml");
        let Some(current) = v.capture.files.get(&target) else {
            continue;
        };
        // Initial planning may already project declaration fields in memory.
        // Candidate membership must come from captured physical bytes instead.
        let declaration = wire::envelope("HostDeclaration", &current.raw, &target)?;
        if declaration["scope_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| affected.contains(id))
        {
            let change = plan["declarations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["checkout"] == path(&p.checkout.root))
                .ok_or_else(|| {
                    Error::new(
                        "conflict",
                        "new declaration candidate outside frozen hosting plan",
                    )
                })?;
            let before = change["before"]["exists"] == true
                && change["before"]["sha256"] == hash(&current.raw);
            let after = current.raw == yaml_bytes(&change["after"]);
            if !before && !after {
                return Err(Error::new(
                    "conflict",
                    "hosting declaration differs from frozen candidate",
                )
                .at(&target));
            }
        }
    }
    Ok(())
}
fn compare_inventory(
    v: &View,
    plan: &Value,
    host: &Path,
    publications: &[(PathBuf, Value, Option<Vec<u8>>)],
) -> Result<()> {
    compare(v, &plan["inventory"], host)?;
    let actual = v.inventory(plan["scope_id"].as_str().unwrap(), host)?;
    if actual["scope_ids"] != plan["inventory"]["scope_ids"] {
        return Err(Error::new("conflict", "continuation scope set changed").at(host));
    }
    let mut files = actual["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["path"].as_str().unwrap().to_owned(), m["sha256"].clone()))
        .collect::<BTreeMap<_, _>>();
    let expected = plan["inventory"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["path"].as_str().unwrap().to_owned(), m["sha256"].clone()))
        .collect::<BTreeMap<_, _>>();
    for (p, before, _) in publications {
        if let Ok(relative) = p.strip_prefix(host)
            && let Some(relative) = relative.to_str()
            && (files.contains_key(relative) || expected.contains_key(relative))
        {
            if before["exists"] == true {
                files.insert(relative.into(), before["sha256"].clone());
            } else {
                files.remove(relative);
            }
        }
    }
    if files != expected {
        return Err(Error::new(
            "conflict",
            "continuation file set widened or differs from frozen inventory",
        )
        .at(host));
    }
    Ok(())
}
fn plan(v: &mut View, input: &Value, mode: &str, sid: &str, destination: &Path) -> Result<Value> {
    if !v.pending.is_empty() {
        return Err(Error::new(
            "transition_pending",
            "use the exact pending continuation",
        ));
    }
    let declared = v
        .declared(sid)
        .iter()
        .map(|p| p.checkout.root.clone())
        .collect::<Vec<_>>();
    v.require_healthy()?;
    if !declared.is_empty() && v.source_host(sid).as_deref() != Some(destination) {
        return Err(Error::new(
            "invalid_argument",
            "scope already has a dedicated host; relocation is downstream",
        ));
    }
    let sources = vec![
        v.source_host(sid)
            .ok_or_else(|| Error::new("host_missing", "authoritative source missing"))?,
    ];
    // All candidates, not just the chosen one, must remain readable/comparable.
    let inventory = v.inventory(sid, &sources[0])?;
    if inventory["sha256"] != input["expected_inventory"] {
        return Err(Error::new("conflict", "inspected inventory changed"));
    }
    for source in &sources {
        let actual = v.inventory(sid, source)?;
        if actual != inventory {
            return Err(Error::new("conflict", "declaration candidates differ"));
        }
        compare(v, &inventory, source)?;
    }
    let destination_inventory = v.inventory(sid, destination)?;
    if destination_inventory != inventory {
        return Err(Error::new(
            "conflict",
            "destination subtree differs; externally prepare an identical copy",
        ));
    }
    compare(v, &inventory, destination)?;
    if claims(v, &inventory) && input["executors_quiescent"] != true {
        return Err(Error::new(
            "claims_present",
            "affected executors must be quiescent",
        ));
    }
    let mut boundaries = Vec::new();
    let original_declarations = v
        .physical
        .iter()
        .map(|p| (p.checkout.root.clone(), p.declaration.clone()))
        .collect::<BTreeMap<_, _>>();
    let root_file = v
        .store
        .resolve(sid)
        .map_err(|_| Error::new("invalid_source", "boundary root must be material and valid"))?;
    let parent = v.nearest(root_file.header.as_ref().unwrap().parent.as_deref());
    let parent_host = v
        .hosts
        .get(&parent)
        .and_then(|h| h.as_ref())
        .ok_or_else(|| Error::new("host_missing", "parent host unavailable"))?
        .clone();
    if !v.anchors.contains_key("root") {
        boundaries.push(boundary(
            "root",
            &v.physical[0].checkout.root,
            None,
            json!({"format_version":2,"scope_id":"root","execution":{"mode":"uninitialized"}}),
            &v.capture,
            None,
        ));
    }
    if !v.anchors.contains_key(sid) {
        boundaries.push(boundary(
            sid,
            destination,
            Some(&parent),
            json!({"format_version":2,"scope_id":sid,"execution":{"mode":"inherit"}}),
            &v.capture,
            Some(&parent_host),
        ));
    }
    let mut tree_references = Vec::new();
    if !v.anchors.contains_key(sid) {
        for (child, old_parent) in &v.parents {
            if child == sid || old_parent != &parent {
                continue;
            }
            let mut cursor = Some(child.clone());
            let mut descendant = false;
            let mut seen = std::collections::BTreeSet::new();
            while let Some(id) = cursor {
                if !seen.insert(id.clone()) {
                    break;
                }
                if id == sid {
                    descendant = true;
                    break;
                }
                cursor = v.store.files.iter().find_map(|f| {
                    f.header
                        .as_ref()
                        .filter(|h| h.id == id)
                        .and_then(|h| h.parent.clone())
                });
            }
            if !descendant {
                continue;
            }
            let new_path = destination.join(format!(".work/scopes/{sid}/children/{child}.yaml"));
            let old_path = parent_host.join(format!(".work/scopes/{parent}/children/{child}.yaml"));
            tree_references.push(json!({"checkout":path(destination),"parent_scope_id":sid,"child_scope_id":child,"before":snapshot(&v.capture,&new_path),"after":{"format_version":2,"child_scope_id":child}}));
            tree_references.push(json!({"checkout":path(&parent_host),"parent_scope_id":parent,"child_scope_id":child,"before":snapshot(&v.capture,&old_path)}));
        }
    }
    let mut declarations = Vec::new();
    let destination_physical = v
        .physical
        .iter()
        .find(|p| p.checkout.root == destination)
        .unwrap();
    let mut destination_ids = original_declarations[destination]
        .as_ref()
        .map(|d| d["scope_ids"].as_array().unwrap().clone())
        .unwrap_or_default();
    if !destination_ids.iter().any(|v| v == sid) {
        destination_ids.push(json!(sid));
    }
    destination_ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    declarations.push(json!({"checkout":path(destination),"before":snapshot(&v.capture,&destination_physical.checkout.git_dir.as_ref().unwrap().join("work-beta2-hosts.yaml")),"before_scope_ids":original_declarations[destination].as_ref().map(|d|d["scope_ids"].clone()).unwrap_or_else(||json!([])),"after":{"format_version":2,"scope_ids":destination_ids}}));
    for source in &sources {
        if source == destination {
            continue;
        }
        let p = v
            .physical
            .iter()
            .find(|p| &p.checkout.root == source)
            .unwrap();
        let mut ids = original_declarations[source]
            .as_ref()
            .map(|d| d["scope_ids"].as_array().unwrap().clone())
            .unwrap_or_default();
        if !ids.iter().any(|id| id == sid) {
            continue;
        }
        ids.retain(|id| id != sid);
        ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        declarations.push(json!({"checkout":path(source),"before":snapshot(&v.capture,&p.checkout.git_dir.as_ref().unwrap().join("work-beta2-hosts.yaml")),"before_scope_ids":original_declarations[source].as_ref().map(|d|d["scope_ids"].clone()).unwrap_or_else(||json!([])),"after":{"format_version":2,"scope_ids":ids}}));
    }
    let p = json!({"format_version":2,"id":new_id()?,"created_at":now_timestamp(),"scope_id":sid,"mode":mode,"source_hosts":sources.iter().map(|s|path(s)).collect::<Vec<_>>(),"destination":path(destination),"inventory":inventory,"declarations":declarations,"new_boundaries":boundaries,"tree_references":tree_references,"executors_quiescent":input["executors_quiescent"]==true,"step":0});
    wire::validate("HostTransferProgress", &p)?;
    v.select_prospective(sid, destination, p["new_boundaries"].as_array().unwrap());
    for change in p["tree_references"].as_array().unwrap() {
        let checkout = decode_path(change["checkout"].as_str().unwrap())?;
        if let Some(physical) = v.physical.iter_mut().find(|c| c.checkout.root == checkout) {
            let parent = change["parent_scope_id"].as_str().unwrap();
            let child = change["child_scope_id"].as_str().unwrap();
            let children = physical.children.entry(parent.into()).or_default();
            children.retain(|c| c != child);
            if change.get("after").is_some() {
                children.push(child.into());
            }
        }
    }
    v.select_prospective(sid, destination, &[]);
    v.require_healthy()?;
    Ok(p)
}
fn validate_publication_hosts(v: &View, plan: &Value) -> Result<()> {
    if plan["inventory"]["sha256"] != wire::inventory_hash(&plan["inventory"]) {
        return Err(Error::new(
            "conflict",
            "frozen inventory digest differs from its scope IDs and file marks",
        ));
    }
    let allowed = |value: &Value| -> Result<PathBuf> {
        let root = decode_path(
            value
                .as_str()
                .ok_or_else(|| Error::new("invalid_format", "publication checkout missing"))?,
        )?;
        if !v
            .physical
            .iter()
            .any(|p| p.checkout.root == root && p.available && p.checkout.available)
        {
            return Err(Error::new(
                "source_unavailable",
                "frozen publication checkout is not an available inventoried Git root",
            )
            .at(root));
        }
        Ok(root)
    };
    let sid = plan["scope_id"].as_str().unwrap();
    let destination = allowed(&plan["destination"])?;
    let mut sources = plan["source_hosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(allowed)
        .collect::<Result<Vec<_>>>()?;
    if sources.len() != 1 {
        return Err(Error::new(
            "invalid_source",
            "boundary establishment has exactly one authoritative source",
        ));
    }
    sources.push(destination.clone());
    let boundaries = plan["new_boundaries"].as_array().unwrap();
    let mut ids = std::collections::BTreeSet::new();
    for boundary in boundaries {
        let host = allowed(&boundary["checkout"])?;
        let id = boundary["scope_id"].as_str().unwrap();
        if !ids.insert(id)
            || boundary["anchor_after"]["scope_id"] != id
            || !(id == "root"
                && host == v.physical[0].checkout.root
                && boundary.get("parent_scope_id").is_none()
                || id == sid
                    && host == destination
                    && boundary["parent_scope_id"].as_str().is_some())
        {
            return Err(Error::new(
                "invalid_source",
                "boundary identity or host exceeds frozen hosting intent",
            ));
        }
        let execution = if id == "root" {
            "uninitialized"
        } else {
            "inherit"
        };
        if boundary["anchor_after"]["execution"]["mode"] != execution
            || boundary["anchor_before"]["exists"] != false
            || (id != "root" && boundary["child_before"]["exists"] != false)
        {
            return Err(Error::new(
                "invalid_source",
                "hosting creates only new inheriting boundaries or an uninitialized root; existing execution is preserved",
            ));
        }
    }
    let declarations = plan["declarations"].as_array().unwrap();
    if declarations
        .iter()
        .filter(|d| d["checkout"] == path(&destination))
        .count()
        != 1
    {
        return Err(Error::new(
            "invalid_source",
            "frozen hosting plan requires exactly one destination declaration",
        ));
    }
    let mut declaration_hosts = std::collections::BTreeSet::new();
    for declaration in declarations {
        let host = allowed(&declaration["checkout"])?;
        if !declaration_hosts.insert(host.clone()) {
            return Err(Error::new(
                "invalid_source",
                "duplicate frozen declaration publication checkout",
            ));
        }
        if !sources.contains(&host) {
            return Err(Error::new(
                "invalid_source",
                "declaration checkout exceeds frozen hosting intent",
            ));
        }
        let membership = |value: &Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = membership(&declaration["before_scope_ids"]);
        if host != destination && before.contains(sid) {
            return Err(Error::new(
                "invalid_source",
                "hosting continuation cannot relocate an existing dedicated boundary",
            ));
        }
        if declaration["before"]["exists"] == false && !before.is_empty() {
            return Err(Error::new(
                "invalid_source",
                "absent declaration has nonempty frozen before membership",
            ));
        }
        let mut expected = before.clone();
        if host == destination {
            expected.insert(sid.to_owned());
        } else {
            expected.remove(sid);
        }
        if membership(&declaration["after"]["scope_ids"]) != expected {
            return Err(Error::new(
                "invalid_source",
                "declaration delta changes scopes outside frozen hosting intent",
            ));
        }
        let metadata = v
            .physical
            .iter()
            .find(|p| p.checkout.root == host)
            .unwrap()
            .checkout
            .git_dir
            .as_ref()
            .unwrap()
            .join("work-beta2-hosts.yaml");
        if matches(&v.capture, &metadata, &declaration["before"])
            && let Some(source) = v.capture.files.get(&metadata)
        {
            let actual = wire::envelope("HostDeclaration", &source.raw, &metadata)?;
            if membership(&actual["scope_ids"]) != before {
                return Err(Error::new(
                    "invalid_source",
                    "frozen declaration membership differs from before bytes",
                ));
            }
        }
    }
    for reference in plan["tree_references"].as_array().unwrap() {
        let host = allowed(&reference["checkout"])?;
        let boundary = boundaries
            .iter()
            .find(|b| b["scope_id"] == sid)
            .ok_or_else(|| {
                Error::new(
                    "invalid_source",
                    "tree reference has no new hosting boundary",
                )
            })?;
        let parent = boundary["parent_scope_id"].as_str().unwrap();
        let parent_host = boundaries
            .iter()
            .find(|b| b["scope_id"] == parent)
            .map(|b| allowed(&b["checkout"]))
            .transpose()?
            .or_else(|| v.hosts.get(parent).cloned().flatten())
            .ok_or_else(|| Error::new("host_missing", "frozen parent host unavailable"))?;
        let valid = if let Some(after) = reference.get("after") {
            reference["parent_scope_id"] == sid
                && host == destination
                && after["child_scope_id"] == reference["child_scope_id"]
        } else {
            reference["parent_scope_id"] == parent && host == parent_host
        };
        if !valid {
            return Err(Error::new(
                "invalid_source",
                "tree reference identity or host exceeds frozen hosting intent",
            ));
        }
    }
    Ok(())
}
fn steps(v: &View, plan: &Value) -> Result<Vec<PublicationStep>> {
    validate_publication_hosts(v, plan)?;
    let mut out = Vec::new();
    for b in plan["new_boundaries"].as_array().unwrap() {
        let sid = b["scope_id"].as_str().unwrap();
        let host = decode_path(b["checkout"].as_str().unwrap())?;
        out.push((
            host.join(format!(".work/scopes/{sid}/scope.yaml")),
            b["anchor_before"].clone(),
            Some(yaml_bytes(&b["anchor_after"])),
        ));
        if let Some(parent) = b["parent_scope_id"].as_str() {
            let parent_host = plan["new_boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["scope_id"] == parent)
                .map(|b| decode_path(b["checkout"].as_str().unwrap()))
                .transpose()?
                .or_else(|| v.hosts.get(parent).cloned().flatten())
                .ok_or_else(|| Error::new("host_missing", "parent source unavailable"))?;
            out.push((
                parent_host.join(format!(".work/scopes/{parent}/children/{sid}.yaml")),
                b["child_before"].clone(),
                Some(yaml_bytes(
                    &json!({"format_version":2,"child_scope_id":sid}),
                )),
            ));
        }
    }
    for change in plan["tree_references"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("after").is_some())
    {
        let host = decode_path(change["checkout"].as_str().unwrap())?;
        out.push((
            host.join(format!(
                ".work/scopes/{}/children/{}.yaml",
                change["parent_scope_id"].as_str().unwrap(),
                change["child_scope_id"].as_str().unwrap()
            )),
            change["before"].clone(),
            Some(yaml_bytes(&change["after"])),
        ));
    }
    for d in plan["declarations"].as_array().unwrap() {
        let host = decode_path(d["checkout"].as_str().unwrap())?;
        let git = v
            .physical
            .iter()
            .find(|p| p.checkout.root == host)
            .and_then(|p| p.checkout.git_dir.as_ref())
            .ok_or_else(|| Error::new("source_unavailable", "declaration metadata missing"))?;
        out.push((
            git.join("work-beta2-hosts.yaml"),
            d["before"].clone(),
            Some(yaml_bytes(&d["after"])),
        ));
    }
    for change in plan["tree_references"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("after").is_none())
    {
        let host = decode_path(change["checkout"].as_str().unwrap())?;
        out.push((
            host.join(format!(
                ".work/scopes/{}/children/{}.yaml",
                change["parent_scope_id"].as_str().unwrap(),
                change["child_scope_id"].as_str().unwrap()
            )),
            change["before"].clone(),
            None,
        ));
    }
    Ok(out)
}
pub(crate) fn transfer(ops: &ScopeOperations, input: &Value, mode: &str) -> Result<Value> {
    if ops.physical || input["view"] == "physical" {
        return Err(Error::new(
            "invalid_argument",
            "physical views cannot mutate authority",
        ));
    }
    if mode != "host" {
        return Err(Error::new(
            "invalid_argument",
            "hosting remediation is downstream",
        ));
    }
    wire::validate("ScopeHostInput", input)?;
    let mut v = View::load(&ops.project, false)?;
    let sid = v.scope_id(input["scope"].as_str().unwrap())?;
    if sid == "root" {
        return Err(Error::new("invalid_argument", "root hosting is a Git fact"));
    }
    if v.store.resolve(&sid).is_ok_and(|f| v.is_wisp(f)) {
        return Err(Error::new(
            "invalid_argument",
            "only material items may establish hosting boundaries; promote the wisp first",
        ));
    }
    let destination = decode_path(input["destination"].as_str().unwrap())?
        .canonicalize()
        .map_err(|e| {
            Error::new(
                "invalid_argument",
                format!("cannot resolve destination: {e}"),
            )
        })?;
    if !v
        .physical
        .iter()
        .any(|p| p.checkout.root == destination && p.available && p.checkout.git_dir.is_some())
    {
        return Err(Error::new(
            "invalid_argument",
            "destination must be an available Git-inventoried checkout",
        ));
    }
    if mode == "host"
        && input.get("continue_id").is_none()
        && v.anchors.contains_key(&sid)
        && v.declared(&sid).len() == 1
        && v.source_host(&sid).as_ref() == Some(&destination)
    {
        v.require_healthy()?;
        let inventory = v.inventory(&sid, &destination)?;
        if inventory["sha256"] != input["expected_inventory"] {
            return Err(Error::new("conflict", "inspected inventory changed"));
        }
        let guard = coordination(&v, false, false)?;
        v.capture.recheck()?;
        if let Some(g) = guard {
            g.verify()?;
        }
        return Ok(v.wrap(json!({"scope":v.scope(&sid),"source_hosts":[path(&destination)],"destination":path(&destination),"inventory_sha256":inventory["sha256"],"changed":false,"partial":partial()})));
    }
    let progress_dir = ops.project.git_common_dir.join("work-beta2/relocations");
    let continuation = input["continue_id"].as_str();
    let mut plan = if let Some(id) = continuation {
        let file = progress_dir.join(format!("{id}.yaml"));
        let source = v
            .capture
            .files
            .get(&file)
            .ok_or_else(|| Error::new("not_found", "hosting continuation not found"))?;
        let p = wire::envelope("HostTransferProgress", &source.raw, &file)?;
        if p["mode"] != mode
            || p["scope_id"] != sid
            || p["destination"] != path(&destination)
            || p["inventory"]["sha256"] != input["expected_inventory"]
        {
            return Err(Error::new(
                "conflict",
                "continuation inputs differ from frozen plan",
            ));
        }
        p
    } else {
        plan(&mut v, input, mode, &sid, &destination).map_err(|mut e| {
            e.details["candidates"] = v.scope(&sid)["candidates"].clone();
            e
        })?
    };
    let sources: Vec<PathBuf> = plan["source_hosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| decode_path(p.as_str().unwrap()))
        .collect::<Result<_>>()?;
    compare_declarations(&v, &plan)?;
    let prelock_steps = steps(&v, &plan)?;
    for source in &sources {
        compare_inventory(&v, &plan, source, &prelock_steps)?;
    }
    compare_inventory(&v, &plan, &destination, &prelock_steps)?;
    if claims(&v, &plan["inventory"]) && input["executors_quiescent"] != true {
        return Err(Error::new(
            "claims_present",
            "repeat quiescence attestation for current claims",
        ));
    }
    v.capture.recheck()?;
    // Guard/candidate rejection above creates neither locks nor intent files.
    let guard = coordination(&v, true, true)?.unwrap();
    let mut locked = View::load(&ops.project, false)?;
    // Preserve the pre-lock whole source/declaration fingerprints, ignoring only
    // this attempt's empty coordination bootstrap and canonical lock directory.
    v.capture.compare_prelock(&locked.capture)?;
    compare_declarations(&locked, &plan)?;
    let planned_steps = steps(&locked, &plan)?;
    for source in &sources {
        compare_inventory(&locked, &plan, source, &planned_steps)?;
    }
    compare_inventory(&locked, &plan, &destination, &planned_steps)?;
    let mut prospective = locked.clone();
    for reference in plan["tree_references"].as_array().unwrap() {
        let checkout = decode_path(reference["checkout"].as_str().unwrap())?;
        let parent = reference["parent_scope_id"].as_str().unwrap();
        let child = reference["child_scope_id"].as_str().unwrap();
        if let Some(p) = prospective
            .physical
            .iter_mut()
            .find(|p| p.checkout.root == checkout)
        {
            let children = p.children.entry(parent.into()).or_default();
            children.retain(|s| s != child);
            if reference.get("after").is_some() {
                children.push(child.into());
            }
        }
    }
    prospective.pending.retain(|p| p["id"] != plan["id"]);
    prospective.select_prospective(
        &sid,
        &destination,
        plan["new_boundaries"].as_array().unwrap(),
    );
    prospective.require_healthy()?;
    let step = plan["step"].as_u64().unwrap() as usize;
    if step > planned_steps.len() {
        return Err(Error::new(
            "invalid_format",
            "hosting progress step exceeds plan",
        ));
    }
    // Completed steps must match; only the next may reconcile a stale counter.
    for (i, (p, before, after)) in planned_steps.iter().enumerate() {
        let installed = locked.capture.files.get(p).map(|s| s.raw.as_slice()) == after.as_deref();
        if (i < step && !installed)
            || (i > step && !matches(&locked.capture, p, before))
            || (i == step && !installed && !matches(&locked.capture, p, before))
        {
            return Err(Error::new("conflict", "hosting planned source changed").at(p));
        }
    }
    if continuation.is_none()
        && planned_steps.iter().all(|(p, _, after)| {
            locked.capture.files.get(p).map(|s| s.raw.as_slice()) == after.as_deref()
        })
    {
        locked.require_healthy()?;
        guard.verify()?;
        return Ok(locked.wrap(json!({"scope":locked.scope(&sid),"source_hosts":plan["source_hosts"],"destination":path(&destination),"inventory_sha256":plan["inventory"]["sha256"],"changed":false,"partial":partial()})));
    }
    // Checkout locks are ordered by canonical OS paths and follow coordination.
    let mut roots: BTreeMap<PathBuf, ()> = sources
        .iter()
        .chain([&destination])
        .map(|p| (p.clone(), ()))
        .collect();
    for (p, _, _) in &planned_steps {
        if let Some(root) = locked
            .physical
            .iter()
            .filter(|c| p.starts_with(&c.checkout.root))
            .max_by_key(|c| c.checkout.root.components().count())
        {
            roots.insert(root.checkout.root.clone(), ());
        }
    }
    let mut writers = Vec::new();
    for root in roots.keys() {
        writers.push(CheckoutGuard::open(root)?);
    }
    // The checkout lock itself is a permitted local coordination file. Reload
    // after acquiring it, then fence all pre-lock content identities again.
    let current = View::load(&ops.project, false)?;
    locked.capture.compare_prelock(&current.capture)?;
    locked.capture = current.capture;
    let operation_id = plan["id"].as_str().unwrap().to_owned();
    let progress = progress_dir.join(format!("{operation_id}.yaml"));
    let mut known = partial();
    let result = (|| -> Result<Value> {
        guard.verify()?;
        for w in &writers {
            w.verify()?;
        }
        if continuation.is_none() {
            locked.capture.publish(&progress, &yaml_bytes(&plan))?;
            known["created"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":operation_id,"path":path(&progress)}));
        }
        for (i, (p, _, after)) in planned_steps.iter().enumerate().skip(step) {
            guard.verify()?;
            for w in &writers {
                w.verify()?;
            }
            locked.capture.recheck()?;
            if locked.capture.files.get(p).map(|s| s.raw.as_slice()) != after.as_deref() {
                let exists = locked.capture.files.contains_key(p);
                if let Some(after) = after {
                    locked.capture.publish(p, after)?;
                } else {
                    locked.capture.delete(p)?;
                }
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
            plan["step"] = json!(i + 1);
            locked.capture.publish(&progress, &yaml_bytes(&plan))?;
        }
        locked.capture.delete(&progress)?;
        let final_view = View::load(&ops.project, false)?;
        final_view.require_healthy()?;
        final_view.capture.recheck()?;
        guard.verify()?;
        Ok(final_view.wrap(json!({"scope":final_view.scope(&sid),"source_hosts":plan["source_hosts"],"destination":path(&destination),"inventory_sha256":plan["inventory"]["sha256"],"changed":true,"partial":known})))
    })();
    result.map_err(|e| super::scopes::error_partial(e, known, &operation_id))
}

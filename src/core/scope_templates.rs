//! Selected-checkout definitions, scope-resolved publication, ordinary partial files.
//! @mara implements REQ-GRAPH-TEMPLATES
//! @mara implements REQ-RUN-ATOMICITY
use super::coordination::{ExecutionError as Error, ExecutionResult as Result, new_id};
use super::items::{Completion, ItemFile, ItemStore};
use super::operations::{self, CheckoutGuard};
use super::project::Project;
use super::scope_publication as publication;
use super::scope_runs as runs;
use super::scopes::{View, coordination, diagnostic, partial, path, read_view};
use super::storage::files::Directory;
use super::templates::{
    self, Persistence, PreviewRequest, TemplateCatalog, TemplateError, TemplateFile,
    TemplatePreview,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn template_error(e: TemplateError) -> Error {
    let code = match e {
        TemplateError::InvalidTemplate(_) => "invalid_format",
        _ => e.code(),
    };
    let mut error = Error::new(code, e.to_string());
    if !e.diagnostics().is_empty() {
        error.details["diagnostics"] = json!(
            e.diagnostics()
                .iter()
                .map(|d| {
                    let mut out = diagnostic(code, &d.message, Some(&d.path), None);
                    out["line"] = json!(d.line);
                    out
                })
                .collect::<Vec<_>>()
        );
    }
    error
}
fn catalog(v: &mut View) -> Result<TemplateCatalog> {
    let root = v.project.worktree_root.clone();
    let mut dir = Directory::open(&root)?;
    v.capture.directory(&dir)?;
    for name in [".work", "templates"] {
        if !dir.exists(name)? {
            return Ok(TemplateCatalog { files: vec![] });
        }
        dir = dir.child(name)?;
        v.capture.directory(&dir)?;
    }
    let mut files = vec![];
    for name in dir.names()? {
        use std::os::unix::ffi::OsStrExt;
        if name.as_bytes().starts_with(b".stage-") || name.as_bytes().starts_with(b".operation-") {
            continue;
        }
        let p = dir.path.join(&name);
        let raw_name = name.to_string_lossy().into_owned();
        let stem = raw_name.strip_suffix(".yaml").unwrap_or(&raw_name);
        if !raw_name.ends_with(".yaml")
            || super::scope_wire::validate("TemplateIdentifier", &json!(stem)).is_err()
        {
            files.push(TemplateFile {
                name: raw_name,
                path: p.clone(),
                definition: None,
                diagnostics: vec![super::items::Diagnostic {
                    path: p,
                    line: None,
                    message: "template filename must be <name>.yaml".into(),
                }],
            });
            continue;
        }
        match v.capture.file(&p) {
            Ok(Some(s)) => files.push(templates::decode_captured(stem, p, &s.raw)),
            Ok(None) => {
                return Err(Error::new("conflict", "template disappeared during capture").at(p));
            }
            Err(e) => files.push(TemplateFile {
                name: stem.into(),
                path: p.clone(),
                definition: None,
                diagnostics: vec![super::items::Diagnostic {
                    path: p,
                    line: None,
                    message: e.to_string(),
                }],
            }),
        }
    }
    v.capture.recheck()?;
    Ok(TemplateCatalog { files })
}
fn diagnostics(file: &TemplateFile) -> Vec<Value> {
    file.diagnostics
        .iter()
        .map(|d| {
            let mut out = diagnostic("invalid_format", &d.message, Some(&d.path), None);
            out["line"] = json!(d.line);
            out
        })
        .collect()
}
fn request(v: &View, a: &Value) -> Result<PreviewRequest> {
    let values = |key: &str| {
        a[key]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(PreviewRequest {
        root: a["root"]
            .as_str()
            .map(|r| super::scope_items::resolve(v, r))
            .transpose()?,
        parameters: values("parameters"),
        existing: values("existing"),
    })
}
fn target(v: &View, a: &Value, require_active: bool) -> Result<Option<String>> {
    let scope = a["scope"].as_str().map(|s| v.scope_id(s)).transpose()?;
    let (inherited, state) = v.effective_run(scope.as_deref().unwrap_or("root"));
    let rid = a["run_id"]
        .as_str()
        .map(str::to_owned)
        .or(inherited.clone());
    if let Some(explicit) = a["run_id"].as_str() {
        runs::manifest(v, explicit)?;
        if scope.is_some() && inherited.as_deref() != Some(explicit) {
            return Err(Error::new(
                "invalid_argument",
                "run/scope execution selectors disagree",
            ));
        }
    }
    if require_active {
        if let Some(rid) = &rid {
            runs::active(v, rid)?;
        } else if state != "active" {
            return Err(Error::new(
                "coordination_unavailable",
                "target execution is not initialized/current",
            ));
        }
    }
    Ok(rid.filter(|r| runs::active(v, r).is_ok()))
}
struct Plan {
    preview: TemplatePreview,
    version: u8,
    candidate: ItemStore,
    items: Vec<(String, String, Persistence, PathBuf, Vec<u8>)>,
    updated: Vec<ItemFile>,
    refs: Vec<Value>,
    run: Option<String>,
    placement: bool,
}
fn plan(v: &mut View, catalog: &TemplateCatalog, a: &Value, expanding: bool) -> Result<Plan> {
    let name = a["name"].as_str().unwrap();
    let request = request(v, a)?;
    let definition = catalog.validate(name).map_err(template_error)?;
    let preview = catalog
        .preview(name, &request, &v.store)
        .map_err(template_error)?;
    let wisps = preview
        .items
        .iter()
        .any(|i| i.persistence == Persistence::Wisp);
    // Scope selection alone is not an execution request for material planning.
    // Explicit runs still require current execution and selector agreement.
    let run = target(v, a, expanding && (wisps || a["run_id"].is_string()))?;
    if expanding && wisps && run.is_none() {
        return Err(Error::new(
            "coordination_unavailable",
            "wisp expansion requires an active initialized effective run",
        ));
    }
    let mut occupied: BTreeSet<_> = v
        .store
        .files
        .iter()
        .filter_map(|f| f.header.as_ref().map(|h| h.id.clone()))
        .collect();
    let mut ids = BTreeMap::new();
    let mut serial = 1u64;
    for i in &preview.items {
        let id = loop {
            let id = if expanding {
                new_id()?
            } else {
                let id = format!("{serial:08x}000040008000000000000000");
                serial += 1;
                id
            };
            if occupied.insert(id.clone()) {
                break id;
            }
        };
        ids.insert(i.key.clone(), id);
    }
    let file = &catalog.files.iter().find(|f| f.name == name).unwrap().path;
    let (files, _) = templates::prospective_files(
        &preview.items,
        &definition.edges,
        file,
        &request,
        &v.store,
        &ids,
    )
    .map_err(template_error)?;
    let mut candidate = ItemStore::from_candidate_files(files);
    let refs = super::scope_tree::changes(v, &candidate)?;
    let prospective = super::scope_tree::prospective(v, &candidate, &refs);
    super::scope_items::validate_context_change(v, &prospective)?;
    let run_home = run
        .as_deref()
        .map(|rid| runs::manifest(v, rid))
        .transpose()?
        .map(|(p, _, _)| p.parent().unwrap().join("items"));
    let mut items = vec![];
    let mut placement = !v.physical_mode && v.diagnostics.is_empty() && v.pending.is_empty();
    for i in &preview.items {
        let id = &ids[&i.key];
        let source = candidate
            .files
            .iter()
            .find(|f| f.header.as_ref().is_some_and(|h| &h.id == id))
            .unwrap();
        let destination = if i.persistence == Persistence::Wisp {
            if let Some(rid) = &run {
                if prospective.effective_run(id).0.as_deref() != Some(rid) {
                    return Err(Error::new(
                        "wisp_run_mismatch",
                        format!(
                            "local:{} does not explicitly descend into target run",
                            i.key
                        ),
                    ));
                }
                run_home.clone()
            } else {
                placement = false;
                None
            }
        } else {
            prospective.source_host(id).map(|h| h.join(".work/items"))
        };
        if destination.is_none() {
            placement = false;
        }
        let p = destination.unwrap_or_default().join(format!("{id}.md"));
        let raw = operations::serialize(
            source.header.as_ref().unwrap(),
            source.body.as_ref().unwrap(),
        );
        items.push((i.key.clone(), id.clone(), i.persistence, p, raw));
    }
    if expanding && !placement {
        v.require_healthy()?;
        return Err(Error::new(
            "source_unavailable",
            "all template destinations must be authoritative",
        ));
    }
    for (_, id, _, p, raw) in &items {
        if let Some(f) = candidate
            .files
            .iter_mut()
            .find(|f| f.header.as_ref().is_some_and(|h| &h.id == id))
        {
            *f = super::items::parse_candidate(p.clone(), raw.clone());
        }
    }
    let mut updated = vec![];
    for f in &v.store.files {
        let id = &f.header.as_ref().unwrap().id;
        let changed = candidate.resolve(id).unwrap();
        if changed.header != f.header {
            if v.is_wisp(f) {
                runs::require_current_wisp(v, f)?;
            }
            updated.push(changed.clone());
        }
    }
    updated.sort_by(|a, b| a.path.cmp(&b.path));
    if expanding {
        let mut auth = BTreeMap::new();
        for authorization in a["authorization"].as_array().into_iter().flatten() {
            let id = super::scope_items::resolve(v, authorization["item"].as_str().unwrap())?;
            if auth.insert(id, authorization.clone()).is_some() {
                return Err(Error::new(
                    "invalid_argument",
                    "duplicate authorization source",
                ));
            }
        }
        for f in &updated {
            let id = &f.header.as_ref().unwrap().id;
            let form = auth
                .remove(id)
                .map_or_else(|| json!({}), |a| json!({"authorization":[a]}));
            super::scope_items::authorize(v, id, &form)?;
        }
        if !auth.is_empty() {
            return Err(Error::new("stale_claim", "unused authorization"));
        }
    }
    Ok(Plan {
        preview,
        version: definition.format_version,
        candidate,
        items,
        updated,
        refs,
        run: if wisps || a.get("run_id").is_some() {
            run
        } else {
            None
        },
        placement,
    })
}
pub fn call(project: &Project, name: &str, a: &Value) -> Result<Value> {
    let expanding = name == "template_expand";
    let (mut v, read_guard) = if expanding {
        (View::load(project, false)?, None)
    } else {
        read_view(project, a["view"] == "physical")?
    };
    let catalog = catalog(&mut v)?;
    if name == "template_list" {
        let templates=catalog.files.iter().map(|f|json!({"name":f.name,"path":path(&f.path),"format_version":f.definition.as_ref().map(|d|d.format_version),"diagnostics":diagnostics(f)})).collect::<Vec<_>>();
        v.capture.recheck()?;
        if let Some(g) = read_guard {
            g.verify()?;
        }
        return Ok(v.wrap(json!({"templates":templates})));
    }
    if name == "template_validate" {
        let f = catalog
            .files
            .iter()
            .find(|f| f.name == a["name"].as_str().unwrap())
            .ok_or_else(|| Error::new("not_found", "template not found"))?;
        let mut out = v.wrap(json!({"name":f.name,"valid":f.definition.is_some()}));
        out["diagnostics"]
            .as_array_mut()
            .unwrap()
            .extend(diagnostics(f));
        v.capture.recheck()?;
        if let Some(g) = read_guard {
            g.verify()?;
        }
        return Ok(out);
    }
    if expanding {
        v.require_healthy()?;
    }
    let plan = plan(&mut v, &catalog, a, expanding)?;
    if !expanding {
        let items=plan.preview.items.iter().map(|i|{
            let destination=plan.items.iter().find(|(key,_,_,_,_)|key==&i.key).map(|(_,_,_,p,_)|p.parent().unwrap());
            let hint=|value:&Option<String>,source:&Option<templates::HintSource>|json!({"value":value,"source":match source {Some(templates::HintSource::Item)=>"item",Some(templates::HintSource::TemplateDefault)=>"default",None=>"absent"}});
            json!({"key":i.key,"title":i.title,"body":i.body,"completion":if i.completion==Completion::Manual{"manual"}else{"children"},"priority":i.priority,"labels":i.labels,"model":hint(&i.model,&i.model_source),"thinking":hint(&i.thinking,&i.thinking_source),"persistence":i.persistence.as_str(),"destination":destination.filter(|p|p.is_absolute()).map(path)})
        }).collect::<Vec<_>>();
        let edges = plan
            .preview
            .edges
            .iter()
            .map(|e| json!({"from":e.from.reference,"kind":e.kind.as_str(),"to":e.to.reference}))
            .collect::<Vec<_>>();
        v.capture.recheck()?;
        if let Some(g) = read_guard {
            g.verify()?;
        }
        return Ok(v.wrap(json!({"name":plan.preview.name,"format_version":plan.version,"parameters":plan.preview.parameters,"existing":plan.preview.existing,"items":items,"edges":edges,"placement_validated":plan.placement})));
    }
    expand(project, v, plan)
}
fn expand(project: &Project, mut v: View, plan: Plan) -> Result<Value> {
    let prospective = super::scope_tree::prospective(&v, &plan.candidate, &plan.refs);
    let mut writes = vec![];
    for (key, id, persistence, p, raw) in &plan.items {
        writes.push((
            p.clone(),
            Some(raw.clone()),
            Some(id.clone()),
            Some((key.clone(), *persistence)),
        ));
    }
    for f in &plan.updated {
        // Reference creations precede their author, removals follow. No template intent/retry entity.
        for create in [true, false] {
            if !create {
                writes.push((
                    f.path.clone(),
                    Some(operations::serialize(
                        f.header.as_ref().unwrap(),
                        f.body.as_ref().unwrap(),
                    )),
                    Some(f.header.as_ref().unwrap().id.clone()),
                    None,
                ));
            }
            for edge in plan
                .refs
                .iter()
                .filter(|e| e.get("after").is_some() == create)
            {
                let parent = edge["parent_scope_id"].as_str().unwrap();
                let child = edge["child_scope_id"].as_str().unwrap();
                let host = prospective
                    .source_host(parent)
                    .ok_or_else(|| Error::new("host_missing", "tree reference home missing"))?;
                let p = host.join(format!(".work/scopes/{parent}/children/{child}.yaml"));
                let step = (
                    p,
                    edge.get("after").map(super::coordination::yaml_bytes),
                    None,
                    None,
                );
                if !writes.contains(&step) {
                    writes.push(step);
                }
            }
        }
    }
    let mut roots = BTreeSet::new();
    for (p, _, _, _) in &writes {
        let root = p
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == ".work"))
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        publication::preflight(&root, p)?;
        publication::preflight(&root, &root.join(".work/operations.lock"))?;
        roots.insert(root);
    }
    let originals: Vec<_> = writes
        .iter()
        .map(|(p, _, _, _)| publication::snapshot(&v.capture, p))
        .collect();
    let stages: Vec<_> = writes
        .iter()
        .map(|_| new_id().map(|id| format!(".stage-{id}")))
        .collect::<Result<_>>()?;
    let mut effects = partial();
    effects["allocated_items"] = json!(
        plan.items
            .iter()
            .map(|(key, id, _, _, _)| json!({"key":key,"id":id}))
            .collect::<Vec<_>>()
    );
    let mut last = None;
    let result = (|| -> Result<Value> {
        let guard = coordination(&v, true, true)?.unwrap();
        let mut fresh = View::load(project, false)?;
        catalog(&mut fresh)?;
        v.capture.compare_prelock(&fresh.capture)?;
        v = fresh;
        let mut checkouts = vec![];
        for root in &roots {
            publication::ensure(&mut v.capture, &root.join(".work"), &mut effects)?;
            checkouts.push(CheckoutGuard::open(root)?);
        }
        let mut fresh = View::load(project, false)?;
        catalog(&mut fresh)?;
        v.capture.compare_prelock(&fresh.capture)?;
        v = fresh;
        v.require_healthy()?;
        let fence = || -> Result<()> {
            guard.verify()?;
            for c in &checkouts {
                c.verify()?;
            }
            Ok(())
        };
        let mut saved = vec![];
        for (i, (p, raw, id, _)) in writes.iter().enumerate() {
            last = Some(p.clone());
            fence()?;
            v.capture.recheck()?;
            if let Some(raw) = raw {
                publication::save(
                    &mut v.capture,
                    p,
                    raw,
                    &stages[i],
                    &mut effects,
                    id.as_deref(),
                    &fence,
                )?;
            } else {
                v.capture.delete(p)?;
                publication::known(&mut effects, "deleted", p, id.as_deref());
            }
            saved.push(i);
        }
        let final_view = View::load(project, false)?;
        final_view.require_healthy()?;
        for (_, id, _, p, raw) in &plan.items {
            if final_view.store.resolve(id).is_err()
                || final_view
                    .capture
                    .files
                    .get(p)
                    .is_none_or(|s| &s.raw != raw)
            {
                return Err(Error::new("conflict", "published item postcondition changed").at(p));
            }
        }
        v.capture.recheck()?;
        final_view.capture.recheck()?;
        fence()?;
        for i in saved {
            let (p, raw, _, _) = &writes[i];
            if raw.is_some() {
                publication::sync(&v.capture, p)?;
                publication::cleanup(
                    &v.capture,
                    &p.parent().unwrap().join(&stages[i]),
                    &originals[i],
                    &fence,
                )?;
            }
        }
        v.capture.recheck()?;
        final_view.capture.recheck()?;
        fence()?;
        let items=plan.items.iter().map(|(key,id,persistence,p,_)|json!({"key":key,"id":id,"persistence":persistence.as_str(),"path":path(p)})).collect::<Vec<_>>();
        let updated = plan
            .updated
            .iter()
            .map(|f| json!({"id":f.header.as_ref().unwrap().id,"path":path(&f.path)}))
            .collect::<Vec<_>>();
        Ok(final_view.wrap(json!({"items":items,"updated":updated,"run_id":plan.run,"changed":true,"partial":effects})))
    })();
    result.map_err(|e| publication::attempted(e, &effects, last.as_deref()))
}

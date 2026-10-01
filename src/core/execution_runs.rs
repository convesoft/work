//! Run and template publication orchestration using the shared execution guard.
use super::claims::ClaimStore;
use super::context::{ContextStore, ResolvedView, Workspace};
use super::coordination::*;
use super::execution::{ExecutionOperations, require_valid, resolve_item, with_setup_progress};
use super::operations::CheckoutWriter;
use super::runs::{RunMutation, RunPhase, RunRecord, RunSnapshot, RunStore};
use super::templates::{Persistence, PreviewRequest, TemplateCatalog, TemplateError};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
impl From<TemplateError> for ExecutionError {
    fn from(e: TemplateError) -> Self {
        let mut result = Self::new(e.code(), e.to_string());
        result.details["diagnostics"] = json!(
            e.diagnostics()
                .iter()
                .map(|d| json!({"path":encode_path(&d.path),"line":d.line,"message":d.message}))
                .collect::<Vec<_>>()
        );
        result
    }
}
fn owners(g: &CoordinationGuard) -> ExecutionResult<BTreeSet<String>> {
    Ok(ClaimStore::list(g, None, true)?
        .into_iter()
        .map(|c| c.claim.item_id)
        .collect())
}
fn workspace(g: &CoordinationGuard, v: &ResolvedView, id: &str) -> ExecutionResult<Workspace> {
    let w = v
        .context
        .workspaces
        .get(id)
        .ok_or_else(|| ExecutionError::new("not_found", "workspace is not registered"))?;
    if w.state != "open" {
        return Err(ExecutionError::new(
            "workspace_busy",
            "workspace is closing",
        ));
    }
    let p = super::project::discover(Some(&w.path))
        .map_err(|e| ExecutionError::new("source_unavailable", e.to_string()).at(&w.path))?;
    if p.git_common_dir != g.project().git_common_dir {
        return Err(ExecutionError::new(
            "source_unavailable",
            "workspace repository changed",
        ));
    }
    Ok(w.clone())
}
fn run_value(r: &RunRecord, snapshot: &RunSnapshot<'_>) -> Value {
    let finished = r.finished(snapshot);
    let (finished, diagnostics) = match finished {
        Ok(v) => (Some(v), json!([])),
        Err(e) => (None, json!([{"code":e.code,"message":e.message}])),
    };
    json!({"run":r.manifest.to_json(),"finished":finished,"members":r.members(),"diagnostics":diagnostics})
}
fn mutation(v: RunMutation) -> Value {
    json!({"run":v.run.to_json(),"finished":v.finished,"changed":v.changed})
}
fn bind_material(
    g: &CoordinationGuard,
    v: &ResolvedView,
    id: &str,
    source: &std::path::Path,
    created: &mut Vec<Value>,
) -> ExecutionResult<Workspace> {
    let workspace = ContextStore::register(g, source)?;
    if !v.context.workspaces.contains_key(&workspace.id) {
        let record = json!({"id":workspace.id,"path":encode_path(&g.root_path().join(format!("workspaces/{}.yaml",workspace.id)))});
        if !created.contains(&record) {
            created.push(record);
        }
    }
    if !v.context.bindings.contains_key(id) {
        ContextStore::bind(g, id, &workspace)?;
        created.push(json!({"id":id,"path":encode_path(&g.root_path().join(format!("workspaces/items/{id}.yaml")))}));
    }
    Ok(workspace)
}
impl ExecutionOperations {
    pub fn run_start(
        &self,
        input: &str,
        default: Option<String>,
        output: Option<String>,
    ) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        let root = resolve_item(&v.store, input)?
            .header
            .as_ref()
            .unwrap()
            .id
            .clone();
        let source = v
            .sources
            .get(&root)
            .ok_or_else(|| ExecutionError::new("invalid_argument", "run root must be material"))?;
        let active = owners(&g)?;
        let material = v.sources.keys().cloned().collect();
        let snapshot = RunSnapshot {
            view: &v.store,
            active_claims: &active,
            material_ids: &material,
        };
        for wid in default.iter().chain(output.iter()) {
            workspace(&g, &v, wid)?;
        }
        if v.runs.current_for_root(&root).is_some() {
            return Ok(mutation(RunStore::start(
                &g, &snapshot, &root, default, output,
            )?));
        }
        if v.runs.membership(&root).is_some() {
            return Err(ExecutionError::new(
                "run_conflict",
                "root already belongs to a current run",
            ));
        }
        v.recheck(&g)?;
        let mut created = Vec::new();
        (|| {
            let root_workspace = bind_material(&g, &v, &root, source, &mut created)?;
            let default = default.or_else(|| Some(root_workspace.id.clone()));
            let output = output.or(Some(root_workspace.id));
            v.recheck(&g)?;
            Ok(mutation(RunStore::start(
                &g, &snapshot, &root, default, output,
            )?))
        })()
        .map_err(|e| with_setup_progress(e, &created))
    }
    pub fn run_inspect(&self, id: &str) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, false)?;
        let v = ResolvedView::load(&g)?;
        let active = owners(&g)?;
        let material = v.sources.keys().cloned().collect();
        Ok(run_value(
            v.runs.get(id)?,
            &RunSnapshot {
                view: &v.store,
                active_claims: &active,
                material_ids: &material,
            },
        ))
    }
    pub fn run_list(&self, all: bool) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, false)?;
        let v = ResolvedView::load(&g)?;
        let active = owners(&g)?;
        let material = v.sources.keys().cloned().collect();
        let snapshot = RunSnapshot {
            view: &v.store,
            active_claims: &active,
            material_ids: &material,
        };
        Ok(
            json!({"runs":v.runs.records.iter().filter(|r|all||r.manifest.phase.is_current()).map(|r|run_value(r,&snapshot)).collect::<Vec<_>>()}),
        )
    }
    pub fn run_membership(
        &self,
        id: &str,
        inputs: &[String],
        attach: bool,
    ) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        let record = v.runs.get(id)?;
        if record.manifest.phase != RunPhase::Active {
            return Err(ExecutionError::new(
                "run_not_current",
                "run is frozen or terminal",
            ));
        }
        let active = owners(&g)?;
        let material: BTreeSet<_> = v.sources.keys().cloned().collect();
        let mut ids = Vec::new();
        for input in inputs {
            let item = resolve_item(&v.store, input)?
                .header
                .as_ref()
                .unwrap()
                .id
                .clone();
            if !material.contains(&item) || item == record.manifest.root_item_id {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "member must be a non-root material item",
                ));
            }
            if attach
                && v.runs
                    .membership(&item)
                    .is_some_and(|r| r.manifest.id != id)
            {
                return Err(ExecutionError::new(
                    "run_conflict",
                    "member belongs to another current run",
                ));
            }
            if record.manifest.material_items.contains(&item) != attach && active.contains(&item) {
                return Err(ExecutionError::new(
                    "claim_conflict",
                    "active claim prevents membership change",
                ));
            }
            ids.push(item);
        }
        if ids.is_empty() {
            return Err(ExecutionError::new(
                "invalid_argument",
                "membership requires items",
            ));
        }
        v.recheck(&g)?;
        let mut created = Vec::new();
        (|| {
            if attach {
                for item in &ids {
                    if !v.context.bindings.contains_key(item) {
                        bind_material(&g, &v, item, &v.sources[item], &mut created)?;
                    }
                }
            }
            // Setup may publish workspaces/bindings while an external editor
            // changes the captured material sources. Refuse before membership
            // publication, retaining setup progress on the returned error.
            v.recheck(&g)?;
            let snapshot = RunSnapshot {
                view: &v.store,
                active_claims: &active,
                material_ids: &material,
            };
            Ok(mutation(if attach {
                RunStore::attach(&g, &snapshot, id, &ids)?
            } else {
                RunStore::detach(&g, &snapshot, id, &ids)?
            }))
        })()
        .map_err(|e| with_setup_progress(e, &created))
    }

    pub fn expand(
        &self,
        name: &str,
        request: &PreviewRequest,
        run_id: Option<&str>,
    ) -> ExecutionResult<Value> {
        self.expand_inner(name, request, run_id, |_| Ok(()))
    }
    fn expand_inner(
        &self,
        name: &str,
        request: &PreviewRequest,
        run_id: Option<&str>,
        before_item: impl Fn(usize) -> ExecutionResult<()>,
    ) -> ExecutionResult<Value> {
        let g = CoordinationGuard::acquire(&self.project, true)?;
        let v = ResolvedView::load(&g)?;
        require_valid(&v.store)?;
        owners(&g)?;
        let run = run_id.map(|id| v.runs.get(id)).transpose()?;
        let catalog = TemplateCatalog::load_from_root(&self.project.worktree_root)?;
        let plan = catalog.plan_expansion(
            name,
            request,
            &v.store,
            &self.project.worktree_root,
            run.map(|r| &r.manifest),
            g.root_path(),
        )?;
        // Validate supplied pairs even when the template creates only new files.
        let check_id = plan
            .items
            .first()
            .map(|i| i.id.clone())
            .unwrap_or(new_id()?);
        ClaimStore::authorize(&g, &check_id, &self.authorization)?;
        for file in &plan.updated {
            let id = &file.header.as_ref().unwrap().id;
            ClaimStore::authorize(&g, id, &self.authorization)?;
            if let Some(owner) = v.runs.membership(id)
                && owner.wisp_sources.contains_key(id)
                && owner.manifest.phase != RunPhase::Active
            {
                let mut error = ExecutionError::new(
                    "run_not_current",
                    "existing wisp run is frozen for cleanup",
                )
                .at(&file.path);
                error.details = json!({"run_id":owner.manifest.id,"item_id":id,"phase":owner.manifest.phase.as_str(),"publication":"not_published"});
                return Err(error);
            }
        }
        let mut roots = BTreeSet::new();
        if plan
            .items
            .iter()
            .any(|i| i.persistence == Persistence::Material)
        {
            roots.insert(self.project.worktree_root.clone());
        }
        for file in &plan.updated {
            if let Some(root) = v.sources.get(&file.header.as_ref().unwrap().id) {
                roots.insert(root.clone());
            }
        }
        let mut writers = BTreeMap::new();
        for root in roots {
            writers.insert(root.clone(), CheckoutWriter::open(&root)?);
        }
        v.recheck(&g)?;
        g.verify()?;
        let mut setup_created = Vec::new();
        let mut created = Vec::new();
        let mut updated = Vec::new();
        let mut attempted = None;
        let publish = (|| -> ExecutionResult<()> {
            let dest = if plan
                .items
                .iter()
                .any(|i| i.persistence == Persistence::Material)
            {
                let workspace = ContextStore::register(&g, &self.project.worktree_root)?;
                if !v.context.workspaces.contains_key(&workspace.id) {
                    setup_created.push(json!({"id":workspace.id,"path":encode_path(&g.root_path().join(format!("workspaces/{}.yaml",workspace.id)))}));
                }
                Some(workspace)
            } else {
                None
            };
            for (index, item) in plan.items.iter().enumerate() {
                attempted = None;
                before_item(index)?;
                match item.persistence {
                    Persistence::Material => {
                        ContextStore::bind(&g, &item.id, dest.as_ref().unwrap())?;
                        setup_created.push(json!({"id":item.id,"path":encode_path(&g.root_path().join(format!("workspaces/items/{}.yaml",item.id)))}));
                        attempted = Some(item.path.clone());
                        writers
                            .get_mut(&self.project.worktree_root)
                            .unwrap()
                            .publish(&item.header, &item.body, None)?;
                    }
                    Persistence::Wisp => {
                        attempted = Some(item.path.clone());
                        RunStore::create_wisp(
                            &g,
                            run_id.expect("planner requires run"),
                            &item.header,
                            &item.body,
                        )
                        .inspect_err(|error| {
                            if error.details["publication"] == "published" {
                                created.push(item.id.clone());
                            }
                        })?;
                    }
                }
                created.push(item.id.clone());
            }
            for file in &plan.updated {
                attempted = Some(file.path.clone());
                let h = file.header.as_ref().unwrap();
                let body = file.body.as_deref().unwrap();
                if let Some(root) = v.sources.get(&h.id) {
                    writers
                        .get_mut(root)
                        .unwrap()
                        .publish(h, body, Some(file))?;
                } else {
                    let run = v.runs.membership(&h.id).unwrap();
                    RunStore::replace_wisp(&g, &run.manifest.id, &run.wisp_sources[&h.id], h, body)
                        .inspect_err(|error| {
                            if error.details["publication"] == "published" {
                                updated.push(h.id.clone());
                            }
                        })?;
                }
                updated.push(h.id.clone());
            }
            if let Some(run_id) = run_id {
                let ids: Vec<_> = plan
                    .items
                    .iter()
                    .filter(|i| i.persistence == Persistence::Material)
                    .map(|i| i.id.clone())
                    .collect();
                if !ids.is_empty() {
                    attempted = Some(g.root_path().join("runs").join(run_id).join("run.yaml"));
                    let current = ResolvedView::load(&g)?;
                    let active = owners(&g)?;
                    let material = current.sources.keys().cloned().collect();
                    RunStore::attach(
                        &g,
                        &RunSnapshot {
                            view: &current.store,
                            active_claims: &active,
                            material_ids: &material,
                        },
                        run_id,
                        &ids,
                    )?;
                }
            }
            Ok(())
        })();
        if let Err(error) = publish {
            let uncertain_paths = if error.details["publication"] == "possible" {
                attempted.into_iter().collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            return Err(plan.partial_error(
                with_setup_progress(error, &setup_created),
                &created,
                &updated,
                &uncertain_paths,
            ));
        }
        Ok(
            json!({"run_id":run_id,"items":plan.items.iter().map(|i|json!({"key":i.key,"id":i.id,"persistence":i.persistence.as_str(),"path":encode_path(&i.path)})).collect::<Vec<_>>(),"updated":plan.updated.iter().map(|f|json!({"id":f.header.as_ref().unwrap().id,"path":encode_path(&f.path)})).collect::<Vec<_>>(),"changed":!created.is_empty()||!updated.is_empty()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    #[test]
    fn actual_expansion_keeps_three_of_five_and_allows_manual_cleanup_retry() {
        let root = std::env::temp_dir().join(format!(
            "work-expansion-partial-{}-{}",
            std::process::id(),
            new_id().unwrap()
        ));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        fs::create_dir_all(root.join(".work/templates")).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join(".work/templates/five.yaml"),"format_version: 2\nname: five\nitems: [{key: a, title: A}, {key: b, title: B}, {key: c, title: C}, {key: d, title: D}, {key: e, title: E}]\n").unwrap();
        let project = super::super::project::discover(Some(&root)).unwrap();
        super::super::storage::Storage::new(project.clone())
            .initialize()
            .unwrap();
        let ops = ExecutionOperations::new(project.clone());
        let request = PreviewRequest {
            root: None,
            parameters: Default::default(),
            existing: Default::default(),
        };
        let error = ops
            .expand_inner("five", &request, None, |index| {
                if index == 3 {
                    Err(ExecutionError::new("io", "injected fourth-write failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let partial = &error.details["partial"];
        let files: Vec<_> = partial["created"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["path"].as_str().unwrap().ends_with(".md"))
            .collect();
        assert_eq!(files.len(), 3, "{error:?}");
        assert_eq!(
            partial["created"].as_array().unwrap().len(),
            7,
            "workspace plus three bindings and three items"
        );
        assert_eq!(ops.list().unwrap().len(), 3);
        assert_eq!(fs::read_dir(root.join(".work/items")).unwrap().count(), 3);
        let original: Vec<_> = files
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_owned())
            .collect();
        {
            let g = CoordinationGuard::acquire(&project, true).unwrap();
            for id in &original {
                fs::remove_file(root.join(format!(".work/items/{id}.md"))).unwrap();
                let path = std::path::PathBuf::from(format!("workspaces/items/{id}.yaml"));
                let source = g.read(&path).unwrap();
                g.delete(&path, &source).unwrap();
            }
        }
        let retry = ops.expand("five", &request, None).unwrap();
        assert_eq!(retry["items"].as_array().unwrap().len(), 5);
        assert!(
            retry["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| !original.iter().any(|id| v["id"] == id.as_str()))
        );
        assert_eq!(ops.list().unwrap().len(), 5);
        assert_eq!(
            fs::read_dir(root.join(".git/work/runs")).unwrap().count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
    struct RunFixture {
        root: std::path::PathBuf,
        ops: ExecutionOperations,
        run_id: String,
        wisp_id: String,
        wisp_path: std::path::PathBuf,
    }
    impl RunFixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("work-wisp-progress-{}", new_id().unwrap()));
            fs::create_dir_all(root.join(".work/items")).unwrap();
            fs::create_dir_all(root.join(".work/templates")).unwrap();
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(["init", "-q"])
                    .status()
                    .unwrap()
                    .success()
            );
            let project = super::super::project::discover(Some(&root)).unwrap();
            let material = super::super::operations::DurableOperations::new(&root)
                .create("root".into(), b"root body".to_vec(), Default::default())
                .unwrap()
                .file
                .header
                .unwrap()
                .id;
            super::super::storage::Storage::new(project.clone())
                .initialize()
                .unwrap();
            let ops = ExecutionOperations::new(project);
            let result = ops.run_start(&material, None, None).unwrap();
            let run_id = result["run"]["id"].as_str().unwrap().to_owned();
            let guard = CoordinationGuard::acquire(&ops.project, true).unwrap();
            let wisp_id = new_id().unwrap();
            let header =
                super::super::operations::default_header(wisp_id.clone(), "existing wisp".into());
            let wisp_path =
                RunStore::create_wisp(&guard, &run_id, &header, b"original body").unwrap();
            Self {
                root,
                ops,
                run_id,
                wisp_id,
                wisp_path,
            }
        }
        fn template(&self, update: bool) {
            let source = if update {
                "format_version: 2\nname: step\nexisting: [seed]\nitems: [{key: a, title: NewWisp, persistence: wisp}]\nedges: [{from: 'existing:seed', kind: depends_on, to: 'local:a'}]\n"
            } else {
                "format_version: 2\nname: step\nitems: [{key: a, title: NewWisp, persistence: wisp}]\n"
            };
            fs::write(self.root.join(".work/templates/step.yaml"), source).unwrap();
        }
        fn request(&self, update: bool) -> PreviewRequest {
            PreviewRequest {
                root: None,
                parameters: Default::default(),
                existing: if update {
                    BTreeMap::from([("seed".into(), self.wisp_id.clone())])
                } else {
                    BTreeMap::new()
                },
            }
        }
    }
    impl Drop for RunFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn expansion_reports_synced_wisp_create_and_update_without_false_uncertainty() {
        for update in [false, true] {
            let fixture = RunFixture::new();
            fixture.template(update);
            let original = fs::read(&fixture.wisp_path).unwrap();
            let lock = fixture
                .ops
                .project
                .git_common_dir
                .join("work/coordination.lock");
            let hook_lock = lock.clone();
            let hook = super::super::storage::files::on_nth(
                "after_directory_sync",
                if update { 2 } else { 1 },
                move || {
                    fs::rename(&hook_lock, hook_lock.with_extension("retained")).unwrap();
                    fs::write(hook_lock, b"").unwrap();
                },
            );
            let error = fixture
                .ops
                .expand("step", &fixture.request(update), Some(&fixture.run_id))
                .unwrap_err();
            drop(hook);
            let new_id = error.details["key_ids"]["a"].as_str().unwrap();
            let new_path = fixture
                .ops
                .project
                .git_common_dir
                .join(format!("work/runs/{}/items/{new_id}.md", fixture.run_id));
            assert_eq!(error.code, "conflict", "{error:?}");
            assert_eq!(error.path, Some(lock));
            assert_eq!(error.details["publication"], "published");
            let attempted = if update {
                &fixture.wisp_path
            } else {
                &new_path
            };
            assert_eq!(error.details["published_path"], encode_path(attempted));
            assert!(error.details["errno"].is_null());
            assert_eq!(
                error.details["partial"]["created"],
                json!([{"id":new_id,"path":encode_path(&new_path)}])
            );
            assert_eq!(
                error.details["partial"]["updated"],
                if update {
                    json!([{"id":fixture.wisp_id,"path":encode_path(&fixture.wisp_path)}])
                } else {
                    json!([])
                }
            );
            assert_eq!(error.details["partial"]["uncertain_paths"], json!([]));
            assert!(new_path.is_file());
            if update {
                assert!(
                    String::from_utf8(fs::read(&fixture.wisp_path).unwrap())
                        .unwrap()
                        .contains(new_id)
                );
                let retained = error.details["recovery_paths"].as_array().unwrap();
                assert_eq!(retained.len(), 1);
                let path = decode_path(retained[0].as_str().unwrap()).unwrap();
                assert_eq!(fs::read(path).unwrap(), original);
            } else {
                assert_eq!(fs::read(&fixture.wisp_path).unwrap(), original);
            }
        }
    }

    #[test]
    fn expansion_wisp_progress_distinguishes_possible_from_unpublished() {
        for possible in [false, true] {
            let fixture = RunFixture::new();
            fixture.template(false);
            let fault = super::super::storage::files::fail_next(if possible {
                "publication_sync"
            } else {
                "before_publication"
            });
            let error = fixture
                .ops
                .expand("step", &fixture.request(false), Some(&fixture.run_id))
                .unwrap_err();
            drop(fault);
            let id = error.details["key_ids"]["a"].as_str().unwrap();
            let path = fixture
                .ops
                .project
                .git_common_dir
                .join(format!("work/runs/{}/items/{id}.md", fixture.run_id));
            assert_eq!(error.code, "io");
            assert_eq!(error.details["errno"], 5);
            assert_eq!(error.path.as_deref(), Some(path.as_path()));
            assert_eq!(
                error.details["publication"],
                if possible {
                    "possible"
                } else {
                    "not_published"
                }
            );
            assert_eq!(error.details["partial"]["created"], json!([]));
            assert_eq!(error.details["partial"]["updated"], json!([]));
            assert_eq!(
                error.details["partial"]["uncertain_paths"],
                if possible {
                    json!([encode_path(&path)])
                } else {
                    json!([])
                }
            );
            assert_eq!(path.is_file(), possible);
            if possible {
                assert!(
                    !error.details["recovery_paths"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn expansion_can_publish_its_planned_entries_then_update_an_existing_wisp() {
        let fixture = RunFixture::new();
        fixture.template(true);
        let result = fixture
            .ops
            .expand("step", &fixture.request(true), Some(&fixture.run_id))
            .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["updated"].as_array().unwrap().len(), 1);
        assert_eq!(result["updated"][0]["id"], fixture.wisp_id);
        let current = fixture.ops.inspect(&fixture.wisp_id).unwrap();
        assert_eq!(
            current.file.header.unwrap().depends_on,
            vec![result["items"][0]["id"].as_str().unwrap().to_owned()]
        );
    }
    #[test]
    fn membership_rechecks_sources_after_binding_setup_and_keeps_known_progress() {
        for change in ["delete", "edit", "swap"] {
            for new_workspace in [false, true] {
                let fixture = RunFixture::new();
                let member_id = new_id().unwrap();
                let member_header = super::super::operations::default_header(
                    member_id.clone(),
                    "completed member".into(),
                );
                let mut member_header = member_header;
                member_header.state = Some(super::super::items::ManualState::Done);
                let source_root = if new_workspace {
                    // Register a different linked caller checkout during setup.
                    assert!(
                        Command::new("git")
                            .arg("-C")
                            .arg(&fixture.root)
                            .args([
                                "-c",
                                "user.name=Test",
                                "-c",
                                "user.email=test@example.invalid",
                                "commit",
                                "--allow-empty",
                                "-qm",
                                "fixture"
                            ])
                            .status()
                            .unwrap()
                            .success()
                    );
                    let linked = fixture.root.join("linked");
                    assert!(
                        Command::new("git")
                            .arg("-C")
                            .arg(&fixture.root)
                            .args(["worktree", "add", "-q", "-b", "feature"])
                            .arg(&linked)
                            .status()
                            .unwrap()
                            .success()
                    );
                    fs::create_dir_all(linked.join(".work/items")).unwrap();
                    linked
                } else {
                    fixture.root.clone()
                };
                let member = source_root.join(format!(".work/items/{member_id}.md"));
                let raw = super::super::operations::serialize(&member_header, b"completed body");
                fs::write(&member, &raw).unwrap();
                let ops = ExecutionOperations::new(
                    super::super::project::discover(Some(&source_root)).unwrap(),
                );
                let shared = ops.project.git_common_dir.join("work");
                let manifest = shared.join(format!("runs/{}/run.yaml", fixture.run_id));
                let before = fs::read(&manifest).unwrap();
                let changed_member = member.clone();
                let hook = super::super::storage::files::on_nth(
                    "after_directory_sync",
                    if new_workspace { 2 } else { 1 },
                    move || match change {
                        "delete" => fs::remove_file(&changed_member).unwrap(),
                        "edit" => fs::write(
                            &changed_member,
                            super::super::operations::serialize(&member_header, b"external edit"),
                        )
                        .unwrap(),
                        "swap" => {
                            let replacement = changed_member.with_extension("replacement");
                            fs::write(&replacement, &raw).unwrap();
                            fs::rename(replacement, &changed_member).unwrap();
                        }
                        _ => unreachable!(),
                    },
                );
                let error = ops
                    .run_membership(&fixture.run_id, std::slice::from_ref(&member_id), true)
                    .unwrap_err();
                drop(hook);
                assert_eq!(error.code, "conflict", "{change}: {error:?}");
                assert!(error.message.contains("material sources changed"));
                assert_eq!(error.path.as_deref(), Some(source_root.as_path()));
                assert_eq!(error.details["publication"], "not_published");
                assert_eq!(fs::read(&manifest).unwrap(), before);
                let binding_path = shared.join(format!("workspaces/items/{member_id}.yaml"));
                assert!(binding_path.is_file());
                let binding: Value =
                    serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
                let workspace_id = binding["workspace_id"].as_str().unwrap();
                let created = error.details["partial"]["created"].as_array().unwrap();
                assert!(
                    created.contains(&json!({"id":member_id,"path":encode_path(&binding_path)}))
                );
                assert_eq!(created.len(), if new_workspace { 2 } else { 1 });
                if new_workspace {
                    let workspace_path = shared.join(format!("workspaces/{workspace_id}.yaml"));
                    assert!(workspace_path.is_file());
                    assert!(
                        created.contains(
                            &json!({"id":workspace_id,"path":encode_path(&workspace_path)})
                        )
                    );
                }
                assert_eq!(error.details["partial"]["updated"], json!([]));
                assert_eq!(error.details["partial"]["uncertain_paths"], json!([]));
            }
        }
    }

    #[test]
    fn run_start_rechecks_root_after_binding_setup_before_creating_manifest() {
        let root =
            std::env::temp_dir().join(format!("work-start-post-setup-{}", new_id().unwrap()));
        fs::create_dir_all(root.join(".work/items")).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        let project = super::super::project::discover(Some(&root)).unwrap();
        let ops = ExecutionOperations::new(project.clone());
        let id = new_id().unwrap();
        let header = super::super::operations::default_header(id.clone(), "root".into());
        let path = root.join(format!(".work/items/{id}.md"));
        fs::write(
            &path,
            super::super::operations::serialize(&header, b"root body"),
        )
        .unwrap();
        super::super::storage::Storage::new(project.clone())
            .initialize()
            .unwrap();
        let hook = super::super::storage::files::on_nth("after_directory_sync", 2, move || {
            fs::remove_file(path).unwrap()
        });
        let error = ops.run_start(&id, None, None).unwrap_err();
        drop(hook);
        assert_eq!(error.code, "conflict");
        assert_eq!(error.details["publication"], "not_published");
        let created = error.details["partial"]["created"].as_array().unwrap();
        assert_eq!(created.len(), 2);
        for record in created {
            assert!(
                decode_path(record["path"].as_str().unwrap())
                    .unwrap()
                    .is_file()
            );
        }
        assert_eq!(
            fs::read_dir(project.git_common_dir.join("work/runs"))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
}

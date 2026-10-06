//! Explicit standing-root initialization. No child-run or rotation fallback.
//! @mara implements DES-REPOSITORY-ONBOARDING
use super::coordination::{
    ExecutionError as Error, ExecutionResult as Result, new_id, now_timestamp, yaml_bytes,
};
use super::operations::CheckoutGuard;
use super::project::Project;
use super::scope_wire::{self as wire, hash};
use super::scopes::{Capture, View, coordination, error_partial, partial, path, read_view};
use super::storage::files::{Directory, Locked};
use serde_json::{Value, json};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const RULES: &[&[u8]] = &[
    b"/.work/operations.lock",
    b"/.work/**/.operation-*",
    b"/.work/**/.storage-*",
    b"/.work/**/.stage-*",
];
fn appended(before: &[u8]) -> Vec<u8> {
    let missing: Vec<_> = RULES
        .iter()
        .filter(|rule| {
            !before
                .split(|b| *b == b'\n')
                .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == **rule)
        })
        .collect();
    let mut out = before.to_vec();
    if !missing.is_empty() {
        if !out.is_empty() && !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        out.extend_from_slice(
            b"# Work housekeeping (recovery files may be needed; inspect before git clean -X)\n",
        );
        for rule in missing {
            out.extend_from_slice(rule);
            out.push(b'\n');
        }
    }
    out
}
fn snapshot(c: &Capture, p: &Path) -> Value {
    c.files.get(p).map_or_else(
        || json!({"exists":false}),
        |s| json!({"exists":true,"sha256":hash(&s.raw)}),
    )
}
fn manifest(plan: &Value) -> Value {
    json!({"format_version":2,"id":plan["new_run_id"],"created_at":plan["run_created_at"],"phase":"active"})
}
fn anchor(plan: &Value) -> Value {
    json!({"format_version":2,"scope_id":"root","execution":{"mode":"boundary","run_id":plan["new_run_id"]}})
}
fn required(root: &Path, rid: &str) -> Vec<PathBuf> {
    let run = root.join(format!(".work/scopes/root/runs/{rid}"));
    let mut dirs = vec![
        root.join(".work"),
        root.join(".work/items"),
        root.join(".work/templates"),
        root.join(".work/scopes"),
        root.join(".work/scopes/root"),
        root.join(".work/scopes/root/runs"),
        run.clone(),
    ];
    dirs.extend(["items", "sessions", "claims", "handoffs"].map(|s| run.join(s)));
    dirs
}
fn writable(d: &Directory) -> Result<()> {
    if d.file.metadata()?.mode() & 0o200 == 0 {
        return Err(
            Error::new("io", "destination directory lacks owner-write permission").at(&d.path),
        );
    }
    Ok(())
}
fn publication_access(root: &Path, file: &Path) -> Result<()> {
    let mut d = Directory::open(root)?;
    for component in file.parent().unwrap().strip_prefix(root).unwrap() {
        if !d.exists(component)? {
            return writable(&d);
        }
        d = d.child(component)?;
    }
    writable(&d)
}
// Traverse existing components descriptor-relatively, stopping only at genuine absence.
fn preflight(root: &Path, dirs: &[PathBuf]) -> Result<()> {
    for p in dirs {
        let mut d = Directory::open(root)?;
        for component in p.strip_prefix(root).unwrap() {
            if !d.exists(component)? {
                writable(&d)?;
                break;
            }
            d = d.child(component)?;
        }
    }
    Ok(())
}
fn pending(v: &View, id: &str, root: &Path) -> Result<(PathBuf, Value)> {
    let p = root.join(format!(".work/scopes/root/progress/start/{id}.yaml"));
    let source = v.capture.files.get(&p).ok_or_else(|| {
        if v.pending.iter().any(|view| view["path"] == path(&p)) {
            Error::new("invalid_format", "initialization progress is unreadable")
        } else {
            Error::new(
                "not_found",
                "no matching initialization plan; inspect actual state, not a retry receipt",
            )
        }
    })?;
    let plan = wire::envelope("StartProgress", &source.raw, &p)?;
    if plan["id"] != id || plan["scope_id"] != "root" || plan.get("new_run_id").is_none() {
        return Err(Error::new("conflict", "continuation is not this root initialization").at(p));
    }
    Ok((p, plan))
}
fn healthy_for_plan(v: &View, plan: &Value, progress: &Path, run: &Path) -> Result<()> {
    let mut projected = v.clone();
    projected.pending.retain(|p| p["path"] != path(progress));
    // The exact new manifest can precede its anchor. Nothing else is ignored.
    projected.diagnostics.retain(|d| {
        let missing = d["message"] == "orphaned scope/run directory lacks its manifest"
            && ((d["path"] == path(run.parent().unwrap()) && plan["step"].as_u64().unwrap() <= 1)
                || (d["path"] == path(run.parent().unwrap().parent().unwrap().parent().unwrap())
                    && plan["anchor_before"]["exists"] == false
                    && plan["step"].as_u64().unwrap() <= 2));
        !(d["scope_id"] == "root"
            && (missing || (d["path"] == path(run) && d["message"] == "unselected active run")))
    });
    projected.require_healthy()?;
    if plan["scope_id"] != "root" {
        return Err(Error::new("conflict", "not root initialization"));
    }
    Ok(())
}
fn verify_sources(v: &View, plan: &Value, root: &Path) -> Result<()> {
    let step = plan["step"].as_u64().unwrap();
    let run = root.join(format!(
        ".work/scopes/root/runs/{}/run.yaml",
        plan["new_run_id"].as_str().unwrap()
    ));
    if let Some(s) = v.capture.files.get(&run) {
        if wire::envelope("RunManifest", &s.raw, &run)? != manifest(plan) {
            return Err(Error::new("conflict", "frozen root manifest changed").at(run));
        }
    } else if step > 0 {
        return Err(Error::new("conflict", "installed root manifest disappeared").at(run));
    }
    let a = root.join(".work/scopes/root/scope.yaml");
    let current = snapshot(&v.capture, &a);
    let installed = v
        .capture
        .files
        .get(&a)
        .is_some_and(|s| s.raw == yaml_bytes(&anchor(plan)))
        || (current == plan["anchor_before"]
            && v.anchors.get("root").is_some_and(|a| *a == anchor(plan)));
    if (step >= 2 && !installed) || (step < 2 && !installed && current != plan["anchor_before"]) {
        return Err(Error::new("conflict", "root anchor differs from frozen initialization").at(a));
    }
    if !installed && let Some(s) = v.capture.files.get(&a) {
        let a = wire::envelope("ScopeAnchor", &s.raw, &a)?;
        if a["execution"]["mode"] != "uninitialized" {
            return Err(Error::new(
                "conflict",
                "initialization cannot rotate a selected round",
            ));
        }
    }
    let ignore = root.join(".gitignore");
    let before = snapshot(&v.capture, &ignore) == plan["ignore_change"]["before"];
    let after = v.capture.files.get(&ignore).is_some_and(|s| {
        hash(&s.raw) == plan["ignore_change"]["after"] && appended(&s.raw) == s.raw
    });
    if (step == 3 && !after) || (step < 3 && !before && !after) {
        return Err(Error::new(
            "conflict",
            "user configuration differs from frozen ignore fingerprints",
        )
        .at(ignore));
    }
    if before {
        let bytes = v
            .capture
            .files
            .get(&ignore)
            .map_or(&[][..], |s| s.raw.as_slice());
        if hash(&appended(bytes)) != plan["ignore_change"]["after"] {
            return Err(Error::new(
                "conflict",
                "frozen ignore result is not the exact housekeeping append",
            )
            .at(ignore));
        }
    } else if plan["ignore_change"]["before"]["exists"] == false
        && plan["ignore_change"]["after"] != hash(&appended(&[]))
    {
        return Err(Error::new(
            "conflict",
            "absent ignore source has a noncanonical append target",
        )
        .at(ignore));
    }
    Ok(())
}
fn known(known: &mut Value, key: &str, p: &Path) {
    let entry = json!({"id":null,"path":path(p)});
    if !known[key].as_array().unwrap().contains(&entry) {
        known[key].as_array_mut().unwrap().push(entry);
    }
}
fn ensure(c: &mut Capture, p: &Path, effects: &mut Value) -> Result<()> {
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
            known(effects, "created", d);
        }
    }
    result.map(|_| ())
}
// Only this operation's deterministic stages, with expected displaced bytes, qualify.
fn cleanup(c: &mut Capture, p: &Path, expected: &Value) -> Result<()> {
    let d = Directory::open(p.parent().unwrap())?;
    let name = p.file_name().unwrap().to_str().unwrap();
    let Some(s) = d.optional(name)? else {
        return Ok(());
    };
    if expected["exists"] != true || expected["sha256"] != hash(&s.raw) {
        return Err(Error::new(
            "conflict",
            "initialization artifact differs from its frozen displaced source",
        )
        .at(p));
    }
    c.recheck()?;
    d.verify()?;
    if d.read(name)? != s {
        return Err(Error::new("conflict", "initialization artifact changed during cleanup").at(p));
    }
    rustix::fs::unlinkat(&d.file, name, rustix::fs::AtFlags::empty())
        .map_err(|e| Error::new("io", e.to_string()).at(p))?;
    d.sync()?;
    d.verify()?;
    c.recheck()
}
fn stage(parent: &Path, op: &str, label: &str) -> PathBuf {
    parent.join(format!(".stage-{op}-{label}"))
}
fn save(
    c: &mut Capture,
    p: &Path,
    bytes: &[u8],
    op: &str,
    label: &str,
    effects: &mut Value,
) -> Result<()> {
    if c.files.get(p).is_some_and(|s| s.raw == bytes) {
        return Ok(());
    }
    let existed = c.files.contains_key(p);
    ensure(c, p.parent().unwrap(), effects)?;
    let s = stage(p.parent().unwrap(), op, label);
    c.publish_named(p, bytes, s.file_name().unwrap().to_str().unwrap())
        .map_err(|mut e| {
            if std::fs::symlink_metadata(&s).is_ok() {
                let mut paths = e.details["recovery_paths"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let entry = json!(path(&s));
                if !paths.contains(&entry) {
                    paths.push(entry);
                }
                e.details["recovery_paths"] = json!(paths);
            }
            e
        })?;
    known(effects, if existed { "updated" } else { "created" }, p);
    Ok(())
}
pub fn status(project: &Project, physical: bool) -> Result<Value> {
    let (v, g) = read_view(project, physical)?;
    if let Some(g) = g {
        g.verify()?;
    }
    v.capture.recheck()?;
    Ok(v.wrap(json!({})))
}
pub fn init(project: &Project, input: &Value) -> Result<Value> {
    wire::validate("ExecutionInitInput", input)?;
    if input["view"] == "physical" {
        return Err(Error::new(
            "invalid_argument",
            "physical-copy mode is read-only",
        ));
    }
    let mut v = View::load(project, false)?;
    let root = v
        .hosts
        .get("root")
        .and_then(|h| h.clone())
        .ok_or_else(|| Error::new("host_missing", "primary root unavailable"))?;
    let ignore = root.join(".gitignore");
    v.capture.file(&ignore)?;
    let continued = input["continue_id"].as_str();
    let (progress, mut plan) = if let Some(id) = continued {
        pending(&v, id, &root)?
    } else {
        v.require_healthy()?;
        let current = v
            .anchors
            .get("root")
            .and_then(|a| a["execution"]["run_id"].as_str());
        let (rid, time) = if let Some(id) = current {
            let p = root.join(format!(".work/scopes/root/runs/{id}/run.yaml"));
            let s =
                v.capture.files.get(&p).ok_or_else(|| {
                    Error::new("source_unavailable", "root manifest missing").at(&p)
                })?;
            let m = wire::envelope("RunManifest", &s.raw, &p)?;
            if m["phase"] != "active" {
                return Err(Error::new(
                    "invalid_source",
                    "init never rotates a terminal root",
                ));
            }
            (id.to_owned(), m["created_at"].as_str().unwrap().to_owned())
        } else {
            (new_id()?, now_timestamp())
        };
        let op = new_id()?;
        let bytes = v
            .capture
            .files
            .get(&ignore)
            .map_or(&[][..], |s| s.raw.as_slice());
        let plan = json!({"format_version":2,"id":op,"created_at":now_timestamp(),"scope_id":"root","new_run_id":rid,"run_created_at":time,"anchor_before":snapshot(&v.capture,&root.join(".work/scopes/root/scope.yaml")),"ignore_change":{"before":snapshot(&v.capture,&ignore),"after":hash(&appended(bytes))},"step":0});
        (
            root.join(format!(".work/scopes/root/progress/start/{op}.yaml")),
            plan,
        )
    };
    wire::validate("StartProgress", &plan)?;
    let op = plan["id"].as_str().unwrap().to_owned();
    let dirs = required(&root, plan["new_run_id"].as_str().unwrap());
    preflight(&root, &dirs)?;
    let checkout_lock = root.join(".work/operations.lock");
    if let Ok(work) = Directory::open(&root.join(".work"))
        && work.optional("operations.lock")?.is_none()
    {
        publication_access(&root, &checkout_lock)?;
    }
    let run = root.join(format!(
        ".work/scopes/root/runs/{}/run.yaml",
        plan["new_run_id"].as_str().unwrap()
    ));
    healthy_for_plan(&v, &plan, &progress, &run)?;
    verify_sources(&v, &plan, &root)?;
    let coord = project.git_common_dir.join("work-beta2/coordination.lock");
    if !v.capture.files.contains_key(&coord) {
        publication_access(&project.git_common_dir, &coord)?;
    }
    let already_complete = continued.is_none()
        && v.execution()["state"] == "ready"
        && snapshot(&v.capture, &ignore)["sha256"] == plan["ignore_change"]["after"]
        && dirs.iter().all(|p| Directory::open(p).is_ok());
    if !already_complete {
        publication_access(&root, &progress)?;
        if !v.capture.files.contains_key(&run) {
            publication_access(&root, &run)?;
        }
        if v.anchors.get("root") != Some(&anchor(&plan)) {
            publication_access(&root, &root.join(".work/scopes/root/scope.yaml"))?;
        }
        if snapshot(&v.capture, &ignore)["sha256"] != plan["ignore_change"]["after"] {
            publication_access(&root, &ignore)?;
        }
    }
    v.capture.recheck()?;
    let mut effects = partial();
    let result = (|| -> Result<Value> {
        let coord_path = project.git_common_dir.join("work-beta2/coordination.lock");
        let coord_before = std::fs::symlink_metadata(&coord_path).is_ok();
        let coord_dir = coord_path.parent().unwrap();
        let dir_before = std::fs::symlink_metadata(coord_dir).is_ok();
        let coordination_result = coordination(&v, true, true);
        if !dir_before && Directory::open(coord_dir).is_ok() {
            known(&mut effects, "created", coord_dir);
        }
        if !coord_before && std::fs::symlink_metadata(&coord_path).is_ok() {
            known(&mut effects, "created", &coord_path);
        }
        let guard: Locked = coordination_result?.unwrap();
        let mut fresh = View::load(project, false)?;
        fresh.capture.file(&ignore)?;
        v.capture.compare_prelock(&fresh.capture)?;
        v = fresh;
        verify_sources(&v, &plan, &root)?;
        healthy_for_plan(&v, &plan, &progress, &run)?;
        ensure(&mut v.capture, &root.join(".work"), &mut effects)?;
        let lock_path = root.join(".work/operations.lock");
        let lock_before = std::fs::symlink_metadata(&lock_path).is_ok();
        let checkout_result = CheckoutGuard::open(&root);
        if !lock_before && std::fs::symlink_metadata(&lock_path).is_ok() {
            known(&mut effects, "created", &lock_path);
        }
        let checkout = checkout_result?;
        // operations.lock belongs to preparation, not a run.
        v = {
            let mut fresh = View::load(project, false)?;
            fresh.capture.file(&ignore)?;
            v.capture.compare_prelock(&fresh.capture)?;
            fresh
        };
        guard.verify()?;
        checkout.verify()?;
        let complete = v.execution()["state"] == "ready"
            && snapshot(&v.capture, &ignore)["sha256"] == plan["ignore_change"]["after"]
            && dirs.iter().all(|p| Directory::open(p).is_ok());
        if continued.is_none() && complete {
            return Ok(v.wrap(json!({"changed":!effects["created"].as_array().unwrap().is_empty(),"partial":effects})));
        }
        if !v.capture.files.contains_key(&progress) {
            save(
                &mut v.capture,
                &progress,
                &yaml_bytes(&plan),
                &op,
                "intent",
                &mut effects,
            )?;
        }
        for i in plan["step"].as_u64().unwrap()..3 {
            guard.verify()?;
            checkout.verify()?;
            if i == 0 {
                for d in &dirs {
                    ensure(&mut v.capture, d, &mut effects)?;
                }
                if !v.capture.files.contains_key(&run) {
                    save(
                        &mut v.capture,
                        &run,
                        &yaml_bytes(&manifest(&plan)),
                        &op,
                        "manifest",
                        &mut effects,
                    )?;
                }
            } else if i == 1 {
                let a = root.join(".work/scopes/root/scope.yaml");
                // Configuration-only init preserves the original anchor bytes.
                let selected = v.capture.files.get(&a).is_some_and(|s| {
                    wire::envelope("ScopeAnchor", &s.raw, &a).is_ok_and(|a| a == anchor(&plan))
                });
                if !selected {
                    save(
                        &mut v.capture,
                        &a,
                        &yaml_bytes(&anchor(&plan)),
                        &op,
                        "anchor",
                        &mut effects,
                    )?;
                }
            } else {
                if v.capture
                    .files
                    .get(&ignore)
                    .is_none_or(|s| hash(&s.raw) != plan["ignore_change"]["after"])
                {
                    let after = appended(
                        v.capture
                            .files
                            .get(&ignore)
                            .map_or(&[][..], |s| s.raw.as_slice()),
                    );
                    if hash(&after) != plan["ignore_change"]["after"] {
                        return Err(Error::new("conflict", "ignore append changed").at(&ignore));
                    }
                    save(&mut v.capture, &ignore, &after, &op, "ignore", &mut effects)?;
                }
            }
            plan["step"] = json!(i + 1);
            save(
                &mut v.capture,
                &progress,
                &yaml_bytes(&plan),
                &op,
                &format!("progress-{i}"),
                &mut effects,
            )?;
        }
        // Verify all source postconditions, including a stale installed-step retry,
        // before removing any displaced evidence or the continuation plan.
        let mut final_view = View::load(project, false)?;
        final_view.capture.file(&ignore)?;
        verify_sources(&final_view, &plan, &root)?;
        healthy_for_plan(&final_view, &plan, &progress, &run)?;
        if dirs.iter().any(|p| Directory::open(p).is_err()) {
            return Err(Error::new(
                "conflict",
                "required initialization directory missing",
            ));
        }
        v.capture.recheck()?;
        guard.verify()?;
        checkout.verify()?;
        // Installed-before-response steps still require durable postconditions
        // on restart; parsed bytes alone are not evidence that a prior sync ran.
        for target in [
            &run,
            &root.join(".work/scopes/root/scope.yaml"),
            &ignore,
            &progress,
        ] {
            let d = Directory::open(target.parent().unwrap())?;
            d.sync_source(
                target.file_name().unwrap().to_str().unwrap(),
                final_view.capture.files.get(target).unwrap(),
            )?;
            d.sync()?;
            d.verify()?;
        }
        for directory in &dirs {
            let d = Directory::open(directory)?;
            d.sync()?;
            d.verify()?;
        }
        guard.verify()?;
        checkout.verify()?;
        v.capture.recheck()?;
        for (target, label, expected) in [
            (
                root.join(".work/scopes/root/scope.yaml"),
                "anchor",
                plan["anchor_before"].clone(),
            ),
            (
                ignore.clone(),
                "ignore",
                plan["ignore_change"]["before"].clone(),
            ),
        ] {
            cleanup(
                &mut v.capture,
                &stage(target.parent().unwrap(), &op, label),
                &expected,
            )?;
        }
        for i in 0..3 {
            let mut previous = plan.clone();
            previous["step"] = json!(i);
            let expected = json!({"exists":true,"sha256":hash(&yaml_bytes(&previous))});
            cleanup(
                &mut v.capture,
                &stage(progress.parent().unwrap(), &op, &format!("progress-{i}")),
                &expected,
            )?;
        }
        guard.verify()?;
        checkout.verify()?;
        v.capture.delete(&progress)?;
        known(&mut effects, "deleted", &progress);
        let final_view = View::load(project, false)?;
        final_view.require_healthy()?;
        final_view.capture.recheck()?;
        guard.verify()?;
        checkout.verify()?;
        if final_view.execution()["state"] != "ready" {
            return Err(Error::new(
                "invalid_source",
                "root initialization postconditions are not ready",
            ));
        }
        Ok(final_view.wrap(json!({"changed":true,"partial":effects})))
    })();
    result.map_err(|e| error_partial(e, effects, &op))
}

//! Shared storage presentation and request validation for CLI and MCP.
use super::cli::{CliError, encode_path};
use serde_json::{Map, Value, json};
use work::core::project::Project;
use work::core::storage::{
    Publication, RecoverRequest, RecreateRequest, Storage, StorageDiagnostic, StorageError,
    StorageErrorCode, StorageInspection, StorageOutcome, StorageState,
};

pub(super) const HELP: &str = "Usage: work [--json] [--worktree PATH] storage inspect|init|recreate|recover\ninspect: read health without writes\ninit: initialize a fresh store explicitly\nrecreate [--expected-store-id ID] [--expected-generation ID] --executors-stopped --acknowledge-loss [--all-clients-stopped]: retain prior state and create a fresh generation\nrecover OPERATION_ID [--executors-stopped] [--acknowledge-loss] [--all-clients-stopped]: resume a supported interrupted operation\nRecreation requires stopped executors and acknowledges lost live coordination. Missing root/lock also requires all clients stopped. Backup/restore is deferred.";

pub(super) enum Request {
    Inspect,
    Initialize,
    Recreate(RecreateRequest),
    Recover(String, RecoverRequest),
}

fn invalid(message: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", message)
}

fn valid_id(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 32
        && b.iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        && b[12] == b'4'
        && matches!(b[16], b'8' | b'9' | b'a' | b'b')
}

pub(super) fn from_cli(words: &[String]) -> Result<Request, CliError> {
    let Some(verb) = words.first().map(String::as_str) else {
        return Err(CliError::new("usage", HELP));
    };
    if !matches!(verb, "inspect" | "init" | "recreate" | "recover") {
        return Err(CliError::new("usage", "unknown storage command"));
    }
    let mut args = Map::new();
    let mut at = 1;
    if verb == "recover" {
        let id = words
            .get(at)
            .ok_or_else(|| invalid("missing operation ID"))?;
        args.insert("operation_id".into(), json!(id));
        at += 1;
    }
    while at < words.len() {
        let flag = words[at].as_str();
        let (key, takes_value) = match flag {
            "--expected-store-id" if verb == "recreate" => ("expected_store_id", true),
            "--expected-generation" if verb == "recreate" => ("expected_generation", true),
            "--executors-stopped" if matches!(verb, "recreate" | "recover") => {
                ("executors_stopped", false)
            }
            "--acknowledge-loss" if matches!(verb, "recreate" | "recover") => {
                ("acknowledge_loss", false)
            }
            "--all-clients-stopped" if matches!(verb, "recreate" | "recover") => {
                ("all_clients_stopped", false)
            }
            _ => return Err(invalid(format!("unknown storage argument {flag}"))),
        };
        if args.contains_key(key) {
            return Err(invalid(format!("duplicate {flag}")));
        }
        let value = if takes_value {
            at += 1;
            json!(
                words
                    .get(at)
                    .ok_or_else(|| invalid(format!("missing value for {flag}")))?
            )
        } else {
            json!(true)
        };
        args.insert(key.into(), value);
        at += 1;
    }
    from_fields(verb, &args)
}

/// Called after MCP's shape validation, before project discovery.
pub(super) fn from_fields(verb: &str, args: &Map<String, Value>) -> Result<Request, CliError> {
    let id = |name: &str| -> Result<Option<String>, CliError> {
        match args.get(name) {
            None => Ok(None),
            Some(Value::String(value)) if valid_id(value) => Ok(Some(value.clone())),
            _ => Err(invalid(format!("{name} must be a canonical UUIDv4"))),
        }
    };
    let flag = |name: &str| args.get(name).and_then(Value::as_bool).unwrap_or(false);
    match verb {
        "inspect" => Ok(Request::Inspect),
        "init" => Ok(Request::Initialize),
        "recreate" => {
            let expected_store_id = id("expected_store_id")?;
            let expected_generation = id("expected_generation")?;
            if !flag("executors_stopped") || !flag("acknowledge_loss") {
                return Err(invalid(
                    "recreation requires executors_stopped and acknowledge_loss",
                ));
            }
            Ok(Request::Recreate(RecreateRequest {
                expected_store_id,
                expected_generation,
                executors_stopped: true,
                acknowledge_loss: true,
                all_clients_stopped: flag("all_clients_stopped"),
            }))
        }
        "recover" => Ok(Request::Recover(
            id("operation_id")?.ok_or_else(|| invalid("missing operation_id"))?,
            RecoverRequest {
                executors_stopped: flag("executors_stopped"),
                acknowledge_loss: flag("acknowledge_loss"),
                all_clients_stopped: flag("all_clients_stopped"),
            },
        )),
        _ => Err(invalid("unknown storage operation")),
    }
}

pub(super) fn execute(project: &Project, request: Request) -> Result<Value, CliError> {
    let storage = Storage::new(project.clone());
    match request {
        Request::Inspect => storage.inspect().map(|s| json!({"storage":inspection(&s)})),
        Request::Initialize => storage.initialize().map(outcome),
        Request::Recreate(request) => storage.recreate(request).map(outcome),
        Request::Recover(id, request) => storage.recover(&id, request).map(outcome),
    }
    .map_err(error)
}

fn code(code: &StorageErrorCode) -> &'static str {
    code.code()
}

fn diagnostic(d: &StorageDiagnostic) -> Value {
    json!({"code":code(&d.code),"message":d.message,"path":d.path.as_deref().map(encode_path),"line":d.line})
}
pub(super) fn inspection(s: &StorageInspection) -> Value {
    json!({
        "state":match s.state { StorageState::Uninitialized=>"uninitialized",StorageState::Initialized=>"initialized",StorageState::RecoveryRequired=>"recovery_required" },
        "path":encode_path(&s.path),"identity_path":encode_path(&s.identity_path),"lock_path":encode_path(&s.lock_path),
        "metadata":s.metadata.as_ref().map(|m|json!({"format_version":m.format_version,"store_id":m.store_id,"recovery_generation":m.recovery_generation})),
        "retained_store_id":s.retained_store_id,"coordination_available":s.coordination_available,
        "storage_warning":s.storage_warning.as_ref().map(diagnostic),
        "diagnostics":s.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
        "pending_operations":s.pending_operations.iter().map(|p|json!({"id":p.id,"path":encode_path(&p.path),"kind":p.kind,"phase":p.phase,"supported":p.supported})).collect::<Vec<_>>()
    })
}
fn outcome(o: StorageOutcome) -> Value {
    json!({"changed":o.changed,"storage":inspection(&o.storage),"operation_id":o.operation_id,
        "recovery_paths":o.recovery_paths.iter().map(|p|encode_path(p)).collect::<Vec<_>>(),
        "loss":o.loss.as_ref().map(|l|json!({"coordination_reset":l.coordination_reset,"missing_or_damaged":l.missing_or_damaged,"prior_state_path":l.prior_state_path.as_deref().map(encode_path)}))})
}
fn error(e: StorageError) -> CliError {
    let details = json!({"path":e.path.as_deref().map(encode_path),"operation_id":e.operation_id,
        "publication":match e.publication {Publication::NotPublished=>"not_published",Publication::Possible=>"possible",Publication::Published=>"published"},
        "recovery_paths":e.recovery_paths.iter().map(|p|encode_path(p)).collect::<Vec<_>>(),
        "diagnostics":e.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),"errno":e.errno});
    CliError::with_details(code(&e.code), e.message, details)
}

pub(super) fn attach_read(
    project: &Project,
    mut value: Value,
    observed: Option<StorageInspection>,
) -> Value {
    // Preserve the inspection that selected a physical fallback, even if a
    // concurrent writer has released its lock before result presentation.
    let observation = match observed {
        Some(s) => Ok(s),
        None => Storage::new(project.clone()).inspect(),
    };
    match observation {
        Ok(s) => {
            value["storage_warning"] = s
                .storage_warning
                .as_ref()
                .map(diagnostic)
                .unwrap_or(Value::Null);
            value["storage"] = inspection(&s);
        }
        Err(e) => {
            value["storage_warning"] = json!({"code":code(&e.code),"message":e.message,"path":e.path.as_deref().map(encode_path),"line":null});
            value["storage"] = Value::Null;
        }
    }
    value
}

pub(super) fn print_warning(value: &Value) {
    if let Some(warning) = value
        .get("storage_warning")
        .filter(|w| !w.is_null())
        .or_else(|| {
            value
                .get("storage")
                .and_then(|s| s.get("storage_warning"))
                .filter(|w| !w.is_null())
        })
    {
        eprintln!(
            "work: {}: {}",
            warning["code"].as_str().unwrap_or("storage"),
            warning["message"].as_str().unwrap_or("storage unavailable")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    use work::core::{
        coordination::{CoordinationGuard, new_id},
        execution::ExecutionOperations,
        operations::{DurableOperations, MetadataChange},
        project::discover,
    };
    #[test]
    fn fallback_warning_survives_writer_release_before_presentation() {
        let root = std::env::temp_dir().join(format!("work-read-warning-{}", new_id().unwrap()));
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
        DurableOperations::new(&root)
            .create("item".into(), Vec::new(), MetadataChange::default())
            .unwrap();
        let project = discover(Some(&root)).unwrap();
        Storage::new(project.clone()).initialize().unwrap();
        let held = CoordinationGuard::acquire(&project, true).unwrap();
        let ops = ExecutionOperations::new(project.clone());
        let ready = ops.ready().unwrap();
        assert_eq!(ready.len(), 1);
        drop(held);
        assert!(
            Storage::new(project.clone())
                .inspect()
                .unwrap()
                .coordination_available
        );
        let presented = attach_read(
            &project,
            json!({"count":ready.len()}),
            ops.take_read_storage(),
        );
        assert_eq!(presented["storage_warning"]["code"], "storage_busy");
        assert_eq!(presented["storage"]["coordination_available"], false);
        assert!(ops.take_read_storage().is_none());
        fs::remove_dir_all(root).unwrap();
    }
}

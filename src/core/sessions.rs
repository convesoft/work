//! Run-scoped names are context, never ownership or process control.
//! @mara implements DES-CONTEXT-API
use super::context::envelope;
use super::coordination::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct NamedSession {
    pub id: String,
    pub name: String,
    pub session: SessionIdentity,
    pub value: Value,
    pub source: EntitySource,
}
pub(crate) fn directory(run: &str) -> PathBuf {
    Path::new("runs").join(run).join("sessions")
}
pub fn validate_availability(value: &Value) -> ExecutionResult<()> {
    exact_keys(value, &["state", "observed_at"], &[])
        .map_err(|e| ExecutionError::new("invalid_argument", e.message))?;
    if !matches!(
        value["state"].as_str(),
        Some("available" | "unavailable" | "unknown")
    ) || !value["observed_at"]
        .as_str()
        .is_some_and(super::claims::valid_timestamp)
    {
        return Err(ExecutionError::new(
            "invalid_argument",
            "availability requires state available|unavailable|unknown and RFC3339 observed_at",
        ));
    }
    Ok(())
}
pub(crate) fn load(g: &CoordinationGuard, run: &str) -> ExecutionResult<Vec<NamedSession>> {
    let dir = directory(run);
    let mut records = Vec::new();
    let mut names = BTreeSet::new();
    for name in g.names(&dir)? {
        let path = dir.join(&name);
        let absolute = g.root_path().join(&path);
        let result = (|| {
            let id = name
                .strip_suffix(".yaml")
                .filter(|s| valid_id(s))
                .ok_or_else(|| {
                    ExecutionError::new("invalid_format", "unexpected named-session entry")
                })?;
            let source = g.read(&path)?;
            let value = super::storage::format::yaml(&source.raw, &absolute)?;
            envelope(&value, g)?;
            exact_keys(
                &value,
                &[
                    "format_version",
                    "store_id",
                    "recovery_generation",
                    "id",
                    "run_id",
                    "name",
                    "session",
                ],
                &["availability"],
            )?;
            let record_name = string(&value, "name")?;
            if string(&value, "id")? != id
                || string(&value, "run_id")? != run
                || record_name.is_empty()
                || !names.insert(record_name.clone())
            {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "invalid or duplicate named-session identity",
                ));
            }
            let session = SessionIdentity::from_json(&value["session"])
                .map_err(|e| ExecutionError::new("invalid_format", e.message))?;
            if let Some(a) = value.get("availability") {
                validate_availability(a)
                    .map_err(|e| ExecutionError::new("invalid_format", e.message))?;
            }
            Ok(NamedSession {
                id: id.into(),
                name: record_name,
                session,
                value,
                source,
            })
        })();
        records.push(result.map_err(|e: ExecutionError| e.at(absolute))?);
    }
    Ok(records)
}
pub(crate) fn value(
    g: &CoordinationGuard,
    run: &str,
    id: &str,
    name: &str,
    session: &SessionIdentity,
    availability: Option<&Value>,
) -> Value {
    let mut value = json!({"format_version":1,"store_id":g.metadata.store_id,"recovery_generation":g.metadata.recovery_generation,"id":id,"run_id":run,"name":name,"session":session.to_json()});
    if let Some(a) = availability {
        value["availability"] = a.clone();
    }
    value
}

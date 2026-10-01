use super::super::coordination::{
    ExecutionError, ExecutionResult, exact_keys, parse_yaml, valid_id,
};
use super::super::storage::StoreMetadata;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Active,
    Squashing,
    Discarding,
    Finalized,
    Disposed,
}
impl RunPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Squashing => "squashing",
            Self::Discarding => "discarding",
            Self::Finalized => "finalized",
            Self::Disposed => "disposed",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finalized | Self::Disposed)
    }
    pub fn is_current(&self) -> bool {
        !self.is_terminal()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupKind {
    Squash,
    Discard,
}
impl CleanupKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Squash => "squash",
            Self::Discard => "discard",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanup {
    pub kind: CleanupKind,
    pub finalize: bool,
    pub item_ids: Vec<String>,
    pub session_ids: Vec<String>,
    pub started_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunManifest {
    pub format_version: u32,
    pub store_id: String,
    pub recovery_generation: String,
    pub id: String,
    pub root_item_id: String,
    pub created_at: String,
    pub phase: RunPhase,
    pub material_items: Vec<String>,
    pub default_workspace_id: Option<String>,
    pub output_workspace_id: Option<String>,
    pub ended_at: Option<String>,
    pub cleanup: Option<RunCleanup>,
}
fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::new("invalid_format", message)
}
fn text(value: &Value, key: &str) -> ExecutionResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("{key} must be a string")))
}
fn id(value: &Value, key: &str) -> ExecutionResult<String> {
    let value = text(value, key)?;
    if !valid_id(&value) {
        return Err(invalid(format!("{key} must be a full lowercase UUIDv4 ID")));
    }
    Ok(value)
}
fn optional_id(value: &Value, key: &str) -> ExecutionResult<Option<String>> {
    value.get(key).map(|_| id(value, key)).transpose()
}
fn ids(value: &Value, key: &str) -> ExecutionResult<Vec<String>> {
    let values = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{key} must be an ID array")))?;
    let mut result = std::collections::BTreeSet::new();
    for value in values {
        let id = value
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or_else(|| invalid(format!("{key} must contain full UUIDv4 IDs")))?;
        if !result.insert(id.to_owned()) {
            return Err(invalid(format!("{key} contains a duplicate ID")));
        }
    }
    Ok(result.into_iter().collect())
}
// Execution timestamps are RFC3339 UTC. Validate calendar and time components;
// lexical ordering is never used for membership or ownership.
fn timestamp(value: &Value, key: &str) -> ExecutionResult<String> {
    let text = text(value, key)?;
    let value = text
        .strip_suffix('Z')
        .or_else(|| text.strip_suffix("+00:00"))
        .ok_or_else(|| invalid(format!("{key} must be UTC RFC3339")))?;
    let b = value.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || !b[..19]
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit())
    {
        return Err(invalid(format!("{key} must be UTC RFC3339")));
    }
    if b.len() > 19 && (b[19] != b'.' || b.len() == 20 || !b[20..].iter().all(u8::is_ascii_digit)) {
        return Err(invalid(format!("{key} has invalid fractional seconds")));
    }
    let number = |a: usize, z: usize| {
        std::str::from_utf8(&b[a..z])
            .unwrap()
            .parse::<u32>()
            .unwrap()
    };
    let y = number(0, 4);
    let m = number(5, 7);
    let d = number(8, 10);
    let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if d == 0 || d > days || number(11, 13) > 23 || number(14, 16) > 59 || number(17, 19) > 60 {
        return Err(invalid(format!(
            "{key} has invalid calendar/time components"
        )));
    }
    Ok(text)
}
impl RunManifest {
    pub fn parse(raw: &[u8], metadata: &StoreMetadata, expected_id: &str) -> ExecutionResult<Self> {
        let value = parse_yaml(raw)?;
        match value.get("format_version").and_then(Value::as_u64) {
            Some(1) => {}
            Some(_) => {
                return Err(ExecutionError::new(
                    "unsupported_format",
                    "unsupported run manifest version",
                ));
            }
            None => return Err(invalid("format_version must be integer 1")),
        }
        exact_keys(
            &value,
            &[
                "format_version",
                "store_id",
                "recovery_generation",
                "id",
                "root_item_id",
                "created_at",
                "phase",
                "material_items",
            ],
            &[
                "default_workspace_id",
                "output_workspace_id",
                "ended_at",
                "cleanup",
            ],
        )?;
        let store_id = id(&value, "store_id")?;
        let recovery_generation = id(&value, "recovery_generation")?;
        if store_id != metadata.store_id || recovery_generation != metadata.recovery_generation {
            return Err(ExecutionError::new(
                "identity_mismatch",
                "run store identity/generation does not match the current foundation",
            ));
        }
        let parsed_id = id(&value, "id")?;
        if parsed_id != expected_id {
            return Err(invalid("run ID does not match directory"));
        }
        let root_item_id = id(&value, "root_item_id")?;
        let material_items = ids(&value, "material_items")?;
        if material_items.contains(&root_item_id) {
            return Err(invalid("root is excluded from material membership"));
        }
        let phase = match text(&value, "phase")?.as_str() {
            "active" => RunPhase::Active,
            "squashing" => RunPhase::Squashing,
            "discarding" => RunPhase::Discarding,
            "finalized" => RunPhase::Finalized,
            "disposed" => RunPhase::Disposed,
            _ => return Err(invalid("invalid run phase")),
        };
        let created_at = timestamp(&value, "created_at")?;
        let ended_at = value
            .get("ended_at")
            .map(|_| timestamp(&value, "ended_at"))
            .transpose()?;
        if phase.is_terminal() != ended_at.is_some() {
            return Err(invalid("ended_at is required only for terminal runs"));
        }
        let default_workspace_id = optional_id(&value, "default_workspace_id")?;
        let output_workspace_id = optional_id(&value, "output_workspace_id")?;
        let cleanup = value
            .get("cleanup")
            .map(|cleanup| -> ExecutionResult<RunCleanup> {
                exact_keys(
                    cleanup,
                    &["kind", "finalize", "item_ids", "session_ids", "started_at"],
                    &[],
                )?;
                let kind = match text(cleanup, "kind")?.as_str() {
                    "squash" => CleanupKind::Squash,
                    "discard" => CleanupKind::Discard,
                    _ => return Err(invalid("invalid cleanup kind")),
                };
                let finalize = cleanup
                    .get("finalize")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| invalid("cleanup finalize must be boolean"))?;
                let item_ids = ids(cleanup, "item_ids")?;
                let session_ids = ids(cleanup, "session_ids")?;
                let started_at = timestamp(cleanup, "started_at")?;
                if kind == CleanupKind::Squash && (!finalize || output_workspace_id.is_none()) {
                    return Err(invalid(
                        "squash requires finalize and captured output workspace",
                    ));
                }
                if !finalize && !session_ids.is_empty() {
                    return Err(invalid("subset discard has no session deletion set"));
                }
                Ok(RunCleanup {
                    kind,
                    finalize,
                    item_ids,
                    session_ids,
                    started_at,
                })
            })
            .transpose()?;
        if phase == RunPhase::Active && cleanup.is_some()
            || phase != RunPhase::Active && cleanup.is_none()
        {
            return Err(invalid(
                "cleanup is required exactly for cleanup/terminal phases",
            ));
        }
        if let Some(cleanup) = &cleanup {
            let expected = match phase {
                RunPhase::Squashing | RunPhase::Finalized => CleanupKind::Squash,
                RunPhase::Discarding | RunPhase::Disposed => CleanupKind::Discard,
                RunPhase::Active => unreachable!(),
            };
            if cleanup.kind != expected || phase.is_terminal() && !cleanup.finalize {
                return Err(invalid("cleanup does not match run phase"));
            }
            if cleanup.item_ids.contains(&root_item_id)
                || cleanup
                    .item_ids
                    .iter()
                    .any(|id| material_items.contains(id))
            {
                return Err(invalid("cleanup cannot delete material membership or root"));
            }
        }
        Ok(Self {
            format_version: 1,
            store_id,
            recovery_generation,
            id: parsed_id,
            root_item_id,
            created_at,
            phase,
            material_items,
            default_workspace_id,
            output_workspace_id,
            ended_at,
            cleanup,
        })
    }

    pub fn to_json(&self) -> Value {
        let mut members = self.material_items.clone();
        members.sort();
        members.dedup();
        let mut value = json!({"format_version":self.format_version,"store_id":self.store_id,"recovery_generation":self.recovery_generation,"id":self.id,"root_item_id":self.root_item_id,"created_at":self.created_at,"phase":self.phase.as_str(),"material_items":members});
        for (key, optional) in [
            ("default_workspace_id", &self.default_workspace_id),
            ("output_workspace_id", &self.output_workspace_id),
            ("ended_at", &self.ended_at),
        ] {
            if let Some(text) = optional {
                value[key] = json!(text);
            }
        }
        if let Some(cleanup) = &self.cleanup {
            let mut item_ids = cleanup.item_ids.clone();
            item_ids.sort();
            item_ids.dedup();
            let mut session_ids = cleanup.session_ids.clone();
            session_ids.sort();
            session_ids.dedup();
            value["cleanup"] = json!({"kind":cleanup.kind.as_str(),"finalize":cleanup.finalize,"item_ids":item_ids,"session_ids":session_ids,"started_at":cleanup.started_at});
        }
        value
    }
}

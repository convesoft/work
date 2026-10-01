use serde_json::Value;
use std::path::Path;

use super::{Claim, ClaimEnding, ClaimOutcome};
use crate::core::coordination::{
    ExecutionError, ExecutionResult, SessionIdentity, exact_keys, valid_id,
};

fn bad(message: impl Into<String>) -> ExecutionError {
    ExecutionError::new("invalid_format", message)
}
fn envelope(
    raw: &[u8],
    path: &Path,
    required: &[&str],
    optional: &[&str],
) -> ExecutionResult<Value> {
    let value = crate::core::storage::format::yaml(raw, path)?;
    let version = value
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad("format_version must be an integer"))?;
    if version != 1 {
        return Err(ExecutionError::new(
            "unsupported_format",
            format!("unsupported claim format {version}"),
        ));
    }
    exact_keys(&value, required, optional)?;
    Ok(value)
}
fn text(value: &Value, key: &str, nonempty: bool) -> ExecutionResult<String> {
    let text = value[key]
        .as_str()
        .ok_or_else(|| bad(format!("{key} must be text")))?;
    if nonempty && text.is_empty() {
        return Err(bad(format!("{key} must be nonempty")));
    }
    Ok(text.to_owned())
}
fn id(value: &Value, key: &str) -> ExecutionResult<String> {
    let id = text(value, key, true)?;
    if !valid_id(&id) {
        return Err(bad(format!("{key} must be a full lowercase entity ID")));
    }
    Ok(id)
}
fn optional_id(value: &Value, key: &str) -> ExecutionResult<Option<String>> {
    if value.get(key).is_some() {
        id(value, key).map(Some)
    } else {
        Ok(None)
    }
}

// Work emits UTC RFC3339. Validate recorded facts without ordering ownership by
// wall clock; accept fractional seconds and RFC3339 numeric UTC offsets too.
fn timestamp(value: &Value, key: &str) -> ExecutionResult<String> {
    let text = text(value, key, true)?;
    if !valid_timestamp(&text) {
        return Err(bad(format!("{key} must be RFC3339")));
    }
    Ok(text)
}
fn valid_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    let number = |start: usize, end: usize| -> Option<u32> {
        bytes[start..end].iter().try_fold(0, |n, b| {
            b.is_ascii_digit().then(|| n * 10 + u32::from(b - b'0'))
        })
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > days || hour > 23 || minute > 59 || second > 60 {
        return false;
    }
    let mut tail = &bytes[19..];
    if tail.first() == Some(&b'.') {
        tail = &tail[1..];
        let digits = tail.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return false;
        }
        tail = &tail[digits..];
    }
    if matches!(tail, [b'Z'] | [b'z']) {
        return true;
    }
    matches!(tail, [b'+' | b'-', h1, h2, b':', m1, m2]
        if [h1,h2,m1,m2].iter().all(|b| b.is_ascii_digit())
        && (h1-b'0')*10+(h2-b'0') <= 23 && (m1-b'0')*10+(m2-b'0') <= 59)
}

pub(super) fn claim(raw: &[u8], path: &Path) -> ExecutionResult<Claim> {
    let value = envelope(
        raw,
        path,
        &[
            "format_version",
            "store_id",
            "recovery_generation",
            "id",
            "item_id",
            "actor",
            "session",
            "acquired_at",
        ],
        &["workspace_id", "run_id", "session_record_id"],
    )?;
    let session = SessionIdentity::from_json(&value["session"]).map_err(|e| bad(e.message))?;
    Ok(Claim {
        store_id: id(&value, "store_id")?,
        recovery_generation: id(&value, "recovery_generation")?,
        id: id(&value, "id")?,
        item_id: id(&value, "item_id")?,
        actor: text(&value, "actor", true)?,
        session,
        acquired_at: timestamp(&value, "acquired_at")?,
        workspace_id: optional_id(&value, "workspace_id")?,
        run_id: optional_id(&value, "run_id")?,
        session_record_id: optional_id(&value, "session_record_id")?,
    })
}
pub(super) fn ending(raw: &[u8], path: &Path) -> ExecutionResult<ClaimEnding> {
    let value = envelope(
        raw,
        path,
        &[
            "format_version",
            "store_id",
            "recovery_generation",
            "claim_id",
            "item_id",
            "ended_at",
            "actor",
            "reason",
            "outcome",
        ],
        &["recovery"],
    )?;
    let outcome = match value["outcome"].as_str() {
        Some("released") => ClaimOutcome::Released,
        Some("completed") => ClaimOutcome::Completed,
        Some("reassigned") => ClaimOutcome::Reassigned,
        _ => return Err(bad("outcome must be released, completed or reassigned")),
    };
    let recovery = match value.get("recovery") {
        None => false,
        Some(Value::Bool(true)) => true,
        _ => return Err(bad("optional recovery must be true")),
    };
    let reason = text(&value, "reason", recovery)?;
    Ok(ClaimEnding {
        store_id: id(&value, "store_id")?,
        recovery_generation: id(&value, "recovery_generation")?,
        claim_id: id(&value, "claim_id")?,
        item_id: id(&value, "item_id")?,
        ended_at: timestamp(&value, "ended_at")?,
        actor: text(&value, "actor", true)?,
        reason,
        outcome,
        recovery,
    })
}

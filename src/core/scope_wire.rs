//! Frozen beta.2 envelope/input validation. This is not an execution engine.
use super::coordination::{ExecutionError as Error, ExecutionResult as Result, valid_id};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::OnceLock;

pub fn catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../docs/schemas/beta2-wire-v2.schema.json"))
            .expect("checked wire catalog")
    })
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// Canonical frozen digest for generated or envelope-validated inventories.
pub(crate) fn inventory_hash(inventory: &Value) -> String {
    let mut scopes = inventory["scope_ids"].as_array().unwrap().clone();
    scopes.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    let mut files = inventory["files"].as_array().unwrap().clone();
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    // Explicit aggregate key order; FileMark keys are path,sha256 in serde's map.
    let canonical = format!(
        "{{\"scope_ids\":{},\"files\":{}}}",
        serde_json::to_string(&scopes).unwrap(),
        serde_json::to_string(&files).unwrap()
    );
    hash(canonical.as_bytes())
}
pub fn validate(name: &str, value: &Value) -> Result<()> {
    check(&catalog()["$defs"][name], value)
        .map_err(|message| Error::new("invalid_argument", format!("{name}: {message}")))
}
pub(crate) fn envelope(name: &str, raw: &[u8], path: &Path) -> Result<Value> {
    if raw.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(Error::new("invalid_format", "BOM is forbidden").at(path));
    }
    let value = super::storage::format::yaml(raw, path)?;
    if value["format_version"].as_u64().is_some_and(|v| v != 2) {
        return Err(Error::new("unsupported_format", "expected beta.2 format_version 2").at(path));
    }
    validate(name, &value).map_err(|e| Error::new("invalid_format", e.message).at(path))?;
    Ok(value)
}
fn check(s: &Value, v: &Value) -> std::result::Result<(), String> {
    if let Some(reference) = s["$ref"].as_str() {
        return check(
            catalog()
                .pointer(&reference[1..])
                .ok_or("unknown schema reference")?,
            v,
        );
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        return Err("constant mismatch".into());
    }
    if let Some(e) = s["enum"].as_array()
        && !e.contains(v)
    {
        return Err("enum mismatch".into());
    }
    if let Some(t) = s["type"].as_str() {
        let valid = match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => false,
        };
        if !valid {
            return Err(format!("expected {t}"));
        }
    }
    for kind in ["allOf", "anyOf", "oneOf"] {
        if let Some(a) = s[kind].as_array() {
            let n = a.iter().filter(|x| check(x, v).is_ok()).count();
            if (kind == "allOf" && n != a.len())
                || (kind == "anyOf" && n == 0)
                || (kind == "oneOf" && n != 1)
            {
                return Err(format!("{kind} mismatch"));
            }
        }
    }
    if let Some(n) = s.get("not")
        && check(n, v).is_ok()
    {
        return Err("prohibited combination".into());
    }
    if let Some(condition) = s.get("if") {
        let branch = if check(condition, v).is_ok() {
            "then"
        } else {
            "else"
        };
        if let Some(b) = s.get(branch) {
            check(b, v)?;
        }
    }
    if let Some(m) = v.as_object() {
        if let Some(r) = s["required"].as_array() {
            for k in r {
                if !m.contains_key(k.as_str().unwrap()) {
                    return Err(format!("missing {k}"));
                }
            }
        }
        if s["minProperties"]
            .as_u64()
            .is_some_and(|n| m.len() < n as usize)
        {
            return Err("too few properties".into());
        }
        for (k, x) in m {
            if let Some(p) = s["properties"].get(k) {
                check(p, x).map_err(|e| format!("{k}: {e}"))?;
            } else if s["additionalProperties"] == false {
                return Err(format!("unknown {k}"));
            } else if s["additionalProperties"].is_object() {
                check(&s["additionalProperties"], x)?;
            }
            if let Some(p) = s.get("propertyNames") {
                check(p, &Value::String(k.clone()))?;
            }
        }
    }
    if let Some(a) = v.as_array() {
        if s["minItems"].as_u64().is_some_and(|n| a.len() < n as usize)
            || s["maxItems"].as_u64().is_some_and(|n| a.len() > n as usize)
        {
            return Err("array length".into());
        }
        if s["uniqueItems"] == true && a.iter().enumerate().any(|(i, x)| a[..i].contains(x)) {
            return Err("duplicate array value".into());
        }
        if let Some(item) = s.get("items") {
            for x in a {
                check(item, x)?;
            }
        }
    }
    if let Some(n) = v.as_f64()
        && (s["minimum"].as_f64().is_some_and(|b| n < b)
            || s["maximum"].as_f64().is_some_and(|b| n > b))
    {
        return Err("number range".into());
    }
    if let Some(text) = v.as_str() {
        if s["minLength"]
            .as_u64()
            .is_some_and(|n| text.chars().count() < n as usize)
            || s["maxLength"]
                .as_u64()
                .is_some_and(|n| text.chars().count() > n as usize)
        {
            return Err("string length".into());
        }
        if let Some(pattern) = s["pattern"].as_str()
            && !matches_pattern(pattern, text)
        {
            return Err("string grammar".into());
        }
        if s["format"] == "date-time" && !time(text) {
            return Err("invalid UTC time".into());
        }
    }
    Ok(())
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn template(s: &str) -> bool {
    s.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn reference(s: &str) -> bool {
    let s = s.strip_prefix("w-").unwrap_or(s);
    (!s.is_empty()
        && s.len() < 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        || valid_id(s)
}
fn time(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b.last() != Some(&b'Z')
    {
        return false;
    }
    if ![0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
        .iter()
        .all(|r| b[r.clone()].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    if b.len() != 20
        && (b[19] != b'.' || b.len() < 22 || !b[20..b.len() - 1].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let n = |a, z| {
        std::str::from_utf8(&b[a..z])
            .unwrap()
            .parse::<u32>()
            .unwrap()
    };
    let y = n(0, 4);
    let m = n(5, 7);
    let d = n(8, 10);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    };
    d >= 1 && d <= days && n(11, 13) < 24 && n(14, 16) < 60 && n(17, 19) <= 60
}
fn matches_pattern(p: &str, s: &str) -> bool {
    match p {
        "^[0-9a-f]{12}4[0-9a-f]{3}[89ab][0-9a-f]{15}$" => valid_id(s),
        "^[0-9a-f]{64}$" => {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        "^(?=.*\\S)[^\\r\\n]+$" => !s.trim().is_empty() && !s.contains(['\r', '\n']),
        "^[A-Za-z][A-Za-z0-9_.-]{0,63}$" => name(s),
        "^[a-z][a-z0-9_]*$" => template(s),
        "^(?:w-)?(?:[0-9a-f]{1,31}|[0-9a-f]{12}4[0-9a-f]{3}[89ab][0-9a-f]{15})$" => reference(s),
        "^(?:[0-9a-f]{12}4[0-9a-f]{3}[89ab][0-9a-f]{15}/)?[A-Za-z][A-Za-z0-9_.-]{0,63}$" => {
            match s.split_once('/') {
                Some((r, n)) => valid_id(r) && name(n),
                None => name(s),
            }
        }
        "^(?:[0-9a-f]{2})*$" => {
            s.len().is_multiple_of(2)
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        "^w-[0-9a-f]{8,32}$" => s.strip_prefix("w-").is_some_and(|x| {
            (8..=32).contains(&x.len())
                && x.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }),
        "^(?:root|(?:local|existing):[a-z][a-z0-9_]*)$" => {
            s == "root"
                || s.split_once(':')
                    .is_some_and(|(k, v)| ["local", "existing"].contains(&k) && template(v))
        }
        "^/(?:[A-Za-z0-9/._-]|%[0-9A-F]{2})*$" => {
            if !s.starts_with('/') {
                return false;
            }
            let b = s.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%' {
                    if i + 2 >= b.len()
                        || !b[i + 1..i + 3]
                            .iter()
                            .all(|x| x.is_ascii_digit() || (b'A'..=b'F').contains(x))
                    {
                        return false;
                    }
                    i += 3;
                } else if b[i].is_ascii_alphanumeric() || b"/._-".contains(&b[i]) {
                    i += 1;
                } else {
                    return false;
                }
            }
            true
        }
        "^(?!/)(?!.*(?:^|/)\\.{1,2}(?:/|$))[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*$" => {
            !s.is_empty()
                && s.split('/').all(|x| {
                    !x.is_empty()
                        && x != "."
                        && x != ".."
                        && x.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                })
        }
        "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\\.[0-9]+)?Z$" => time(s),
        _ => false, // A changed catalog grammar must gain an explicit implementation.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inventory_digest_uses_sorted_arrays_and_exact_frozen_key_order() {
        let a = "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb";
        let zero = "0".repeat(64);
        let one = "1".repeat(64);
        let expected = format!(
            "{{\"scope_ids\":[\"{a}\",\"{b}\"],\"files\":[{{\"path\":\".work/items/{a}.md\",\"sha256\":\"{zero}\"}},{{\"path\":\".work/items/{b}.md\",\"sha256\":\"{one}\"}}]}}"
        );
        let inventory = json!({"scope_ids":[b,a],"files":[
            {"path":format!(".work/items/{b}.md"),"sha256":one},
            {"path":format!(".work/items/{a}.md"),"sha256":zero}
        ],"sha256":hash(expected.as_bytes())});
        validate("HostedInventory", &inventory).unwrap();
        assert_eq!(inventory_hash(&inventory), hash(expected.as_bytes()));
        let mut changed = inventory.clone();
        changed["files"][0]["sha256"] = json!("2".repeat(64));
        assert_ne!(inventory_hash(&changed), inventory_hash(&inventory));
        changed = inventory.clone();
        changed["scope_ids"] = json!([a]);
        assert_ne!(inventory_hash(&changed), inventory_hash(&inventory));
    }
}

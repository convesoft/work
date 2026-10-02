//! One filter spelling, parser and schema for item list/ready and claim-next.
use serde_json::{Map, Value, json};
use work::core::selection::{Persistence, SelectionFilters};

use super::cli::CliError;
pub(super) const FIELDS: &[&str] = &[
    "root",
    "run_id",
    "labels_all",
    "priority_max",
    "persistence",
];
fn invalid(message: impl Into<String>) -> CliError {
    CliError::new("invalid_argument", message)
}

pub(super) fn consume_cli(
    words: &[String],
    at: &mut usize,
    fields: &mut Map<String, Value>,
) -> Result<bool, CliError> {
    let flag = words[*at].as_str();
    let key = match flag {
        "--root" => "root",
        "--run" => "run_id",
        "--label" => "labels_all",
        "--priority-max" => "priority_max",
        "--persistence" => "persistence",
        _ => return Ok(false),
    };
    let value = words
        .get(*at + 1)
        .ok_or_else(|| invalid(format!("missing {flag} value")))?;
    *at += 2;
    if key == "labels_all" {
        fields
            .entry(key)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!(value));
    } else {
        let value = if key == "priority_max" {
            json!(
                value
                    .parse::<u8>()
                    .map_err(|_| invalid("priority_max must be 0–4"))?
            )
        } else {
            json!(value)
        };
        if fields.insert(key.into(), value).is_some() {
            return Err(invalid(format!("duplicate {flag}")));
        }
    }
    Ok(true)
}
pub(super) fn from_cli(words: &[String]) -> Result<SelectionFilters, CliError> {
    let mut fields = Map::new();
    let mut at = 0;
    while at < words.len() {
        if !consume_cli(words, &mut at, &mut fields)? {
            return Err(invalid(format!("unknown filter {}", words[at])));
        }
    }
    parse(&fields)
}
pub(super) fn parse(fields: &Map<String, Value>) -> Result<SelectionFilters, CliError> {
    let text = |key: &str| -> Result<Option<String>, CliError> {
        fields
            .get(key)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid(format!("invalid {key}")))
            })
            .transpose()
    };
    let persistence = match text("persistence")?.as_deref() {
        None => None,
        Some("material") => Some(Persistence::Material),
        Some("wisp") => Some(Persistence::Wisp),
        _ => return Err(invalid("persistence must be material|wisp")),
    };
    let priority_max = fields
        .get("priority_max")
        .map(|value| {
            value
                .as_u64()
                .filter(|n| *n <= 4)
                .map(|n| n as u8)
                .ok_or_else(|| invalid("priority_max must be 0–4"))
        })
        .transpose()?;
    let labels_all = fields
        .get("labels_all")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| invalid("labels_all must be an array"))?
                .iter()
                .map(|label| {
                    label
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("labels_all must contain strings"))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let filters = SelectionFilters {
        root: text("root")?,
        run_id: text("run_id")?,
        labels_all,
        priority_max,
        persistence,
    };
    filters.validate(false)?;
    Ok(filters)
}
pub(super) fn properties() -> Value {
    json!({"root":{"type":"string"},"run_id":{"type":"string"},"labels_all":{"type":"array","items":{"type":"string"}},"priority_max":{"type":"integer","minimum":0,"maximum":4},"persistence":{"type":"string","enum":["material","wisp"]}})
}

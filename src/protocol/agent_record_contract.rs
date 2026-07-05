// EasyNet CLI — Agent record shared contract
// ===========================================
//
// File: src/protocol/agent_record_contract.rs
// Description: Shared SDK AgentRecord projection for daemon agent list rows.
//
// Protocol Responsibility
// -----------------------
// Own the language-neutral AgentRecord DTO projection used by Directory and
// Admin + Gateway SDK profiles. This module preserves daemon facts and derives
// owner refs only from valid Agent URAs.
//
// Implementation Approach
// -----------------------
// Accept daemon `agent.list` rows in either direct array or `{agents:[...]}`
// shape, validate typed fields, preserve nullable hosted-agent URAs, and
// project schema-shaped AgentRecord objects without leaking registry internals.
//
// Usage Contract
// --------------
// Callers pass daemon output only. Missing hosted-agent URAs remain null;
// malformed URAs, path-like names, and non-string ability entries are rejected.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK shared DTO layer. Profile clients wrap these records
// in profile-specific page/result envelopes.

use serde_json::{json, Map, Value};

use crate::core::ura;
use crate::protocol::sdk_contract::{
    first_optional_string_field, object, optional_bool_field, optional_string_array_field,
    optional_string_field, required_string, SdkContractError,
};

pub(crate) type AgentRecordError = SdkContractError;

pub(crate) fn project_agent_record_items_for_profile(
    input: &Value,
    profile: &'static str,
) -> Result<Vec<Value>, AgentRecordError> {
    let rows = agent_rows(input)?;
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        records.push(project_agent_row(row, profile)?);
    }
    Ok(records)
}

fn agent_rows(input: &Value) -> Result<&Vec<Value>, AgentRecordError> {
    if let Some(rows) = input.as_array() {
        return Ok(rows);
    }
    let obj = object(input, "AgentRowsInput")?;
    obj.get("agents")
        .and_then(Value::as_array)
        .ok_or(AgentRecordError::MissingField("agents"))
}

fn project_agent_row(row: &Value, profile: &'static str) -> Result<Value, AgentRecordError> {
    let obj = object(row, "AgentRow")?;
    let name = required_string(obj, "name")?;
    validate_agent_name(name, "name")?;
    let agent_ura = first_optional_string_field(obj, "ura", "agent_ura")?;
    let (owner_ura, device_ura) = match agent_ura.as_deref() {
        Some(agent_ura) => owner_refs_from_agent_ura(agent_ura)?,
        None => (None, None),
    };
    let runtime = required_string(obj, "runtime")?;
    let root_exists = optional_bool_field(obj, "root_exists")?.unwrap_or(true);
    let abilities = optional_string_array_field(obj, "abilities")?.unwrap_or_default();
    Ok(json!({
        "name": name,
        "agent_ura": agent_ura,
        "owner_ura": owner_ura,
        "device_ura": device_ura,
        "state": if root_exists { "registered" } else { "degraded" },
        "runtime": runtime,
        "model": optional_string_field(obj, "model")?,
        "label": optional_string_field(obj, "label")?,
        "abilities": abilities,
        "metadata": {
            "profile": profile,
            "source": "agent.list",
            "root_path": optional_string_field(obj, "root_path")?,
            "root_exists": root_exists,
            "timeout_secs": optional_u64(obj, "timeout_secs"),
        },
    }))
}

fn owner_refs_from_agent_ura(
    agent_ura: &str,
) -> Result<(Option<String>, Option<String>), AgentRecordError> {
    let parsed = ura::parse_ura(agent_ura)
        .map_err(|err| AgentRecordError::InvalidField("agent_ura", err.to_string()))?;
    if parsed.kind != ura::URAKind::Agent {
        return Err(AgentRecordError::InvalidField(
            "agent_ura",
            "must be an Agent URA".to_string(),
        ));
    }
    if let Some((user_id, _)) = parsed.agent_ids() {
        return Ok((Some(ura::user_ura(&parsed.realm, user_id)), None));
    }
    if let Some((device_id, _)) = parsed.device_agent_ids() {
        let device_ura = ura::device_ura(&parsed.realm, device_id);
        return Ok((Some(device_ura.clone()), Some(device_ura)));
    }
    Ok((None, None))
}

fn validate_agent_name(raw: &str, field: &'static str) -> Result<(), AgentRecordError> {
    if raw.trim().is_empty() {
        return Err(AgentRecordError::InvalidField(
            field,
            "must not be empty".to_string(),
        ));
    }
    if raw == "device" || raw.starts_with("device.") {
        return Err(AgentRecordError::InvalidField(
            field,
            "`device` is reserved for device-sponsored System Agents".to_string(),
        ));
    }
    if raw.contains('/') || raw.contains('\\') || raw.chars().any(char::is_whitespace) {
        return Err(AgentRecordError::InvalidField(
            field,
            "must be an owner-local agent id, not a path or whitespace token".to_string(),
        ));
    }
    Ok(())
}

fn optional_u64(obj: &Map<String, Value>, field: &'static str) -> Option<u64> {
    match obj.get(field) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(raw)) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_agent_records_preserves_missing_ura_as_null() {
        let input = json!({
            "agents": [{
                "name": "codex",
                "ura": null,
                "runtime": "codex",
                "model": "gpt-5",
                "label": "primary",
                "root_exists": true
            }]
        });

        let items = project_agent_record_items_for_profile(&input, "agent_record").unwrap();

        assert_eq!(items[0]["agent_ura"], Value::Null);
        assert_eq!(items[0]["owner_ura"], Value::Null);
        assert_eq!(items[0]["state"], "registered");
    }

    #[test]
    fn project_agent_records_derives_owner_ref_from_agent_ura() {
        let input = json!({
            "agents": [{
                "name": "codex",
                "ura": "easynet:///r/example/agent/alice.codex",
                "runtime": "codex",
                "root_exists": false,
                "abilities": ["chat.complete"]
            }]
        });

        let items = project_agent_record_items_for_profile(&input, "agent_record").unwrap();

        assert_eq!(items[0]["owner_ura"], "easynet:///r/example/user/alice");
        assert_eq!(items[0]["state"], "degraded");
        assert_eq!(items[0]["abilities"][0], "chat.complete");
    }

    #[test]
    fn project_agent_records_rejects_non_string_abilities() {
        let input = json!({
            "agents": [{
                "name": "codex",
                "ura": "easynet:///r/example/agent/alice.codex",
                "runtime": "codex",
                "abilities": [{"descriptor": "chat.complete"}]
            }]
        });

        let err = project_agent_record_items_for_profile(&input, "agent_record").unwrap_err();

        assert!(err.to_string().contains("array of strings"));
    }

    #[test]
    fn project_agent_records_uses_calling_profile_metadata() {
        let input = json!({
            "agents": [{
                "name": "codex",
                "ura": null,
                "runtime": "codex"
            }]
        });

        let items = project_agent_record_items_for_profile(&input, "directory_identity").unwrap();

        assert_eq!(items[0]["metadata"]["profile"], "directory_identity");
    }
}

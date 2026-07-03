// EasyNet CLI — Directory shared contract
// ========================================
//
// File: src/daemon/directory_contract.rs
// Description: Shared daemon SDK contract for Directory read-model carriers
//              and paginated device/agent/ability page projections.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Directory DTO projection for daemon read models.
// This module builds complete Invocation carriers for existing daemon
// read-model abilities and projects their outputs into stable paginated DTOs.
// It does not implement a second directory database, perform live distributed
// fan-out, own event subscriptions, or execute abilities.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for system abilities. Keep
// cursor pagination in one explicit state object so all list projections share
// the same DefaultPageSize/MaxPageSize, cursor parsing, and next-cursor rules.
//
// Usage Contract
// --------------
// Carrier construction requires explicit Invocation tuple fields. Directory
// list methods accept only SDK query fields and lower to daemon read-model
// abilities. Projection accepts daemon output JSON facts and returns bounded
// pages; malformed rows are rejected instead of silently skipped.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Directory profile. Runtime Core remains the only
// submit/open path for returned Invocation carriers; Events owns directory
// stream subscription DTOs.

use serde_json::{json, Map, Value};

use crate::core::ura::{self, URAKind};
use crate::daemon::agent_record_contract;
use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_bool_field, optional_string, optional_string_field,
    required_string, validate_ura, SdkContractError,
};

const DIRECTORY_PROFILE: &str = "directory_identity";
const DIRECTORY_SOURCE: &str = "read_model";
const ABILITY_NODE_LIST: &str = crate::daemon::ability::names::device_control::NODE_LIST;
const ABILITY_AGENT_LIST: &str = crate::daemon::ability::names::agents::AGENT_LIST;
const ABILITY_META_LIST_ABILITIES: &str =
    crate::daemon::ability::names::governance::META_LIST_ABILITIES;

pub(crate) const DIRECTORY_DEFAULT_PAGE_SIZE: usize = 50;
pub(crate) const DIRECTORY_MAX_PAGE_SIZE: usize = 500;

pub(crate) type DirectoryError = SdkContractError;

pub(crate) fn build_list_devices_invocation(request: &Value) -> Result<Value, DirectoryError> {
    let obj = object(request, "DirectoryListDevicesRequest")?;
    reject_unsupported_fields(obj, DIRECTORY_PAGE_REQUEST_FIELDS)?;
    let _ = PageControls::from_request(obj)?;
    build_system_invocation(obj, DIRECTORY_PROFILE, ABILITY_NODE_LIST, json!({}))
}

pub(crate) fn build_list_agents_invocation(request: &Value) -> Result<Value, DirectoryError> {
    let obj = object(request, "DirectoryListAgentsRequest")?;
    reject_unsupported_fields(obj, DIRECTORY_PAGE_REQUEST_FIELDS)?;
    let _ = PageControls::from_request(obj)?;
    build_system_invocation(obj, DIRECTORY_PROFILE, ABILITY_AGENT_LIST, json!({}))
}

pub(crate) fn build_list_abilities_invocation(request: &Value) -> Result<Value, DirectoryError> {
    let obj = object(request, "DirectoryListAbilitiesRequest")?;
    reject_unsupported_fields(obj, DIRECTORY_ABILITY_REQUEST_FIELDS)?;
    let _ = PageControls::from_request(obj)?;
    let args = list_abilities_args(obj)?;
    build_system_invocation(obj, DIRECTORY_PROFILE, ABILITY_META_LIST_ABILITIES, args)
}

pub(crate) fn project_device_page(input: &Value) -> Result<Value, DirectoryError> {
    let page_input = PageInput::parse(input)?;
    let rows = rows_from_value(page_input.result, "nodes", "devices", "DeviceRows")?;
    let page = page_input.controls.slice(rows)?;
    let mut items = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        items.push(project_device_row(row)?);
    }
    Ok(directory_page_json(
        "device_page",
        "device",
        ABILITY_NODE_LIST,
        page_input.controls.limit,
        page.next_cursor,
        items,
        rows.len(),
    ))
}

pub(crate) fn project_agent_page(input: &Value) -> Result<Value, DirectoryError> {
    let page_input = PageInput::parse(input)?;
    let records = agent_record_contract::project_agent_record_items_for_profile(
        page_input.result,
        DIRECTORY_PROFILE,
    )?;
    let page = page_input.controls.slice(&records)?;
    let items = page.rows.to_vec();
    Ok(directory_page_json(
        "agent_page",
        "agent",
        ABILITY_AGENT_LIST,
        page_input.controls.limit,
        page.next_cursor,
        items,
        records.len(),
    ))
}

pub(crate) fn project_ability_page(input: &Value) -> Result<Value, DirectoryError> {
    let page_input = PageInput::parse(input)?;
    let rows = rows_from_value(page_input.result, "abilities", "items", "AbilityRows")?;
    let page = page_input.controls.slice(rows)?;
    let mut items = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        items.push(project_ability_row(row)?);
    }
    Ok(directory_page_json(
        "ability_page",
        "ability",
        ABILITY_META_LIST_ABILITIES,
        page_input.controls.limit,
        page.next_cursor,
        items,
        rows.len(),
    ))
}

const COMMON_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "limit",
    "cursor",
];
const DIRECTORY_PAGE_REQUEST_FIELDS: &[&str] = COMMON_REQUEST_FIELDS;
const DIRECTORY_ABILITY_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "limit",
    "cursor",
    "scope",
    "owner_ura",
    "ability_ura",
];

fn reject_unsupported_fields(
    obj: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), DirectoryError> {
    for key in obj.keys() {
        if !allowed.iter().any(|allowed| allowed == key) {
            return Err(DirectoryError::InvalidField(
                "request",
                format!("unsupported field `{key}`"),
            ));
        }
    }
    Ok(())
}

fn list_abilities_args(obj: &Map<String, Value>) -> Result<Value, DirectoryError> {
    let mut args = Map::new();
    if let Some(scope) = optional_string_field(obj, "scope")? {
        match scope.as_str() {
            "local" | "realm" => {
                args.insert("scope".to_string(), Value::String(scope));
            }
            _ => {
                return Err(DirectoryError::InvalidField(
                    "scope",
                    "must be `local` or `realm`".to_string(),
                ));
            }
        }
    }
    if let Some(owner_ura) = optional_string_field(obj, "owner_ura")? {
        validate_owner_ura(&owner_ura, "owner_ura")?;
        args.insert("agent_ura".to_string(), Value::String(owner_ura));
    }
    if let Some(ability_ura) = optional_string_field(obj, "ability_ura")? {
        validate_ability_ura(&ability_ura, "ability_ura")?;
        args.insert("subject_ura".to_string(), Value::String(ability_ura));
    }
    Ok(Value::Object(args))
}

#[derive(Debug, Clone, Copy)]
struct PageControls {
    limit: usize,
    offset: usize,
}

impl PageControls {
    fn from_request(obj: &Map<String, Value>) -> Result<Self, DirectoryError> {
        let limit = optional_usize(obj, "limit")?.unwrap_or(DIRECTORY_DEFAULT_PAGE_SIZE);
        validate_limit(limit)?;
        let offset = optional_cursor_offset(obj, "cursor")?.unwrap_or(0);
        Ok(Self { limit, offset })
    }

    fn slice<'a, T>(&self, rows: &'a [T]) -> Result<PageSlice<'a, T>, DirectoryError> {
        if self.offset > rows.len() {
            return Err(DirectoryError::InvalidField(
                "cursor",
                "must not point past the current read-model snapshot".to_string(),
            ));
        }
        let end = self.offset.saturating_add(self.limit).min(rows.len());
        let next_cursor = if end < rows.len() {
            Some(end.to_string())
        } else {
            None
        };
        Ok(PageSlice {
            rows: &rows[self.offset..end],
            next_cursor,
        })
    }
}

struct PageSlice<'a, T> {
    rows: &'a [T],
    next_cursor: Option<String>,
}

struct PageInput<'a> {
    result: &'a Value,
    controls: PageControls,
}

impl<'a> PageInput<'a> {
    fn parse(input: &'a Value) -> Result<Self, DirectoryError> {
        let Some(obj) = input.as_object() else {
            return Ok(Self {
                result: input,
                controls: PageControls {
                    limit: DIRECTORY_DEFAULT_PAGE_SIZE,
                    offset: 0,
                },
            });
        };
        if let Some(result) = obj.get("result").filter(|value| !value.is_null()) {
            return Ok(Self {
                result,
                controls: PageControls::from_request(obj)?,
            });
        }
        Ok(Self {
            result: input,
            controls: PageControls::from_request(obj)?,
        })
    }
}

fn validate_limit(limit: usize) -> Result<(), DirectoryError> {
    if limit == 0 || limit > DIRECTORY_MAX_PAGE_SIZE {
        return Err(DirectoryError::InvalidField(
            "limit",
            format!("must be between 1 and {DIRECTORY_MAX_PAGE_SIZE}"),
        ));
    }
    Ok(())
}

fn optional_usize(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, DirectoryError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| DirectoryError::InvalidField(field, "must be unsigned".to_string())),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| DirectoryError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(DirectoryError::InvalidField(
            field,
            "must be an integer or decimal string".to_string(),
        )),
    }
}

fn optional_cursor_offset(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, DirectoryError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.starts_with('-') || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(DirectoryError::InvalidField(
                    field,
                    "must be a non-negative decimal offset cursor".to_string(),
                ));
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| DirectoryError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(DirectoryError::InvalidField(
            field,
            "must be a cursor string".to_string(),
        )),
    }
}

fn rows_from_value<'a>(
    value: &'a Value,
    primary: &'static str,
    fallback: &'static str,
    name: &'static str,
) -> Result<&'a Vec<Value>, DirectoryError> {
    if let Some(rows) = value.as_array() {
        return Ok(rows);
    }
    let obj = object(value, name)?;
    obj.get(primary)
        .or_else(|| obj.get(fallback))
        .and_then(Value::as_array)
        .ok_or(DirectoryError::MissingField(primary))
}

fn project_device_row(row: &Value) -> Result<Value, DirectoryError> {
    let obj = object(row, "DeviceRow")?;
    let node_id = required_string(obj, "node_id")?;
    let device_ura = optional_string_field(obj, "agent_ura")?;
    if let Some(device_ura) = device_ura.as_deref() {
        validate_ura(device_ura, "agent_ura")?;
    }
    let abilities = optional_string_array(obj, "abilities")?.unwrap_or_default();
    Ok(json!({
        "profile": DIRECTORY_PROFILE,
        "kind": "device",
        "node_id": node_id,
        "device_ura": device_ura,
        "state": optional_string(obj, "state").unwrap_or_else(|| "unknown".to_string()),
        "online": optional_bool_field(obj, "online")?,
        "is_self": optional_bool_field(obj, "is_self")?.unwrap_or(false),
        "paired": optional_bool_field(obj, "paired")?,
        "tenant_id": optional_string_field(obj, "tenant_id")?,
        "hub_endpoint": optional_string_field(obj, "hub_endpoint")?,
        "probe_status": optional_string_field(obj, "probe_status")?,
        "probe_error": optional_string_field(obj, "probe_error")?,
        "latency_ms": optional_u64(obj, "latency_ms"),
        "abilities": abilities,
        "metadata": {
            "profile": DIRECTORY_PROFILE,
            "source": ABILITY_NODE_LIST,
            "raw_node": row,
        },
    }))
}

fn project_ability_row(row: &Value) -> Result<Value, DirectoryError> {
    let obj = object(row, "AbilityRow")?;
    let name = required_string(obj, "name")?;
    let ability_ura = optional_string_field(obj, "ability_ura")?;
    if let Some(ability_ura) = ability_ura.as_deref() {
        validate_ability_ura(ability_ura, "ability_ura")?;
    }
    let owner_ura = optional_string_field(obj, "owner_ura")?;
    if let Some(owner_ura) = owner_ura.as_deref() {
        validate_owner_ura(owner_ura, "owner_ura")?;
    }
    Ok(json!({
        "profile": DIRECTORY_PROFILE,
        "kind": "ability",
        "name": name,
        "ability_ura": ability_ura,
        "owner_ura": owner_ura,
        "descriptor_ref": optional_string_field(obj, "descriptor_ref")?,
        "descriptor_version": optional_string_field(obj, "version")?,
        "visibility": optional_string_field(obj, "visibility")?,
        "class": optional_string_field(obj, "class")?,
        "description": optional_string_field(obj, "description")?,
        "source": optional_string_field(obj, "source")?,
        "schema_summary": obj.get("schema_summary").cloned().unwrap_or(Value::Null),
        "hints": obj.get("hints").cloned().unwrap_or(Value::Null),
        "metadata": {
            "profile": DIRECTORY_PROFILE,
            "source": ABILITY_META_LIST_ABILITIES,
            "raw_descriptor": row,
        },
    }))
}

fn directory_page_json(
    kind: &'static str,
    item_kind: &'static str,
    source_ability: &'static str,
    limit: usize,
    next_cursor: Option<String>,
    items: Vec<Value>,
    total_available: usize,
) -> Value {
    json!({
        "profile": DIRECTORY_PROFILE,
        "kind": kind,
        "item_kind": item_kind,
        "items": items,
        "next_cursor": next_cursor,
        "limit": limit,
        "source": DIRECTORY_SOURCE,
        "metadata": {
            "profile": DIRECTORY_PROFILE,
            "source": DIRECTORY_SOURCE,
            "source_ability": source_ability,
            "page_size_default": DIRECTORY_DEFAULT_PAGE_SIZE,
            "page_size_max": DIRECTORY_MAX_PAGE_SIZE,
            "total_available": total_available,
        },
    })
}

fn validate_owner_ura(raw: &str, field: &'static str) -> Result<(), DirectoryError> {
    let parsed =
        ura::parse_ura(raw).map_err(|err| DirectoryError::InvalidField(field, err.to_string()))?;
    match parsed.kind {
        URAKind::Agent | URAKind::Device | URAKind::Hub | URAKind::User => Ok(()),
        _ => Err(DirectoryError::InvalidField(
            field,
            "must be an owner URA".to_string(),
        )),
    }
}

fn validate_ability_ura(raw: &str, field: &'static str) -> Result<(), DirectoryError> {
    let parsed =
        ura::parse_ura(raw).map_err(|err| DirectoryError::InvalidField(field, err.to_string()))?;
    if parsed.kind == URAKind::Ability {
        return Ok(());
    }
    Err(DirectoryError::InvalidField(
        field,
        "must be an Ability URA".to_string(),
    ))
}

fn optional_u64(obj: &Map<String, Value>, field: &'static str) -> Option<u64> {
    match obj.get(field) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(raw)) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn optional_string_array(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Vec<String>>, DirectoryError> {
    let Some(value) = obj.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let values = value.as_array().ok_or_else(|| {
        DirectoryError::InvalidField(field, "must be an array of strings".to_string())
    })?;
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                DirectoryError::InvalidField(field, "must be an array of strings".to_string())
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request(extra: Value) -> Value {
        let mut request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "directory-1"}
        });
        let Value::Object(extra) = extra else {
            return request;
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        request
    }

    #[test]
    fn build_list_devices_invocation_targets_node_list() {
        let request = base_request(json!({"limit": 2}));

        let invocation = build_list_devices_invocation(&request).unwrap();

        assert_eq!(invocation["metadata"]["system_ability"], ABILITY_NODE_LIST);
        assert_eq!(invocation["args"], json!({}));
        assert_eq!(
            invocation["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.node.list@1.0.0"
        );
    }

    #[test]
    fn build_list_abilities_invocation_maps_sdk_owner_filter_to_daemon_arg() {
        let request = base_request(json!({
            "scope": "local",
            "owner_ura": "easynet:///r/example/device/dev-a",
            "ability_ura": "easynet:///r/example/ability/device.dev-a.fs.read"
        }));

        let invocation = build_list_abilities_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_META_LIST_ABILITIES
        );
        assert_eq!(invocation["args"]["scope"], "local");
        assert_eq!(
            invocation["args"]["agent_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(
            invocation["args"]["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.fs.read"
        );
    }

    #[test]
    fn list_requests_reject_limits_above_max_page_size() {
        let request = base_request(json!({"limit": DIRECTORY_MAX_PAGE_SIZE + 1}));

        let err = build_list_agents_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("limit"));
    }

    #[test]
    fn project_device_page_applies_cursor_pagination() {
        let input = json!({
            "result": {
                "nodes": [
                    {"node_id": "a", "state": "online"},
                    {"node_id": "b", "state": "online"},
                    {"node_id": "c", "state": "offline"}
                ]
            },
            "limit": 2,
            "cursor": "1"
        });

        let page = project_device_page(&input).unwrap();

        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["items"][0]["node_id"], "b");
        assert_eq!(page["next_cursor"], Value::Null);
        assert_eq!(page["metadata"]["page_size_max"], DIRECTORY_MAX_PAGE_SIZE);
    }

    #[test]
    fn project_agent_page_reuses_shared_agent_record_projection() {
        let input = json!({
            "result": {
                "agents": [{
                    "name": "codex",
                    "ura": "easynet:///r/example/agent/alice.codex",
                    "runtime": "codex",
                    "root_exists": true
                }]
            }
        });

        let page = project_agent_page(&input).unwrap();

        assert_eq!(
            page["items"][0]["owner_ura"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(page["kind"], "agent_page");
    }

    #[test]
    fn project_ability_page_preserves_raw_descriptor() {
        let input = json!({
            "abilities": [{
                "name": "fs.read",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.fs.read",
                "owner_ura": "easynet:///r/example/device/dev-a",
                "version": "1.0.0",
                "visibility": "SCOPED",
                "class": "query",
                "description": "Read a file.",
                "schema_summary": {"input": {"type": "object"}},
                "hints": {"read_only": true}
            }]
        });

        let page = project_ability_page(&input).unwrap();

        assert_eq!(page["items"][0]["name"], "fs.read");
        assert_eq!(
            page["items"][0]["metadata"]["raw_descriptor"]["ability_ura"],
            "easynet:///r/example/ability/device.dev-a.fs.read"
        );
    }

    #[test]
    fn project_ability_page_rejects_non_ability_ura() {
        let input = json!({
            "abilities": [{
                "name": "fs.read",
                "ability_ura": "easynet:///r/example/device/dev-a"
            }]
        });

        let err = project_ability_page(&input).unwrap_err();

        assert!(err.to_string().contains("Ability URA"));
    }
}

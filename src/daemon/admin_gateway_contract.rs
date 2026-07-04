// EasyNet CLI — Admin + Gateway shared contract
// ==============================================
//
// File: src/daemon/admin_gateway_contract.rs
// Description: Shared daemon SDK contract for Admin + Gateway Invocation
//              carriers, gateway readiness projection, and agent records.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Admin + Gateway DTO projection for daemon lifecycle
// and admin ability boundaries. Backend account state, pairing-token HTTP,
// certificate policy, and browser session UX are outside this module.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for daemon-owned system
// abilities. Project existing daemon lifecycle/status and agent.list results
// into binding-facing DTOs that preserve degraded states instead of collapsing
// them into generic booleans.
//
// Usage Contract
// --------------
// Carrier construction requires explicit Invocation tuple fields. Projection
// accepts daemon status/result JSON facts only; it never fabricates agent URAs,
// owner refs, trust state, or public listener readiness.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Admin + Gateway profile. Runtime Core remains the
// submit/open path for returned Invocation carriers.

use std::path::{Component, Path};
use std::str::FromStr;

use serde_json::{json, Map, Value};

use crate::core::ura;
use crate::daemon::agent_record_contract;
use crate::daemon::persistence::agent_registry::AgentType;
use crate::daemon::sdk_contract::{
    build_system_invocation, first_optional_string_field, object, optional_bool_field,
    optional_string_field, required_string, validate_ura, SdkContractError,
};

const ADMIN_PROFILE: &str = "admin_gateway";
const ABILITY_AGENT_LIST: &str = crate::daemon::ability::names::agents::AGENT_LIST;
const ABILITY_AGENT_START: &str = crate::daemon::ability::names::agents::AGENT_START;
const ABILITY_AGENT_STOP: &str = crate::daemon::ability::names::agents::AGENT_STOP;
const ABILITY_AGENT_REFRESH: &str = crate::daemon::ability::names::agents::AGENT_REFRESH;
const ABILITY_SESSION_LIST: &str = crate::daemon::ability::names::device_control::SESSION_LIST;

pub(crate) type AdminGatewayError = SdkContractError;

pub(crate) fn build_agent_list_invocation(request: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(request, "AdminAgentListRequest")?;
    build_system_invocation(obj, ADMIN_PROFILE, ABILITY_AGENT_LIST, json!({}))
}

pub(crate) fn build_agent_start_invocation(request: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(request, "AdminAgentStartRequest")?;
    let args = agent_start_args(obj)?;
    build_system_invocation(obj, ADMIN_PROFILE, ABILITY_AGENT_START, args)
}

pub(crate) fn build_agent_stop_invocation(request: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(request, "AdminAgentStopRequest")?;
    let args = agent_stop_args(obj)?;
    build_system_invocation(obj, ADMIN_PROFILE, ABILITY_AGENT_STOP, args)
}

pub(crate) fn build_agent_refresh_invocation(request: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(request, "AdminAgentRefreshRequest")?;
    let mut args = Map::new();
    if let Some(name) = optional_string_field(obj, "name")? {
        validate_agent_name(&name, "name")?;
        args.insert("name".to_string(), Value::String(name));
    }
    build_system_invocation(
        obj,
        ADMIN_PROFILE,
        ABILITY_AGENT_REFRESH,
        Value::Object(args),
    )
}

pub(crate) fn build_session_list_invocation(request: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(request, "AdminSessionListRequest")?;
    let mut args = Map::new();
    if let Some(include_terminated) = optional_bool_field(obj, "include_terminated")? {
        args.insert(
            "include_terminated".to_string(),
            Value::Bool(include_terminated),
        );
    }
    build_system_invocation(
        obj,
        ADMIN_PROFILE,
        ABILITY_SESSION_LIST,
        Value::Object(args),
    )
}

pub(crate) fn project_gateway_status(input: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(input, "GatewayStatusInput")?;
    let lifecycle_state = first_optional_string_field(obj, "runtime_status", "state")?
        .unwrap_or_else(|| "unknown".to_string());
    let daemon = obj.get("daemon").and_then(Value::as_object);
    let runtime = obj.get("runtime").and_then(Value::as_object);
    let presence = obj.get("product_presence").and_then(Value::as_object);
    let identity = daemon
        .and_then(|daemon| daemon.get("identity"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let mode = identity
        .as_object()
        .and_then(|identity| identity.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("device");

    let process_live = daemon
        .map(|daemon| {
            let matching_pid_alive = optional_bool_value(daemon, "pid_alive").unwrap_or(false)
                && optional_bool_value(daemon, "pid_matches_easynet").unwrap_or(true);
            let control_accepting =
                optional_bool_value(daemon, "control_accepting").unwrap_or(false);
            let invocation_accepting =
                optional_bool_value(daemon, "invocation_accepting").unwrap_or(false);
            matching_pid_alive || control_accepting || invocation_accepting
        })
        .unwrap_or(false);
    let control_ready = daemon
        .and_then(|daemon| optional_bool_value(daemon, "control_accepting"))
        .or_else(|| optional_bool_field(obj, "control_ready").ok().flatten())
        .unwrap_or(false);
    let runtime_ready = daemon
        .and_then(|daemon| optional_bool_value(daemon, "invocation_accepting"))
        .or_else(|| optional_bool_field(obj, "runtime_ready").ok().flatten())
        .unwrap_or(false);
    let directory_ready = presence
        .and_then(|presence| optional_bool_value(presence, "session_admitted"))
        .or_else(|| optional_bool_field(obj, "directory_ready").ok().flatten())
        .unwrap_or(false);
    let trust_ready = runtime
        .and_then(|runtime| optional_bool_value(runtime, "credential_verified"))
        .or_else(|| {
            presence
                .and_then(|presence| presence.get("device_ura"))
                .and_then(Value::as_str)
                .map(|value| !value.trim().is_empty())
        })
        .or_else(|| optional_bool_field(obj, "trust_ready").ok().flatten())
        .unwrap_or(false);

    let mut listeners = listeners_from_status(obj, daemon)?;
    let public_listener_ready = listeners
        .iter()
        .any(|listener| listener.public && listener.ready);
    let requires_public_listener = optional_bool_field(obj, "require_public_listener")?
        .unwrap_or_else(|| matches!(mode, "hub" | "both"));
    let ready = process_live
        && control_ready
        && runtime_ready
        && directory_ready
        && trust_ready
        && (!requires_public_listener || public_listener_ready);
    listeners.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.endpoint.cmp(&b.endpoint)));

    let gateway_id =
        optional_string_field(obj, "gateway_id")?.unwrap_or_else(|| gateway_id(&identity));
    let mut metadata = Map::new();
    metadata.insert(
        "profile".to_string(),
        Value::String(ADMIN_PROFILE.to_string()),
    );
    metadata.insert(
        "source".to_string(),
        Value::String("daemon_lifecycle_status".to_string()),
    );
    metadata.insert(
        "lifecycle_state".to_string(),
        Value::String(lifecycle_state.clone()),
    );
    metadata.insert(
        "requires_public_listener".to_string(),
        Value::Bool(requires_public_listener),
    );
    copy_optional_value(obj, &mut metadata, "connection");

    Ok(json!({
        "profile": ADMIN_PROFILE,
        "gateway_id": gateway_id,
        "ready": ready,
        "state": if ready { "ready" } else { lifecycle_state.as_str() },
        "process_live": process_live,
        "control_ready": control_ready,
        "runtime_ready": runtime_ready,
        "directory_ready": directory_ready,
        "trust_ready": trust_ready,
        "public_listener_ready": public_listener_ready,
        "listeners": listeners.into_iter().map(GatewayListener::to_json).collect::<Vec<_>>(),
        "identity": identity,
        "metadata": metadata,
    }))
}

pub(crate) fn project_agent_records(input: &Value) -> Result<Value, AdminGatewayError> {
    let records =
        agent_record_contract::project_agent_record_items_for_profile(input, ADMIN_PROFILE)?;
    Ok(json!({
        "profile": ADMIN_PROFILE,
        "kind": "agent_records",
        "state": "ok",
        "items": records,
        "next_cursor": Value::Null,
        "metadata": {
            "profile": ADMIN_PROFILE,
            "source": "agent.list",
            "count": records.len(),
        },
    }))
}

pub(crate) fn project_device_session_page(input: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(input, "DeviceSessionPageInput")?;
    let result = obj
        .get("result")
        .filter(|value| !value.is_null())
        .unwrap_or(input);
    let result_obj = object(result, "DeviceSessionPage")?;
    let raw_items = result_obj
        .get("sessions")
        .or_else(|| result_obj.get("items"))
        .and_then(Value::as_array)
        .ok_or(AdminGatewayError::MissingField("sessions"))?;
    let items = raw_items
        .iter()
        .map(project_device_session)
        .collect::<Result<Vec<_>, _>>()?;
    let state = optional_string_field(result_obj, "state")?.unwrap_or_else(|| "ok".to_string());
    let next_cursor = result_obj
        .get("next_cursor")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let count = items.len();

    Ok(json!({
        "profile": ADMIN_PROFILE,
        "kind": "device_sessions",
        "state": state,
        "items": items,
        "next_cursor": next_cursor,
        "metadata": {
            "profile": ADMIN_PROFILE,
            "source": "session.list",
            "count": count,
            "raw_result": result,
        },
    }))
}

pub(crate) fn project_agent_lifecycle_result(input: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(input, "AgentLifecycleResultInput")?;
    let result = obj
        .get("result")
        .filter(|value| !value.is_null())
        .unwrap_or(input);
    let result_obj = object(result, "AgentLifecycleResult")?;
    let operation = optional_string_field(obj, "operation")?
        .or_else(|| infer_lifecycle_operation(result_obj).map(str::to_string))
        .ok_or(AdminGatewayError::MissingField("operation"))?;
    let agent_ura = optional_string_field(result_obj, "agent_ura")?;
    if let Some(agent_ura) = agent_ura.as_deref() {
        validate_ura(agent_ura, "agent_ura")?;
    }
    let state = lifecycle_state(result_obj);

    Ok(json!({
        "profile": ADMIN_PROFILE,
        "kind": "agent_lifecycle_result",
        "operation": operation,
        "state": state,
        "agent_ura": agent_ura,
        "ack": result_obj.get("ack").and_then(Value::as_bool),
        "runtime_not_ready": optional_bool_value(result_obj, "runtime_not_ready").unwrap_or(false),
        "runtime_catalog_not_ready": optional_bool_value(result_obj, "runtime_catalog_not_ready").unwrap_or(false),
        "metadata": {
            "profile": ADMIN_PROFILE,
            "source": "agent_lifecycle",
            "runtime_registered": optional_u64(result_obj, "runtime_registered").unwrap_or(0),
            "runtime_failed": optional_u64(result_obj, "runtime_failed").unwrap_or(0),
            "runtime_removed": optional_u64(result_obj, "runtime_removed").unwrap_or(0),
            "raw_result": result,
        },
    }))
}

fn project_device_session(input: &Value) -> Result<Value, AdminGatewayError> {
    let obj = object(input, "DeviceSession")?;
    let session_id = first_optional_string_field(obj, "session_id", "id")?
        .ok_or(AdminGatewayError::MissingField("session_id"))?;
    let tenant = first_optional_string_field(obj, "tenant", "realm")?;
    let node = first_optional_string_field(obj, "node", "node_id")?;
    let device_ura = match optional_string_field(obj, "device_ura")? {
        Some(device_ura) => {
            validate_ura(&device_ura, "device_ura")?;
            device_ura
        }
        None => match (tenant.as_deref(), node.as_deref()) {
            (Some(tenant), Some(node)) => ura::device_ura(tenant, node),
            _ => return Err(AdminGatewayError::MissingField("device_ura")),
        },
    };
    let hub_ura = match optional_string_field(obj, "hub_ura")? {
        Some(hub_ura) => {
            validate_ura(&hub_ura, "hub_ura")?;
            hub_ura
        }
        None => tenant
            .as_deref()
            .map(ura::hub_ura)
            .ok_or(AdminGatewayError::MissingField("hub_ura"))?,
    };
    let state = first_optional_string_field(obj, "state", "status")?.unwrap_or_else(|| {
        if obj
            .get("ended_unix_ms")
            .filter(|value| !value.is_null())
            .is_some()
        {
            "terminated".to_string()
        } else {
            "active".to_string()
        }
    });
    let session_kind = first_optional_string_field(obj, "session_kind", "kind")?
        .unwrap_or_else(|| "daemon_session".to_string());
    let created_unix_ms = optional_u64(obj, "created_unix_ms")
        .or_else(|| optional_u64(obj, "started_unix_ms"))
        .ok_or(AdminGatewayError::MissingField("created_unix_ms"))?;
    let expires_unix_ms = optional_u64(obj, "expires_unix_ms")
        .or_else(|| optional_u64(obj, "ended_unix_ms"))
        .unwrap_or(0);
    let mut metadata = obj
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert(
        "profile".to_string(),
        Value::String(ADMIN_PROFILE.to_string()),
    );
    metadata.insert(
        "source".to_string(),
        Value::String("session.list".to_string()),
    );
    copy_optional_value(obj, &mut metadata, "agent");
    copy_optional_value(obj, &mut metadata, "tenant");
    copy_optional_value(obj, &mut metadata, "node");
    copy_optional_value(obj, &mut metadata, "ended_unix_ms");

    Ok(json!({
        "profile": ADMIN_PROFILE,
        "kind": "device_session",
        "session_id": session_id,
        "device_ura": device_ura,
        "hub_ura": hub_ura,
        "state": state,
        "session_kind": session_kind,
        "created_unix_ms": created_unix_ms,
        "expires_unix_ms": expires_unix_ms,
        "metadata": metadata,
    }))
}

fn agent_start_args(obj: &Map<String, Value>) -> Result<Value, AdminGatewayError> {
    let name = required_string(obj, "name")?;
    validate_agent_name(name, "name")?;
    let agent_type = optional_string_field(obj, "agent_type")?;
    let has_agent_type = agent_type.is_some();
    let entry = obj.get("entry").filter(|value| !value.is_null());
    if !has_agent_type && entry.is_none() {
        return Err(AdminGatewayError::InvalidField(
            "agent_type",
            "either agent_type or entry is required".to_string(),
        ));
    }
    let mut args = Map::new();
    args.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(agent_type) = agent_type {
        validate_agent_type(&agent_type, "agent_type")?;
        args.insert("agent_type".to_string(), Value::String(agent_type));
    }
    if let Some(entry) = entry {
        let entry_obj = object(entry, "entry")?;
        if let Some(entry_agent_type) = optional_string_field(entry_obj, "agent_type")? {
            validate_agent_type(&entry_agent_type, "entry.agent_type")?;
            if let Some(agent_type) = args.get("agent_type").and_then(Value::as_str) {
                if agent_type != entry_agent_type {
                    return Err(AdminGatewayError::InvalidField(
                        "entry.agent_type",
                        "must match top-level agent_type".to_string(),
                    ));
                }
            }
        }
        args.insert("entry".to_string(), entry.clone());
    }
    for field in ["model", "label", "command"] {
        if let Some(value) = optional_string_field(obj, field)? {
            args.insert(field.to_string(), Value::String(value));
        }
    }
    if let Some(root_path) = optional_string_field(obj, "root_path")? {
        validate_absolute_path(&root_path, "root_path")?;
        args.insert("root_path".to_string(), Value::String(root_path));
    }
    if let Some(command_args) = obj.get("command_args").filter(|value| !value.is_null()) {
        let values = command_args
            .as_array()
            .ok_or_else(|| {
                AdminGatewayError::InvalidField(
                    "command_args",
                    "must be an array of strings".to_string(),
                )
            })?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|value| Value::String(value.to_string()))
                    .ok_or_else(|| {
                        AdminGatewayError::InvalidField(
                            "command_args",
                            "must be an array of strings".to_string(),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        args.insert("command_args".to_string(), Value::Array(values));
    }
    for field in [
        "model_present",
        "materialize_directory",
        "update_existing_spec",
        "project_workspace",
    ] {
        if let Some(value) = optional_bool_field(obj, field)? {
            args.insert(field.to_string(), Value::Bool(value));
        }
    }
    Ok(Value::Object(args))
}

fn agent_stop_args(obj: &Map<String, Value>) -> Result<Value, AdminGatewayError> {
    let name = optional_string_field(obj, "name")?;
    if let Some(name) = name.as_deref() {
        validate_agent_name(name, "name")?;
    }
    let agent_ura = optional_string_field(obj, "agent_ura")?;
    if let Some(agent_ura) = agent_ura.as_deref() {
        validate_agent_ura(agent_ura)?;
    }
    match (name.as_deref(), agent_ura.as_deref()) {
        (None, None) => {
            return Err(AdminGatewayError::InvalidField(
                "name",
                "either name or agent_ura is required".to_string(),
            ));
        }
        (Some(name), Some(agent_ura)) => {
            let from_ura = agent_name_from_ura(agent_ura)?;
            if from_ura != name {
                return Err(AdminGatewayError::InvalidField(
                    "agent_ura",
                    "agent_ura must name the same hosted agent as name".to_string(),
                ));
            }
        }
        _ => {}
    }

    let mut args = Map::new();
    if let Some(name) = name {
        args.insert("name".to_string(), Value::String(name));
    }
    if let Some(agent_ura) = agent_ura {
        args.insert("agent_ura".to_string(), Value::String(agent_ura));
    }
    Ok(Value::Object(args))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayListener {
    kind: String,
    endpoint: String,
    ready: bool,
    public: bool,
}

impl GatewayListener {
    fn to_json(self) -> Value {
        json!({
            "kind": self.kind,
            "endpoint": self.endpoint,
            "ready": self.ready,
            "public": self.public,
        })
    }
}

fn listeners_from_status(
    obj: &Map<String, Value>,
    daemon: Option<&Map<String, Value>>,
) -> Result<Vec<GatewayListener>, AdminGatewayError> {
    let mut listeners = Vec::new();
    if let Some(daemon) = daemon {
        if let Some(endpoint) = optional_string_field(daemon, "control_socket")? {
            listeners.push(GatewayListener {
                kind: "control".to_string(),
                endpoint,
                ready: optional_bool_value(daemon, "control_accepting").unwrap_or(false),
                public: false,
            });
        }
        if let Some(endpoint) = optional_string_field(daemon, "invocation_endpoint")? {
            listeners.push(GatewayListener {
                kind: "invocation".to_string(),
                endpoint,
                ready: optional_bool_value(daemon, "invocation_accepting").unwrap_or(false),
                public: false,
            });
        }
    }
    if let Some(public_listeners) = obj.get("public_listeners").filter(|value| !value.is_null()) {
        let array = public_listeners.as_array().ok_or_else(|| {
            AdminGatewayError::InvalidField("public_listeners", "must be an array".to_string())
        })?;
        for listener in array {
            let listener = object(listener, "public_listener")?;
            listeners.push(GatewayListener {
                kind: optional_string_field(listener, "kind")?
                    .unwrap_or_else(|| "public".to_string()),
                endpoint: required_string(listener, "endpoint")?.to_string(),
                ready: optional_bool_field(listener, "ready")?.unwrap_or(false),
                public: true,
            });
        }
    }
    Ok(listeners)
}

fn lifecycle_state(obj: &Map<String, Value>) -> &'static str {
    if obj
        .get("ack")
        .and_then(Value::as_bool)
        .is_some_and(|ack| !ack)
    {
        return "not_found";
    }
    if optional_bool_value(obj, "runtime_not_ready").unwrap_or(false)
        || optional_bool_value(obj, "runtime_catalog_not_ready").unwrap_or(false)
    {
        return "not_ready";
    }
    if optional_u64(obj, "runtime_failed").unwrap_or(0) > 0
        || obj
            .get("hub_advertise_error")
            .is_some_and(|value| !value.is_null())
        || obj
            .get("workspace_projection_error")
            .is_some_and(|value| !value.is_null())
        || obj.get("ok").and_then(Value::as_bool).is_some_and(|ok| !ok)
    {
        return "partial";
    }
    "ok"
}

fn infer_lifecycle_operation(obj: &Map<String, Value>) -> Option<&'static str> {
    if obj.contains_key("replaced_prior") || obj.contains_key("agent_ura") {
        Some("agent.start")
    } else if obj.contains_key("ack") {
        Some("agent.stop")
    } else if obj.contains_key("agents_scanned") {
        Some("agent.refresh")
    } else {
        None
    }
}

fn gateway_id(identity: &Value) -> String {
    let Some(identity) = identity.as_object() else {
        return "local-daemon".to_string();
    };
    let mode = identity
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("device");
    let realm = identity
        .get("realm")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let node_id = identity
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("local");
    format!("{mode}:{realm}:{node_id}")
}

fn validate_agent_type(raw: &str, field: &'static str) -> Result<(), AdminGatewayError> {
    AgentType::from_str(raw)
        .map(|_| ())
        .map_err(|err| AdminGatewayError::InvalidField(field, err.to_string()))
}

fn validate_agent_name(raw: &str, field: &'static str) -> Result<(), AdminGatewayError> {
    if raw.trim().is_empty() {
        return Err(AdminGatewayError::InvalidField(
            field,
            "must not be empty".to_string(),
        ));
    }
    if raw == "device" || raw.starts_with("device.") {
        return Err(AdminGatewayError::InvalidField(
            field,
            "`device` is reserved for device-sponsored System Agents".to_string(),
        ));
    }
    if raw.contains('/') || raw.contains('\\') || raw.chars().any(char::is_whitespace) {
        return Err(AdminGatewayError::InvalidField(
            field,
            "must be an owner-local agent id, not a path or whitespace token".to_string(),
        ));
    }
    Ok(())
}

fn validate_agent_ura(raw: &str) -> Result<(), AdminGatewayError> {
    let parsed = ura::parse_ura(raw)
        .map_err(|err| AdminGatewayError::InvalidField("agent_ura", err.to_string()))?;
    if parsed.kind != ura::URAKind::Agent {
        return Err(AdminGatewayError::InvalidField(
            "agent_ura",
            "must be an Agent URA".to_string(),
        ));
    }
    if parsed.device_agent_ids().is_some() {
        return Err(AdminGatewayError::InvalidField(
            "agent_ura",
            "device-sponsored System Agents are not managed by hosted agent lifecycle".to_string(),
        ));
    }
    Ok(())
}

fn agent_name_from_ura(raw: &str) -> Result<String, AdminGatewayError> {
    validate_agent_ura(raw)?;
    let parsed = ura::parse_ura(raw)
        .map_err(|err| AdminGatewayError::InvalidField("agent_ura", err.to_string()))?;
    parsed
        .agent_ids()
        .map(|(_, agent_id)| agent_id.to_string())
        .ok_or_else(|| AdminGatewayError::InvalidField("agent_ura", "missing agent id".to_string()))
}

fn validate_absolute_path(raw: &str, field: &'static str) -> Result<(), AdminGatewayError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(AdminGatewayError::InvalidField(
            field,
            "must be absolute".to_string(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AdminGatewayError::InvalidField(
            field,
            "must not contain `..` components".to_string(),
        ));
    }
    Ok(())
}

fn optional_bool_value(obj: &Map<String, Value>, field: &'static str) -> Option<bool> {
    obj.get(field).and_then(Value::as_bool)
}

fn optional_u64(obj: &Map<String, Value>, field: &'static str) -> Option<u64> {
    match obj.get(field) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(raw)) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn copy_optional_value(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key).filter(|value| !value.is_null()) {
        target.insert(key.to_string(), value.clone());
    }
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
            "metadata": {"request_id": "admin-1"}
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
    fn build_agent_start_invocation_projects_complete_tuple() {
        let request = base_request(json!({
            "name": "codex",
            "agent_type": "codex",
            "model": "gpt-5",
            "label": "primary"
        }));

        let carrier = build_agent_start_invocation(&request).unwrap();

        assert_eq!(carrier["metadata"]["profile"], ADMIN_PROFILE);
        assert_eq!(carrier["metadata"]["system_ability"], "agent.start");
        assert_eq!(carrier["args"]["name"], "codex");
        assert_eq!(carrier["args"]["agent_type"], "codex");
        assert_eq!(
            carrier["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0"
        );
    }

    #[test]
    fn build_agent_stop_invocation_rejects_system_agent_ura() {
        let request = base_request(json!({
            "agent_ura": "easynet:///r/example/agent/device.dev-a.local"
        }));

        let err = build_agent_stop_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("device-sponsored System Agents"));
    }

    #[test]
    fn build_session_list_invocation_preserves_filter() {
        let request = base_request(json!({
            "include_terminated": false
        }));

        let carrier = build_session_list_invocation(&request).unwrap();

        assert_eq!(carrier["metadata"]["system_ability"], "session.list");
        assert_eq!(carrier["args"]["include_terminated"], false);
    }

    #[test]
    fn build_agent_refresh_rejects_non_string_name() {
        let request = base_request(json!({
            "name": 42
        }));

        let err = build_agent_refresh_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("must be a string"));
    }

    #[test]
    fn project_gateway_status_preserves_degraded_control_only_state() {
        let status = json!({
            "runtime_status": "control_only_invocation_down",
            "daemon": {
                "pid": 42,
                "pid_alive": true,
                "pid_matches_easynet": true,
                "control_accepting": true,
                "invocation_accepting": false,
                "control_socket": "/tmp/easynet-control.sock",
                "invocation_endpoint": "/tmp/easynet-daemon.sock",
                "identity": {"mode": "device", "realm": "example", "node_id": "dev-a"}
            },
            "runtime": {"credential_verified": true},
            "product_presence": {
                "device_ura": "easynet:///r/example/device/dev-a",
                "session_admitted": false,
                "directory_status": "suspect"
            }
        });

        let projected = project_gateway_status(&status).unwrap();

        assert_eq!(projected["ready"], false);
        assert_eq!(projected["control_ready"], true);
        assert_eq!(projected["runtime_ready"], false);
        assert_eq!(projected["directory_ready"], false);
        assert_eq!(projected["state"], "control_only_invocation_down");
    }

    #[test]
    fn project_gateway_status_allows_device_ready_without_public_listener() {
        let status = json!({
            "runtime_status": "running",
            "daemon": {
                "pid_alive": true,
                "pid_matches_easynet": true,
                "control_accepting": true,
                "invocation_accepting": true,
                "control_socket": "/tmp/easynet-control.sock",
                "invocation_endpoint": "/tmp/easynet-daemon.sock",
                "identity": {"mode": "device", "realm": "example", "node_id": "dev-a"}
            },
            "runtime": {"credential_verified": true},
            "product_presence": {
                "device_ura": "easynet:///r/example/device/dev-a",
                "session_admitted": true,
                "directory_status": "online"
            }
        });

        let projected = project_gateway_status(&status).unwrap();

        assert_eq!(projected["ready"], true);
        assert_eq!(projected["state"], "ready");
        assert_eq!(projected["public_listener_ready"], false);
    }

    #[test]
    fn project_device_session_page_maps_daemon_session_list() {
        let projected = project_device_session_page(&json!({
            "sessions": [{
                "id": "session-1",
                "tenant": "example",
                "node": "dev-a",
                "agent": "assistant",
                "started_unix_ms": 1767225600000i64
            }]
        }))
        .unwrap();

        assert_eq!(projected["profile"], ADMIN_PROFILE);
        assert_eq!(projected["kind"], "device_sessions");
        assert_eq!(projected["items"][0]["session_id"], "session-1");
        assert_eq!(
            projected["items"][0]["device_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(projected["items"][0]["hub_ura"], "easynet:///r/example/hub");
        assert_eq!(projected["items"][0]["session_kind"], "daemon_session");
        assert_eq!(projected["items"][0]["state"], "active");
        assert_eq!(projected["metadata"]["count"], 1);
    }

    #[test]
    fn project_device_session_page_preserves_explicit_session_facts() {
        let projected = project_device_session_page(&json!({
            "result": {
                "state": "ok",
                "items": [{
                    "session_id": "dev-session-1",
                    "device_ura": "easynet:///r/example/device/dev-a",
                    "hub_ura": "easynet:///r/example/hub",
                    "state": "active",
                    "session_kind": "remote_desktop",
                    "created_unix_ms": 1767225600000i64,
                    "expires_unix_ms": 1893456000000i64,
                    "metadata": {"source": "daemon"}
                }]
            }
        }))
        .unwrap();

        assert_eq!(projected["items"][0]["session_kind"], "remote_desktop");
        assert_eq!(projected["items"][0]["metadata"]["source"], "session.list");
        assert_eq!(projected["items"][0]["metadata"]["profile"], ADMIN_PROFILE);
        assert_eq!(projected["items"][0]["expires_unix_ms"], 1893456000000i64);
    }

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

        let page = project_agent_records(&input).unwrap();

        assert_eq!(page["items"][0]["agent_ura"], Value::Null);
        assert_eq!(page["items"][0]["owner_ura"], Value::Null);
        assert_eq!(page["items"][0]["state"], "registered");
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

        let page = project_agent_records(&input).unwrap();

        assert_eq!(
            page["items"][0]["owner_ura"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(page["items"][0]["state"], "degraded");
        assert_eq!(page["items"][0]["abilities"][0], "chat.complete");
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

        let err = project_agent_records(&input).unwrap_err();

        assert!(err.to_string().contains("array of strings"));
    }

    #[test]
    fn project_agent_lifecycle_classifies_runtime_not_ready() {
        let result = json!({
            "agent_ura": "easynet:///r/example/agent/alice.codex",
            "replaced_prior": false,
            "runtime_not_ready": true,
            "runtime_failed": 0
        });

        let projected = project_agent_lifecycle_result(&result).unwrap();

        assert_eq!(projected["operation"], "agent.start");
        assert_eq!(projected["state"], "not_ready");
        assert_eq!(
            projected["agent_ura"],
            "easynet:///r/example/agent/alice.codex"
        );
    }
}

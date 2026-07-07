// EasyNet CLI — Desktop companion shared contract
// =================================================
//
// File: src/protocol/companion_contract.rs
// Description: Shared SDK/control-plane DTO projection for desktop companion
//              plugin status and lifecycle action results.
//
// Protocol Responsibility
// -----------------------
// Own the language-neutral DesktopCompanionStatus and
// DesktopCompanionActionResult JSON shapes consumed by CLI, local daemon
// control surfaces, FFI, and language SDK facades.
//
// Implementation Approach
// -----------------------
// Accept companion manager facts, validate bounded enum/string fields, and
// project a stable schema-shaped DTO. OS adapters detect and supervise platform
// state; this module only canonicalizes the wire/read-model shape.
//
// Usage Contract
// --------------
// Callers pass daemon-local companion facts only. This contract is not an Axon
// Invocation primitive and must not be published as a remote ability surface.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK/control-plane projection layer. Axon remains the
// authority for Invocation and receipt semantics; desktop companion lifecycle is
// local EasyNet-Cli product/runtime state.

use serde_json::{json, Map, Value};

use crate::protocol::sdk_contract::{
    object, optional_string_field, required_string, SdkContractError,
};

pub(crate) type CompanionContractError = SdkContractError;

const PROFILE: &str = "desktop_companion";
const STATUS_KIND: &str = "desktop_companion_status";
const ACTION_KIND: &str = "desktop_companion_action_result";

const PLATFORMS: &[&str] = &["macos", "windows", "linux", "unknown"];
const DESIRED_STATES: &[&str] = &["enabled", "disabled"];
const SUPERVISOR_STATES: &[&str] = &[
    "unsupported_platform",
    "unsupported_session",
    "not_installed",
    "installed_disabled",
    "installed_enabled",
    "install_error",
    "enable_error",
    "disable_error",
];
const OBSERVED_STATES: &[&str] = &[
    "unknown",
    "not_running",
    "starting",
    "running",
    "stale",
    "exited",
    "version_mismatch",
    "health_error",
];
const PROJECTED_STATES: &[&str] = &[
    "disabled",
    "unsupported_platform",
    "unsupported_session",
    "not_installed",
    "installed_disabled",
    "ready_stopped",
    "starting",
    "running",
    "stale",
    "error",
];
const BOOT_POLICIES: &[&str] = &["manual", "ensure_running_after_daemon_ready"];
const STOP_POLICIES: &[&str] = &[
    "keep_running",
    "stop_on_runtime_stop",
    "stop_on_plugin_disable",
];
const HEALTH_MODES: &[&str] = &["process_name", "status_file", "local_ipc"];
const ACTIONS: &[&str] = &[
    "status",
    "install",
    "remove",
    "enable",
    "disable",
    "start",
    "stop",
    "restart",
    "reconcile",
];

pub(crate) fn project_status(input: &Value) -> Result<Value, CompanionContractError> {
    let payload = projection_payload(input);
    let obj = object(payload, "DesktopCompanionStatus")?;
    let package_id = required_string(obj, "package_id")?;
    let package_version =
        required_string(obj, "package_version").or_else(|_| required_string(obj, "version"))?;
    let display_name = required_string(obj, "display_name")?;
    let platform = required_enum(obj, "platform", PLATFORMS)?;
    let desired_state = required_enum(obj, "desired_state", DESIRED_STATES)?;
    let supervisor_state = required_enum(obj, "supervisor_state", SUPERVISOR_STATES)?;
    let observed_state = required_enum(obj, "observed_state", OBSERVED_STATES)?;
    let projected_state = required_enum(obj, "projected_state", PROJECTED_STATES)?;
    let boot_policy = required_enum(obj, "boot_policy", BOOT_POLICIES)?;
    let stop_policy = required_enum(obj, "stop_policy", STOP_POLICIES)?;
    let health = required_enum(obj, "health", HEALTH_MODES)?;

    Ok(json!({
        "profile": PROFILE,
        "kind": STATUS_KIND,
        "package_id": package_id,
        "package_version": package_version,
        "display_name": display_name,
        "platform": platform,
        "desired_state": desired_state,
        "supervisor_state": supervisor_state,
        "observed_state": observed_state,
        "projected_state": projected_state,
        "boot_policy": boot_policy,
        "stop_policy": stop_policy,
        "health": health,
        "pid": optional_u64_field(obj, "pid")?.map(Value::from).unwrap_or(Value::Null),
        "version": optional_string_field(obj, "version")?,
        "last_seen_unix_ms": optional_u64_field(obj, "last_seen_unix_ms")?.map(Value::from).unwrap_or(Value::Null),
        "launch_method": optional_string_field(obj, "launch_method")?,
        "error": optional_object_or_null(obj, "error")?,
        "metadata": optional_object_or_empty(obj, "metadata")?,
    }))
}

pub(crate) fn project_action_result(input: &Value) -> Result<Value, CompanionContractError> {
    let payload = projection_payload(input);
    let obj = object(payload, "DesktopCompanionActionResult")?;
    let package_id = required_string(obj, "package_id")?;
    let action = required_enum(obj, "action", ACTIONS)?;
    let changed = required_bool(obj, "changed")?;
    let status_before = optional_status_projection(obj, "status_before")?;
    let status_after = optional_status_projection(obj, "status_after")?;

    Ok(json!({
        "profile": PROFILE,
        "kind": ACTION_KIND,
        "package_id": package_id,
        "action": action,
        "status_before": status_before,
        "status_after": status_after,
        "changed": changed,
        "error": optional_object_or_null(obj, "error")?,
        "metadata": optional_object_or_empty(obj, "metadata")?,
    }))
}

fn projection_payload(input: &Value) -> &Value {
    input
        .as_object()
        .and_then(|obj| obj.get("output_json").filter(|value| !value.is_null()))
        .unwrap_or(input)
}

fn required_enum(
    obj: &Map<String, Value>,
    field: &'static str,
    allowed: &[&str],
) -> Result<String, CompanionContractError> {
    let raw = required_string(obj, field)?;
    if allowed.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(CompanionContractError::InvalidField(
            field,
            format!("must be one of {}", allowed.join(", ")),
        ))
    }
}

fn required_bool(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<bool, CompanionContractError> {
    obj.get(field)
        .and_then(Value::as_bool)
        .ok_or(CompanionContractError::MissingField(field))
}

fn optional_u64_field(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, CompanionContractError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| {
                CompanionContractError::InvalidField(
                    field,
                    "must be an unsigned integer".to_string(),
                )
            })
            .map(Some),
        Some(_) => Err(CompanionContractError::InvalidField(
            field,
            "must be an unsigned integer".to_string(),
        )),
    }
}

fn optional_object_or_null(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Value, CompanionContractError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err(CompanionContractError::InvalidField(
            field,
            "must be an object or null".to_string(),
        )),
    }
}

fn optional_object_or_empty(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Value, CompanionContractError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(json!({})),
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err(CompanionContractError::InvalidField(
            field,
            "must be an object or null".to_string(),
        )),
    }
}

fn optional_status_projection(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Value, CompanionContractError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(value @ Value::Object(_)) => project_status(value),
        Some(_) => Err(CompanionContractError::InvalidField(
            field,
            "must be a DesktopCompanionStatus object or null".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_input() -> Value {
        json!({
            "package_id": "easynet.desktop.menubar",
            "package_version": "0.1.0",
            "display_name": "EasyNet Menu Bar",
            "platform": "macos",
            "desired_state": "enabled",
            "supervisor_state": "installed_enabled",
            "observed_state": "running",
            "projected_state": "running",
            "boot_policy": "ensure_running_after_daemon_ready",
            "stop_policy": "keep_running",
            "health": "status_file",
            "pid": 12345,
            "version": "0.1.0",
            "last_seen_unix_ms": 1783411200000_u64,
            "launch_method": "launch_agent",
            "error": null,
            "metadata": {"source": "test"}
        })
    }

    #[test]
    fn project_status_outputs_stable_shape() {
        let status = project_status(&status_input()).unwrap();

        assert_eq!(status["profile"], PROFILE);
        assert_eq!(status["kind"], STATUS_KIND);
        assert_eq!(status["package_id"], "easynet.desktop.menubar");
        assert_eq!(status["package_version"], "0.1.0");
        assert_eq!(status["projected_state"], "running");
        assert_eq!(status["pid"], 12345);
        assert_eq!(status["metadata"]["source"], "test");
    }

    #[test]
    fn project_status_accepts_version_alias_for_package_version() {
        let mut input = status_input();
        let obj = input.as_object_mut().unwrap();
        obj.remove("package_version");

        let status = project_status(&input).unwrap();

        assert_eq!(status["package_version"], "0.1.0");
    }

    #[test]
    fn project_status_rejects_invalid_projected_state() {
        let mut input = status_input();
        input["projected_state"] = json!("booted");

        let err = project_status(&input).unwrap_err();

        assert_eq!(
            err,
            CompanionContractError::InvalidField(
                "projected_state",
                "must be one of disabled, unsupported_platform, unsupported_session, not_installed, installed_disabled, ready_stopped, starting, running, stale, error".to_string()
            )
        );
    }

    #[test]
    fn project_action_result_outputs_stable_shape() {
        let input = json!({
            "package_id": "easynet.desktop.menubar",
            "action": "enable",
            "status_before": null,
            "status_after": status_input(),
            "changed": true,
            "error": null
        });

        let result = project_action_result(&input).unwrap();

        assert_eq!(result["profile"], PROFILE);
        assert_eq!(result["kind"], ACTION_KIND);
        assert_eq!(result["action"], "enable");
        assert_eq!(result["changed"], true);
        assert_eq!(result["status_after"]["kind"], STATUS_KIND);
    }

    #[test]
    fn project_action_result_rejects_missing_package_id() {
        let input = json!({
            "action": "status",
            "changed": false,
            "status_before": null,
            "status_after": null
        });

        let err = project_action_result(&input).unwrap_err();

        assert_eq!(err, CompanionContractError::MissingField("package_id"));
    }
}

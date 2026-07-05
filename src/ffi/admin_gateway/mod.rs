// EasyNet CLI — Admin + Gateway C ABI projection
// ===============================================
//
// File: src/ffi/admin_gateway/mod.rs
// Description: C ABI AdminClient/Gateway projection helpers for daemon SDK
//              admin carriers, gateway status, and agent records.
//
// Protocol Responsibility
// -----------------------
// Expose Admin + Gateway DTO construction without letting language facades own
// daemon lifecycle status classification or agent lifecycle carrier shapes.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at the exported
// boundary. Delegate Admin + Gateway carrier and projection semantics to
// `daemon::admin_gateway_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Admin + Gateway profile projection. Runtime Core remains the
// only submit/observe path for returned Invocation carriers.

use std::os::raw::c_char;

use crate::daemon::admin_gateway_contract::{
    build_agent_list_invocation, build_agent_refresh_invocation, build_agent_start_invocation,
    build_agent_stop_invocation, build_revoke_device_invocation, build_session_create_invocation,
    build_session_delete_invocation, build_session_list_invocation, project_agent_lifecycle_result,
    project_agent_records, project_device_admin_result, project_device_session_page,
    project_device_session_result, project_gateway_status, AdminGatewayError,
};
use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};

/// Build a complete Invocation JSON carrier for daemon `agent.list`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_agent_list_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_agent_list_invocation",
        "out_invocation_json",
        "request_json",
        build_agent_list_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `agent.start`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_agent_start_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_agent_start_invocation",
        "out_invocation_json",
        "request_json",
        build_agent_start_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `agent.stop`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_agent_stop_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_agent_stop_invocation",
        "out_invocation_json",
        "request_json",
        build_agent_stop_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `agent.refresh`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_agent_refresh_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_agent_refresh_invocation",
        "out_invocation_json",
        "request_json",
        build_agent_refresh_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `session.list`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_session_list_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_session_list_invocation",
        "out_invocation_json",
        "request_json",
        build_session_list_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `session.create`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_session_create_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_session_create_invocation",
        "out_invocation_json",
        "request_json",
        build_session_create_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `session.delete`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_session_delete_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_session_delete_invocation",
        "out_invocation_json",
        "request_json",
        build_session_delete_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `federation.revoke`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_build_revoke_device_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_admin_build_revoke_device_invocation",
        "out_invocation_json",
        "request_json",
        build_revoke_device_invocation,
    )
}

/// Project daemon lifecycle/status JSON into the SDK GatewayStatus DTO.
///
/// # Safety
/// `status_json` must be a valid UTF-8 C string and `out_status_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_project_gateway_status(
    handle: EasynetHandle,
    status_json: *const c_char,
    out_status_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        status_json,
        out_status_json,
        "easynet_admin_project_gateway_status",
        "out_status_json",
        "status_json",
        project_gateway_status,
    )
}

/// Project daemon `agent.list` JSON into SDK AgentRecord page DTOs.
///
/// # Safety
/// `agents_json` must be a valid UTF-8 C string and `out_agents_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_project_agent_records(
    handle: EasynetHandle,
    agents_json: *const c_char,
    out_agents_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        agents_json,
        out_agents_json,
        "easynet_admin_project_agent_records",
        "out_agents_json",
        "agents_json",
        project_agent_records,
    )
}

/// Project daemon agent lifecycle ability results into SDK Admin result DTOs.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_result_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_project_agent_lifecycle_result(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        result_json,
        out_result_json,
        "easynet_admin_project_agent_lifecycle_result",
        "out_result_json",
        "result_json",
        project_agent_lifecycle_result,
    )
}

/// Project daemon `session.list` JSON into SDK DeviceSessionPage DTOs.
///
/// # Safety
/// `sessions_json` must be a valid UTF-8 C string and `out_sessions_json` must
/// be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_project_device_session_page(
    handle: EasynetHandle,
    sessions_json: *const c_char,
    out_sessions_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        sessions_json,
        out_sessions_json,
        "easynet_admin_project_device_session_page",
        "out_sessions_json",
        "sessions_json",
        project_device_session_page,
    )
}

/// Project daemon `session.create` JSON into an SDK DeviceSession DTO.
///
/// # Safety
/// `session_json` must be a valid UTF-8 C string and `out_session_json` must
/// be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_project_device_session_result(
    handle: EasynetHandle,
    session_json: *const c_char,
    out_session_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        session_json,
        out_session_json,
        "easynet_admin_project_device_session_result",
        "out_session_json",
        "session_json",
        project_device_session_result,
    )
}

/// Project daemon device-admin mutation results into SDK Admin result DTOs.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_result_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_admin_project_device_admin_result(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    project_admin_gateway_json(
        handle,
        result_json,
        out_result_json,
        "easynet_admin_project_device_admin_result",
        "out_result_json",
        "result_json",
        project_device_admin_result,
    )
}

fn project_admin_gateway_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, AdminGatewayError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "admin_gateway",
        },
        project,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
    use crate::ffi::errors::{EASYNET_OK, ERR_INVALID_ARG, ERR_INVALID_HANDLE};
    use serde_json::Value;
    use std::ffi::{CStr, CString};

    fn handle() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    fn read_json(ptr: *mut c_char) -> Value {
        let value = unsafe { serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(ptr) };
        value
    }

    fn base_request(extra: Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
        });
        let Value::Object(extra) = extra else {
            return CString::new(request.to_string()).unwrap();
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        CString::new(request.to_string()).unwrap()
    }

    #[test]
    fn admin_build_agent_start_invocation_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "name": "codex",
            "agent_type": "codex"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_admin_build_agent_start_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "agent.start");
        assert_eq!(value["args"]["name"], "codex");
        release(handle);
    }

    #[test]
    fn admin_project_gateway_status_preserves_readiness_flags() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
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
                    "session_admitted": true
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_admin_project_gateway_status(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["ready"], true);
        assert_eq!(value["runtime_ready"], true);
        release(handle);
    }

    #[test]
    fn admin_project_agent_records_derives_owner() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "agents": [{
                    "name": "codex",
                    "ura": "easynet:///r/example/agent/alice.codex",
                    "runtime": "codex",
                    "root_exists": true
                }]
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_admin_project_agent_records(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["items"][0]["owner_ura"],
            "easynet:///r/example/user/alice"
        );
        release(handle);
    }

    #[test]
    fn admin_project_device_sessions_maps_session_list_output() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "sessions": [{
                    "id": "session-1",
                    "tenant": "example",
                    "node": "dev-a",
                    "started_unix_ms": 1767225600000i64
                }]
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_admin_project_device_session_page(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "device_sessions");
        assert_eq!(value["items"][0]["session_id"], "session-1");
        assert_eq!(
            value["items"][0]["device_ura"],
            "easynet:///r/example/device/dev-a"
        );
        release(handle);
    }

    #[test]
    fn admin_build_session_create_invocation_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "device_ura": "easynet:///r/example/device/dev-a",
            "hub_ura": "easynet:///r/example/hub",
            "session_kind": "remote_desktop",
            "expires_unix_ms": 1893456000000i64
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_admin_build_session_create_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "session.create");
        assert_eq!(value["args"]["session_kind"], "remote_desktop");
        release(handle);
    }

    #[test]
    fn admin_project_device_session_result_maps_create_output() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "device_ura": "easynet:///r/example/device/dev-a",
                "hub_ura": "easynet:///r/example/hub",
                "session_kind": "remote_desktop",
                "expires_unix_ms": 1893456000000i64,
                "result": {
                    "session_id": "dev-session-1",
                    "state": "active",
                    "created_unix_ms": 1767225600000i64
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_admin_project_device_session_result(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "device_session");
        assert_eq!(value["session_id"], "dev-session-1");
        assert_eq!(value["metadata"]["source"], "session.create");
        release(handle);
    }

    #[test]
    fn admin_build_revoke_device_invocation_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "device_ura": "easynet:///r/example/device/dev-a",
            "reason": "operator/key rotation"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_admin_build_revoke_device_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "federation.revoke");
        assert_eq!(
            value["args"]["agent_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(value["args"]["reason"], "operator/key rotation");
        release(handle);
    }

    #[test]
    fn admin_project_device_admin_result_preserves_device_context() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "operation": "federation.revoke",
                "device_ura": "easynet:///r/example/device/dev-a",
                "result": {"ack": true, "was_active": true}
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_admin_project_device_admin_result(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "device_admin_result");
        assert_eq!(value["operation"], "federation.revoke");
        assert_eq!(value["device_ura"], "easynet:///r/example/device/dev-a");
        assert_eq!(value["ack"], true);
        release(handle);
    }

    #[test]
    fn admin_agent_stop_rejects_invalid_target() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "agent_ura": "easynet:///r/example/device/dev-a"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_admin_build_agent_stop_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn admin_projection_rejects_invalid_handle_after_zeroing_output() {
        let raw = base_request(serde_json::json!({
            "name": "codex",
            "agent_type": "codex"
        }));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_admin_build_agent_start_invocation(9_999_999, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

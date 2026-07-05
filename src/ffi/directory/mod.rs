// EasyNet CLI — Directory C ABI projection
// =========================================
//
// File: src/ffi/directory/mod.rs
// Description: C ABI DirectoryClient projection helpers for daemon SDK
//              resolve/read-model carriers and result DTOs.
//
// Protocol Responsibility
// -----------------------
// Expose Directory DTO construction without letting language facades own
// daemon read-model ability names, pagination rules, or row projection logic.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at the exported
// boundary. Delegate Directory carrier and projection semantics to
// `protocol::directory_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Directory profile projection. Runtime Core remains the only
// submit/observe path for returned Invocation carriers.

use std::os::raw::c_char;

use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};
use crate::protocol::directory_contract::{
    build_list_abilities_invocation, build_list_agents_invocation, build_list_devices_invocation,
    build_resolve_invocation, project_ability_page, project_agent_page, project_device_page,
    project_resolved_ref, DirectoryError,
};

/// Build a complete Invocation JSON carrier for daemon `node.list`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_build_list_devices_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_directory_build_list_devices_invocation",
        "out_invocation_json",
        "request_json",
        build_list_devices_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `agent.list`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_build_list_agents_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_directory_build_list_agents_invocation",
        "out_invocation_json",
        "request_json",
        build_list_agents_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `meta.list_abilities`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_build_list_abilities_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_directory_build_list_abilities_invocation",
        "out_invocation_json",
        "request_json",
        build_list_abilities_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `namespace.resolve`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_build_resolve_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_directory_build_resolve_invocation",
        "out_invocation_json",
        "request_json",
        build_resolve_invocation,
    )
}

/// Project daemon `node.list` output into a Directory device page.
///
/// # Safety
/// `devices_json` must be a valid UTF-8 C string and `out_page_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_project_device_page(
    handle: EasynetHandle,
    devices_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        devices_json,
        out_page_json,
        "easynet_directory_project_device_page",
        "out_page_json",
        "devices_json",
        project_device_page,
    )
}

/// Project daemon `agent.list` output into a Directory agent page.
///
/// # Safety
/// `agents_json` must be a valid UTF-8 C string and `out_page_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_project_agent_page(
    handle: EasynetHandle,
    agents_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        agents_json,
        out_page_json,
        "easynet_directory_project_agent_page",
        "out_page_json",
        "agents_json",
        project_agent_page,
    )
}

/// Project daemon `meta.list_abilities` output into a Directory ability page.
///
/// # Safety
/// `abilities_json` must be a valid UTF-8 C string and `out_page_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_project_ability_page(
    handle: EasynetHandle,
    abilities_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        abilities_json,
        out_page_json,
        "easynet_directory_project_ability_page",
        "out_page_json",
        "abilities_json",
        project_ability_page,
    )
}

/// Project daemon `namespace.resolve` output into a stable resolved-ref DTO.
///
/// # Safety
/// `answer_json` must be a valid UTF-8 C string and `out_resolved_ref_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_directory_project_resolved_ref(
    handle: EasynetHandle,
    answer_json: *const c_char,
    out_resolved_ref_json: *mut *mut c_char,
) -> i32 {
    project_directory_json(
        handle,
        answer_json,
        out_resolved_ref_json,
        "easynet_directory_project_resolved_ref",
        "out_resolved_ref_json",
        "answer_json",
        project_resolved_ref,
    )
}

fn project_directory_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, DirectoryError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "directory_identity",
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
            "metadata": {"request_id": "directory-1"}
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
    fn directory_build_list_devices_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({"limit": 2}));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_directory_build_list_devices_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "node.list");
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.node.list@1.0.0"
        );
        release(handle);
    }

    #[test]
    fn directory_build_list_abilities_uses_single_catalog_ability() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "scope": "local",
            "owner_ura": "easynet:///r/example/device/dev-a"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_directory_build_list_abilities_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "meta.list_abilities");
        assert_eq!(
            value["args"],
            serde_json::json!({
                "scope": "local",
                "agent_ura": "easynet:///r/example/device/dev-a"
            })
        );
        release(handle);
    }

    #[test]
    fn directory_build_resolve_projects_namespace_resolve_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "query_name": "easynet:///r/example/device/dev-a",
            "ability_name": "agent.list",
            "qtype": "route"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_directory_build_resolve_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "namespace.resolve");
        assert_eq!(
            value["args"]["queryName"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(value["args"]["abilityName"], "agent.list");
        assert_eq!(value["args"]["qtype"], "RESOLVE_TYPE_ROUTE");
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.namespace.resolve@1.0.0"
        );
        release(handle);
    }

    #[test]
    fn directory_project_device_page_projects_page() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "result": {
                    "nodes": [
                        {
                            "node_id": "dev-a",
                            "agent_ura": "easynet:///r/example/device/dev-a",
                            "state": "online",
                            "online": true
                        }
                    ]
                },
                "limit": 1
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_directory_project_device_page(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "directory_identity");
        assert_eq!(value["kind"], "device_page");
        assert_eq!(value["items"][0]["node_id"], "dev-a");
        release(handle);
    }

    #[test]
    fn directory_project_resolved_ref_projects_final_route_answer() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "answerKind": "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
                "canonicalName": "easynet:///r/example/device/dev-a",
                "ownerUra": "easynet:///r/example/device/dev-a",
                "abilityUra": "easynet:///r/example/ability/device.dev-a.agent.list",
                "routeUra": "route-ref::easynet:///r/example/ability/device.dev-a.agent.list",
                "nextHop": {
                    "localDeviceAbility": {
                        "deviceUra": "easynet:///r/example/device/dev-a",
                        "dispatchName": "agent.list"
                    }
                },
                "selectedRoute": {"reason": "ROUTE_REASON_LOCAL_DEVICE"},
                "routeCandidates": [],
                "records": [],
                "releaseProfile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_directory_project_resolved_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "directory_identity");
        assert_eq!(value["kind"], "resolved_ref");
        assert_eq!(value["answer_kind"], "RESOLVE_ANSWER_KIND_FINAL_ROUTE");
        assert_eq!(value["metadata"]["source"], "namespace.resolve");
        release(handle);
    }

    #[test]
    fn directory_rejects_max_page_overflow_after_zeroing_output() {
        let handle = handle();
        let raw = base_request(serde_json::json!({"limit": 1000000}));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_directory_build_list_agents_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn directory_projection_rejects_invalid_handle_after_zeroing_output() {
        let raw = base_request(serde_json::json!({"limit": 2}));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_directory_build_list_devices_invocation(9_999_999, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

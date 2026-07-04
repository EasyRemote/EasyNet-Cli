// EasyNet CLI — Publication C ABI projection
// ===========================================
//
// File: src/ffi/publication/mod.rs
// Description: C ABI PublicationClient projection helpers for daemon SDK
//              ResourceRef, package validation, and publication carriers.
//
// Protocol Responsibility
// -----------------------
// Expose Publication DTO construction without letting language facades
// hand-build ResourceRefs or daemon system-ability Invocation carriers.
// This module does not execute product host code or own daemon publication
// state machines.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, and JSON validation at the exported boundary. Delegate
// ResourceRef, package validation, and carrier semantics to
// `daemon::publication_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Publication profile projection. Runtime Core remains the
// only submit/observe path for the returned Invocation carriers.

use std::os::raw::c_char;

use crate::daemon::publication_contract::{
    build_deploy_invocation, build_list_abilities_invocation, build_local_resource_ref,
    build_show_ability_invocation, build_unpublish_invocation, project_ability_deploy_result,
    project_ability_unpublish_result, project_published_ability_page,
    project_published_ability_record, validate_package,
};
use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};

/// Build a daemon-authored local filesystem ResourceRef DTO.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_resource_ref_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_build_resource_ref(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_resource_ref_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        request_json,
        out_resource_ref_json,
        "easynet_publication_build_resource_ref",
        "out_resource_ref_json",
        "request_json",
        build_local_resource_ref,
    )
}

/// Validate an ability package directory and return package manifest facts.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_validation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_validate_package(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_validation_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        request_json,
        out_validation_json,
        "easynet_publication_validate_package",
        "out_validation_json",
        "request_json",
        validate_package,
    )
}

/// Build a complete Invocation JSON carrier for daemon `ability.deploy`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_build_deploy_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_publication_build_deploy_invocation",
        "out_invocation_json",
        "request_json",
        build_deploy_invocation,
    )
}

/// Project daemon `ability.deploy` output into an SDK deploy-result DTO.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_result_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_project_deploy_result(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        result_json,
        out_result_json,
        "easynet_publication_project_deploy_result",
        "out_result_json",
        "result_json",
        project_ability_deploy_result,
    )
}

/// Build a complete Invocation JSON carrier for daemon `meta.list_abilities`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_build_list_abilities_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_publication_build_list_abilities_invocation",
        "out_invocation_json",
        "request_json",
        build_list_abilities_invocation,
    )
}

/// Project daemon `meta.list_abilities` output into a Publication ability page.
///
/// # Safety
/// `page_json` must be a valid UTF-8 C string and `out_page_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_project_ability_page(
    handle: EasynetHandle,
    page_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        page_json,
        out_page_json,
        "easynet_publication_project_ability_page",
        "out_page_json",
        "page_json",
        project_published_ability_page,
    )
}

/// Build a complete Invocation JSON carrier for daemon `meta.list_abilities`
/// scoped to one target AbilityDescriptorRef.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_build_show_ability_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_publication_build_show_ability_invocation",
        "out_invocation_json",
        "request_json",
        build_show_ability_invocation,
    )
}

/// Project daemon `meta.list_abilities` output into one PublishedAbility DTO.
///
/// # Safety
/// `record_json` must be a valid UTF-8 C string and `out_ability_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_project_ability_record(
    handle: EasynetHandle,
    record_json: *const c_char,
    out_ability_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        record_json,
        out_ability_json,
        "easynet_publication_project_ability_record",
        "out_ability_json",
        "record_json",
        project_published_ability_record,
    )
}

/// Build a complete Invocation JSON carrier for daemon `ability.unpublish`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_build_unpublish_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_publication_build_unpublish_invocation",
        "out_invocation_json",
        "request_json",
        build_unpublish_invocation,
    )
}

/// Project daemon `ability.unpublish` output into an SDK mutation DTO.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_result_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_publication_project_unpublish_result(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    project_publication_json(
        handle,
        result_json,
        out_result_json,
        "easynet_publication_project_unpublish_result",
        "out_result_json",
        "result_json",
        project_ability_unpublish_result,
    )
}

fn project_publication_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(
        &serde_json::Value,
    )
        -> Result<serde_json::Value, crate::daemon::publication_contract::PublicationError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "publication",
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
    use std::io::Write;

    fn handle() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    fn read_json(ptr: *mut c_char) -> Value {
        let value = unsafe { serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(ptr) };
        value
    }

    fn write_package(dir: &std::path::Path) {
        let body = r#"{
            "name": "weather",
            "namespace": "er",
            "description": "Weather stream",
            "input_schema": {"type": "object", "properties": {}},
            "exec": {
                "kind": "host_stream",
                "host_socket": "/tmp/easynet-weather.sock",
                "function": "weather.stream"
            }
        }"#;
        let mut file = std::fs::File::create(dir.join("ability.json")).unwrap();
        file.write_all(body.as_bytes()).unwrap();
    }

    fn nonce() -> &'static str {
        "AQIDBAUGBwgJCgsMDQ4PEA=="
    }

    #[test]
    fn publication_validate_package_projects_manifest() {
        let handle = handle();
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path());
        let raw =
            CString::new(serde_json::json!({"path": dir.path().display().to_string()}).to_string())
                .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_publication_validate_package(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["valid"], true);
        assert_eq!(value["manifest"]["wire_key"], "er.weather");
        release(handle);
    }

    #[test]
    fn publication_build_resource_ref_rejects_invalid_handle_after_zeroing_output() {
        let raw =
            CString::new(serde_json::json!({"path": "/tmp", "capability": "read"}).to_string())
                .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_publication_build_resource_ref(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn publication_build_deploy_invocation_projects_complete_tuple() {
        let handle = handle();
        let resource_ref = serde_json::json!({
            "resource_ura": "easynet:///r/example/resource/device.dev-a/fs/tmp/pkg",
            "owner_ura": "easynet:///r/example/device/dev-a",
            "namespace": "fs",
            "display_path": "tmp/pkg",
            "capability": "read",
            "expires_unix_ms": 4102444800000i64,
            "revision": "fs-local-mapping-v1"
        });
        let raw = CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/example/agent/alice.sdk",
                "callee_ura": "easynet:///r/example/device/dev-a",
                "subject_ura": "easynet:///r/example/device/dev-a",
                "descriptor_version": "1.0.0",
                "nonce_base64": nonce(),
                "causal_context": {"form": "none"},
                "resource_ref": resource_ref,
                "node_id": "local"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_publication_build_deploy_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "ability.deploy");
        assert_eq!(value["args"]["node_id"], "local");
        assert!(value["descriptor_ref"]
            .as_str()
            .unwrap()
            .contains("ability.deploy@1.0.0"));
        release(handle);
    }

    #[test]
    fn publication_project_deploy_result_projects_daemon_output() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "public_name": "weather",
                "namespace": "er",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                "node_id": "dev-a",
                "mutated_by": "easynet:///r/example/device/dev-a",
                "install_id": "install-1",
                "bundle": "tmp/pkg",
                "state": "ACTIVE"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_publication_project_deploy_result(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "ability_deploy_result");
        assert_eq!(value["state"], "enabled");
        assert_eq!(value["metadata"]["source_ability"], "ability.deploy");
        release(handle);
    }

    #[test]
    fn publication_build_list_abilities_invocation_projects_complete_tuple() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/example/agent/alice.sdk",
                "callee_ura": "easynet:///r/example/device/dev-a",
                "subject_ura": "easynet:///r/example/device/dev-a",
                "descriptor_version": "1.0.0",
                "nonce_base64": nonce(),
                "causal_context": {"form": "none"},
                "limit": 25,
                "owner_ura": "easynet:///r/example/device/dev-a"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_publication_build_list_abilities_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "meta.list_abilities");
        assert_eq!(
            value["args"]["agent_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert!(value["descriptor_ref"]
            .as_str()
            .unwrap()
            .contains("meta.list_abilities@1.0.0"));
        release(handle);
    }

    #[test]
    fn publication_project_ability_page_projects_daemon_catalog_rows() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "result": {
                    "abilities": [{
                        "name": "weather",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "1.0.0",
                        "schema_hash": "sha256:abc",
                        "metadata": {"source": "registry"}
                    }]
                },
                "limit": 50
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_publication_project_ability_page(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "publication");
        assert_eq!(value["item_kind"], "published_ability");
        assert_eq!(
            value["items"][0]["descriptor"]["descriptor_version"],
            "1.0.0"
        );
        assert_eq!(
            value["items"][0]["descriptor"]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
        );
        release(handle);
    }

    #[test]
    fn publication_build_show_ability_invocation_targets_descriptor_ref() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/example/agent/alice.sdk",
                "callee_ura": "easynet:///r/example/device/dev-a",
                "subject_ura": "easynet:///r/example/device/dev-a",
                "descriptor_version": "1.0.0",
                "nonce_base64": nonce(),
                "causal_context": {"form": "none"},
                "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
                "owner_ura": "easynet:///r/example/device/dev-a"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_publication_build_show_ability_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "meta.list_abilities");
        assert_eq!(
            value["args"]["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather"
        );
        release(handle);
    }

    #[test]
    fn publication_project_ability_record_selects_descriptor_ref() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@2.0.0",
                "result": {
                    "abilities": [
                        {
                            "name": "weather",
                            "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                            "owner_ura": "easynet:///r/example/device/dev-a",
                            "version": "1.0.0"
                        },
                        {
                            "name": "weather",
                            "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                            "owner_ura": "easynet:///r/example/device/dev-a",
                            "version": "2.0.0"
                        }
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_publication_project_ability_record(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["descriptor"]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@2.0.0"
        );
        release(handle);
    }

    #[test]
    fn publication_build_unpublish_invocation_rejects_device_ura() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/example/agent/alice.sdk",
                "callee_ura": "easynet:///r/example/device/dev-a",
                "subject_ura": "easynet:///r/example/device/dev-a",
                "descriptor_version": "1.0.0",
                "nonce_base64": nonce(),
                "causal_context": {"form": "none"},
                "ability_ura": "easynet:///r/example/device/dev-a"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_publication_build_unpublish_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn publication_project_unpublish_result_projects_daemon_output() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "descriptor_version": "1.0.0",
                "result": {
                    "ok": true,
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "public_name": "weather",
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                    "removed_path": "/tmp/easynet/abilities/weather.ability.json",
                    "content_hash": "sha256:abc"
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_publication_project_unpublish_result(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "ability_unpublished");
        assert_eq!(value["status"], "unpublished");
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
        );
        assert_eq!(value["metadata"]["source_ability"], "ability.unpublish");
        release(handle);
    }
}

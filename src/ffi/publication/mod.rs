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

use serde_json::Value;

use crate::daemon::publication_contract::{
    build_deploy_invocation, build_local_resource_ref, build_unpublish_invocation,
    validate_package, PublicationError,
};
use crate::ffi::client::handle::{get, EasynetHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

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

fn project_publication_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&Value) -> Result<Value, PublicationError>,
) -> i32 {
    let raw = match read_publication_args(handle, input, output, function, output_name, input_name)
    {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let input = match parse_json_value(raw, function, input_name) {
        Ok(value) => value,
        Err(code) => return code,
    };
    match project(&input) {
        Ok(value) => write_json_output(function, output, value),
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("{function}: {err}"));
            ERR_INVALID_ARG
        }
    }
}

fn read_publication_args<'a>(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
) -> Result<&'a str, i32> {
    if output.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            format!("{function}: {output_name} pointer is null"),
        );
        return Err(ERR_NULL_POINTER);
    }
    unsafe { *output = std::ptr::null_mut() };

    if get(handle).is_none() {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("{function}: handle {handle} is not registered"),
        );
        return Err(ERR_INVALID_HANDLE);
    }

    match read_cstr(input) {
        Ok(raw) => Ok(raw),
        Err(StringError::Null) => {
            set_last_error_code(
                ERR_NULL_POINTER,
                format!("{function}: {input_name} pointer is null"),
            );
            Err(ERR_NULL_POINTER)
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                format!("{function}: {input_name} is not valid UTF-8"),
            );
            Err(ERR_INVALID_UTF8)
        }
    }
}

fn parse_json_value(
    raw: &str,
    function: &'static str,
    input_name: &'static str,
) -> Result<Value, i32> {
    serde_json::from_str(raw).map_err(|err| {
        set_last_error_code(
            ERR_INVALID_ARG,
            format!("{function}: decode {input_name} failed: {err}"),
        );
        ERR_INVALID_ARG
    })
}

fn write_json_output(function: &'static str, output: *mut *mut c_char, value: Value) -> i32 {
    let ptr = alloc_output_cstring(value.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating publication JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *output = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
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
}

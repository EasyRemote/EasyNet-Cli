// EasyNet CLI — Compatibility C ABI projection
// =============================================
//
// File: src/ffi/compatibility/mod.rs
// Description: C ABI CompatibilityClient projection helpers for daemon SDK
//              OpenAI-compatible carriers and result DTOs.
//
// Protocol Responsibility
// -----------------------
// Expose Compatibility DTO construction without letting language facades own
// OpenAI-to-daemon Invocation carrier shapes or chat completion projections.
// This module does not own product HTTP auth, quotas, billing, or SSE fanout.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at the exported
// boundary. Delegate carrier and projection semantics to
// `daemon::compatibility_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Compatibility profile projection. Runtime Core remains the
// only submit/observe path for returned Invocation carriers.

use std::os::raw::c_char;

use crate::daemon::compatibility_contract::{
    build_chat_completion_invocation, build_list_models_invocation,
    build_stream_chat_completion_invocation, project_chat_completion, project_chat_stream,
    project_model_page, CompatibilityError,
};
use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};

/// Build a complete Invocation JSON carrier for daemon `openai.list_models`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_compatibility_build_list_models_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_compatibility_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_compatibility_build_list_models_invocation",
        "out_invocation_json",
        "request_json",
        build_list_models_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `openai.chat_completions`.
///
/// The request must be non-streaming. Use
/// `easynet_compatibility_build_stream_chat_completion_invocation` for stream
/// requests.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_compatibility_build_chat_completion_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_compatibility_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_compatibility_build_chat_completion_invocation",
        "out_invocation_json",
        "request_json",
        build_chat_completion_invocation,
    )
}

/// Build a complete Invocation JSON carrier for streaming daemon
/// `openai.chat_completions`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_compatibility_build_stream_chat_completion_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_compatibility_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_compatibility_build_stream_chat_completion_invocation",
        "out_invocation_json",
        "request_json",
        build_stream_chat_completion_invocation,
    )
}

/// Project daemon `openai.list_models` output into a Compatibility model page.
///
/// # Safety
/// `models_json` must be a valid UTF-8 C string and `out_models_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_compatibility_project_model_page(
    handle: EasynetHandle,
    models_json: *const c_char,
    out_models_json: *mut *mut c_char,
) -> i32 {
    project_compatibility_json(
        handle,
        models_json,
        out_models_json,
        "easynet_compatibility_project_model_page",
        "out_models_json",
        "models_json",
        project_model_page,
    )
}

/// Project daemon `openai.chat_completions` unary output into a Compatibility
/// chat completion DTO.
///
/// # Safety
/// `completion_json` must be a valid UTF-8 C string and
/// `out_completion_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_compatibility_project_chat_completion(
    handle: EasynetHandle,
    completion_json: *const c_char,
    out_completion_json: *mut *mut c_char,
) -> i32 {
    project_compatibility_json(
        handle,
        completion_json,
        out_completion_json,
        "easynet_compatibility_project_chat_completion",
        "out_completion_json",
        "completion_json",
        project_chat_completion,
    )
}

/// Project daemon `openai.chat_completions` streaming envelope into typed
/// Compatibility chunk DTOs.
///
/// # Safety
/// `stream_json` must be a valid UTF-8 C string and `out_stream_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_compatibility_project_chat_stream(
    handle: EasynetHandle,
    stream_json: *const c_char,
    out_stream_json: *mut *mut c_char,
) -> i32 {
    project_compatibility_json(
        handle,
        stream_json,
        out_stream_json,
        "easynet_compatibility_project_chat_stream",
        "out_stream_json",
        "stream_json",
        project_chat_stream,
    )
}

fn project_compatibility_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, CompatibilityError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "compatibility",
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

    fn model_id() -> &'static str {
        "easynet:///r/example/ability/alice.codex.chat"
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
    fn compatibility_build_chat_completion_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "request": {
                "model": model_id(),
                "messages": [{"role": "user", "content": "hello"}]
            }
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_compatibility_build_chat_completion_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "openai.chat_completions"
        );
        assert_eq!(value["args"]["request"]["model"], model_id());
        release(handle);
    }

    #[test]
    fn compatibility_build_stream_sets_stream_true() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "request": {
                "model": model_id(),
                "messages": [{"role": "user", "content": "hello"}]
            }
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_compatibility_build_stream_chat_completion_invocation(
                handle,
                raw.as_ptr(),
                &mut out,
            )
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["args"]["request"]["stream"], true);
        release(handle);
    }

    #[test]
    fn compatibility_project_model_page_validates_model() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "object": "list",
                "data": [{
                    "id": model_id(),
                    "object": "model",
                    "created": 0,
                    "owned_by": "easynet",
                    "ability": "codex.chat"
                }]
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_compatibility_project_model_page(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["data"][0]["ability_ref"], model_id());
        release(handle);
    }

    #[test]
    fn compatibility_rejects_invalid_model_after_zeroing_output() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "request": {
                "model": "gpt-5",
                "messages": [{"role": "user", "content": "hello"}]
            }
        }));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_compatibility_build_chat_completion_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn compatibility_projection_rejects_invalid_handle_after_zeroing_output() {
        let raw = CString::new(
            serde_json::json!({
                "object": "list",
                "data": []
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_compatibility_project_model_page(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

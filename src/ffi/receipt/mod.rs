// EasyNet CLI — Receipt C ABI projection
// =======================================
//
// File: src/ffi/receipt/mod.rs
// Description: C ABI ReceiptClient helpers for daemon SDK fetch carriers and
//              receipt DTO projections.
//
// Protocol Responsibility
// -----------------------
// Expose Receipt DTO construction without letting language facades or the C
// ABI boundary own daemon read-model ability names, receipt summary semantics,
// causal-ref construction, or verification claims.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at the exported
// boundary. Delegate Receipt carrier and projection semantics to
// `protocol::receipt_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Receipt profile projection. Runtime Core remains the only
// submit path for returned Invocation carriers; full cryptographic receipt
// verification remains Axon-owned verifier behavior.

use std::os::raw::c_char;

use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};
use crate::protocol::receipt_contract::{
    build_fetch_invocation, build_get_history_invocation, build_list_history_invocation,
    build_trace_invocation, project_causal_ref, project_receipt_chain_verification,
    project_receipt_summary, project_receipt_verification, ReceiptError,
};

/// Build a complete Invocation JSON carrier for daemon `invocation.history.get`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_build_fetch_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_receipt_build_fetch_invocation",
        "out_invocation_json",
        "request_json",
        build_fetch_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `invocation.history.list`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_build_list_history_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_receipt_build_list_history_invocation",
        "out_invocation_json",
        "request_json",
        build_list_history_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `invocation.history.get`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_build_get_history_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_receipt_build_get_history_invocation",
        "out_invocation_json",
        "request_json",
        build_get_history_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `invocation.trace.get`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_build_trace_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_receipt_build_trace_invocation",
        "out_invocation_json",
        "request_json",
        build_trace_invocation,
    )
}

/// Project a receipt-like JSON object into the shared ReceiptSummary DTO.
///
/// # Safety
/// `receipt_json` must be a valid UTF-8 C string and `out_summary_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_project(
    handle: EasynetHandle,
    receipt_json: *const c_char,
    out_summary_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        receipt_json,
        out_summary_json,
        "easynet_receipt_project",
        "out_summary_json",
        "receipt_json",
        project_receipt_summary,
    )
}

/// Return a conservative verification projection for a receipt-like JSON object.
///
/// # Safety
/// `receipt_json` must be a valid UTF-8 C string and `out_verification_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_verify(
    handle: EasynetHandle,
    receipt_json: *const c_char,
    out_verification_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        receipt_json,
        out_verification_json,
        "easynet_receipt_verify",
        "out_verification_json",
        "receipt_json",
        project_receipt_verification,
    )
}

/// Return a daemon/Axon-owned receipt chain continuity projection.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and
/// `out_verification_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_verify_chain(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_verification_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        request_json,
        out_verification_json,
        "easynet_receipt_verify_chain",
        "out_verification_json",
        "request_json",
        project_receipt_chain_verification,
    )
}

/// Build an Invocation causal ref from explicit receipt facts.
///
/// # Safety
/// `receipt_json` must be a valid UTF-8 C string and `out_causal_ref_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_receipt_causal_ref(
    handle: EasynetHandle,
    receipt_json: *const c_char,
    out_causal_ref_json: *mut *mut c_char,
) -> i32 {
    project_receipt_json(
        handle,
        receipt_json,
        out_causal_ref_json,
        "easynet_receipt_causal_ref",
        "out_causal_ref_json",
        "receipt_json",
        project_causal_ref,
    )
}

fn project_receipt_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, ReceiptError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "receipt",
        },
        project,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
    use crate::ffi::errors::{EASYNET_OK, ERR_INVALID_ARG, ERR_INVALID_HANDLE};
    use std::ffi::{CStr, CString};

    fn handle() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    fn read_json(ptr: *mut c_char) -> serde_json::Value {
        let value = unsafe { serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(ptr) };
        value
    }

    fn last_error_text() -> String {
        let ptr = crate::ffi::errors::easynet_last_error();
        if ptr.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
    }

    fn base_fetch_request(extra: serde_json::Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "descriptor_ref": "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "receipt-fetch-1"}
        });
        let serde_json::Value::Object(extra) = extra else {
            return CString::new(request.to_string()).unwrap();
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        CString::new(request.to_string()).unwrap()
    }

    fn base_history_request(extra: serde_json::Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "timeout_ms": 2500,
            "metadata": {"request_id": "history-1"},
            "arguments": {"key": {"request_id": "req-123"}}
        });
        let serde_json::Value::Object(extra) = extra else {
            return CString::new(request.to_string()).unwrap();
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        CString::new(request.to_string()).unwrap()
    }

    #[test]
    fn receipt_build_fetch_projects_invocation_history_carrier() {
        let handle = handle();
        let raw = base_fetch_request(serde_json::json!({"request_id": "req-123"}));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_receipt_build_fetch_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK, "{}", last_error_text());
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "invocation.history.get"
        );
        assert_eq!(
            value["args"],
            serde_json::json!({"key": {"request_id": "req-123"}})
        );
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0"
        );
        release(handle);
    }

    #[test]
    fn receipt_build_list_history_projects_complete_carrier() {
        let handle = handle();
        let raw = base_history_request(serde_json::json!({}));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_receipt_build_list_history_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK, "{}", last_error_text());
        let value = read_json(out);
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0"
        );
        assert_eq!(
            value["metadata"]["system_ability"],
            "invocation.history.list"
        );
        assert_eq!(value["metadata"]["timeout_ms"], 2500);
        assert_eq!(value["args"]["key"]["request_id"], "req-123");
        release(handle);
    }

    #[test]
    fn receipt_build_get_history_projects_complete_carrier() {
        let handle = handle();
        let raw = base_history_request(serde_json::json!({}));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_receipt_build_get_history_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK, "{}", last_error_text());
        let value = read_json(out);
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0"
        );
        assert_eq!(
            value["metadata"]["system_ability"],
            "invocation.history.get"
        );
        release(handle);
    }

    #[test]
    fn receipt_build_trace_projects_complete_carrier() {
        let handle = handle();
        let raw = base_history_request(serde_json::json!({
            "arguments": {"key": {"trace_id": "trace-1"}}
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_receipt_build_trace_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.trace.get@1.0.0"
        );
        assert_eq!(value["metadata"]["system_ability"], "invocation.trace.get");
        assert_eq!(value["args"]["key"]["trace_id"], "trace-1");
        release(handle);
    }

    #[test]
    fn receipt_project_normalizes_summary_without_verification_claim() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                "invocation_id": "inv-1",
                "state": "completed",
                "verified": true,
                "output": {"ok": true},
                "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "metadata": {"source": "test"}
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_receipt_project(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["state"], "completed");
        assert_eq!(value["verified"], false);
        assert_eq!(value["output"]["ok"], true);
        assert_eq!(value["metadata"]["source"], "test");
        assert_eq!(value["metadata"]["verification_claim_downgraded"], true);
        release(handle);
    }

    #[test]
    fn receipt_project_rejects_missing_state() {
        let handle = handle();
        let raw = CString::new(serde_json::json!({"invocation_id": "inv-1"}).to_string()).unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_receipt_project(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn receipt_verify_is_conservative_for_summary_input() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                "invocation_id": "inv-1",
                "state": "completed",
                "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_receipt_verify(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["verified"], false);
        assert_eq!(value["method"], "summary_projection");
        assert_eq!(value["metadata"]["has_receipt_hash"], true);
        release(handle);
    }

    #[test]
    fn receipt_verify_chain_projects_continuity() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "receipts": [
                    {
                        "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                        "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    },
                    {
                        "receipt_ura": "easynet:///r/acme/resource/invocations/inv-2/receipt/1",
                        "self_hash_hex": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "prev_receipt_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_receipt_verify_chain(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["verified"], false);
        assert_eq!(value["continuous"], true);
        assert_eq!(value["method"], "daemon_receipt_chain_continuity");
        release(handle);
    }

    #[test]
    fn receipt_causal_ref_requires_explicit_hash_pair() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                "state": "completed"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_receipt_causal_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn receipt_causal_ref_builds_scalar_context_from_hash_pair() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                "state": "completed",
                "receipt_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_receipt_causal_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["causal_context"]["receipt_hash_hex"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(value["verified"], false);
        release(handle);
    }

    #[test]
    fn receipt_project_rejects_invalid_handle_after_zeroing_output() {
        let raw = CString::new(serde_json::json!({"state": "completed"}).to_string()).unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_receipt_project(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

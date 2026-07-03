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
// `daemon::receipt_contract`.
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

use crate::daemon::receipt_contract::{
    build_fetch_invocation, project_causal_ref, project_receipt_summary,
    project_receipt_verification, ReceiptError,
};
use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};

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

    fn base_fetch_request(extra: serde_json::Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
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

    #[test]
    fn receipt_build_fetch_projects_invocation_history_carrier() {
        let handle = handle();
        let raw = base_fetch_request(serde_json::json!({"request_id": "req-123"}));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_receipt_build_fetch_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
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
        assert_eq!(value["level"], "summary_projection");
        assert_eq!(value["details"]["has_receipt_hash"], true);
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

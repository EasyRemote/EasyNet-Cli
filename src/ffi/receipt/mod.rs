// EasyNet CLI — Receipt C ABI projection
// =======================================
//
// File: src/ffi/receipt/mod.rs
// Description: C ABI ReceiptClient projection helpers for the daemon SDK.
//
// Protocol Responsibility
// -----------------------
// Project receipt-like JSON into binding-facing SDK DTOs without changing
// Axon receipt semantics. This module does not implement canonical receipt
// verification and never claims cryptographic validity for summary-only data.
//
// Implementation Approach
// -----------------------
// Keep C ABI pointer validation and allocation at the exported boundary, then
// delegate receipt field interpretation to `ReceiptProjection`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli owns the daemon SDK projection. Full cryptographic receipt
// verification remains Axon-owned verifier behavior.

use std::os::raw::c_char;

use crate::ffi::client::handle::{get, EasynetHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

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
    let raw = match read_receipt_args(
        handle,
        receipt_json,
        out_summary_json,
        "easynet_receipt_project",
        "out_summary_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let input = match parse_receipt_object(raw, "easynet_receipt_project") {
        Ok(input) => input,
        Err(code) => return code,
    };
    let summary = match ReceiptProjection::from_object(input) {
        Ok(projection) => projection.summary_json(),
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("easynet_receipt_project: {err}"));
            return ERR_INVALID_ARG;
        }
    };
    write_json_output("easynet_receipt_project", out_summary_json, summary)
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
    let raw = match read_receipt_args(
        handle,
        receipt_json,
        out_verification_json,
        "easynet_receipt_verify",
        "out_verification_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let input = match parse_receipt_object(raw, "easynet_receipt_verify") {
        Ok(input) => input,
        Err(code) => return code,
    };
    let projection = ReceiptProjection::from_object_lossy(input);
    write_json_output(
        "easynet_receipt_verify",
        out_verification_json,
        projection.verification_json(),
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
    let raw = match read_receipt_args(
        handle,
        receipt_json,
        out_causal_ref_json,
        "easynet_receipt_causal_ref",
        "out_causal_ref_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let input = match parse_receipt_object(raw, "easynet_receipt_causal_ref") {
        Ok(input) => input,
        Err(code) => return code,
    };
    let projection = ReceiptProjection::from_object_lossy(input);
    let causal_ref = match projection.causal_ref_json() {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("easynet_receipt_causal_ref: {err}"),
            );
            return ERR_INVALID_ARG;
        }
    };
    write_json_output(
        "easynet_receipt_causal_ref",
        out_causal_ref_json,
        causal_ref,
    )
}

fn read_receipt_args<'a>(
    handle: EasynetHandle,
    receipt_json: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
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

    match read_cstr(receipt_json) {
        Ok(raw) => Ok(raw),
        Err(StringError::Null) => {
            set_last_error_code(
                ERR_NULL_POINTER,
                format!("{function}: receipt_json pointer is null"),
            );
            Err(ERR_NULL_POINTER)
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                format!("{function}: receipt_json is not valid UTF-8"),
            );
            Err(ERR_INVALID_UTF8)
        }
    }
}

fn parse_receipt_object(
    raw: &str,
    function: &'static str,
) -> Result<serde_json::Map<String, serde_json::Value>, i32> {
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("{function}: decode JSON failed: {err}"),
            );
            return Err(ERR_INVALID_ARG);
        }
    };
    match value {
        serde_json::Value::Object(obj) => Ok(obj),
        _ => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("{function}: receipt_json must be an object"),
            );
            Err(ERR_INVALID_ARG)
        }
    }
}

fn write_json_output(
    function: &'static str,
    output: *mut *mut c_char,
    value: serde_json::Value,
) -> i32 {
    let ptr = alloc_output_cstring(value.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating receipt JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *output = ptr };
    clear_last_error();
    EASYNET_OK
}

#[derive(Debug, Clone)]
struct ReceiptProjection {
    receipt_ura: Option<String>,
    invocation_id: Option<String>,
    state: Option<String>,
    verified_input: bool,
    output: serde_json::Value,
    error: serde_json::Value,
    causal_ref: Option<String>,
    receipt_hash_hex: Option<String>,
    metadata: serde_json::Map<String, serde_json::Value>,
}

impl ReceiptProjection {
    fn from_object(obj: serde_json::Map<String, serde_json::Value>) -> Result<Self, ReceiptError> {
        let projection = Self::from_object_lossy(obj);
        if projection.state.is_none() {
            return Err(ReceiptError::MissingState);
        }
        Ok(projection)
    }

    fn from_object_lossy(obj: serde_json::Map<String, serde_json::Value>) -> Self {
        let receipt_ura = optional_string(&obj, "receipt_ura");
        let invocation_id = optional_string(&obj, "invocation_id");
        let state =
            optional_string(&obj, "state").or_else(|| optional_string(&obj, "terminal_state"));
        let verified_input = obj
            .get("verified")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let output = obj
            .get("output")
            .or_else(|| obj.get("output_json"))
            .or_else(|| obj.get("payload_json"))
            .or_else(|| obj.get("result_json"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let error = obj.get("error").cloned().unwrap_or(serde_json::Value::Null);
        let causal_ref = optional_string(&obj, "causal_ref");
        let receipt_hash_hex = receipt_hash_hex(&obj);
        let metadata = receipt_metadata(&obj, verified_input, receipt_hash_hex.as_deref());
        Self {
            receipt_ura,
            invocation_id,
            state,
            verified_input,
            output,
            error,
            causal_ref,
            receipt_hash_hex,
            metadata,
        }
    }

    fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "receipt_ura": self.receipt_ura,
            "invocation_id": self.invocation_id,
            "state": self.state.as_deref().unwrap_or("unknown"),
            "verified": false,
            "output": self.output,
            "error": self.error,
            "causal_ref": self.causal_ref,
            "metadata": self.metadata,
        })
    }

    fn verification_json(&self) -> serde_json::Value {
        serde_json::json!({
            "verified": false,
            "level": "summary_projection",
            "reason": "C ABI receipt projection does not perform Axon cryptographic receipt verification",
            "requires_full_receipt": true,
            "receipt_ura": self.receipt_ura,
            "invocation_id": self.invocation_id,
            "state": self.state,
            "details": {
                "has_receipt_hash": self.receipt_hash_hex.is_some(),
                "verified_input_downgraded": self.verified_input,
            },
        })
    }

    fn causal_ref_json(&self) -> Result<serde_json::Value, ReceiptError> {
        let receipt_ura = self
            .receipt_ura
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(ReceiptError::MissingReceiptUra)?;
        let receipt_hash_hex = self
            .receipt_hash_hex
            .as_deref()
            .ok_or(ReceiptError::MissingReceiptHash)?;
        validate_hash_hex(receipt_hash_hex)?;
        Ok(serde_json::json!({
            "receipt_ura": receipt_ura,
            "receipt_hash_hex": receipt_hash_hex,
            "verified": false,
            "causal_context": {
                "form": "scalar",
                "receipt_ura": receipt_ura,
                "receipt_hash_hex": receipt_hash_hex,
            },
        }))
    }
}

#[derive(Debug, thiserror::Error)]
enum ReceiptError {
    #[error("missing field `state`")]
    MissingState,
    #[error("missing field `receipt_ura`")]
    MissingReceiptUra,
    #[error("missing receipt hash field `self_hash_hex`, `receipt_hash_hex`, or `receipt_hash`")]
    MissingReceiptHash,
    #[error("receipt hash must decode to exactly 32 bytes")]
    InvalidReceiptHash,
}

fn optional_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Option<String> {
    obj.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn receipt_hash_hex(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    for key in ["self_hash_hex", "receipt_hash_hex", "receipt_hash"] {
        let Some(raw) = optional_string(obj, key) else {
            continue;
        };
        let raw = raw.strip_prefix("sha256:").unwrap_or(&raw);
        let normalized = raw.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    None
}

fn validate_hash_hex(raw: &str) -> Result<(), ReceiptError> {
    let decoded = hex::decode(raw).map_err(|_| ReceiptError::InvalidReceiptHash)?;
    if decoded.len() != 32 {
        return Err(ReceiptError::InvalidReceiptHash);
    }
    Ok(())
}

fn receipt_metadata(
    obj: &serde_json::Map<String, serde_json::Value>,
    verified_input: bool,
    receipt_hash_hex: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut metadata = obj
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    for key in [
        "index",
        "receipt_type",
        "timestamp_unix_ms",
        "prev_receipt_hash_hex",
        "payload_content_type",
        "cleanup_complete",
        "reason",
        "child_invocation_id",
    ] {
        if let Some(value) = obj.get(key) {
            metadata.insert(key.to_string(), value.clone());
        }
    }
    if let Some(hash) = receipt_hash_hex {
        metadata.insert(
            "receipt_hash_hex".to_string(),
            serde_json::Value::String(hash.to_string()),
        );
    }
    if verified_input {
        metadata.insert(
            "verification_claim_downgraded".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
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

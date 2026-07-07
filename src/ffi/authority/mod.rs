// EasyNet CLI — Authority C ABI projection
// =========================================
//
// File: src/ffi/authority/mod.rs
// Description: C ABI projection for authority signing material and metadata
//              materialization.
//
// Protocol Responsibility
// -----------------------
// Expose daemon admission authority metadata preparation through
// `libeasynet_cli` without giving language SDKs ownership of canonical payload
// construction or wire metadata shape.
//
// Implementation Approach
// -----------------------
// Keep pointer/UTF-8/output allocation at the ABI edge. Delegate all authority
// payload validation, canonical signing material, and metadata materialization
// to `daemon::invocation::admission::authority_metadata`.
//
// Usage Contract
// --------------
// Prepare functions accept request JSON and return signing material JSON.
// Materialize functions accept the same request JSON plus signature JSON and
// return metadata projection JSON. Returned strings are caller-owned and must
// be freed through `easynet_string_free`.
//
// Architectural Position
// ----------------------
// This is the concrete C ABI core for Go/Python authority transports. It is not
// an Axon SDK binding and it never accepts private key material.

use std::os::raw::c_char;

use serde_json::Value;

use crate::daemon::invocation::admission::authority_metadata::{
    materialize_delegation_from_json, materialize_session_authority_from_json,
    prepare_delegation_from_json, prepare_session_authority_from_json, AuthorityMetadataError,
};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

type AuthorityFn = fn(&str) -> Result<Value, AuthorityMetadataError>;
type AuthorityMaterializeFn = fn(&str, &str) -> Result<Value, AuthorityMetadataError>;

/// Prepare canonical signing material for delegated authority metadata.
///
/// # Safety
/// `request_json` must be valid UTF-8 and `out_material_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_authority_prepare_delegation(
    request_json: *const c_char,
    out_material_json: *mut *mut c_char,
) -> i32 {
    prepare_authority_output(
        "easynet_authority_prepare_delegation",
        request_json,
        out_material_json,
        prepare_delegation_from_json,
    )
}

/// Materialize delegated authority metadata from request JSON and a signature.
///
/// `signature_json` accepts `{"signature_base64":"..."}`. The returned JSON
/// carries `metadata_value` and a single-entry `metadata` object keyed by
/// `x-easynet-delegation`.
///
/// # Safety
/// Pointers must be valid UTF-8 C strings and `out_metadata_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_authority_materialize_delegation(
    request_json: *const c_char,
    signature_json: *const c_char,
    out_metadata_json: *mut *mut c_char,
) -> i32 {
    materialize_authority_output(
        "easynet_authority_materialize_delegation",
        request_json,
        signature_json,
        out_metadata_json,
        materialize_delegation_from_json,
    )
}

/// Prepare canonical signing material for session authority metadata.
///
/// # Safety
/// `request_json` must be valid UTF-8 and `out_material_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_authority_prepare_session(
    request_json: *const c_char,
    out_material_json: *mut *mut c_char,
) -> i32 {
    prepare_authority_output(
        "easynet_authority_prepare_session",
        request_json,
        out_material_json,
        prepare_session_authority_from_json,
    )
}

/// Materialize session authority metadata from request JSON and a signature.
///
/// `signature_json` accepts `{"signature_base64":"..."}`. The returned JSON
/// carries `metadata_value` and a single-entry `metadata` object keyed by
/// `x-easynet-session-authority`.
///
/// # Safety
/// Pointers must be valid UTF-8 C strings and `out_metadata_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_authority_materialize_session(
    request_json: *const c_char,
    signature_json: *const c_char,
    out_metadata_json: *mut *mut c_char,
) -> i32 {
    materialize_authority_output(
        "easynet_authority_materialize_session",
        request_json,
        signature_json,
        out_metadata_json,
        materialize_session_authority_from_json,
    )
}

fn prepare_authority_output(
    fn_name: &'static str,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    prepare: AuthorityFn,
) -> i32 {
    if out_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            format!("{fn_name}: out_json pointer is null"),
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_json = std::ptr::null_mut() };

    let request = match read_authority_cstr(fn_name, "request_json", request_json) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let output = match prepare(request) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("{fn_name}: {err}"));
            return ERR_INVALID_ARG;
        }
    };
    write_authority_json(fn_name, out_json, output)
}

fn materialize_authority_output(
    fn_name: &'static str,
    request_json: *const c_char,
    signature_json: *const c_char,
    out_json: *mut *mut c_char,
    materialize: AuthorityMaterializeFn,
) -> i32 {
    if out_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            format!("{fn_name}: out_json pointer is null"),
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_json = std::ptr::null_mut() };

    let request = match read_authority_cstr(fn_name, "request_json", request_json) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let signature = match read_authority_cstr(fn_name, "signature_json", signature_json) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let output = match materialize(request, signature) {
        Ok(value) => value,
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("{fn_name}: {err}"));
            return ERR_INVALID_ARG;
        }
    };
    write_authority_json(fn_name, out_json, output)
}

fn read_authority_cstr<'a>(
    fn_name: &'static str,
    arg_name: &'static str,
    ptr: *const c_char,
) -> Result<&'a str, i32> {
    match read_cstr(ptr) {
        Ok(value) => Ok(value),
        Err(StringError::Null) => {
            set_last_error_code(ERR_NULL_POINTER, format!("{fn_name}: {arg_name} is null"));
            Err(ERR_NULL_POINTER)
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                format!("{fn_name}: {arg_name} is not valid UTF-8"),
            );
            Err(ERR_INVALID_UTF8)
        }
    }
}

fn write_authority_json(fn_name: &'static str, out_json: *mut *mut c_char, value: Value) -> i32 {
    let encoded = match serde_json::to_string(&value) {
        Ok(encoded) => encoded,
        Err(err) => {
            set_last_error_code(
                ERR_GENERIC,
                format!("{fn_name}: authority output JSON marshal failed: {err}"),
            );
            return ERR_GENERIC;
        }
    };
    let ptr = alloc_output_cstring(encoded);
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{fn_name}: out-of-memory allocating JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *out_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};

    use serde_json::Value;

    use super::*;
    use crate::ffi::strings::easynet_string_free;

    const DELEGATION_REQUEST: &str = r#"{
      "issuer_ura":"easynet:///r/example/user/alice",
      "subject_ura":"easynet:///r/example/user/alice",
      "caller_ura":"easynet:///r/example/agent/backend",
      "audience":"easynet:///r/example/device/dev-a",
      "scopes":["device.observe.*"],
      "issued_at_ms":1000,
      "expires_at_ms":2000
    }"#;

    #[test]
    fn prepare_delegation_projects_signing_material() {
        let request = CString::new(DELEGATION_REQUEST).unwrap();
        let mut out = std::ptr::null_mut();
        let rc = unsafe { easynet_authority_prepare_delegation(request.as_ptr(), &mut out) };
        assert_eq!(rc, EASYNET_OK);
        let value = read_output_json(out);
        assert_eq!(value["profile"], "authority");
        assert_eq!(value["kind"], "delegation");
        assert_eq!(value["metadata_key"], "x-easynet-delegation");
    }

    #[test]
    fn materialize_delegation_projects_metadata_map() {
        let request = CString::new(DELEGATION_REQUEST).unwrap();
        let signature =
            CString::new(r#"{"signature_base64":"ZGVsZWdhdGlvbi1zaWduYXR1cmU="}"#).unwrap();
        let mut out = std::ptr::null_mut();
        let rc = unsafe {
            easynet_authority_materialize_delegation(request.as_ptr(), signature.as_ptr(), &mut out)
        };
        assert_eq!(rc, EASYNET_OK);
        let value = read_output_json(out);
        assert_eq!(value["profile"], "authority");
        assert!(
            value["metadata"]["x-easynet-delegation"]
                .as_str()
                .unwrap()
                .len()
                > 20
        );
    }

    #[test]
    fn prepare_delegation_rejects_invalid_json() {
        let request = CString::new("{}").unwrap();
        let mut out = std::ptr::null_mut();
        let rc = unsafe { easynet_authority_prepare_delegation(request.as_ptr(), &mut out) };
        assert_eq!(rc, ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn prepare_delegation_rejects_null_output_pointer() {
        let request = CString::new(DELEGATION_REQUEST).unwrap();
        let rc =
            unsafe { easynet_authority_prepare_delegation(request.as_ptr(), std::ptr::null_mut()) };
        assert_eq!(rc, ERR_NULL_POINTER);
    }

    fn read_output_json(ptr: *mut c_char) -> Value {
        assert!(!ptr.is_null());
        let raw = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { easynet_string_free(ptr) };
        serde_json::from_str(&raw).unwrap()
    }
}

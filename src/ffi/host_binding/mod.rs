// EasyNet CLI — Host Binding C ABI projection
// ============================================
//
// File: src/ffi/host_binding/mod.rs
// Description: C ABI HostBindingClient projection helpers for daemon SDK
//              host-stream binding and frame/hash DTOs.
//
// Protocol Responsibility
// -----------------------
// Expose the daemon-owned host-stream request, frame, terminal, and output-hash
// contract to language bindings. This module does not execute product host
// code, load Python functions, inspect decorators, or manage warm host threads.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, and JSON validation at the C ABI boundary. Delegate
// host-stream request/frame/hash semantics to `protocol::host_stream_contract`
// and descriptor-ref grammar to Axon helpers.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// This is the SDK Host Binding profile contract. Product hosts own process
// warmth and function/class introspection; EasyNet-Cli owns the canonical
// host-stream frame semantics and terminal-state mapping.

use std::fmt;
use std::os::raw::c_char;
use std::path::{Component, Path};

use easynet_axon::invocation::canonical_ability_descriptor_ref;
use serde_json::{json, Map, Value};

use crate::ffi::client::handle::{get, EasynetHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};
use crate::protocol::host_stream_contract::{
    canonical_value_json, decode_host_stream_request, hash_state_from_json, sdk_error_frame,
    sdk_item_frame, sdk_terminal_frame, HOST_STREAM_FRAME_SCHEMA, HOST_STREAM_HASH_ALGORITHM,
};

/// Build a schema-backed host-stream binding DTO.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_binding_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_host_binding_build(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_binding_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_host_binding_args(
        handle,
        request_json,
        out_binding_json,
        "easynet_host_binding_build",
        "out_binding_json",
        "request_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let obj = match parse_json_object(raw, "easynet_host_binding_build", "request_json") {
        Ok(obj) => obj,
        Err(code) => return code,
    };
    match build_binding_json(&obj) {
        Ok(value) => write_json_output("easynet_host_binding_build", out_binding_json, value),
        Err(err) => fail_invalid_arg("easynet_host_binding_build", err),
    }
}

/// Decode the daemon-to-host request envelope into the shared request DTO.
///
/// # Safety
/// `envelope_json` must be a valid UTF-8 C string and `out_request_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_host_binding_decode_request(
    handle: EasynetHandle,
    envelope_json: *const c_char,
    out_request_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_host_binding_args(
        handle,
        envelope_json,
        out_request_json,
        "easynet_host_binding_decode_request",
        "out_request_json",
        "envelope_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let envelope =
        match parse_json_value(raw, "easynet_host_binding_decode_request", "envelope_json") {
            Ok(value) => value,
            Err(code) => return code,
        };
    match decode_host_stream_request(&envelope) {
        Ok(value) => write_json_output(
            "easynet_host_binding_decode_request",
            out_request_json,
            value,
        ),
        Err(err) => fail_invalid_arg(
            "easynet_host_binding_decode_request",
            HostBindingError::Contract(err.message),
        ),
    }
}

/// Encode a schema-shaped host-stream item frame.
///
/// # Safety
/// `item_json` must be a valid UTF-8 C string and `out_frame_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_host_binding_encode_item(
    handle: EasynetHandle,
    item_json: *const c_char,
    out_frame_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_host_binding_args(
        handle,
        item_json,
        out_frame_json,
        "easynet_host_binding_encode_item",
        "out_frame_json",
        "item_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let obj = match parse_json_object(raw, "easynet_host_binding_encode_item", "item_json") {
        Ok(obj) => obj,
        Err(code) => return code,
    };
    match encode_item_json(&obj) {
        Ok(value) => write_json_output("easynet_host_binding_encode_item", out_frame_json, value),
        Err(err) => fail_invalid_arg("easynet_host_binding_encode_item", err),
    }
}

/// Encode a schema-shaped host-stream error frame.
///
/// # Safety
/// `error_json` must be a valid UTF-8 C string and `out_frame_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_host_binding_encode_error(
    handle: EasynetHandle,
    error_json: *const c_char,
    out_frame_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_host_binding_args(
        handle,
        error_json,
        out_frame_json,
        "easynet_host_binding_encode_error",
        "out_frame_json",
        "error_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let error = match parse_json_value(raw, "easynet_host_binding_encode_error", "error_json") {
        Ok(value) => value,
        Err(code) => return code,
    };
    if !error.is_object() {
        return fail_invalid_arg(
            "easynet_host_binding_encode_error",
            HostBindingError::InvalidField("error_json", "must be an object".to_string()),
        );
    }
    if let Err(err) = validate_error_dto(&error) {
        return fail_invalid_arg("easynet_host_binding_encode_error", err);
    }
    write_json_output(
        "easynet_host_binding_encode_error",
        out_frame_json,
        sdk_error_frame(error),
    )
}

/// Encode a schema-shaped host-stream terminal frame from a terminal summary.
///
/// # Safety
/// `terminal_json` must be a valid UTF-8 C string and `out_frame_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_host_binding_encode_terminal(
    handle: EasynetHandle,
    terminal_json: *const c_char,
    out_frame_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_host_binding_args(
        handle,
        terminal_json,
        out_frame_json,
        "easynet_host_binding_encode_terminal",
        "out_frame_json",
        "terminal_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let terminal =
        match parse_json_value(raw, "easynet_host_binding_encode_terminal", "terminal_json") {
            Ok(value) => value,
            Err(code) => return code,
        };
    match sdk_terminal_frame(terminal) {
        Ok(value) => write_json_output(
            "easynet_host_binding_encode_terminal",
            out_frame_json,
            value,
        ),
        Err(err) => fail_invalid_arg(
            "easynet_host_binding_encode_terminal",
            HostBindingError::Contract(err.message),
        ),
    }
}

/// Fold one item value into the schema-backed host-stream output hash state.
///
/// # Safety
/// `fold_json` must be a valid UTF-8 C string and `out_state_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_host_binding_fold_output_hash(
    handle: EasynetHandle,
    fold_json: *const c_char,
    out_state_json: *mut *mut c_char,
) -> i32 {
    let raw = match read_host_binding_args(
        handle,
        fold_json,
        out_state_json,
        "easynet_host_binding_fold_output_hash",
        "out_state_json",
        "fold_json",
    ) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let obj = match parse_json_object(raw, "easynet_host_binding_fold_output_hash", "fold_json") {
        Ok(obj) => obj,
        Err(code) => return code,
    };
    match fold_output_hash_json(&obj) {
        Ok(value) => write_json_output(
            "easynet_host_binding_fold_output_hash",
            out_state_json,
            value,
        ),
        Err(err) => fail_invalid_arg("easynet_host_binding_fold_output_hash", err),
    }
}

fn read_host_binding_args<'a>(
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

fn parse_json_object(
    raw: &str,
    function: &'static str,
    input_name: &'static str,
) -> Result<Map<String, Value>, i32> {
    match parse_json_value(raw, function, input_name)? {
        Value::Object(obj) => Ok(obj),
        _ => {
            set_last_error_code(
                ERR_INVALID_ARG,
                format!("{function}: {input_name} must be an object"),
            );
            Err(ERR_INVALID_ARG)
        }
    }
}

fn write_json_output(function: &'static str, output: *mut *mut c_char, value: Value) -> i32 {
    let ptr = alloc_output_cstring(value.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating host binding JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *output = ptr };
    clear_last_error();
    EASYNET_OK
}

fn fail_invalid_arg(function: &'static str, err: HostBindingError) -> i32 {
    set_last_error_code(ERR_INVALID_ARG, format!("{function}: {err}"));
    ERR_INVALID_ARG
}

fn build_binding_json(obj: &Map<String, Value>) -> Result<Value, HostBindingError> {
    let binding_id = required_string(obj, "binding_id")?;
    let descriptor_ref = required_string(obj, "descriptor_ref")?;
    let descriptor_ref = canonical_ability_descriptor_ref(descriptor_ref)
        .map_err(|err| HostBindingError::InvalidField("descriptor_ref", err.to_string()))?;
    let endpoint = validate_endpoint(required_string(obj, "endpoint")?)?;
    let frame_schema = required_string(obj, "frame_schema")?;
    if frame_schema != HOST_STREAM_FRAME_SCHEMA {
        return Err(HostBindingError::InvalidField(
            "frame_schema",
            format!("must be {HOST_STREAM_FRAME_SCHEMA:?}"),
        ));
    }
    let cleanup = typed_object_or_default(obj, "cleanup", json!({"mode": "none"}))?;
    let readiness = typed_object_or_default(
        obj,
        "readiness",
        json!({
            "state": "declared",
            "checked": false,
            "endpoint_ready": null,
        }),
    )?;
    let timeout_ms = optional_u64_or_null(obj, "timeout_ms")?;
    let request_metadata = typed_object_or_default(obj, "metadata", json!({}))?;

    Ok(json!({
        "binding_id": binding_id,
        "descriptor_ref": descriptor_ref,
        "endpoint": endpoint,
        "frame_schema": frame_schema,
        "cleanup": cleanup,
        "timeout_ms": timeout_ms,
        "readiness": readiness,
        "lifecycle": {
            "endpoint_owner": "product_host",
            "process_owner": "product_host",
            "frame_contract_owner": "daemon_sdk",
        },
        "metadata": {
            "profile": "host_binding",
            "source": "easynet_host_binding_build",
            "frame_schema": HOST_STREAM_FRAME_SCHEMA,
            "hash_algorithm": HOST_STREAM_HASH_ALGORITHM,
            "request": request_metadata,
        },
    }))
}

fn encode_item_json(obj: &Map<String, Value>) -> Result<Value, HostBindingError> {
    let seq = required_u64(obj, "seq")?;
    let value = obj
        .get("value")
        .ok_or(HostBindingError::MissingField("value"))?;
    Ok(sdk_item_frame(seq, value.clone()))
}

fn fold_output_hash_json(obj: &Map<String, Value>) -> Result<Value, HostBindingError> {
    let seq = required_u64(obj, "seq")?;
    let value = obj
        .get("value")
        .ok_or(HostBindingError::MissingField("value"))?;
    let state_value = obj
        .get("state")
        .ok_or(HostBindingError::MissingField("state"))?;
    let mut state =
        hash_state_from_json(state_value).map_err(|err| HostBindingError::Contract(err.message))?;
    let canonical_json = canonical_value_json(value);
    state
        .fold_item(seq, value)
        .map_err(|err| HostBindingError::Contract(err.message))?;
    let mut state_json = state.to_json();
    state_json["canonical_json"] = Value::String(canonical_json);
    Ok(state_json)
}

fn validate_error_dto(error: &Value) -> Result<(), HostBindingError> {
    let obj = error.as_object().ok_or_else(|| {
        HostBindingError::InvalidField("error_json", "must be an object".to_string())
    })?;
    required_string(obj, "code")?;
    required_string(obj, "stage")?;
    let message = obj
        .get("message")
        .and_then(Value::as_str)
        .ok_or(HostBindingError::MissingField("message"))?;
    if message.contains('\0') {
        return Err(HostBindingError::InvalidField(
            "message",
            "must not contain NUL".to_string(),
        ));
    }
    let retry = required_string(obj, "retry")?;
    if !matches!(retry, "never" | "safe" | "after_backoff" | "unknown") {
        return Err(HostBindingError::InvalidField(
            "retry",
            "must be never, safe, after_backoff, or unknown".to_string(),
        ));
    }
    Ok(())
}

fn required_string<'a>(
    obj: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, HostBindingError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(HostBindingError::MissingField(key))
}

fn required_u64(obj: &Map<String, Value>, key: &'static str) -> Result<u64, HostBindingError> {
    obj.get(key)
        .and_then(Value::as_u64)
        .ok_or(HostBindingError::MissingField(key))
}

fn optional_u64_or_null(
    obj: &Map<String, Value>,
    key: &'static str,
) -> Result<Value, HostBindingError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(value) => value.as_u64().map(|value| json!(value)).ok_or_else(|| {
            HostBindingError::InvalidField(key, "must be a u64 or null".to_string())
        }),
    }
}

fn typed_object_or_default(
    obj: &Map<String, Value>,
    key: &'static str,
    default: Value,
) -> Result<Value, HostBindingError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err(HostBindingError::InvalidField(
            key,
            "must be an object or null".to_string(),
        )),
    }
}

fn validate_endpoint(endpoint: &str) -> Result<&str, HostBindingError> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(HostBindingError::MissingField("endpoint"));
    }
    let path = Path::new(endpoint);
    if !path.is_absolute() {
        return Err(HostBindingError::InvalidField(
            "endpoint",
            "must be an absolute Unix socket path".to_string(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(HostBindingError::InvalidField(
            "endpoint",
            "must not contain `..` components".to_string(),
        ));
    }
    Ok(endpoint)
}

#[derive(Debug)]
enum HostBindingError {
    MissingField(&'static str),
    InvalidField(&'static str, String),
    Contract(String),
}

impl fmt::Display for HostBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostBindingError::MissingField(field) => write!(f, "missing required field {field}"),
            HostBindingError::InvalidField(field, message) => {
                write!(f, "invalid field {field}: {message}")
            }
            HostBindingError::Contract(message) => f.write_str(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
    use crate::protocol::host_stream_contract::HostStreamHashState;
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

    #[test]
    fn host_binding_build_projects_typed_binding() {
        let handle = handle();
        let raw = CString::new(
            json!({
                "binding_id": "binding-1",
                "descriptor_ref": "easynet:///r/acme/ability/device.dev-1.fs.read@1.0.0",
                "endpoint": "/tmp/easynet-host.sock",
                "frame_schema": HOST_STREAM_FRAME_SCHEMA,
                "cleanup": {"mode": "unlink_socket"},
                "timeout_ms": 30_000,
                "metadata": {"owner": "easyremote"}
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_host_binding_build(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["binding_id"], "binding-1");
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/acme/ability/device.dev-1.fs.read@1.0.0"
        );
        assert_eq!(value["endpoint"], "/tmp/easynet-host.sock");
        assert_eq!(value["lifecycle"]["frame_contract_owner"], "daemon_sdk");
        assert_eq!(
            value["metadata"]["hash_algorithm"],
            HOST_STREAM_HASH_ALGORITHM
        );
        release(handle);
    }

    #[test]
    fn host_binding_build_rejects_relative_endpoint() {
        let handle = handle();
        let raw = CString::new(
            json!({
                "binding_id": "binding-1",
                "descriptor_ref": "easynet:///r/acme/ability/device.dev-1.fs.read@1.0.0",
                "endpoint": "tmp/easynet-host.sock",
                "frame_schema": HOST_STREAM_FRAME_SCHEMA
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_host_binding_build(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn decode_request_projects_current_daemon_envelope() {
        let handle = handle();
        let raw = CString::new(
            json!({
                "request": {
                    "fn": "weather.stream",
                    "args": {"city": "Singapore"},
                    "call_id": "call-1",
                    "caller": "easynet:///r/acme/user/alice"
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_host_binding_decode_request(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["function"], "weather.stream");
        assert_eq!(value["args"]["city"], "Singapore");
        assert_eq!(value["caller"], "easynet:///r/acme/user/alice");
        release(handle);
    }

    #[test]
    fn encode_item_builds_schema_frame() {
        let handle = handle();
        let raw = CString::new(json!({"seq": 0, "value": {"token": "hello"}}).to_string()).unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_host_binding_encode_item(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["frame_type"], "item");
        assert_eq!(value["seq"], 0);
        assert_eq!(value["value"]["token"], "hello");
        release(handle);
    }

    #[test]
    fn encode_error_builds_schema_frame_without_seq() {
        let handle = handle();
        let raw = CString::new(
            json!({
                "code": "InvalidArgument",
                "stage": "host",
                "message": "bad host argument",
                "retry": "never"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_host_binding_encode_error(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["frame_type"], "error");
        assert!(value["seq"].is_null());
        assert_eq!(value["error"]["code"], "InvalidArgument");
        release(handle);
    }

    #[test]
    fn encode_terminal_derives_seq_from_terminal_frames() {
        let handle = handle();
        let state = HostStreamHashState::new();
        let raw =
            CString::new(json!({"output_hash": state.output_hash(), "frames": 0}).to_string())
                .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_host_binding_encode_terminal(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["frame_type"], "terminal");
        assert_eq!(value["seq"], 0);
        assert_eq!(value["output_hash"], state.output_hash());
        release(handle);
    }

    #[test]
    fn fold_output_hash_requires_explicit_state_and_seq() {
        let handle = handle();
        let state = HostStreamHashState::new().to_json();
        let raw = CString::new(
            json!({
                "state": state,
                "seq": 0,
                "value": {"b": 2, "a": 1}
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_host_binding_fold_output_hash(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["frames"], 1);
        assert_eq!(value["last_seq"], 0);
        assert_eq!(value["canonical_json"], r#"{"a":1,"b":2}"#);
        release(handle);
    }

    #[test]
    fn encode_item_rejects_invalid_handle_after_zeroing_output() {
        let raw = CString::new(json!({"seq": 0, "value": {"token": "hello"}}).to_string()).unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe { easynet_host_binding_encode_item(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

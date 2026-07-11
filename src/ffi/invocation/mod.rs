// EasyNet CLI — Complete Invocation ABI
// ======================================
//
// File: src/ffi/invocation.rs
// Description: C ABI entry points for submitting complete Axon
//              Invocations through the local EasyNet daemon.
//
// Boundary
// --------
// This module is the language-binding facade for the daemon
// Invocation surface. It accepts a JSON representation of the Axon
// seven-tuple, validates that every load-bearing field is present,
// and then delegates envelope construction to `crate::daemon`.
//
// What this module is NOT
// -----------------------
// - It is not a secondary ability-call ABI. The clean FFI surface exports
//   complete Invocation carriers only.
// - It is not an Axon protocol implementation. Axon owns the proto
//   types and canonical semantics; this module only maps C strings
//   to the Rust daemon SDK.
// - It is not a JSON-control bridge. Unary, server-stream, and bidi
//   all go through the daemon's Axon Invocation gRPC endpoint.

use std::os::raw::{c_char, c_void};
#[cfg(feature = "axon-pb")]
use std::path::PathBuf;
#[cfg(feature = "axon-pb")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "axon-pb")]
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

#[cfg(feature = "axon-pb")]
use crate::ffi::client::handle::lib_runtime;
use crate::ffi::client::handle::{get, EasynetHandle};
#[cfg(not(feature = "axon-pb"))]
use crate::ffi::errors::ERR_NOT_IMPLEMENTED;
#[cfg(feature = "axon-pb")]
use crate::ffi::errors::{
    clear_last_error, EASYNET_OK, ERR_ABILITY_FAILED, ERR_CANCELLED, ERR_DAEMON_DOWN, ERR_GENERIC,
    ERR_INVALID_ARG, ERR_NOT_FOUND, ERR_NOT_IMPLEMENTED, ERR_PERMISSION_DENIED, ERR_PROTOCOL,
    ERR_TIMEOUT,
};
use crate::ffi::errors::{
    set_last_error_code, ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
#[cfg(feature = "axon-pb")]
use crate::ffi::strings::alloc_output_cstring;
use crate::ffi::strings::{read_cstr, StringError};
#[cfg(feature = "axon-pb")]
use crate::protocol::runtime_stream_contract::{
    bidi_callback_backpressure_frame, stream_callback_backpressure_event,
};

/// Opaque id for a server-stream opened through
/// `easynet_invocation_stream_open`.
///
/// A value of 0 means no stream was allocated. Stream ids are
/// process-local; they are not Axon protocol ids and must not be
/// serialized as receipt or invocation identifiers.
pub type InvocationStreamId = u64;

/// Opaque id for an InvokeBidi session opened through
/// `easynet_invocation_bidi_open`.
///
/// A value of 0 means no session was allocated. Bidi ids are
/// process-local handles, not protocol identifiers.
pub type InvocationBidiId = u64;

/// Opaque id for a mutable Invocation builder object.
pub type InvocationBuilderId = u64;

/// Opaque id for a prepared canonical signing-material object.
pub type PreparedInvocationId = u64;

/// Opaque id for a submit-ready signed Invocation object.
pub type SignedInvocationId = u64;

/// Opaque id for a submitted Invocation observer.
pub type InvocationHandleId = u64;

/// Callback invoked once per decoded `InvokeStreamChunk` summary, then
/// ONCE MORE with a null `chunk_json` to mark end-of-stream.
///
/// `chunk_json` is borrowed for the duration of the callback. A
/// binding that wants to retain the frame must copy it before the
/// callback returns.
///
/// **End-of-stream contract:** when the stream finishes (terminal frame
/// delivered, or the transport closed), the callback fires exactly one
/// final time with `chunk_json == null`. Bindings MUST treat a null
/// `chunk_json` as "no more frames", not as a data frame — it is the
/// only unambiguous EOF signal for a queue-backed consumer.
pub type InvocationStreamCallback =
    unsafe extern "C" fn(user_data: *mut c_void, chunk_json: *const c_char);

/// Callback invoked once per decoded `InvokeBidiDown` frame summary.
///
/// `frame_json` is borrowed for the duration of the callback. A
/// binding that wants to retain the frame must copy it before the
/// callback returns.
pub type InvocationBidiCallback =
    unsafe extern "C" fn(user_data: *mut c_void, frame_json: *const c_char);

#[cfg(feature = "axon-pb")]
const STREAM_CALLBACK_QUEUE_CAPACITY: usize = 64;
#[cfg(feature = "axon-pb")]
const BIDI_CALLBACK_QUEUE_CAPACITY: usize = 64;

fn record_invocation_error(code: i32, message: impl Into<String>) -> i32 {
    set_last_error_code(code, message);
    code
}

#[cfg(not(feature = "axon-pb"))]
fn record_invocation_feature_disabled(function: &str) -> i32 {
    record_invocation_error(
        ERR_NOT_IMPLEMENTED,
        format!("{function}: axon-pb feature is not enabled in this build"),
    )
}

/// Invoke a complete Axon Invocation through the local daemon.
///
/// `invocation_json` must be a UTF-8 JSON object with these fields:
///
/// ```text
/// {
///   "caller_ura": "...",
///   "callee_ura": "...",
///   "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0",
///   "subject_ura": "...",
///   "nonce_base64": "<16 bytes, base64>",
///   "causal_context": {"form": "none"},
///   "args": {...}
/// }
/// ```
///
/// `causal_context` uses the Axon JSON surface:
/// `none`, `scalar`, `list`, or `merkle`. For non-JSON payloads,
/// callers may pass `arguments_base64` and `content_type` instead
/// of `args`.
///
/// On success, `*out_receipt_json` receives a JSON response summary
/// containing the daemon result, content type, selected scheduling
/// metadata, the echoed nonce, and any admission receipt summary the
/// daemon returned. The caller frees it with `easynet_string_free`.
///
/// # Safety
/// - `handle` must be a valid handle from `easynet_init`.
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `out_receipt_json` must be a non-null pointer to a `*mut c_char`
///   owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_invoke(
    handle: EasynetHandle,
    invocation_json: *const c_char,
    out_receipt_json: *mut *mut c_char,
) -> i32 {
    if out_receipt_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_invoke: out_receipt_json pointer is null",
        );
    }
    unsafe { *out_receipt_json = std::ptr::null_mut() };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_invocation_invoke: handle {handle} is not registered"),
            );
        }
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "easynet_invocation_invoke: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_invoke: invocation_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw);
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "easynet_invocation_invoke: axon-pb feature is not enabled in this build",
        )
    }

    #[cfg(feature = "axon-pb")]
    {
        invoke_with_axon_pb(session, raw, out_receipt_json)
    }
}

/// Return typed runtime readiness for an Invocation-capable client
/// handle.
///
/// # Safety
/// `out_health_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_runtime_health(
    handle: EasynetHandle,
    out_health_json: *mut *mut c_char,
) -> i32 {
    if out_health_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_runtime_health: out_health_json pointer is null",
        );
    }
    unsafe { *out_health_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_runtime_health: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = session;
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "easynet_runtime_health: axon-pb feature is not enabled in this build",
        )
    }

    #[cfg(feature = "axon-pb")]
    {
        let json = runtime_health_json(session.as_ref()).to_string();
        let ptr = alloc_output_cstring(json);
        if ptr.is_null() {
            return record_invocation_error(
                ERR_GENERIC,
                "easynet_runtime_health: out-of-memory allocating health string",
            );
        }
        unsafe { *out_health_json = ptr };
        clear_last_error();
        EASYNET_OK
    }
}

/// Return typed runtime diagnostics for an Invocation-capable client
/// handle.
///
/// # Safety
/// `out_diagnostics_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_runtime_diagnostics(
    handle: EasynetHandle,
    out_diagnostics_json: *mut *mut c_char,
) -> i32 {
    if out_diagnostics_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_runtime_diagnostics: out_diagnostics_json pointer is null",
        );
    }
    unsafe { *out_diagnostics_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_runtime_diagnostics: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = session;
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "easynet_runtime_diagnostics: axon-pb feature is not enabled in this build",
        )
    }

    #[cfg(feature = "axon-pb")]
    {
        let json = runtime_diagnostics_json(session.as_ref()).to_string();
        let ptr = alloc_output_cstring(json);
        if ptr.is_null() {
            return record_invocation_error(
                ERR_GENERIC,
                "easynet_runtime_diagnostics: out-of-memory allocating diagnostics string",
            );
        }
        unsafe { *out_diagnostics_json = ptr };
        clear_last_error();
        EASYNET_OK
    }
}

/// Allocate a mutable Invocation builder handle.
///
/// The builder starts empty. Bindings must set the complete seven-tuple
/// fields before inspect/build/prepare.
///
/// # Safety
/// `out_builder_id` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_builder_new(
    out_builder_id: *mut InvocationBuilderId,
) -> i32 {
    if out_builder_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_builder_new: out_builder_id pointer is null",
        );
    }
    unsafe { *out_builder_id = 0 };

    #[cfg(not(feature = "axon-pb"))]
    {
        record_invocation_feature_disabled("easynet_invocation_builder_new")
    }

    #[cfg(feature = "axon-pb")]
    {
        let id = insert_builder(InvocationBuilderState::default());
        unsafe { *out_builder_id = id };
        clear_last_error();
        EASYNET_OK
    }
}

macro_rules! builder_string_setter {
    ($fn_name:ident, $arg_name:literal, $field:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(
            builder_id: InvocationBuilderId,
            value: *const c_char,
        ) -> i32 {
            #[cfg(not(feature = "axon-pb"))]
            {
                let _ = (builder_id, value);
                record_invocation_feature_disabled(stringify!($fn_name))
            }

            #[cfg(feature = "axon-pb")]
            {
                set_builder_string_field(builder_id, value, stringify!($fn_name), $arg_name, $field)
            }
        }
    };
}

builder_string_setter!(
    easynet_invocation_builder_set_caller,
    "caller_ura",
    InvocationBuilderStringField::Caller
);
builder_string_setter!(
    easynet_invocation_builder_set_callee,
    "callee_ura",
    InvocationBuilderStringField::Callee
);
builder_string_setter!(
    easynet_invocation_builder_set_descriptor_ref,
    "descriptor_ref",
    InvocationBuilderStringField::DescriptorRef
);
builder_string_setter!(
    easynet_invocation_builder_set_subject,
    "subject_ura",
    InvocationBuilderStringField::Subject
);
builder_string_setter!(
    easynet_invocation_builder_set_nonce_base64,
    "nonce_base64",
    InvocationBuilderStringField::NonceBase64
);
builder_string_setter!(
    easynet_invocation_builder_set_causal_context_json,
    "causal_context_json",
    InvocationBuilderStringField::CausalContextJson
);
builder_string_setter!(
    easynet_invocation_builder_set_args_json,
    "args_json",
    InvocationBuilderStringField::ArgsJson
);
builder_string_setter!(
    easynet_invocation_builder_set_metadata_json,
    "metadata_json",
    InvocationBuilderStringField::MetadataJson
);
builder_string_setter!(
    easynet_invocation_builder_set_idempotency_key,
    "idempotency_key",
    InvocationBuilderStringField::IdempotencyKey
);
builder_string_setter!(
    easynet_invocation_builder_set_caller_signature_json,
    "signature_json",
    InvocationBuilderStringField::CallerSignatureJson
);

/// Set raw non-JSON Invocation arguments on a builder.
///
/// # Safety
/// `arguments_base64` and `content_type` must be non-null valid UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_builder_set_arguments_base64(
    builder_id: InvocationBuilderId,
    arguments_base64: *const c_char,
    content_type: *const c_char,
) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (builder_id, arguments_base64, content_type);
        record_invocation_feature_disabled("easynet_invocation_builder_set_arguments_base64")
    }

    #[cfg(feature = "axon-pb")]
    {
        let arguments_base64 = match read_builder_arg(
            "easynet_invocation_builder_set_arguments_base64",
            "arguments_base64",
            arguments_base64,
        ) {
            Ok(value) => value,
            Err(code) => return code,
        };
        let content_type = match read_builder_arg(
            "easynet_invocation_builder_set_arguments_base64",
            "content_type",
            content_type,
        ) {
            Ok(value) => value,
            Err(code) => return code,
        };
        mutate_builder(
            builder_id,
            "easynet_invocation_builder_set_arguments_base64",
            |builder| builder.set_arguments_base64(arguments_base64, content_type),
        )
    }
}

/// Set per-call timeout in seconds on a builder.
#[no_mangle]
pub extern "C" fn easynet_invocation_builder_set_timeout_seconds(
    builder_id: InvocationBuilderId,
    timeout_seconds: u32,
) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (builder_id, timeout_seconds);
        record_invocation_feature_disabled("easynet_invocation_builder_set_timeout_seconds")
    }

    #[cfg(feature = "axon-pb")]
    {
        mutate_builder(
            builder_id,
            "easynet_invocation_builder_set_timeout_seconds",
            |builder| builder.set_timeout_seconds(timeout_seconds),
        )
    }
}

/// Inspect a complete immutable Invocation draft without consuming the builder.
///
/// # Safety
/// `out_invocation_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_builder_inspect(
    builder_id: InvocationBuilderId,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    builder_output_invocation_json(
        builder_id,
        out_invocation_json,
        false,
        "easynet_invocation_builder_inspect",
    )
}

/// Build a complete Invocation JSON draft and consume the builder on success.
///
/// # Safety
/// `out_invocation_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_builder_build(
    builder_id: InvocationBuilderId,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    builder_output_invocation_json(
        builder_id,
        out_invocation_json,
        true,
        "easynet_invocation_builder_build",
    )
}

/// Prepare a builder into canonical signing material and consume the builder
/// on success.
///
/// # Safety
/// - `options_json` may be null; if non-null it must be valid UTF-8 JSON.
/// - output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_builder_prepare(
    handle: EasynetHandle,
    builder_id: InvocationBuilderId,
    options_json: *const c_char,
    out_prepared_id: *mut PreparedInvocationId,
    out_prepared_json: *mut *mut c_char,
) -> i32 {
    if out_prepared_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_builder_prepare: out_prepared_id pointer is null",
        );
    }
    if out_prepared_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_builder_prepare: out_prepared_json pointer is null",
        );
    }
    unsafe {
        *out_prepared_id = 0;
        *out_prepared_json = std::ptr::null_mut();
    }
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_builder_prepare: handle {handle} is not registered"),
        );
    }
    let options_raw = match read_optional_cstr(options_json) {
        Ok(value) => value,
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_builder_prepare: options_json is not valid UTF-8",
            );
        }
        Err(StringError::Null) => None,
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (builder_id, options_raw);
        record_invocation_feature_disabled("easynet_invocation_builder_prepare")
    }

    #[cfg(feature = "axon-pb")]
    {
        prepare_builder_with_axon_pb(builder_id, options_raw, out_prepared_id, out_prepared_json)
    }
}

/// Free a mutable builder handle.
#[no_mangle]
pub extern "C" fn easynet_invocation_builder_free(builder_id: InvocationBuilderId) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = builder_id;
        record_invocation_feature_disabled("easynet_invocation_builder_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        remove_builder(builder_id);
        clear_last_error();
        EASYNET_OK
    }
}

/// Prepare an immutable complete Invocation draft into canonical
/// signing material.
///
/// # Safety
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `options_json` may be null; if non-null it must be valid UTF-8 JSON.
/// - output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_prepare(
    handle: EasynetHandle,
    invocation_json: *const c_char,
    options_json: *const c_char,
    out_prepared_id: *mut PreparedInvocationId,
    out_prepared_json: *mut *mut c_char,
) -> i32 {
    if out_prepared_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_prepare: out_prepared_id pointer is null",
        );
    }
    if out_prepared_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_prepare: out_prepared_json pointer is null",
        );
    }
    unsafe {
        *out_prepared_id = 0;
        *out_prepared_json = std::ptr::null_mut();
    }
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_prepare: handle {handle} is not registered"),
        );
    }
    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "easynet_invocation_prepare: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_prepare: invocation_json is not valid UTF-8",
            );
        }
    };
    let options_raw = match read_optional_cstr(options_json) {
        Ok(value) => value,
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_prepare: options_json is not valid UTF-8",
            );
        }
        Err(StringError::Null) => None,
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (raw, options_raw);
        record_invocation_feature_disabled("easynet_invocation_prepare")
    }

    #[cfg(feature = "axon-pb")]
    {
        prepare_with_axon_pb(raw, options_raw, out_prepared_id, out_prepared_json)
    }
}

/// Attach caller signature material to a prepared Invocation.
///
/// # Safety
/// - `signature_json` must be a valid UTF-8 C string.
/// - output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_sign_prepared(
    prepared_id: PreparedInvocationId,
    signature_json: *const c_char,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
) -> i32 {
    if out_signed_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_sign_prepared: out_signed_id pointer is null",
        );
    }
    if out_signed_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_sign_prepared: out_signed_json pointer is null",
        );
    }
    unsafe {
        *out_signed_id = 0;
        *out_signed_json = std::ptr::null_mut();
    }
    let raw = match read_cstr(signature_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "easynet_invocation_sign_prepared: signature_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_sign_prepared: signature_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (prepared_id, raw);
        record_invocation_feature_disabled("easynet_invocation_sign_prepared")
    }

    #[cfg(feature = "axon-pb")]
    {
        sign_prepared_with_axon_pb(prepared_id, raw, out_signed_id, out_signed_json)
    }
}

/// Locally sign a prepared Invocation with the daemon SDK keyring.
///
/// # Safety
/// output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_sign_prepared_local(
    prepared_id: PreparedInvocationId,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
) -> i32 {
    if out_signed_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_sign_prepared_local: out_signed_id pointer is null",
        );
    }
    if out_signed_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_sign_prepared_local: out_signed_json pointer is null",
        );
    }
    unsafe {
        *out_signed_id = 0;
        *out_signed_json = std::ptr::null_mut();
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = prepared_id;
        record_invocation_feature_disabled("easynet_invocation_sign_prepared_local")
    }

    #[cfg(feature = "axon-pb")]
    {
        sign_prepared_local_with_axon_pb(prepared_id, out_signed_id, out_signed_json)
    }
}

/// Submit a signed Invocation through the daemon runtime endpoint.
///
/// # Safety
/// `out_result_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_submit_signed(
    handle: EasynetHandle,
    signed_id: SignedInvocationId,
    out_result_json: *mut *mut c_char,
) -> i32 {
    if out_result_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_submit_signed: out_result_json pointer is null",
        );
    }
    unsafe { *out_result_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_invocation_submit_signed: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, signed_id);
        record_invocation_feature_disabled("easynet_invocation_submit_signed")
    }

    #[cfg(feature = "axon-pb")]
    {
        submit_signed_sync_with_axon_pb(handle, session, signed_id, out_result_json)
    }
}

/// Submit a signed Invocation and return an observer handle.
///
/// # Safety
/// Output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_submit_signed_handle(
    handle: EasynetHandle,
    signed_id: SignedInvocationId,
    out_invocation_handle_id: *mut InvocationHandleId,
    out_submitted_json: *mut *mut c_char,
) -> i32 {
    if out_invocation_handle_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_submit_signed_handle: out_invocation_handle_id pointer is null",
        );
    }
    if out_submitted_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_submit_signed_handle: out_submitted_json pointer is null",
        );
    }
    unsafe {
        *out_invocation_handle_id = 0;
        *out_submitted_json = std::ptr::null_mut();
    }
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_submit_signed_handle: handle {handle} is not registered"
                ),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, signed_id);
        record_invocation_feature_disabled("easynet_invocation_submit_signed_handle")
    }

    #[cfg(feature = "axon-pb")]
    {
        submit_signed_handle_with_axon_pb(
            handle,
            session,
            signed_id,
            out_invocation_handle_id,
            out_submitted_json,
        )
    }
}

/// Await a submitted Invocation handle until it reaches a terminal state.
///
/// # Safety
/// `out_result_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_handle_await(
    handle: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
    out_result_json: *mut *mut c_char,
) -> i32 {
    if out_result_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_handle_await: out_result_json pointer is null",
        );
    }
    unsafe { *out_result_json = std::ptr::null_mut() };
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_handle_await: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = invocation_handle_id;
        record_invocation_feature_disabled("easynet_invocation_handle_await")
    }

    #[cfg(feature = "axon-pb")]
    {
        invocation_handle_await_with_axon_pb(handle, invocation_handle_id, out_result_json)
    }
}

/// Cancel a submitted Invocation handle if it has not already reached terminal.
///
/// # Safety
/// `reason_json` may be null; `out_cancel_json` must be a non-null caller-owned
/// pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_handle_cancel(
    handle: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
    reason_json: *const c_char,
    out_cancel_json: *mut *mut c_char,
) -> i32 {
    if out_cancel_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_handle_cancel: out_cancel_json pointer is null",
        );
    }
    unsafe { *out_cancel_json = std::ptr::null_mut() };
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_handle_cancel: handle {handle} is not registered"),
        );
    }
    let reason_raw = match read_optional_cstr(reason_json) {
        Ok(value) => value,
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_handle_cancel: reason_json is not valid UTF-8",
            );
        }
        Err(StringError::Null) => None,
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (invocation_handle_id, reason_raw);
        record_invocation_feature_disabled("easynet_invocation_handle_cancel")
    }

    #[cfg(feature = "axon-pb")]
    {
        invocation_handle_cancel_with_axon_pb(
            handle,
            invocation_handle_id,
            reason_raw,
            out_cancel_json,
        )
    }
}

/// Return an ordered event snapshot for a submitted Invocation handle.
///
/// # Safety
/// `out_events_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_handle_events(
    handle: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
    out_events_json: *mut *mut c_char,
) -> i32 {
    if out_events_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_handle_events: out_events_json pointer is null",
        );
    }
    unsafe { *out_events_json = std::ptr::null_mut() };
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_handle_events: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = invocation_handle_id;
        record_invocation_feature_disabled("easynet_invocation_handle_events")
    }

    #[cfg(feature = "axon-pb")]
    {
        invocation_handle_events_with_axon_pb(handle, invocation_handle_id, out_events_json)
    }
}

/// Free a submitted Invocation handle.
#[no_mangle]
pub extern "C" fn easynet_invocation_handle_free(
    handle: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_handle_free: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = invocation_handle_id;
        record_invocation_feature_disabled("easynet_invocation_handle_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        match remove_invocation_handle_for_owner(handle, invocation_handle_id) {
            Ok(_) => {
                clear_last_error();
                EASYNET_OK
            }
            Err(RegistryOwnerMismatch) => record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_free: invocation handle {invocation_handle_id} does not belong to handle {handle}"
                ),
            ),
        }
    }
}

/// Free a prepared Invocation handle.
#[no_mangle]
pub extern "C" fn easynet_prepared_invocation_free(prepared_id: PreparedInvocationId) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = prepared_id;
        record_invocation_feature_disabled("easynet_prepared_invocation_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        remove_prepared(prepared_id);
        clear_last_error();
        EASYNET_OK
    }
}

/// Free a signed Invocation handle.
#[no_mangle]
pub extern "C" fn easynet_signed_invocation_free(signed_id: SignedInvocationId) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = signed_id;
        record_invocation_feature_disabled("easynet_signed_invocation_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        remove_signed(signed_id);
        clear_last_error();
        EASYNET_OK
    }
}

/// Open a complete Axon server-stream Invocation through the local
/// daemon.
///
/// `invocation_json` has the same shape as `easynet_invocation_invoke`.
/// Each daemon `InvokeStreamChunk` is delivered to `on_chunk` as a
/// JSON summary. The returned `stream_id` may be passed to
/// `easynet_invocation_stream_cancel`; cancellation drops the local
/// gRPC stream and lets the daemon observe normal transport
/// cancellation. No JSON-control `Cancel` frame is emitted.
///
/// # Safety
/// - `handle` must be a valid handle from `easynet_init`.
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `on_chunk` must be a valid function pointer for the lifetime of
///   the stream.
/// - `out_stream_id` must be a non-null pointer owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_stream_open(
    handle: EasynetHandle,
    invocation_json: *const c_char,
    on_chunk: Option<InvocationStreamCallback>,
    user_data: *mut c_void,
    out_stream_id: *mut InvocationStreamId,
) -> i32 {
    if out_stream_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_stream_open: out_stream_id pointer is null",
        );
    }
    unsafe { *out_stream_id = 0 };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_invocation_stream_open: handle {handle} is not registered"),
            );
        }
    };

    let Some(on_chunk) = on_chunk else {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_stream_open: on_chunk callback is null",
        );
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "easynet_invocation_stream_open: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_stream_open: invocation_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw, on_chunk, user_data);
        record_invocation_feature_disabled("easynet_invocation_stream_open")
    }

    #[cfg(feature = "axon-pb")]
    {
        stream_open_with_axon_pb(handle, session, raw, on_chunk, user_data, out_stream_id)
    }
}

/// Cancel a stream opened by `easynet_invocation_stream_open`.
///
/// Unknown `stream_id` values are treated as already-closed streams
/// and return `EASYNET_OK`. A valid `handle` is still required so a
/// process cannot call this ABI before library initialization.
///
/// # Safety
/// `handle` may be any value; the function does not dereference
/// caller memory.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_stream_cancel(
    handle: EasynetHandle,
    stream_id: InvocationStreamId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_stream_cancel: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = stream_id;
        record_invocation_feature_disabled("easynet_invocation_stream_cancel")
    }

    #[cfg(feature = "axon-pb")]
    {
        release_stream_with_reader_cancel(handle, stream_id, "easynet_invocation_stream_cancel")
    }
}

/// Close and release a stream handle.
///
/// Unknown ids are treated as already closed and return `EASYNET_OK`.
/// This is a local resource close; daemon terminal frames are still
/// delivered through the callback path when available before close.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_stream_close(
    handle: EasynetHandle,
    stream_id: InvocationStreamId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_stream_close: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = stream_id;
        record_invocation_feature_disabled("easynet_invocation_stream_close")
    }

    #[cfg(feature = "axon-pb")]
    {
        stream_close_with_axon_pb(handle, stream_id)
    }
}

/// Open a complete Axon InvokeBidi session through the local daemon.
///
/// `invocation_json` uses the same complete seven-tuple shape as
/// `easynet_invocation_invoke`, with one additional required field:
///
/// ```text
/// "bidi_streams": [
///   {"stream_id": 1, "content_type": "application/json", "ordering": "STRICT"}
/// ]
/// ```
///
/// Optional `metadata` and `caller_signature` fields are forwarded
/// into `EnvelopeOpen` and `Envelope` respectively. Down-direction
/// frames are delivered to `on_frame` as JSON summaries. Up-direction
/// frames are sent with `easynet_invocation_bidi_send`.
///
/// # Safety
/// - `handle` must be a valid handle from `easynet_init`.
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `on_frame` must be a valid function pointer for the lifetime of
///   the session.
/// - `out_bidi_id` must be a non-null pointer owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_bidi_open(
    handle: EasynetHandle,
    invocation_json: *const c_char,
    on_frame: Option<InvocationBidiCallback>,
    user_data: *mut c_void,
    out_bidi_id: *mut InvocationBidiId,
) -> i32 {
    if out_bidi_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_bidi_open: out_bidi_id pointer is null",
        );
    }
    unsafe { *out_bidi_id = 0 };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_invocation_bidi_open: handle {handle} is not registered"),
            );
        }
    };

    let Some(on_frame) = on_frame else {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "easynet_invocation_bidi_open: on_frame callback is null",
        );
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "easynet_invocation_bidi_open: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_bidi_open: invocation_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw, on_frame, user_data);
        record_invocation_feature_disabled("easynet_invocation_bidi_open")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_open_with_axon_pb(handle, session, raw, on_frame, user_data, out_bidi_id)
    }
}

/// Send one up-direction frame on an active InvokeBidi session.
///
/// `frame_json` must be one of:
///
/// ```text
/// {"type":"binary_chunk","stream_id":1,"data_base64":"...","pts":0}
/// {"type":"control","eof":true}
/// {"type":"control","pty_resize":{"cols":120,"rows":40}}
/// {"type":"control","pty_signal":2}
/// ```
///
/// The ABI assigns the monotonic up-direction sequence number.
///
/// # Safety
/// `frame_json` must be a valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_bidi_send(
    handle: EasynetHandle,
    bidi_id: InvocationBidiId,
    frame_json: *const c_char,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_bidi_send: handle {handle} is not registered"),
        );
    }

    let raw = match read_cstr(frame_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "easynet_invocation_bidi_send: frame_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "easynet_invocation_bidi_send: frame_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (bidi_id, raw);
        record_invocation_feature_disabled("easynet_invocation_bidi_send")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_send_with_axon_pb(handle, bidi_id, raw)
    }
}

/// Half-close the local send side of an InvokeBidi session.
///
/// The session remains registered so the binding can continue to
/// receive down-direction frames and then call `easynet_invocation_bidi_close`
/// or `easynet_invocation_bidi_cancel`.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_bidi_close_send(
    handle: EasynetHandle,
    bidi_id: InvocationBidiId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_bidi_close_send: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        record_invocation_feature_disabled("easynet_invocation_bidi_close_send")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_close_send_with_axon_pb(handle, bidi_id)
    }
}

/// Gracefully close an InvokeBidi session by sending EOF, then drop
/// the local up-direction sender.
///
/// Unknown ids are treated as already closed and return `EASYNET_OK`.
///
/// # Safety
/// `handle` must be a live handle returned by this FFI and not
/// used concurrently from another thread during this call.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_bidi_close(
    handle: EasynetHandle,
    bidi_id: InvocationBidiId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_bidi_close: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        record_invocation_feature_disabled("easynet_invocation_bidi_close")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_close_with_axon_pb(handle, bidi_id)
    }
}

/// Cancel an InvokeBidi session locally.
///
/// Cancellation drops the local reader and up-direction sender
/// without sending protocol EOF. Unknown ids are treated as already
/// closed and return `EASYNET_OK`.
///
/// # Safety
/// `handle` must be a live handle returned by this FFI and not
/// used concurrently from another thread during this call.
#[no_mangle]
pub unsafe extern "C" fn easynet_invocation_bidi_cancel(
    handle: EasynetHandle,
    bidi_id: InvocationBidiId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("easynet_invocation_bidi_cancel: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        record_invocation_feature_disabled("easynet_invocation_bidi_cancel")
    }

    #[cfg(feature = "axon-pb")]
    {
        match remove_bidi_for_handle(handle, bidi_id) {
            Ok(Some(session)) => {
                session.cancel.cancel();
                clear_last_error();
                EASYNET_OK
            }
            Ok(None) => {
                clear_last_error();
                EASYNET_OK
            }
            Err(RegistryOwnerMismatch) => record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_bidi_cancel: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            ),
        }
    }
}

fn read_optional_cstr(ptr: *const c_char) -> Result<Option<String>, StringError> {
    if ptr.is_null() {
        return Ok(None);
    }
    read_cstr(ptr).map(|value| Some(value.to_string()))
}

#[cfg(feature = "axon-pb")]
fn runtime_health_json(session: &crate::ffi::client::handle::ClientSession) -> serde_json::Value {
    let report = RuntimeHealthReport::from_session(session);
    serde_json::json!({
        "api_ready": true,
        "daemon_ready": true,
        "invocation_ready": report.invocation_ready,
        "directory_ready": report.directory_ready,
        "trust_ready": report.trust_ready,
        "runtime_ready": report.runtime_ready,
        "version": env!("CARGO_PKG_VERSION"),
        "abi_version": crate::ffi::EASYNET_ABI_VERSION,
        "mismatch": null,
        "diagnostics": report.diagnostics,
    })
}

#[cfg(feature = "axon-pb")]
fn runtime_diagnostics_json(
    session: &crate::ffi::client::handle::ClientSession,
) -> serde_json::Value {
    let report = RuntimeHealthReport::from_session(session);
    serde_json::json!({
        "profile": "health",
        "kind": "diagnostics_report",
        "state": if report.runtime_ready { "Running" } else { "ControlOnly" },
        "ready": report.runtime_ready,
        "version": env!("CARGO_PKG_VERSION"),
        "abi_version": crate::ffi::EASYNET_ABI_VERSION,
        "control_endpoint": session.control_path,
        "invocation_endpoint": report.invocation_endpoint,
        "checks": report.checks(),
        "diagnostics": report.diagnostics,
    })
}

#[cfg(feature = "axon-pb")]
struct RuntimeHealthReport {
    invocation_endpoint: Option<String>,
    invocation_ready: bool,
    directory_ready: bool,
    trust_ready: bool,
    runtime_ready: bool,
    diagnostics: Vec<String>,
}

#[cfg(feature = "axon-pb")]
impl RuntimeHealthReport {
    fn from_session(session: &crate::ffi::client::handle::ClientSession) -> Self {
        match invocation_endpoint_for_session(session) {
            Ok(endpoint) => {
                let invocation_ready =
                    crate::support::platform::local_daemon_grpc::probe_accepting(&endpoint);
                let diagnostics = if invocation_ready {
                    Vec::new()
                } else {
                    vec!["invocation endpoint is not accepting connections".to_string()]
                };
                Self {
                    invocation_endpoint: Some(endpoint.display().to_string()),
                    invocation_ready,
                    directory_ready: invocation_ready,
                    trust_ready: true,
                    runtime_ready: invocation_ready,
                    diagnostics,
                }
            }
            Err(err) => Self {
                invocation_endpoint: None,
                invocation_ready: false,
                directory_ready: false,
                trust_ready: true,
                runtime_ready: false,
                diagnostics: vec![err.to_string()],
            },
        }
    }

    fn checks(&self) -> serde_json::Value {
        serde_json::json!([
            {"name": "api", "ready": true, "message": null},
            {"name": "daemon", "ready": true, "message": null},
            {
                "name": "invocation",
                "ready": self.invocation_ready,
                "message": if self.invocation_ready {
                    None
                } else {
                    Some("invocation endpoint unavailable")
                }
            },
            {
                "name": "directory",
                "ready": self.directory_ready,
                "message": if self.directory_ready {
                    None
                } else {
                    Some("directory readiness requires invocation endpoint")
                }
            },
            {"name": "trust", "ready": self.trust_ready, "message": null},
            {
                "name": "runtime",
                "ready": self.runtime_ready,
                "message": if self.runtime_ready {
                    None
                } else {
                    Some("runtime is degraded")
                }
            }
        ])
    }
}

#[cfg(feature = "axon-pb")]
fn read_builder_arg<'a>(
    function: &str,
    argument: &'static str,
    ptr: *const c_char,
) -> Result<&'a str, i32> {
    match read_cstr(ptr) {
        Ok(value) => Ok(value),
        Err(StringError::Null) => Err(record_invocation_error(
            ERR_NULL_POINTER,
            format!("{function}: {argument} pointer is null"),
        )),
        Err(StringError::NotUtf8) => Err(record_invocation_error(
            ERR_INVALID_UTF8,
            format!("{function}: {argument} is not valid UTF-8"),
        )),
    }
}

#[cfg(feature = "axon-pb")]
fn mutate_builder(
    builder_id: InvocationBuilderId,
    function: &str,
    mutate: impl FnOnce(&mut InvocationBuilderState) -> Result<(), InvocationJsonError>,
) -> i32 {
    let registry = builder_registry();
    let mut entries = lock_builder_entries(registry);
    let Some(builder) = entries.get_mut(&builder_id) else {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("{function}: builder handle {builder_id} is not registered"),
        );
    };
    if let Err(err) = mutate(builder) {
        return record_invocation_error(ERR_INVALID_ARG, format!("{function}: {err}"));
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn set_builder_string_field(
    builder_id: InvocationBuilderId,
    value: *const c_char,
    function: &str,
    argument: &'static str,
    field: InvocationBuilderStringField,
) -> i32 {
    let value = match read_builder_arg(function, argument, value) {
        Ok(value) => value,
        Err(code) => return code,
    };
    mutate_builder(builder_id, function, |builder| {
        builder.set_string_field(field, value)
    })
}

#[cfg(feature = "axon-pb")]
fn builder_output_invocation_json(
    builder_id: InvocationBuilderId,
    out_invocation_json: *mut *mut c_char,
    consume_on_success: bool,
    function: &str,
) -> i32 {
    if out_invocation_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            format!("{function}: out_invocation_json pointer is null"),
        );
    }
    unsafe { *out_invocation_json = std::ptr::null_mut() };

    let builder = if consume_on_success {
        match take_builder(builder_id) {
            Some(builder) => builder,
            None => {
                return record_invocation_error(
                    ERR_INVALID_HANDLE,
                    format!("{function}: builder handle {builder_id} is not registered"),
                );
            }
        }
    } else {
        match get_builder(builder_id) {
            Some(builder) => builder,
            None => {
                return record_invocation_error(
                    ERR_INVALID_HANDLE,
                    format!("{function}: builder handle {builder_id} is not registered"),
                );
            }
        }
    };
    let invocation = match builder.build_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            if consume_on_success {
                restore_builder(builder_id, builder);
            }
            return record_invocation_error(ERR_INVALID_ARG, format!("{function}: {err}"));
        }
    };
    let ptr = alloc_output_cstring(invocation_json(&invocation).to_string());
    if ptr.is_null() {
        if consume_on_success {
            restore_builder(builder_id, builder);
        }
        return record_invocation_error(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating invocation JSON"),
        );
    }
    unsafe { *out_invocation_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn prepare_builder_with_axon_pb(
    builder_id: InvocationBuilderId,
    options_raw: Option<String>,
    out_prepared_id: *mut PreparedInvocationId,
    out_prepared_json: *mut *mut c_char,
) -> i32 {
    let builder = match take_builder(builder_id) {
        Some(builder) => builder,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_builder_prepare: builder handle {builder_id} is not registered"
                ),
            );
        }
    };
    let options = match PrepareOptionsJson::parse(options_raw.as_deref()) {
        Ok(options) => options,
        Err(err) => {
            restore_builder(builder_id, builder);
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_builder_prepare: {err}"),
            );
        }
    };
    let invocation = match builder.build_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            restore_builder(builder_id, builder);
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_builder_prepare: {err}"),
            );
        }
    };
    let prepared = match invocation
        .into_draft()
        .prepare(options.into_prepare_options())
    {
        Ok(prepared) => prepared,
        Err(err) => {
            restore_builder(builder_id, builder);
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_builder_prepare: {err}"),
            );
        }
    };
    let ptr = alloc_output_cstring(prepared_invocation_json(&prepared).to_string());
    if ptr.is_null() {
        restore_builder(builder_id, builder);
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_builder_prepare: out-of-memory allocating prepared JSON",
        );
    }
    let id = insert_prepared(prepared);
    unsafe {
        *out_prepared_id = id;
        *out_prepared_json = ptr;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn invoke_with_axon_pb(
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    raw: &str,
    out_receipt_json: *mut *mut c_char,
) -> i32 {
    let spec = match InvocationJson::parse(raw) {
        Ok(spec) => spec,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_invoke: {err}"),
            );
        }
    };

    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_invoke: {err}"),
            );
        }
    };
    let tuple_json = invocation_json(&invocation);
    let tuple = invocation.clone().into_draft().inspect_tuple();

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_invoke: {err}"),
            );
        }
    };

    let response = match rt.block_on(async {
        let client = crate::daemon::DaemonClient::connect(invocation_endpoint_for_session(
            session.as_ref(),
        )?)?;
        client.invoke(invocation).await
    }) {
        Ok(response) => response,
        Err(err) => return ffi_daemon_error("easynet_invocation_invoke", err),
    };

    if let Some(err) = response.error.as_ref() {
        return record_invocation_error(
            ERR_ABILITY_FAILED,
            format!(
                "easynet_invocation_invoke: daemon returned error \"{}\": {}",
                err.code, err.message
            ),
        );
    }

    let output = invocation_result_json_with_tuple(
        invocation_result_from_invoke_response(tuple, response),
        tuple_json,
    );
    let json = match serde_json::to_string(&output) {
        Ok(json) => json,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_invoke: encode response JSON failed: {err}"),
            );
        }
    };
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_invoke: out-of-memory allocating response string",
        );
    }
    unsafe { *out_receipt_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn prepare_with_axon_pb(
    raw: &str,
    options_raw: Option<String>,
    out_prepared_id: *mut PreparedInvocationId,
    out_prepared_json: *mut *mut c_char,
) -> i32 {
    let spec = match InvocationJson::parse(raw) {
        Ok(spec) => spec,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_prepare: {err}"),
            );
        }
    };
    let options = match PrepareOptionsJson::parse(options_raw.as_deref()) {
        Ok(options) => options,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_prepare: {err}"),
            );
        }
    };
    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_prepare: {err}"),
            );
        }
    };
    let prepared = match invocation
        .into_draft()
        .prepare(options.into_prepare_options())
    {
        Ok(prepared) => prepared,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_prepare: {err}"),
            );
        }
    };
    let json = prepared_invocation_json(&prepared).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_prepare: out-of-memory allocating prepared JSON",
        );
    }
    let id = insert_prepared(prepared);
    unsafe {
        *out_prepared_id = id;
        *out_prepared_json = ptr;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn sign_prepared_with_axon_pb(
    prepared_id: PreparedInvocationId,
    raw: &str,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
) -> i32 {
    let signature = match SignatureMaterialJson::parse(raw) {
        Ok(signature) => signature.into_signature_material(),
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_sign_prepared: {err}"),
            );
        }
    };
    let prepared = match remove_prepared(prepared_id) {
        Some(prepared) => prepared,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_sign_prepared: prepared handle {prepared_id} is not registered"
                ),
            );
        }
    };
    let signed = match prepared.sign_with_caller_signature(signature) {
        Ok(signed) => signed,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_sign_prepared: {err}"),
            );
        }
    };
    let json = signed_invocation_json(&signed).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_sign_prepared: out-of-memory allocating signed JSON",
        );
    }
    let id = insert_signed(signed);
    unsafe {
        *out_signed_id = id;
        *out_signed_json = ptr;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn sign_prepared_local_with_axon_pb(
    prepared_id: PreparedInvocationId,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
) -> i32 {
    let prepared = match get_prepared(prepared_id) {
        Some(prepared) => prepared,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_sign_prepared_local: prepared handle {prepared_id} is not registered"
                ),
            );
        }
    };
    let signer = crate::daemon::KeyServiceLocalDaemonInvocationSigner::at_default_endpoint();
    let signed = match prepared.sign_with_local_daemon_signer(&signer) {
        Ok(signed) => signed,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_sign_prepared_local: {err}"),
            );
        }
    };
    let Some(_) = remove_prepared(prepared_id) else {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!(
                "easynet_invocation_sign_prepared_local: prepared handle {prepared_id} disappeared"
            ),
        );
    };
    let json = signed_invocation_json(&signed).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_sign_prepared_local: out-of-memory allocating signed JSON",
        );
    }
    let id = insert_signed(signed);
    unsafe {
        *out_signed_id = id;
        *out_signed_json = ptr;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn submit_signed_sync_with_axon_pb(
    owner: EasynetHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    signed_id: SignedInvocationId,
    out_result_json: *mut *mut c_char,
) -> i32 {
    let mut invocation_handle_id: InvocationHandleId = 0;
    let mut submitted_json_ptr: *mut c_char = std::ptr::null_mut();
    let submit_code = submit_signed_handle_with_axon_pb(
        owner,
        session,
        signed_id,
        &mut invocation_handle_id,
        &mut submitted_json_ptr,
    );
    if submit_code != EASYNET_OK {
        return submit_code;
    }
    unsafe { crate::ffi::strings::easynet_string_free(submitted_json_ptr) };

    let handle = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle,
        Ok(None) | Err(_) => {
            return record_invocation_error(
                ERR_GENERIC,
                "easynet_invocation_submit_signed: submitted invocation handle disappeared",
            );
        }
    };
    let (result, tuple_json) = handle.await_result_with_tuple_json();
    let _ = remove_invocation_handle_for_owner(owner, invocation_handle_id);
    if let Some(code) = record_sync_submit_terminal_error(&result) {
        return code;
    }
    let json = invocation_result_json_with_tuple(result, tuple_json).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_submit_signed: out-of-memory allocating result JSON",
        );
    }
    unsafe { *out_result_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn submit_signed_handle_with_axon_pb(
    owner: EasynetHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    signed_id: SignedInvocationId,
    out_invocation_handle_id: *mut InvocationHandleId,
    out_submitted_json: *mut *mut c_char,
) -> i32 {
    let signed = match remove_signed(signed_id) {
        Some(signed) => signed,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_submit_signed_handle: signed handle {signed_id} is not registered"
                ),
            );
        }
    };
    let endpoint = match invocation_endpoint_for_session(session.as_ref()) {
        Ok(endpoint) => endpoint,
        Err(err) => return ffi_daemon_error("easynet_invocation_submit_signed_handle", err),
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_submit_signed_handle: {err}"),
            );
        }
    };

    let tuple = signed.prepared().tuple();
    let tuple_json = invocation_json(signed.prepared().draft().invocation());
    let active = ActiveInvocationHandle::new(owner, tuple.clone(), tuple_json.clone());
    let cancel = active.cancel.clone();
    let shared = active.shared.clone();
    let invocation_handle_id = insert_invocation_handle(active);
    let submitted = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle.submitted_json(invocation_handle_id).to_string(),
        Ok(None) | Err(_) => {
            return record_invocation_error(
                ERR_GENERIC,
                "easynet_invocation_submit_signed_handle: inserted invocation handle disappeared",
            );
        }
    };
    let ptr = alloc_output_cstring(submitted);
    if ptr.is_null() {
        let _ = remove_invocation_handle_for_owner(owner, invocation_handle_id);
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_submit_signed_handle: out-of-memory allocating submitted JSON",
        );
    }

    rt.spawn(run_invocation_handle_task(
        endpoint, signed, tuple, shared, cancel,
    ));
    unsafe {
        *out_invocation_handle_id = invocation_handle_id;
        *out_submitted_json = ptr;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_await_with_axon_pb(
    owner: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
    out_result_json: *mut *mut c_char,
) -> i32 {
    let handle = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_await: invocation handle {invocation_handle_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_await: invocation handle {invocation_handle_id} does not belong to handle {owner}"
                ),
            );
        }
    };
    let json = handle.await_result_json().to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_handle_await: out-of-memory allocating result JSON",
        );
    }
    unsafe { *out_result_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_cancel_with_axon_pb(
    owner: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
    reason_raw: Option<String>,
    out_cancel_json: *mut *mut c_char,
) -> i32 {
    let handle = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_cancel: invocation handle {invocation_handle_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_cancel: invocation handle {invocation_handle_id} does not belong to handle {owner}"
                ),
            );
        }
    };
    let outcome = handle.cancel(reason_raw);
    let json = serde_json::json!({
        "handle_id": invocation_handle_id,
        "cancelled": outcome.cancelled,
        "state": outcome.state.as_str(),
        "terminal": outcome.terminal,
    })
    .to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_handle_cancel: out-of-memory allocating cancel JSON",
        );
    }
    unsafe { *out_cancel_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_events_with_axon_pb(
    owner: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
    out_events_json: *mut *mut c_char,
) -> i32 {
    let handle = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_events: invocation handle {invocation_handle_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_handle_events: invocation handle {invocation_handle_id} does not belong to handle {owner}"
                ),
            );
        }
    };
    let json = handle.events_json(invocation_handle_id).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "easynet_invocation_handle_events: out-of-memory allocating events JSON",
        );
    }
    unsafe { *out_events_json = ptr };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
async fn run_invocation_handle_task(
    endpoint: PathBuf,
    signed: crate::daemon::SignedInvocation,
    tuple: crate::daemon::InvocationTuple,
    shared: Arc<InvocationHandleShared>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let result = tokio::select! {
        _ = cancel.cancelled() => invocation_cancelled_result(&tuple, Some("cancelled before runtime terminal")),
        outcome = async {
            let client = crate::daemon::RuntimeClient::connect(endpoint)?;
            client.submit_signed(signed).await.map(|handle| handle.await_result())
        } => match outcome {
            Ok(result) => result,
            Err(err) => invocation_failed_result(&tuple, err),
        },
    };
    let _ = shared.mark_terminal(result);
}

#[cfg(feature = "axon-pb")]
fn stream_open_with_axon_pb(
    handle: EasynetHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    raw: &str,
    on_chunk: InvocationStreamCallback,
    user_data: *mut c_void,
    out_stream_id: *mut InvocationStreamId,
) -> i32 {
    let spec = match InvocationJson::parse(raw) {
        Ok(spec) => spec,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_stream_open: {err}"),
            );
        }
    };

    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_stream_open: {err}"),
            );
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_stream_open: {err}"),
            );
        }
    };

    let stream = match rt.block_on(async {
        let client = crate::daemon::DaemonClient::connect(invocation_endpoint_for_session(
            session.as_ref(),
        )?)?;
        client.invoke_stream(invocation).await
    }) {
        Ok(stream) => stream,
        Err(err) => return ffi_daemon_error("easynet_invocation_stream_open", err),
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(STREAM_CALLBACK_QUEUE_CAPACITY);
    let callback_user_data = CallbackUserData(user_data);
    let dispatcher = std::thread::Builder::new()
        .name("easynet-inv-stream-callback".to_string())
        .spawn(move || dispatch_stream_callbacks(rx, on_chunk, callback_user_data));
    if let Err(err) = dispatcher {
        return record_invocation_error(
            ERR_GENERIC,
            format!("easynet_invocation_stream_open: spawn callback dispatcher failed: {err}"),
        );
    }

    let cancel = tokio_util::sync::CancellationToken::new();
    let stream_id = insert_stream(ActiveInvocationStream {
        owner: handle,
        cancel: cancel.clone(),
    });
    rt.spawn(run_stream_reader(stream_id, stream, cancel, tx));

    unsafe { *out_stream_id = stream_id };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn bidi_open_with_axon_pb(
    handle: EasynetHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    raw: &str,
    on_frame: InvocationBidiCallback,
    user_data: *mut c_void,
    out_bidi_id: *mut InvocationBidiId,
) -> i32 {
    let spec = match InvocationJson::parse(raw) {
        Ok(spec) => spec,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_bidi_open: {err}"),
            );
        }
    };
    if spec.bidi_streams.is_empty() {
        return record_invocation_error(
            ERR_INVALID_ARG,
            "easynet_invocation_bidi_open: bidi_streams must not be empty",
        );
    }

    let streams = spec.bidi_streams.clone();
    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_bidi_open: {err}"),
            );
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_bidi_open: {err}"),
            );
        }
    };

    let session = match rt.block_on(async {
        let client = crate::daemon::DaemonClient::connect(invocation_endpoint_for_session(
            session.as_ref(),
        )?)?;
        client.invoke_bidi(invocation, streams).await
    }) {
        Ok(session) => session,
        Err(err) => return ffi_daemon_error("easynet_invocation_bidi_open", err),
    };

    let (callback_tx, callback_rx) =
        tokio::sync::mpsc::channel::<Vec<u8>>(BIDI_CALLBACK_QUEUE_CAPACITY);
    let callback_user_data = CallbackUserData(user_data);
    let dispatcher = std::thread::Builder::new()
        .name("easynet-inv-bidi-callback".to_string())
        .spawn(move || dispatch_bidi_callbacks(callback_rx, on_frame, callback_user_data));
    if let Err(err) = dispatcher {
        return record_invocation_error(
            ERR_GENERIC,
            format!("easynet_invocation_bidi_open: spawn callback dispatcher failed: {err}"),
        );
    }

    let (ability, up_tx, down) = session.into_parts();
    let cancel = tokio_util::sync::CancellationToken::new();
    let bidi_id = insert_bidi(ActiveInvocationBidi::new(
        handle,
        ability,
        up_tx,
        cancel.clone(),
    ));
    rt.spawn(run_bidi_down_reader(bidi_id, down, cancel, callback_tx));

    unsafe { *out_bidi_id = bidi_id };
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn bidi_send_with_axon_pb(handle: EasynetHandle, bidi_id: InvocationBidiId, raw: &str) -> i32 {
    let frame = match parse_bidi_up_frame_json(raw) {
        Ok(frame) => frame,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("easynet_invocation_bidi_send: {err}"),
            );
        }
    };
    let session = match get_bidi_for_handle(handle, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("easynet_invocation_bidi_send: bidi session {bidi_id} is not registered"),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_bidi_send: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            );
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_bidi_send: {err}"),
            );
        }
    };

    let up_frame = match session.reserve_up_frame(frame) {
        Ok(up_frame) => up_frame,
        Err(BidiLocalSendClosed) => {
            return record_invocation_error(
                ERR_CANCELLED,
                format!(
                    "easynet_invocation_bidi_send: bidi session {} for {} is locally half-closed",
                    bidi_id, session.ability
                ),
            );
        }
    };

    let send_code = send_bidi_up_frame(
        rt,
        "easynet_invocation_bidi_send",
        bidi_id,
        &session,
        up_frame,
    );
    if send_code != EASYNET_OK {
        let _ = remove_bidi_for_handle(handle, bidi_id);
        return send_code;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn stream_close_with_axon_pb(handle: EasynetHandle, stream_id: InvocationStreamId) -> i32 {
    release_stream_with_reader_cancel(handle, stream_id, "easynet_invocation_stream_close")
}

#[cfg(feature = "axon-pb")]
fn release_stream_with_reader_cancel(
    handle: EasynetHandle,
    stream_id: InvocationStreamId,
    function: &str,
) -> i32 {
    match remove_stream_for_handle(handle, stream_id) {
        Ok(Some(stream)) => {
            stream.cancel.cancel();
            clear_last_error();
            EASYNET_OK
        }
        Ok(None) => {
            clear_last_error();
            EASYNET_OK
        }
        Err(RegistryOwnerMismatch) => record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("{function}: stream {stream_id} does not belong to handle {handle}"),
        ),
    }
}

#[cfg(feature = "axon-pb")]
fn bidi_close_send_with_axon_pb(handle: EasynetHandle, bidi_id: InvocationBidiId) -> i32 {
    let session = match get_bidi_for_handle(handle, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_bidi_close_send: bidi session {bidi_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_bidi_close_send: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            );
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_bidi_close_send: {err}"),
            );
        }
    };

    let Some(up_frame) = session.reserve_close_send_frame() else {
        clear_last_error();
        return EASYNET_OK;
    };
    let send_code = send_bidi_up_frame(
        rt,
        "easynet_invocation_bidi_close_send",
        bidi_id,
        &session,
        up_frame,
    );
    if send_code != EASYNET_OK {
        let _ = remove_bidi_for_handle(handle, bidi_id);
        return send_code;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn bidi_close_with_axon_pb(handle: EasynetHandle, bidi_id: InvocationBidiId) -> i32 {
    let session = match remove_bidi_for_handle(handle, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            clear_last_error();
            return EASYNET_OK;
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "easynet_invocation_bidi_close: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            );
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("easynet_invocation_bidi_close: {err}"),
            );
        }
    };

    let Some(up_frame) = session.reserve_close_send_frame() else {
        clear_last_error();
        return EASYNET_OK;
    };
    let send_code = send_bidi_up_frame(
        rt,
        "easynet_invocation_bidi_close",
        bidi_id,
        &session,
        up_frame,
    );
    if send_code != EASYNET_OK {
        return send_code;
    }
    clear_last_error();
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
fn send_bidi_up_frame(
    rt: &'static tokio::runtime::Runtime,
    function: &str,
    bidi_id: InvocationBidiId,
    session: &ActiveInvocationBidi,
    up_frame: easynet_axon::pb::axon::v1::InvokeBidiUp,
) -> i32 {
    let send_result = rt.block_on(async { session.up_tx.send(up_frame).await });
    if send_result.is_err() {
        return record_invocation_error(
            ERR_CANCELLED,
            format!(
                "{function}: bidi session {} for {} is closed",
                bidi_id, session.ability
            ),
        );
    }
    EASYNET_OK
}

#[cfg(feature = "axon-pb")]
#[derive(Clone, Copy)]
struct CallbackUserData(*mut c_void);

#[cfg(feature = "axon-pb")]
unsafe impl Send for CallbackUserData {}

#[cfg(feature = "axon-pb")]
impl CallbackUserData {
    fn raw(self) -> *mut c_void {
        self.0
    }
}

#[cfg(feature = "axon-pb")]
struct ActiveInvocationStream {
    owner: EasynetHandle,
    cancel: tokio_util::sync::CancellationToken,
}

#[cfg(feature = "axon-pb")]
struct ActiveInvocationHandle {
    owner: EasynetHandle,
    cancel: tokio_util::sync::CancellationToken,
    shared: Arc<InvocationHandleShared>,
}

#[cfg(feature = "axon-pb")]
impl ActiveInvocationHandle {
    fn new(
        owner: EasynetHandle,
        tuple: crate::daemon::InvocationTuple,
        tuple_json: serde_json::Value,
    ) -> Self {
        Self {
            owner,
            cancel: tokio_util::sync::CancellationToken::new(),
            shared: Arc::new(InvocationHandleShared::new(tuple, tuple_json)),
        }
    }

    fn submitted_json(&self, invocation_handle_id: InvocationHandleId) -> serde_json::Value {
        self.shared.snapshot_json(invocation_handle_id)
    }

    fn await_result_with_tuple_json(&self) -> (crate::daemon::InvocationResult, serde_json::Value) {
        self.shared.await_result_with_tuple_json()
    }

    #[cfg(test)]
    fn await_result(&self) -> crate::daemon::InvocationResult {
        self.shared.await_result()
    }

    fn await_result_json(&self) -> serde_json::Value {
        let (result, tuple_json) = self.shared.await_result_with_tuple_json();
        invocation_result_json_with_tuple(result, tuple_json)
    }

    fn cancel(&self, reason: Option<String>) -> InvocationHandleCancelOutcome {
        self.cancel.cancel();
        self.shared.cancel(reason)
    }

    fn events_json(&self, invocation_handle_id: InvocationHandleId) -> serde_json::Value {
        self.shared.snapshot_json(invocation_handle_id)
    }
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleShared {
    inner: Mutex<InvocationHandleState>,
    terminal: Condvar,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandleShared {
    fn new(tuple: crate::daemon::InvocationTuple, tuple_json: serde_json::Value) -> Self {
        Self {
            inner: Mutex::new(InvocationHandleState::submitted(tuple, tuple_json)),
            terminal: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, InvocationHandleState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn await_result_with_tuple_json(&self) -> (crate::daemon::InvocationResult, serde_json::Value) {
        let mut state = self.lock();
        while state.terminal_result.is_none() {
            state = self
                .terminal
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        (
            state
                .terminal_result
                .clone()
                .expect("terminal result is present after wait"),
            state.tuple_json.clone(),
        )
    }

    #[cfg(test)]
    fn await_result(&self) -> crate::daemon::InvocationResult {
        self.await_result_with_tuple_json().0
    }

    fn cancel(&self, reason: Option<String>) -> InvocationHandleCancelOutcome {
        let mut state = self.lock();
        if state.phase.is_terminal() {
            return InvocationHandleCancelOutcome {
                cancelled: false,
                state: state.phase,
                terminal: true,
            };
        }
        let result = invocation_cancelled_result(&state.tuple, reason.as_deref());
        state.push_terminal(
            InvocationHandlePhase::Cancelled,
            "cancelled",
            reason,
            result,
        );
        self.terminal.notify_all();
        InvocationHandleCancelOutcome {
            cancelled: true,
            state: InvocationHandlePhase::Cancelled,
            terminal: true,
        }
    }

    fn mark_terminal(&self, result: crate::daemon::InvocationResult) -> bool {
        let mut state = self.lock();
        if state.phase.is_terminal() {
            return false;
        }
        let phase = terminal_phase_for_result(&result);
        state.push_terminal(phase, phase.event_kind(), None, result);
        self.terminal.notify_all();
        true
    }

    fn snapshot_json(&self, invocation_handle_id: InvocationHandleId) -> serde_json::Value {
        let state = self.lock();
        state.snapshot_json(invocation_handle_id)
    }
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleState {
    tuple: crate::daemon::InvocationTuple,
    tuple_json: serde_json::Value,
    phase: InvocationHandlePhase,
    next_sequence: u64,
    events: Vec<InvocationHandleEvent>,
    terminal_result: Option<crate::daemon::InvocationResult>,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandleState {
    fn submitted(tuple: crate::daemon::InvocationTuple, tuple_json: serde_json::Value) -> Self {
        Self {
            tuple,
            tuple_json,
            phase: InvocationHandlePhase::Submitted,
            next_sequence: 2,
            events: vec![InvocationHandleEvent {
                sequence: 1,
                state: InvocationHandlePhase::Submitted,
                kind: "submitted".to_string(),
                terminal: false,
                reason: None,
                result: None,
            }],
            terminal_result: None,
        }
    }

    fn push_terminal(
        &mut self,
        phase: InvocationHandlePhase,
        kind: &'static str,
        reason: Option<String>,
        result: crate::daemon::InvocationResult,
    ) {
        self.phase = phase;
        self.events.push(InvocationHandleEvent {
            sequence: self.next_sequence,
            state: phase,
            kind: kind.to_string(),
            terminal: true,
            reason,
            result: Some(result.clone()),
        });
        self.next_sequence += 1;
        self.terminal_result = Some(result);
    }

    fn snapshot_json(&self, invocation_handle_id: InvocationHandleId) -> serde_json::Value {
        serde_json::json!({
            "handle_id": invocation_handle_id,
            "state": self.phase.as_str(),
            "terminal": self.phase.is_terminal(),
            "events": self.events.iter().map(|event| event.to_json(&self.tuple_json)).collect::<Vec<_>>(),
            "result": self.terminal_result.clone().map(|result| invocation_result_json_with_tuple(result, self.tuple_json.clone())),
        })
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct InvocationHandleEvent {
    sequence: u64,
    state: InvocationHandlePhase,
    kind: String,
    terminal: bool,
    reason: Option<String>,
    result: Option<crate::daemon::InvocationResult>,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandleEvent {
    fn to_json(&self, tuple_json: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "sequence": self.sequence,
            "kind": self.kind,
            "state": self.state.as_str(),
            "terminal": self.terminal,
            "reason": self.reason,
            "result": self.result.clone().map(|result| invocation_result_json_with_tuple(result, tuple_json.clone())),
        })
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvocationHandlePhase {
    Submitted,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandlePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "Submitted",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::TimedOut => "TimedOut",
            Self::Cancelled => "Cancelled",
        }
    }

    fn event_kind(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    fn is_terminal(self) -> bool {
        !matches!(self, Self::Submitted)
    }

    fn to_axon_wire_state_string(self) -> String {
        match self {
            Self::Submitted => {
                axon_state_wire_string(easynet_axon::invocation::InvocationState::Running)
            }
            Self::Completed => {
                axon_state_wire_string(easynet_axon::invocation::InvocationState::Completed)
            }
            Self::Failed => {
                axon_state_wire_string(easynet_axon::invocation::InvocationState::Failed)
            }
            Self::TimedOut => {
                axon_state_wire_string(easynet_axon::invocation::InvocationState::TimedOut)
            }
            Self::Cancelled => {
                axon_state_wire_string(easynet_axon::invocation::InvocationState::Cancelled)
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleCancelOutcome {
    cancelled: bool,
    state: InvocationHandlePhase,
    terminal: bool,
}

#[cfg(feature = "axon-pb")]
struct ActiveInvocationBidi {
    owner: EasynetHandle,
    ability: String,
    up_tx: tokio::sync::mpsc::Sender<easynet_axon::pb::axon::v1::InvokeBidiUp>,
    cancel: tokio_util::sync::CancellationToken,
    local_send: Mutex<BidiLocalSendState>,
}

#[cfg(feature = "axon-pb")]
impl ActiveInvocationBidi {
    fn new(
        owner: EasynetHandle,
        ability: String,
        up_tx: tokio::sync::mpsc::Sender<easynet_axon::pb::axon::v1::InvokeBidiUp>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            owner,
            ability,
            up_tx,
            cancel,
            local_send: Mutex::new(BidiLocalSendState::new()),
        }
    }

    fn reserve_up_frame(
        &self,
        frame: BidiUpFrame,
    ) -> Result<easynet_axon::pb::axon::v1::InvokeBidiUp, BidiLocalSendClosed> {
        let mut state = self
            .local_send
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.is_half_closed_local() {
            return Err(BidiLocalSendClosed);
        }

        let sequence = state.next_sequence();
        if bidi_up_payload_is_eof(&frame.payload) {
            state.half_close_local();
        }
        Ok(easynet_axon::pb::axon::v1::InvokeBidiUp {
            sequence,
            mac: frame.mac,
            payload: Some(frame.payload),
        })
    }

    fn reserve_close_send_frame(&self) -> Option<easynet_axon::pb::axon::v1::InvokeBidiUp> {
        let mut state = self
            .local_send
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.is_half_closed_local() {
            return None;
        }
        state.half_close_local();
        Some(bidi_eof_up_frame(state.next_sequence()))
    }
}

#[cfg(feature = "axon-pb")]
struct BidiLocalSendState {
    phase: BidiLocalSendPhase,
    next_sequence: u64,
}

#[cfg(feature = "axon-pb")]
impl BidiLocalSendState {
    fn new() -> Self {
        Self {
            phase: BidiLocalSendPhase::Open,
            next_sequence: 1,
        }
    }

    fn is_half_closed_local(&self) -> bool {
        self.phase == BidiLocalSendPhase::HalfClosedLocal
    }

    fn half_close_local(&mut self) {
        self.phase = BidiLocalSendPhase::HalfClosedLocal;
    }

    fn next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BidiLocalSendPhase {
    Open,
    HalfClosedLocal,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BidiLocalSendClosed;

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegistryOwnerMismatch;

#[cfg(feature = "axon-pb")]
struct StreamRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<InvocationStreamId, ActiveInvocationStream>>,
}

#[cfg(feature = "axon-pb")]
struct BidiRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<InvocationBidiId, Arc<ActiveInvocationBidi>>>,
}

#[cfg(feature = "axon-pb")]
struct BuilderRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<InvocationBuilderId, InvocationBuilderState>>,
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<InvocationHandleId, Arc<ActiveInvocationHandle>>>,
}

#[cfg(feature = "axon-pb")]
struct PreparedRegistry {
    next: AtomicU64,
    entries:
        Mutex<std::collections::HashMap<PreparedInvocationId, crate::daemon::PreparedInvocation>>,
}

#[cfg(feature = "axon-pb")]
struct SignedRegistry {
    next: AtomicU64,
    entries: Mutex<std::collections::HashMap<SignedInvocationId, crate::daemon::SignedInvocation>>,
}

#[cfg(feature = "axon-pb")]
fn stream_registry() -> &'static StreamRegistry {
    static REGISTRY: OnceLock<StreamRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| StreamRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

#[cfg(feature = "axon-pb")]
fn bidi_registry() -> &'static BidiRegistry {
    static REGISTRY: OnceLock<BidiRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| BidiRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

#[cfg(feature = "axon-pb")]
fn builder_registry() -> &'static BuilderRegistry {
    static REGISTRY: OnceLock<BuilderRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| BuilderRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_registry() -> &'static InvocationHandleRegistry {
    static REGISTRY: OnceLock<InvocationHandleRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| InvocationHandleRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

#[cfg(feature = "axon-pb")]
fn prepared_registry() -> &'static PreparedRegistry {
    static REGISTRY: OnceLock<PreparedRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| PreparedRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

#[cfg(feature = "axon-pb")]
fn signed_registry() -> &'static SignedRegistry {
    static REGISTRY: OnceLock<SignedRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| SignedRegistry {
        next: AtomicU64::new(1),
        entries: Mutex::new(std::collections::HashMap::new()),
    })
}

#[cfg(feature = "axon-pb")]
fn lock_stream_entries(
    registry: &StreamRegistry,
) -> MutexGuard<'_, std::collections::HashMap<InvocationStreamId, ActiveInvocationStream>> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "axon-pb")]
fn lock_bidi_entries(
    registry: &BidiRegistry,
) -> MutexGuard<'_, std::collections::HashMap<InvocationBidiId, Arc<ActiveInvocationBidi>>> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "axon-pb")]
fn lock_builder_entries(
    registry: &BuilderRegistry,
) -> MutexGuard<'_, std::collections::HashMap<InvocationBuilderId, InvocationBuilderState>> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "axon-pb")]
fn lock_invocation_handle_entries(
    registry: &InvocationHandleRegistry,
) -> MutexGuard<'_, std::collections::HashMap<InvocationHandleId, Arc<ActiveInvocationHandle>>> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "axon-pb")]
fn lock_prepared_entries(
    registry: &PreparedRegistry,
) -> MutexGuard<
    '_,
    std::collections::HashMap<PreparedInvocationId, crate::daemon::PreparedInvocation>,
> {
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "axon-pb")]
fn lock_signed_entries(
    registry: &SignedRegistry,
) -> MutexGuard<'_, std::collections::HashMap<SignedInvocationId, crate::daemon::SignedInvocation>>
{
    registry
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "axon-pb")]
fn insert_stream(stream: ActiveInvocationStream) -> InvocationStreamId {
    let registry = stream_registry();
    let stream_id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_stream_entries(registry).insert(stream_id, stream);
    stream_id
}

#[cfg(feature = "axon-pb")]
fn insert_bidi(session: ActiveInvocationBidi) -> InvocationBidiId {
    let registry = bidi_registry();
    let bidi_id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_bidi_entries(registry).insert(bidi_id, Arc::new(session));
    bidi_id
}

#[cfg(feature = "axon-pb")]
fn insert_builder(builder: InvocationBuilderState) -> InvocationBuilderId {
    let registry = builder_registry();
    let builder_id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_builder_entries(registry).insert(builder_id, builder);
    builder_id
}

#[cfg(feature = "axon-pb")]
fn insert_invocation_handle(handle: ActiveInvocationHandle) -> InvocationHandleId {
    let registry = invocation_handle_registry();
    let invocation_handle_id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_invocation_handle_entries(registry).insert(invocation_handle_id, Arc::new(handle));
    invocation_handle_id
}

#[cfg(feature = "axon-pb")]
fn insert_prepared(prepared: crate::daemon::PreparedInvocation) -> PreparedInvocationId {
    let registry = prepared_registry();
    let prepared_id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_prepared_entries(registry).insert(prepared_id, prepared);
    prepared_id
}

#[cfg(feature = "axon-pb")]
fn insert_signed(signed: crate::daemon::SignedInvocation) -> SignedInvocationId {
    let registry = signed_registry();
    let signed_id = registry.next.fetch_add(1, Ordering::Relaxed);
    lock_signed_entries(registry).insert(signed_id, signed);
    signed_id
}

#[cfg(feature = "axon-pb")]
fn get_builder(builder_id: InvocationBuilderId) -> Option<InvocationBuilderState> {
    if builder_id == 0 {
        return None;
    }
    lock_builder_entries(builder_registry())
        .get(&builder_id)
        .cloned()
}

#[cfg(feature = "axon-pb")]
fn get_invocation_handle_for_owner(
    owner: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
) -> Result<Option<Arc<ActiveInvocationHandle>>, RegistryOwnerMismatch> {
    if invocation_handle_id == 0 {
        return Ok(None);
    }
    let handle = lock_invocation_handle_entries(invocation_handle_registry())
        .get(&invocation_handle_id)
        .cloned();
    let Some(handle) = handle else {
        return Ok(None);
    };
    if handle.owner != owner {
        return Err(RegistryOwnerMismatch);
    }
    Ok(Some(handle))
}

#[cfg(feature = "axon-pb")]
fn take_builder(builder_id: InvocationBuilderId) -> Option<InvocationBuilderState> {
    if builder_id == 0 {
        return None;
    }
    lock_builder_entries(builder_registry()).remove(&builder_id)
}

#[cfg(feature = "axon-pb")]
fn restore_builder(builder_id: InvocationBuilderId, builder: InvocationBuilderState) {
    if builder_id == 0 {
        return;
    }
    lock_builder_entries(builder_registry()).insert(builder_id, builder);
}

#[cfg(feature = "axon-pb")]
fn get_prepared(prepared_id: PreparedInvocationId) -> Option<crate::daemon::PreparedInvocation> {
    if prepared_id == 0 {
        return None;
    }
    lock_prepared_entries(prepared_registry())
        .get(&prepared_id)
        .cloned()
}

#[cfg(all(test, feature = "axon-pb"))]
fn get_signed(signed_id: SignedInvocationId) -> Option<crate::daemon::SignedInvocation> {
    if signed_id == 0 {
        return None;
    }
    lock_signed_entries(signed_registry())
        .get(&signed_id)
        .cloned()
}

#[cfg(feature = "axon-pb")]
fn remove_prepared(prepared_id: PreparedInvocationId) -> Option<crate::daemon::PreparedInvocation> {
    if prepared_id == 0 {
        return None;
    }
    lock_prepared_entries(prepared_registry()).remove(&prepared_id)
}

#[cfg(feature = "axon-pb")]
fn remove_signed(signed_id: SignedInvocationId) -> Option<crate::daemon::SignedInvocation> {
    if signed_id == 0 {
        return None;
    }
    lock_signed_entries(signed_registry()).remove(&signed_id)
}

#[cfg(feature = "axon-pb")]
fn remove_builder(builder_id: InvocationBuilderId) -> Option<InvocationBuilderState> {
    if builder_id == 0 {
        return None;
    }
    lock_builder_entries(builder_registry()).remove(&builder_id)
}

#[cfg(feature = "axon-pb")]
fn remove_invocation_handle_for_owner(
    owner: EasynetHandle,
    invocation_handle_id: InvocationHandleId,
) -> Result<Option<Arc<ActiveInvocationHandle>>, RegistryOwnerMismatch> {
    if invocation_handle_id == 0 {
        return Ok(None);
    }
    let registry = invocation_handle_registry();
    let mut entries = lock_invocation_handle_entries(registry);
    let Some(handle) = entries.get(&invocation_handle_id) else {
        return Ok(None);
    };
    if handle.owner != owner {
        return Err(RegistryOwnerMismatch);
    }
    Ok(entries.remove(&invocation_handle_id))
}

#[cfg(feature = "axon-pb")]
fn remove_stream(stream_id: InvocationStreamId) -> Option<ActiveInvocationStream> {
    if stream_id == 0 {
        return None;
    }
    lock_stream_entries(stream_registry()).remove(&stream_id)
}

#[cfg(feature = "axon-pb")]
fn remove_stream_for_handle(
    owner: EasynetHandle,
    stream_id: InvocationStreamId,
) -> Result<Option<ActiveInvocationStream>, RegistryOwnerMismatch> {
    if stream_id == 0 {
        return Ok(None);
    }
    let registry = stream_registry();
    let mut entries = lock_stream_entries(registry);
    let Some(stream) = entries.get(&stream_id) else {
        return Ok(None);
    };
    if stream.owner != owner {
        return Err(RegistryOwnerMismatch);
    }
    Ok(entries.remove(&stream_id))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn cancel_invocations_for_handle(owner: EasynetHandle) {
    if owner == 0 {
        return;
    }

    let handles = {
        let registry = invocation_handle_registry();
        let mut entries = lock_invocation_handle_entries(registry);
        let owned_ids = entries
            .iter()
            .filter_map(|(id, handle)| (handle.owner == owner).then_some(*id))
            .collect::<Vec<_>>();
        owned_ids
            .into_iter()
            .filter_map(|id| entries.remove(&id))
            .collect::<Vec<_>>()
    };
    for handle in handles {
        let _ = handle.cancel(Some("owning EasynetHandle shutdown".to_string()));
    }

    let streams = {
        let registry = stream_registry();
        let mut entries = lock_stream_entries(registry);
        let owned_ids = entries
            .iter()
            .filter_map(|(id, stream)| (stream.owner == owner).then_some(*id))
            .collect::<Vec<_>>();
        owned_ids
            .into_iter()
            .filter_map(|id| entries.remove(&id))
            .collect::<Vec<_>>()
    };
    for stream in streams {
        stream.cancel.cancel();
    }

    let bidis = {
        let registry = bidi_registry();
        let mut entries = lock_bidi_entries(registry);
        let owned_ids = entries
            .iter()
            .filter_map(|(id, session)| (session.owner == owner).then_some(*id))
            .collect::<Vec<_>>();
        owned_ids
            .into_iter()
            .filter_map(|id| entries.remove(&id))
            .collect::<Vec<_>>()
    };
    for session in bidis {
        session.cancel.cancel();
    }
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn cancel_invocations_for_handle(_owner: EasynetHandle) {}

#[cfg(feature = "axon-pb")]
fn get_bidi_for_handle(
    owner: EasynetHandle,
    bidi_id: InvocationBidiId,
) -> Result<Option<Arc<ActiveInvocationBidi>>, RegistryOwnerMismatch> {
    if bidi_id == 0 {
        return Ok(None);
    }
    let session = lock_bidi_entries(bidi_registry()).get(&bidi_id).cloned();
    let Some(session) = session else {
        return Ok(None);
    };
    if session.owner != owner {
        return Err(RegistryOwnerMismatch);
    }
    Ok(Some(session))
}

#[cfg(feature = "axon-pb")]
fn remove_bidi(bidi_id: InvocationBidiId) -> Option<Arc<ActiveInvocationBidi>> {
    if bidi_id == 0 {
        return None;
    }
    lock_bidi_entries(bidi_registry()).remove(&bidi_id)
}

#[cfg(feature = "axon-pb")]
fn remove_bidi_for_handle(
    owner: EasynetHandle,
    bidi_id: InvocationBidiId,
) -> Result<Option<Arc<ActiveInvocationBidi>>, RegistryOwnerMismatch> {
    if bidi_id == 0 {
        return Ok(None);
    }
    let registry = bidi_registry();
    let mut entries = lock_bidi_entries(registry);
    let Some(session) = entries.get(&bidi_id) else {
        return Ok(None);
    };
    if session.owner != owner {
        return Err(RegistryOwnerMismatch);
    }
    Ok(entries.remove(&bidi_id))
}

#[cfg(feature = "axon-pb")]
fn ffi_daemon_error(context: &str, err: crate::daemon::DaemonError) -> i32 {
    let code = match &err {
        crate::daemon::DaemonError::InvocationEndpointDown { .. }
        | crate::daemon::DaemonError::InvocationEndpointMissing { .. }
        | crate::daemon::DaemonError::Connect { .. } => ERR_DAEMON_DOWN,
        crate::daemon::DaemonError::InvokeStatus { code, .. }
        | crate::daemon::DaemonError::InvokeStreamStatus { code, .. }
        | crate::daemon::DaemonError::InvokeBidiStatus { code, .. } => {
            ffi_status_code_to_error(*code)
        }
        crate::daemon::DaemonError::InvalidInvocation(_) => ERR_INVALID_ARG,
        crate::daemon::DaemonError::InvokeBidiClosed { .. } => ERR_CANCELLED,
        _ => ERR_GENERIC,
    };
    record_invocation_error(code, format!("{context}: {err}"))
}

#[cfg(feature = "axon-pb")]
fn ffi_status_code_to_error(code: tonic::Code) -> i32 {
    match code {
        tonic::Code::Ok => EASYNET_OK,
        tonic::Code::Cancelled => ERR_CANCELLED,
        tonic::Code::InvalidArgument | tonic::Code::OutOfRange => ERR_INVALID_ARG,
        tonic::Code::DeadlineExceeded => ERR_TIMEOUT,
        tonic::Code::NotFound => ERR_NOT_FOUND,
        tonic::Code::AlreadyExists
        | tonic::Code::FailedPrecondition
        | tonic::Code::Aborted
        | tonic::Code::ResourceExhausted => ERR_ABILITY_FAILED,
        tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => ERR_PERMISSION_DENIED,
        tonic::Code::Unimplemented => ERR_NOT_IMPLEMENTED,
        tonic::Code::Unavailable => ERR_DAEMON_DOWN,
        tonic::Code::Unknown | tonic::Code::Internal | tonic::Code::DataLoss => ERR_PROTOCOL,
    }
}

#[cfg(feature = "axon-pb")]
fn dispatch_stream_callbacks(
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    on_chunk: InvocationStreamCallback,
    user_data: CallbackUserData,
) {
    let raw_user_data = user_data.raw();
    while let Some(json_bytes) = rx.blocking_recv() {
        let cstr = match std::ffi::CString::new(json_bytes) {
            Ok(cstr) => cstr,
            Err(_) => continue,
        };
        unsafe {
            on_chunk(raw_user_data, cstr.as_ptr());
        }
    }
    // End-of-stream signal: the daemon stream closed (terminal frame
    // delivered, or transport ended). Deliver ONE final callback with a
    // null `chunk_json` so the consumer has an unambiguous EOF marker —
    // without it a queue-backed consumer blocks forever waiting on a
    // frame that will never arrive. Bindings treat a null chunk as
    // "stream finished", never as a data frame.
    unsafe {
        on_chunk(raw_user_data, std::ptr::null());
    }
}

#[cfg(feature = "axon-pb")]
fn dispatch_bidi_callbacks(
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    on_frame: InvocationBidiCallback,
    user_data: CallbackUserData,
) {
    let raw_user_data = user_data.raw();
    while let Some(json_bytes) = rx.blocking_recv() {
        let cstr = match std::ffi::CString::new(json_bytes) {
            Ok(cstr) => cstr,
            Err(_) => continue,
        };
        unsafe {
            on_frame(raw_user_data, cstr.as_ptr());
        }
    }
}

#[cfg(feature = "axon-pb")]
async fn run_stream_reader(
    stream_id: InvocationStreamId,
    mut stream: tonic::Streaming<easynet_axon::pb::axon::v1::InvokeStreamChunk>,
    cancel: tokio_util::sync::CancellationToken,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let mut next_error_sequence = 1;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            message = stream.message() => match message {
                Ok(Some(chunk)) => {
                    let terminal = chunk.terminal;
                    let sequence = chunk.sequence;
                    next_error_sequence = sequence.saturating_add(1).max(1);
                    let bytes = stream_chunk_json(chunk).to_string().into_bytes();
                    let sent = send_callback_frame_or_backpressure(
                        &tx,
                        bytes,
                        stream_callback_backpressure_event(sequence, STREAM_CALLBACK_QUEUE_CAPACITY),
                    )
                    .await;
                    if !sent || terminal {
                        break;
                    }
                }
                Ok(None) => break,
                Err(status) => {
                    let bytes = stream_status_error_json(status, next_error_sequence).to_string().into_bytes();
                    let _ = tx.send(bytes).await;
                    break;
                }
            }
        }
    }
    let _ = remove_stream(stream_id);
}

#[cfg(feature = "axon-pb")]
async fn run_bidi_down_reader(
    bidi_id: InvocationBidiId,
    mut down: tonic::Streaming<easynet_axon::pb::axon::v1::InvokeBidiDown>,
    cancel: tokio_util::sync::CancellationToken,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let mut next_error_sequence = 1;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            message = down.message() => match message {
                Ok(Some(frame)) => {
                    let terminal = bidi_down_frame_is_terminal(&frame);
                    let sequence = frame.sequence;
                    next_error_sequence = sequence.saturating_add(1).max(1);
                    let bytes = bidi_down_frame_json(frame).to_string().into_bytes();
                    let sent = send_callback_frame_or_backpressure(
                        &tx,
                        bytes,
                        bidi_callback_backpressure_frame(sequence, BIDI_CALLBACK_QUEUE_CAPACITY),
                    )
                    .await;
                    if !sent || terminal {
                        break;
                    }
                }
                Ok(None) => break,
                Err(status) => {
                    let bytes =
                        stream_status_error_json(status, next_error_sequence).to_string().into_bytes();
                    let _ = tx.send(bytes).await;
                    break;
                }
            }
        }
    }
    let _ = remove_bidi(bidi_id);
}

#[cfg(feature = "axon-pb")]
async fn send_callback_frame_or_backpressure(
    tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    bytes: Vec<u8>,
    backpressure: serde_json::Value,
) -> bool {
    match tx.try_send(bytes) {
        Ok(()) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            let _ = tx.send(backpressure.to_string().into_bytes()).await;
            false
        }
    }
}

#[cfg(feature = "axon-pb")]
fn invocation_endpoint_for_session(
    session: &crate::ffi::client::handle::ClientSession,
) -> crate::daemon::Result<PathBuf> {
    if let Some(endpoint) = &session.invocation_endpoint {
        return Ok(PathBuf::from(endpoint));
    }
    Err(crate::daemon::DaemonError::InvocationEndpointMissing {
        control: PathBuf::from(&session.control_path),
    })
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct InvocationJson {
    caller_ura: String,
    callee_ura: String,
    descriptor_ref: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: easynet_axon::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
    metadata: std::collections::HashMap<String, String>,
    caller_signature: Option<easynet_axon::pb::axon::v1::CallerSignature>,
    bidi_streams: Vec<easynet_axon::pb::axon::v1::StreamDescriptor>,
    timeout_seconds: Option<i32>,
}

#[cfg(feature = "axon-pb")]
impl InvocationJson {
    fn parse(raw: &str) -> Result<Self, InvocationJsonError> {
        let value: serde_json::Value = serde_json::from_str(raw)?;
        let obj = value
            .as_object()
            .ok_or(InvocationJsonError::ExpectedObject)?;

        let caller_ura = required_string(obj, "caller_ura")?;
        let callee_ura = required_string(obj, "callee_ura")?;
        let descriptor_ref = required_string(obj, "descriptor_ref")?;
        let subject_ura = required_string(obj, "subject_ura")?;
        let nonce = decode_nonce(required_string(obj, "nonce_base64")?)?;
        let causal_context = parse_causal_context(
            obj.get("causal_context")
                .ok_or(InvocationJsonError::MissingField("causal_context"))?,
        )?;
        let (args, content_type) = parse_arguments(obj)?;
        let metadata = parse_metadata(obj)?;
        let caller_signature = parse_caller_signature(obj)?;
        let bidi_streams = parse_bidi_streams(obj)?;
        let timeout_seconds = parse_timeout_seconds(obj)?;

        Ok(Self {
            caller_ura,
            callee_ura,
            descriptor_ref,
            subject_ura,
            nonce,
            causal_context,
            args,
            content_type,
            metadata,
            caller_signature,
            bidi_streams,
            timeout_seconds,
        })
    }

    fn into_daemon_invocation(self) -> crate::daemon::Result<crate::daemon::DaemonInvocation> {
        let mut builder = crate::daemon::DaemonInvocation::builder(
            self.caller_ura,
            self.callee_ura,
            self.descriptor_ref,
            self.subject_ura,
        )?
        .nonce(self.nonce)
        .causal_context(self.causal_context)
        .args_bytes(self.args, self.content_type)?
        .metadata(self.metadata);
        if let Some(caller_signature) = self.caller_signature {
            builder = builder.caller_signature(caller_signature);
        }
        if let Some(seconds) = self.timeout_seconds {
            builder = builder.timeout_seconds(seconds)?;
        }
        let invocation = builder.build();
        Ok(invocation)
    }
}

/// Optional `timeout_seconds` field (F-045): positive integer,
/// forwarded to `InvokeRequest.timeout_seconds`. Absent/null = daemon
/// default.
#[cfg(feature = "axon-pb")]
fn parse_timeout_seconds(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<i32>, InvocationJsonError> {
    match obj.get("timeout_seconds") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|s| *s > 0 && *s <= i32::MAX as i64)
            .map(|s| Some(s as i32))
            .ok_or(InvocationJsonError::InvalidTimeoutSeconds),
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, thiserror::Error)]
enum InvocationJsonError {
    #[error("invocation_json must be a JSON object")]
    ExpectedObject,
    #[error("missing field `{0}`")]
    MissingField(&'static str),
    #[error("field `{0}` must be a non-empty string")]
    InvalidString(&'static str),
    #[error("nonce_base64 is not valid base64: {0}")]
    InvalidNonceBase64(base64::DecodeError),
    #[error("nonce_base64 must decode to exactly 16 bytes, got {0}")]
    InvalidNonceLength(usize),
    #[error("nonce_base64 must not be all-zero")]
    ZeroNonce,
    #[error("causal_context must be a JSON object")]
    InvalidCausalContext,
    #[error("timeout_seconds must be a positive integer within i32 range")]
    InvalidTimeoutSeconds,
    #[error("unsupported causal_context.form `{0}`")]
    UnsupportedCausalForm(String),
    #[error("{0} is not valid hex: {1}")]
    InvalidHex(&'static str, hex::FromHexError),
    #[error("{0} must decode to 32 bytes, got {1}")]
    InvalidHashLength(&'static str, usize),
    #[error("field `{0}` must be a non-empty array")]
    InvalidArray(&'static str),
    #[error("field `{0}` must be a JSON object")]
    InvalidObject(&'static str),
    #[error("field `{0}` must be a positive unsigned integer")]
    InvalidPositiveU64(&'static str),
    #[error("field `{0}` must be a boolean")]
    InvalidBool(&'static str),
    #[error("metadata values must be strings; invalid key `{0}`")]
    InvalidMetadataValue(String),
    #[error("caller_signature.signature_base64 is not valid base64: {0}")]
    InvalidSignatureBase64(base64::DecodeError),
    #[error("bidi_streams[{index}].stream_id must fit into u32")]
    InvalidStreamId { index: usize },
    #[error("bidi_streams[{index}].{field} must be a non-empty string")]
    InvalidStreamString { index: usize, field: &'static str },
    #[error("provide exactly one of `args` or `arguments_base64`")]
    AmbiguousArguments,
    #[error("arguments_base64 is not valid base64: {0}")]
    InvalidArgumentsBase64(base64::DecodeError),
    #[error("content_type is required when using arguments_base64")]
    MissingContentType,
    #[error("encode args JSON failed: {0}")]
    EncodeArgs(serde_json::Error),
    #[error("decode invocation_json failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
enum InvocationBuilderStringField {
    Caller,
    Callee,
    DescriptorRef,
    Subject,
    NonceBase64,
    CausalContextJson,
    ArgsJson,
    MetadataJson,
    IdempotencyKey,
    CallerSignatureJson,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
enum InvocationBuilderArgs {
    Json(serde_json::Value),
    Raw {
        bytes: Vec<u8>,
        content_type: String,
    },
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Default)]
struct InvocationBuilderState {
    caller_ura: Option<String>,
    callee_ura: Option<String>,
    descriptor_ref: Option<String>,
    subject_ura: Option<String>,
    nonce: Option<[u8; 16]>,
    causal_context: Option<easynet_axon::pb::axon::v1::CausalContext>,
    args: Option<InvocationBuilderArgs>,
    metadata: std::collections::HashMap<String, String>,
    caller_signature: Option<easynet_axon::pb::axon::v1::CallerSignature>,
    timeout_seconds: Option<i32>,
}

#[cfg(feature = "axon-pb")]
impl InvocationBuilderState {
    fn set_string_field(
        &mut self,
        field: InvocationBuilderStringField,
        raw: &str,
    ) -> Result<(), InvocationJsonError> {
        match field {
            InvocationBuilderStringField::Caller => {
                self.caller_ura = Some(non_empty_builder_string(raw, "caller_ura")?);
            }
            InvocationBuilderStringField::Callee => {
                self.callee_ura = Some(non_empty_builder_string(raw, "callee_ura")?);
            }
            InvocationBuilderStringField::DescriptorRef => {
                self.descriptor_ref = Some(non_empty_builder_string(raw, "descriptor_ref")?);
            }
            InvocationBuilderStringField::Subject => {
                self.subject_ura = Some(non_empty_builder_string(raw, "subject_ura")?);
            }
            InvocationBuilderStringField::NonceBase64 => {
                self.nonce = Some(decode_nonce(non_empty_builder_string(
                    raw,
                    "nonce_base64",
                )?)?);
            }
            InvocationBuilderStringField::CausalContextJson => {
                self.causal_context = Some(parse_causal_context_value(raw)?);
            }
            InvocationBuilderStringField::ArgsJson => {
                let value: serde_json::Value = serde_json::from_str(raw)?;
                self.args = Some(InvocationBuilderArgs::Json(value));
            }
            InvocationBuilderStringField::MetadataJson => {
                self.metadata = parse_metadata_value(raw)?;
            }
            InvocationBuilderStringField::IdempotencyKey => {
                self.metadata.insert(
                    "idempotency_key".to_string(),
                    non_empty_builder_string(raw, "idempotency_key")?,
                );
            }
            InvocationBuilderStringField::CallerSignatureJson => {
                self.caller_signature =
                    Some(SignatureMaterialJson::parse(raw)?.into_wire_signature());
            }
        }
        Ok(())
    }

    fn set_arguments_base64(
        &mut self,
        arguments_base64: &str,
        content_type: &str,
    ) -> Result<(), InvocationJsonError> {
        use base64::Engine;
        let content_type = non_empty_builder_string(content_type, "content_type")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(non_empty_builder_string(arguments_base64, "arguments_base64")?.as_bytes())
            .map_err(InvocationJsonError::InvalidArgumentsBase64)?;
        self.args = Some(InvocationBuilderArgs::Raw {
            bytes,
            content_type,
        });
        Ok(())
    }

    fn set_timeout_seconds(&mut self, timeout_seconds: u32) -> Result<(), InvocationJsonError> {
        if timeout_seconds == 0 || timeout_seconds > i32::MAX as u32 {
            return Err(InvocationJsonError::InvalidTimeoutSeconds);
        }
        self.timeout_seconds = Some(timeout_seconds as i32);
        Ok(())
    }

    fn build_invocation(&self) -> crate::daemon::Result<crate::daemon::DaemonInvocation> {
        let caller_ura = self
            .caller_ura
            .clone()
            .ok_or_else(|| missing_builder_field("caller_ura"))?;
        let callee_ura = self
            .callee_ura
            .clone()
            .ok_or_else(|| missing_builder_field("callee_ura"))?;
        let descriptor_ref = self
            .descriptor_ref
            .clone()
            .ok_or_else(|| missing_builder_field("descriptor_ref"))?;
        let subject_ura = self
            .subject_ura
            .clone()
            .ok_or_else(|| missing_builder_field("subject_ura"))?;
        let nonce = self
            .nonce
            .ok_or_else(|| missing_builder_field("nonce_base64"))?;
        let causal_context = self
            .causal_context
            .clone()
            .ok_or_else(|| missing_builder_field("causal_context"))?;
        let args = self
            .args
            .clone()
            .ok_or_else(|| missing_builder_field("args or arguments_base64"))?;

        let mut builder = crate::daemon::DaemonInvocation::builder(
            caller_ura,
            callee_ura,
            descriptor_ref,
            subject_ura,
        )?
        .nonce(nonce)
        .causal_context(causal_context)
        .metadata(self.metadata.clone());
        builder = match args {
            InvocationBuilderArgs::Json(value) => builder.args_json(&value)?,
            InvocationBuilderArgs::Raw {
                bytes,
                content_type,
            } => builder.args_bytes(bytes, content_type)?,
        };
        if let Some(signature) = self.caller_signature.clone() {
            builder = builder.caller_signature(signature);
        }
        if let Some(timeout_seconds) = self.timeout_seconds {
            builder = builder.timeout_seconds(timeout_seconds)?;
        }
        Ok(builder.build_draft()?.into_daemon_invocation())
    }
}

#[cfg(feature = "axon-pb")]
fn missing_builder_field(field: &'static str) -> crate::daemon::DaemonError {
    crate::daemon::DaemonError::InvalidInvocation(format!(
        "builder field `{field}` must be set before inspect, build, or prepare"
    ))
}

#[cfg(feature = "axon-pb")]
fn non_empty_builder_string(raw: &str, field: &'static str) -> Result<String, InvocationJsonError> {
    let value = raw.trim().to_string();
    if value.is_empty() {
        return Err(InvocationJsonError::InvalidString(field));
    }
    Ok(value)
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct PrepareOptionsJson {
    expires_in_ms: Option<u64>,
    signer_id: Option<String>,
    policy_ref: Option<String>,
    local_daemon_signing: bool,
}

#[cfg(feature = "axon-pb")]
impl PrepareOptionsJson {
    fn parse(raw: Option<&str>) -> Result<Self, InvocationJsonError> {
        let Some(raw) = raw else {
            return Ok(Self::default());
        };
        let value: serde_json::Value = serde_json::from_str(raw)?;
        if value.is_null() {
            return Ok(Self::default());
        }
        let obj = value
            .as_object()
            .ok_or(InvocationJsonError::InvalidObject("options_json"))?;
        let expires_in_ms = match obj.get("expires_in_ms") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .filter(|value| *value > 0)
                    .ok_or(InvocationJsonError::InvalidPositiveU64("expires_in_ms"))?,
            ),
        };
        let local_daemon_signing = match obj.get("local_daemon_signing") {
            None | Some(serde_json::Value::Null) => false,
            Some(value) => value
                .as_bool()
                .ok_or(InvocationJsonError::InvalidBool("local_daemon_signing"))?,
        };
        Ok(Self {
            expires_in_ms,
            signer_id: optional_string(obj, "signer_id")?,
            policy_ref: optional_string(obj, "policy_ref")?,
            local_daemon_signing,
        })
    }

    fn into_prepare_options(self) -> crate::daemon::PrepareOptions {
        crate::daemon::PrepareOptions {
            expires_in: std::time::Duration::from_millis(self.expires_in_ms.unwrap_or(300_000)),
            signer_id: self.signer_id,
            policy_ref: self.policy_ref,
            local_daemon_signing: self.local_daemon_signing,
        }
    }
}

#[cfg(feature = "axon-pb")]
impl Default for PrepareOptionsJson {
    fn default() -> Self {
        Self {
            expires_in_ms: None,
            signer_id: None,
            policy_ref: None,
            local_daemon_signing: false,
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct SignatureMaterialJson {
    algorithm: String,
    signature: Vec<u8>,
    key_id_hint: String,
}

#[cfg(feature = "axon-pb")]
impl SignatureMaterialJson {
    fn parse(raw: &str) -> Result<Self, InvocationJsonError> {
        use base64::Engine;
        let value: serde_json::Value = serde_json::from_str(raw)?;
        let obj = value
            .as_object()
            .ok_or(InvocationJsonError::InvalidObject("signature_json"))?;
        let algorithm = required_string(obj, "algorithm")?;
        let signature = base64::engine::general_purpose::STANDARD
            .decode(required_string(obj, "signature_base64")?.as_bytes())
            .map_err(InvocationJsonError::InvalidSignatureBase64)?;
        let key_id_hint = caller_signature_key_id_hint(obj)?;
        Ok(Self {
            algorithm,
            signature,
            key_id_hint,
        })
    }

    fn into_signature_material(self) -> crate::daemon::CallerSignatureMaterial {
        crate::daemon::CallerSignatureMaterial::new(
            self.algorithm,
            self.signature,
            self.key_id_hint,
        )
    }

    fn into_wire_signature(self) -> easynet_axon::pb::axon::v1::CallerSignature {
        easynet_axon::pb::axon::v1::CallerSignature {
            algorithm: self.algorithm,
            signature: self.signature,
            key_id_hint: self.key_id_hint,
        }
    }
}

#[cfg(feature = "axon-pb")]
struct BidiUpFrame {
    mac: Vec<u8>,
    payload: easynet_axon::pb::axon::v1::invoke_bidi_up::Payload,
}

#[cfg(feature = "axon-pb")]
fn bidi_eof_up_frame(sequence: u64) -> easynet_axon::pb::axon::v1::InvokeBidiUp {
    use easynet_axon::pb::axon::v1::{bidi_control, invoke_bidi_up, BidiControl, InvokeBidiUp};
    InvokeBidiUp {
        sequence,
        mac: Vec::new(),
        payload: Some(invoke_bidi_up::Payload::Control(BidiControl {
            control: Some(bidi_control::Control::Eof(true)),
        })),
    }
}

#[cfg(feature = "axon-pb")]
fn bidi_up_payload_is_eof(payload: &easynet_axon::pb::axon::v1::invoke_bidi_up::Payload) -> bool {
    use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload;
    matches!(payload, Payload::Control(control) if bidi_control_is_eof(control))
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, thiserror::Error)]
enum BidiFrameJsonError {
    #[error("frame_json must be a JSON object")]
    ExpectedObject,
    #[error("missing field `{0}`")]
    MissingField(&'static str),
    #[error("field `{0}` must be a non-empty string")]
    InvalidString(&'static str),
    #[error("unsupported frame type `{0}`")]
    UnsupportedType(String),
    #[error("field `{0}` must fit into u32")]
    InvalidU32(&'static str),
    #[error("field `{0}` must fit into i32")]
    InvalidI32(&'static str),
    #[error("field `{0}` must fit into u64")]
    InvalidU64(&'static str),
    #[error("field `control` must specify exactly one control variant")]
    InvalidControlVariant,
    #[error("field `{0}` must be a JSON object")]
    InvalidObject(&'static str),
    #[error("{0} is not valid base64: {1}")]
    InvalidBase64(&'static str, base64::DecodeError),
    #[error("decode frame_json failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(feature = "axon-pb")]
fn parse_bidi_up_frame_json(raw: &str) -> Result<BidiUpFrame, BidiFrameJsonError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let obj = value
        .as_object()
        .ok_or(BidiFrameJsonError::ExpectedObject)?;
    let kind = frame_required_string(obj, "type")?;
    let mac = match obj.get("mac_base64") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(value) => {
            use base64::Engine;
            let raw = value
                .as_str()
                .ok_or(BidiFrameJsonError::InvalidString("mac_base64"))?;
            base64::engine::general_purpose::STANDARD
                .decode(raw.as_bytes())
                .map_err(|err| BidiFrameJsonError::InvalidBase64("mac_base64", err))?
        }
    };
    let payload = match kind.as_str() {
        "binary_chunk" => {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD
                .decode(frame_required_string(obj, "data_base64")?.as_bytes())
                .map_err(|err| BidiFrameJsonError::InvalidBase64("data_base64", err))?;
            easynet_axon::pb::axon::v1::invoke_bidi_up::Payload::BinaryChunk(
                easynet_axon::pb::axon::v1::BinaryChunk {
                    stream_id: frame_u32(obj, "stream_id")?,
                    data,
                    pts: frame_optional_u64(obj, "pts")?.unwrap_or_default(),
                },
            )
        }
        "control" => {
            easynet_axon::pb::axon::v1::invoke_bidi_up::Payload::Control(parse_bidi_control(obj)?)
        }
        other => return Err(BidiFrameJsonError::UnsupportedType(other.to_string())),
    };
    Ok(BidiUpFrame { mac, payload })
}

#[cfg(feature = "axon-pb")]
fn parse_bidi_control(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<easynet_axon::pb::axon::v1::BidiControl, BidiFrameJsonError> {
    use easynet_axon::pb::axon::v1::{
        bidi_control, BidiControl, MediaTimestamp, PtyResize, PtySignal,
    };
    let mut controls = Vec::new();
    if let Some(value) = obj.get("eof").and_then(serde_json::Value::as_bool) {
        controls.push(bidi_control::Control::Eof(value));
    }
    if let Some(value) = obj.get("pty_resize") {
        let resize = value
            .as_object()
            .ok_or(BidiFrameJsonError::InvalidObject("pty_resize"))?;
        controls.push(bidi_control::Control::PtyResize(PtyResize {
            cols: frame_u32(resize, "cols")?,
            rows: frame_u32(resize, "rows")?,
        }));
    }
    if obj.contains_key("pty_signal") {
        controls.push(bidi_control::Control::PtySignal(PtySignal {
            signal: frame_i32(obj, "pty_signal")?,
        }));
    }
    if let Some(value) = obj.get("media_pts") {
        let media = value
            .as_object()
            .ok_or(BidiFrameJsonError::InvalidObject("media_pts"))?;
        controls.push(bidi_control::Control::MediaPts(MediaTimestamp {
            stream_id: frame_u32(media, "stream_id")?,
            pts: frame_u64(media, "pts")?,
        }));
    }
    if controls.len() != 1 {
        return Err(BidiFrameJsonError::InvalidControlVariant);
    }
    Ok(BidiControl {
        control: controls.pop(),
    })
}

#[cfg(feature = "axon-pb")]
fn frame_required_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<String, BidiFrameJsonError> {
    let value = obj
        .get(field)
        .ok_or(BidiFrameJsonError::MissingField(field))?
        .as_str()
        .ok_or(BidiFrameJsonError::InvalidString(field))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(BidiFrameJsonError::InvalidString(field));
    }
    Ok(value)
}

#[cfg(feature = "axon-pb")]
fn frame_u32(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u32, BidiFrameJsonError> {
    obj.get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(BidiFrameJsonError::InvalidU32(field))
}

#[cfg(feature = "axon-pb")]
fn frame_i32(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<i32, BidiFrameJsonError> {
    obj.get(field)
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(BidiFrameJsonError::InvalidI32(field))
}

#[cfg(feature = "axon-pb")]
fn frame_u64(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u64, BidiFrameJsonError> {
    obj.get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or(BidiFrameJsonError::InvalidU64(field))
}

#[cfg(feature = "axon-pb")]
fn frame_optional_u64(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<Option<u64>, BidiFrameJsonError> {
    match obj.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(_) => Ok(Some(frame_u64(obj, field)?)),
    }
}

#[cfg(feature = "axon-pb")]
fn required_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<String, InvocationJsonError> {
    let value = obj
        .get(field)
        .ok_or(InvocationJsonError::MissingField(field))?
        .as_str()
        .ok_or(InvocationJsonError::InvalidString(field))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(InvocationJsonError::InvalidString(field));
    }
    Ok(value)
}

#[cfg(feature = "axon-pb")]
fn optional_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<Option<String>, InvocationJsonError> {
    match obj.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let value = value
                .as_str()
                .ok_or(InvocationJsonError::InvalidString(field))?
                .trim()
                .to_string();
            if value.is_empty() {
                return Err(InvocationJsonError::InvalidString(field));
            }
            Ok(Some(value))
        }
    }
}

#[cfg(feature = "axon-pb")]
fn decode_nonce(raw: String) -> Result<[u8; 16], InvocationJsonError> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(raw.as_bytes())
        .map_err(InvocationJsonError::InvalidNonceBase64)?;
    if decoded.len() != 16 {
        return Err(InvocationJsonError::InvalidNonceLength(decoded.len()));
    }
    if decoded.iter().all(|byte| *byte == 0) {
        return Err(InvocationJsonError::ZeroNonce);
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&decoded);
    Ok(out)
}

#[cfg(feature = "axon-pb")]
fn parse_arguments(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(Vec<u8>, String), InvocationJsonError> {
    let has_args = obj.contains_key("args");
    let has_raw = obj.contains_key("arguments_base64");
    match (has_args, has_raw) {
        (true, true) | (false, false) => Err(InvocationJsonError::AmbiguousArguments),
        (true, false) => {
            let bytes = serde_json::to_vec(obj.get("args").expect("contains_key checked"))
                .map_err(InvocationJsonError::EncodeArgs)?;
            Ok((bytes, "application/json".to_string()))
        }
        (false, true) => {
            use base64::Engine;
            let content_type = optional_string(obj, "content_type")?
                .ok_or(InvocationJsonError::MissingContentType)?;
            let raw = required_string(obj, "arguments_base64")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(raw.as_bytes())
                .map_err(InvocationJsonError::InvalidArgumentsBase64)?;
            Ok((bytes, content_type))
        }
    }
}

#[cfg(feature = "axon-pb")]
fn parse_metadata(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<std::collections::HashMap<String, String>, InvocationJsonError> {
    let Some(value) = obj.get("metadata") else {
        return Ok(std::collections::HashMap::new());
    };
    if value.is_null() {
        return Ok(std::collections::HashMap::new());
    }
    let metadata = value
        .as_object()
        .ok_or(InvocationJsonError::InvalidObject("metadata"))?;
    let mut out = std::collections::HashMap::with_capacity(metadata.len());
    for (key, value) in metadata {
        if key.trim().is_empty() {
            return Err(InvocationJsonError::InvalidString("metadata key"));
        }
        let value = value
            .as_str()
            .ok_or_else(|| InvocationJsonError::InvalidMetadataValue(key.clone()))?
            .to_string();
        out.insert(key.trim().to_string(), value);
    }
    Ok(out)
}

#[cfg(feature = "axon-pb")]
fn parse_metadata_value(
    raw: &str,
) -> Result<std::collections::HashMap<String, String>, InvocationJsonError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let obj = value
        .as_object()
        .ok_or(InvocationJsonError::InvalidObject("metadata"))?;
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "metadata".to_string(),
        serde_json::Value::Object(obj.clone()),
    );
    parse_metadata(&envelope)
}

#[cfg(feature = "axon-pb")]
fn parse_caller_signature(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<easynet_axon::pb::axon::v1::CallerSignature>, InvocationJsonError> {
    use base64::Engine;
    let Some(value) = obj.get("caller_signature") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let signature_obj = value
        .as_object()
        .ok_or(InvocationJsonError::InvalidObject("caller_signature"))?;
    let algorithm = required_string(signature_obj, "algorithm")?;
    let signature = base64::engine::general_purpose::STANDARD
        .decode(required_string(signature_obj, "signature_base64")?.as_bytes())
        .map_err(InvocationJsonError::InvalidSignatureBase64)?;
    let key_id_hint = caller_signature_key_id_hint(signature_obj)?;
    Ok(Some(easynet_axon::pb::axon::v1::CallerSignature {
        algorithm,
        signature,
        key_id_hint,
    }))
}

#[cfg(feature = "axon-pb")]
fn caller_signature_key_id_hint(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, InvocationJsonError> {
    let key_id_hint = optional_string(obj, "key_id_hint")?.unwrap_or_default();
    if !key_id_hint.trim().is_empty() {
        return Ok(key_id_hint);
    }
    Ok(optional_string(obj, "signer_public_key_base64")?.unwrap_or_default())
}

#[cfg(feature = "axon-pb")]
fn parse_bidi_streams(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<easynet_axon::pb::axon::v1::StreamDescriptor>, InvocationJsonError> {
    let Some(value) = obj.get("bidi_streams") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or(InvocationJsonError::InvalidArray("bidi_streams"))?;
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item = item
            .as_object()
            .ok_or(InvocationJsonError::InvalidObject("bidi_streams[]"))?;
        let stream_id = item
            .get("stream_id")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(InvocationJsonError::InvalidStreamId { index })?;
        let content_type = stream_required_string(item, index, "content_type")?;
        let ordering = stream_required_string(item, index, "ordering")?;
        let codec_params = match item.get("codec_params") {
            None | Some(serde_json::Value::Null) => String::new(),
            Some(value) => value
                .as_str()
                .ok_or(InvocationJsonError::InvalidStreamString {
                    index,
                    field: "codec_params",
                })?
                .trim()
                .to_string(),
        };
        out.push(easynet_axon::pb::axon::v1::StreamDescriptor {
            stream_id,
            content_type,
            codec_params,
            ordering,
        });
    }
    Ok(out)
}

#[cfg(feature = "axon-pb")]
fn stream_required_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    index: usize,
    field: &'static str,
) -> Result<String, InvocationJsonError> {
    let value = obj
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(InvocationJsonError::InvalidStreamString { index, field })?;
    Ok(value.to_string())
}

#[cfg(feature = "axon-pb")]
fn parse_causal_context(
    value: &serde_json::Value,
) -> Result<easynet_axon::pb::axon::v1::CausalContext, InvocationJsonError> {
    use easynet_axon::pb::axon::v1 as pb;
    let obj = value
        .as_object()
        .ok_or(InvocationJsonError::InvalidCausalContext)?;
    let form = required_string(obj, "form")?;
    match form.as_str() {
        "none" => Ok(pb::CausalContext {
            form: Some(pb::causal_context::Form::None(pb::Empty {})),
        }),
        "scalar" => Ok(pb::CausalContext {
            form: Some(pb::causal_context::Form::Scalar(pb::ReceiptRef {
                receipt_hash: decode_hash(obj, "receipt_hash_hex")?.to_vec(),
                receipt_ura: required_string(obj, "receipt_ura")?,
            })),
        }),
        "list" => {
            let prior = obj
                .get("prior")
                .and_then(serde_json::Value::as_array)
                .filter(|items| !items.is_empty())
                .ok_or(InvocationJsonError::InvalidArray("prior"))?;
            let mut refs = Vec::with_capacity(prior.len());
            for item in prior {
                let item = item
                    .as_object()
                    .ok_or(InvocationJsonError::InvalidCausalContext)?;
                refs.push(pb::ReceiptRef {
                    receipt_hash: decode_hash(item, "receipt_hash_hex")?.to_vec(),
                    receipt_ura: required_string(item, "receipt_ura")?,
                });
            }
            Ok(pb::CausalContext {
                form: Some(pb::causal_context::Form::List(pb::ReceiptList {
                    prior: refs,
                })),
            })
        }
        "merkle" => Ok(pb::CausalContext {
            form: Some(pb::causal_context::Form::Merkle(pb::MerkleRoot {
                root: decode_hash(obj, "root_hex")?.to_vec(),
                proof_ura: required_string(obj, "proof_ura")?,
            })),
        }),
        other => Err(InvocationJsonError::UnsupportedCausalForm(
            other.to_string(),
        )),
    }
}

#[cfg(feature = "axon-pb")]
fn parse_causal_context_value(
    raw: &str,
) -> Result<easynet_axon::pb::axon::v1::CausalContext, InvocationJsonError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    parse_causal_context(&value)
}

#[cfg(feature = "axon-pb")]
fn decode_hash(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<[u8; 32], InvocationJsonError> {
    let raw = required_string(obj, field)?;
    let decoded =
        hex::decode(raw.trim()).map_err(|err| InvocationJsonError::InvalidHex(field, err))?;
    if decoded.len() != 32 {
        return Err(InvocationJsonError::InvalidHashLength(field, decoded.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

#[cfg(feature = "axon-pb")]
fn invocation_json(invocation: &crate::daemon::DaemonInvocation) -> serde_json::Value {
    use base64::Engine;
    let mut obj = serde_json::json!({
        "caller_ura": invocation.caller_ura(),
        "callee_ura": invocation.callee_ura(),
        "descriptor_ref": invocation.descriptor_ref(),
        "subject_ura": invocation.subject_ura(),
        "nonce_base64": base64::engine::general_purpose::STANDARD.encode(invocation.nonce()),
        "causal_context": causal_context_json(invocation.causal_context()),
        "content_type": invocation.content_type(),
        "metadata": invocation.metadata(),
    });
    if invocation.content_type() == "application/json" {
        match serde_json::from_slice::<serde_json::Value>(invocation.args()) {
            Ok(args) => {
                obj["args"] = args;
            }
            Err(_) => {
                obj["arguments_base64"] = serde_json::json!(
                    base64::engine::general_purpose::STANDARD.encode(invocation.args())
                );
            }
        }
    } else {
        obj["arguments_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(invocation.args()));
    }
    if let Some(signature) = invocation.caller_signature() {
        obj["caller_signature"] = serde_json::json!({
            "algorithm": signature.algorithm.as_str(),
            "signature_base64": base64::engine::general_purpose::STANDARD.encode(&signature.signature),
            "key_id_hint": signature.key_id_hint.as_str(),
        });
    }
    obj
}

#[cfg(feature = "axon-pb")]
fn causal_context_json(context: &easynet_axon::pb::axon::v1::CausalContext) -> serde_json::Value {
    use easynet_axon::pb::axon::v1::causal_context::Form;
    match context.form.as_ref() {
        Some(Form::None(_)) => serde_json::json!({"form": "none"}),
        Some(Form::Scalar(receipt)) => serde_json::json!({
            "form": "scalar",
            "receipt_hash_hex": hex::encode(&receipt.receipt_hash),
            "receipt_ura": receipt.receipt_ura,
        }),
        Some(Form::List(list)) => serde_json::json!({
            "form": "list",
            "prior": list.prior.iter().map(|receipt| serde_json::json!({
                "receipt_hash_hex": hex::encode(&receipt.receipt_hash),
                "receipt_ura": receipt.receipt_ura,
            })).collect::<Vec<_>>(),
        }),
        Some(Form::Merkle(root)) => serde_json::json!({
            "form": "merkle",
            "root_hex": hex::encode(&root.root),
            "proof_ura": root.proof_ura,
        }),
        None => serde_json::json!({"form": "invalid"}),
    }
}

#[cfg(feature = "axon-pb")]
fn prepared_invocation_json(prepared: &crate::daemon::PreparedInvocation) -> serde_json::Value {
    let material = prepared.signing_material();
    let policy = material.signer_policy();
    serde_json::json!({
        "request_id": prepared.request_id(),
        "descriptor_ref": prepared.descriptor_ref(),
        "descriptor_hash_hex": prepared.descriptor_hash_hex(),
        "schema_hash_hex": prepared.schema_hash_hex(),
        "canonical_hash_hex": prepared.canonical_hash_hex(),
        "expires_at_unix_ms": prepared.expires_at_unix_ms(),
        "tuple": invocation_json(prepared.draft().invocation()),
        "signing_material": {
            "algorithm": "ed25519",
            "canonical_bytes_base64": material.canonical_bytes_base64(),
            "args_digest_hex": material.args_digest_hex(),
            "descriptor_ref": prepared.descriptor_ref(),
            "expires_at_unix_ms": prepared.expires_at_unix_ms(),
            "nonce_base64": material.nonce_base64(),
            "signed_fields": material.signed_fields(),
            "signer_policy": {
                "mode": policy.mode.as_str(),
                "signer_id": policy.signer_id.as_str(),
                "policy_ref": policy.policy_ref.as_str(),
                "expires_at_unix_ms": policy.expires_at_unix_ms,
            }
        }
    })
}

#[cfg(feature = "axon-pb")]
fn signed_invocation_json(signed: &crate::daemon::SignedInvocation) -> serde_json::Value {
    use base64::Engine;
    let policy = signed.policy();
    serde_json::json!({
        "signer_id": signed.signer_id(),
        "prepared": {
            "request_id": signed.prepared().request_id(),
            "descriptor_ref": signed.prepared().descriptor_ref(),
            "canonical_hash_hex": signed.prepared().canonical_hash_hex(),
            "expires_at_unix_ms": signed.prepared().expires_at_unix_ms(),
        },
        "policy": {
            "mode": policy.mode.as_str(),
            "signer_id": policy.signer_id.as_str(),
            "policy_ref": policy.policy_ref.as_str(),
            "expires_at_unix_ms": policy.expires_at_unix_ms,
        },
        "signature": {
            "algorithm": signed.signature().algorithm.as_str(),
            "signature_base64": base64::engine::general_purpose::STANDARD.encode(&signed.signature().signature),
            "key_id_hint": signed.signature().key_id_hint.as_str(),
        }
    })
}

#[cfg(feature = "axon-pb")]
fn invocation_result_from_invoke_response(
    tuple: crate::daemon::InvocationTuple,
    response: easynet_axon::pb::axon::v1::InvokeResponse,
) -> crate::daemon::InvocationResult {
    crate::daemon::InvocationResult {
        tuple,
        terminal_state: axon_state_name_from_i32(response.state),
        output_content_type: response.result_content_type,
        output: response.result,
        selected_node_id: response.selected_node_id,
        scheduling_reason: response.scheduling_reason,
        elapsed_ms: response.elapsed_ms.max(0) as u64,
        receipt: response
            .admission_receipt
            .as_ref()
            .map(crate::daemon::ReceiptSummary::from_wire),
        error: response
            .error
            .as_ref()
            .map(crate::daemon::RuntimeErrorSummary::from_wire),
    }
}

#[cfg(feature = "axon-pb")]
fn invocation_result_json_with_tuple(
    result: crate::daemon::InvocationResult,
    tuple_json: serde_json::Value,
) -> serde_json::Value {
    use base64::Engine;
    let output_json = if result.output_content_type == "application/json" {
        serde_json::from_slice::<serde_json::Value>(&result.output).ok()
    } else {
        None
    };
    serde_json::json!({
        "ok": result.error.is_none(),
        "tuple": tuple_json,
        "terminal_state": result.terminal_state,
        "output_content_type": result.output_content_type,
        "output_base64": base64::engine::general_purpose::STANDARD.encode(&result.output),
        "output_json": output_json,
        "selected_node_id": result.selected_node_id,
        "scheduling_reason": result.scheduling_reason,
        "elapsed_ms": result.elapsed_ms,
        "receipt": result.receipt.map(receipt_summary_dto_json),
        "error": result.error.map(runtime_error_json),
    })
}

#[cfg(feature = "axon-pb")]
fn invocation_cancelled_result(
    tuple: &crate::daemon::InvocationTuple,
    reason: Option<&str>,
) -> crate::daemon::InvocationResult {
    crate::daemon::InvocationResult {
        tuple: tuple.clone(),
        terminal_state: axon_state_wire_string(
            easynet_axon::invocation::InvocationState::Cancelled,
        ),
        output_content_type: "application/json".to_string(),
        output: b"{}".to_vec(),
        selected_node_id: String::new(),
        scheduling_reason: "ffi_invocation_handle_cancel".to_string(),
        elapsed_ms: 0,
        receipt: None,
        error: Some(crate::daemon::RuntimeErrorSummary {
            code: "CANCELLED".to_string(),
            stage: "client".to_string(),
            message: reason.unwrap_or("invocation handle cancelled").to_string(),
            retryable: false,
        }),
    }
}

#[cfg(feature = "axon-pb")]
fn invocation_failed_result(
    tuple: &crate::daemon::InvocationTuple,
    err: crate::daemon::DaemonError,
) -> crate::daemon::InvocationResult {
    let phase = daemon_error_terminal_phase(&err);
    crate::daemon::InvocationResult {
        tuple: tuple.clone(),
        terminal_state: phase.to_axon_wire_state_string(),
        output_content_type: "application/json".to_string(),
        output: b"{}".to_vec(),
        selected_node_id: String::new(),
        scheduling_reason: "ffi_invocation_handle_terminal_error".to_string(),
        elapsed_ms: 0,
        receipt: None,
        error: Some(runtime_error_summary_for_daemon_error(&err)),
    }
}

#[cfg(feature = "axon-pb")]
fn runtime_error_summary_for_daemon_error(
    err: &crate::daemon::DaemonError,
) -> crate::daemon::RuntimeErrorSummary {
    let (code, stage, retryable) = match err {
        crate::daemon::DaemonError::InvocationEndpointDown { .. }
        | crate::daemon::DaemonError::InvocationEndpointMissing { .. }
        | crate::daemon::DaemonError::Connect { .. } => ("DAEMON_OFFLINE", "transport", true),
        crate::daemon::DaemonError::InvokeStatus { code, .. }
        | crate::daemon::DaemonError::InvokeStreamStatus { code, .. }
        | crate::daemon::DaemonError::InvokeBidiStatus { code, .. } => (
            tonic_code_to_runtime_error_code(*code),
            "runtime",
            tonic_code_retryable(*code),
        ),
        crate::daemon::DaemonError::InvalidInvocation(_) => ("INVALID_INVOCATION", "sdk", false),
        crate::daemon::DaemonError::InvokeBidiClosed { .. } => ("CANCELLED", "runtime", false),
        _ => ("RUNTIME_ERROR", "runtime", false),
    };
    crate::daemon::RuntimeErrorSummary {
        code: code.to_string(),
        stage: stage.to_string(),
        message: err.to_string(),
        retryable,
    }
}

#[cfg(feature = "axon-pb")]
fn sync_submit_error_code_for_result(result: &crate::daemon::InvocationResult) -> Option<i32> {
    let error = result.error.as_ref()?;
    match error.code.as_str() {
        "DAEMON_OFFLINE" | "DAEMON_DOWN" => Some(ERR_DAEMON_DOWN),
        "CANCELLED" => Some(ERR_CANCELLED),
        "DEADLINE_EXCEEDED" => Some(ERR_TIMEOUT),
        "TIMEOUT" => Some(ERR_TIMEOUT),
        "INVALID_INVOCATION" => Some(ERR_INVALID_ARG),
        "ADMISSION_DENIED" => Some(ERR_ABILITY_FAILED),
        "PERMISSION_DENIED" | "UNAUTHENTICATED" => Some(ERR_PERMISSION_DENIED),
        "ABILITY_NOT_FOUND" | "NOT_FOUND" => Some(ERR_NOT_FOUND),
        "UNIMPLEMENTED" => Some(ERR_NOT_IMPLEMENTED),
        "PROTOCOL_MISMATCH" | "UNKNOWN" | "INTERNAL" | "DATA_LOSS" => Some(ERR_PROTOCOL),
        _ if error.stage == "transport" => Some(ERR_DAEMON_DOWN),
        _ => None,
    }
}

#[cfg(feature = "axon-pb")]
fn record_sync_submit_terminal_error(result: &crate::daemon::InvocationResult) -> Option<i32> {
    let code = sync_submit_error_code_for_result(result)?;
    let message = result
        .error
        .as_ref()
        .map(|err| err.message.as_str())
        .unwrap_or("submitted invocation failed before terminal result");
    Some(record_invocation_error(
        code,
        format!("easynet_invocation_submit_signed: {message}"),
    ))
}

#[cfg(feature = "axon-pb")]
fn daemon_error_terminal_phase(err: &crate::daemon::DaemonError) -> InvocationHandlePhase {
    match err {
        crate::daemon::DaemonError::InvokeStatus { code, .. }
        | crate::daemon::DaemonError::InvokeStreamStatus { code, .. }
        | crate::daemon::DaemonError::InvokeBidiStatus { code, .. } => match code {
            tonic::Code::DeadlineExceeded => InvocationHandlePhase::TimedOut,
            tonic::Code::Cancelled => InvocationHandlePhase::Cancelled,
            _ => InvocationHandlePhase::Failed,
        },
        crate::daemon::DaemonError::InvokeBidiClosed { .. } => InvocationHandlePhase::Cancelled,
        _ => InvocationHandlePhase::Failed,
    }
}

#[cfg(feature = "axon-pb")]
fn terminal_phase_for_result(result: &crate::daemon::InvocationResult) -> InvocationHandlePhase {
    if result.terminal_state
        == axon_state_wire_string(easynet_axon::invocation::InvocationState::TimedOut)
        || result.terminal_state.eq_ignore_ascii_case("TimedOut")
        || result.terminal_state.eq_ignore_ascii_case("TIMED_OUT")
    {
        return InvocationHandlePhase::TimedOut;
    }
    if result.terminal_state
        == axon_state_wire_string(easynet_axon::invocation::InvocationState::Cancelled)
        || result.terminal_state.eq_ignore_ascii_case("Cancelled")
        || result.terminal_state.eq_ignore_ascii_case("CANCELLED")
    {
        return InvocationHandlePhase::Cancelled;
    }
    if result.error.is_none()
        && (result.terminal_state
            == axon_state_wire_string(easynet_axon::invocation::InvocationState::Completed)
            || result.terminal_state.eq_ignore_ascii_case("Completed")
            || result.terminal_state.eq_ignore_ascii_case("COMPLETED"))
    {
        return InvocationHandlePhase::Completed;
    }
    if result.error.is_none() {
        InvocationHandlePhase::Completed
    } else {
        InvocationHandlePhase::Failed
    }
}

#[cfg(feature = "axon-pb")]
fn axon_state_wire_string(state: easynet_axon::invocation::InvocationState) -> String {
    axon_state_name_from_i32(state.to_wire_i32())
}

#[cfg(feature = "axon-pb")]
fn axon_state_name_from_i32(state: i32) -> String {
    if state == easynet_axon::invocation::InvocationState::Accepted.to_wire_i32() {
        "Accepted".to_string()
    } else if state == easynet_axon::invocation::InvocationState::Admitted.to_wire_i32() {
        "Admitted".to_string()
    } else if state == easynet_axon::invocation::InvocationState::Dispatched.to_wire_i32() {
        "Dispatched".to_string()
    } else if state == easynet_axon::invocation::InvocationState::Running.to_wire_i32() {
        "Running".to_string()
    } else if state == easynet_axon::invocation::InvocationState::Completed.to_wire_i32() {
        "Completed".to_string()
    } else if state == easynet_axon::invocation::InvocationState::Failed.to_wire_i32() {
        "Failed".to_string()
    } else if state == easynet_axon::invocation::InvocationState::TimedOut.to_wire_i32() {
        "TimedOut".to_string()
    } else if state == easynet_axon::invocation::InvocationState::Cancelled.to_wire_i32() {
        "Cancelled".to_string()
    } else {
        state.to_string()
    }
}

#[cfg(feature = "axon-pb")]
fn tonic_code_to_runtime_error_code(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::Ok => "OK",
        tonic::Code::Cancelled => "CANCELLED",
        tonic::Code::InvalidArgument | tonic::Code::OutOfRange => "INVALID_INVOCATION",
        tonic::Code::DeadlineExceeded => "TIMEOUT",
        tonic::Code::NotFound => "ABILITY_NOT_FOUND",
        tonic::Code::AlreadyExists
        | tonic::Code::FailedPrecondition
        | tonic::Code::Aborted
        | tonic::Code::ResourceExhausted => "ADMISSION_DENIED",
        tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => "PERMISSION_DENIED",
        tonic::Code::Unimplemented
        | tonic::Code::Unknown
        | tonic::Code::Internal
        | tonic::Code::DataLoss => "PROTOCOL_MISMATCH",
        tonic::Code::Unavailable => "DAEMON_OFFLINE",
    }
}

#[cfg(feature = "axon-pb")]
fn tonic_code_retryable(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Cancelled
            | tonic::Code::DeadlineExceeded
            | tonic::Code::ResourceExhausted
            | tonic::Code::Unavailable
    )
}

#[cfg(feature = "axon-pb")]
fn receipt_summary_dto_json(receipt: crate::daemon::ReceiptSummary) -> serde_json::Value {
    serde_json::json!({
        "index": receipt.index,
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": receipt.state,
        "timestamp_unix_ms": receipt.timestamp_unix_ms,
        "prev_receipt_hash_hex": receipt.prev_receipt_hash_hex,
        "self_hash_hex": receipt.self_hash_hex,
        "payload_content_type": receipt.payload_content_type,
        "cleanup_complete": receipt.cleanup_complete,
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
    })
}

#[cfg(feature = "axon-pb")]
fn runtime_error_json(error: crate::daemon::RuntimeErrorSummary) -> serde_json::Value {
    serde_json::json!({
        "code": error.code,
        "stage": error.stage,
        "message": error.message,
        "retryable": error.retryable,
    })
}

#[cfg(feature = "axon-pb")]
fn stream_chunk_json(chunk: easynet_axon::pb::axon::v1::InvokeStreamChunk) -> serde_json::Value {
    use base64::Engine;
    let payload_base64 = base64::engine::general_purpose::STANDARD.encode(&chunk.payload);
    let payload_json = if chunk.content_type == "application/json" {
        serde_json::from_slice::<serde_json::Value>(&chunk.payload).ok()
    } else {
        None
    };
    serde_json::json!({
        "ok": chunk.error.is_none(),
        "event": "chunk",
        "invocation_id": chunk.invocation_id,
        "selected_node_id": chunk.selected_node_id,
        "scheduling_reason": chunk.scheduling_reason,
        "state": chunk.state,
        "sequence": chunk.sequence,
        "terminal": chunk.terminal,
        "elapsed_ms": chunk.elapsed_ms,
        "content_type": chunk.content_type,
        "payload_base64": payload_base64,
        "payload_json": payload_json,
        "error": chunk.error.as_ref().map(protocol_error_json),
    })
}

#[cfg(feature = "axon-pb")]
fn stream_status_error_json(status: tonic::Status, sequence: u64) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "event": "error",
        "sequence": sequence.max(1),
        "code": format!("{:?}", status.code()),
        "message": status.message(),
        "terminal": true,
    })
}

#[cfg(feature = "axon-pb")]
fn bidi_down_frame_json(frame: easynet_axon::pb::axon::v1::InvokeBidiDown) -> serde_json::Value {
    use base64::Engine;
    use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
    let mac_base64 = base64::engine::general_purpose::STANDARD.encode(&frame.mac);
    match frame.payload {
        Some(Payload::Receipt(receipt)) => serde_json::json!({
            "ok": true,
            "event": "receipt",
            "sequence": frame.sequence,
            "mac_base64": mac_base64,
            "receipt": receipt_summary_json(&receipt),
            "terminal": receipt.cleanup_complete,
        }),
        Some(Payload::BinaryChunk(chunk)) => {
            let data_base64 = base64::engine::general_purpose::STANDARD.encode(&chunk.data);
            serde_json::json!({
                "ok": true,
                "event": "binary_chunk",
                "sequence": frame.sequence,
                "mac_base64": mac_base64,
                "stream_id": chunk.stream_id,
                "data_base64": data_base64,
                "pts": chunk.pts,
                "terminal": false,
            })
        }
        Some(Payload::Control(control)) => {
            let terminal = bidi_control_is_eof(&control);
            serde_json::json!({
                "ok": true,
                "event": "control",
                "sequence": frame.sequence,
                "mac_base64": mac_base64,
                "control": bidi_control_json(control),
                "terminal": terminal,
            })
        }
        // Carrier-v1 frames (DEC-F004): the FFI JSON projection learns
        // these shapes when dual-read lands (T2.1 steps 2-3); until
        // then they surface as an explicit unsupported event.
        Some(Payload::DispatchCall(_)) | Some(Payload::ReverseDispatchResult(_)) => {
            serde_json::json!({
                "ok": false,
                "event": "unsupported_frame",
                "sequence": frame.sequence,
                "mac_base64": mac_base64,
                "message": "carrier-v1 dispatch frame before dual-read support",
                "terminal": false,
            })
        }
        None => serde_json::json!({
            "ok": false,
            "event": "unknown",
            "sequence": frame.sequence,
            "mac_base64": mac_base64,
            "message": "InvokeBidiDown frame has no payload",
            "terminal": false,
        }),
    }
}

#[cfg(feature = "axon-pb")]
fn bidi_down_frame_is_terminal(frame: &easynet_axon::pb::axon::v1::InvokeBidiDown) -> bool {
    use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
    match frame.payload.as_ref() {
        Some(Payload::Receipt(receipt)) => receipt.cleanup_complete,
        Some(Payload::Control(control)) => bidi_control_is_eof(control),
        _ => false,
    }
}

#[cfg(feature = "axon-pb")]
fn bidi_control_is_eof(control: &easynet_axon::pb::axon::v1::BidiControl) -> bool {
    matches!(
        control.control,
        Some(easynet_axon::pb::axon::v1::bidi_control::Control::Eof(true))
    )
}

#[cfg(feature = "axon-pb")]
fn bidi_control_json(control: easynet_axon::pb::axon::v1::BidiControl) -> serde_json::Value {
    use easynet_axon::pb::axon::v1::bidi_control::Control;
    match control.control {
        Some(Control::PtyResize(resize)) => serde_json::json!({
            "type": "pty_resize",
            "cols": resize.cols,
            "rows": resize.rows,
        }),
        Some(Control::PtySignal(signal)) => serde_json::json!({
            "type": "pty_signal",
            "signal": signal.signal,
        }),
        Some(Control::MediaPts(media)) => serde_json::json!({
            "type": "media_pts",
            "stream_id": media.stream_id,
            "pts": media.pts,
        }),
        Some(Control::Eof(value)) => serde_json::json!({
            "type": "eof",
            "eof": value,
        }),
        None => serde_json::json!({
            "type": "empty",
        }),
    }
}

#[cfg(feature = "axon-pb")]
fn receipt_summary_json(
    receipt: &easynet_axon::pb::axon::v1::InvocationReceipt,
) -> serde_json::Value {
    serde_json::json!({
        "index": receipt.index,
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": receipt.state,
        "timestamp_unix_ms": receipt.timestamp_unix_ms,
        "prev_receipt_hash_hex": hex::encode(&receipt.prev_receipt_hash),
        "self_hash_hex": hex::encode(&receipt.self_hash),
        "payload_content_type": receipt.payload_content_type,
        "cleanup_complete": receipt.cleanup_complete,
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
    })
}

#[cfg(feature = "axon-pb")]
fn protocol_error_json(error: &easynet_axon::pb::axon::v1::Error) -> serde_json::Value {
    serde_json::json!({
        "code": error.code,
        "message": error.message,
        "retryable": error.retryable,
        "context": error.context,
    })
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, test_session};
    use std::ffi::{c_void, CStr, CString};

    unsafe extern "C" fn ignore_stream_chunk(_: *mut c_void, _: *const c_char) {}
    unsafe extern "C" fn ignore_bidi_frame(_: *mut c_void, _: *const c_char) {}

    fn descriptor_ref(owner_ura: &str, public_name: &str, version: &str) -> String {
        format!(
            "{}@{}",
            crate::core::ura::owner_ability_ura(owner_ura, public_name).unwrap(),
            version
        )
    }

    fn valid_invocation_json() -> CString {
        let callee_ura = "easynet:///r/acme/device/dev-a";
        let descriptor_ref = descriptor_ref(callee_ura, "observe.health", "2.4.0");
        CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": callee_ura,
                "descriptor_ref": descriptor_ref,
                "subject_ura": "easynet:///r/acme/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "args": {"ping": true}
            })
            .to_string(),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn with_test_key_service<F>(expected_connections: usize, f: F)
    where
        F: FnOnce(crate::daemon::keyring::ManagedSigningKeyProjection),
    {
        struct EnvRestore {
            socket: Option<std::ffi::OsString>,
        }

        impl Drop for EnvRestore {
            fn drop(&mut self) {
                match self.socket.take() {
                    Some(value) => std::env::set_var("EASYNET_KEYRING_SOCKET_PATH", value),
                    None => std::env::remove_var("EASYNET_KEYRING_SOCKET_PATH"),
                }
            }
        }

        let _guard = crate::cli::commands::test_support::env_lock();
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("key-service.sock");
        let _restore = EnvRestore {
            socket: std::env::var_os("EASYNET_KEYRING_SOCKET_PATH"),
        };
        std::env::set_var("EASYNET_KEYRING_SOCKET_PATH", &socket);
        let caller = "easynet:///r/acme/device/dev-a";

        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let socket_path = socket.clone();
        let vault_path = temp.path().join("key-service.enc");
        let server = std::thread::spawn(move || {
            crate::daemon::keyring::service::run_test_unix_key_service(
                socket_path,
                vault_path,
                "test-passphrase".to_string(),
                caller.to_string(),
                expected_connections,
                ready_tx,
            );
        });
        let entry = ready_rx
            .recv()
            .expect("test key service must report readiness")
            .expect("test key service must start");
        f(entry);
        server.join().unwrap();
    }

    fn valid_bidi_invocation_json() -> CString {
        let callee_ura = "easynet:///r/acme/device/dev-a";
        let descriptor_ref = descriptor_ref(callee_ura, "device.pty.attach", "2.4.0");
        CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": callee_ura,
                "descriptor_ref": descriptor_ref,
                "subject_ura": "easynet:///r/acme/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "args": {"session_id": "pty-1"},
                "metadata": {"x-easynet-delegation": "producer"},
                "caller_signature": {
                    "algorithm": "ed25519",
                    "signature_base64": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==",
                    "key_id_hint": "caller-key"
                },
                "bidi_streams": [{
                    "stream_id": 1,
                    "content_type": "text/pty",
                    "codec_params": "raw",
                    "ordering": "STRICT"
                }]
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn parse_invocation_json_requires_complete_axiom_fields() {
        let err = InvocationJson::parse(
            r#"{
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": "easynet:///r/acme/device/dev-a",
                "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0",
                "subject_ura": "easynet:///r/acme/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "args": {}
            }"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("causal_context"),
            "missing causal_context must be reported explicitly: {err}"
        );
    }

    /// Canonical URA invocation JSON for tests that go past parse into
    /// `into_daemon_invocation`.
    fn canonical_invocation_json(extra: serde_json::Value) -> String {
        let callee_ura = "easynet:///r/acme/device/dev-a";
        let mut obj = serde_json::json!({
            "caller_ura": "easynet:///r/acme/device/dev-a",
            "callee_ura": callee_ura,
            "descriptor_ref": descriptor_ref(callee_ura, "observe.health", "2.4.0"),
            "subject_ura": "easynet:///r/acme/device/dev-a",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "args": {}
        });
        if let (Some(base), Some(more)) = (obj.as_object_mut(), extra.as_object()) {
            for (k, v) in more {
                base.insert(k.clone(), v.clone());
            }
        }
        obj.to_string()
    }

    fn signature_json() -> CString {
        CString::new(
            serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6eg==",
                "key_id_hint": "caller-key"
            })
            .to_string(),
        )
        .unwrap()
    }

    fn new_signed_invocation_id() -> SignedInvocationId {
        let raw = CString::new(canonical_invocation_json(serde_json::json!({
            "args": {"probe": true}
        })))
        .unwrap();
        let (prepare_handle, _) = alloc(test_session());
        let mut prepared_id: PreparedInvocationId = 0;
        let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
        let prepare_code = unsafe {
            easynet_invocation_prepare(
                prepare_handle,
                raw.as_ptr(),
                std::ptr::null(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };
        assert_eq!(prepare_code, EASYNET_OK);
        unsafe { crate::ffi::strings::easynet_string_free(prepared_json_ptr) };

        let signature = signature_json();
        let mut signed_id: SignedInvocationId = 0;
        let mut signed_json_ptr: *mut c_char = std::ptr::null_mut();
        let sign_code = unsafe {
            easynet_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut signed_id,
                &mut signed_json_ptr,
            )
        };
        assert_eq!(sign_code, EASYNET_OK);
        unsafe { crate::ffi::strings::easynet_string_free(signed_json_ptr) };
        crate::ffi::client::handle::release(prepare_handle);
        signed_id
    }

    fn signed_fixture_tuple() -> crate::daemon::InvocationTuple {
        let signed_id = new_signed_invocation_id();
        let tuple = get_signed(signed_id).unwrap().prepared().tuple();
        assert_eq!(easynet_signed_invocation_free(signed_id), EASYNET_OK);
        tuple
    }

    fn completed_result_for_tuple(
        tuple: crate::daemon::InvocationTuple,
    ) -> crate::daemon::InvocationResult {
        crate::daemon::InvocationResult {
            tuple,
            terminal_state: axon_state_wire_string(
                easynet_axon::invocation::InvocationState::Completed,
            ),
            output_content_type: "application/json".to_string(),
            output: br#"{"ok":true}"#.to_vec(),
            selected_node_id: "local".to_string(),
            scheduling_reason: "test".to_string(),
            elapsed_ms: 1,
            receipt: None,
            error: None,
        }
    }

    fn active_bidi_session(
        owner: EasynetHandle,
        capacity: usize,
    ) -> (
        ActiveInvocationBidi,
        tokio::sync::mpsc::Receiver<easynet_axon::pb::axon::v1::InvokeBidiUp>,
        tokio_util::sync::CancellationToken,
    ) {
        let (up_tx, up_rx) = tokio::sync::mpsc::channel(capacity);
        let cancel = tokio_util::sync::CancellationToken::new();
        (
            ActiveInvocationBidi::new(
                owner,
                "device.pty.attach".to_string(),
                up_tx,
                cancel.clone(),
            ),
            up_rx,
            cancel,
        )
    }

    fn assert_bidi_eof_frame(frame: easynet_axon::pb::axon::v1::InvokeBidiUp, sequence: u64) {
        use easynet_axon::pb::axon::v1::{bidi_control, invoke_bidi_up};
        assert_eq!(frame.sequence, sequence);
        assert!(frame.mac.is_empty());
        match frame.payload {
            Some(invoke_bidi_up::Payload::Control(control)) => {
                assert!(matches!(
                    control.control,
                    Some(bidi_control::Control::Eof(true))
                ));
            }
            other => panic!("expected EOF control frame, got {other:?}"),
        }
    }

    fn read_last_error_json() -> serde_json::Value {
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { crate::ffi::errors::easynet_last_error_json(&mut out) };
        assert_eq!(code, EASYNET_OK);
        assert!(!out.is_null());
        let value = unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(out) };
        value
    }

    fn assert_typed_last_error(code_name: &str, abi_code: i32, message_fragment: &str) {
        let error = read_last_error_json();
        assert_eq!(error["code"], code_name);
        assert_eq!(error["details"]["abi_code"], abi_code);
        assert_eq!(error["details"]["legacy_untyped"], false);
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| message.contains(message_fragment)),
            "last-error message should contain {message_fragment:?}: {error}"
        );
    }

    fn new_builder_handle() -> InvocationBuilderId {
        let mut builder_id: InvocationBuilderId = 0;
        let code = unsafe { easynet_invocation_builder_new(&mut builder_id) };
        assert_eq!(code, EASYNET_OK);
        assert_ne!(builder_id, 0);
        builder_id
    }

    fn set_complete_builder(builder_id: InvocationBuilderId) {
        let callee_ura = CString::new("easynet:///r/acme/device/dev-a").unwrap();
        let caller_ura = CString::new("easynet:///r/acme/device/dev-a").unwrap();
        let descriptor = CString::new(descriptor_ref(
            "easynet:///r/acme/device/dev-a",
            "observe.health",
            "2.4.0",
        ))
        .unwrap();
        let subject = CString::new("easynet:///r/acme/device/dev-a").unwrap();
        let nonce = CString::new("AQIDBAUGBwgJCgsMDQ4PEA==").unwrap();
        let causal = CString::new(serde_json::json!({"form": "none"}).to_string()).unwrap();
        let args = CString::new(serde_json::json!({"probe": true}).to_string()).unwrap();
        let metadata =
            CString::new(serde_json::json!({"trace": "sdk-builder"}).to_string()).unwrap();
        let idempotency_key = CString::new("idem-1").unwrap();

        assert_eq!(
            unsafe { easynet_invocation_builder_set_caller(builder_id, caller_ura.as_ptr()) },
            EASYNET_OK
        );
        assert_eq!(
            unsafe { easynet_invocation_builder_set_callee(builder_id, callee_ura.as_ptr()) },
            EASYNET_OK
        );
        assert_eq!(
            unsafe {
                easynet_invocation_builder_set_descriptor_ref(builder_id, descriptor.as_ptr())
            },
            EASYNET_OK
        );
        assert_eq!(
            unsafe { easynet_invocation_builder_set_subject(builder_id, subject.as_ptr()) },
            EASYNET_OK
        );
        assert_eq!(
            unsafe { easynet_invocation_builder_set_nonce_base64(builder_id, nonce.as_ptr()) },
            EASYNET_OK
        );
        assert_eq!(
            unsafe {
                easynet_invocation_builder_set_causal_context_json(builder_id, causal.as_ptr())
            },
            EASYNET_OK
        );
        assert_eq!(
            unsafe { easynet_invocation_builder_set_args_json(builder_id, args.as_ptr()) },
            EASYNET_OK
        );
        assert_eq!(
            unsafe { easynet_invocation_builder_set_metadata_json(builder_id, metadata.as_ptr()) },
            EASYNET_OK
        );
        assert_eq!(
            unsafe {
                easynet_invocation_builder_set_idempotency_key(builder_id, idempotency_key.as_ptr())
            },
            EASYNET_OK
        );
        assert_eq!(
            easynet_invocation_builder_set_timeout_seconds(builder_id, 45),
            EASYNET_OK
        );
    }

    #[test]
    fn invocation_builder_inspect_rejects_incomplete_tuple() {
        let builder_id = new_builder_handle();
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_invocation_builder_inspect(builder_id, &mut out) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        assert!(get_builder(builder_id).is_some());
        assert_eq!(easynet_invocation_builder_free(builder_id), EASYNET_OK);
    }

    #[test]
    fn invocation_builder_inspect_and_build_preserve_complete_tuple_state() {
        let builder_id = new_builder_handle();
        set_complete_builder(builder_id);

        let mut inspect_ptr: *mut c_char = std::ptr::null_mut();
        let inspect_code =
            unsafe { easynet_invocation_builder_inspect(builder_id, &mut inspect_ptr) };
        assert_eq!(inspect_code, EASYNET_OK);
        assert!(get_builder(builder_id).is_some());
        let inspect_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(inspect_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(inspect_ptr) };
        assert_eq!(inspect_json["args"]["probe"], true);
        assert_eq!(inspect_json["metadata"]["trace"], "sdk-builder");
        assert_eq!(inspect_json["metadata"]["idempotency_key"], "idem-1");
        assert!(inspect_json.get("timeout_seconds").is_none());

        let mut build_ptr: *mut c_char = std::ptr::null_mut();
        let build_code = unsafe { easynet_invocation_builder_build(builder_id, &mut build_ptr) };
        assert_eq!(build_code, EASYNET_OK);
        assert!(get_builder(builder_id).is_none());
        let build_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(build_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(build_ptr) };
        assert_eq!(build_json["descriptor_ref"], inspect_json["descriptor_ref"]);
        assert!(build_json.get("timeout_seconds").is_none());

        let mut second_ptr: *mut c_char = std::ptr::dangling_mut();
        let second_code =
            unsafe { easynet_invocation_builder_inspect(builder_id, &mut second_ptr) };
        assert_eq!(second_code, ERR_INVALID_HANDLE);
        assert!(second_ptr.is_null());
        assert_typed_last_error("INVALID_HANDLE", ERR_INVALID_HANDLE, "builder handle");
    }

    #[test]
    fn invocation_builder_prepare_consumes_builder_on_success() {
        let (handle, _session) = alloc(test_session());
        let builder_id = new_builder_handle();
        set_complete_builder(builder_id);
        let options = CString::new(
            serde_json::json!({
                "expires_in_ms": 60_000,
                "signer_id": "browser-key"
            })
            .to_string(),
        )
        .unwrap();
        let mut prepared_id: PreparedInvocationId = 0;
        let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
        let code = unsafe {
            easynet_invocation_builder_prepare(
                handle,
                builder_id,
                options.as_ptr(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };

        assert_eq!(code, EASYNET_OK);
        assert_ne!(prepared_id, 0);
        assert!(get_builder(builder_id).is_none());
        assert!(get_prepared(prepared_id).is_some());
        let prepared_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(prepared_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::easynet_string_free(prepared_json_ptr) };
        assert_eq!(
            prepared_json["signing_material"]["signer_policy"]["signer_id"],
            "browser-key"
        );
        assert_eq!(
            prepared_json["signing_material"]["descriptor_ref"],
            prepared_json["descriptor_ref"]
        );
        assert_eq!(
            prepared_json["signing_material"]["expires_at_unix_ms"],
            prepared_json["expires_at_unix_ms"]
        );
        assert_eq!(easynet_prepared_invocation_free(prepared_id), EASYNET_OK);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_builder_prepare_failure_keeps_builder_mutable() {
        let (handle, _session) = alloc(test_session());
        let builder_id = new_builder_handle();
        set_complete_builder(builder_id);
        let invalid_options = CString::new("{not-json").unwrap();
        let mut prepared_id: PreparedInvocationId = 999;
        let mut prepared_json_ptr: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe {
            easynet_invocation_builder_prepare(
                handle,
                builder_id,
                invalid_options.as_ptr(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };

        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(prepared_id, 0);
        assert!(prepared_json_ptr.is_null());
        assert!(get_builder(builder_id).is_some());

        let mut build_ptr: *mut c_char = std::ptr::null_mut();
        let build_code = unsafe { easynet_invocation_builder_build(builder_id, &mut build_ptr) };
        assert_eq!(build_code, EASYNET_OK);
        assert!(get_builder(builder_id).is_none());
        unsafe { crate::ffi::strings::easynet_string_free(build_ptr) };
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_builder_raw_arguments_round_trip_as_base64() {
        let builder_id = new_builder_handle();
        set_complete_builder(builder_id);
        let payload = CString::new("AQID").unwrap();
        let content_type = CString::new("application/octet-stream").unwrap();
        assert_eq!(
            unsafe {
                easynet_invocation_builder_set_arguments_base64(
                    builder_id,
                    payload.as_ptr(),
                    content_type.as_ptr(),
                )
            },
            EASYNET_OK
        );

        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { easynet_invocation_builder_build(builder_id, &mut out) };
        assert_eq!(code, EASYNET_OK);
        let json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(out) };
        assert_eq!(json["arguments_base64"], "AQID");
        assert_eq!(json["content_type"], "application/octet-stream");
        assert!(json.get("args").is_none());
    }

    #[test]
    fn timeout_seconds_passes_through_to_the_invoke_request_wire(// F-045
    ) {
        let raw = canonical_invocation_json(serde_json::json!({"timeout_seconds": 45}));
        let request = InvocationJson::parse(&raw)
            .expect("parse")
            .into_daemon_invocation()
            .expect("build")
            .into_request()
            .expect("request");
        assert_eq!(request.timeout_seconds, 45);

        // Absent field = proto default (0 → daemon default budget).
        let request = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({})))
            .expect("parse")
            .into_daemon_invocation()
            .expect("build")
            .into_request()
            .expect("request");
        assert_eq!(request.timeout_seconds, 0);

        // Non-positive and non-integer values are typed parse errors.
        for bad in ["0", "-3", "\"45\"", "1.5"] {
            let raw = format!(
                r#"{{
                    "caller_ura": "easynet:///r/acme/device/dev-a",
                    "callee_ura": "easynet:///r/acme/device/dev-a",
                    "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0",
                    "subject_ura": "easynet:///r/acme/device/dev-a",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "causal_context": {{"form": "none"}},
                    "args": {{}},
                    "timeout_seconds": {bad}
                }}"#
            );
            let err = InvocationJson::parse(&raw).expect_err("must reject");
            assert!(
                matches!(err, InvocationJsonError::InvalidTimeoutSeconds),
                "timeout_seconds={bad} must be InvalidTimeoutSeconds, got {err}"
            );
        }
    }

    #[test]
    fn invocation_prepare_and_sign_prepared_allocate_state_handles() {
        let (handle, _session) = alloc(test_session());
        let raw = CString::new(canonical_invocation_json(serde_json::json!({
            "args": {"probe": true}
        })))
        .unwrap();
        let options = CString::new(
            serde_json::json!({
                "expires_in_ms": 60_000,
                "signer_id": "browser-key",
                "policy_ref": "policy/local",
                "local_daemon_signing": false
            })
            .to_string(),
        )
        .unwrap();
        let mut prepared_id: PreparedInvocationId = 0;
        let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_invocation_prepare(
                handle,
                raw.as_ptr(),
                options.as_ptr(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };

        assert_eq!(code, EASYNET_OK);
        assert_ne!(prepared_id, 0);
        assert!(get_prepared(prepared_id).is_some());
        let prepared_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(prepared_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::easynet_string_free(prepared_json_ptr) };
        assert_eq!(
            prepared_json["signing_material"]["signer_policy"]["mode"],
            "caller_signing"
        );
        assert_eq!(
            prepared_json["signing_material"]["signer_policy"]["signer_id"],
            "browser-key"
        );
        assert_eq!(
            prepared_json["signing_material"]["descriptor_ref"],
            prepared_json["descriptor_ref"]
        );
        assert_eq!(
            prepared_json["signing_material"]["expires_at_unix_ms"],
            prepared_json["expires_at_unix_ms"]
        );
        assert!(prepared_json["signing_material"]["canonical_bytes_base64"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));

        let signature = CString::new(
            serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6eg==",
                "key_id_hint": "caller-key"
            })
            .to_string(),
        )
        .unwrap();
        let mut signed_id: SignedInvocationId = 0;
        let mut signed_json_ptr: *mut c_char = std::ptr::null_mut();
        let code = unsafe {
            easynet_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut signed_id,
                &mut signed_json_ptr,
            )
        };

        assert_eq!(code, EASYNET_OK);
        assert_ne!(signed_id, 0);
        assert!(get_prepared(prepared_id).is_none());
        assert!(get_signed(signed_id).is_some());
        let signed_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(signed_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::easynet_string_free(signed_json_ptr) };
        assert_eq!(signed_json["signer_id"], "browser-key");
        assert_eq!(signed_json["policy"]["mode"], "caller_signing");
        assert_eq!(signed_json["policy"]["signer_id"], "browser-key");
        assert_eq!(signed_json["policy"]["policy_ref"], "policy/local");
        assert_eq!(signed_json["signature"]["algorithm"], "ed25519");
        assert_eq!(signed_json["signature"]["key_id_hint"], "caller-key");

        let mut duplicate_signed_id: SignedInvocationId = 0;
        let mut duplicate_signed_json_ptr: *mut c_char = std::ptr::null_mut();
        let duplicate_code = unsafe {
            easynet_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut duplicate_signed_id,
                &mut duplicate_signed_json_ptr,
            )
        };
        assert_eq!(duplicate_code, ERR_INVALID_HANDLE);
        assert_eq!(duplicate_signed_id, 0);
        assert!(duplicate_signed_json_ptr.is_null());
        assert_eq!(easynet_signed_invocation_free(signed_id), EASYNET_OK);
        assert!(get_signed(signed_id).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn invocation_sign_prepared_local_uses_default_daemon_keyring() {
        with_test_key_service(2, |entry| {
            let caller = "easynet:///r/acme/device/dev-a";
            let signer_id = format!("signer-{}", entry.key_id);
            let policy_ref = entry.signer_policy_ref.unwrap();
            let (handle, _session) = alloc(test_session());
            let raw = CString::new(canonical_invocation_json(serde_json::json!({
                "caller_ura": caller,
                "subject_ura": caller,
                "args": {"probe": true}
            })))
            .unwrap();
            let options = CString::new(
                serde_json::json!({
                    "expires_in_ms": 60_000,
                    "signer_id": signer_id,
                    "policy_ref": policy_ref,
                    "local_daemon_signing": true
                })
                .to_string(),
            )
            .unwrap();
            let mut prepared_id: PreparedInvocationId = 0;
            let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
            let prepare_code = unsafe {
                easynet_invocation_prepare(
                    handle,
                    raw.as_ptr(),
                    options.as_ptr(),
                    &mut prepared_id,
                    &mut prepared_json_ptr,
                )
            };
            assert_eq!(prepare_code, EASYNET_OK);
            assert_ne!(prepared_id, 0);
            unsafe { crate::ffi::strings::easynet_string_free(prepared_json_ptr) };

            let mut signed_id: SignedInvocationId = 0;
            let mut signed_json_ptr: *mut c_char = std::ptr::null_mut();
            let sign_code = unsafe {
                easynet_invocation_sign_prepared_local(
                    prepared_id,
                    &mut signed_id,
                    &mut signed_json_ptr,
                )
            };

            assert_eq!(
                sign_code,
                EASYNET_OK,
                "local daemon sign error: {}",
                read_last_error_json()
            );
            assert_ne!(signed_id, 0);
            assert!(get_prepared(prepared_id).is_none());
            assert!(get_signed(signed_id).is_some());
            let signed_json: serde_json::Value = unsafe {
                serde_json::from_str(CStr::from_ptr(signed_json_ptr).to_str().unwrap()).unwrap()
            };
            unsafe { crate::ffi::strings::easynet_string_free(signed_json_ptr) };
            assert_eq!(signed_json["policy"]["mode"], "local_daemon_signing");
            assert_eq!(signed_json["policy"]["signer_id"], signer_id);
            assert_eq!(signed_json["policy"]["policy_ref"], policy_ref);
            assert_eq!(signed_json["signature"]["algorithm"], "ed25519");
            assert_eq!(signed_json["signature"]["key_id_hint"], signer_id);
            assert!(signed_json["signature"]["signature_base64"]
                .as_str()
                .is_some_and(|value| !value.is_empty()));
            assert_eq!(easynet_signed_invocation_free(signed_id), EASYNET_OK);
            crate::ffi::client::handle::release(handle);
        });
    }

    #[test]
    #[cfg(unix)]
    fn invocation_sign_prepared_local_failure_preserves_prepared_handle() {
        with_test_key_service(1, |entry| {
            let caller = "easynet:///r/acme/device/dev-a";
            let signer_id = format!("signer-{}", entry.key_id);
            let (handle, _session) = alloc(test_session());
            let raw = CString::new(canonical_invocation_json(serde_json::json!({
                "caller_ura": caller,
                "subject_ura": caller,
                "args": {"probe": true}
            })))
            .unwrap();
            let options = CString::new(
                serde_json::json!({
                    "expires_in_ms": 60_000,
                    "signer_id": signer_id,
                    "policy_ref": "daemon-key-inventory:sha256:wrong",
                    "local_daemon_signing": true
                })
                .to_string(),
            )
            .unwrap();
            let mut prepared_id: PreparedInvocationId = 0;
            let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
            let prepare_code = unsafe {
                easynet_invocation_prepare(
                    handle,
                    raw.as_ptr(),
                    options.as_ptr(),
                    &mut prepared_id,
                    &mut prepared_json_ptr,
                )
            };
            assert_eq!(prepare_code, EASYNET_OK);
            unsafe { crate::ffi::strings::easynet_string_free(prepared_json_ptr) };

            let mut signed_id: SignedInvocationId = 99;
            let mut signed_json_ptr: *mut c_char = std::ptr::dangling_mut();
            let sign_code = unsafe {
                easynet_invocation_sign_prepared_local(
                    prepared_id,
                    &mut signed_id,
                    &mut signed_json_ptr,
                )
            };

            assert_eq!(sign_code, ERR_INVALID_ARG);
            assert_eq!(signed_id, 0);
            assert!(signed_json_ptr.is_null());
            assert!(get_prepared(prepared_id).is_some());
            assert_typed_last_error("INVALID_ARGUMENT", ERR_INVALID_ARG, "policy_ref");
            assert_eq!(easynet_prepared_invocation_free(prepared_id), EASYNET_OK);
            crate::ffi::client::handle::release(handle);
        });
    }

    #[test]
    fn invocation_submit_signed_rejects_invalid_client_before_daemon_io() {
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_invocation_submit_signed(9_999_999, 1, &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn invocation_submit_signed_consumes_signed_handle_before_transport() {
        let raw = CString::new(canonical_invocation_json(serde_json::json!({
            "args": {"probe": true}
        })))
        .unwrap();
        let (prepare_handle, _) = alloc(test_session());
        let mut prepared_id: PreparedInvocationId = 0;
        let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
        let prepare_code = unsafe {
            easynet_invocation_prepare(
                prepare_handle,
                raw.as_ptr(),
                std::ptr::null(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };
        assert_eq!(prepare_code, EASYNET_OK);
        unsafe { crate::ffi::strings::easynet_string_free(prepared_json_ptr) };

        let signature = CString::new(
            serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6eg==",
                "key_id_hint": "caller-key"
            })
            .to_string(),
        )
        .unwrap();
        let mut signed_id: SignedInvocationId = 0;
        let mut signed_json_ptr: *mut c_char = std::ptr::null_mut();
        let sign_code = unsafe {
            easynet_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut signed_id,
                &mut signed_json_ptr,
            )
        };
        assert_eq!(sign_code, EASYNET_OK);
        assert!(get_signed(signed_id).is_some());
        unsafe { crate::ffi::strings::easynet_string_free(signed_json_ptr) };

        let (client_handle, _) = alloc(
            crate::ffi::client::handle::ClientSession::with_control_path_only(
                "/tmp/easynet-control.json".to_string(),
                Some("/tmp/easynet-missing-daemon.sock".to_string()),
            ),
        );
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let submit_code =
            unsafe { easynet_invocation_submit_signed(client_handle, signed_id, &mut out) };
        assert_eq!(submit_code, ERR_DAEMON_DOWN);
        assert!(out.is_null());
        assert!(get_signed(signed_id).is_none());

        let mut second_out: *mut c_char = std::ptr::dangling_mut();
        let second_code =
            unsafe { easynet_invocation_submit_signed(client_handle, signed_id, &mut second_out) };
        assert_eq!(second_code, ERR_INVALID_HANDLE);
        assert!(second_out.is_null());
        crate::ffi::client::handle::release(prepare_handle);
        crate::ffi::client::handle::release(client_handle);
    }

    #[test]
    fn invocation_handle_submit_rejects_invalid_client_before_consuming_signed() {
        let signed_id = new_signed_invocation_id();
        let mut invocation_handle_id: InvocationHandleId = 999;
        let mut submitted_ptr: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_invocation_submit_signed_handle(
                9_999_999,
                signed_id,
                &mut invocation_handle_id,
                &mut submitted_ptr,
            )
        };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_eq!(invocation_handle_id, 0);
        assert!(submitted_ptr.is_null());
        assert!(get_signed(signed_id).is_some());
        assert_eq!(easynet_signed_invocation_free(signed_id), EASYNET_OK);
    }

    #[test]
    fn invocation_handle_await_observes_transport_failure_terminal() {
        let signed_id = new_signed_invocation_id();
        let (client_handle, _) = alloc(
            crate::ffi::client::handle::ClientSession::with_control_path_only(
                "/tmp/easynet-control.json".to_string(),
                Some("/tmp/easynet-missing-daemon.sock".to_string()),
            ),
        );
        let mut invocation_handle_id: InvocationHandleId = 0;
        let mut submitted_ptr: *mut c_char = std::ptr::null_mut();
        let submit_code = unsafe {
            easynet_invocation_submit_signed_handle(
                client_handle,
                signed_id,
                &mut invocation_handle_id,
                &mut submitted_ptr,
            )
        };
        assert_eq!(submit_code, EASYNET_OK);
        assert_ne!(invocation_handle_id, 0);
        assert!(get_signed(signed_id).is_none());
        let submitted_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(submitted_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::easynet_string_free(submitted_ptr) };
        assert_eq!(submitted_json["state"], "Submitted");
        assert_eq!(submitted_json["terminal"], false);

        let mut result_ptr: *mut c_char = std::ptr::null_mut();
        let await_code = unsafe {
            easynet_invocation_handle_await(client_handle, invocation_handle_id, &mut result_ptr)
        };
        assert_eq!(await_code, EASYNET_OK);
        let result_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(result_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(result_ptr) };
        assert_eq!(result_json["ok"], false);
        assert_eq!(result_json["error"]["code"], "DAEMON_OFFLINE");

        let mut events_ptr: *mut c_char = std::ptr::null_mut();
        let events_code = unsafe {
            easynet_invocation_handle_events(client_handle, invocation_handle_id, &mut events_ptr)
        };
        assert_eq!(events_code, EASYNET_OK);
        let events_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(events_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(events_ptr) };
        assert_eq!(events_json["terminal"], true);
        assert_eq!(events_json["events"][0]["state"], "Submitted");
        assert_eq!(events_json["events"][1]["state"], "Failed");
        assert_eq!(
            easynet_invocation_handle_free(client_handle, invocation_handle_id),
            EASYNET_OK
        );
        crate::ffi::client::handle::release(client_handle);
    }

    #[test]
    fn invocation_handle_cancel_after_terminal_does_not_rewrite_state() {
        let signed_id = new_signed_invocation_id();
        let (client_handle, _) = alloc(
            crate::ffi::client::handle::ClientSession::with_control_path_only(
                "/tmp/easynet-control.json".to_string(),
                Some("/tmp/easynet-missing-daemon.sock".to_string()),
            ),
        );
        let mut invocation_handle_id: InvocationHandleId = 0;
        let mut submitted_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                easynet_invocation_submit_signed_handle(
                    client_handle,
                    signed_id,
                    &mut invocation_handle_id,
                    &mut submitted_ptr,
                )
            },
            EASYNET_OK
        );
        unsafe { crate::ffi::strings::easynet_string_free(submitted_ptr) };

        let mut result_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                easynet_invocation_handle_await(
                    client_handle,
                    invocation_handle_id,
                    &mut result_ptr,
                )
            },
            EASYNET_OK
        );
        unsafe { crate::ffi::strings::easynet_string_free(result_ptr) };

        let reason = CString::new(serde_json::json!({"reason": "too-late"}).to_string()).unwrap();
        let mut cancel_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                easynet_invocation_handle_cancel(
                    client_handle,
                    invocation_handle_id,
                    reason.as_ptr(),
                    &mut cancel_ptr,
                )
            },
            EASYNET_OK
        );
        let cancel_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(cancel_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(cancel_ptr) };
        assert_eq!(cancel_json["cancelled"], false);
        assert_eq!(cancel_json["state"], "Failed");

        let mut events_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                easynet_invocation_handle_events(
                    client_handle,
                    invocation_handle_id,
                    &mut events_ptr,
                )
            },
            EASYNET_OK
        );
        let events_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(events_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(events_ptr) };
        assert_eq!(events_json["events"].as_array().unwrap().len(), 2);
        assert_eq!(events_json["events"][1]["state"], "Failed");
        assert_eq!(
            easynet_invocation_handle_free(client_handle, invocation_handle_id),
            EASYNET_OK
        );
        crate::ffi::client::handle::release(client_handle);
    }

    #[test]
    fn invocation_handle_rejects_cross_owner_access() {
        let signed_id = new_signed_invocation_id();
        let (owner_handle, _) = alloc(
            crate::ffi::client::handle::ClientSession::with_control_path_only(
                "/tmp/easynet-control.json".to_string(),
                Some("/tmp/easynet-missing-daemon.sock".to_string()),
            ),
        );
        let (other_handle, _) = alloc(test_session());
        let mut invocation_handle_id: InvocationHandleId = 0;
        let mut submitted_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                easynet_invocation_submit_signed_handle(
                    owner_handle,
                    signed_id,
                    &mut invocation_handle_id,
                    &mut submitted_ptr,
                )
            },
            EASYNET_OK
        );
        unsafe { crate::ffi::strings::easynet_string_free(submitted_ptr) };

        let mut events_ptr: *mut c_char = std::ptr::dangling_mut();
        let events_code = unsafe {
            easynet_invocation_handle_events(other_handle, invocation_handle_id, &mut events_ptr)
        };
        assert_eq!(events_code, ERR_INVALID_HANDLE);
        assert!(events_ptr.is_null());
        assert_eq!(
            easynet_invocation_handle_free(owner_handle, invocation_handle_id),
            EASYNET_OK
        );
        crate::ffi::client::handle::release(owner_handle);
        crate::ffi::client::handle::release(other_handle);
    }

    #[test]
    fn invocation_handle_cancel_before_completion_is_terminal_monotonic() {
        let (owner_handle, _) = alloc(test_session());
        let tuple = signed_fixture_tuple();
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let active = ActiveInvocationHandle::new(owner_handle, tuple.clone(), tuple_json);
        let shared = active.shared.clone();
        let invocation_handle_id = insert_invocation_handle(active);
        let handle = get_invocation_handle_for_owner(owner_handle, invocation_handle_id)
            .unwrap()
            .unwrap();

        let outcome = handle.cancel(Some("client stop".to_string()));
        assert!(outcome.cancelled);
        assert_eq!(outcome.state, InvocationHandlePhase::Cancelled);
        assert!(!shared.mark_terminal(completed_result_for_tuple(tuple)));
        let result = handle.await_result();
        assert_eq!(
            terminal_phase_for_result(&result),
            InvocationHandlePhase::Cancelled
        );

        let events_json = handle.events_json(invocation_handle_id);
        assert_eq!(events_json["events"].as_array().unwrap().len(), 2);
        assert_eq!(events_json["events"][1]["state"], "Cancelled");
        assert_eq!(
            easynet_invocation_handle_free(owner_handle, invocation_handle_id),
            EASYNET_OK
        );
        crate::ffi::client::handle::release(owner_handle);
    }

    #[test]
    fn parse_invocation_json_rejects_zero_nonce() {
        let err = InvocationJson::parse(
            r#"{
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": "easynet:///r/acme/device/dev-a",
                "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0",
                "subject_ura": "easynet:///r/acme/device/dev-a",
                "nonce_base64": "AAAAAAAAAAAAAAAAAAAAAA==",
                "causal_context": {"form": "none"},
                "args": {}
            }"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("all-zero"),
            "zero nonce must be rejected: {err}"
        );
    }

    #[test]
    fn parse_invocation_json_supports_raw_payloads() {
        let spec = InvocationJson::parse(
            r#"{
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": "easynet:///r/acme/device/dev-a",
                "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0",
                "subject_ura": "easynet:///r/acme/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "arguments_base64": "aGVsbG8=",
                "content_type": "text/plain"
            }"#,
        )
        .unwrap();
        assert_eq!(spec.args, b"hello");
        assert_eq!(spec.content_type, "text/plain");
    }

    #[test]
    fn parse_invocation_json_supports_complete_bidi_invocation() {
        let raw = valid_bidi_invocation_json();
        let spec = InvocationJson::parse(raw.to_str().unwrap()).unwrap();
        assert_eq!(
            spec.descriptor_ref,
            descriptor_ref(
                "easynet:///r/acme/device/dev-a",
                "device.pty.attach",
                "2.4.0"
            )
        );
        assert_eq!(spec.metadata["x-easynet-delegation"], "producer");
        let signature = spec.caller_signature.expect("caller signature required");
        assert_eq!(signature.algorithm, "ed25519");
        assert_eq!(signature.signature, vec![7; 64]);
        assert_eq!(signature.key_id_hint, "caller-key");
        assert_eq!(spec.bidi_streams.len(), 1);
        assert_eq!(spec.bidi_streams[0].stream_id, 1);
        assert_eq!(spec.bidi_streams[0].content_type, "text/pty");
        assert_eq!(spec.bidi_streams[0].codec_params, "raw");
        assert_eq!(spec.bidi_streams[0].ordering, "STRICT");
    }

    #[test]
    fn parse_invocation_json_projects_signer_pubkey_to_key_hint() {
        let raw = canonical_invocation_json(serde_json::json!({
            "caller_signature": {
                "algorithm": "ed25519",
                "signature_base64": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==",
                "signer_public_key_base64": "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
            }
        }));

        let signature = InvocationJson::parse(&raw)
            .unwrap()
            .caller_signature
            .expect("caller signature");

        assert_eq!(
            signature.key_id_hint,
            "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
        );
    }

    #[test]
    fn signature_json_projects_signer_pubkey_to_key_hint() {
        let parsed = SignatureMaterialJson::parse(
            &serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6eg==",
                "signer_public_key_base64": "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            parsed.key_id_hint,
            "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
        );
    }

    #[test]
    fn parse_bidi_up_frame_json_supports_binary_chunk_and_controls() {
        use easynet_axon::pb::axon::v1::{bidi_control, invoke_bidi_up};

        let chunk = parse_bidi_up_frame_json(
            r#"{"type":"binary_chunk","stream_id":1,"data_base64":"aGVsbG8=","pts":9}"#,
        )
        .unwrap();
        let invoke_bidi_up::Payload::BinaryChunk(chunk) = chunk.payload else {
            panic!("expected binary chunk");
        };
        assert_eq!(chunk.stream_id, 1);
        assert_eq!(chunk.data, b"hello");
        assert_eq!(chunk.pts, 9);

        let control =
            parse_bidi_up_frame_json(r#"{"type":"control","media_pts":{"stream_id":2,"pts":123}}"#)
                .unwrap();
        let invoke_bidi_up::Payload::Control(control) = control.payload else {
            panic!("expected control");
        };
        assert!(matches!(
            control.control,
            Some(bidi_control::Control::MediaPts(media)) if media.stream_id == 2 && media.pts == 123
        ));
    }

    #[test]
    fn invocation_endpoint_requires_advertised_daemon_endpoint() {
        let session = test_session();
        let err = invocation_endpoint_for_session(&session).unwrap_err();
        assert!(matches!(
            err,
            crate::daemon::DaemonError::InvocationEndpointMissing { .. }
        ));
    }

    #[test]
    fn invocation_endpoint_prefers_daemon_handle_endpoint_override() {
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            "/tmp/custom/control.sock".into(),
            Some("/tmp/other/daemon.sock".into()),
        );
        assert_eq!(
            invocation_endpoint_for_session(&session).unwrap(),
            std::path::PathBuf::from("/tmp/other/daemon.sock")
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_health_json_uses_shared_health_dto_shape() {
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            "/tmp/custom/control.sock".into(),
            Some("/tmp/other/daemon.sock".into()),
        );
        let health = runtime_health_json(&session);

        assert_eq!(health["api_ready"], true);
        assert_eq!(health["daemon_ready"], true);
        assert_eq!(health["invocation_ready"], false);
        assert_eq!(health["runtime_ready"], false);
        assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(health["abi_version"], crate::ffi::EASYNET_ABI_VERSION);
        assert!(health["diagnostics"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
        assert!(health.get("control_ready").is_none());
        assert!(health.get("connection_state").is_none());
        assert!(health.get("last_error").is_none());
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_diagnostics_json_reports_health_profile_checks() {
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            "/tmp/custom/control.sock".into(),
            Some("/tmp/other/daemon.sock".into()),
        );
        let diagnostics = runtime_diagnostics_json(&session);

        assert_eq!(diagnostics["profile"], "health");
        assert_eq!(diagnostics["kind"], "diagnostics_report");
        assert_eq!(diagnostics["ready"], false);
        assert_eq!(diagnostics["control_endpoint"], "/tmp/custom/control.sock");
        assert_eq!(diagnostics["invocation_endpoint"], "/tmp/other/daemon.sock");
        assert_eq!(diagnostics["checks"].as_array().map(Vec::len), Some(6));
        assert!(diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn invocation_invoke_rejects_invalid_handle_after_zeroing_out_pointer() {
        let raw = valid_invocation_json();
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_invocation_invoke(9_999_999, raw.as_ptr(), &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
        assert_typed_last_error(
            "INVALID_HANDLE",
            ERR_INVALID_HANDLE,
            "handle 9999999 is not registered",
        );
    }

    #[test]
    fn invocation_invoke_rejects_malformed_json_before_daemon_io() {
        let (handle, _) = alloc(test_session());
        let raw = CString::new("{not-json").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { easynet_invocation_invoke(handle, raw.as_ptr(), &mut out) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn invocation_stream_open_rejects_invalid_handle_after_zeroing_stream_id() {
        let raw = valid_invocation_json();
        let mut stream_id: InvocationStreamId = 42;
        let code = unsafe {
            easynet_invocation_stream_open(
                9_999_999,
                raw.as_ptr(),
                Some(ignore_stream_chunk),
                std::ptr::null_mut(),
                &mut stream_id,
            )
        };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_eq!(stream_id, 0);
    }

    #[test]
    fn invocation_bidi_open_rejects_invalid_handle_after_zeroing_bidi_id() {
        let raw = valid_bidi_invocation_json();
        let mut bidi_id: InvocationBidiId = 42;
        let code = unsafe {
            easynet_invocation_bidi_open(
                9_999_999,
                raw.as_ptr(),
                Some(ignore_bidi_frame),
                std::ptr::null_mut(),
                &mut bidi_id,
            )
        };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_eq!(bidi_id, 0);
    }

    #[test]
    fn invocation_bidi_open_requires_callback_before_daemon_io() {
        let (handle, _) = alloc(test_session());
        let raw = valid_bidi_invocation_json();
        let mut bidi_id: InvocationBidiId = 42;
        let code = unsafe {
            easynet_invocation_bidi_open(
                handle,
                raw.as_ptr(),
                None,
                std::ptr::null_mut(),
                &mut bidi_id,
            )
        };
        assert_eq!(code, ERR_NULL_POINTER);
        assert_eq!(bidi_id, 0);
        assert_typed_last_error(
            "NULL_POINTER",
            ERR_NULL_POINTER,
            "on_frame callback is null",
        );
    }

    #[test]
    fn invocation_stream_open_requires_callback_before_daemon_io() {
        let (handle, _) = alloc(test_session());
        let raw = valid_invocation_json();
        let mut stream_id: InvocationStreamId = 42;
        let code = unsafe {
            easynet_invocation_stream_open(
                handle,
                raw.as_ptr(),
                None,
                std::ptr::null_mut(),
                &mut stream_id,
            )
        };
        assert_eq!(code, ERR_NULL_POINTER);
        assert_eq!(stream_id, 0);
        assert_typed_last_error(
            "NULL_POINTER",
            ERR_NULL_POINTER,
            "on_chunk callback is null",
        );
    }

    #[test]
    fn invocation_stream_open_rejects_malformed_json_before_daemon_io() {
        let (handle, _) = alloc(test_session());
        let raw = CString::new("{not-json").unwrap();
        let mut stream_id: InvocationStreamId = 0;
        let code = unsafe {
            easynet_invocation_stream_open(
                handle,
                raw.as_ptr(),
                Some(ignore_stream_chunk),
                std::ptr::null_mut(),
                &mut stream_id,
            )
        };
        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(stream_id, 0);
    }

    #[test]
    fn invocation_stream_cancel_is_idempotent_for_unknown_stream() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { easynet_invocation_stream_cancel(handle, 9_999_999) };
        assert_eq!(code, EASYNET_OK);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_cancel_is_idempotent_for_unknown_session() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { easynet_invocation_bidi_cancel(handle, 9_999_999) };
        assert_eq!(code, EASYNET_OK);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_close_is_idempotent_for_unknown_stream() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { easynet_invocation_stream_close(handle, 9_999_999) };
        assert_eq!(code, EASYNET_OK);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_close_refuses_cross_handle_access() {
        let (owner, _) = alloc(test_session());
        let (other, _) = alloc(test_session());
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream {
            owner,
            cancel: cancel.clone(),
        });

        let code = unsafe { easynet_invocation_stream_close(other, stream_id) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(!cancel.is_cancelled());
        assert!(remove_stream(stream_id).is_some());
        crate::ffi::client::handle::release(owner);
        crate::ffi::client::handle::release(other);
    }

    #[test]
    fn invocation_bidi_close_send_rejects_unknown_session() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { easynet_invocation_bidi_close_send(handle, 9_999_999) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_close_send_refuses_cross_handle_access() {
        let (owner, _) = alloc(test_session());
        let (other, _) = alloc(test_session());
        let (session, mut up_rx, _cancel) = active_bidi_session(owner, 4);
        let bidi_id = insert_bidi(session);

        let code = unsafe { easynet_invocation_bidi_close_send(other, bidi_id) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(matches!(
            up_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_some());
        assert_eq!(
            unsafe { easynet_invocation_bidi_cancel(owner, bidi_id) },
            EASYNET_OK
        );
        crate::ffi::client::handle::release(owner);
        crate::ffi::client::handle::release(other);
    }

    #[test]
    fn invocation_bidi_close_send_keeps_session_and_blocks_later_send() {
        let (handle, _) = alloc(test_session());
        let (session, mut up_rx, _cancel) = active_bidi_session(handle, 4);
        let bidi_id = insert_bidi(session);

        assert_eq!(
            unsafe { easynet_invocation_bidi_close_send(handle, bidi_id) },
            EASYNET_OK
        );
        assert!(get_bidi_for_handle(handle, bidi_id).unwrap().is_some());
        assert_bidi_eof_frame(up_rx.try_recv().expect("EOF frame must be sent"), 1);

        assert_eq!(
            unsafe { easynet_invocation_bidi_close_send(handle, bidi_id) },
            EASYNET_OK
        );
        assert!(matches!(
            up_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let frame = CString::new(
            serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8="
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            unsafe { easynet_invocation_bidi_send(handle, bidi_id, frame.as_ptr()) },
            ERR_CANCELLED
        );
        assert!(get_bidi_for_handle(handle, bidi_id).unwrap().is_some());

        assert_eq!(
            unsafe { easynet_invocation_bidi_close(handle, bidi_id) },
            EASYNET_OK
        );
        assert!(get_bidi_for_handle(handle, bidi_id).unwrap().is_none());
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_send_eof_also_half_closes_local_send() {
        let (handle, _) = alloc(test_session());
        let (session, mut up_rx, _cancel) = active_bidi_session(handle, 4);
        let bidi_id = insert_bidi(session);
        let eof =
            CString::new(serde_json::json!({"type": "control", "eof": true}).to_string()).unwrap();
        let frame = CString::new(
            serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8="
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            unsafe { easynet_invocation_bidi_send(handle, bidi_id, eof.as_ptr()) },
            EASYNET_OK
        );
        assert_bidi_eof_frame(up_rx.try_recv().expect("EOF frame must be sent"), 1);
        assert_eq!(
            unsafe { easynet_invocation_bidi_send(handle, bidi_id, frame.as_ptr()) },
            ERR_CANCELLED
        );
        assert!(get_bidi_for_handle(handle, bidi_id).unwrap().is_some());
        assert_eq!(
            unsafe { easynet_invocation_bidi_close(handle, bidi_id) },
            EASYNET_OK
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn stream_registry_remove_returns_registered_cancel_token() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream {
            owner: 41,
            cancel: cancel.clone(),
        });
        let stream = remove_stream(stream_id).expect("registered stream should be removable");
        stream.cancel.cancel();
        assert!(cancel.is_cancelled());
        assert!(
            remove_stream(stream_id).is_none(),
            "stream removal must be one-shot"
        );
    }

    #[test]
    fn bidi_registry_remove_returns_registered_session() {
        let (session, _up_rx, cancel) = active_bidi_session(41, 1);
        let bidi_id = insert_bidi(session);
        let session = remove_bidi(bidi_id).expect("registered bidi session should be removable");
        session.cancel.cancel();
        assert!(cancel.is_cancelled());
        assert!(
            remove_bidi(bidi_id).is_none(),
            "bidi removal must be one-shot"
        );
    }

    #[test]
    fn cancel_invocations_for_handle_removes_only_owned_entries() {
        let owned_stream_cancel = tokio_util::sync::CancellationToken::new();
        let other_stream_cancel = tokio_util::sync::CancellationToken::new();
        let owned_stream_id = insert_stream(ActiveInvocationStream {
            owner: 41,
            cancel: owned_stream_cancel.clone(),
        });
        let other_stream_id = insert_stream(ActiveInvocationStream {
            owner: 42,
            cancel: other_stream_cancel.clone(),
        });

        let (owned_bidi, _owned_up_rx, owned_bidi_cancel) = active_bidi_session(41, 1);
        let (other_bidi, _other_up_rx, other_bidi_cancel) = active_bidi_session(42, 1);
        let owned_bidi_id = insert_bidi(owned_bidi);
        let other_bidi_id = insert_bidi(other_bidi);

        cancel_invocations_for_handle(41);

        assert!(owned_stream_cancel.is_cancelled());
        assert!(owned_bidi_cancel.is_cancelled());
        assert!(remove_stream(owned_stream_id).is_none());
        assert!(remove_bidi(owned_bidi_id).is_none());

        assert!(!other_stream_cancel.is_cancelled());
        assert!(!other_bidi_cancel.is_cancelled());
        assert!(remove_stream(other_stream_id).is_some());
        assert!(remove_bidi(other_bidi_id).is_some());
    }

    #[test]
    fn stream_registry_refuses_cross_handle_cancel() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream { owner: 101, cancel });

        assert!(matches!(
            remove_stream_for_handle(202, stream_id),
            Err(RegistryOwnerMismatch)
        ));
        assert!(
            remove_stream(stream_id).is_some(),
            "owner mismatch must not remove another handle's stream"
        );
    }

    #[test]
    fn bidi_registry_refuses_cross_handle_access() {
        let (session, _up_rx, _cancel) = active_bidi_session(101, 1);
        let bidi_id = insert_bidi(session);

        assert!(matches!(
            get_bidi_for_handle(202, bidi_id),
            Err(RegistryOwnerMismatch)
        ));
        assert!(
            remove_bidi(bidi_id).is_some(),
            "owner mismatch must not remove another handle's bidi session"
        );
    }

    #[test]
    fn tonic_status_codes_keep_binding_error_categories() {
        assert_eq!(
            ffi_status_code_to_error(tonic::Code::Unavailable),
            ERR_DAEMON_DOWN
        );
        assert_eq!(
            ffi_status_code_to_error(tonic::Code::InvalidArgument),
            ERR_INVALID_ARG
        );
        assert_eq!(
            ffi_status_code_to_error(tonic::Code::PermissionDenied),
            ERR_PERMISSION_DENIED
        );
        assert_eq!(
            ffi_status_code_to_error(tonic::Code::NotFound),
            ERR_NOT_FOUND
        );
        assert_eq!(
            ffi_status_code_to_error(tonic::Code::Internal),
            ERR_PROTOCOL
        );
    }

    #[test]
    fn daemon_transport_error_records_typed_last_error() {
        let code = ffi_daemon_error(
            "easynet_invocation_invoke",
            crate::daemon::DaemonError::InvocationEndpointMissing {
                control: "/tmp/easynet/control.json".into(),
            },
        );

        assert_eq!(code, ERR_DAEMON_DOWN);
        let error = read_last_error_json();
        assert_eq!(error["code"], "DAEMON_OFFLINE");
        assert_eq!(error["stage"], "transport");
        assert_eq!(error["retry"], "after_backoff");
        assert_eq!(error["details"]["abi_code"], ERR_DAEMON_DOWN);
        assert_eq!(error["details"]["legacy_untyped"], false);
    }

    #[test]
    fn daemon_status_error_records_typed_last_error() {
        let code = ffi_daemon_error(
            "easynet_invocation_stream_open",
            crate::daemon::DaemonError::InvokeStreamStatus {
                ability: "observe.health".to_string(),
                code: tonic::Code::PermissionDenied,
                message: "caller is not authorized".to_string(),
            },
        );

        assert_eq!(code, ERR_PERMISSION_DENIED);
        let error = read_last_error_json();
        assert_eq!(error["code"], "PERMISSION_DENIED");
        assert_eq!(error["stage"], "runtime");
        assert_eq!(error["retry"], "never");
        assert_eq!(error["details"]["abi_code"], ERR_PERMISSION_DENIED);
        assert_eq!(error["details"]["legacy_untyped"], false);
    }

    #[test]
    fn sync_submit_terminal_error_records_typed_last_error() {
        let mut result = completed_result_for_tuple(signed_fixture_tuple());
        result.error = Some(crate::daemon::RuntimeErrorSummary {
            code: "DEADLINE_EXCEEDED".to_string(),
            stage: "transport".to_string(),
            message: "runtime deadline exceeded".to_string(),
            retryable: true,
        });

        let code = record_sync_submit_terminal_error(&result);

        assert_eq!(code, Some(ERR_TIMEOUT));
        let error = read_last_error_json();
        assert_eq!(error["code"], "TIMEOUT");
        assert_eq!(error["stage"], "transport");
        assert_eq!(error["retry"], "safe");
        assert_eq!(error["details"]["abi_code"], ERR_TIMEOUT);
        assert_eq!(error["details"]["legacy_untyped"], false);
    }

    #[test]
    fn stream_chunk_json_decodes_json_payload() {
        let chunk = easynet_axon::pb::axon::v1::InvokeStreamChunk {
            invocation_id: "inv-1".to_string(),
            state: 2,
            payload: br#"{"ready":true}"#.to_vec(),
            content_type: "application/json".to_string(),
            sequence: 7,
            terminal: true,
            ..easynet_axon::pb::axon::v1::InvokeStreamChunk::default()
        };
        let value = stream_chunk_json(chunk);
        assert_eq!(value["ok"], true);
        assert_eq!(value["event"], "chunk");
        assert_eq!(value["sequence"], 7);
        assert_eq!(value["terminal"], true);
        assert_eq!(value["payload_json"]["ready"], true);
        assert_eq!(value["payload_base64"], "eyJyZWFkeSI6dHJ1ZX0=");
    }

    #[test]
    fn bounded_callback_enqueue_reports_terminal_backpressure_when_full() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
            tx.try_send(br#"{"sequence":1,"event":"chunk"}"#.to_vec())
                .unwrap();

            let sender = tokio::spawn(async move {
                send_callback_frame_or_backpressure(
                    &tx,
                    br#"{"sequence":2,"event":"chunk"}"#.to_vec(),
                    stream_callback_backpressure_event(2, 1),
                )
                .await
            });
            tokio::task::yield_now().await;

            assert_eq!(
                rx.recv().await.unwrap(),
                br#"{"sequence":1,"event":"chunk"}"#.to_vec()
            );
            assert!(!sender.await.unwrap());
            let value =
                serde_json::from_slice::<serde_json::Value>(&rx.recv().await.unwrap()).unwrap();
            assert_eq!(value["event"], "error");
            assert_eq!(value["sequence"], 2);
            assert_eq!(value["terminal"], true);
            assert_eq!(
                value["error"]["details"]["reason"],
                "callback_queue_overflow"
            );
            assert_eq!(value["error"]["details"]["queue_capacity"], 1);
        });
    }

    #[test]
    fn bidi_down_frame_json_decodes_binary_chunk() {
        let frame = easynet_axon::pb::axon::v1::InvokeBidiDown {
            sequence: 3,
            payload: Some(
                easynet_axon::pb::axon::v1::invoke_bidi_down::Payload::BinaryChunk(
                    easynet_axon::pb::axon::v1::BinaryChunk {
                        stream_id: 1,
                        data: b"hello".to_vec(),
                        pts: 11,
                    },
                ),
            ),
            ..easynet_axon::pb::axon::v1::InvokeBidiDown::default()
        };
        let value = bidi_down_frame_json(frame);
        assert_eq!(value["ok"], true);
        assert_eq!(value["event"], "binary_chunk");
        assert_eq!(value["sequence"], 3);
        assert_eq!(value["stream_id"], 1);
        assert_eq!(value["data_base64"], "aGVsbG8=");
        assert_eq!(value["pts"], 11);
    }
}

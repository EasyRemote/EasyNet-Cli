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

#[cfg(feature = "axon-pb")]
use rand::{rngs::OsRng, RngCore};
use std::os::raw::{c_char, c_void};
#[cfg(feature = "axon-pb")]
use std::path::PathBuf;
#[cfg(feature = "axon-pb")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(feature = "axon-pb")]
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

#[cfg(feature = "axon-pb")]
mod backpressure;

#[cfg(feature = "axon-pb")]
use self::backpressure::{bidi_callback_backpressure_frame, stream_callback_backpressure_event};
#[cfg(feature = "axon-pb")]
use crate::daemon::axon_bridge::runtime_descriptor_provider::{
    DescriptorResolutionError, RuntimeDescriptorResolutionProvider,
};
#[cfg(feature = "axon-pb")]
use crate::ffi::client::handle::{binding_for_handle, lib_runtime, ClientSessionBinding};
use crate::ffi::client::handle::{get, RuntimeHandle};
#[cfg(not(feature = "axon-pb"))]
use crate::ffi::errors::ERR_NOT_IMPLEMENTED;
#[cfg(feature = "axon-pb")]
use crate::ffi::errors::{
    clear_last_error, ERR_ABILITY_FAILED, ERR_CANCELLED, ERR_DAEMON_DOWN, ERR_GENERIC,
    ERR_INVALID_ARG, ERR_NOT_FOUND, ERR_NOT_IMPLEMENTED, ERR_PERMISSION_DENIED, ERR_PROTOCOL,
    ERR_TIMEOUT, RUNTIME_OK,
};
use crate::ffi::errors::{
    set_last_error_code, set_last_error_projection, ErrorProjection, ERR_INVALID_HANDLE,
    ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
#[cfg(feature = "axon-pb")]
use crate::ffi::strings::alloc_output_cstring;
use crate::ffi::strings::{read_cstr, StringError};

/// Opaque id for a server-stream opened through
/// `runtime_invocation_stream_open`.
///
/// A value of 0 means no stream was allocated. Stream ids are
/// process-local; they are not Axon protocol ids and must not be
/// serialized as receipt or invocation identifiers.
pub type InvocationStreamId = u64;

/// Opaque id for an InvokeBidi session opened through
/// `runtime_invocation_bidi_open`.
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
#[cfg(feature = "axon-pb")]
const PROVIDER_CANCEL_REASON: &str = "consumer_request";
#[cfg(feature = "axon-pb")]
const CALLER_SIGNER_UNAVAILABLE_CODE: &str = "CALLER_SIGNER_UNAVAILABLE";
#[cfg(feature = "axon-pb")]
const DESCRIPTOR_OWNER_OFFLINE_CODE: &str = "DESCRIPTOR_OWNER_OFFLINE";

fn record_invocation_error(code: i32, message: impl Into<String>) -> i32 {
    set_last_error_code(code, message);
    code
}

#[cfg(feature = "axon-pb")]
fn record_invocation_projected_error(
    abi_code: i32,
    projection: ErrorProjection,
    message: impl Into<String>,
) -> i32 {
    set_last_error_projection(abi_code, projection, message);
    abi_code
}

#[cfg(feature = "axon-pb")]
fn record_caller_signer_unavailable_error(message: impl Into<String>) -> i32 {
    record_invocation_projected_error(
        ERR_PERMISSION_DENIED,
        ErrorProjection {
            code: CALLER_SIGNER_UNAVAILABLE_CODE,
            stage: "caller_identity",
            retry: "never",
        },
        message,
    )
}

#[cfg(feature = "axon-pb")]
fn record_descriptor_owner_offline_error(message: impl Into<String>) -> i32 {
    record_invocation_projected_error(
        ERR_DAEMON_DOWN,
        ErrorProjection {
            code: DESCRIPTOR_OWNER_OFFLINE_CODE,
            stage: "routing",
            retry: "safe",
        },
        message,
    )
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
///   "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
/// metadata, the echoed nonce, and terminal plus admission receipt
/// checkpoints returned by the daemon. The caller frees it with
/// `runtime_string_free`.
///
/// # Safety
/// - `handle` must be a valid handle from `runtime_init`.
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `out_receipt_json` must be a non-null pointer to a `*mut c_char`
///   owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_invoke(
    handle: RuntimeHandle,
    invocation_json: *const c_char,
    out_receipt_json: *mut *mut c_char,
) -> i32 {
    if out_receipt_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_invoke: out_receipt_json pointer is null",
        );
    }
    unsafe { *out_receipt_json = std::ptr::null_mut() };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_invoke: handle {handle} is not registered"),
            );
        }
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "runtime_invocation_invoke: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_invoke: invocation_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw);
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "runtime_invocation_invoke: axon-pb feature is not enabled in this build",
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
pub unsafe extern "C" fn runtime_health(
    handle: RuntimeHandle,
    out_health_json: *mut *mut c_char,
) -> i32 {
    if out_health_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_health: out_health_json pointer is null",
        );
    }
    unsafe { *out_health_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_health: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = session;
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "runtime_health: axon-pb feature is not enabled in this build",
        )
    }

    #[cfg(feature = "axon-pb")]
    {
        let json = runtime_health_json(session.as_ref()).to_string();
        let ptr = alloc_output_cstring(json);
        if ptr.is_null() {
            return record_invocation_error(
                ERR_GENERIC,
                "runtime_health: out-of-memory allocating health string",
            );
        }
        unsafe { *out_health_json = ptr };
        clear_last_error();
        RUNTIME_OK
    }
}

/// Return typed runtime diagnostics for an Invocation-capable client
/// handle.
///
/// # Safety
/// `out_diagnostics_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_diagnostics(
    handle: RuntimeHandle,
    out_diagnostics_json: *mut *mut c_char,
) -> i32 {
    if out_diagnostics_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_diagnostics: out_diagnostics_json pointer is null",
        );
    }
    unsafe { *out_diagnostics_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_diagnostics: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = session;
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "runtime_diagnostics: axon-pb feature is not enabled in this build",
        )
    }

    #[cfg(feature = "axon-pb")]
    {
        let json = runtime_diagnostics_json(session.as_ref()).to_string();
        let ptr = alloc_output_cstring(json);
        if ptr.is_null() {
            return record_invocation_error(
                ERR_GENERIC,
                "runtime_diagnostics: out-of-memory allocating diagnostics string",
            );
        }
        unsafe { *out_diagnostics_json = ptr };
        clear_last_error();
        RUNTIME_OK
    }
}

/// Resolve one AbilityDescriptorRef through the daemon-owned descriptor
/// catalogue for the requested callee.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_descriptor_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_resolve_descriptor_ref(
    handle: RuntimeHandle,
    request_json: *const c_char,
    out_descriptor_json: *mut *mut c_char,
) -> i32 {
    if out_descriptor_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_resolve_descriptor_ref: out_descriptor_json pointer is null",
        );
    }
    unsafe { *out_descriptor_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_resolve_descriptor_ref: handle {handle} is not registered"),
            );
        }
    };
    let raw = match read_cstr(request_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "runtime_resolve_descriptor_ref: request_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_resolve_descriptor_ref: request_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw);
        record_invocation_error(
            ERR_NOT_IMPLEMENTED,
            "runtime_resolve_descriptor_ref: axon-pb feature is not enabled in this build",
        )
    }

    #[cfg(feature = "axon-pb")]
    {
        match runtime_resolve_descriptor_ref_json(session.as_ref(), raw) {
            Ok(value) => {
                let ptr = alloc_output_cstring(value.to_string());
                if ptr.is_null() {
                    return record_invocation_error(
                        ERR_GENERIC,
                        "runtime_resolve_descriptor_ref: out-of-memory allocating descriptor JSON",
                    );
                }
                unsafe { *out_descriptor_json = ptr };
                clear_last_error();
                RUNTIME_OK
            }
            Err(error) => {
                let (abi_code, projection) = descriptor_resolution_abi_projection(&error);
                let message = format!("runtime_resolve_descriptor_ref: {error}");
                record_invocation_projected_error(abi_code, projection, message)
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
fn descriptor_resolution_abi_projection(
    error: &DescriptorResolutionError,
) -> (i32, ErrorProjection) {
    match error {
        DescriptorResolutionError::InvalidRequest(_)
        | DescriptorResolutionError::OwnerMismatch(_) => (
            ERR_INVALID_ARG,
            ErrorProjection {
                code: "INVALID_ARGUMENT",
                stage: "sdk",
                retry: "never",
            },
        ),
        DescriptorResolutionError::InvalidCatalogPayload(_) => (
            ERR_INVALID_ARG,
            ErrorProjection {
                code: "INVALID_ARGUMENT",
                stage: "provider_payload",
                retry: "never",
            },
        ),
        DescriptorResolutionError::RuntimeOwnerUnavailable(_) => (
            ERR_PERMISSION_DENIED,
            ErrorProjection {
                code: CALLER_SIGNER_UNAVAILABLE_CODE,
                stage: "caller_identity",
                retry: "never",
            },
        ),
        DescriptorResolutionError::DescriptorNotFound(_) => (
            ERR_NOT_FOUND,
            ErrorProjection {
                code: "DESCRIPTOR_NOT_FOUND",
                stage: "routing",
                retry: "never",
            },
        ),
        DescriptorResolutionError::CallModeUnsupported(_) => (
            ERR_NOT_FOUND,
            ErrorProjection {
                code: "DESCRIPTOR_CALL_MODE_UNSUPPORTED",
                stage: "routing",
                retry: "never",
            },
        ),
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
pub unsafe extern "C" fn runtime_invocation_builder_new(
    out_builder_id: *mut InvocationBuilderId,
) -> i32 {
    if out_builder_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_builder_new: out_builder_id pointer is null",
        );
    }
    unsafe { *out_builder_id = 0 };

    #[cfg(not(feature = "axon-pb"))]
    {
        record_invocation_feature_disabled("runtime_invocation_builder_new")
    }

    #[cfg(feature = "axon-pb")]
    {
        let id = insert_builder(InvocationBuilderState::default());
        unsafe { *out_builder_id = id };
        clear_last_error();
        RUNTIME_OK
    }
}

macro_rules! builder_string_setter {
    ($fn_name:ident, $arg_name:literal, $field:expr) => {
        /// Set one string field on an Invocation builder.
        ///
        /// # Safety
        /// `value` must be a non-null pointer to a valid UTF-8 C string for
        /// the selected field. `builder_id` must identify a live builder
        /// created by `runtime_invocation_builder_new`.
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
    runtime_invocation_builder_set_caller,
    "caller_ura",
    InvocationBuilderStringField::Caller
);
builder_string_setter!(
    runtime_invocation_builder_set_callee,
    "callee_ura",
    InvocationBuilderStringField::Callee
);
builder_string_setter!(
    runtime_invocation_builder_set_descriptor_ref,
    "descriptor_ref",
    InvocationBuilderStringField::DescriptorRef
);
builder_string_setter!(
    runtime_invocation_builder_set_subject,
    "subject_ura",
    InvocationBuilderStringField::Subject
);
builder_string_setter!(
    runtime_invocation_builder_set_nonce_base64,
    "nonce_base64",
    InvocationBuilderStringField::NonceBase64
);
builder_string_setter!(
    runtime_invocation_builder_set_causal_context_json,
    "causal_context_json",
    InvocationBuilderStringField::CausalContextJson
);
builder_string_setter!(
    runtime_invocation_builder_set_args_json,
    "args_json",
    InvocationBuilderStringField::ArgsJson
);
builder_string_setter!(
    runtime_invocation_builder_set_metadata_json,
    "metadata_json",
    InvocationBuilderStringField::MetadataJson
);
builder_string_setter!(
    runtime_invocation_builder_set_idempotency_key,
    "idempotency_key",
    InvocationBuilderStringField::IdempotencyKey
);
builder_string_setter!(
    runtime_invocation_builder_set_caller_signature_json,
    "signature_json",
    InvocationBuilderStringField::CallerSignatureJson
);

/// Set raw non-JSON Invocation arguments on a builder.
///
/// # Safety
/// `arguments_base64` and `content_type` must be non-null valid UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_builder_set_arguments_base64(
    builder_id: InvocationBuilderId,
    arguments_base64: *const c_char,
    content_type: *const c_char,
) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (builder_id, arguments_base64, content_type);
        record_invocation_feature_disabled("runtime_invocation_builder_set_arguments_base64")
    }

    #[cfg(feature = "axon-pb")]
    {
        let arguments_base64 = match read_builder_arg(
            "runtime_invocation_builder_set_arguments_base64",
            "arguments_base64",
            arguments_base64,
        ) {
            Ok(value) => value,
            Err(code) => return code,
        };
        let content_type = match read_builder_arg(
            "runtime_invocation_builder_set_arguments_base64",
            "content_type",
            content_type,
        ) {
            Ok(value) => value,
            Err(code) => return code,
        };
        mutate_builder(
            builder_id,
            "runtime_invocation_builder_set_arguments_base64",
            |builder| builder.set_arguments_base64(arguments_base64, content_type),
        )
    }
}

/// Set per-call timeout in seconds on a builder.
#[no_mangle]
pub extern "C" fn runtime_invocation_builder_set_timeout_seconds(
    builder_id: InvocationBuilderId,
    timeout_seconds: u32,
) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (builder_id, timeout_seconds);
        record_invocation_feature_disabled("runtime_invocation_builder_set_timeout_seconds")
    }

    #[cfg(feature = "axon-pb")]
    {
        mutate_builder(
            builder_id,
            "runtime_invocation_builder_set_timeout_seconds",
            |builder| builder.set_timeout_seconds(timeout_seconds),
        )
    }
}

/// Inspect a complete immutable Invocation draft without consuming the builder.
///
/// # Safety
/// `out_invocation_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_builder_inspect(
    builder_id: InvocationBuilderId,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    builder_output_invocation_json(
        builder_id,
        out_invocation_json,
        false,
        "runtime_invocation_builder_inspect",
    )
}

/// Build a complete Invocation JSON draft and consume the builder on success.
///
/// # Safety
/// `out_invocation_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_builder_build(
    builder_id: InvocationBuilderId,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    builder_output_invocation_json(
        builder_id,
        out_invocation_json,
        true,
        "runtime_invocation_builder_build",
    )
}

/// Prepare a builder into canonical signing material and consume the builder
/// on success.
///
/// # Safety
/// - `options_json` may be null; if non-null it must be valid UTF-8 JSON.
/// - output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_builder_prepare(
    handle: RuntimeHandle,
    builder_id: InvocationBuilderId,
    options_json: *const c_char,
    out_prepared_id: *mut PreparedInvocationId,
    out_prepared_json: *mut *mut c_char,
) -> i32 {
    if out_prepared_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_builder_prepare: out_prepared_id pointer is null",
        );
    }
    if out_prepared_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_builder_prepare: out_prepared_json pointer is null",
        );
    }
    unsafe {
        *out_prepared_id = 0;
        *out_prepared_json = std::ptr::null_mut();
    }
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_builder_prepare: handle {handle} is not registered"),
        );
    }
    let options_raw = match read_optional_cstr(options_json) {
        Ok(value) => value,
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_builder_prepare: options_json is not valid UTF-8",
            );
        }
        Err(StringError::Null) => None,
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (builder_id, options_raw);
        record_invocation_feature_disabled("runtime_invocation_builder_prepare")
    }

    #[cfg(feature = "axon-pb")]
    {
        prepare_builder_with_axon_pb(builder_id, options_raw, out_prepared_id, out_prepared_json)
    }
}

/// Free a mutable builder handle.
#[no_mangle]
pub extern "C" fn runtime_invocation_builder_free(builder_id: InvocationBuilderId) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = builder_id;
        record_invocation_feature_disabled("runtime_invocation_builder_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        remove_builder(builder_id);
        clear_last_error();
        RUNTIME_OK
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
pub unsafe extern "C" fn runtime_invocation_prepare(
    handle: RuntimeHandle,
    invocation_json: *const c_char,
    options_json: *const c_char,
    out_prepared_id: *mut PreparedInvocationId,
    out_prepared_json: *mut *mut c_char,
) -> i32 {
    if out_prepared_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_prepare: out_prepared_id pointer is null",
        );
    }
    if out_prepared_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_prepare: out_prepared_json pointer is null",
        );
    }
    unsafe {
        *out_prepared_id = 0;
        *out_prepared_json = std::ptr::null_mut();
    }
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_prepare: handle {handle} is not registered"),
        );
    }
    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "runtime_invocation_prepare: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_prepare: invocation_json is not valid UTF-8",
            );
        }
    };
    let options_raw = match read_optional_cstr(options_json) {
        Ok(value) => value,
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_prepare: options_json is not valid UTF-8",
            );
        }
        Err(StringError::Null) => None,
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (raw, options_raw);
        record_invocation_feature_disabled("runtime_invocation_prepare")
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
pub unsafe extern "C" fn runtime_invocation_sign_prepared(
    prepared_id: PreparedInvocationId,
    signature_json: *const c_char,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
) -> i32 {
    if out_signed_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_sign_prepared: out_signed_id pointer is null",
        );
    }
    if out_signed_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_sign_prepared: out_signed_json pointer is null",
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
                "runtime_invocation_sign_prepared: signature_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_sign_prepared: signature_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (prepared_id, raw);
        record_invocation_feature_disabled("runtime_invocation_sign_prepared")
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
pub unsafe extern "C" fn runtime_invocation_sign_prepared_local(
    prepared_id: PreparedInvocationId,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
) -> i32 {
    if out_signed_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_sign_prepared_local: out_signed_id pointer is null",
        );
    }
    if out_signed_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_sign_prepared_local: out_signed_json pointer is null",
        );
    }
    unsafe {
        *out_signed_id = 0;
        *out_signed_json = std::ptr::null_mut();
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = prepared_id;
        record_invocation_feature_disabled("runtime_invocation_sign_prepared_local")
    }

    #[cfg(feature = "axon-pb")]
    {
        sign_prepared_local_with_axon_pb(prepared_id, out_signed_id, out_signed_json)
    }
}

/// Submit a signed Invocation and return an observer handle.
///
/// # Safety
/// Output pointers must be non-null caller-owned pointers.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_submit_signed_handle(
    handle: RuntimeHandle,
    signed_id: SignedInvocationId,
    out_invocation_handle_id: *mut InvocationHandleId,
    out_submitted_json: *mut *mut c_char,
) -> i32 {
    if out_invocation_handle_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_submit_signed_handle: out_invocation_handle_id pointer is null",
        );
    }
    if out_submitted_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_submit_signed_handle: out_submitted_json pointer is null",
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
                    "runtime_invocation_submit_signed_handle: handle {handle} is not registered"
                ),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, signed_id);
        record_invocation_feature_disabled("runtime_invocation_submit_signed_handle")
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
pub unsafe extern "C" fn runtime_invocation_handle_await(
    handle: RuntimeHandle,
    invocation_handle_id: InvocationHandleId,
    out_result_json: *mut *mut c_char,
) -> i32 {
    if out_result_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_handle_await: out_result_json pointer is null",
        );
    }
    unsafe { *out_result_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_handle_await: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, invocation_handle_id);
        record_invocation_feature_disabled("runtime_invocation_handle_await")
    }

    #[cfg(feature = "axon-pb")]
    {
        invocation_handle_await_with_axon_pb(
            session.binding(handle),
            invocation_handle_id,
            out_result_json,
        )
    }
}

/// Cancel a submitted Invocation handle if it has not already reached terminal.
///
/// # Safety
/// `reason_json` may be null; `out_cancel_json` must be a non-null caller-owned
/// pointer.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_handle_cancel(
    handle: RuntimeHandle,
    invocation_handle_id: InvocationHandleId,
    reason_json: *const c_char,
    out_cancel_json: *mut *mut c_char,
) -> i32 {
    if out_cancel_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_handle_cancel: out_cancel_json pointer is null",
        );
    }
    unsafe { *out_cancel_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_handle_cancel: handle {handle} is not registered"),
            );
        }
    };
    let reason_raw = match read_optional_cstr(reason_json) {
        Ok(value) => value,
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_handle_cancel: reason_json is not valid UTF-8",
            );
        }
        Err(StringError::Null) => None,
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, invocation_handle_id, reason_raw);
        record_invocation_feature_disabled("runtime_invocation_handle_cancel")
    }

    #[cfg(feature = "axon-pb")]
    {
        invocation_handle_cancel_with_axon_pb(
            session.binding(handle),
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
pub unsafe extern "C" fn runtime_invocation_handle_events(
    handle: RuntimeHandle,
    invocation_handle_id: InvocationHandleId,
    out_events_json: *mut *mut c_char,
) -> i32 {
    if out_events_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_handle_events: out_events_json pointer is null",
        );
    }
    unsafe { *out_events_json = std::ptr::null_mut() };
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_handle_events: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, invocation_handle_id);
        record_invocation_feature_disabled("runtime_invocation_handle_events")
    }

    #[cfg(feature = "axon-pb")]
    {
        invocation_handle_events_with_axon_pb(
            session.binding(handle),
            invocation_handle_id,
            out_events_json,
        )
    }
}

/// Free a submitted Invocation handle.
#[no_mangle]
pub extern "C" fn runtime_invocation_handle_free(
    handle: RuntimeHandle,
    invocation_handle_id: InvocationHandleId,
) -> i32 {
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_handle_free: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, invocation_handle_id);
        record_invocation_feature_disabled("runtime_invocation_handle_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        match remove_invocation_handle_for_owner(session.binding(handle), invocation_handle_id) {
            Ok(Some(_)) => {
                clear_last_error();
                RUNTIME_OK
            }
            Ok(None) => record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_free: invocation handle {invocation_handle_id} is not registered"
                ),
            ),
            Err(RegistryOwnerMismatch) => record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_free: invocation handle {invocation_handle_id} does not belong to handle {handle}"
                ),
            ),
        }
    }
}

/// Free a prepared Invocation handle.
#[no_mangle]
pub extern "C" fn runtime_prepared_invocation_free(prepared_id: PreparedInvocationId) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = prepared_id;
        record_invocation_feature_disabled("runtime_prepared_invocation_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        remove_prepared(prepared_id);
        clear_last_error();
        RUNTIME_OK
    }
}

/// Free a signed Invocation handle.
#[no_mangle]
pub extern "C" fn runtime_signed_invocation_free(signed_id: SignedInvocationId) -> i32 {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = signed_id;
        record_invocation_feature_disabled("runtime_signed_invocation_free")
    }

    #[cfg(feature = "axon-pb")]
    {
        remove_signed(signed_id);
        clear_last_error();
        RUNTIME_OK
    }
}

/// Open a complete Axon server-stream Invocation through the local
/// daemon.
///
/// `invocation_json` has the same shape as `runtime_invocation_invoke`.
/// Each daemon `InvokeStreamChunk` is delivered to `on_chunk` as a
/// JSON summary. The returned `stream_id` may be passed to
/// `runtime_invocation_stream_cancel`; cancellation submits a signed
/// canonical `invocation.cancel` command while the original stream
/// remains registered and draining toward its receipt-backed terminal.
///
/// # Safety
/// - `handle` must be a valid handle from `runtime_init`.
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `on_chunk` must be a valid function pointer for the lifetime of
///   the stream.
/// - `out_stream_id` must be a non-null pointer owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_stream_open(
    handle: RuntimeHandle,
    invocation_json: *const c_char,
    on_chunk: Option<InvocationStreamCallback>,
    user_data: *mut c_void,
    out_stream_id: *mut InvocationStreamId,
) -> i32 {
    if out_stream_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_stream_open: out_stream_id pointer is null",
        );
    }
    unsafe { *out_stream_id = 0 };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_stream_open: handle {handle} is not registered"),
            );
        }
    };

    let Some(on_chunk) = on_chunk else {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_stream_open: on_chunk callback is null",
        );
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "runtime_invocation_stream_open: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_stream_open: invocation_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw, on_chunk, user_data);
        record_invocation_feature_disabled("runtime_invocation_stream_open")
    }

    #[cfg(feature = "axon-pb")]
    {
        stream_open_with_axon_pb(handle, session, raw, on_chunk, user_data, out_stream_id)
    }
}

/// Cancel a stream opened by `runtime_invocation_stream_open`.
///
/// `stream_id` must identify a stream currently registered to `handle`.
/// Cancellation idempotency is owned by the registered provider cancellation
/// state machine; unknown ids are invalid lifecycle state.
///
/// # Safety
/// `handle` may be any value; the function does not dereference
/// caller memory.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_stream_cancel(
    handle: RuntimeHandle,
    stream_id: InvocationStreamId,
) -> i32 {
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_stream_cancel: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, stream_id);
        record_invocation_feature_disabled("runtime_invocation_stream_cancel")
    }

    #[cfg(feature = "axon-pb")]
    {
        request_stream_cancellation(
            session.binding(handle),
            stream_id,
            "runtime_invocation_stream_cancel",
        )
    }
}

/// Close and release a stream handle.
///
/// This is a local resource close; daemon terminal frames are still
/// delivered through the callback path when available before close. `stream_id`
/// must identify a stream currently registered to `handle`.
///
/// # Safety
/// `handle` and `stream_id` must have been returned by this C ABI.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_stream_close(
    handle: RuntimeHandle,
    stream_id: InvocationStreamId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_stream_close: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = stream_id;
        record_invocation_feature_disabled("runtime_invocation_stream_close")
    }

    #[cfg(feature = "axon-pb")]
    {
        stream_close_with_axon_pb(handle, stream_id)
    }
}

/// Open a complete Axon InvokeBidi session through the local daemon.
///
/// `invocation_json` uses the same complete seven-tuple shape as
/// `runtime_invocation_invoke`, with one additional required field:
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
/// frames are sent with `runtime_invocation_bidi_send`.
///
/// # Safety
/// - `handle` must be a valid handle from `runtime_init`.
/// - `invocation_json` must be a valid UTF-8 C string.
/// - `on_frame` must be a valid function pointer for the lifetime of
///   the session.
/// - `out_bidi_id` must be a non-null pointer owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_bidi_open(
    handle: RuntimeHandle,
    invocation_json: *const c_char,
    on_frame: Option<InvocationBidiCallback>,
    user_data: *mut c_void,
    out_bidi_id: *mut InvocationBidiId,
) -> i32 {
    if out_bidi_id.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_bidi_open: out_bidi_id pointer is null",
        );
    }
    unsafe { *out_bidi_id = 0 };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_bidi_open: handle {handle} is not registered"),
            );
        }
    };

    let Some(on_frame) = on_frame else {
        return record_invocation_error(
            ERR_NULL_POINTER,
            "runtime_invocation_bidi_open: on_frame callback is null",
        );
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "runtime_invocation_bidi_open: invocation_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_bidi_open: invocation_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw, on_frame, user_data);
        record_invocation_feature_disabled("runtime_invocation_bidi_open")
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
/// {"type":"binary_chunk","stream_id":1,"data_base64":"...","pts":0,"mac_base64":"..."}
/// {"type":"control","eof":true,"mac_base64":"..."}
/// {"type":"control","pty_resize":{"cols":120,"rows":40},"mac_base64":"..."}
/// {"type":"control","pty_signal":2,"mac_base64":"..."}
/// ```
///
/// The ABI assigns the monotonic up-direction sequence number. `mac_base64`
/// is required and must decode to the 32-byte N≥1 frame-chain tag defined by
/// Axon InvokeBidi.
///
/// # Safety
/// `frame_json` must be a valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_bidi_send(
    handle: RuntimeHandle,
    bidi_id: InvocationBidiId,
    frame_json: *const c_char,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_bidi_send: handle {handle} is not registered"),
        );
    }

    let raw = match read_cstr(frame_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            return record_invocation_error(
                ERR_NULL_POINTER,
                "runtime_invocation_bidi_send: frame_json pointer is null",
            );
        }
        Err(StringError::NotUtf8) => {
            return record_invocation_error(
                ERR_INVALID_UTF8,
                "runtime_invocation_bidi_send: frame_json is not valid UTF-8",
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (bidi_id, raw);
        record_invocation_feature_disabled("runtime_invocation_bidi_send")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_send_with_axon_pb(handle, bidi_id, raw)
    }
}

/// Legacy close-send entry point for an InvokeBidi session.
///
/// This ABI shape cannot carry the required N≥1 frame-chain MAC. It therefore
/// fails closed with `ERR_NOT_IMPLEMENTED`; callers that need graceful EOF must
/// send an EOF control frame through `runtime_invocation_bidi_send` with
/// `mac_base64`.
///
/// # Safety
/// `handle` and `bidi_id` must have been returned by this C ABI. This function
/// does not close the local send side. Callers must still close or cancel the
/// session handle when receive processing is complete.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_bidi_close_send(
    handle: RuntimeHandle,
    bidi_id: InvocationBidiId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_bidi_close_send: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        record_invocation_feature_disabled("runtime_invocation_bidi_close_send")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_close_send_with_axon_pb(handle, bidi_id)
    }
}

/// Close an InvokeBidi session and drop the local up-direction sender.
///
/// This function does not synthesize a graceful EOF because doing so would
/// require fabricating a frame-chain MAC. Send an explicit EOF control frame
/// first if graceful half-close is required.
///
/// `bidi_id` must identify a session currently registered to `handle`.
///
/// # Safety
/// `handle` must be a live handle returned by this FFI and not
/// used concurrently from another thread during this call.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_bidi_close(
    handle: RuntimeHandle,
    bidi_id: InvocationBidiId,
) -> i32 {
    if get(handle).is_none() {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_bidi_close: handle {handle} is not registered"),
        );
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        record_invocation_feature_disabled("runtime_invocation_bidi_close")
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_close_with_axon_pb(handle, bidi_id)
    }
}

/// Request canonical cancellation of an InvokeBidi session.
///
/// Cancellation submits a signed `invocation.cancel` command and keeps
/// the local session registered so its reader can drain the canonical
/// receipt-backed terminal. Unknown ids are invalid lifecycle state.
///
/// # Safety
/// `handle` must be a live handle returned by this FFI. Concurrent
/// cancellation requests for the same session are serialized and
/// deduplicated by the provider cancellation state machine.
#[no_mangle]
pub unsafe extern "C" fn runtime_invocation_bidi_cancel(
    handle: RuntimeHandle,
    bidi_id: InvocationBidiId,
) -> i32 {
    let session = match get(handle) {
        Some(session) => session,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_bidi_cancel: handle {handle} is not registered"),
            );
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, bidi_id);
        record_invocation_feature_disabled("runtime_invocation_bidi_cancel")
    }

    #[cfg(feature = "axon-pb")]
    {
        request_bidi_cancellation(
            session.binding(handle),
            bidi_id,
            "runtime_invocation_bidi_cancel",
        )
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
        "invocation_ready": report.invocation_ready,
        "directory_ready": report.directory_ready,
        "trust_ready": report.trust_ready,
        "runtime_ready": report.runtime_ready,
        "version": env!("CARGO_PKG_VERSION"),
        "abi_version": crate::ffi::RUNTIME_ABI_VERSION,
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
        "abi_version": crate::ffi::RUNTIME_ABI_VERSION,
        "control_endpoint": session.control_path,
        "invocation_endpoint": report.invocation_endpoint,
        "checks": report.checks(),
        "diagnostics": report.diagnostics,
        "descriptor_catalog": runtime_descriptor_catalog_json(session),
    })
}

#[cfg(feature = "axon-pb")]
fn runtime_descriptor_catalog_json(
    session: &crate::ffi::client::handle::ClientSession,
) -> serde_json::Value {
    RuntimeDescriptorResolutionProvider::diagnostics_catalog_json(runtime_owner_ura_from_session(
        session,
    ))
}

#[cfg(feature = "axon-pb")]
fn runtime_owner_ura_from_session(
    session: &crate::ffi::client::handle::ClientSession,
) -> std::result::Result<String, String> {
    let discovery = runtime_discovery_from_session(session)?;
    runtime_owner_ura_from_discovery(&discovery)
}

#[cfg(feature = "axon-pb")]
fn runtime_discovery_from_session(
    session: &crate::ffi::client::handle::ClientSession,
) -> std::result::Result<crate::daemon::control::discovery::ControlDiscovery, String> {
    let session_control_path = PathBuf::from(&session.control_path);
    let control_path =
        crate::daemon::control::discovery::resolve_control_json_path(&session_control_path)
            .map_err(|error| format!("resolve control discovery path: {error}"))?;
    crate::daemon::control::discovery::read(&control_path)
        .map_err(|error| format!("read control discovery {}: {error}", control_path.display()))?
        .ok_or_else(|| {
            format!(
                "control discovery {} does not exist",
                control_path.display()
            )
        })
}

#[cfg(feature = "axon-pb")]
fn runtime_owner_ura_from_discovery(
    discovery: &crate::daemon::control::discovery::ControlDiscovery,
) -> std::result::Result<String, String> {
    let identity = discovery
        .daemon_identity
        .as_ref()
        .ok_or_else(|| "control discovery has no daemon_identity".to_string())?;
    let realm = identity.realm.trim();
    if realm.is_empty() {
        return Err("control discovery daemon_identity.realm is empty".to_string());
    }
    match identity.mode.trim() {
        "hub" => Ok(crate::core::ura::hub_ura(realm)),
        "device" | "both" => {
            let node_id = identity
                .node_id
                .as_deref()
                .map(str::trim)
                .filter(|node_id| !node_id.is_empty())
                .ok_or_else(|| {
                    "control discovery device daemon_identity.node_id is empty".to_string()
                })?;
            Ok(crate::core::ura::device_ura(realm, node_id))
        }
        other => Err(format!(
            "control discovery daemon_identity.mode {other:?} cannot resolve a runtime owner URA"
        )),
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSessionCallerAuthority {
    runtime_owner_ura: String,
    paired_user_ura: Option<String>,
}

#[cfg(feature = "axon-pb")]
impl RuntimeSessionCallerAuthority {
    fn from_session(
        session: &crate::ffi::client::handle::ClientSession,
    ) -> std::result::Result<Self, String> {
        let discovery = runtime_discovery_from_session(session)?;
        let runtime_owner_ura = runtime_owner_ura_from_discovery(&discovery)?;
        let paired_user_ura = paired_user_ura_from_discovery(&discovery)?;
        Ok(Self {
            runtime_owner_ura,
            paired_user_ura,
        })
    }

    fn admit_caller(&self, caller_ura: &str) -> crate::daemon::Result<()> {
        if caller_ura == self.runtime_owner_ura {
            return Ok(());
        }
        if self.paired_user_ura.as_deref() == Some(caller_ura) {
            return Ok(());
        }
        Err(crate::daemon::DaemonError::InvalidInvocation(format!(
            "native runtime invocation caller `{caller_ura}` is not admitted by session authority \
             owner `{}`",
            self.runtime_owner_ura
        )))
    }

    fn admitted_owner_label(&self) -> String {
        match self.paired_user_ura.as_deref() {
            Some(user_ura) => format!("{} or paired user `{user_ura}`", self.runtime_owner_ura),
            None => self.runtime_owner_ura.clone(),
        }
    }
}

#[cfg(feature = "axon-pb")]
fn paired_user_ura_from_discovery(
    discovery: &crate::daemon::control::discovery::ControlDiscovery,
) -> std::result::Result<Option<String>, String> {
    let Some(identity) = discovery.daemon_identity.as_ref() else {
        return Ok(None);
    };
    match identity.mode.trim() {
        "device" | "both" => {}
        _ => return Ok(None),
    }
    let has_paired_user_signer = discovery
        .capability_flags
        .iter()
        .any(|flag| flag == crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER);
    if !has_paired_user_signer {
        return Ok(None);
    }
    let credentials = crate::daemon::persistence::config::load_credentials().map_err(|error| {
        format!("load paired credentials for session caller authority: {error}")
    })?;
    if credentials.realm_str() != identity.realm.trim() {
        return Err(format!(
            "paired credentials realm `{}` does not match session realm `{}`",
            credentials.realm_str(),
            identity.realm.trim()
        ));
    }
    let Some(node_id) = identity
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err("paired-user session authority requires daemon_identity.node_id".to_string());
    };
    if credentials.node_id.trim() != node_id {
        return Err(format!(
            "paired credentials node `{}` does not match session node `{node_id}`",
            credentials.node_id.trim()
        ));
    }
    match credentials
        .runtime_user_binding()
        .map_err(|error| format!("read paired runtime user binding: {error}"))?
    {
        crate::daemon::persistence::config::RuntimeUserBinding::Bound { user_ura } => {
            Ok(Some(user_ura))
        }
        crate::daemon::persistence::config::RuntimeUserBinding::Unbound { .. } => Err(
            "session advertises paired_user_runtime_signer without bound User credentials"
                .to_string(),
        ),
    }
}

/// Binds unsigned native-runtime calls to the daemon identity advertised by
/// the exact client session. Explicit caller signatures pass through
/// unchanged; only the session owner or the Ready-proven paired User may use
/// daemon KeyService signing.
///
/// This is the single native-provider authority boundary for unary, stream,
/// and bidi carriers. It does not generate signer material, infer a caller, or
/// weaken daemon admission.
#[cfg(feature = "axon-pb")]
struct SessionInvocationAuthority<'a> {
    session: &'a crate::ffi::client::handle::ClientSession,
}

#[cfg(feature = "axon-pb")]
impl<'a> SessionInvocationAuthority<'a> {
    fn new(session: &'a crate::ffi::client::handle::ClientSession) -> Self {
        Self { session }
    }

    fn caller_authority(&self) -> crate::daemon::Result<RuntimeSessionCallerAuthority> {
        RuntimeSessionCallerAuthority::from_session(self.session).map_err(|error| {
            crate::daemon::DaemonError::InvalidInvocation(format!(
                "resolve native runtime session authority: {error}"
            ))
        })
    }

    async fn load_admitted_caller_signer(
        caller_ura: String,
    ) -> crate::daemon::Result<Arc<dyn crate::daemon::identity::self_identity::CanonicalSigner>>
    {
        let signer_owner_ura = caller_ura.clone();
        let signer = tokio::task::spawn_blocking(move || {
            crate::daemon::identity::self_identity::load_runtime_caller_signer(
                signer_owner_ura.clone(),
            )
            .map_err(|_error| Self::caller_signer_unavailable_error(&signer_owner_ura))
        })
        .await
        .map_err(|error| {
            crate::daemon::DaemonError::InvalidInvocation(format!(
                "native runtime caller signer task failed: {error}"
            ))
        })??;
        Ok(signer)
    }

    fn caller_signer_unavailable_error(owner_ura: &str) -> crate::daemon::DaemonError {
        crate::daemon::DaemonError::InvalidInvocation(format!(
            "{CALLER_SIGNER_UNAVAILABLE_CODE}: native runtime invocation requires a caller signer \
             for `{owner_ura}`; load or provision that identity in the local key service"
        ))
    }

    async fn owner_signer(
        &self,
        caller_ura: &str,
    ) -> crate::daemon::Result<Arc<dyn crate::daemon::identity::self_identity::CanonicalSigner>>
    {
        let authority = self.caller_authority()?;
        authority.admit_caller(caller_ura)?;
        Self::load_admitted_caller_signer(caller_ura.to_string()).await
    }

    async fn cancellation_authority_for_signed(
        &self,
        signed: &crate::daemon::SignedInvocation,
    ) -> InvocationCancellationGate {
        let authority = match self.caller_authority() {
            Ok(authority) => authority,
            Err(error) => {
                return InvocationCancellationGate::Unavailable {
                    reason: format!(
                        "resolve session authority for cancellation authority: {error}"
                    ),
                };
            }
        };
        let caller_ura = signed.prepared().tuple().caller_ura.clone();
        let caller_ura = caller_ura.as_str();
        if let Err(error) = authority.admit_caller(caller_ura) {
            return InvocationCancellationGate::Unavailable {
                reason: format!(
                    "signed invocation caller `{}` is not admitted by session authority {}: {error}",
                    signed.prepared().tuple().caller_ura,
                    authority.admitted_owner_label()
                ),
            };
        }
        match Self::load_admitted_caller_signer(caller_ura.to_string()).await {
            Ok(signer) => InvocationCancellationGate::Available(
                crate::daemon::InvocationCancellationAuthority::new(signer),
            ),
            Err(error) => InvocationCancellationGate::Unavailable {
                reason: format!("load cancellation authority signer: {error}"),
            },
        }
    }

    async fn bind_with_owner_signer(
        invocation: crate::daemon::DaemonInvocation,
        signer: Arc<dyn crate::daemon::identity::self_identity::CanonicalSigner>,
    ) -> crate::daemon::Result<crate::daemon::SignedInvocation> {
        if let Some(signature) = invocation.caller_signature().cloned() {
            return Self::bind_caller_signature(invocation, signature);
        }

        invocation
            .into_draft()
            .prepare(crate::daemon::PrepareOptions::default())?
            .sign_with_canonical_signer(signer.as_ref())
            .await
    }

    async fn bind(
        &self,
        invocation: crate::daemon::DaemonInvocation,
    ) -> crate::daemon::Result<crate::daemon::SignedInvocation> {
        if let Some(signature) = invocation.caller_signature().cloned() {
            return Self::bind_caller_signature(invocation, signature);
        }

        let caller_ura = invocation.caller_ura().to_string();
        let signer = self.owner_signer(&caller_ura).await?;
        Self::bind_with_owner_signer(invocation, signer).await
    }

    async fn bind_cancellable(
        &self,
        invocation: crate::daemon::DaemonInvocation,
    ) -> crate::daemon::Result<(
        crate::daemon::SignedInvocation,
        crate::daemon::InvocationCancellationAuthority,
    )> {
        let caller_ura = invocation.caller_ura().to_string();
        let signer = self.owner_signer(&caller_ura).await?;
        let signed = Self::bind_with_owner_signer(invocation, Arc::clone(&signer)).await?;
        Ok((
            signed,
            crate::daemon::InvocationCancellationAuthority::new(signer),
        ))
    }

    fn bind_caller_signature(
        invocation: crate::daemon::DaemonInvocation,
        signature: axon_sdk::pb::axon::v1::CallerSignature,
    ) -> crate::daemon::Result<crate::daemon::SignedInvocation> {
        invocation
            .into_draft()
            .prepare(crate::daemon::PrepareOptions::default())?
            .sign_with_caller_signature(crate::daemon::CallerSignatureMaterial::new(
                signature.algorithm,
                signature.signature,
                signature.key_id_hint,
            ))
    }
}

#[cfg(feature = "axon-pb")]
fn runtime_resolve_descriptor_ref_json(
    session: &crate::ffi::client::handle::ClientSession,
    request_json: &str,
) -> Result<serde_json::Value, DescriptorResolutionError> {
    RuntimeDescriptorResolutionProvider::resolve_json(request_json, || {
        runtime_owner_ura_from_session(session)
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
    RUNTIME_OK
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
    RUNTIME_OK
}

#[cfg(not(feature = "axon-pb"))]
fn builder_output_invocation_json(
    builder_id: InvocationBuilderId,
    out_invocation_json: *mut *mut c_char,
    _consume_on_success: bool,
    function: &str,
) -> i32 {
    if out_invocation_json.is_null() {
        return record_invocation_error(
            ERR_NULL_POINTER,
            format!("{function}: out_invocation_json pointer is null"),
        );
    }
    unsafe { *out_invocation_json = std::ptr::null_mut() };
    let _ = builder_id;
    record_invocation_feature_disabled(function)
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
                    "runtime_invocation_builder_prepare: builder handle {builder_id} is not registered"
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
                format!("runtime_invocation_builder_prepare: {err}"),
            );
        }
    };
    let invocation = match builder.build_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            restore_builder(builder_id, builder);
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_builder_prepare: {err}"),
            );
        }
    };
    let material_only = options.material_only;
    let prepared = match invocation
        .into_draft()
        .prepare(options.into_prepare_options())
    {
        Ok(prepared) => prepared,
        Err(err) => {
            restore_builder(builder_id, builder);
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_builder_prepare: {err}"),
            );
        }
    };
    let id = if material_only {
        0
    } else {
        insert_prepared(prepared.clone())
    };
    let ptr = alloc_output_cstring(prepared_invocation_json(&prepared, nonzero_id(id)).to_string());
    if ptr.is_null() {
        if id != 0 {
            remove_prepared(id);
        }
        restore_builder(builder_id, builder);
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_builder_prepare: out-of-memory allocating prepared JSON",
        );
    }
    unsafe {
        *out_prepared_id = id;
        *out_prepared_json = ptr;
    }
    clear_last_error();
    RUNTIME_OK
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
                format!("runtime_invocation_invoke: {err}"),
            );
        }
    };

    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_invoke: {err}"),
            );
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("runtime_invocation_invoke: {err}"),
            );
        }
    };

    let (response, tuple, tuple_json) = match rt.block_on(async {
        let signed = SessionInvocationAuthority::new(session.as_ref())
            .bind(invocation)
            .await?;
        let tuple = signed.prepared().tuple();
        let tuple_json = invocation_json(&signed.clone().into_daemon_invocation());
        let client = crate::daemon::DaemonClient::connect(invocation_endpoint_for_session(
            session.as_ref(),
        )?)?;
        let response = client.invoke(signed).await?;
        Ok::<_, crate::daemon::DaemonError>((response, tuple, tuple_json))
    }) {
        Ok(bound) => bound,
        Err(err) => return ffi_daemon_error("runtime_invocation_invoke", err),
    };

    let outcome = match crate::daemon::InvocationOutcome::from_invoke_response(
        tuple,
        response,
        &crate::support::platform::local_daemon_grpc::CanonicalRuntimeReceiptResolver::new(),
    ) {
        Ok(outcome) => outcome,
        Err(error) => return ffi_daemon_error("runtime_invocation_invoke", error),
    };
    let output = match invocation_outcome_json_with_tuple(outcome, tuple_json) {
        Ok(output) => output,
        Err(message) => {
            return record_invocation_error(
                ERR_PROTOCOL,
                format!("runtime_invocation_invoke: {message}"),
            );
        }
    };
    let json = match serde_json::to_string(&output) {
        Ok(json) => json,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("runtime_invocation_invoke: encode response JSON failed: {err}"),
            );
        }
    };
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_invoke: out-of-memory allocating response string",
        );
    }
    unsafe { *out_receipt_json = ptr };
    clear_last_error();
    RUNTIME_OK
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
                format!("runtime_invocation_prepare: {err}"),
            );
        }
    };
    let options = match PrepareOptionsJson::parse(options_raw.as_deref()) {
        Ok(options) => options,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_prepare: {err}"),
            );
        }
    };
    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_prepare: {err}"),
            );
        }
    };
    let material_only = options.material_only;
    let prepared = match invocation
        .into_draft()
        .prepare(options.into_prepare_options())
    {
        Ok(prepared) => prepared,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_prepare: {err}"),
            );
        }
    };
    let id = if material_only {
        0
    } else {
        insert_prepared(prepared.clone())
    };
    let json = prepared_invocation_json(&prepared, nonzero_id(id)).to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        if id != 0 {
            remove_prepared(id);
        }
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_prepare: out-of-memory allocating prepared JSON",
        );
    }
    unsafe {
        *out_prepared_id = id;
        *out_prepared_json = ptr;
    }
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn commit_prepared_as_signed(
    prepared_id: PreparedInvocationId,
    signed: crate::daemon::SignedInvocation,
    out_signed_id: *mut SignedInvocationId,
    out_signed_json: *mut *mut c_char,
    function: &str,
) -> i32 {
    let ptr = alloc_output_cstring(signed_invocation_json(&signed).to_string());
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating signed JSON"),
        );
    }
    let Some(_) = remove_prepared(prepared_id) else {
        unsafe { crate::ffi::strings::runtime_string_free(ptr) };
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("{function}: prepared handle {prepared_id} disappeared"),
        );
    };
    let id = insert_signed(signed);
    unsafe {
        *out_signed_id = id;
        *out_signed_json = ptr;
    }
    clear_last_error();
    RUNTIME_OK
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
                format!("runtime_invocation_sign_prepared: {err}"),
            );
        }
    };
    let prepared = match get_prepared(prepared_id) {
        Some(prepared) => prepared,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_sign_prepared: prepared handle {prepared_id} is not registered"
                ),
            );
        }
    };
    let signed = match prepared.sign_with_caller_signature(signature) {
        Ok(signed) => signed,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_sign_prepared: {err}"),
            );
        }
    };
    commit_prepared_as_signed(
        prepared_id,
        signed,
        out_signed_id,
        out_signed_json,
        "runtime_invocation_sign_prepared",
    )
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
                    "runtime_invocation_sign_prepared_local: prepared handle {prepared_id} is not registered"
                ),
            );
        }
    };
    let signer = crate::daemon::KeyServiceProviderManagedInvocationSigner::at_default_endpoint();
    let signed = match prepared.sign_with_provider_managed_signer(&signer) {
        Ok(signed) => signed,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_sign_prepared_local: {err}"),
            );
        }
    };
    commit_prepared_as_signed(
        prepared_id,
        signed,
        out_signed_id,
        out_signed_json,
        "runtime_invocation_sign_prepared_local",
    )
}

#[cfg(feature = "axon-pb")]
fn submit_signed_handle_with_axon_pb(
    owner: RuntimeHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    signed_id: SignedInvocationId,
    out_invocation_handle_id: *mut InvocationHandleId,
    out_submitted_json: *mut *mut c_char,
) -> i32 {
    let registration = match session.resource_registration_guard(owner) {
        Ok(registration) => registration,
        Err(_) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_submit_signed_handle: handle {owner} is closing or released"
                ),
            );
        }
    };
    let signed = match remove_signed(signed_id) {
        Some(signed) => signed,
        None => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_submit_signed_handle: signed handle {signed_id} is not registered"
                ),
            );
        }
    };
    let endpoint = match invocation_endpoint_for_session(session.as_ref()) {
        Ok(endpoint) => endpoint,
        Err(err) => return ffi_daemon_error("runtime_invocation_submit_signed_handle", err),
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("runtime_invocation_submit_signed_handle: {err}"),
            );
        }
    };
    let cancellation_gate = rt.block_on(
        SessionInvocationAuthority::new(session.as_ref())
            .cancellation_authority_for_signed(&signed),
    );

    let tuple_json = invocation_json(signed.prepared().draft().invocation());
    let owner_binding = registration.binding();
    let (active, cancel_requests) = ActiveInvocationHandle::with_cancel_channel(
        owner_binding,
        tuple_json.clone(),
        cancellation_gate.clone(),
    );
    let shared = active.shared.clone();
    let invocation_handle_id = insert_invocation_handle(active);
    let submitted = match get_invocation_handle_for_owner(owner_binding, invocation_handle_id) {
        Ok(Some(handle)) => match handle.submitted_json(invocation_handle_id) {
            Ok(json) => json.to_string(),
            Err(failure) => {
                return record_invocation_error(
                    failure.abi_code,
                    format!(
                        "runtime_invocation_submit_signed_handle: {}",
                        failure.message
                    ),
                );
            }
        },
        Ok(None) | Err(_) => {
            return record_invocation_error(
                ERR_GENERIC,
                "runtime_invocation_submit_signed_handle: inserted invocation handle disappeared",
            );
        }
    };
    let ptr = alloc_output_cstring(submitted);
    if ptr.is_null() {
        let _ = remove_invocation_handle_for_owner(owner_binding, invocation_handle_id);
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_submit_signed_handle: out-of-memory allocating submitted JSON",
        );
    }

    rt.spawn(run_invocation_handle_task(
        endpoint,
        signed,
        cancellation_gate.into_authority(),
        shared,
        cancel_requests,
    ));
    drop(registration);
    unsafe {
        *out_invocation_handle_id = invocation_handle_id;
        *out_submitted_json = ptr;
    }
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_await_with_axon_pb(
    owner: ClientSessionBinding,
    invocation_handle_id: InvocationHandleId,
    out_result_json: *mut *mut c_char,
) -> i32 {
    let handle = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_await: invocation handle {invocation_handle_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_await: invocation handle {invocation_handle_id} does not belong to handle {}",
                    owner.handle
                ),
            );
        }
    };
    let json = match handle.await_result_json() {
        Ok(json) => json.to_string(),
        Err(failure) => {
            return record_invocation_error(
                failure.abi_code,
                format!("runtime_invocation_handle_await: {}", failure.message),
            );
        }
    };
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_handle_await: out-of-memory allocating result JSON",
        );
    }
    unsafe { *out_result_json = ptr };
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_cancel_with_axon_pb(
    owner: ClientSessionBinding,
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
                    "runtime_invocation_handle_cancel: invocation handle {invocation_handle_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_cancel: invocation handle {invocation_handle_id} does not belong to handle {}",
                    owner.handle
                ),
            );
        }
    };
    let outcome = handle.cancel(reason_raw);
    let json = serde_json::json!({
        "handle_id": invocation_handle_id,
        "request_accepted": outcome.request_accepted,
        "deduplicated": outcome.deduplicated,
        "cancelled": outcome.cancelled,
        "state": outcome.state.as_str(),
        "terminal": outcome.terminal,
        "rejection": outcome.rejection,
    })
    .to_string();
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_handle_cancel: out-of-memory allocating cancel JSON",
        );
    }
    unsafe { *out_cancel_json = ptr };
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn invocation_handle_events_with_axon_pb(
    owner: ClientSessionBinding,
    invocation_handle_id: InvocationHandleId,
    out_events_json: *mut *mut c_char,
) -> i32 {
    let handle = match get_invocation_handle_for_owner(owner, invocation_handle_id) {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_events: invocation handle {invocation_handle_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_handle_events: invocation handle {invocation_handle_id} does not belong to handle {}",
                    owner.handle
                ),
            );
        }
    };
    let json = match handle.events_json(invocation_handle_id) {
        Ok(json) => json.to_string(),
        Err(failure) => {
            return record_invocation_error(
                failure.abi_code,
                format!("runtime_invocation_handle_events: {}", failure.message),
            );
        }
    };
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        return record_invocation_error(
            ERR_GENERIC,
            "runtime_invocation_handle_events: out-of-memory allocating events JSON",
        );
    }
    unsafe { *out_events_json = ptr };
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
async fn run_invocation_handle_task(
    endpoint: PathBuf,
    signed: crate::daemon::SignedInvocation,
    cancellation_authority: Option<crate::daemon::InvocationCancellationAuthority>,
    shared: Arc<InvocationHandleShared>,
    mut cancel_requests: tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    let client = match crate::daemon::RuntimeClient::connect(endpoint) {
        Ok(client) => client,
        Err(err) => {
            shared.mark_observation_failed(invocation_observation_failure(&err));
            return;
        }
    };
    let client = match cancellation_authority {
        Some(authority) => client.with_cancellation_authority(authority),
        None => client,
    };
    let mut submission = Box::pin(client.submit_signed(signed.clone()));
    let mut cancel_request = None;
    let mut submission_observed = false;

    loop {
        tokio::select! {
            result = &mut submission, if !submission_observed => {
                submission_observed = true;
                match result {
                    Ok(handle) => {
                        if let Err(failure) = shared.observe_canonical_outcome(handle.await_outcome()) {
                            shared.mark_observation_failed(failure);
                        }
                    }
                    Err(err) => shared.mark_observation_failed(invocation_observation_failure(&err)),
                }
                if cancel_request.is_none() {
                    return;
                }
            }
            reason = cancel_requests.recv(), if cancel_request.is_none() => {
                let Some(reason) = reason else {
                    continue;
                };
                let client = client.clone();
                let signed = signed.clone();
                cancel_request = Some(Box::pin(async move {
                    client.request_cancel_signed(signed, reason).await
                }));
            }
            result = async {
                match cancel_request.as_mut() {
                    Some(request) => Some(request.await),
                    None => std::future::pending().await,
                }
            }, if cancel_request.is_some() => {
                cancel_request = None;
                match result.expect("cancel request branch is enabled only with a future") {
                    Ok(handle) => {
                        if let Err(failure) = shared.observe_cancel_command_outcome(handle.await_outcome()) {
                            shared.mark_cancel_request_failed(failure);
                        }
                    }
                    Err(err) => shared.mark_cancel_request_failed(invocation_observation_failure(&err)),
                }
                if submission_observed {
                    return;
                }
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
fn stream_open_with_axon_pb(
    handle: RuntimeHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    raw: &str,
    on_chunk: InvocationStreamCallback,
    user_data: *mut c_void,
    out_stream_id: *mut InvocationStreamId,
) -> i32 {
    let registration = match session.resource_registration_guard(handle) {
        Ok(registration) => registration,
        Err(_) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_stream_open: handle {handle} is closing or released"),
            );
        }
    };
    let spec = match InvocationJson::parse(raw) {
        Ok(spec) => spec,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_stream_open: {err}"),
            );
        }
    };

    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_stream_open: {err}"),
            );
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("runtime_invocation_stream_open: {err}"),
            );
        }
    };

    let (stream, cancellation) = match rt.block_on(async {
        let (signed, cancellation_authority) = SessionInvocationAuthority::new(session.as_ref())
            .bind_cancellable(invocation)
            .await?;
        let endpoint = invocation_endpoint_for_session(session.as_ref())?;
        let client = crate::daemon::DaemonClient::connect(endpoint.clone())?;
        let stream = client.invoke_stream(signed.clone()).await?;
        Ok::<_, crate::daemon::DaemonError>((
            stream,
            Arc::new(ProviderCancellationControl::runtime(
                endpoint,
                signed,
                cancellation_authority,
            )),
        ))
    }) {
        Ok(opened) => opened,
        Err(err) => return ffi_daemon_error("runtime_invocation_stream_open", err),
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(STREAM_CALLBACK_QUEUE_CAPACITY);
    let callback_user_data = CallbackUserData(user_data);
    let dispatcher = std::thread::Builder::new()
        .name("easynet-inv-stream-callback".to_string())
        .spawn(move || dispatch_stream_callbacks(rx, on_chunk, callback_user_data));
    if let Err(err) = dispatcher {
        return record_invocation_error(
            ERR_GENERIC,
            format!("runtime_invocation_stream_open: spawn callback dispatcher failed: {err}"),
        );
    }

    let cancel = tokio_util::sync::CancellationToken::new();
    let owner = registration.binding();
    let stream_id = insert_stream(ActiveInvocationStream::new(
        owner,
        cancel.clone(),
        cancellation,
    ));
    rt.spawn(run_stream_reader(stream_id, stream, cancel, tx));
    drop(registration);

    unsafe { *out_stream_id = stream_id };
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn bidi_open_with_axon_pb(
    handle: RuntimeHandle,
    session: std::sync::Arc<crate::ffi::client::handle::ClientSession>,
    raw: &str,
    on_frame: InvocationBidiCallback,
    user_data: *mut c_void,
    out_bidi_id: *mut InvocationBidiId,
) -> i32 {
    let registration = match session.resource_registration_guard(handle) {
        Ok(registration) => registration,
        Err(_) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_bidi_open: handle {handle} is closing or released"),
            );
        }
    };
    let spec = match InvocationJson::parse(raw) {
        Ok(spec) => spec,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_bidi_open: {err}"),
            );
        }
    };
    if spec.bidi_streams.is_empty() {
        return record_invocation_error(
            ERR_INVALID_ARG,
            "runtime_invocation_bidi_open: bidi_streams must not be empty",
        );
    }

    let streams = spec.bidi_streams.clone();
    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_bidi_open: {err}"),
            );
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("runtime_invocation_bidi_open: {err}"),
            );
        }
    };

    let (session, cancellation) = match rt.block_on(async {
        let (signed, cancellation_authority) = SessionInvocationAuthority::new(session.as_ref())
            .bind_cancellable(invocation)
            .await?;
        let endpoint = invocation_endpoint_for_session(session.as_ref())?;
        let client = crate::daemon::DaemonClient::connect(endpoint.clone())?;
        let bidi = client.invoke_bidi(signed.clone(), streams).await?;
        Ok::<_, crate::daemon::DaemonError>((
            bidi,
            Arc::new(ProviderCancellationControl::runtime(
                endpoint,
                signed,
                cancellation_authority,
            )),
        ))
    }) {
        Ok(opened) => opened,
        Err(err) => return ffi_daemon_error("runtime_invocation_bidi_open", err),
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
            format!("runtime_invocation_bidi_open: spawn callback dispatcher failed: {err}"),
        );
    }

    let (ability, up_tx, down) = session.into_parts();
    let cancel = tokio_util::sync::CancellationToken::new();
    let owner = registration.binding();
    let bidi_id = insert_bidi(ActiveInvocationBidi::new(
        owner,
        ability,
        up_tx,
        cancel.clone(),
        cancellation,
    ));
    rt.spawn(run_bidi_down_reader(bidi_id, down, cancel, callback_tx));
    drop(registration);

    unsafe { *out_bidi_id = bidi_id };
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn bidi_send_with_axon_pb(handle: RuntimeHandle, bidi_id: InvocationBidiId, raw: &str) -> i32 {
    let frame = match parse_bidi_up_frame_json(raw) {
        Ok(frame) => frame,
        Err(err) => {
            return record_invocation_error(
                ERR_INVALID_ARG,
                format!("runtime_invocation_bidi_send: {err}"),
            );
        }
    };
    let Some(owner) = binding_for_handle(handle) else {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_bidi_send: handle {handle} is not registered"),
        );
    };
    let session = match get_bidi_for_handle(owner, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("runtime_invocation_bidi_send: bidi session {bidi_id} is not registered"),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_bidi_send: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            );
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            return record_invocation_error(
                ERR_GENERIC,
                format!("runtime_invocation_bidi_send: {err}"),
            );
        }
    };

    let up_frame = match session.reserve_up_frame(frame) {
        Ok(up_frame) => up_frame,
        Err(BidiLocalSendClosed) => {
            return record_invocation_error(
                ERR_CANCELLED,
                format!(
                    "runtime_invocation_bidi_send: bidi session {} for {} is locally half-closed",
                    bidi_id, session.ability
                ),
            );
        }
    };

    let send_code = send_bidi_up_frame(
        rt,
        "runtime_invocation_bidi_send",
        bidi_id,
        &session,
        up_frame,
    );
    if send_code != RUNTIME_OK {
        let _ = remove_bidi_for_handle(owner, bidi_id);
        return send_code;
    }
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn stream_close_with_axon_pb(handle: RuntimeHandle, stream_id: InvocationStreamId) -> i32 {
    let Some(owner) = binding_for_handle(handle) else {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_stream_close: handle {handle} is not registered"),
        );
    };
    release_stream_with_reader_cancel(owner, stream_id, "runtime_invocation_stream_close")
}

#[cfg(feature = "axon-pb")]
fn request_stream_cancellation(
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
    function: &str,
) -> i32 {
    request_registered_provider_cancellation(
        get_stream_for_handle(owner, stream_id),
        function,
        "stream",
        stream_id,
    )
}

#[cfg(feature = "axon-pb")]
fn release_stream_with_reader_cancel(
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
    function: &str,
) -> i32 {
    match remove_stream_for_handle(owner, stream_id) {
        Ok(Some(stream)) => {
            if !stream.reader_finished() {
                stream.reader_cancel.cancel();
            }
            clear_last_error();
            RUNTIME_OK
        }
        Ok(None) => unregistered_invocation_resource_error(function, "stream", stream_id),
        Err(RegistryOwnerMismatch) => record_invocation_error(
            ERR_INVALID_HANDLE,
            format!(
                "{function}: stream {stream_id} does not belong to handle {}",
                owner.handle
            ),
        ),
    }
}

#[cfg(feature = "axon-pb")]
fn request_bidi_cancellation(
    owner: ClientSessionBinding,
    bidi_id: InvocationBidiId,
    function: &str,
) -> i32 {
    request_registered_provider_cancellation(
        get_bidi_for_handle(owner, bidi_id),
        function,
        "bidi session",
        bidi_id,
    )
}

#[cfg(feature = "axon-pb")]
fn request_registered_provider_cancellation<T>(
    resource: Result<Option<Arc<T>>, RegistryOwnerMismatch>,
    function: &str,
    resource_kind: &str,
    resource_id: u64,
) -> i32
where
    T: ProviderCancellableResource,
{
    let resource = match resource {
        Ok(Some(resource)) => resource,
        Ok(None) => {
            return unregistered_invocation_resource_error(function, resource_kind, resource_id);
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!("{function}: {resource_kind} {resource_id} does not belong to this handle"),
            );
        }
    };

    match resource.cancellation().request() {
        Ok(()) => {
            clear_last_error();
            RUNTIME_OK
        }
        Err(error) => ffi_provider_cancellation_error(function, error),
    }
}

#[cfg(feature = "axon-pb")]
fn unregistered_invocation_resource_error(
    function: &str,
    resource_kind: &str,
    resource_id: u64,
) -> i32 {
    record_invocation_error(
        ERR_INVALID_HANDLE,
        format!("{function}: {resource_kind} {resource_id} is not registered"),
    )
}

#[cfg(feature = "axon-pb")]
fn bidi_close_send_with_axon_pb(handle: RuntimeHandle, bidi_id: InvocationBidiId) -> i32 {
    let Some(owner) = binding_for_handle(handle) else {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_bidi_close_send: handle {handle} is not registered"),
        );
    };
    let _session = match get_bidi_for_handle(owner, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_bidi_close_send: bidi session {bidi_id} is not registered"
                ),
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_bidi_close_send: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            );
        }
    };
    record_invocation_error(
        ERR_NOT_IMPLEMENTED,
        "runtime_invocation_bidi_close_send: close-send cannot attach the required 32-byte frame-chain MAC; send an EOF control frame through runtime_invocation_bidi_send with mac_base64",
    )
}

#[cfg(feature = "axon-pb")]
fn bidi_close_with_axon_pb(handle: RuntimeHandle, bidi_id: InvocationBidiId) -> i32 {
    let Some(owner) = binding_for_handle(handle) else {
        return record_invocation_error(
            ERR_INVALID_HANDLE,
            format!("runtime_invocation_bidi_close: handle {handle} is not registered"),
        );
    };
    let session = match remove_bidi_for_handle(owner, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return unregistered_invocation_resource_error(
                "runtime_invocation_bidi_close",
                "bidi session",
                bidi_id,
            );
        }
        Err(RegistryOwnerMismatch) => {
            return record_invocation_error(
                ERR_INVALID_HANDLE,
                format!(
                    "runtime_invocation_bidi_close: bidi session {bidi_id} does not belong to handle {handle}"
                ),
            );
        }
    };
    session.reader_cancel.cancel();
    clear_last_error();
    RUNTIME_OK
}

#[cfg(feature = "axon-pb")]
fn send_bidi_up_frame(
    rt: &'static tokio::runtime::Runtime,
    function: &str,
    bidi_id: InvocationBidiId,
    session: &ActiveInvocationBidi,
    up_frame: axon_sdk::pb::axon::v1::InvokeBidiUp,
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
    RUNTIME_OK
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
trait CanonicalCancellationCommandSubmitter: Send + Sync {
    fn submit(&self, reason: &str) -> Result<(), ProviderCancellationError>;
}

#[cfg(feature = "axon-pb")]
struct RuntimeCancellationCommandSubmitter {
    endpoint: PathBuf,
    signed_invocation: crate::daemon::SignedInvocation,
    authority: crate::daemon::InvocationCancellationAuthority,
}

#[cfg(feature = "axon-pb")]
impl CanonicalCancellationCommandSubmitter for RuntimeCancellationCommandSubmitter {
    fn submit(&self, reason: &str) -> Result<(), ProviderCancellationError> {
        let runtime = lib_runtime()
            .map_err(|error| ProviderCancellationError::RuntimeUnavailable(error.to_string()))?;
        let handle = runtime
            .block_on(async {
                let client = crate::daemon::RuntimeClient::connect(self.endpoint.clone())?
                    .with_cancellation_authority(self.authority.clone());
                client
                    .request_cancel_signed(self.signed_invocation.clone(), reason.to_string())
                    .await
            })
            .map_err(ProviderCancellationError::from_daemon)?;
        let result = handle.result();
        if let Some(error) = &result.error {
            return Err(ProviderCancellationError::CommandRejected(format!(
                "{} at {}: {}",
                error.code, error.stage, error.message
            )));
        }
        if result.terminal_state != "Completed" {
            return Err(ProviderCancellationError::CommandRejected(format!(
                "invocation.cancel completed in terminal state {}",
                result.terminal_state
            )));
        }
        let acknowledgement: ProviderCancellationAcknowledgement =
            serde_json::from_slice(&result.output).map_err(|error| {
                ProviderCancellationError::InvalidAcknowledgement(format!(
                    "decode invocation.cancel acknowledgement: {error}"
                ))
            })?;
        if !acknowledgement.accepted {
            return Err(ProviderCancellationError::CommandRejected(
                "invocation.cancel acknowledgement was not accepted".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "axon-pb")]
#[derive(serde::Deserialize)]
struct ProviderCancellationAcknowledgement {
    accepted: bool,
}

#[cfg(feature = "axon-pb")]
#[derive(Clone, Debug)]
enum ProviderCancellationError {
    RuntimeUnavailable(String),
    Daemon { code: i32, message: String },
    CommandRejected(String),
    InvalidAcknowledgement(String),
}

#[cfg(feature = "axon-pb")]
impl ProviderCancellationError {
    fn from_daemon(error: crate::daemon::DaemonError) -> Self {
        Self::Daemon {
            code: ffi_code_for_daemon_error(&error),
            message: error.to_string(),
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderCancellationPhase {
    Ready,
    Submitting,
    Accepted,
    Rejected,
}

#[cfg(feature = "axon-pb")]
struct ProviderCancellationState {
    phase: ProviderCancellationPhase,
    rejection: Option<ProviderCancellationError>,
    waiting_callers: usize,
}

#[cfg(feature = "axon-pb")]
struct ProviderCancellationControl {
    submitter: Arc<dyn CanonicalCancellationCommandSubmitter>,
    state: Mutex<ProviderCancellationState>,
    state_changed: Condvar,
}

#[cfg(feature = "axon-pb")]
impl ProviderCancellationControl {
    fn runtime(
        endpoint: PathBuf,
        signed_invocation: crate::daemon::SignedInvocation,
        authority: crate::daemon::InvocationCancellationAuthority,
    ) -> Self {
        Self::with_submitter(Arc::new(RuntimeCancellationCommandSubmitter {
            endpoint,
            signed_invocation,
            authority,
        }))
    }

    fn with_submitter(submitter: Arc<dyn CanonicalCancellationCommandSubmitter>) -> Self {
        Self {
            submitter,
            state: Mutex::new(ProviderCancellationState {
                phase: ProviderCancellationPhase::Ready,
                rejection: None,
                waiting_callers: 0,
            }),
            state_changed: Condvar::new(),
        }
    }

    fn request(&self) -> Result<(), ProviderCancellationError> {
        loop {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match state.phase {
                ProviderCancellationPhase::Accepted => return Ok(()),
                ProviderCancellationPhase::Rejected => {
                    return Err(state
                        .rejection
                        .clone()
                        .expect("rejected cancellation must retain its failure"));
                }
                ProviderCancellationPhase::Ready => {
                    state.phase = ProviderCancellationPhase::Submitting;
                    drop(state);

                    let result = self.submitter.submit(PROVIDER_CANCEL_REASON);
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match &result {
                        Ok(()) => state.phase = ProviderCancellationPhase::Accepted,
                        Err(error) => {
                            state.phase = ProviderCancellationPhase::Rejected;
                            state.rejection = Some(error.clone());
                        }
                    }
                    self.state_changed.notify_all();
                    return result;
                }
                ProviderCancellationPhase::Submitting => {
                    state.waiting_callers += 1;
                    self.state_changed.notify_all();
                    state = self
                        .state_changed
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.waiting_callers -= 1;
                    self.state_changed.notify_all();
                }
            }
        }
    }

    #[cfg(test)]
    fn wait_for_waiting_callers(&self, expected: usize) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while state.waiting_callers < expected {
            state = self
                .state_changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

#[cfg(feature = "axon-pb")]
fn ffi_provider_cancellation_error(function: &str, error: ProviderCancellationError) -> i32 {
    match error {
        ProviderCancellationError::RuntimeUnavailable(message) => {
            record_invocation_error(ERR_GENERIC, format!("{function}: {message}"))
        }
        ProviderCancellationError::Daemon { code, message } => {
            record_invocation_error(code, format!("{function}: {message}"))
        }
        ProviderCancellationError::CommandRejected(message) => {
            record_invocation_error(ERR_ABILITY_FAILED, format!("{function}: {message}"))
        }
        ProviderCancellationError::InvalidAcknowledgement(message) => {
            record_invocation_error(ERR_PROTOCOL, format!("{function}: {message}"))
        }
    }
}

#[cfg(feature = "axon-pb")]
trait ProviderCancellableResource {
    fn cancellation(&self) -> &ProviderCancellationControl;
}

#[cfg(feature = "axon-pb")]
struct ActiveInvocationStream {
    owner: ClientSessionBinding,
    reader_cancel: tokio_util::sync::CancellationToken,
    cancellation: Arc<ProviderCancellationControl>,
    reader_finished: AtomicBool,
}

#[cfg(feature = "axon-pb")]
impl ActiveInvocationStream {
    fn new(
        owner: ClientSessionBinding,
        reader_cancel: tokio_util::sync::CancellationToken,
        cancellation: Arc<ProviderCancellationControl>,
    ) -> Self {
        Self {
            owner,
            reader_cancel,
            cancellation,
            reader_finished: AtomicBool::new(false),
        }
    }

    fn mark_reader_finished(&self) {
        self.reader_finished.store(true, Ordering::Release);
    }

    fn reader_finished(&self) -> bool {
        self.reader_finished.load(Ordering::Acquire)
    }
}

#[cfg(feature = "axon-pb")]
impl ProviderCancellableResource for ActiveInvocationStream {
    fn cancellation(&self) -> &ProviderCancellationControl {
        self.cancellation.as_ref()
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Clone)]
enum InvocationCancellationGate {
    Available(crate::daemon::InvocationCancellationAuthority),
    Unavailable { reason: String },
}

#[cfg(feature = "axon-pb")]
impl InvocationCancellationGate {
    fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }

    fn into_authority(self) -> Option<crate::daemon::InvocationCancellationAuthority> {
        match self {
            Self::Available(authority) => Some(authority),
            Self::Unavailable { .. } => None,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Available(authority) => serde_json::json!({
                "state": "available",
                "owner_ura": authority.owner_ura(),
            }),
            Self::Unavailable { reason } => serde_json::json!({
                "state": "unavailable",
                "reason": reason,
            }),
        }
    }
}

#[cfg(feature = "axon-pb")]
struct ActiveInvocationHandle {
    owner: ClientSessionBinding,
    cancel_requests: tokio::sync::mpsc::UnboundedSender<String>,
    cancellation_gate: InvocationCancellationGate,
    shared: Arc<InvocationHandleShared>,
}

#[cfg(feature = "axon-pb")]
impl ActiveInvocationHandle {
    fn with_cancel_channel(
        owner: ClientSessionBinding,
        tuple_json: serde_json::Value,
        cancellation_gate: InvocationCancellationGate,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (cancel_requests, receiver) = tokio::sync::mpsc::unbounded_channel();
        let handle = Self {
            owner,
            cancel_requests,
            cancellation_gate,
            shared: Arc::new(InvocationHandleShared::new(tuple_json)),
        };
        (handle, receiver)
    }

    fn submitted_json(
        &self,
        invocation_handle_id: InvocationHandleId,
    ) -> Result<serde_json::Value, InvocationObservationFailure> {
        self.shared
            .snapshot_json(invocation_handle_id, &self.cancellation_gate)
    }

    #[cfg(test)]
    fn await_result(&self) -> crate::daemon::InvocationResult {
        self.shared.await_result()
    }

    fn await_result_json(&self) -> Result<serde_json::Value, InvocationObservationFailure> {
        let (outcome, tuple_json) = self.shared.await_outcome_with_tuple_json()?;
        invocation_outcome_json_with_tuple(outcome, tuple_json).map_err(|message| {
            InvocationObservationFailure {
                abi_code: ERR_PROTOCOL,
                message,
            }
        })
    }

    fn cancel(&self, reason: Option<String>) -> InvocationHandleCancelOutcome {
        let reason = reason.unwrap_or_else(|| "user_request".to_string());
        if let Some(unavailable) = self.cancellation_gate.unavailable_reason() {
            return self
                .shared
                .reject_cancel_unavailable(reason, unavailable.to_string());
        }
        let outcome = self.shared.request_cancel(reason.clone());
        if outcome.dispatch_request && self.cancel_requests.send(reason).is_err() {
            self.shared
                .mark_cancel_request_failed(InvocationObservationFailure {
                    abi_code: ERR_DAEMON_DOWN,
                    message: "cancel request transport is no longer available".to_string(),
                });
        }
        outcome
    }

    fn events_json(
        &self,
        invocation_handle_id: InvocationHandleId,
    ) -> Result<serde_json::Value, InvocationObservationFailure> {
        self.shared
            .snapshot_json(invocation_handle_id, &self.cancellation_gate)
    }
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleShared {
    inner: Mutex<InvocationHandleState>,
    terminal: Condvar,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandleShared {
    fn new(tuple_json: serde_json::Value) -> Self {
        Self {
            inner: Mutex::new(InvocationHandleState::submitted(tuple_json)),
            terminal: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, InvocationHandleState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn await_outcome_with_tuple_json(
        &self,
    ) -> Result<(crate::daemon::InvocationOutcome, serde_json::Value), InvocationObservationFailure>
    {
        let mut state = self.lock();
        while state.terminal_outcome.is_none() && state.observation_failure.is_none() {
            state = self
                .terminal
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        if let Some(failure) = state.observation_failure.clone() {
            return Err(failure);
        }
        Ok((
            state
                .terminal_outcome
                .clone()
                .expect("terminal outcome is present after wait"),
            state.tuple_json.clone(),
        ))
    }

    #[cfg(test)]
    fn await_result(&self) -> crate::daemon::InvocationResult {
        self.await_outcome_with_tuple_json()
            .expect("test handle reaches canonical terminal")
            .0
            .into_result()
    }

    fn request_cancel(&self, reason: String) -> InvocationHandleCancelOutcome {
        let mut state = self.lock();
        if state.phase.is_terminal() {
            return InvocationHandleCancelOutcome {
                request_accepted: false,
                deduplicated: true,
                dispatch_request: false,
                cancelled: state.phase == InvocationHandlePhase::Cancelled,
                state: state.phase,
                terminal: true,
                rejection: None,
            };
        }
        let deduplicated = state.phase == InvocationHandlePhase::CancelRequested;
        let dispatch_request = !state.cancel_request_in_flight;
        state.cancel_request_in_flight = true;
        if !deduplicated {
            state.phase = InvocationHandlePhase::CancelRequested;
            state.push_event("cancel_requested", Some(reason), None);
        }
        InvocationHandleCancelOutcome {
            request_accepted: true,
            deduplicated,
            dispatch_request,
            cancelled: false,
            state: InvocationHandlePhase::CancelRequested,
            terminal: false,
            rejection: None,
        }
    }

    fn reject_cancel_unavailable(
        &self,
        reason: String,
        unavailable: String,
    ) -> InvocationHandleCancelOutcome {
        let mut state = self.lock();
        if !state.phase.is_terminal() && !state.has_event_kind("cancel_unavailable") {
            state.push_event(
                "cancel_unavailable",
                Some(format!("{reason}: {unavailable}")),
                None,
            );
        }
        InvocationHandleCancelOutcome {
            request_accepted: false,
            deduplicated: true,
            dispatch_request: false,
            cancelled: state.phase == InvocationHandlePhase::Cancelled,
            state: state.phase,
            terminal: state.phase.is_terminal(),
            rejection: Some(unavailable),
        }
    }

    fn observe_canonical_outcome(
        &self,
        outcome: crate::daemon::InvocationOutcome,
    ) -> Result<bool, InvocationObservationFailure> {
        let phase =
            canonical_terminal_phase(&outcome).map_err(|message| InvocationObservationFailure {
                abi_code: ERR_PROTOCOL,
                message,
            })?;
        let mut state = self.lock();
        if state.phase.is_terminal() {
            return Ok(false);
        }
        state.push_terminal(phase, phase.event_kind(), None, outcome);
        self.terminal.notify_all();
        Ok(true)
    }

    fn observe_cancel_command_outcome(
        &self,
        outcome: crate::daemon::InvocationOutcome,
    ) -> Result<(), InvocationObservationFailure> {
        let phase =
            canonical_terminal_phase(&outcome).map_err(|message| InvocationObservationFailure {
                abi_code: ERR_PROTOCOL,
                message,
            })?;
        if phase != InvocationHandlePhase::Completed || outcome.result().error.is_some() {
            return Err(InvocationObservationFailure {
                abi_code: ERR_ABILITY_FAILED,
                message: format!(
                    "invocation.cancel command did not complete successfully: state={}",
                    outcome.result().terminal_state
                ),
            });
        }

        let mut state = self.lock();
        if !state.phase.is_terminal() {
            state.cancel_request_in_flight = false;
            state.push_event("cancel_command_completed", None, None);
        }
        Ok(())
    }

    fn mark_cancel_request_failed(&self, failure: InvocationObservationFailure) {
        let mut state = self.lock();
        if state.phase.is_terminal() {
            return;
        }
        state.cancel_request_in_flight = false;
        state.push_event("cancel_request_failed", Some(failure.message), None);
    }

    fn mark_observation_failed(&self, failure: InvocationObservationFailure) {
        let mut state = self.lock();
        if state.phase.is_terminal() || state.observation_failure.is_some() {
            return;
        }
        state.cancel_request_in_flight = false;
        state.push_event("observation_failed", Some(failure.message.clone()), None);
        state.observation_failure = Some(failure);
        self.terminal.notify_all();
    }

    fn snapshot_json(
        &self,
        invocation_handle_id: InvocationHandleId,
        cancellation_gate: &InvocationCancellationGate,
    ) -> Result<serde_json::Value, InvocationObservationFailure> {
        let state = self.lock();
        state.snapshot_json(invocation_handle_id, cancellation_gate)
    }
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleState {
    tuple_json: serde_json::Value,
    phase: InvocationHandlePhase,
    next_sequence: u64,
    events: Vec<InvocationHandleEvent>,
    terminal_outcome: Option<crate::daemon::InvocationOutcome>,
    observation_failure: Option<InvocationObservationFailure>,
    cancel_request_in_flight: bool,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandleState {
    fn submitted(tuple_json: serde_json::Value) -> Self {
        Self {
            tuple_json,
            phase: InvocationHandlePhase::Submitted,
            next_sequence: 2,
            events: vec![InvocationHandleEvent {
                sequence: 1,
                state: InvocationHandlePhase::Submitted,
                kind: "submitted".to_string(),
                terminal: false,
                reason: None,
                outcome: None,
            }],
            terminal_outcome: None,
            observation_failure: None,
            cancel_request_in_flight: false,
        }
    }

    fn push_event(
        &mut self,
        kind: &'static str,
        reason: Option<String>,
        outcome: Option<crate::daemon::InvocationOutcome>,
    ) {
        self.events.push(InvocationHandleEvent {
            sequence: self.next_sequence,
            state: self.phase,
            kind: kind.to_string(),
            terminal: false,
            reason,
            outcome,
        });
        self.next_sequence += 1;
    }

    fn push_terminal(
        &mut self,
        phase: InvocationHandlePhase,
        kind: &'static str,
        reason: Option<String>,
        outcome: crate::daemon::InvocationOutcome,
    ) {
        self.phase = phase;
        self.events.push(InvocationHandleEvent {
            sequence: self.next_sequence,
            state: phase,
            kind: kind.to_string(),
            terminal: true,
            reason,
            outcome: Some(outcome.clone()),
        });
        self.next_sequence += 1;
        self.terminal_outcome = Some(outcome);
    }

    fn has_event_kind(&self, kind: &str) -> bool {
        self.events.iter().any(|event| event.kind == kind)
    }

    fn snapshot_json(
        &self,
        invocation_handle_id: InvocationHandleId,
        cancellation_gate: &InvocationCancellationGate,
    ) -> Result<serde_json::Value, InvocationObservationFailure> {
        let events = self
            .events
            .iter()
            .map(|event| event.to_json(&self.tuple_json))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self
            .terminal_outcome
            .clone()
            .map(|outcome| invocation_outcome_json_with_tuple(outcome, self.tuple_json.clone()))
            .transpose()
            .map_err(|message| InvocationObservationFailure {
                abi_code: ERR_PROTOCOL,
                message,
            })?;
        Ok(serde_json::json!({
            "handle_id": invocation_handle_id,
            "state": self.phase.as_str(),
            "terminal": self.phase.is_terminal(),
            "events": events,
            "result": result,
            "cancellation_authority": cancellation_gate.to_json(),
            "observation_error": self.observation_failure.as_ref().map(|failure| serde_json::json!({
                "abi_code": failure.abi_code,
                "message": failure.message,
            })),
        }))
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
    outcome: Option<crate::daemon::InvocationOutcome>,
}

#[cfg(feature = "axon-pb")]
impl InvocationHandleEvent {
    fn to_json(
        &self,
        tuple_json: &serde_json::Value,
    ) -> Result<serde_json::Value, InvocationObservationFailure> {
        let result = self
            .outcome
            .clone()
            .map(|outcome| invocation_outcome_json_with_tuple(outcome, tuple_json.clone()))
            .transpose()
            .map_err(|message| InvocationObservationFailure {
                abi_code: ERR_PROTOCOL,
                message,
            })?;
        Ok(serde_json::json!({
            "sequence": self.sequence,
            "kind": self.kind,
            "state": self.state.as_str(),
            "terminal": self.terminal,
            "reason": self.reason,
            "result": result,
        }))
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvocationHandlePhase {
    Submitted,
    CancelRequested,
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
            Self::CancelRequested => "CancelRequested",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::TimedOut => "TimedOut",
            Self::Cancelled => "Cancelled",
        }
    }

    fn event_kind(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::CancelRequested => "cancel_requested",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }
}

#[cfg(feature = "axon-pb")]
struct InvocationHandleCancelOutcome {
    request_accepted: bool,
    deduplicated: bool,
    dispatch_request: bool,
    cancelled: bool,
    state: InvocationHandlePhase,
    terminal: bool,
    rejection: Option<String>,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct InvocationObservationFailure {
    abi_code: i32,
    message: String,
}

#[cfg(feature = "axon-pb")]
struct ActiveInvocationBidi {
    owner: ClientSessionBinding,
    ability: String,
    up_tx: tokio::sync::mpsc::Sender<axon_sdk::pb::axon::v1::InvokeBidiUp>,
    reader_cancel: tokio_util::sync::CancellationToken,
    cancellation: Arc<ProviderCancellationControl>,
    local_send: Mutex<BidiLocalSendState>,
}

#[cfg(feature = "axon-pb")]
impl ActiveInvocationBidi {
    fn new(
        owner: ClientSessionBinding,
        ability: String,
        up_tx: tokio::sync::mpsc::Sender<axon_sdk::pb::axon::v1::InvokeBidiUp>,
        reader_cancel: tokio_util::sync::CancellationToken,
        cancellation: Arc<ProviderCancellationControl>,
    ) -> Self {
        Self {
            owner,
            ability,
            up_tx,
            reader_cancel,
            cancellation,
            local_send: Mutex::new(BidiLocalSendState::new()),
        }
    }

    fn reserve_up_frame(
        &self,
        frame: BidiUpFrame,
    ) -> Result<axon_sdk::pb::axon::v1::InvokeBidiUp, BidiLocalSendClosed> {
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
        Ok(axon_sdk::pb::axon::v1::InvokeBidiUp {
            sequence,
            mac: frame.mac,
            payload: Some(frame.payload),
        })
    }
}

#[cfg(feature = "axon-pb")]
impl ProviderCancellableResource for ActiveInvocationBidi {
    fn cancellation(&self) -> &ProviderCancellationControl {
        self.cancellation.as_ref()
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
    entries: Mutex<std::collections::HashMap<InvocationStreamId, Arc<ActiveInvocationStream>>>,
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
) -> MutexGuard<'_, std::collections::HashMap<InvocationStreamId, Arc<ActiveInvocationStream>>> {
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
    lock_stream_entries(registry).insert(stream_id, Arc::new(stream));
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
    let mut entries = lock_invocation_handle_entries(registry);
    let invocation_handle_id = mint_invocation_handle_token(&entries);
    entries.insert(invocation_handle_id, Arc::new(handle));
    invocation_handle_id
}

#[cfg(feature = "axon-pb")]
fn mint_invocation_handle_token(
    entries: &std::collections::HashMap<InvocationHandleId, Arc<ActiveInvocationHandle>>,
) -> InvocationHandleId {
    const JSON_SAFE_TOKEN_MASK: u64 = (1_u64 << 53) - 1;
    const JSON_SAFE_TOKEN_FLOOR: u64 = 1_u64 << 52;
    loop {
        // Preserve the existing uint64 C ABI while making the value an opaque
        // provider control token instead of a predictable registry sequence.
        // Keep it within JavaScript's exact integer range because this token is
        // also projected through the public JSON snapshot as `handle_id`.
        let candidate = (OsRng.next_u64() & JSON_SAFE_TOKEN_MASK) | JSON_SAFE_TOKEN_FLOOR;
        if !entries.contains_key(&candidate) {
            return candidate;
        }
    }
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
    owner: ClientSessionBinding,
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
    owner: ClientSessionBinding,
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

#[cfg(all(test, feature = "axon-pb"))]
fn remove_stream(stream_id: InvocationStreamId) -> Option<Arc<ActiveInvocationStream>> {
    if stream_id == 0 {
        return None;
    }
    lock_stream_entries(stream_registry()).remove(&stream_id)
}

#[cfg(feature = "axon-pb")]
fn get_stream(stream_id: InvocationStreamId) -> Option<Arc<ActiveInvocationStream>> {
    if stream_id == 0 {
        return None;
    }
    lock_stream_entries(stream_registry())
        .get(&stream_id)
        .cloned()
}

#[cfg(feature = "axon-pb")]
fn get_stream_for_handle(
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
) -> Result<Option<Arc<ActiveInvocationStream>>, RegistryOwnerMismatch> {
    if stream_id == 0 {
        return Ok(None);
    }
    let stream = lock_stream_entries(stream_registry())
        .get(&stream_id)
        .cloned();
    let Some(stream) = stream else {
        return Ok(None);
    };
    if stream.owner != owner {
        return Err(RegistryOwnerMismatch);
    }
    Ok(Some(stream))
}

#[cfg(feature = "axon-pb")]
fn remove_stream_for_handle(
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
) -> Result<Option<Arc<ActiveInvocationStream>>, RegistryOwnerMismatch> {
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
pub(crate) fn cancel_invocations_for_binding(owner: ClientSessionBinding) {
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
        let _ = handle.cancel(Some("owning RuntimeHandle shutdown".to_string()));
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
        stream.reader_cancel.cancel();
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
        session.reader_cancel.cancel();
    }
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn cancel_invocations_for_binding(
    _owner: crate::ffi::client::handle::ClientSessionBinding,
) {
}

#[cfg(feature = "axon-pb")]
fn get_bidi_for_handle(
    owner: ClientSessionBinding,
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
    owner: ClientSessionBinding,
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
    let code = ffi_code_for_daemon_error(&err);
    let message = format!("{context}: {err}");
    match err.invocation_error_projection() {
        crate::daemon::DaemonInvocationErrorProjection::CallerSignerUnavailable => {
            record_caller_signer_unavailable_error(message)
        }
        crate::daemon::DaemonInvocationErrorProjection::DescriptorOwnerOffline => {
            record_descriptor_owner_offline_error(message)
        }
        _ => record_invocation_error(code, message),
    }
}

#[cfg(feature = "axon-pb")]
fn ffi_code_for_daemon_error(err: &crate::daemon::DaemonError) -> i32 {
    ffi_code_for_daemon_error_projection(err.invocation_error_projection())
}

#[cfg(feature = "axon-pb")]
fn ffi_code_for_daemon_error_projection(
    projection: crate::daemon::DaemonInvocationErrorProjection,
) -> i32 {
    match projection {
        crate::daemon::DaemonInvocationErrorProjection::DaemonDown
        | crate::daemon::DaemonInvocationErrorProjection::DescriptorOwnerOffline => ERR_DAEMON_DOWN,
        crate::daemon::DaemonInvocationErrorProjection::CallerSignerUnavailable => {
            ERR_PERMISSION_DENIED
        }
        crate::daemon::DaemonInvocationErrorProjection::Status(code) => {
            ffi_status_code_to_error(code)
        }
        crate::daemon::DaemonInvocationErrorProjection::InvalidInvocation => ERR_INVALID_ARG,
        crate::daemon::DaemonInvocationErrorProjection::Cancelled => ERR_CANCELLED,
        crate::daemon::DaemonInvocationErrorProjection::Generic => ERR_GENERIC,
    }
}

#[cfg(feature = "axon-pb")]
fn ffi_status_code_to_error(code: tonic::Code) -> i32 {
    match code {
        tonic::Code::Ok => RUNTIME_OK,
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
    mut stream: tonic::Streaming<axon_sdk::pb::axon::v1::InvokeStreamChunk>,
    cancel: tokio_util::sync::CancellationToken,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let mut next_error_sequence = 1;
    let mut receipt_verifier = InboundReceiptCheckpointVerifier::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            message = stream.message() => match message {
                Ok(Some(chunk)) => {
                    let sequence = chunk.sequence;
                    next_error_sequence = sequence.saturating_add(1).max(1);
                    let projection = match stream_chunk_json(&mut receipt_verifier, chunk) {
                        Ok(projection) => projection,
                        Err(message) => {
                            let _ = tx.send(serde_json::json!({
                                "ok": false,
                                "kind": "receipt_verification_error",
                                "sequence": sequence,
                                "message": message,
                                "terminal": false,
                            }).to_string().into_bytes()).await;
                            break;
                        }
                    };
                    let terminal = projection.is_canonical_terminal();
                    let bytes = projection.into_json_bytes();
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
    if let Some(stream) = get_stream(stream_id) {
        stream.mark_reader_finished();
    }
}

#[cfg(feature = "axon-pb")]
async fn run_bidi_down_reader(
    bidi_id: InvocationBidiId,
    mut down: tonic::Streaming<axon_sdk::pb::axon::v1::InvokeBidiDown>,
    cancel: tokio_util::sync::CancellationToken,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let mut next_error_sequence = 1;
    let mut receipt_verifier = InboundReceiptCheckpointVerifier::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            message = down.message() => match message {
                Ok(Some(frame)) => {
                    let sequence = frame.sequence;
                    next_error_sequence = sequence.saturating_add(1).max(1);
                    let projection = match bidi_down_frame_json(&mut receipt_verifier, frame) {
                        Ok(projection) => projection,
                        Err(message) => {
                            let _ = tx.send(serde_json::json!({
                                "ok": false,
                                "kind": "receipt_verification_error",
                                "sequence": sequence,
                                "message": message,
                                "terminal": false,
                            }).to_string().into_bytes()).await;
                            break;
                        }
                    };
                    let terminal = projection.is_canonical_terminal();
                    let bytes = projection.into_json_bytes();
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
    causal_context: axon_sdk::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
    metadata: std::collections::HashMap<String, String>,
    caller_signature: Option<axon_sdk::pb::axon::v1::CallerSignature>,
    bidi_streams: Vec<axon_sdk::pb::axon::v1::StreamDescriptor>,
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
        validate_public_invocation_tuple(
            &caller_ura,
            &callee_ura,
            &descriptor_ref,
            &subject_ura,
            &metadata,
        )?;
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
        let derivation_policy =
            axon_sdk::invocation::InvocationDerivationPolicy::try_explicit_from_wire_causal_context(
                self.nonce,
                self.causal_context,
            )
            .map_err(|error| {
                crate::daemon::DaemonError::InvalidInvocation(error.to_string())
            })?;
        let mut builder = crate::daemon::DaemonInvocation::builder(
            self.caller_ura,
            self.callee_ura,
            self.descriptor_ref,
            self.subject_ura,
            derivation_policy,
        )?
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
    #[error("field `{field}` must be a canonical URA: {reason}")]
    InvalidUra { field: &'static str, reason: String },
    #[error("descriptor_ref is not a public invocation descriptor: {0}")]
    InvalidDescriptorRef(String),
    #[error(
        "receipt history ability `{0}` is not a public invocation action; use the canonical invocation history read path"
    )]
    ReceiptHistoryReadDescriptor(String),
    #[error("field `{0}` must not contain the all-zero principal placeholder")]
    AllZeroPrincipal(&'static str),
    #[error("field `{0}` uses a noncanonical session subject")]
    NoncanonicalSessionSubject(&'static str),
    #[error("authority metadata is invalid: {0}")]
    AuthorityMetadata(String),
    #[error("authority metadata subject does not admit invocation subject_ura: {0}")]
    AuthoritySubjectMismatch(String),
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
    causal_context: Option<axon_sdk::pb::axon::v1::CausalContext>,
    args: Option<InvocationBuilderArgs>,
    metadata: std::collections::HashMap<String, String>,
    caller_signature: Option<axon_sdk::pb::axon::v1::CallerSignature>,
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
        validate_public_invocation_tuple(
            &caller_ura,
            &callee_ura,
            &descriptor_ref,
            &subject_ura,
            &self.metadata,
        )
        .map_err(|error| crate::daemon::DaemonError::InvalidInvocation(error.to_string()))?;

        let derivation_policy =
            axon_sdk::invocation::InvocationDerivationPolicy::try_explicit_from_wire_causal_context(
                nonce,
                causal_context,
            )
            .map_err(|error| {
                crate::daemon::DaemonError::InvalidInvocation(error.to_string())
            })?;
        let builder = crate::daemon::DaemonInvocation::builder(
            caller_ura,
            callee_ura,
            descriptor_ref,
            subject_ura,
            derivation_policy,
        )?
        .metadata(self.metadata.clone());
        let mut builder = match args {
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
#[derive(Debug, Clone, Default)]
struct PrepareOptionsJson {
    expires_in_ms: Option<u64>,
    signer_id: Option<String>,
    policy_ref: Option<String>,
    provider_managed_signing: bool,
    material_only: bool,
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
        let provider_managed_signing = match obj.get("provider_managed_signing") {
            None | Some(serde_json::Value::Null) => false,
            Some(value) => value
                .as_bool()
                .ok_or(InvocationJsonError::InvalidBool("provider_managed_signing"))?,
        };
        let material_only = match obj.get("material_only") {
            None | Some(serde_json::Value::Null) => false,
            Some(value) => value
                .as_bool()
                .ok_or(InvocationJsonError::InvalidBool("material_only"))?,
        };
        Ok(Self {
            expires_in_ms,
            signer_id: optional_string(obj, "signer_id")?,
            policy_ref: optional_string(obj, "policy_ref")?,
            provider_managed_signing,
            material_only,
        })
    }

    fn into_prepare_options(self) -> crate::daemon::PrepareOptions {
        crate::daemon::PrepareOptions {
            expires_in: std::time::Duration::from_millis(self.expires_in_ms.unwrap_or(300_000)),
            signer_id: self.signer_id,
            policy_ref: self.policy_ref,
            provider_managed_signing: self.provider_managed_signing,
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

    fn into_wire_signature(self) -> axon_sdk::pb::axon::v1::CallerSignature {
        axon_sdk::pb::axon::v1::CallerSignature {
            algorithm: self.algorithm,
            signature: self.signature,
            key_id_hint: self.key_id_hint,
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct BidiUpFrame {
    mac: Vec<u8>,
    payload: axon_sdk::pb::axon::v1::invoke_bidi_up::Payload,
}

#[cfg(feature = "axon-pb")]
fn bidi_up_payload_is_eof(payload: &axon_sdk::pb::axon::v1::invoke_bidi_up::Payload) -> bool {
    use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload;
    matches!(payload, Payload::Control(control) if bidi_control_is_eof(control))
}

#[cfg(feature = "axon-pb")]
const BIDI_FRAME_CHAIN_MAC_BYTES: usize = 32;

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiFrameChainMac(Vec<u8>);

#[cfg(feature = "axon-pb")]
impl BidiFrameChainMac {
    fn parse_base64(raw: &str) -> Result<Self, BidiFrameJsonError> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw.as_bytes())
            .map_err(|err| BidiFrameJsonError::InvalidBase64("mac_base64", err))?;
        if bytes.len() != BIDI_FRAME_CHAIN_MAC_BYTES {
            return Err(BidiFrameJsonError::InvalidMacLength(bytes.len()));
        }
        Ok(Self(bytes))
    }

    fn into_bytes(self) -> Vec<u8> {
        self.0
    }
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
    #[error("mac_base64 must decode to exactly 32 bytes, got {0}")]
    InvalidMacLength(usize),
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
    let mac = BidiFrameChainMac::parse_base64(&frame_required_string(obj, "mac_base64")?)?;
    let payload = match kind.as_str() {
        "binary_chunk" => {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD
                .decode(frame_required_string(obj, "data_base64")?.as_bytes())
                .map_err(|err| BidiFrameJsonError::InvalidBase64("data_base64", err))?;
            axon_sdk::pb::axon::v1::invoke_bidi_up::Payload::BinaryChunk(
                axon_sdk::pb::axon::v1::BinaryChunk {
                    stream_id: frame_u32(obj, "stream_id")?,
                    data,
                    pts: frame_u64(obj, "pts")?,
                },
            )
        }
        "control" => {
            axon_sdk::pb::axon::v1::invoke_bidi_up::Payload::Control(parse_bidi_control(obj)?)
        }
        other => return Err(BidiFrameJsonError::UnsupportedType(other.to_string())),
    };
    Ok(BidiUpFrame {
        mac: mac.into_bytes(),
        payload,
    })
}

#[cfg(feature = "axon-pb")]
fn parse_bidi_control(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<axon_sdk::pb::axon::v1::BidiControl, BidiFrameJsonError> {
    use axon_sdk::pb::axon::v1::{bidi_control, BidiControl, MediaTimestamp, PtyResize, PtySignal};
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
fn validate_public_invocation_tuple(
    caller_ura: &str,
    callee_ura: &str,
    descriptor_ref: &str,
    subject_ura: &str,
    metadata: &std::collections::HashMap<String, String>,
) -> Result<(), InvocationJsonError> {
    validate_public_tuple_ura("caller_ura", caller_ura)?;
    validate_public_tuple_ura("callee_ura", callee_ura)?;
    validate_public_tuple_ura("subject_ura", subject_ura)?;
    validate_public_invocation_descriptor_ref(descriptor_ref)?;
    validate_public_authority_binding(caller_ura, callee_ura, subject_ura, metadata)
}

#[cfg(feature = "axon-pb")]
fn validate_public_invocation_descriptor_ref(
    descriptor_ref: &str,
) -> Result<(), InvocationJsonError> {
    let public_ability =
        crate::daemon::ability::public_route_ability_from_descriptor_ref(descriptor_ref)
            .map_err(|error| InvocationJsonError::InvalidDescriptorRef(error.to_string()))?;
    if crate::daemon::ability::names::governance::is_invocation_history_read(&public_ability) {
        return Err(InvocationJsonError::ReceiptHistoryReadDescriptor(
            public_ability,
        ));
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
fn validate_public_tuple_ura(field: &'static str, value: &str) -> Result<(), InvocationJsonError> {
    if crate::core::identity::contains_all_zero_principal_placeholder(value) {
        return Err(InvocationJsonError::AllZeroPrincipal(field));
    }
    if field == "subject_ura"
        && crate::core::identity::session_resource_subject_has_noncanonical_session_id(value)
    {
        return Err(InvocationJsonError::NoncanonicalSessionSubject(field));
    }
    crate::core::ura::parse_ura(value.trim())
        .map(|_| ())
        .map_err(|error| InvocationJsonError::InvalidUra {
            field,
            reason: error.to_string(),
        })
}

#[cfg(feature = "axon-pb")]
fn validate_public_authority_binding(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    metadata: &std::collections::HashMap<String, String>,
) -> Result<(), InvocationJsonError> {
    let authority =
        crate::daemon::invocation::admission::authority_metadata::project_invocation_authority_metadata_shape(metadata)
            .map_err(|error| InvocationJsonError::AuthorityMetadata(error.to_string()))?;
    match authority {
        Some(
            crate::daemon::invocation::admission::authority_metadata::InvocationAuthorityMetadata::Delegation(
                payload,
            ),
        ) => {
            if payload.caller_ura.trim() != caller_ura.trim() {
                return Err(InvocationJsonError::AuthorityMetadata(
                    "delegation authority caller_ura does not match invocation caller_ura"
                        .to_string(),
                ));
            }
            if payload.subject_ura.trim() != subject_ura.trim() {
                return Err(InvocationJsonError::AuthoritySubjectMismatch(format!(
                    "delegation subject `{}` != invocation subject `{subject_ura}`",
                    payload.subject_ura
                )));
            }
            if !crate::daemon::invocation::admission::authority_metadata::authority_audience_admits(&payload.audience, callee_ura) {
                return Err(InvocationJsonError::AuthorityMetadata(
                    "delegation authority audience does not admit invocation callee_ura"
                        .to_string(),
                ));
            }
        }
        Some(
            crate::daemon::invocation::admission::authority_metadata::InvocationAuthorityMetadata::Session(
                payload,
            ),
        ) => {
            if payload.issuer_ura.trim() != caller_ura.trim() {
                return Err(InvocationJsonError::AuthorityMetadata(
                    "session authority issuer_ura does not match invocation caller_ura"
                        .to_string(),
                ));
            }
            if payload.callee_ura.trim() != callee_ura.trim() {
                return Err(InvocationJsonError::AuthorityMetadata(
                    "session authority callee_ura does not match invocation callee_ura"
                        .to_string(),
                ));
            }
            if !crate::daemon::invocation::admission::authority_metadata::session_authority_admits_subject(&payload, subject_ura) {
                return Err(InvocationJsonError::AuthoritySubjectMismatch(format!(
                    "session subject `{}` owned by `{}` does not admit invocation subject `{subject_ura}`",
                    payload.subject_ura, payload.session_owner_user_id
                )));
            }
            if !crate::daemon::invocation::admission::authority_metadata::authority_audience_admits(&payload.audience, callee_ura) {
                return Err(InvocationJsonError::AuthorityMetadata(
                    "session authority audience does not admit invocation callee_ura".to_string(),
                ));
            }
        }
        None => {}
    }
    Ok(())
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
) -> Result<Option<axon_sdk::pb::axon::v1::CallerSignature>, InvocationJsonError> {
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
    Ok(Some(axon_sdk::pb::axon::v1::CallerSignature {
        algorithm,
        signature,
        key_id_hint,
    }))
}

#[cfg(feature = "axon-pb")]
fn caller_signature_key_id_hint(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, InvocationJsonError> {
    required_string(obj, "key_id_hint")
}

#[cfg(feature = "axon-pb")]
fn parse_bidi_streams(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<axon_sdk::pb::axon::v1::StreamDescriptor>, InvocationJsonError> {
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
        out.push(axon_sdk::pb::axon::v1::StreamDescriptor {
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
) -> Result<axon_sdk::pb::axon::v1::CausalContext, InvocationJsonError> {
    use axon_sdk::pb::axon::v1 as pb;
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
) -> Result<axon_sdk::pb::axon::v1::CausalContext, InvocationJsonError> {
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
fn causal_context_json(context: &axon_sdk::pb::axon::v1::CausalContext) -> serde_json::Value {
    use axon_sdk::pb::axon::v1::causal_context::Form;
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
fn nonzero_id(id: u64) -> Option<u64> {
    (id != 0).then_some(id)
}

#[cfg(feature = "axon-pb")]
fn prepared_invocation_json(
    prepared: &crate::daemon::PreparedInvocation,
    prepared_id: Option<PreparedInvocationId>,
) -> serde_json::Value {
    let material = prepared.signing_material();
    let policy = material.signer_policy();
    let mut value = serde_json::json!({
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
    });
    if let Some(prepared_id) = prepared_id {
        value["prepared_id"] = serde_json::json!(prepared_id.to_string());
    }
    value
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
fn invocation_outcome_json_with_tuple(
    outcome: crate::daemon::InvocationOutcome,
    tuple_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use base64::Engine;
    let (result, stages) = outcome.into_parts();
    debug_assert_eq!(result.receipt, stages.terminal);
    let output_json =
        runtime_json_projection(&result.output, &result.output_content_type, "output_json")?;
    let terminal_receipt = stages.terminal;
    Ok(serde_json::json!({
        "ok": result.error.is_none(),
        "tuple": tuple_json,
        "terminal_state": result.terminal_state,
        "output_content_type": result.output_content_type,
        "output_base64": base64::engine::general_purpose::STANDARD.encode(&result.output),
        "output_json": output_json,
        "elapsed_ms": result.elapsed_ms,
        "admission_receipt": stages.admission.map(receipt_summary_dto_json),
        "terminal_receipt": terminal_receipt.map(receipt_summary_dto_json),
        "error": result.error.map(runtime_error_json),
    }))
}

#[cfg(feature = "axon-pb")]
fn result_content_type_is_json(content_type: &str) -> bool {
    content_type.to_ascii_lowercase().contains("json")
}

#[cfg(feature = "axon-pb")]
fn runtime_json_projection(
    payload: &[u8],
    content_type: &str,
    projection_field: &'static str,
) -> Result<Option<serde_json::Value>, String> {
    if !result_content_type_is_json(content_type) {
        return Ok(None);
    }
    if payload.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice::<serde_json::Value>(payload)
        .map(Some)
        .map_err(|error| {
            format!(
                "{projection_field} declares JSON content type {content_type:?} but payload is not valid JSON: {error}"
            )
        })
}

#[cfg(feature = "axon-pb")]
fn invocation_observation_failure(
    err: &crate::daemon::DaemonError,
) -> InvocationObservationFailure {
    InvocationObservationFailure {
        abi_code: ffi_code_for_daemon_error(err),
        message: err.to_string(),
    }
}

#[cfg(feature = "axon-pb")]
/// Project the terminal state already validated by `InvocationOutcome`.
///
/// Receipt-chain verification and receipt-free pre-admission classification
/// belong to `InvocationOutcome::from_invoke_response`; the C ABI handle owns
/// only observation order and terminal monotonicity.
fn canonical_terminal_phase(
    outcome: &crate::daemon::InvocationOutcome,
) -> Result<InvocationHandlePhase, String> {
    explicit_terminal_phase(&outcome.result().terminal_state).ok_or_else(|| {
        format!(
            "runtime returned non-terminal state `{}` for handle await",
            outcome.result().terminal_state
        )
    })
}

#[cfg(feature = "axon-pb")]
fn explicit_terminal_phase(state: &str) -> Option<InvocationHandlePhase> {
    match state {
        "Completed" => Some(InvocationHandlePhase::Completed),
        "Failed" => Some(InvocationHandlePhase::Failed),
        "TimedOut" => Some(InvocationHandlePhase::TimedOut),
        "Cancelled" => Some(InvocationHandlePhase::Cancelled),
        _ => None,
    }
}

#[cfg(feature = "axon-pb")]
fn receipt_summary_dto_json(receipt: crate::daemon::ReceiptSummary) -> serde_json::Value {
    serde_json::json!({
        "verification": receipt.verification,
        "receipt_ura": receipt.receipt_ura,
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
        "payload_base64": receipt.payload_base64,
        "caller_binding": receipt.caller_binding,
        "callee_binding": receipt.callee_binding,
        "subject_binding": receipt.subject_binding,
        "invocation_nonce_base64": receipt.invocation_nonce_base64,
        "causal_binding_kind": receipt.causal_binding_kind,
        "causal_binding": receipt.causal_binding,
        "callee_signature": receipt.callee_signature,
        "signer_binding": receipt.signer_binding,
        "host_attestation_base64": receipt.host_attestation_base64,
        "authority_binding_kind": receipt.authority_binding_kind,
        "authority_binding": receipt.authority_binding,
        "ability_binding": receipt.ability_binding,
        "failure": receipt.failure,
        "usage": receipt.usage,
        "subject_ref": receipt.subject_ref,
        "descriptor_version": receipt.descriptor_version,
        "schema_hash_hex": receipt.schema_hash_hex,
        "impl_hash_hex": receipt.impl_hash_hex,
        "runtime_env": receipt.runtime_env,
        "authority_proof": receipt.authority_proof,
        "input_hash_hex": receipt.input_hash_hex,
        "output_hash_hex": receipt.output_hash_hex,
        "parent_receipts": receipt.parent_receipts,
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
#[derive(Clone, Debug)]
struct CallbackFrameProjection {
    frame_json: serde_json::Value,
    lifecycle: CallbackFrameLifecycle,
}

#[cfg(feature = "axon-pb")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallbackFrameLifecycle {
    Continue,
    CanonicalTerminal,
}

#[cfg(feature = "axon-pb")]
impl CallbackFrameProjection {
    fn new(frame_json: serde_json::Value, lifecycle: CallbackFrameLifecycle) -> Self {
        Self {
            frame_json,
            lifecycle,
        }
    }

    fn is_canonical_terminal(&self) -> bool {
        self.lifecycle == CallbackFrameLifecycle::CanonicalTerminal
    }

    #[cfg(test)]
    fn json(&self) -> &serde_json::Value {
        &self.frame_json
    }

    fn into_json_bytes(self) -> Vec<u8> {
        self.frame_json.to_string().into_bytes()
    }
}

#[cfg(feature = "axon-pb")]
fn stream_chunk_json(
    verifier: &mut InboundReceiptCheckpointVerifier,
    chunk: axon_sdk::pb::axon::v1::InvokeStreamChunk,
) -> Result<CallbackFrameProjection, String> {
    use base64::Engine;
    let payload_base64 = base64::engine::general_purpose::STANDARD.encode(&chunk.payload);
    let payload_json =
        runtime_json_projection(&chunk.payload, &chunk.content_type, "payload_json")?;
    let admission_receipt = chunk
        .admission_receipt
        .map(|receipt| verifier.verify_admission(receipt))
        .transpose()?;
    let terminal_receipt = chunk
        .terminal_receipt
        .map(|receipt| verifier.verify_terminal(receipt))
        .transpose()?;
    let proven_terminal = terminal_receipt.is_some();
    let lifecycle = if proven_terminal {
        CallbackFrameLifecycle::CanonicalTerminal
    } else {
        CallbackFrameLifecycle::Continue
    };
    Ok(CallbackFrameProjection::new(
        serde_json::json!({
            "ok": chunk.error.is_none(),
            "kind": if proven_terminal { "terminal" } else { "data" },
            "invocation_id": chunk.invocation_id,
            "state": chunk.state,
            "sequence": chunk.sequence,
            "terminal": proven_terminal,
            "elapsed_ms": chunk.elapsed_ms,
            "payload_content_type": chunk.content_type,
            "payload_base64": payload_base64,
            "payload_json": payload_json,
            "admission_receipt": admission_receipt,
            "terminal_receipt": terminal_receipt,
            "proof_error": chunk.proof_error.as_ref().map(protocol_error_json),
            "error": chunk.error.as_ref().map(protocol_error_json),
        }),
        lifecycle,
    ))
}

#[cfg(feature = "axon-pb")]
fn stream_status_error_json(status: tonic::Status, sequence: u64) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "kind": "error",
        "sequence": sequence.max(1),
        "code": format!("{:?}", status.code()),
        "message": status.message(),
        "terminal": false,
        "transport_terminal": true,
    })
}

#[cfg(feature = "axon-pb")]
fn bidi_down_frame_json(
    verifier: &mut InboundReceiptCheckpointVerifier,
    frame: axon_sdk::pb::axon::v1::InvokeBidiDown,
) -> Result<CallbackFrameProjection, String> {
    use axon_sdk::pb::axon::v1::invoke_bidi_down::Payload;
    use base64::Engine;
    let mac_base64 = base64::engine::general_purpose::STANDARD.encode(&frame.mac);
    match frame.payload {
        Some(Payload::Receipt(receipt)) => {
            let state = axon_sdk::invocation::InvocationState::try_from(receipt.state)
                .map_err(|error| format!("bidi receipt state is invalid: {error}"))?;
            let (summary, is_admission, is_terminal) =
                if state == axon_sdk::invocation::InvocationState::Admitted {
                    (verifier.verify_admission(receipt)?, true, false)
                } else if state.is_terminal() {
                    (verifier.verify_terminal(receipt)?, false, true)
                } else {
                    return Err("bidi receipt is neither admission nor terminal checkpoint".into());
                };
            let lifecycle = if is_terminal {
                CallbackFrameLifecycle::CanonicalTerminal
            } else {
                CallbackFrameLifecycle::Continue
            };
            Ok(CallbackFrameProjection::new(
                serde_json::json!({
                    "ok": true,
                    "kind": "receipt",
                    "sequence": frame.sequence,
                    "mac_base64": mac_base64,
                    "admission_receipt": is_admission.then(|| summary.clone()),
                    "terminal_receipt": is_terminal.then(|| summary.clone()),
                    "terminal": is_terminal,
                }),
                lifecycle,
            ))
        }
        Some(Payload::BinaryChunk(chunk)) => {
            let payload_base64 = base64::engine::general_purpose::STANDARD.encode(&chunk.data);
            Ok(CallbackFrameProjection::new(
                serde_json::json!({
                    "ok": true,
                    "kind": "data",
                    "sequence": frame.sequence,
                    "mac_base64": mac_base64,
                    "stream_id": chunk.stream_id,
                    "payload_base64": payload_base64,
                    "pts": chunk.pts,
                    "terminal": false,
                }),
                CallbackFrameLifecycle::Continue,
            ))
        }
        Some(Payload::Control(control)) => {
            Ok(CallbackFrameProjection::new(
                serde_json::json!({
                    "ok": true,
                    "kind": "control",
                    "sequence": frame.sequence,
                    "mac_base64": mac_base64,
                    "control": bidi_control_json(control),
                    // A down-direction EOF is a remote half-close signal, not the
                    // canonical invocation terminal state. The terminal state is
                    // carried by the cleanup-complete receipt so SDK consumers can
                    // keep draining until the authoritative outcome arrives.
                    "terminal": false,
                }),
                CallbackFrameLifecycle::Continue,
            ))
        }
        // Carrier-v1 frames (DEC-F004): the FFI JSON projection learns
        // these shapes when dual-read lands (T2.1 steps 2-3); until
        // then they surface as an explicit unsupported event.
        Some(Payload::DispatchCall(_)) | Some(Payload::ReverseDispatchResult(_)) => {
            Ok(CallbackFrameProjection::new(
                serde_json::json!({
                    "ok": false,
                    "kind": "unsupported_frame",
                    "sequence": frame.sequence,
                    "mac_base64": mac_base64,
                    "message": "carrier-v1 dispatch frame before dual-read support",
                    "terminal": false,
                }),
                CallbackFrameLifecycle::Continue,
            ))
        }
        None => Ok(CallbackFrameProjection::new(
            serde_json::json!({
                "ok": false,
                "kind": "unknown",
                "sequence": frame.sequence,
                "mac_base64": mac_base64,
                "message": "InvokeBidiDown frame has no payload",
                "terminal": false,
            }),
            CallbackFrameLifecycle::Continue,
        )),
    }
}

#[cfg(feature = "axon-pb")]
fn bidi_control_is_eof(control: &axon_sdk::pb::axon::v1::BidiControl) -> bool {
    matches!(
        control.control,
        Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true))
    )
}

#[cfg(feature = "axon-pb")]
fn bidi_control_json(control: axon_sdk::pb::axon::v1::BidiControl) -> serde_json::Value {
    use axon_sdk::pb::axon::v1::bidi_control::Control;
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
        Some(Control::SessionEstablished(session)) => serde_json::json!({
            "type": "session_established",
            "contract_version": session.contract_version,
            "dispatch_encoding": session.dispatch_encoding,
            "session_id": session.session_id.to_string(),
            "displaced_prior": session.displaced_prior,
        }),
        None => serde_json::json!({
            "type": "empty",
        }),
    }
}

#[cfg(feature = "axon-pb")]
struct InboundReceiptCheckpointVerifier {
    resolver: crate::support::platform::local_daemon_grpc::CanonicalRuntimeReceiptResolver,
    admission: Option<axon_sdk::invocation::SignedInvocationReceipt>,
}

#[cfg(feature = "axon-pb")]
impl InboundReceiptCheckpointVerifier {
    fn new() -> Self {
        Self {
            resolver:
                crate::support::platform::local_daemon_grpc::CanonicalRuntimeReceiptResolver::new(),
            admission: None,
        }
    }

    fn verify_admission(
        &mut self,
        receipt: axon_sdk::pb::axon::v1::InvocationReceipt,
    ) -> Result<serde_json::Value, String> {
        let signed =
            crate::daemon::invocation::receipts::finalization_projection::verify_admission_checkpoint(
                receipt,
                &self.resolver,
            )
            .map_err(|error| error.to_string())?;
        let summary = crate::daemon::ReceiptSummary::from_signed(&signed)
            .map(receipt_summary_dto_json)
            .map_err(|error| error.to_string())?;
        self.admission = Some(signed);
        Ok(summary)
    }

    fn verify_terminal(
        &mut self,
        receipt: axon_sdk::pb::axon::v1::InvocationReceipt,
    ) -> Result<serde_json::Value, String> {
        let terminal =
            crate::daemon::invocation::receipts::finalization_projection::verify_terminal_checkpoint(
                receipt,
                &self.resolver,
            )
            .map_err(|error| error.to_string())?;
        let admission = self.admission.as_ref().ok_or_else(|| {
            crate::daemon::invocation::receipts::finalization_projection::FinalizationProjectionError::TerminalBeforeAdmission
                .to_string()
        })?;
        let verified =
            crate::daemon::invocation::receipts::finalization_projection::verify_signed_finalization_checkpoints(
                admission,
                &terminal,
                &self.resolver,
            )
            .map_err(|error| error.to_string())?;
        crate::daemon::ReceiptSummary::from_signed(verified.terminal())
            .map(receipt_summary_dto_json)
            .map_err(|error| error.to_string())
    }
}

#[cfg(feature = "axon-pb")]
fn protocol_error_json(error: &axon_sdk::pb::axon::v1::Error) -> serde_json::Value {
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        Arc, Barrier,
    };

    unsafe extern "C" fn ignore_stream_chunk(_: *mut c_void, _: *const c_char) {}
    unsafe extern "C" fn ignore_bidi_frame(_: *mut c_void, _: *const c_char) {}

    #[test]
    fn explicit_terminal_phase_accepts_only_canonical_public_states() {
        for (state, expected) in [
            ("Completed", InvocationHandlePhase::Completed),
            ("Failed", InvocationHandlePhase::Failed),
            ("TimedOut", InvocationHandlePhase::TimedOut),
            ("Cancelled", InvocationHandlePhase::Cancelled),
        ] {
            assert_eq!(explicit_terminal_phase(state), Some(expected), "{state}");
        }
    }

    #[test]
    fn explicit_terminal_phase_rejects_retired_case_variants() {
        for state in [
            "completed",
            "COMPLETED",
            "failed",
            "FAILED",
            "timed_out",
            "TIMED_OUT",
            "timedout",
            "cancelled",
            "CANCELLED",
        ] {
            assert_eq!(
                explicit_terminal_phase(state),
                None,
                "non-canonical terminal state must fail closed: {state}"
            );
        }
    }

    struct AcceptingCancellationCommandSubmitter;

    impl CanonicalCancellationCommandSubmitter for AcceptingCancellationCommandSubmitter {
        fn submit(&self, _reason: &str) -> Result<(), ProviderCancellationError> {
            Ok(())
        }
    }

    struct BlockingCancellationCommandSubmitter {
        calls: AtomicUsize,
        entered: Barrier,
        release: Barrier,
    }

    impl BlockingCancellationCommandSubmitter {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                entered: Barrier::new(2),
                release: Barrier::new(2),
            }
        }
    }

    impl CanonicalCancellationCommandSubmitter for BlockingCancellationCommandSubmitter {
        fn submit(&self, _reason: &str) -> Result<(), ProviderCancellationError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.entered.wait();
            self.release.wait();
            Ok(())
        }
    }

    struct CountingCancellationCommandSubmitter {
        calls: AtomicUsize,
    }

    impl CountingCancellationCommandSubmitter {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl CanonicalCancellationCommandSubmitter for CountingCancellationCommandSubmitter {
        fn submit(&self, _reason: &str) -> Result<(), ProviderCancellationError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(())
        }
    }

    struct RejectingCancellationCommandSubmitter {
        calls: AtomicUsize,
    }

    impl RejectingCancellationCommandSubmitter {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl CanonicalCancellationCommandSubmitter for RejectingCancellationCommandSubmitter {
        fn submit(&self, _reason: &str) -> Result<(), ProviderCancellationError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Err(ProviderCancellationError::CommandRejected(
                "canonical cancellation rejected".to_string(),
            ))
        }
    }

    fn test_cancellation_control() -> Arc<ProviderCancellationControl> {
        Arc::new(ProviderCancellationControl::with_submitter(Arc::new(
            AcceptingCancellationCommandSubmitter,
        )))
    }

    fn registry_owner(handle: RuntimeHandle, incarnation: u64) -> ClientSessionBinding {
        ClientSessionBinding {
            handle,
            incarnation,
        }
    }

    fn descriptor_ref(owner_ura: &str, public_name: &str, version: &str) -> String {
        format!(
            "{}@{}#{}!invoke",
            crate::core::ura::owner_ability_ura(owner_ura, public_name).unwrap(),
            version,
            "aa".repeat(32)
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
        with_test_key_service_for(
            "easynet:///r/acme/device/dev-a",
            "agent_signing",
            expected_connections,
            f,
        )
    }

    #[cfg(unix)]
    fn with_test_key_service_for<F>(
        caller: &'static str,
        purpose: &'static str,
        expected_connections: usize,
        f: F,
    ) where
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

        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let socket_path = socket.clone();
        let vault_path = temp.path().join("key-service.enc");
        let server = std::thread::spawn(move || {
            crate::daemon::keyring::service::run_test_unix_key_service_with_purpose(
                socket_path,
                vault_path,
                "test-passphrase".to_string(),
                caller.to_string(),
                purpose.to_string(),
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
                "metadata": {"x-easynet-test-producer": "producer"},
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
                "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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

    #[test]
    fn parse_invocation_json_rejects_all_zero_subject_before_daemon_io() {
        let err = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "subject_ura": "easynet:///r/acme/resource/user.00000000-0000-0000-0000-000000000000/session/invocation_history"
        })))
        .expect_err("all-zero placeholder subjects must fail at public FFI ingress");

        assert!(
            matches!(err, InvocationJsonError::AllZeroPrincipal("subject_ura")),
            "unexpected all-zero rejection: {err}"
        );
    }

    #[test]
    fn parse_invocation_json_rejects_receipt_history_descriptor_before_daemon_io() {
        let callee_ura = "easynet:///r/acme/device/dev-a";
        let history_descriptor_ref = format!(
            "{}@1.0.0#{}!read",
            crate::core::ura::owner_ability_ura(
                callee_ura,
                crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST
            )
            .expect("history ability URA"),
            "aa".repeat(32)
        );

        let err = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "descriptor_ref": history_descriptor_ref,
            "subject_ura": "easynet:///r/acme/resource/user.alice/runtime-state/read"
        })))
        .expect_err("receipt history must not enter generic public invocation ingress");

        assert!(
            matches!(&err, InvocationJsonError::ReceiptHistoryReadDescriptor(name) if name == crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST),
            "unexpected history descriptor rejection: {err}"
        );
        assert!(
            err.to_string()
                .contains("canonical invocation history read path"),
            "error should direct callers to the canonical read path: {err}"
        );
    }

    #[test]
    fn parse_invocation_json_rejects_noncanonical_session_subject_before_daemon_io() {
        let err = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "subject_ura": "easynet:///r/acme/resource/user.alice/session/invocation_history"
        })))
        .expect_err("noncanonical session subjects must fail at public FFI ingress");

        assert!(
            matches!(
                err,
                InvocationJsonError::NoncanonicalSessionSubject("subject_ura")
            ),
            "unexpected noncanonical session subject rejection: {err}"
        );
    }

    #[test]
    fn parse_invocation_json_rejects_session_authority_subject_mismatch_before_daemon_io() {
        let session_authority = signed_authority_metadata_value(serde_json::json!({
            "issuer_ura": "easynet:///r/acme/device/dev-a",
            "session_id": "session-1",
            "session_owner_user_id": "alice",
            "creator_principal_id": "easynet:///r/acme/device/dev-a",
            "callee_ura": "easynet:///r/acme/device/dev-a",
            "subject_ura": "easynet:///r/acme/resource/user.alice/session/session-1",
            "audience": "easynet:///r/acme/device/dev-a",
            "scopes": ["invocation.history.list"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["invocation.history.list"],
            "issued_at_ms": now_ms() - 1_000,
            "expires_at_ms": now_ms() + 60_000
        }));

        let err = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "metadata": {
                crate::daemon::invocation::admission::authority_metadata::SESSION_AUTHORITY_METADATA_KEY: session_authority
            }
        })))
        .expect_err("session authority must admit the public invocation subject");

        assert!(
            matches!(err, InvocationJsonError::AuthoritySubjectMismatch(_)),
            "unexpected authority mismatch rejection: {err}"
        );
        assert!(
            err.to_string()
                .contains("does not admit invocation subject"),
            "error should name subject-admission mismatch: {err}"
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

    #[cfg(feature = "axon-pb")]
    fn test_cancellation_gate(owner_ura: &str) -> InvocationCancellationGate {
        InvocationCancellationGate::Available(crate::daemon::InvocationCancellationAuthority::new(
            Arc::new(
                crate::daemon::identity::self_identity::TestCanonicalSigner::new(
                    owner_ura, [0x42; 32],
                ),
            ),
        ))
    }

    #[cfg(feature = "axon-pb")]
    fn unavailable_cancellation_gate(reason: impl Into<String>) -> InvocationCancellationGate {
        InvocationCancellationGate::Unavailable {
            reason: reason.into(),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_millis() as i64
    }

    fn signed_authority_metadata_value(payload: serde_json::Value) -> String {
        use base64::Engine as _;

        base64::engine::general_purpose::STANDARD.encode(
            serde_json::json!({
                "payload": payload,
                "signature": "c2lnbmF0dXJl"
            })
            .to_string(),
        )
    }

    fn write_runtime_discovery(
        directory: &std::path::Path,
        mode: &str,
        realm: &str,
        node_id: Option<&str>,
    ) -> std::path::PathBuf {
        write_runtime_discovery_with_flags(directory, mode, realm, node_id, Vec::new())
    }

    fn write_runtime_discovery_with_flags(
        directory: &std::path::Path,
        mode: &str,
        realm: &str,
        node_id: Option<&str>,
        capability_flags: Vec<String>,
    ) -> std::path::PathBuf {
        let path = directory.join(crate::daemon::control::discovery::CONTROL_JSON_FILENAME);
        crate::daemon::control::discovery::write(
            &path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: None,
                pipe_name: None,
                invocation_endpoint: None,
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: mode.to_string(),
                    realm: realm.to_string(),
                    node_id: node_id.map(str::to_string),
                }),
                pid: std::process::id(),
                daemon_version: "test".to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    crate::daemon::control::discovery::IPC_VERSION_V1,
                ),
                capability_flags,
                pages_port: None,
            },
        )
        .expect("write runtime discovery");
        path
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

    #[test]
    #[cfg(unix)]
    fn session_invocation_authority_signs_unsigned_owner_through_key_service() {
        use base64::Engine as _;
        use ed25519_dalek::Verifier as _;

        with_test_key_service(3, |_managed_entry| {
            let directory = tempfile::tempdir().expect("runtime discovery directory");
            let control_path =
                write_runtime_discovery(directory.path(), "device", "acme", Some("dev-a"));
            let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
                control_path.display().to_string(),
                None,
            );
            let runtime_public_key =
                crate::daemon::identity::self_identity::KeyringClient::default_path()
                    .ensure("easynet:///r/acme/device/dev-a")
                    .expect("test fixture must provision the runtime owner explicitly");
            let runtime_public_key_b64 =
                base64::engine::general_purpose::STANDARD.encode(runtime_public_key.to_bytes());
            let invocation =
                InvocationJson::parse(&canonical_invocation_json(serde_json::json!({})))
                    .expect("parse complete invocation")
                    .into_daemon_invocation()
                    .expect("build daemon invocation");
            let canonical_bytes = invocation
                .clone()
                .into_draft()
                .prepare(crate::daemon::PrepareOptions::default())
                .expect("prepare signing material")
                .signing_material()
                .canonical_bytes()
                .to_vec();

            let (bound, cancellation_authority) = lib_runtime()
                .expect("library runtime")
                .block_on(SessionInvocationAuthority::new(&session).bind_cancellable(invocation))
                .expect("session owner invocation must bind");
            assert_eq!(
                cancellation_authority.owner_ura(),
                "easynet:///r/acme/device/dev-a"
            );
            let signature = bound.signature();
            assert_eq!(signature.algorithm, "ed25519");
            assert_eq!(signature.key_id_hint, runtime_public_key_b64);
            let signature =
                ed25519_dalek::Signature::from_slice(&signature.signature).expect("signature");
            runtime_public_key
                .verify(&canonical_bytes, &signature)
                .expect("session signature must verify over canonical bytes");
        });
    }

    #[test]
    #[cfg(unix)]
    fn session_invocation_authority_admits_ready_paired_user_signer() {
        use base64::Engine as _;
        use ed25519_dalek::Verifier as _;

        let user_ura = "easynet:///r/acme/user/user-alice";
        with_test_key_service_for(
            user_ura,
            crate::daemon::identity::self_identity::USER_SIGNING_CLI_PURPOSE,
            2,
            |managed_entry| {
                struct HomeRestore {
                    previous_home: Option<std::ffi::OsString>,
                }

                impl Drop for HomeRestore {
                    fn drop(&mut self) {
                        match self.previous_home.take() {
                            Some(value) => std::env::set_var("HOME", value),
                            None => std::env::remove_var("HOME"),
                        }
                    }
                }

                let directory = tempfile::tempdir().expect("runtime discovery directory");
                let _home_restore = HomeRestore {
                    previous_home: std::env::var_os("HOME"),
                };
                std::env::set_var("HOME", directory.path());
                crate::daemon::persistence::config::save_credentials(
                    &crate::daemon::persistence::config::Credentials {
                        node_id: "dev-a".to_string(),
                        credential_token: "token".to_string(),
                        hub_endpoint: "https://hub.example:50443".to_string(),
                        realm: "acme".to_string(),
                        deploy_signature: String::new(),
                        hub_api_base: None,
                        username: Some("alice".to_string()),
                        user_id: Some("user-alice".to_string()),
                        hub_pubkey_b64: None,
                        hub_tls_ca_pem_b64: None,
                        join_receipt_hash: None,
                    },
                )
                .expect("paired credentials");
                let control_path = write_runtime_discovery_with_flags(
                    directory.path(),
                    "device",
                    "acme",
                    Some("dev-a"),
                    vec![
                        crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER
                            .to_string(),
                    ],
                );
                let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
                    control_path.display().to_string(),
                    None,
                );
                let invocation =
                    InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
                        "caller_ura": user_ura,
                    })))
                    .expect("parse paired user invocation")
                    .into_daemon_invocation()
                    .expect("build daemon invocation");
                let canonical_bytes = invocation
                    .clone()
                    .into_draft()
                    .prepare(crate::daemon::PrepareOptions::default())
                    .expect("prepare signing material")
                    .signing_material()
                    .canonical_bytes()
                    .to_vec();

                let (bound, cancellation_authority) = lib_runtime()
                    .expect("library runtime")
                    .block_on(
                        SessionInvocationAuthority::new(&session).bind_cancellable(invocation),
                    )
                    .expect("Ready-proven paired user invocation must bind");
                assert_eq!(cancellation_authority.owner_ura(), user_ura);
                let signature = bound.signature();
                assert_eq!(signature.algorithm, "ed25519");
                assert_eq!(signature.key_id_hint, managed_entry.public_key_b64);
                let public_key = base64::engine::general_purpose::STANDARD
                    .decode(managed_entry.public_key_b64)
                    .expect("managed public key base64");
                let public_key = ed25519_dalek::VerifyingKey::from_bytes(
                    public_key.as_slice().try_into().expect("public key length"),
                )
                .expect("managed public key");
                let signature =
                    ed25519_dalek::Signature::from_slice(&signature.signature).expect("signature");
                public_key
                    .verify(&canonical_bytes, &signature)
                    .expect("paired user signature must verify over canonical bytes");
            },
        );
    }

    #[test]
    fn session_invocation_authority_rejects_unsigned_non_owner() {
        let directory = tempfile::tempdir().expect("runtime discovery directory");
        let control_path =
            write_runtime_discovery(directory.path(), "device", "acme", Some("dev-a"));
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            None,
        );
        let invocation = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "caller_ura": "easynet:///r/acme/device/dev-b"
        })))
        .expect("parse complete invocation")
        .into_daemon_invocation()
        .expect("build daemon invocation");

        let error = lib_runtime()
            .expect("library runtime")
            .block_on(SessionInvocationAuthority::new(&session).bind(invocation))
            .expect_err("unsigned non-owner must fail closed");

        assert!(
            error.to_string().contains(
                "caller `easynet:///r/acme/device/dev-b` is not admitted by session authority \
                     owner `easynet:///r/acme/device/dev-a`",
            ),
            "unexpected owner binding error: {error}"
        );
    }

    #[test]
    fn session_invocation_authority_preserves_explicit_caller_signature() {
        let session = test_session();
        let invocation = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "caller_signature": {
                "algorithm": "ed25519",
                "signature_base64": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==",
                "key_id_hint": "explicit-caller-key"
            }
        })))
        .expect("parse explicitly signed invocation")
        .into_daemon_invocation()
        .expect("build daemon invocation");

        let bound = lib_runtime()
            .expect("library runtime")
            .block_on(SessionInvocationAuthority::new(&session).bind(invocation))
            .expect("explicit signature must not require session discovery");
        let signature = bound.signature();

        assert_eq!(signature.algorithm, "ed25519");
        assert_eq!(signature.signature, vec![7; 64]);
        assert_eq!(signature.key_id_hint, "explicit-caller-key");
    }

    #[test]
    fn session_invocation_authority_rejects_foreign_cancellation_authority() {
        let directory = tempfile::tempdir().expect("runtime discovery directory");
        let control_path =
            write_runtime_discovery(directory.path(), "device", "acme", Some("dev-a"));
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            None,
        );
        let invocation = InvocationJson::parse(&canonical_invocation_json(serde_json::json!({
            "caller_ura": "easynet:///r/acme/device/dev-b",
            "caller_signature": {
                "algorithm": "ed25519",
                "signature_base64": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==",
                "key_id_hint": "external-caller-key"
            }
        })))
        .expect("parse externally signed invocation")
        .into_daemon_invocation()
        .expect("build daemon invocation");

        let error = lib_runtime()
            .expect("library runtime")
            .block_on(SessionInvocationAuthority::new(&session).bind_cancellable(invocation))
            .expect_err("foreign caller must not inherit the session cancellation authority");

        assert!(
            error.to_string().contains(
                "caller `easynet:///r/acme/device/dev-b` is not admitted by session authority \
                     owner `easynet:///r/acme/device/dev-a`",
            ),
            "unexpected cancellation authority error: {error}"
        );
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
            runtime_invocation_prepare(
                prepare_handle,
                raw.as_ptr(),
                std::ptr::null(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };
        assert_eq!(prepare_code, RUNTIME_OK);
        unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };

        let signature = signature_json();
        let mut signed_id: SignedInvocationId = 0;
        let mut signed_json_ptr: *mut c_char = std::ptr::null_mut();
        let sign_code = unsafe {
            runtime_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut signed_id,
                &mut signed_json_ptr,
            )
        };
        assert_eq!(sign_code, RUNTIME_OK);
        unsafe { crate::ffi::strings::runtime_string_free(signed_json_ptr) };
        crate::ffi::client::handle::release(prepare_handle);
        signed_id
    }

    fn signed_fixture_tuple() -> crate::daemon::InvocationTuple {
        let signed_id = new_signed_invocation_id();
        let tuple = get_signed(signed_id).unwrap().prepared().tuple();
        assert_eq!(runtime_signed_invocation_free(signed_id), RUNTIME_OK);
        tuple
    }

    fn canonical_finalized_response(
        terminal_state: axon_sdk::invocation::InvocationState,
    ) -> (
        crate::daemon::InvocationTuple,
        axon_sdk::pb::axon::v1::InvokeResponse,
        std::sync::Arc<dyn axon_sdk::invocation::KeyResolver>,
    ) {
        use axon_sdk::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, AxonError, CallMode, CausalContext,
        };

        let callee_ura = "easynet:///r/acme/device/dev-a";
        let subject_ura = "easynet:///r/acme/device/dev-a";
        let ability = match terminal_state {
            axon_sdk::invocation::InvocationState::Completed => "test.ffi.completed",
            axon_sdk::invocation::InvocationState::Cancelled => "test.ffi.cancelled",
            state => panic!("unsupported finalized response fixture state {state:?}"),
        };
        let schema_hash = [0x11; 32];
        let impl_hash = [0x22; 32];
        let descriptor_hash = [0x33; 32];
        let descriptor_version = "1.0.0";
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                descriptor_version,
                descriptor_hash,
                "invoke",
            )
            .expect("fixture descriptor binding");
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                callee_ura,
                ability,
                &descriptor_binding,
            )
            .expect("fixture descriptor ref");
        let runtime =
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime_with_receipt_provider(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                crate::daemon::axon_bridge::runtime_factory::ephemeral_test_canonical_receipt_provider(),
            );
        let resolver =
            crate::daemon::axon_bridge::runtime_factory::ephemeral_test_receipt_key_resolver();
        let pending = terminal_state == axon_sdk::invocation::InvocationState::Cancelled;
        let arguments = b"{}".to_vec();

        let (tuple, response) = lib_runtime()
            .expect("library runtime")
            .block_on(async {
                let ability_ura =
                    crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
                        callee_ura, ability,
                    )
                    .expect("fixture ability URA");
                runtime
                    .register_ability_with_options(
                        ability_ura,
                        make_ability(move |_| async move {
                            if pending {
                                std::future::pending::<Result<Vec<u8>, AxonError>>().await
                            } else {
                                Ok(br#"{"ok":true}"#.to_vec())
                            }
                        }),
                        AbilityOptions::default()
                            .with_modes(AbilityCallModes::RPC)
                            .with_descriptor_proof(
                                descriptor_version,
                                "invoke",
                                descriptor_hash,
                                schema_hash,
                                impl_hash,
                            ),
                    )
                    .await
                    .expect("register fixture ability");
                let request =
                    crate::daemon::axon_bridge::local_runtime_request::SystemInvocationIssuer::request_for_descriptor_ref(
                        CallMode::Rpc,
                        callee_ura,
                        descriptor_ref.clone(),
                        subject_ura,
                        arguments.clone(),
                        CausalContext::None,
                        Default::default(),
                    )
                    .expect("build fixture descriptor-bound request");
                let envelope = request.envelope().envelope();
                let tuple = crate::daemon::InvocationTuple {
                    caller_ura: envelope.caller.ura.clone(),
                    callee_ura: envelope.callee.ura.clone(),
                    descriptor_ref: envelope.ability.clone(),
                    subject_ura: envelope.subject.ura.clone(),
                    nonce_base64: {
                        use base64::Engine as _;
                        base64::engine::general_purpose::STANDARD
                            .encode(envelope.invocation_nonce)
                    },
                    causal_context: serde_json::json!({"form": "none"}),
                    args_digest_hex: hex::encode(axon_sdk::invocation::sha256(&arguments)),
                    content_type: "application/json".to_string(),
                    metadata: std::collections::HashMap::new(),
                    timeout_seconds: None,
                };
                let (handle, _signed) = runtime
                    .invoke_descriptor_bound_request_async(request)
                    .await
                    .expect("start fixture invocation");
                if pending {
                    handle
                        .cancel("client stop")
                        .await
                        .expect("cancel fixture invocation");
                }
                let finalized = handle.finalized().await.expect("finalized fixture");
                assert_eq!(finalized.terminal_state, terminal_state);
                let response = axon_sdk::pb::axon::v1::InvokeResponse {
                    state: finalized.terminal_state.to_wire_i32(),
                    result: finalized.output().to_vec(),
                    result_content_type: "application/json".to_string(),
                    error: finalized
                        .failure
                        .as_ref()
                        .map(axon_sdk::invocation::wire::error_to_wire),
                    admission_receipt: Some(
                        axon_sdk::invocation::wire::receipt_to_wire(
                            &finalized.admission_receipt,
                        )
                        .expect("project admission receipt"),
                    ),
                    terminal_receipt: Some(
                        axon_sdk::invocation::wire::receipt_to_wire(
                            &finalized.terminal_receipt,
                        )
                        .expect("project terminal receipt"),
                    ),
                    ..Default::default()
                };
                (tuple, response)
            });
        (tuple, response, resolver)
    }

    #[test]
    fn unary_result_projects_terminal_receipt_without_losing_admission_checkpoint() {
        let (tuple, response, resolver) =
            canonical_finalized_response(axon_sdk::invocation::InvocationState::Completed);
        let outcome = crate::daemon::InvocationOutcome::from_invoke_response(
            tuple,
            response,
            resolver.as_ref(),
        )
        .expect("canonical signed finalization must project");
        let admission_index = outcome
            .stages()
            .admission()
            .map(|receipt| receipt.index)
            .expect("admission checkpoint");
        let terminal_index = outcome
            .stages()
            .terminal()
            .map(|receipt| receipt.index)
            .expect("terminal checkpoint");
        assert_eq!(
            outcome
                .result()
                .receipt
                .as_ref()
                .map(|receipt| receipt.index),
            Some(terminal_index)
        );
        assert!(admission_index < terminal_index);

        let json = invocation_outcome_json_with_tuple(outcome, serde_json::json!({}))
            .expect("canonical JSON result projection");
        assert_eq!(json["output_json"], serde_json::json!({"ok": true}));
        assert!(json["output_base64"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(json["admission_receipt"]["index"], admission_index);
        assert_eq!(json["terminal_receipt"]["index"], terminal_index);
        assert!(json["admission_receipt"]["receipt_ura"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(json["terminal_receipt"]["receipt_ura"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(
            json.get("receipt").is_none(),
            "unary result JSON must expose terminal_receipt, not the retired receipt alias"
        );
        assert_eq!(
            json["terminal_receipt"]["caller_binding"]["ura"],
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
        );
        assert!(json["terminal_receipt"]["invocation_nonce_base64"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(
            json["terminal_receipt"]["callee_signature"]["signature_base64"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(
            json["terminal_receipt"]["signer_binding"], json["terminal_receipt"]["callee_binding"],
            "self-signed receipts expose the effective callee signer"
        );
        assert_eq!(json["terminal_receipt"]["host_attestation_base64"], "");
        assert_eq!(json["terminal_receipt"]["causal_binding_kind"], "none");
        assert_eq!(json["terminal_receipt"]["causal_binding"]["form"], "none");
        assert_eq!(json["terminal_receipt"]["authority_binding_kind"], "self");
        assert_eq!(
            json["terminal_receipt"]["authority_binding"]["principal_ura"],
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
        );
        assert_eq!(json["terminal_receipt"]["descriptor_version"], "1.0.0");
        assert_eq!(json["terminal_receipt"]["schema_hash_hex"], "11".repeat(32));
        assert_eq!(json["terminal_receipt"]["impl_hash_hex"], "22".repeat(32));
        assert!(
            json["terminal_receipt"]["authority_proof"]["proof_hash_hex"]
                .as_str()
                .is_some_and(|value| value.len() == 64)
        );
        assert!(
            json["admission_receipt"]["authority_proof"]["signature"]["signature_base64"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "admission checkpoint must carry a signed authority proof fact"
        );
        assert!(
            json["terminal_receipt"]["authority_proof"]["signature"]["signature_base64"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "terminal receipt must carry a signed authority proof fact"
        );
        assert_eq!(
            json["terminal_receipt"]["authority_proof"]["binding"]["kind"],
            "self"
        );
    }

    #[test]
    fn unary_result_json_rejects_declared_json_output_that_is_not_json() {
        let result = crate::daemon::InvocationResult {
            tuple: signed_fixture_tuple(),
            terminal_state: "Completed".to_string(),
            output_content_type: "application/json".to_string(),
            output: b"not-json".to_vec(),
            elapsed_ms: 7,
            receipt: None,
            error: None,
        };
        let outcome = crate::daemon::InvocationOutcome::new(
            result,
            crate::daemon::InvocationReceiptStages::default(),
        );

        let error = invocation_outcome_json_with_tuple(outcome, serde_json::json!({}))
            .expect_err("declared JSON output must fail closed");
        assert!(error.contains("output_json declares JSON content type"));
        assert!(error.contains("payload is not valid JSON"));
    }

    #[test]
    fn unary_result_json_projects_empty_declared_json_output_as_no_value() {
        let result = crate::daemon::InvocationResult {
            tuple: signed_fixture_tuple(),
            terminal_state: "Completed".to_string(),
            output_content_type: "application/json".to_string(),
            output: Vec::new(),
            elapsed_ms: 7,
            receipt: None,
            error: None,
        };
        let outcome = crate::daemon::InvocationOutcome::new(
            result,
            crate::daemon::InvocationReceiptStages::default(),
        );

        let json = invocation_outcome_json_with_tuple(outcome, serde_json::json!({})).unwrap();

        assert_eq!(json["output_base64"], "");
        assert!(json["output_json"].is_null());
    }

    fn active_bidi_session(
        owner: ClientSessionBinding,
        capacity: usize,
    ) -> (
        ActiveInvocationBidi,
        tokio::sync::mpsc::Receiver<axon_sdk::pb::axon::v1::InvokeBidiUp>,
        tokio_util::sync::CancellationToken,
    ) {
        active_bidi_session_with_cancellation(owner, capacity, test_cancellation_control())
    }

    fn active_bidi_session_with_cancellation(
        owner: ClientSessionBinding,
        capacity: usize,
        cancellation: Arc<ProviderCancellationControl>,
    ) -> (
        ActiveInvocationBidi,
        tokio::sync::mpsc::Receiver<axon_sdk::pb::axon::v1::InvokeBidiUp>,
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
                cancellation,
            ),
            up_rx,
            cancel,
        )
    }

    fn active_bidi_len_for_owner(owner: ClientSessionBinding) -> usize {
        lock_bidi_entries(bidi_registry())
            .values()
            .filter(|session| session.owner == owner)
            .count()
    }

    fn assert_bidi_eof_frame(frame: axon_sdk::pb::axon::v1::InvokeBidiUp, sequence: u64) {
        use axon_sdk::pb::axon::v1::{bidi_control, invoke_bidi_up};
        assert_eq!(frame.sequence, sequence);
        assert_eq!(frame.mac, vec![0xA5; BIDI_FRAME_CHAIN_MAC_BYTES]);
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

    fn test_bidi_mac_base64() -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode([0xA5; BIDI_FRAME_CHAIN_MAC_BYTES])
    }

    fn read_last_error_json() -> serde_json::Value {
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { crate::ffi::errors::runtime_last_error_json(&mut out) };
        assert_eq!(code, RUNTIME_OK);
        assert!(!out.is_null());
        let value = unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(out) };
        value
    }

    fn assert_typed_last_error(code_name: &str, abi_code: i32, message_fragment: &str) {
        let error = read_last_error_json();
        assert_eq!(error["code"], code_name);
        assert_eq!(error["details"]["abi_code"], abi_code);
        assert!(error["details"]["abi_symbol"].as_str().is_some());
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| message.contains(message_fragment)),
            "last-error message should contain {message_fragment:?}: {error}"
        );
    }

    fn new_builder_handle() -> InvocationBuilderId {
        let mut builder_id: InvocationBuilderId = 0;
        let code = unsafe { runtime_invocation_builder_new(&mut builder_id) };
        assert_eq!(code, RUNTIME_OK);
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
            unsafe { runtime_invocation_builder_set_caller(builder_id, caller_ura.as_ptr()) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe { runtime_invocation_builder_set_callee(builder_id, callee_ura.as_ptr()) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe {
                runtime_invocation_builder_set_descriptor_ref(builder_id, descriptor.as_ptr())
            },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe { runtime_invocation_builder_set_subject(builder_id, subject.as_ptr()) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe { runtime_invocation_builder_set_nonce_base64(builder_id, nonce.as_ptr()) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe {
                runtime_invocation_builder_set_causal_context_json(builder_id, causal.as_ptr())
            },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe { runtime_invocation_builder_set_args_json(builder_id, args.as_ptr()) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe { runtime_invocation_builder_set_metadata_json(builder_id, metadata.as_ptr()) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe {
                runtime_invocation_builder_set_idempotency_key(builder_id, idempotency_key.as_ptr())
            },
            RUNTIME_OK
        );
        assert_eq!(
            runtime_invocation_builder_set_timeout_seconds(builder_id, 45),
            RUNTIME_OK
        );
    }

    #[test]
    fn invocation_builder_inspect_rejects_incomplete_tuple() {
        let builder_id = new_builder_handle();
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { runtime_invocation_builder_inspect(builder_id, &mut out) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        assert!(get_builder(builder_id).is_some());
        assert_eq!(runtime_invocation_builder_free(builder_id), RUNTIME_OK);
    }

    #[test]
    fn invocation_builder_inspect_and_build_preserve_complete_tuple_state() {
        let builder_id = new_builder_handle();
        set_complete_builder(builder_id);

        let mut inspect_ptr: *mut c_char = std::ptr::null_mut();
        let inspect_code =
            unsafe { runtime_invocation_builder_inspect(builder_id, &mut inspect_ptr) };
        assert_eq!(inspect_code, RUNTIME_OK);
        assert!(get_builder(builder_id).is_some());
        let inspect_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(inspect_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(inspect_ptr) };
        assert_eq!(inspect_json["args"]["probe"], true);
        assert_eq!(inspect_json["metadata"]["trace"], "sdk-builder");
        assert_eq!(inspect_json["metadata"]["idempotency_key"], "idem-1");
        assert!(inspect_json.get("timeout_seconds").is_none());

        let mut build_ptr: *mut c_char = std::ptr::null_mut();
        let build_code = unsafe { runtime_invocation_builder_build(builder_id, &mut build_ptr) };
        assert_eq!(build_code, RUNTIME_OK);
        assert!(get_builder(builder_id).is_none());
        let build_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(build_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(build_ptr) };
        assert_eq!(build_json["descriptor_ref"], inspect_json["descriptor_ref"]);
        assert!(build_json.get("timeout_seconds").is_none());

        let mut second_ptr: *mut c_char = std::ptr::dangling_mut();
        let second_code =
            unsafe { runtime_invocation_builder_inspect(builder_id, &mut second_ptr) };
        assert_eq!(second_code, ERR_INVALID_HANDLE);
        assert!(second_ptr.is_null());
        assert_typed_last_error("INVALID_HANDLE", ERR_INVALID_HANDLE, "builder handle");
    }

    #[test]
    fn builder_rejects_receipt_history_descriptor_before_daemon_io() {
        let builder_id = new_builder_handle();
        set_complete_builder(builder_id);
        let history_descriptor_ref = CString::new(format!(
            "{}@1.0.0#{}!read",
            crate::core::ura::owner_ability_ura(
                "easynet:///r/acme/device/dev-a",
                crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST
            )
            .expect("history ability URA"),
            "aa".repeat(32)
        ))
        .unwrap();
        assert_eq!(
            unsafe {
                runtime_invocation_builder_set_descriptor_ref(
                    builder_id,
                    history_descriptor_ref.as_ptr(),
                )
            },
            RUNTIME_OK
        );

        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { runtime_invocation_builder_build(builder_id, &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        assert!(get_builder(builder_id).is_some());
        assert_typed_last_error(
            "INVALID_ARGUMENT",
            ERR_INVALID_ARG,
            "canonical invocation history read path",
        );
        assert_eq!(runtime_invocation_builder_free(builder_id), RUNTIME_OK);
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
            runtime_invocation_builder_prepare(
                handle,
                builder_id,
                options.as_ptr(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };

        assert_eq!(code, RUNTIME_OK);
        assert_ne!(prepared_id, 0);
        assert!(get_builder(builder_id).is_none());
        assert!(get_prepared(prepared_id).is_some());
        let prepared_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(prepared_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };
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
        assert_eq!(prepared_json["prepared_id"], prepared_id.to_string());
        assert_eq!(runtime_prepared_invocation_free(prepared_id), RUNTIME_OK);
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
            runtime_invocation_builder_prepare(
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
        let build_code = unsafe { runtime_invocation_builder_build(builder_id, &mut build_ptr) };
        assert_eq!(build_code, RUNTIME_OK);
        assert!(get_builder(builder_id).is_none());
        unsafe { crate::ffi::strings::runtime_string_free(build_ptr) };
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
                runtime_invocation_builder_set_arguments_base64(
                    builder_id,
                    payload.as_ptr(),
                    content_type.as_ptr(),
                )
            },
            RUNTIME_OK
        );

        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { runtime_invocation_builder_build(builder_id, &mut out) };
        assert_eq!(code, RUNTIME_OK);
        let json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(out) };
        assert_eq!(json["arguments_base64"], "AQID");
        assert_eq!(json["content_type"], "application/octet-stream");
        assert!(json.get("args").is_none());
    }

    #[test]
    fn timeout_seconds_passes_through_to_the_invoke_request_wire(// F-045
    ) {
        let signed_request = |raw: &str| {
            InvocationJson::parse(raw)
                .expect("parse")
                .into_daemon_invocation()
                .expect("build")
                .into_draft()
                .prepare(crate::daemon::PrepareOptions::default())
                .expect("prepare")
                .sign_with_caller_signature(crate::daemon::CallerSignatureMaterial::new(
                    "ed25519",
                    vec![7; 64],
                    "caller-key",
                ))
                .expect("sign")
                .into_daemon_invocation()
                .into_request()
                .expect("request")
        };
        let raw = canonical_invocation_json(serde_json::json!({"timeout_seconds": 45}));
        let request = signed_request(&raw);
        assert_eq!(request.timeout_seconds, 45);

        // Absent field = proto default (0 → daemon default budget).
        let request = signed_request(&canonical_invocation_json(serde_json::json!({})));
        assert_eq!(request.timeout_seconds, 0);

        // Non-positive and non-integer values are typed parse errors.
        for bad in ["0", "-3", "\"45\"", "1.5"] {
            let raw = format!(
                r#"{{
                    "caller_ura": "easynet:///r/acme/device/dev-a",
                    "callee_ura": "easynet:///r/acme/device/dev-a",
                    "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
                "provider_managed_signing": false
            })
            .to_string(),
        )
        .unwrap();
        let mut prepared_id: PreparedInvocationId = 0;
        let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            runtime_invocation_prepare(
                handle,
                raw.as_ptr(),
                options.as_ptr(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };

        assert_eq!(code, RUNTIME_OK);
        assert_ne!(prepared_id, 0);
        assert!(get_prepared(prepared_id).is_some());
        let prepared_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(prepared_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };
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
        assert_eq!(prepared_json["prepared_id"], prepared_id.to_string());

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
            runtime_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut signed_id,
                &mut signed_json_ptr,
            )
        };

        assert_eq!(code, RUNTIME_OK);
        assert_ne!(signed_id, 0);
        assert!(get_prepared(prepared_id).is_none());
        assert!(get_signed(signed_id).is_some());
        let signed_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(signed_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::runtime_string_free(signed_json_ptr) };
        assert_eq!(signed_json["signer_id"], "browser-key");
        assert_eq!(signed_json["policy"]["mode"], "caller_signing");
        assert_eq!(signed_json["policy"]["signer_id"], "browser-key");
        assert_eq!(signed_json["policy"]["policy_ref"], "policy/local");
        assert_eq!(signed_json["signature"]["algorithm"], "ed25519");
        assert_eq!(signed_json["signature"]["key_id_hint"], "caller-key");

        let mut duplicate_signed_id: SignedInvocationId = 0;
        let mut duplicate_signed_json_ptr: *mut c_char = std::ptr::null_mut();
        let duplicate_code = unsafe {
            runtime_invocation_sign_prepared(
                prepared_id,
                signature.as_ptr(),
                &mut duplicate_signed_id,
                &mut duplicate_signed_json_ptr,
            )
        };
        assert_eq!(duplicate_code, ERR_INVALID_HANDLE);
        assert_eq!(duplicate_signed_id, 0);
        assert!(duplicate_signed_json_ptr.is_null());
        assert_eq!(runtime_signed_invocation_free(signed_id), RUNTIME_OK);
        assert!(get_signed(signed_id).is_none());
    }

    #[test]
    fn invocation_sign_prepared_validation_failure_preserves_prepared_handle() {
        let (handle, _session) = alloc(test_session());
        let raw = CString::new(canonical_invocation_json(serde_json::json!({
            "args": {"probe": true}
        })))
        .unwrap();
        let options = CString::new(
            serde_json::json!({
                "expires_in_ms": 60_000,
                "provider_managed_signing": false
            })
            .to_string(),
        )
        .unwrap();
        let mut prepared_id: PreparedInvocationId = 0;
        let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
        let prepare_code = unsafe {
            runtime_invocation_prepare(
                handle,
                raw.as_ptr(),
                options.as_ptr(),
                &mut prepared_id,
                &mut prepared_json_ptr,
            )
        };
        assert_eq!(prepare_code, RUNTIME_OK);
        unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };

        let invalid_signature = CString::new(
            serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "AQ=="
            })
            .to_string(),
        )
        .unwrap();
        let mut signed_id: SignedInvocationId = 99;
        let mut signed_json_ptr: *mut c_char = std::ptr::dangling_mut();
        let sign_code = unsafe {
            runtime_invocation_sign_prepared(
                prepared_id,
                invalid_signature.as_ptr(),
                &mut signed_id,
                &mut signed_json_ptr,
            )
        };

        assert_eq!(sign_code, ERR_INVALID_ARG);
        assert_eq!(signed_id, 0);
        assert!(signed_json_ptr.is_null());
        assert!(get_prepared(prepared_id).is_some());
        assert_typed_last_error(
            "INVALID_ARGUMENT",
            ERR_INVALID_ARG,
            "missing field `key_id_hint`",
        );
        assert_eq!(runtime_prepared_invocation_free(prepared_id), RUNTIME_OK);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_prepare_material_only_does_not_allocate_a_prepared_handle() {
        let (handle, _session) = alloc(test_session());
        let raw = CString::new(canonical_invocation_json(serde_json::json!({
            "args": {"browser": true}
        })))
        .unwrap();
        let material_only_options = CString::new(
            serde_json::json!({
                "expires_in_ms": 60_000,
                "signer_id": "browser-key",
                "material_only": true
            })
            .to_string(),
        )
        .unwrap();
        let mut material_only_id: PreparedInvocationId = 0;
        let mut material_only_json_ptr: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            runtime_invocation_prepare(
                handle,
                raw.as_ptr(),
                material_only_options.as_ptr(),
                &mut material_only_id,
                &mut material_only_json_ptr,
            )
        };

        assert_eq!(code, RUNTIME_OK);
        assert_eq!(material_only_id, 0);
        let material_only_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(material_only_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::runtime_string_free(material_only_json_ptr) };
        assert!(
            material_only_json["signing_material"]["canonical_bytes_base64"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );

        let normal_options =
            CString::new(r#"{"expires_in_ms":60000,"signer_id":"browser-key"}"#).unwrap();
        let mut normal_id: PreparedInvocationId = 0;
        let mut normal_json_ptr: *mut c_char = std::ptr::null_mut();
        let normal_code = unsafe {
            runtime_invocation_prepare(
                handle,
                raw.as_ptr(),
                normal_options.as_ptr(),
                &mut normal_id,
                &mut normal_json_ptr,
            )
        };
        assert_eq!(normal_code, RUNTIME_OK);
        assert_ne!(normal_id, 0);
        let normal_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(normal_json_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::runtime_string_free(normal_json_ptr) };
        assert_eq!(normal_json["prepared_id"], normal_id.to_string());
        assert_eq!(runtime_prepared_invocation_free(normal_id), RUNTIME_OK);
    }

    #[test]
    fn invocation_prepare_same_draft_allocates_distinct_request_and_handle_ids() {
        let (handle, _session) = alloc(test_session());
        let raw = CString::new(canonical_invocation_json(serde_json::json!({
            "args": {"probe": true}
        })))
        .unwrap();
        let options = CString::new(r#"{"expires_in_ms":60000,"signer_id":"browser-key"}"#).unwrap();

        let prepare_once = || {
            let mut prepared_id: PreparedInvocationId = 0;
            let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
            let code = unsafe {
                runtime_invocation_prepare(
                    handle,
                    raw.as_ptr(),
                    options.as_ptr(),
                    &mut prepared_id,
                    &mut prepared_json_ptr,
                )
            };
            assert_eq!(code, RUNTIME_OK);
            assert_ne!(prepared_id, 0);
            let json: serde_json::Value = unsafe {
                serde_json::from_str(CStr::from_ptr(prepared_json_ptr).to_str().unwrap()).unwrap()
            };
            unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };
            (prepared_id, json)
        };

        let (first_id, first_json) = prepare_once();
        let (second_id, second_json) = prepare_once();

        assert_ne!(first_id, second_id);
        assert_eq!(first_json["prepared_id"], first_id.to_string());
        assert_eq!(second_json["prepared_id"], second_id.to_string());
        assert_ne!(first_json["request_id"], second_json["request_id"]);
        assert_eq!(
            first_json["canonical_hash_hex"], second_json["canonical_hash_hex"],
            "same draft must keep the same canonical content hash"
        );

        assert_eq!(runtime_prepared_invocation_free(first_id), RUNTIME_OK);
        assert_eq!(runtime_prepared_invocation_free(second_id), RUNTIME_OK);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    #[cfg(unix)]
    fn invocation_sign_prepared_local_uses_default_provider_key_inventory() {
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
                    "provider_managed_signing": true
                })
                .to_string(),
            )
            .unwrap();
            let mut prepared_id: PreparedInvocationId = 0;
            let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
            let prepare_code = unsafe {
                runtime_invocation_prepare(
                    handle,
                    raw.as_ptr(),
                    options.as_ptr(),
                    &mut prepared_id,
                    &mut prepared_json_ptr,
                )
            };
            assert_eq!(prepare_code, RUNTIME_OK);
            assert_ne!(prepared_id, 0);
            unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };

            let mut signed_id: SignedInvocationId = 0;
            let mut signed_json_ptr: *mut c_char = std::ptr::null_mut();
            let sign_code = unsafe {
                runtime_invocation_sign_prepared_local(
                    prepared_id,
                    &mut signed_id,
                    &mut signed_json_ptr,
                )
            };

            assert_eq!(
                sign_code,
                RUNTIME_OK,
                "provider-managed sign error: {}",
                read_last_error_json()
            );
            assert_ne!(signed_id, 0);
            assert!(get_prepared(prepared_id).is_none());
            assert!(get_signed(signed_id).is_some());
            let signed_json: serde_json::Value = unsafe {
                serde_json::from_str(CStr::from_ptr(signed_json_ptr).to_str().unwrap()).unwrap()
            };
            unsafe { crate::ffi::strings::runtime_string_free(signed_json_ptr) };
            assert_eq!(signed_json["policy"]["mode"], "provider_managed_signing");
            assert_eq!(signed_json["policy"]["signer_id"], signer_id);
            assert_eq!(signed_json["policy"]["policy_ref"], policy_ref);
            assert_eq!(signed_json["signature"]["algorithm"], "ed25519");
            assert_eq!(signed_json["signature"]["key_id_hint"], signer_id);
            assert!(signed_json["signature"]["signature_base64"]
                .as_str()
                .is_some_and(|value| !value.is_empty()));
            assert_eq!(runtime_signed_invocation_free(signed_id), RUNTIME_OK);
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
                    "policy_ref": "provider-key-inventory:sha256:wrong",
                    "provider_managed_signing": true
                })
                .to_string(),
            )
            .unwrap();
            let mut prepared_id: PreparedInvocationId = 0;
            let mut prepared_json_ptr: *mut c_char = std::ptr::null_mut();
            let prepare_code = unsafe {
                runtime_invocation_prepare(
                    handle,
                    raw.as_ptr(),
                    options.as_ptr(),
                    &mut prepared_id,
                    &mut prepared_json_ptr,
                )
            };
            assert_eq!(prepare_code, RUNTIME_OK);
            unsafe { crate::ffi::strings::runtime_string_free(prepared_json_ptr) };

            let mut signed_id: SignedInvocationId = 99;
            let mut signed_json_ptr: *mut c_char = std::ptr::dangling_mut();
            let sign_code = unsafe {
                runtime_invocation_sign_prepared_local(
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
            assert_eq!(runtime_prepared_invocation_free(prepared_id), RUNTIME_OK);
            crate::ffi::client::handle::release(handle);
        });
    }

    #[test]
    fn invocation_handle_submit_rejects_invalid_client_before_consuming_signed() {
        let signed_id = new_signed_invocation_id();
        let mut invocation_handle_id: InvocationHandleId = 999;
        let mut submitted_ptr: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            runtime_invocation_submit_signed_handle(
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
        assert_eq!(runtime_signed_invocation_free(signed_id), RUNTIME_OK);
    }

    #[test]
    fn invocation_handle_await_reports_transport_failure_without_terminal_state() {
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
            runtime_invocation_submit_signed_handle(
                client_handle,
                signed_id,
                &mut invocation_handle_id,
                &mut submitted_ptr,
            )
        };
        assert_eq!(submit_code, RUNTIME_OK);
        assert_ne!(invocation_handle_id, 0);
        assert!(get_signed(signed_id).is_none());
        let submitted_json: serde_json::Value = unsafe {
            serde_json::from_str(CStr::from_ptr(submitted_ptr).to_str().unwrap()).unwrap()
        };
        unsafe { crate::ffi::strings::runtime_string_free(submitted_ptr) };
        assert_eq!(submitted_json["state"], "Submitted");
        assert_eq!(submitted_json["terminal"], false);

        let mut result_ptr: *mut c_char = std::ptr::null_mut();
        let await_code = unsafe {
            runtime_invocation_handle_await(client_handle, invocation_handle_id, &mut result_ptr)
        };
        assert_eq!(await_code, ERR_DAEMON_DOWN);
        assert!(result_ptr.is_null());

        let mut events_ptr: *mut c_char = std::ptr::null_mut();
        let events_code = unsafe {
            runtime_invocation_handle_events(client_handle, invocation_handle_id, &mut events_ptr)
        };
        assert_eq!(events_code, RUNTIME_OK);
        let events_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(events_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(events_ptr) };
        assert_eq!(events_json["terminal"], false);
        assert_eq!(events_json["events"][0]["state"], "Submitted");
        assert_eq!(events_json["events"][1]["kind"], "observation_failed");
        assert_eq!(events_json["events"][1]["terminal"], false);
        assert_eq!(
            events_json["observation_error"]["abi_code"],
            ERR_DAEMON_DOWN
        );
        assert_eq!(
            runtime_invocation_handle_free(client_handle, invocation_handle_id),
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(client_handle);
    }

    #[test]
    fn invocation_handle_cancel_after_transport_failure_does_not_fake_terminal() {
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
                runtime_invocation_submit_signed_handle(
                    client_handle,
                    signed_id,
                    &mut invocation_handle_id,
                    &mut submitted_ptr,
                )
            },
            RUNTIME_OK
        );
        unsafe { crate::ffi::strings::runtime_string_free(submitted_ptr) };

        let mut result_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_await(
                    client_handle,
                    invocation_handle_id,
                    &mut result_ptr,
                )
            },
            ERR_DAEMON_DOWN
        );
        assert!(result_ptr.is_null());

        let reason = CString::new(serde_json::json!({"reason": "too-late"}).to_string()).unwrap();
        let mut cancel_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_cancel(
                    client_handle,
                    invocation_handle_id,
                    reason.as_ptr(),
                    &mut cancel_ptr,
                )
            },
            RUNTIME_OK
        );
        let cancel_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(cancel_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(cancel_ptr) };
        assert_eq!(cancel_json["request_accepted"], false);
        assert_eq!(cancel_json["deduplicated"], true);
        assert_eq!(cancel_json["cancelled"], false);
        assert_eq!(cancel_json["state"], "Submitted");
        assert_eq!(cancel_json["terminal"], false);
        assert!(cancel_json["rejection"]
            .as_str()
            .expect("cancel rejection")
            .contains("resolve session owner for cancellation authority"));

        let mut events_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_events(
                    client_handle,
                    invocation_handle_id,
                    &mut events_ptr,
                )
            },
            RUNTIME_OK
        );
        let events_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(events_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(events_ptr) };
        assert_eq!(events_json["terminal"], false);
        assert!(events_json["result"].is_null());
        assert_eq!(
            events_json["cancellation_authority"]["state"],
            "unavailable"
        );
        assert_eq!(events_json["events"][2]["kind"], "cancel_unavailable");
        assert_eq!(
            runtime_invocation_handle_free(client_handle, invocation_handle_id),
            RUNTIME_OK
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
                runtime_invocation_submit_signed_handle(
                    owner_handle,
                    signed_id,
                    &mut invocation_handle_id,
                    &mut submitted_ptr,
                )
            },
            RUNTIME_OK
        );
        unsafe { crate::ffi::strings::runtime_string_free(submitted_ptr) };

        let mut events_ptr: *mut c_char = std::ptr::dangling_mut();
        let events_code = unsafe {
            runtime_invocation_handle_events(other_handle, invocation_handle_id, &mut events_ptr)
        };
        assert_eq!(events_code, ERR_INVALID_HANDLE);
        assert!(events_ptr.is_null());
        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(owner_handle);
        crate::ffi::client::handle::release(other_handle);
    }

    #[test]
    fn invocation_handle_ids_are_opaque_provider_tokens() {
        let (owner_handle, owner_session) = alloc(test_session());
        let owner = owner_session.binding(owner_handle);
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let (first, _) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json.clone(),
            test_cancellation_gate("easynet:///r/acme/device/dev-a"),
        );
        let (second, _) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json,
            test_cancellation_gate("easynet:///r/acme/device/dev-a"),
        );

        let first_id = insert_invocation_handle(first);
        let second_id = insert_invocation_handle(second);

        assert_ne!(first_id, 0);
        assert_ne!(second_id, 0);
        assert_ne!(first_id, second_id);
        assert_ne!(first_id, 1);
        assert_ne!(second_id, 2);
        assert!(first_id >= (1_u64 << 52));
        assert!(second_id >= (1_u64 << 52));
        assert!(first_id <= ((1_u64 << 53) - 1));
        assert!(second_id <= ((1_u64 << 53) - 1));
        assert!(get_invocation_handle_for_owner(owner, first_id)
            .unwrap()
            .is_some());
        assert!(get_invocation_handle_for_owner(owner, second_id)
            .unwrap()
            .is_some());

        assert!(remove_invocation_handle_for_owner(owner, first_id)
            .unwrap()
            .is_some());
        assert!(remove_invocation_handle_for_owner(owner, second_id)
            .unwrap()
            .is_some());
        crate::ffi::client::handle::release(owner_handle);
    }

    #[test]
    fn invocation_handle_rejects_stale_session_incarnation() {
        let owner = registry_owner(77, 7700);
        let stale_same_handle = registry_owner(77, 8800);
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let (active, _) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json,
            test_cancellation_gate("easynet:///r/acme/device/dev-a"),
        );
        let invocation_handle_id = insert_invocation_handle(active);

        assert!(matches!(
            get_invocation_handle_for_owner(stale_same_handle, invocation_handle_id),
            Err(RegistryOwnerMismatch)
        ));
        assert!(matches!(
            remove_invocation_handle_for_owner(stale_same_handle, invocation_handle_id),
            Err(RegistryOwnerMismatch)
        ));
        assert!(
            remove_invocation_handle_for_owner(owner, invocation_handle_id)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn invocation_handle_rejects_post_free_replay() {
        let (owner_handle, owner_session) = alloc(test_session());
        let owner = owner_session.binding(owner_handle);
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let (active, _) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json,
            test_cancellation_gate("easynet:///r/acme/device/dev-a"),
        );
        let invocation_handle_id = insert_invocation_handle(active);

        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            RUNTIME_OK
        );

        let mut result_ptr: *mut c_char = std::ptr::dangling_mut();
        let await_code = unsafe {
            runtime_invocation_handle_await(owner_handle, invocation_handle_id, &mut result_ptr)
        };
        assert_eq!(await_code, ERR_INVALID_HANDLE);
        assert!(result_ptr.is_null());

        let reason = CString::new(serde_json::json!({"reason": "after-free"}).to_string()).unwrap();
        let mut cancel_ptr: *mut c_char = std::ptr::dangling_mut();
        let cancel_code = unsafe {
            runtime_invocation_handle_cancel(
                owner_handle,
                invocation_handle_id,
                reason.as_ptr(),
                &mut cancel_ptr,
            )
        };
        assert_eq!(cancel_code, ERR_INVALID_HANDLE);
        assert!(cancel_ptr.is_null());

        let mut events_ptr: *mut c_char = std::ptr::dangling_mut();
        let events_code = unsafe {
            runtime_invocation_handle_events(owner_handle, invocation_handle_id, &mut events_ptr)
        };
        assert_eq!(events_code, ERR_INVALID_HANDLE);
        assert!(events_ptr.is_null());

        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            ERR_INVALID_HANDLE
        );
        crate::ffi::client::handle::release(owner_handle);
    }

    #[test]
    fn invocation_handle_provider_authority_conformance() {
        let owner = registry_owner(515, 1);
        let stale_same_handle = registry_owner(515, 2);
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let (active, _) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json,
            test_cancellation_gate("easynet:///r/acme/device/dev-a"),
        );
        let stale_id = insert_invocation_handle(active);
        assert!(matches!(
            get_invocation_handle_for_owner(stale_same_handle, stale_id),
            Err(RegistryOwnerMismatch)
        ));
        assert!(remove_invocation_handle_for_owner(owner, stale_id)
            .unwrap()
            .is_some());

        let runtime_directory =
            tempfile::tempdir().expect("isolated provider-authority runtime directory");
        let missing_control = runtime_directory
            .path()
            .join(crate::daemon::control::discovery::CONTROL_JSON_FILENAME);
        let missing_daemon = runtime_directory.path().join("missing-daemon.sock");
        let (owner_handle, owner_session) = alloc(
            crate::ffi::client::handle::ClientSession::with_control_path_only(
                missing_control.display().to_string(),
                Some(missing_daemon.display().to_string()),
            ),
        );
        let (other_handle, _) = alloc(test_session());
        let signed_id = new_signed_invocation_id();
        let mut invocation_handle_id: InvocationHandleId = 0;
        let mut submitted_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_submit_signed_handle(
                    owner_handle,
                    signed_id,
                    &mut invocation_handle_id,
                    &mut submitted_ptr,
                )
            },
            RUNTIME_OK
        );
        assert_ne!(invocation_handle_id, 0);
        unsafe { crate::ffi::strings::runtime_string_free(submitted_ptr) };

        let reason =
            CString::new(serde_json::json!({"reason": "conformance"}).to_string()).unwrap();

        let mut other_await_ptr: *mut c_char = std::ptr::dangling_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_await(
                    other_handle,
                    invocation_handle_id,
                    &mut other_await_ptr,
                )
            },
            ERR_INVALID_HANDLE
        );
        assert!(other_await_ptr.is_null());

        let mut other_cancel_ptr: *mut c_char = std::ptr::dangling_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_cancel(
                    other_handle,
                    invocation_handle_id,
                    reason.as_ptr(),
                    &mut other_cancel_ptr,
                )
            },
            ERR_INVALID_HANDLE
        );
        assert!(other_cancel_ptr.is_null());

        let mut other_events_ptr: *mut c_char = std::ptr::dangling_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_events(
                    other_handle,
                    invocation_handle_id,
                    &mut other_events_ptr,
                )
            },
            ERR_INVALID_HANDLE
        );
        assert!(other_events_ptr.is_null());
        assert_eq!(
            runtime_invocation_handle_free(other_handle, invocation_handle_id),
            ERR_INVALID_HANDLE
        );

        let mut await_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_await(owner_handle, invocation_handle_id, &mut await_ptr)
            },
            ERR_DAEMON_DOWN
        );
        assert!(await_ptr.is_null());

        let mut cancel_ptr: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_cancel(
                    owner_handle,
                    invocation_handle_id,
                    reason.as_ptr(),
                    &mut cancel_ptr,
                )
            },
            RUNTIME_OK
        );
        let cancel_json: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(cancel_ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::runtime_string_free(cancel_ptr) };
        assert_eq!(cancel_json["request_accepted"], false);
        assert_eq!(cancel_json["deduplicated"], true);
        assert_eq!(cancel_json["cancelled"], false);
        assert_eq!(cancel_json["state"], "Submitted");
        assert_eq!(cancel_json["terminal"], false);
        assert!(cancel_json["rejection"]
            .as_str()
            .expect("cancel rejection")
            .contains("resolve session owner for cancellation authority"));

        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            RUNTIME_OK
        );
        let mut post_free_await_ptr: *mut c_char = std::ptr::dangling_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_await(
                    owner_handle,
                    invocation_handle_id,
                    &mut post_free_await_ptr,
                )
            },
            ERR_INVALID_HANDLE
        );
        assert!(post_free_await_ptr.is_null());

        let mut post_free_cancel_ptr: *mut c_char = std::ptr::dangling_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_cancel(
                    owner_handle,
                    invocation_handle_id,
                    reason.as_ptr(),
                    &mut post_free_cancel_ptr,
                )
            },
            ERR_INVALID_HANDLE
        );
        assert!(post_free_cancel_ptr.is_null());

        let mut post_free_events_ptr: *mut c_char = std::ptr::dangling_mut();
        assert_eq!(
            unsafe {
                runtime_invocation_handle_events(
                    owner_handle,
                    invocation_handle_id,
                    &mut post_free_events_ptr,
                )
            },
            ERR_INVALID_HANDLE
        );
        assert!(post_free_events_ptr.is_null());
        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            ERR_INVALID_HANDLE
        );

        assert_eq!(
            owner_session.begin_closing(owner_handle).unwrap().handle,
            owner_handle
        );
        assert!(owner_session
            .resource_registration_guard(owner_handle)
            .is_err());
        owner_session.mark_released();
        crate::ffi::client::handle::release(owner_handle);
        crate::ffi::client::handle::release(other_handle);
    }

    #[test]
    fn receipt_free_admission_rejection_is_a_terminal_handle_event() {
        use axon_sdk::pb::axon::v1::{Error, ErrorStage, InvokeResponse};

        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let shared = InvocationHandleShared::new(tuple_json);
        let outcome = crate::daemon::InvocationOutcome::from_invoke_response(
            signed_fixture_tuple(),
            InvokeResponse {
                state: axon_sdk::invocation::InvocationState::Failed.to_wire_i32(),
                error: Some(Error {
                    code: "CALLER_SIGNATURE_INVALID".to_string(),
                    message: "ed25519_signature_wrong_length".to_string(),
                    stage: ErrorStage::CallerAuthentication as i32,
                    ..Error::default()
                }),
                ..InvokeResponse::default()
            },
            &crate::support::platform::local_daemon_grpc::LocalKeyServiceReceiptResolver::new(),
        )
        .expect("receipt-free pre-admission rejection");

        assert!(shared.observe_canonical_outcome(outcome).unwrap());
        let snapshot = shared
            .snapshot_json(
                41,
                &test_cancellation_gate("easynet:///r/acme/device/dev-a"),
            )
            .unwrap();
        assert_eq!(snapshot["terminal"], true);
        assert_eq!(snapshot["state"], "Failed");
        assert_eq!(snapshot["events"][1]["kind"], "failed");
        assert_eq!(snapshot["events"][1]["terminal"], true);
        assert_eq!(
            snapshot["result"]["error"]["stage"],
            "caller_authentication"
        );
        assert!(snapshot["result"]["admission_receipt"].is_null());
        assert!(snapshot["result"]["terminal_receipt"].is_null());
    }

    #[test]
    fn invocation_handle_cancel_is_request_then_canonical_terminal() {
        let (owner_handle, owner_session) = alloc(test_session());
        let owner = owner_session.binding(owner_handle);
        let (tuple, response, resolver) =
            canonical_finalized_response(axon_sdk::invocation::InvocationState::Cancelled);
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let (active, _cancel_requests) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json,
            test_cancellation_gate("easynet:///r/acme/device/dev-a"),
        );
        let shared = active.shared.clone();
        let invocation_handle_id = insert_invocation_handle(active);
        let handle = get_invocation_handle_for_owner(owner, invocation_handle_id)
            .unwrap()
            .unwrap();

        let first = handle.cancel(Some("client stop".to_string()));
        assert!(first.request_accepted);
        assert!(!first.deduplicated);
        assert!(first.dispatch_request);
        assert!(!first.cancelled);
        assert!(!first.terminal);
        assert_eq!(first.state, InvocationHandlePhase::CancelRequested);

        let duplicate = handle.cancel(Some("client stop again".to_string()));
        assert!(duplicate.request_accepted);
        assert!(duplicate.deduplicated);
        assert!(!duplicate.dispatch_request);
        assert!(!duplicate.cancelled);
        assert!(!duplicate.terminal);

        let (command_tuple, command_response, command_resolver) =
            canonical_finalized_response(axon_sdk::invocation::InvocationState::Completed);
        let command_outcome = crate::daemon::InvocationOutcome::from_invoke_response(
            command_tuple,
            command_response,
            command_resolver.as_ref(),
        )
        .expect("cancel command completion must carry canonical finalization");
        shared
            .observe_cancel_command_outcome(command_outcome)
            .expect("cancel command has canonical completion");
        assert_eq!(
            handle
                .shared
                .snapshot_json(
                    invocation_handle_id,
                    &test_cancellation_gate("easynet:///r/acme/device/dev-a"),
                )
                .unwrap()["terminal"],
            false
        );
        assert_eq!(
            handle
                .shared
                .snapshot_json(
                    invocation_handle_id,
                    &test_cancellation_gate("easynet:///r/acme/device/dev-a"),
                )
                .unwrap()["state"],
            "CancelRequested"
        );

        let canonical = crate::daemon::InvocationOutcome::from_invoke_response(
            tuple,
            response,
            resolver.as_ref(),
        )
        .expect("cancelled target must carry canonical finalization");
        assert!(shared.observe_canonical_outcome(canonical).unwrap());
        let result = handle.await_result();
        assert_eq!(result.terminal_state, "Cancelled");
        assert!(result.receipt.is_some());

        let after_terminal = handle.cancel(Some("too late".to_string()));
        assert!(!after_terminal.request_accepted);
        assert!(after_terminal.deduplicated);
        assert!(after_terminal.cancelled);
        assert!(after_terminal.terminal);

        let events_json = handle.events_json(invocation_handle_id).unwrap();
        assert_eq!(events_json["events"].as_array().unwrap().len(), 4);
        assert_eq!(events_json["events"][1]["state"], "CancelRequested");
        assert_eq!(events_json["events"][1]["terminal"], false);
        assert_eq!(events_json["events"][2]["kind"], "cancel_command_completed");
        assert_eq!(events_json["events"][2]["terminal"], false);
        assert_eq!(events_json["events"][3]["state"], "Cancelled");
        assert_eq!(events_json["events"][3]["terminal"], true);
        assert!(events_json["result"]["terminal_receipt"].is_object());
        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(owner_handle);
    }

    #[test]
    fn invocation_handle_cancel_unavailable_is_explicit_non_terminal_state() {
        let (owner_handle, owner_session) = alloc(test_session());
        let owner = owner_session.binding(owner_handle);
        let tuple_json: serde_json::Value =
            serde_json::from_str(&canonical_invocation_json(serde_json::json!({}))).unwrap();
        let (active, mut cancel_requests) = ActiveInvocationHandle::with_cancel_channel(
            owner,
            tuple_json,
            unavailable_cancellation_gate("session owner unavailable"),
        );
        let invocation_handle_id = insert_invocation_handle(active);
        let handle = get_invocation_handle_for_owner(owner, invocation_handle_id)
            .unwrap()
            .unwrap();

        let outcome = handle.cancel(Some("client stop".to_string()));
        assert!(!outcome.request_accepted);
        assert!(outcome.deduplicated);
        assert!(!outcome.dispatch_request);
        assert!(!outcome.cancelled);
        assert!(!outcome.terminal);
        assert_eq!(outcome.state, InvocationHandlePhase::Submitted);
        assert_eq!(
            outcome.rejection.as_deref(),
            Some("session owner unavailable")
        );
        assert!(cancel_requests.try_recv().is_err());

        let duplicate = handle.cancel(Some("client stop again".to_string()));
        assert!(!duplicate.request_accepted);
        assert!(duplicate.deduplicated);
        assert!(!duplicate.dispatch_request);
        assert!(!duplicate.terminal);
        assert_eq!(
            duplicate.rejection.as_deref(),
            Some("session owner unavailable")
        );
        assert!(cancel_requests.try_recv().is_err());

        let events_json = handle.events_json(invocation_handle_id).unwrap();
        assert_eq!(events_json["state"], "Submitted");
        assert_eq!(events_json["terminal"], false);
        assert_eq!(
            events_json["cancellation_authority"]["state"],
            "unavailable"
        );
        assert_eq!(
            events_json["cancellation_authority"]["reason"],
            "session owner unavailable"
        );
        assert_eq!(events_json["events"][1]["kind"], "cancel_unavailable");
        assert_eq!(events_json["events"][1]["terminal"], false);
        assert_eq!(events_json["events"].as_array().unwrap().len(), 2);

        assert_eq!(
            runtime_invocation_handle_free(owner_handle, invocation_handle_id),
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(owner_handle);
    }

    #[test]
    fn parse_invocation_json_rejects_zero_nonce() {
        let err = InvocationJson::parse(
            r#"{
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": "easynet:///r/acme/device/dev-a",
                "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
                "descriptor_ref": "easynet:///r/acme/device/dev-a/ability/observe.health@2.4.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
        assert_eq!(spec.metadata["x-easynet-test-producer"], "producer");
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
    fn parse_invocation_json_rejects_caller_signature_without_key_hint() {
        let raw = canonical_invocation_json(serde_json::json!({
            "caller_signature": {
                "algorithm": "ed25519",
                "signature_base64": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==",
                "signer_public_key_base64": "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
            }
        }));

        let error = InvocationJson::parse(&raw)
            .expect_err("caller signature key identity must be explicit");

        assert_eq!(error.to_string(), "missing field `key_id_hint`");
    }

    #[test]
    fn parse_invocation_json_rejects_blank_caller_signature_key_hint() {
        let raw = canonical_invocation_json(serde_json::json!({
            "caller_signature": {
                "algorithm": "ed25519",
                "signature_base64": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==",
                "key_id_hint": "   ",
                "signer_public_key_base64": "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
            }
        }));

        let error = InvocationJson::parse(&raw)
            .expect_err("blank caller signature key identity must be rejected");

        assert_eq!(
            error.to_string(),
            "field `key_id_hint` must be a non-empty string"
        );
    }

    #[test]
    fn signature_json_rejects_missing_key_hint() {
        let error = SignatureMaterialJson::parse(
            &serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6eg==",
                "signer_public_key_base64": "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
            })
            .to_string(),
        )
        .expect_err("detached signature key identity must be explicit");

        assert_eq!(error.to_string(), "missing field `key_id_hint`");
    }

    #[test]
    fn signature_json_rejects_blank_key_hint() {
        let error = SignatureMaterialJson::parse(
            &serde_json::json!({
                "algorithm": "ed25519",
                "signature_base64": "enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6enp6eg==",
                "key_id_hint": "\t"
            })
            .to_string(),
        )
        .expect_err("blank detached signature key identity must be rejected");

        assert_eq!(
            error.to_string(),
            "field `key_id_hint` must be a non-empty string"
        );
    }

    #[test]
    fn parse_bidi_up_frame_json_supports_binary_chunk_and_controls() {
        use axon_sdk::pb::axon::v1::{bidi_control, invoke_bidi_up};

        let chunk = parse_bidi_up_frame_json(
            &serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8=",
                "pts": 9,
                "mac_base64": test_bidi_mac_base64()
            })
            .to_string(),
        )
        .unwrap();
        let invoke_bidi_up::Payload::BinaryChunk(chunk) = chunk.payload else {
            panic!("expected binary chunk");
        };
        assert_eq!(chunk.stream_id, 1);
        assert_eq!(chunk.data, b"hello");
        assert_eq!(chunk.pts, 9);

        let control = parse_bidi_up_frame_json(
            &serde_json::json!({
                "type": "control",
                "media_pts": {"stream_id": 2, "pts": 123},
                "mac_base64": test_bidi_mac_base64()
            })
            .to_string(),
        )
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
    fn parse_bidi_up_frame_json_rejects_missing_binary_chunk_pts() {
        for frame in [
            serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8=",
                "mac_base64": test_bidi_mac_base64()
            }),
            serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8=",
                "pts": null,
                "mac_base64": test_bidi_mac_base64()
            }),
        ] {
            let error = parse_bidi_up_frame_json(&frame.to_string())
                .expect_err("binary_chunk pts must be explicit at the public ABI boundary");

            assert_eq!(error.to_string(), "field `pts` must fit into u64");
        }
    }

    #[test]
    fn parse_bidi_up_frame_json_rejects_missing_or_noncanonical_mac() {
        let missing = parse_bidi_up_frame_json(
            r#"{"type":"binary_chunk","stream_id":1,"data_base64":"aGVsbG8="}"#,
        )
        .expect_err("missing frame-chain MAC must fail closed");
        assert_eq!(missing.to_string(), "missing field `mac_base64`");

        let short = parse_bidi_up_frame_json(
            &serde_json::json!({
                "type": "control",
                "eof": true,
                "mac_base64": "AQID"
            })
            .to_string(),
        )
        .expect_err("noncanonical MAC length must fail closed");
        assert_eq!(
            short.to_string(),
            "mac_base64 must decode to exactly 32 bytes, got 3"
        );
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
        assert_eq!(health["invocation_ready"], false);
        assert_eq!(health["runtime_ready"], false);
        assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(health["abi_version"], crate::ffi::RUNTIME_ABI_VERSION);
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

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_diagnostics_catalog_includes_device_meta_descriptor_ref() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let device_ura =
            crate::core::ura::device_ura("localhost", "386b1258-3c89-494a-90a2-2321c29bf992");
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some("386b1258-3c89-494a-90a2-2321c29bf992".to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("daemon.sock").display().to_string()),
        );

        let diagnostics = runtime_diagnostics_json(&session);
        let entries = diagnostics["descriptor_catalog"]["entries"]
            .as_array()
            .expect("descriptor catalog entries");
        let meta = entries
            .iter()
            .find(|entry| {
                entry["owner_ura"] == device_ura
                    && entry["name"]
                        == crate::daemon::ability::names::governance::META_LIST_ABILITIES
                    && entry["call_mode"] == "rpc"
            })
            .unwrap_or_else(|| panic!("device meta.list_abilities missing: {entries:?}"));

        let descriptor_ref = meta["descriptor_ref"].as_str().expect("descriptor_ref");
        assert!(descriptor_ref.starts_with(&format!(
            "{}/ability/device.386b1258-3c89-494a-90a2-2321c29bf992.meta.list_abilities@",
            "easynet:///r/localhost"
        )));
        assert!(descriptor_ref.ends_with("!read"));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_owner_resolution_rejects_relative_control_endpoint_before_cwd_lookup() {
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            "daemon.sock".to_string(),
            Some("/tmp/offline-daemon.sock".to_string()),
        );

        let error = runtime_owner_ura_from_session(&session)
            .expect_err("relative control endpoint must not resolve through cwd");

        assert!(
            error.contains("resolve control discovery path") && error.contains("must be absolute"),
            "unexpected runtime owner error: {error}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_prefers_local_catalog_for_runtime_owner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let device_ura = crate::core::ura::device_ura("localhost", node_id);
        let ability_ura = format!(
            "easynet:///r/localhost/ability/device.{node_id}.{}",
            crate::daemon::ability::names::resources::META_LIST_RESOURCES
        );
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let resolved = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": device_ura,
                "caller_ura": device_ura,
                "subject_ura": crate::core::ura::hub_ura("localhost"),
                "ability": ability_ura,
                "call_mode": "rpc",
                "provider": "ability_descriptor",
            })
            .to_string(),
        )
        .expect("local runtime owner catalogue descriptor resolves through explicit provider");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], device_ura);
        assert_eq!(
            resolved["name"],
            crate::daemon::ability::names::resources::META_LIST_RESOURCES
        );
        assert_eq!(resolved["call_mode"], "rpc");
        assert_eq!(resolved["source"], "runtime_local_descriptor_catalog");
        assert!(resolved["descriptor_ref"]
            .as_str()
            .is_some_and(
                |descriptor_ref| descriptor_ref.starts_with(&format!("{ability_ura}@"))
                    && descriptor_ref.ends_with("!read")
            ));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_requires_explicit_call_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let device_ura = crate::core::ura::device_ura("localhost", node_id);
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": device_ura,
                "caller_ura": device_ura,
                "subject_ura": device_ura,
                "ability": crate::daemon::ability::names::resources::META_LIST_RESOURCES,
            })
            .to_string(),
        )
        .expect_err("descriptor resolver must reject missing call_mode");

        assert!(
            error
                .to_string()
                .contains("descriptor_ref request missing call_mode"),
            "unexpected error: {error:#}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_rejects_ability_owner_mismatch_before_catalog_lookup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_control_path = dir.path().join("missing-control.json");
        let callee_ura = crate::core::ura::device_ura("localhost", "callee-device");
        let other_ura = crate::core::ura::device_ura("localhost", "other-device");
        let other_ability = crate::core::ura::owner_ability_ura(&other_ura, "meta.list_abilities")
            .expect("other ability URA");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            missing_control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": callee_ura,
                "caller_ura": callee_ura,
                "subject_ura": callee_ura,
                "ability": other_ability,
                "call_mode": "rpc",
            })
            .to_string(),
        )
        .expect_err("ability owner mismatch must fail before runtime owner lookup");

        let message = error.to_string();
        assert!(
            message.contains("does not match callee"),
            "unexpected descriptor resolver error: {message}"
        );
        assert!(
            !message.contains("resolve descriptor_ref runtime owner")
                && !message.contains("descriptor_ref not found"),
            "owner mismatch must not be reclassified through runtime owner or catalog lookup: {message}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_does_not_remote_probe_local_catalog_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let device_ura = crate::core::ura::device_ura("localhost", node_id);
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": device_ura,
                "caller_ura": device_ura,
                "subject_ura": device_ura,
                "ability": format!(
                    "easynet:///r/localhost/ability/device.{node_id}.missing.local.catalog"
                ),
                "call_mode": "rpc",
            })
            .to_string(),
        )
        .expect_err("local runtime owner descriptor miss must not remote probe");

        let message = error.to_string();
        assert!(message.contains("descriptor_ref not found in local runtime catalog"));
        assert!(!message.contains("offline-daemon.sock"));
        assert!(!message.contains("ROUTE_NEGATIVE"));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn descriptor_catalog_resolution_rejects_matching_row_without_descriptor_ref() {
        let ability_ura = "easynet:///r/localhost/ability/device.dev-a.observe.health";
        let entries = vec![serde_json::json!({
            "ability_ura": ability_ura,
            "owner_ura": "easynet:///r/localhost/device/dev-a",
            "name": "observe.health",
            "call_mode": "rpc"
        })];

        let error = RuntimeDescriptorResolutionProvider::resolve_catalog_entries_for_test(
            &entries,
            ability_ura,
            "rpc",
            "test_descriptor_catalog",
        )
        .expect_err("matching descriptor catalog rows must be schema-complete");

        assert!(
            error.to_string().contains("missing descriptor_ref"),
            "unexpected descriptor catalog error: {error}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn descriptor_catalog_dedupe_rejects_schema_incomplete_rows() {
        let entries = vec![serde_json::json!({
            "ability_ura": "easynet:///r/localhost/ability/device.dev-a.observe.health",
            "owner_ura": "easynet:///r/localhost/device/dev-a",
            "call_mode": "rpc"
        })];

        let error = RuntimeDescriptorResolutionProvider::dedupe_catalog_entries_for_test(entries)
            .expect_err("dedupe must not silently drop schema-incomplete descriptor rows");

        assert!(
            error.contains("missing descriptor_ref before dedupe"),
            "unexpected descriptor catalog dedupe error: {error}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_does_not_remote_probe_realm_catalog_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let local_node_id = "local-runtime-node";
        let remote_node_id = "remote-runtime-node";
        let local_device_ura = crate::core::ura::device_ura("localhost", local_node_id);
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("offline-daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(local_node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": "easynet:///r/localhost/user/missing-descriptor-probe-signer",
                "subject_ura": remote_device_ura,
                "ability": format!(
                    "easynet:///r/localhost/ability/device.{remote_node_id}.custom.not.system"
                ),
                "call_mode": "rpc",
            })
            .to_string(),
        )
        .expect_err("realm catalog miss must not fall back to a remote descriptor probe");

        let message = error.to_string();
        assert!(
            message.contains("descriptor_ref not found in runtime realm catalog"),
            "unexpected descriptor resolver error: {message}"
        );
        assert!(
            !message.contains("requires a caller signer")
                && !message.contains("prepare remote descriptor catalog probe signer")
                && !message.contains("ROUTE_NEGATIVE")
                && !message.contains("owner is not online"),
            "catalog miss must not be reclassified through remote probe state: {message}"
        );
        assert!(
            !message.contains(&local_device_ura),
            "resolver must not synthesize the local runtime owner as remote caller: {message}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_requires_runtime_owner_for_realm_catalog() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_control_path = dir.path().join("missing-control.json");
        let remote_node_id = "remote-runtime-node";
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        let caller_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            missing_control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": caller_ura,
                "subject_ura": remote_device_ura,
                "ability": format!(
                    "easynet:///r/localhost/ability/device.{remote_node_id}.custom.not.system"
                ),
                "call_mode": "rpc",
            })
            .to_string(),
        )
        .expect_err(
            "descriptor resolver must fail before realm catalog lookup without runtime owner",
        );

        let message = error.to_string();
        assert!(
            message.contains(CALLER_SIGNER_UNAVAILABLE_CODE)
                && message.contains("descriptor resolution requires a caller signer"),
            "unexpected descriptor resolver error: {message}"
        );
        assert!(
            !message.contains("resolve descriptor_ref runtime owner")
                && !message.contains("control discovery")
                && !message.contains("keyring")
                && !message.contains("self-identity"),
            "runtime owner failure must not expose custody implementation details: {message}"
        );
        assert!(
            !message.contains("offline-daemon.sock"),
            "runtime owner failure must happen before daemon IO: {message}"
        );
    }

    #[cfg(feature = "axon-pb")]
    fn runtime_descriptor_resolution_missing_owner_error_message() -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_control_path = dir.path().join("missing-control.json");
        let remote_device_ura = crate::core::ura::device_ura("localhost", "remote-runtime-node");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            missing_control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );
        runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": "easynet:///r/localhost/device/local-runtime-node",
                "subject_ura": remote_device_ura,
                "ability": "custom.not.system",
                "call_mode": "rpc",
            })
            .to_string(),
        )
        .expect_err("missing runtime owner must fail closed")
        .to_string()
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn descriptor_resolution_errors_project_canonical_runtime_codes() {
        let not_found = DescriptorResolutionError::DescriptorNotFound(
            "descriptor_ref not found in local runtime catalog".to_string(),
        );
        let (abi_code, projection) = descriptor_resolution_abi_projection(&not_found);
        assert_eq!(abi_code, ERR_NOT_FOUND);
        assert_eq!(projection.code, "DESCRIPTOR_NOT_FOUND");
        assert_eq!(projection.stage, "routing");

        let runtime_owner_unavailable = DescriptorResolutionError::RuntimeOwnerUnavailable(
            format!("{CALLER_SIGNER_UNAVAILABLE_CODE}: descriptor resolution requires a caller signer; load or provision that identity in the local key service"),
        );
        let (abi_code, projection) =
            descriptor_resolution_abi_projection(&runtime_owner_unavailable);
        assert_eq!(abi_code, ERR_PERMISSION_DENIED);
        assert_eq!(projection.code, CALLER_SIGNER_UNAVAILABLE_CODE);
        assert_eq!(projection.stage, "caller_identity");

        let message = runtime_descriptor_resolution_missing_owner_error_message();
        assert!(message.contains(CALLER_SIGNER_UNAVAILABLE_CODE));
        assert!(
            !message.contains("resolve descriptor_ref runtime owner")
                && !message.contains("keyring entry not found"),
            "descriptor resolver must not expose signer custody internals: {message}"
        );

        let invalid_catalog_payload = DescriptorResolutionError::InvalidCatalogPayload(
            "descriptor catalog row for ability \"easynet:///r/localhost/ability/device.dev-a.observe.health\" from runtime_local_descriptor_catalog missing descriptor_ref".to_string(),
        );
        let (abi_code, projection) = descriptor_resolution_abi_projection(&invalid_catalog_payload);
        assert_eq!(abi_code, ERR_INVALID_ARG);
        assert_eq!(projection.code, "INVALID_ARGUMENT");
        assert_eq!(projection.stage, "provider_payload");
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_rejects_generic_remote_governance_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let local_node_id = "local-runtime-node";
        let remote_node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let local_device_ura = crate::core::ura::device_ura("localhost", local_node_id);
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(local_node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": local_device_ura,
                "subject_ura": remote_device_ura,
                "ability": crate::daemon::ability::builtins::governance::invocation_history::ABILITY_HISTORY_LIST,
                "call_mode": "rpc",
            })
            .to_string(),
        )
        .expect_err("governance read descriptors must declare the canonical provider");

        let message = error.to_string();
        assert!(
            message.contains("generic provider cannot resolve receipt history read ability")
                && message.contains("provider \"receipt_history\""),
            "generic governance descriptor resolution must fail at provider boundary, got: {message}"
        );
        assert!(
            !message.contains("descriptor_ref not found")
                && !message.contains("meta.list_abilities")
                && !message.contains("requires a caller signer")
                && !message.contains("ROUTE_NEGATIVE"),
            "generic governance read must not fall through to catalog/route/signer state: {message}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_uses_explicit_provider_for_remote_catalogue_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let local_node_id = "local-runtime-node";
        let remote_node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let local_device_ura = crate::core::ura::device_ura("localhost", local_node_id);
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        let authority_subject = crate::core::ura::hub_ura("localhost");
        let ability_ura = format!(
            "easynet:///r/localhost/ability/device.{remote_node_id}.{}",
            crate::daemon::ability::names::governance::META_LIST_ABILITIES
        );
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(local_node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let resolved = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": local_device_ura,
                "subject_ura": authority_subject,
                "ability": crate::daemon::ability::names::governance::META_LIST_ABILITIES,
                "call_mode": "rpc",
                "provider": "ability_descriptor",
            })
            .to_string(),
        )
        .expect("explicit ability descriptor provider resolves remote catalogue descriptor");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], remote_device_ura);
        assert_eq!(
            resolved["name"],
            crate::daemon::ability::names::governance::META_LIST_ABILITIES
        );
        assert_eq!(resolved["source"], "runtime_ability_descriptor_provider");
        assert!(resolved["descriptor_ref"]
            .as_str()
            .is_some_and(
                |descriptor_ref| descriptor_ref.starts_with(&format!("{ability_ura}@"))
                    && descriptor_ref.ends_with("!read")
            ));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_rejects_ability_descriptor_non_authority_subjects() {
        let session = test_session();
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let remote_device_ura =
            crate::core::ura::device_ura("localhost", "a364ba18-8961-4b31-838a-31c7d776c709");
        let cases = [
            (
                Some(remote_device_ura.as_str()),
                "descriptor_ref provider ability_descriptor subject_ura must be an Authority URA",
            ),
            (
                Some("easynet:///r/other/authority"),
                "descriptor_ref provider ability_descriptor subject_ura must be the callee realm authority subject",
            ),
            (
                None,
                "descriptor_ref provider ability_descriptor requires subject_ura",
            ),
        ];

        for (subject_ura, expected) in cases {
            let mut request = serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": local_device_ura,
                "ability": crate::daemon::ability::names::governance::META_LIST_ABILITIES,
                "call_mode": "rpc",
                "provider": "ability_descriptor",
            });
            if let Some(subject_ura) = subject_ura {
                request["subject_ura"] = serde_json::Value::String(subject_ura.to_string());
            }

            let error = runtime_resolve_descriptor_ref_json(&session, &request.to_string())
                .expect_err("invalid ability descriptor catalogue subject must fail closed");
            let message = error.to_string();
            assert!(
                message.contains(expected),
                "unexpected ability descriptor subject error: {message}"
            );
            assert!(
                !message.contains("ROUTE_NEGATIVE")
                    && !message.contains("owner is not online")
                    && !message.contains("requires a caller signer"),
                "subject validation must fail before route/signer resolution: {message}"
            );
        }
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_uses_explicit_provider_for_remote_receipt_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let local_node_id = "local-runtime-node";
        let remote_node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let local_device_ura = crate::core::ura::device_ura("localhost", local_node_id);
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        let runtime_state_subject = "easynet:///r/localhost/resource/user.alice/runtime-state/read";
        let ability_ura = format!(
            "easynet:///r/localhost/ability/device.{remote_node_id}.{}",
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST
        );
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(local_node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let resolved = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": local_device_ura,
                "subject_ura": runtime_state_subject,
                "ability": crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
                "call_mode": "rpc",
                "provider": "receipt_history",
            })
            .to_string(),
        )
        .expect("explicit receipt provider resolves remote history descriptor");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], remote_device_ura);
        assert_eq!(
            resolved["name"],
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST
        );
        assert_eq!(resolved["source"], "runtime_receipt_provider");
        assert!(resolved["descriptor_ref"]
            .as_str()
            .is_some_and(
                |descriptor_ref| descriptor_ref.starts_with(&format!("{ability_ura}@"))
                    && descriptor_ref.ends_with("!read")
            ));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_uses_authority_subject_for_hub_receipt_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let authority_ura = crate::core::ura::hub_ura("localhost");
        let ability_ura = "easynet:///r/localhost/ability/authority.invocation.history.list";
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "hub".to_string(),
                    realm: "localhost".to_string(),
                    node_id: None,
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let resolved = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": authority_ura,
                "caller_ura": crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                "subject_ura": authority_ura,
                "ability": crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
                "call_mode": "rpc",
                "provider": "receipt_history",
            })
            .to_string(),
        )
        .expect("hub Authority receipt provider resolves local history descriptor");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], authority_ura);
        assert_eq!(
            resolved["name"],
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST
        );
        assert_eq!(resolved["source"], "runtime_local_descriptor_catalog");
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_rejects_receipt_provider_non_runtime_state_subjects() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let local_node_id = "local-runtime-node";
        let remote_node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let local_device_ura = crate::core::ura::device_ura("localhost", local_node_id);
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(local_node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        for (case, subject_ura, expected) in [
            (
                "missing subject",
                None,
                "provider receipt_history requires subject_ura",
            ),
            (
                "wrong device subject",
                Some(local_device_ura.as_str()),
                "user-owned runtime-state read subject or the callee runtime-owner subject",
            ),
            (
                "noncanonical session subject",
                Some("easynet:///r/localhost/resource/user.alice/session/invocation_history"),
                "user-owned runtime-state read subject or the callee runtime-owner subject",
            ),
            (
                "all-zero runtime-state subject",
                Some("easynet:///r/localhost/resource/user.00000000-0000-0000-0000-000000000000/runtime-state/read"),
                "subject_ura must not be all-zero",
            ),
        ] {
            let mut request = serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": local_device_ura,
                "ability": crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
                "call_mode": "rpc",
                "provider": "receipt_history",
            });
            if let Some(subject_ura) = subject_ura {
                request
                    .as_object_mut()
                    .expect("descriptor request object")
                    .insert("subject_ura".to_string(), serde_json::json!(subject_ura));
            }
            let error = runtime_resolve_descriptor_ref_json(&session, &request.to_string())
                .expect_err(&format!("{case} must be rejected"));
            let message = error.to_string();
            assert!(
                message.contains(expected),
                "{case} error must contain {expected:?}, got {message}"
            );
            assert!(
                !message.contains("descriptor_ref not found") && !message.contains("ROUTE_NEGATIVE"),
                "{case} must fail before catalog or route state: {message}"
            );
        }
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_descriptor_resolver_rejects_provider_ability_family_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let control_path = dir.path().join("control.json");
        let local_node_id = "local-runtime-node";
        let remote_node_id = "a364ba18-8961-4b31-838a-31c7d776c709";
        let local_device_ura = crate::core::ura::device_ura("localhost", local_node_id);
        let remote_device_ura = crate::core::ura::device_ura("localhost", remote_node_id);
        crate::daemon::control::discovery::write(
            &control_path,
            &crate::daemon::control::discovery::ControlDiscovery {
                socket_path: Some(dir.path().join("control.sock")),
                pipe_name: None,
                invocation_endpoint: Some(dir.path().join("daemon.sock")),
                daemon_identity: Some(crate::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "localhost".to_string(),
                    node_id: Some(local_node_id.to_string()),
                }),
                pid: 1,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions: crate::daemon::control::discovery::IpcVersionRange::single(
                    1,
                ),
                capability_flags: Vec::new(),
                pages_port: None,
            },
        )
        .expect("write control discovery");
        let session = crate::ffi::client::handle::ClientSession::with_control_path_only(
            control_path.display().to_string(),
            Some(dir.path().join("offline-daemon.sock").display().to_string()),
        );

        let error = runtime_resolve_descriptor_ref_json(
            &session,
            &serde_json::json!({
                "callee_ura": remote_device_ura,
                "caller_ura": local_device_ura,
                "subject_ura": remote_device_ura,
                "ability": crate::daemon::ability::names::governance::META_LIST_ABILITIES,
                "call_mode": "rpc",
                "provider": "receipt_history",
            })
            .to_string(),
        )
        .expect_err("receipt provider must not resolve catalogue ability");

        let message = error.to_string();
        assert!(
            message.contains("provider receipt_history cannot resolve non-receipt ability"),
            "unexpected provider mismatch error: {message}"
        );
        assert!(
            !message.contains("descriptor_ref not found") && !message.contains("ROUTE_NEGATIVE"),
            "provider family mismatch must fail before catalog or route state: {message}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_system_descriptor_catalog_includes_authority_daemon_invocation_contracts() {
        let authority = crate::core::ura::hub_ura("localhost");
        let entries =
            RuntimeDescriptorResolutionProvider::system_catalog_entries_for_test(&authority)
                .expect("Authority system descriptor catalog");
        let principal_create = entries
            .iter()
            .find(|entry| entry["name"] == "principal.lifecycle.create")
            .unwrap_or_else(|| {
                panic!(
                    "principal.lifecycle.create missing from Authority descriptor catalog: {entries:?}"
                )
            });

        assert_eq!(principal_create["owner_ura"], authority);
        assert_eq!(principal_create["call_mode"], "rpc");
        assert_eq!(principal_create["admission_action"], "invoke");
        assert!(principal_create["descriptor_ref"]
            .as_str()
            .is_some_and(|descriptor_ref| descriptor_ref.starts_with(&format!(
                "{}/ability/authority.principal.lifecycle.create@",
                "easynet:///r/localhost"
            )) && descriptor_ref.ends_with("!invoke")));

        let runtime_bootstrap = entries
            .iter()
            .find(|entry| entry["name"] == "runtime.bootstrap_self_identity")
            .unwrap_or_else(|| {
                panic!(
                    "runtime.bootstrap_self_identity missing from Authority descriptor catalog: {entries:?}"
                )
            });
        assert_eq!(runtime_bootstrap["owner_ura"], authority);
        assert_eq!(runtime_bootstrap["call_mode"], "rpc");
        assert_eq!(runtime_bootstrap["admission_action"], "manage");
        assert!(runtime_bootstrap["descriptor_ref"]
            .as_str()
            .is_some_and(|descriptor_ref| descriptor_ref.starts_with(&format!(
                "{}/ability/authority.runtime.bootstrap_self_identity@",
                "easynet:///r/localhost"
            )) && descriptor_ref.ends_with("!manage")));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn runtime_system_descriptor_catalog_keeps_user_files_out_of_system_plane() {
        let device = crate::core::ura::device_ura("localhost", "host-a");
        let entries = RuntimeDescriptorResolutionProvider::system_catalog_entries_for_test(&device)
            .expect("Device system descriptor catalog");
        let names = entries
            .iter()
            .filter_map(|entry| entry.get("name").and_then(serde_json::Value::as_str))
            .collect::<std::collections::BTreeSet<_>>();

        assert!(names.contains("fs.read"), "{names:?}");
        assert!(names.contains("context.fs.list"), "{names:?}");
        assert!(names.contains("skill.list"), "{names:?}");
        assert!(!names.contains("files.put"), "{names:?}");
        assert!(!names.contains("files.get"), "{names:?}");
        assert!(!names.contains("files.list"), "{names:?}");
    }

    #[test]
    fn invocation_invoke_rejects_invalid_handle_after_zeroing_out_pointer() {
        let raw = valid_invocation_json();
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { runtime_invocation_invoke(9_999_999, raw.as_ptr(), &mut out) };
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
        let code = unsafe { runtime_invocation_invoke(handle, raw.as_ptr(), &mut out) };
        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn invocation_stream_open_rejects_invalid_handle_after_zeroing_stream_id() {
        let raw = valid_invocation_json();
        let mut stream_id: InvocationStreamId = 42;
        let code = unsafe {
            runtime_invocation_stream_open(
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
            runtime_invocation_bidi_open(
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
            runtime_invocation_bidi_open(
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
    fn invocation_bidi_open_rejects_missing_frame_zero_before_session_entry() {
        let (handle, session) = alloc(test_session());
        let owner = session.binding(handle);
        let raw = CString::new(
            serde_json::json!({
                "caller_ura": "easynet:///r/acme/device/dev-a",
                "callee_ura": "easynet:///r/acme/device/dev-a",
                "descriptor_ref": descriptor_ref(
                    "easynet:///r/acme/device/dev-a",
                    "device.pty.attach",
                    "2.4.0"
                ),
                "subject_ura": "easynet:///r/acme/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "args": {"session_id": "pty-1"},
            })
            .to_string(),
        )
        .unwrap();
        let before = active_bidi_len_for_owner(owner);
        let mut bidi_id: InvocationBidiId = 42;

        let code = unsafe {
            runtime_invocation_bidi_open(
                handle,
                raw.as_ptr(),
                Some(ignore_bidi_frame),
                std::ptr::null_mut(),
                &mut bidi_id,
            )
        };

        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(bidi_id, 0);
        assert_eq!(
            active_bidi_len_for_owner(owner),
            before,
            "missing C ABI bidi frame-0 material must be rejected before runtime session entry"
        );
        assert_typed_last_error(
            "INVALID_ARGUMENT",
            ERR_INVALID_ARG,
            "bidi_streams must not be empty",
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn rust_bidi_open_rejects_missing_frame_zero_before_session_entry() {
        let (handle, session) = alloc(test_session());
        let owner = session.binding(handle);
        let raw = serde_json::json!({
            "caller_ura": "easynet:///r/acme/device/dev-a",
            "callee_ura": "easynet:///r/acme/device/dev-a",
            "descriptor_ref": descriptor_ref(
                "easynet:///r/acme/device/dev-a",
                "device.pty.attach",
                "2.4.0"
            ),
            "subject_ura": "easynet:///r/acme/device/dev-a",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "args": {"session_id": "pty-1"},
        })
        .to_string();
        let before = active_bidi_len_for_owner(owner);
        let mut bidi_id: InvocationBidiId = 42;

        let code = bidi_open_with_axon_pb(
            handle,
            session,
            &raw,
            ignore_bidi_frame,
            std::ptr::null_mut(),
            &mut bidi_id,
        );

        assert_eq!(code, ERR_INVALID_ARG);
        assert_eq!(
            active_bidi_len_for_owner(owner),
            before,
            "missing Rust bidi frame-0 material must be rejected before runtime session entry"
        );
        assert_typed_last_error(
            "INVALID_ARGUMENT",
            ERR_INVALID_ARG,
            "bidi_streams must not be empty",
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_open_requires_callback_before_daemon_io() {
        let (handle, _) = alloc(test_session());
        let raw = valid_invocation_json();
        let mut stream_id: InvocationStreamId = 42;
        let code = unsafe {
            runtime_invocation_stream_open(
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
            runtime_invocation_stream_open(
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
    fn invocation_stream_cancel_rejects_unknown_invocation_resource() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { runtime_invocation_stream_cancel(handle, 9_999_999) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_typed_last_error(
            "INVALID_HANDLE",
            ERR_INVALID_HANDLE,
            "stream 9999999 is not registered",
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_cancel_rejects_unknown_invocation_resource() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { runtime_invocation_bidi_cancel(handle, 9_999_999) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_typed_last_error(
            "INVALID_HANDLE",
            ERR_INVALID_HANDLE,
            "bidi session 9999999 is not registered",
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_cancel_waits_for_canonical_acceptance_without_releasing_reader() {
        let (handle, session) = alloc(test_session());
        let owner = session.binding(handle);
        let submitter = Arc::new(BlockingCancellationCommandSubmitter::new());
        let cancellation = Arc::new(ProviderCancellationControl::with_submitter(
            submitter.clone(),
        ));
        let reader_cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream::new(
            owner,
            reader_cancel.clone(),
            cancellation.clone(),
        ));

        let first = std::thread::spawn(move || unsafe {
            runtime_invocation_stream_cancel(handle, stream_id)
        });
        submitter.entered.wait();
        assert!(
            get_stream_for_handle(owner, stream_id).unwrap().is_some(),
            "canonical cancel submission must not remove the stream"
        );
        assert!(
            !reader_cancel.is_cancelled(),
            "canonical cancel submission must leave the reader draining"
        );

        let second = std::thread::spawn(move || unsafe {
            runtime_invocation_stream_cancel(handle, stream_id)
        });
        cancellation.wait_for_waiting_callers(1);
        assert_eq!(
            submitter.calls.load(AtomicOrdering::SeqCst),
            1,
            "a duplicate request must wait for the in-flight canonical command"
        );
        assert!(get_stream_for_handle(owner, stream_id).unwrap().is_some());
        assert!(!reader_cancel.is_cancelled());

        submitter.release.wait();
        assert_eq!(first.join().expect("first cancel thread"), RUNTIME_OK);
        assert_eq!(second.join().expect("duplicate cancel thread"), RUNTIME_OK);
        assert_eq!(submitter.calls.load(AtomicOrdering::SeqCst), 1);
        assert!(get_stream_for_handle(owner, stream_id).unwrap().is_some());
        assert!(!reader_cancel.is_cancelled());

        assert_eq!(
            unsafe { runtime_invocation_stream_close(handle, stream_id) },
            RUNTIME_OK
        );
        assert!(reader_cancel.is_cancelled());
        assert!(get_stream_for_handle(owner, stream_id).unwrap().is_none());
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_cancel_is_idempotent_and_preserves_reader_until_local_close() {
        let (handle, session) = alloc(test_session());
        let owner = session.binding(handle);
        let submitter = Arc::new(CountingCancellationCommandSubmitter::new());
        let cancellation = Arc::new(ProviderCancellationControl::with_submitter(
            submitter.clone(),
        ));
        let (session, mut up_rx, reader_cancel) =
            active_bidi_session_with_cancellation(owner, 1, cancellation);
        let bidi_id = insert_bidi(session);

        assert_eq!(
            unsafe { runtime_invocation_bidi_cancel(handle, bidi_id) },
            RUNTIME_OK
        );
        assert_eq!(
            unsafe { runtime_invocation_bidi_cancel(handle, bidi_id) },
            RUNTIME_OK
        );
        assert_eq!(submitter.calls.load(AtomicOrdering::SeqCst), 1);
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_some());
        assert!(
            !reader_cancel.is_cancelled(),
            "accepted cancellation must preserve the terminal drain path"
        );

        assert_eq!(
            unsafe { runtime_invocation_bidi_close(handle, bidi_id) },
            RUNTIME_OK
        );
        assert_bidi_eof_frame(up_rx.try_recv().expect("local close EOF"), 1);
        assert!(reader_cancel.is_cancelled());
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_none());
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_cancel_memoizes_rejection_without_resubmission() {
        let (handle, session) = alloc(test_session());
        let owner = session.binding(handle);
        let submitter = Arc::new(RejectingCancellationCommandSubmitter::new());
        let cancellation = Arc::new(ProviderCancellationControl::with_submitter(
            submitter.clone(),
        ));
        let reader_cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream::new(
            owner,
            reader_cancel.clone(),
            cancellation,
        ));

        assert_eq!(
            unsafe { runtime_invocation_stream_cancel(handle, stream_id) },
            ERR_ABILITY_FAILED
        );
        assert_eq!(
            unsafe { runtime_invocation_stream_cancel(handle, stream_id) },
            ERR_ABILITY_FAILED
        );
        assert_eq!(
            submitter.calls.load(AtomicOrdering::SeqCst),
            1,
            "a rejected canonical cancellation must not sign and submit a second command"
        );
        assert!(get_stream_for_handle(owner, stream_id).unwrap().is_some());
        assert!(!reader_cancel.is_cancelled());

        assert_eq!(
            unsafe { runtime_invocation_stream_close(handle, stream_id) },
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_close_rejects_unknown_invocation_resource() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { runtime_invocation_stream_close(handle, 9_999_999) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_typed_last_error(
            "INVALID_HANDLE",
            ERR_INVALID_HANDLE,
            "stream 9999999 is not registered",
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_close_accepts_terminal_drained_resource() {
        let (handle, session) = alloc(test_session());
        let owner = session.binding(handle);
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream::new(
            owner,
            cancel.clone(),
            test_cancellation_control(),
        ));
        get_stream(stream_id)
            .expect("registered stream")
            .mark_reader_finished();

        let code = unsafe { runtime_invocation_stream_close(handle, stream_id) };

        assert_eq!(code, RUNTIME_OK);
        assert!(
            !cancel.is_cancelled(),
            "terminal-drained close must not submit a second local reader cancellation"
        );
        assert!(get_stream_for_handle(owner, stream_id).unwrap().is_none());
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_close_rejects_unknown_invocation_resource() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { runtime_invocation_bidi_close(handle, 9_999_999) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert_typed_last_error(
            "INVALID_HANDLE",
            ERR_INVALID_HANDLE,
            "bidi session 9999999 is not registered",
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_stream_close_refuses_cross_handle_access() {
        let (owner, owner_session) = alloc(test_session());
        let owner_binding = owner_session.binding(owner);
        let (other, _) = alloc(test_session());
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream::new(
            owner_binding,
            cancel.clone(),
            test_cancellation_control(),
        ));

        let code = unsafe { runtime_invocation_stream_close(other, stream_id) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(!cancel.is_cancelled());
        assert!(remove_stream(stream_id).is_some());
        crate::ffi::client::handle::release(owner);
        crate::ffi::client::handle::release(other);
    }

    #[test]
    fn invocation_bidi_close_send_rejects_unknown_session() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { runtime_invocation_bidi_close_send(handle, 9_999_999) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_close_send_refuses_cross_handle_access() {
        let (owner, owner_session) = alloc(test_session());
        let owner_binding = owner_session.binding(owner);
        let (other, _) = alloc(test_session());
        let (session, mut up_rx, _cancel) = active_bidi_session(owner_binding, 4);
        let bidi_id = insert_bidi(session);

        let code = unsafe { runtime_invocation_bidi_close_send(other, bidi_id) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(matches!(
            up_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(get_bidi_for_handle(owner_binding, bidi_id)
            .unwrap()
            .is_some());
        assert_eq!(
            unsafe { runtime_invocation_bidi_close(owner, bidi_id) },
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(owner);
        crate::ffi::client::handle::release(other);
    }

    #[test]
    fn invocation_bidi_close_send_fails_closed_without_frame_chain_mac() {
        let (handle, client_session) = alloc(test_session());
        let owner = client_session.binding(handle);
        let (session, mut up_rx, _cancel) = active_bidi_session(owner, 4);
        let bidi_id = insert_bidi(session);

        assert_eq!(
            unsafe { runtime_invocation_bidi_close_send(handle, bidi_id) },
            ERR_NOT_IMPLEMENTED
        );
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_some());
        assert_typed_last_error(
            "NOT_IMPLEMENTED",
            ERR_NOT_IMPLEMENTED,
            "close-send cannot attach the required 32-byte frame-chain MAC",
        );
        assert!(matches!(
            up_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let frame = CString::new(
            serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8=",
                "mac_base64": test_bidi_mac_base64()
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            unsafe { runtime_invocation_bidi_send(handle, bidi_id, frame.as_ptr()) },
            RUNTIME_OK
        );
        let sent = up_rx.try_recv().expect("MAC-bound data frame must be sent");
        assert_eq!(sent.sequence, 1);
        assert_eq!(sent.mac, vec![0xA5; BIDI_FRAME_CHAIN_MAC_BYTES]);
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_some());

        assert_eq!(
            unsafe { runtime_invocation_bidi_close(handle, bidi_id) },
            RUNTIME_OK
        );
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_none());
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn invocation_bidi_send_eof_also_half_closes_local_send() {
        let (handle, client_session) = alloc(test_session());
        let owner = client_session.binding(handle);
        let (session, mut up_rx, _cancel) = active_bidi_session(owner, 4);
        let bidi_id = insert_bidi(session);
        let eof = CString::new(
            serde_json::json!({
                "type": "control",
                "eof": true,
                "mac_base64": test_bidi_mac_base64()
            })
            .to_string(),
        )
        .unwrap();
        let frame = CString::new(
            serde_json::json!({
                "type": "binary_chunk",
                "stream_id": 1,
                "data_base64": "aGVsbG8=",
                "mac_base64": test_bidi_mac_base64()
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            unsafe { runtime_invocation_bidi_send(handle, bidi_id, eof.as_ptr()) },
            RUNTIME_OK
        );
        assert_bidi_eof_frame(up_rx.try_recv().expect("EOF frame must be sent"), 1);
        assert_eq!(
            unsafe { runtime_invocation_bidi_send(handle, bidi_id, frame.as_ptr()) },
            ERR_CANCELLED
        );
        assert!(get_bidi_for_handle(owner, bidi_id).unwrap().is_some());
        assert_eq!(
            unsafe { runtime_invocation_bidi_close(handle, bidi_id) },
            RUNTIME_OK
        );
        crate::ffi::client::handle::release(handle);
    }

    #[test]
    fn stream_registry_remove_returns_registered_cancel_token() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream::new(
            registry_owner(41, 4100),
            cancel.clone(),
            test_cancellation_control(),
        ));
        let stream = remove_stream(stream_id).expect("registered stream should be removable");
        stream.reader_cancel.cancel();
        assert!(cancel.is_cancelled());
        assert!(
            remove_stream(stream_id).is_none(),
            "stream removal must be one-shot"
        );
    }

    #[test]
    fn bidi_registry_remove_returns_registered_session() {
        let (session, _up_rx, cancel) = active_bidi_session(registry_owner(41, 4100), 1);
        let bidi_id = insert_bidi(session);
        let session = remove_bidi(bidi_id).expect("registered bidi session should be removable");
        session.reader_cancel.cancel();
        assert!(cancel.is_cancelled());
        assert!(
            remove_bidi(bidi_id).is_none(),
            "bidi removal must be one-shot"
        );
    }

    #[test]
    fn cancel_invocations_for_binding_removes_only_owned_entries() {
        let (owned_handle, owned_session) = alloc(test_session());
        let owned = owned_session.binding(owned_handle);
        let (other_handle, other_session) = alloc(test_session());
        let other = other_session.binding(other_handle);
        let owned_stream_cancel = tokio_util::sync::CancellationToken::new();
        let other_stream_cancel = tokio_util::sync::CancellationToken::new();
        let owned_stream_id = insert_stream(ActiveInvocationStream::new(
            owned,
            owned_stream_cancel.clone(),
            test_cancellation_control(),
        ));
        let other_stream_id = insert_stream(ActiveInvocationStream::new(
            other,
            other_stream_cancel.clone(),
            test_cancellation_control(),
        ));

        let (owned_bidi, _owned_up_rx, owned_bidi_cancel) = active_bidi_session(owned, 1);
        let (other_bidi, _other_up_rx, other_bidi_cancel) = active_bidi_session(other, 1);
        let owned_bidi_id = insert_bidi(owned_bidi);
        let other_bidi_id = insert_bidi(other_bidi);

        cancel_invocations_for_binding(owned);

        assert!(owned_stream_cancel.is_cancelled());
        assert!(owned_bidi_cancel.is_cancelled());
        assert!(remove_stream(owned_stream_id).is_none());
        assert!(remove_bidi(owned_bidi_id).is_none());

        assert!(!other_stream_cancel.is_cancelled());
        assert!(!other_bidi_cancel.is_cancelled());
        assert!(remove_stream(other_stream_id).is_some());
        assert!(remove_bidi(other_bidi_id).is_some());
        crate::ffi::client::handle::release(owned_handle);
        crate::ffi::client::handle::release(other_handle);
    }

    #[test]
    fn stream_registry_refuses_cross_handle_cancel() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let stream_id = insert_stream(ActiveInvocationStream::new(
            registry_owner(101, 1010),
            cancel,
            test_cancellation_control(),
        ));

        assert!(matches!(
            remove_stream_for_handle(registry_owner(101, 2020), stream_id),
            Err(RegistryOwnerMismatch)
        ));
        assert!(
            remove_stream(stream_id).is_some(),
            "owner mismatch must not remove another handle's stream"
        );
    }

    #[test]
    fn bidi_registry_refuses_cross_handle_access() {
        let (session, _up_rx, _cancel) = active_bidi_session(registry_owner(101, 1010), 1);
        let bidi_id = insert_bidi(session);

        assert!(matches!(
            get_bidi_for_handle(registry_owner(101, 2020), bidi_id),
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
            "runtime_invocation_invoke",
            crate::daemon::DaemonError::InvocationEndpointMissing {
                control: "/tmp/easynet/control.json".into(),
            },
        );

        assert_eq!(code, ERR_DAEMON_DOWN);
        let error = read_last_error_json();
        assert_eq!(error["code"], "RUNTIME_OFFLINE");
        assert_eq!(error["stage"], "transport");
        assert_eq!(error["retry"], "after_backoff");
        assert_eq!(error["details"]["abi_code"], ERR_DAEMON_DOWN);
        assert_eq!(error["details"]["abi_symbol"], "ERR_DAEMON_DOWN");
    }

    #[test]
    fn daemon_status_error_records_typed_last_error() {
        let code = ffi_daemon_error(
            "runtime_invocation_stream_open",
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
        assert_eq!(error["details"]["abi_symbol"], "ERR_PERMISSION_DENIED");
    }

    #[test]
    fn native_runtime_owner_offline_status_records_descriptor_owner_offline_projection() {
        let code = ffi_daemon_error(
            "runtime_invocation_invoke",
            crate::daemon::DaemonError::InvokeStatus {
                ability: "meta.list_abilities".to_string(),
                code: tonic::Code::Unavailable,
                message: "ROUTE_NEGATIVE: namespace.resolve negative for \
                     `easynet:///r/localhost/ability/device.dev-a.meta.list_abilities`: \
                     NEGATIVE_REASON_NXDOMAIN: owner is not online"
                    .to_string(),
            },
        );

        assert_eq!(
            code, ERR_DAEMON_DOWN,
            "ABI integer remains stable while typed JSON carries canonical routing state"
        );
        let error = read_last_error_json();
        assert_eq!(error["code"], "DESCRIPTOR_OWNER_OFFLINE");
        assert_eq!(error["stage"], "routing");
        assert_eq!(error["retry"], "safe");
        assert_eq!(error["details"]["abi_code"], ERR_DAEMON_DOWN);
        assert_eq!(error["details"]["abi_symbol"], "ERR_DAEMON_DOWN");
        assert!(error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("owner is not online"));
    }

    #[test]
    fn native_runtime_unavailable_without_owner_offline_remains_runtime_offline() {
        let code = ffi_daemon_error(
            "runtime_invocation_invoke",
            crate::daemon::DaemonError::InvokeStatus {
                ability: "observe.health".to_string(),
                code: tonic::Code::Unavailable,
                message: "transport unavailable".to_string(),
            },
        );

        assert_eq!(code, ERR_DAEMON_DOWN);
        let error = read_last_error_json();
        assert_eq!(error["code"], "RUNTIME_OFFLINE");
        assert_eq!(error["stage"], "transport");
        assert_eq!(error["retry"], "after_backoff");
    }

    #[test]
    fn native_runtime_signer_error_records_caller_signer_projection() {
        let err = SessionInvocationAuthority::caller_signer_unavailable_error(
            "easynet:///r/localhost/device/dev-a",
        );
        let message = err.to_string();
        assert!(message.contains("CALLER_SIGNER_UNAVAILABLE"));
        assert!(message.contains("requires a caller signer"));
        assert!(
            !message.contains("keyring entry not found")
                && !message.contains("keyring rejected request")
                && !message.contains("self-identity:")
                && !message.contains("KeyService signer"),
            "native signer error must not expose custody implementation details: {message}"
        );

        let code = ffi_daemon_error("runtime_invocation_invoke", err);

        assert_eq!(code, ERR_PERMISSION_DENIED);
        let error = read_last_error_json();
        assert_eq!(error["code"], "CALLER_SIGNER_UNAVAILABLE");
        assert_eq!(error["stage"], "caller_identity");
        assert_eq!(error["retry"], "never");
        assert_eq!(error["details"]["abi_code"], ERR_PERMISSION_DENIED);
        assert_eq!(error["details"]["abi_symbol"], "ERR_PERMISSION_DENIED");
        let projected_message = error["message"].as_str().unwrap_or_default();
        assert!(projected_message.contains("CALLER_SIGNER_UNAVAILABLE"));
        assert!(
            !projected_message.contains("keyring entry not found")
                && !projected_message.contains("keyring rejected request")
                && !projected_message.contains("self-identity:")
                && !projected_message.contains("KeyService signer"),
            "typed last-error must not expose custody implementation details: {projected_message}"
        );
    }

    #[test]
    fn stream_chunk_json_decodes_json_payload() {
        let chunk = axon_sdk::pb::axon::v1::InvokeStreamChunk {
            invocation_id: "inv-1".to_string(),
            state: 2,
            payload: br#"{"ready":true}"#.to_vec(),
            content_type: "application/json".to_string(),
            sequence: 7,
            terminal: true,
            ..axon_sdk::pb::axon::v1::InvokeStreamChunk::default()
        };
        let projection =
            stream_chunk_json(&mut InboundReceiptCheckpointVerifier::new(), chunk).unwrap();
        let value = projection.json();
        assert!(!projection.is_canonical_terminal());
        assert_eq!(value["ok"], true);
        assert_eq!(value["kind"], "data");
        assert_eq!(value["sequence"], 7);
        assert_eq!(value["terminal"], false);
        assert_eq!(value["payload_content_type"], "application/json");
        assert!(value.get("content_type").is_none());
        assert_eq!(value["payload_json"]["ready"], true);
        assert_eq!(value["payload_base64"], "eyJyZWFkeSI6dHJ1ZX0=");
    }

    #[test]
    fn stream_chunk_json_rejects_declared_json_payload_that_is_not_json() {
        let chunk = axon_sdk::pb::axon::v1::InvokeStreamChunk {
            invocation_id: "inv-1".to_string(),
            state: 2,
            payload: b"not-json".to_vec(),
            content_type: "application/json".to_string(),
            sequence: 7,
            terminal: false,
            ..axon_sdk::pb::axon::v1::InvokeStreamChunk::default()
        };

        let error = stream_chunk_json(&mut InboundReceiptCheckpointVerifier::new(), chunk)
            .expect_err("declared JSON stream payload must fail closed");
        assert!(error.contains("payload_json declares JSON content type"));
        assert!(error.contains("payload is not valid JSON"));
    }

    #[test]
    fn stream_chunk_json_projects_empty_declared_json_payload_as_no_value() {
        let chunk = axon_sdk::pb::axon::v1::InvokeStreamChunk {
            invocation_id: "inv-1".to_string(),
            state: 2,
            payload: Vec::new(),
            content_type: "application/json".to_string(),
            sequence: 7,
            terminal: false,
            ..axon_sdk::pb::axon::v1::InvokeStreamChunk::default()
        };

        let projection =
            stream_chunk_json(&mut InboundReceiptCheckpointVerifier::new(), chunk).unwrap();
        let value = projection.json();

        assert_eq!(value["payload_content_type"], "application/json");
        assert_eq!(value["payload_base64"], "");
        assert!(value["payload_json"].is_null());
    }

    #[test]
    fn callback_frame_projection_terminality_is_not_inferred_from_json_shape() {
        let projection = CallbackFrameProjection::new(
            serde_json::json!({
                "kind": "data",
                "terminal": false
            }),
            CallbackFrameLifecycle::CanonicalTerminal,
        );

        assert!(projection.is_canonical_terminal());
        assert_eq!(projection.json()["terminal"], false);
    }

    #[test]
    fn stream_status_error_is_transport_terminal_not_runtime_terminal() {
        let value =
            stream_status_error_json(tonic::Status::unavailable("stream transport closed"), 4);

        assert_eq!(value["ok"], false);
        assert_eq!(value["kind"], "error");
        assert_eq!(value["sequence"], 4);
        assert_eq!(value["code"], "Unavailable");
        assert_eq!(value["message"], "stream transport closed");
        assert_eq!(value["terminal"], false);
        assert_eq!(value["transport_terminal"], true);
    }

    #[test]
    fn bounded_callback_enqueue_reports_transport_terminal_backpressure_when_full() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
            tx.try_send(br#"{"sequence":1,"kind":"data"}"#.to_vec())
                .unwrap();

            let sender = tokio::spawn(async move {
                send_callback_frame_or_backpressure(
                    &tx,
                    br#"{"sequence":2,"kind":"data"}"#.to_vec(),
                    stream_callback_backpressure_event(2, 1),
                )
                .await
            });
            tokio::task::yield_now().await;

            assert_eq!(
                rx.recv().await.unwrap(),
                br#"{"sequence":1,"kind":"data"}"#.to_vec()
            );
            assert!(!sender.await.unwrap());
            let value =
                serde_json::from_slice::<serde_json::Value>(&rx.recv().await.unwrap()).unwrap();
            assert_eq!(value["kind"], "error");
            assert_eq!(value["sequence"], 2);
            assert_eq!(value["terminal"], false);
            assert_eq!(value["transport_terminal"], true);
            assert_eq!(
                value["error"]["details"]["reason"],
                "callback_queue_overflow"
            );
            assert_eq!(value["error"]["details"]["queue_capacity"], 1);
        });
    }

    #[test]
    fn bidi_down_frame_json_decodes_binary_chunk() {
        let frame = axon_sdk::pb::axon::v1::InvokeBidiDown {
            sequence: 3,
            payload: Some(
                axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::BinaryChunk(
                    axon_sdk::pb::axon::v1::BinaryChunk {
                        stream_id: 1,
                        data: b"hello".to_vec(),
                        pts: 11,
                    },
                ),
            ),
            ..axon_sdk::pb::axon::v1::InvokeBidiDown::default()
        };
        let projection =
            bidi_down_frame_json(&mut InboundReceiptCheckpointVerifier::new(), frame).unwrap();
        let value = projection.json();
        assert!(!projection.is_canonical_terminal());
        assert_eq!(value["ok"], true);
        assert_eq!(value["kind"], "data");
        assert_eq!(value["sequence"], 3);
        assert_eq!(value["stream_id"], 1);
        assert_eq!(value["payload_base64"], "aGVsbG8=");
        assert!(value.get("data_base64").is_none());
        assert_eq!(value["pts"], 11);
    }

    #[test]
    fn bidi_down_control_eof_is_remote_half_close_not_terminal() {
        let frame = axon_sdk::pb::axon::v1::InvokeBidiDown {
            sequence: 4,
            payload: Some(axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::Control(
                axon_sdk::pb::axon::v1::BidiControl {
                    control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true)),
                },
            )),
            ..axon_sdk::pb::axon::v1::InvokeBidiDown::default()
        };
        let projection =
            bidi_down_frame_json(&mut InboundReceiptCheckpointVerifier::new(), frame).unwrap();
        let value = projection.json();
        assert!(!projection.is_canonical_terminal());
        assert_eq!(value["ok"], true);
        assert_eq!(value["kind"], "control");
        assert_eq!(value["sequence"], 4);
        assert_eq!(value["control"]["eof"], true);
        assert_eq!(value["terminal"], false);
    }
}

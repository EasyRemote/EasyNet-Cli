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
// - It is not the retired ability+args ABI. That ABI is not exported
//   from the clean Invocation-only FFI surface.
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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

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
use crate::ffi::errors::{set_last_error, ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER};
#[cfg(feature = "axon-pb")]
use crate::ffi::strings::alloc_output_cstring;
use crate::ffi::strings::{read_cstr, StringError};

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
        set_last_error("easynet_invocation_invoke: out_receipt_json pointer is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_receipt_json = std::ptr::null_mut() };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            set_last_error(format!(
                "easynet_invocation_invoke: handle {handle} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            set_last_error("easynet_invocation_invoke: invocation_json pointer is null");
            return ERR_NULL_POINTER;
        }
        Err(StringError::NotUtf8) => {
            set_last_error("easynet_invocation_invoke: invocation_json is not valid UTF-8");
            return ERR_INVALID_UTF8;
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw);
        set_last_error("easynet_invocation_invoke: axon-pb feature is not enabled in this build");
        ERR_NOT_IMPLEMENTED
    }

    #[cfg(feature = "axon-pb")]
    {
        invoke_with_axon_pb(session, raw, out_receipt_json)
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
        set_last_error("easynet_invocation_stream_open: out_stream_id pointer is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_stream_id = 0 };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            set_last_error(format!(
                "easynet_invocation_stream_open: handle {handle} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
    };

    let Some(on_chunk) = on_chunk else {
        set_last_error("easynet_invocation_stream_open: on_chunk callback is null");
        return ERR_NULL_POINTER;
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            set_last_error("easynet_invocation_stream_open: invocation_json pointer is null");
            return ERR_NULL_POINTER;
        }
        Err(StringError::NotUtf8) => {
            set_last_error("easynet_invocation_stream_open: invocation_json is not valid UTF-8");
            return ERR_INVALID_UTF8;
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw, on_chunk, user_data);
        set_last_error(
            "easynet_invocation_stream_open: axon-pb feature is not enabled in this build",
        );
        ERR_NOT_IMPLEMENTED
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
        set_last_error(format!(
            "easynet_invocation_stream_cancel: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = stream_id;
        set_last_error(
            "easynet_invocation_stream_cancel: axon-pb feature is not enabled in this build",
        );
        ERR_NOT_IMPLEMENTED
    }

    #[cfg(feature = "axon-pb")]
    {
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
            Err(RegistryOwnerMismatch) => {
                set_last_error(format!(
                    "easynet_invocation_stream_cancel: stream {stream_id} does not belong to handle {handle}"
                ));
                ERR_INVALID_HANDLE
            }
        }
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
        set_last_error("easynet_invocation_bidi_open: out_bidi_id pointer is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_bidi_id = 0 };

    let session = match get(handle) {
        Some(session) => session,
        None => {
            set_last_error(format!(
                "easynet_invocation_bidi_open: handle {handle} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
    };

    let Some(on_frame) = on_frame else {
        set_last_error("easynet_invocation_bidi_open: on_frame callback is null");
        return ERR_NULL_POINTER;
    };

    let raw = match read_cstr(invocation_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            set_last_error("easynet_invocation_bidi_open: invocation_json pointer is null");
            return ERR_NULL_POINTER;
        }
        Err(StringError::NotUtf8) => {
            set_last_error("easynet_invocation_bidi_open: invocation_json is not valid UTF-8");
            return ERR_INVALID_UTF8;
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (session, raw, on_frame, user_data);
        set_last_error(
            "easynet_invocation_bidi_open: axon-pb feature is not enabled in this build",
        );
        ERR_NOT_IMPLEMENTED
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
        set_last_error(format!(
            "easynet_invocation_bidi_send: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    let raw = match read_cstr(frame_json) {
        Ok(value) => value,
        Err(StringError::Null) => {
            set_last_error("easynet_invocation_bidi_send: frame_json pointer is null");
            return ERR_NULL_POINTER;
        }
        Err(StringError::NotUtf8) => {
            set_last_error("easynet_invocation_bidi_send: frame_json is not valid UTF-8");
            return ERR_INVALID_UTF8;
        }
    };

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (bidi_id, raw);
        set_last_error(
            "easynet_invocation_bidi_send: axon-pb feature is not enabled in this build",
        );
        ERR_NOT_IMPLEMENTED
    }

    #[cfg(feature = "axon-pb")]
    {
        bidi_send_with_axon_pb(handle, bidi_id, raw)
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
        set_last_error(format!(
            "easynet_invocation_bidi_close: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        set_last_error(
            "easynet_invocation_bidi_close: axon-pb feature is not enabled in this build",
        );
        ERR_NOT_IMPLEMENTED
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
        set_last_error(format!(
            "easynet_invocation_bidi_cancel: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = bidi_id;
        set_last_error(
            "easynet_invocation_bidi_cancel: axon-pb feature is not enabled in this build",
        );
        ERR_NOT_IMPLEMENTED
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
            Err(RegistryOwnerMismatch) => {
                set_last_error(format!(
                    "easynet_invocation_bidi_cancel: bidi session {bidi_id} does not belong to handle {handle}"
                ));
                ERR_INVALID_HANDLE
            }
        }
    }
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
            set_last_error(format!("easynet_invocation_invoke: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let invocation = match spec.clone().into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            set_last_error(format!("easynet_invocation_invoke: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            set_last_error(format!("easynet_invocation_invoke: {err}"));
            return ERR_GENERIC;
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

    if let Some(err) = response.error {
        set_last_error(format!(
            "easynet_invocation_invoke: daemon returned error \"{}\": {}",
            err.code, err.message
        ));
        return ERR_ABILITY_FAILED;
    }

    let output = invocation_output_json(&spec, response);
    let json = match serde_json::to_string(&output) {
        Ok(json) => json,
        Err(err) => {
            set_last_error(format!(
                "easynet_invocation_invoke: encode response JSON failed: {err}"
            ));
            return ERR_GENERIC;
        }
    };
    let ptr = alloc_output_cstring(json);
    if ptr.is_null() {
        set_last_error("easynet_invocation_invoke: out-of-memory allocating response string");
        return ERR_GENERIC;
    }
    unsafe { *out_receipt_json = ptr };
    clear_last_error();
    EASYNET_OK
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
            set_last_error(format!("easynet_invocation_stream_open: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            set_last_error(format!("easynet_invocation_stream_open: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            set_last_error(format!("easynet_invocation_stream_open: {err}"));
            return ERR_GENERIC;
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
        set_last_error(format!(
            "easynet_invocation_stream_open: spawn callback dispatcher failed: {err}"
        ));
        return ERR_GENERIC;
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
            set_last_error(format!("easynet_invocation_bidi_open: {err}"));
            return ERR_INVALID_ARG;
        }
    };
    if spec.bidi_streams.is_empty() {
        set_last_error("easynet_invocation_bidi_open: bidi_streams must not be empty");
        return ERR_INVALID_ARG;
    }

    let streams = spec.bidi_streams.clone();
    let invocation = match spec.into_daemon_invocation() {
        Ok(invocation) => invocation,
        Err(err) => {
            set_last_error(format!("easynet_invocation_bidi_open: {err}"));
            return ERR_INVALID_ARG;
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            set_last_error(format!("easynet_invocation_bidi_open: {err}"));
            return ERR_GENERIC;
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
        set_last_error(format!(
            "easynet_invocation_bidi_open: spawn callback dispatcher failed: {err}"
        ));
        return ERR_GENERIC;
    }

    let (ability, up_tx, down) = session.into_parts();
    let cancel = tokio_util::sync::CancellationToken::new();
    let bidi_id = insert_bidi(ActiveInvocationBidi {
        owner: handle,
        ability,
        up_tx,
        cancel: cancel.clone(),
        next_sequence: AtomicU64::new(1),
    });
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
            set_last_error(format!("easynet_invocation_bidi_send: {err}"));
            return ERR_INVALID_ARG;
        }
    };
    let session = match get_bidi_for_handle(handle, bidi_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            set_last_error(format!(
                "easynet_invocation_bidi_send: bidi session {bidi_id} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
        Err(RegistryOwnerMismatch) => {
            set_last_error(format!(
                "easynet_invocation_bidi_send: bidi session {bidi_id} does not belong to handle {handle}"
            ));
            return ERR_INVALID_HANDLE;
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            set_last_error(format!("easynet_invocation_bidi_send: {err}"));
            return ERR_GENERIC;
        }
    };
    let send_result = rt.block_on(async {
        let sequence = session.next_sequence.fetch_add(1, Ordering::Relaxed);
        session
            .up_tx
            .send(easynet_axon::pb::axon::v1::InvokeBidiUp {
                sequence,
                mac: frame.mac,
                payload: Some(frame.payload),
            })
            .await
    });
    if send_result.is_err() {
        set_last_error(format!(
            "easynet_invocation_bidi_send: bidi session {} for {} is closed",
            bidi_id, session.ability
        ));
        let _ = remove_bidi(bidi_id);
        return ERR_CANCELLED;
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
            set_last_error(format!(
                "easynet_invocation_bidi_close: bidi session {bidi_id} does not belong to handle {handle}"
            ));
            return ERR_INVALID_HANDLE;
        }
    };
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(err) => {
            set_last_error(format!("easynet_invocation_bidi_close: {err}"));
            return ERR_GENERIC;
        }
    };
    let sequence = session.next_sequence.fetch_add(1, Ordering::Relaxed);
    let send_result = rt.block_on(async {
        use easynet_axon::pb::axon::v1::{bidi_control, invoke_bidi_up, BidiControl, InvokeBidiUp};
        session
            .up_tx
            .send(InvokeBidiUp {
                sequence,
                mac: Vec::new(),
                payload: Some(invoke_bidi_up::Payload::Control(BidiControl {
                    control: Some(bidi_control::Control::Eof(true)),
                })),
            })
            .await
    });
    if send_result.is_err() {
        set_last_error(format!(
            "easynet_invocation_bidi_close: bidi session {} for {} is already closed",
            bidi_id, session.ability
        ));
        return ERR_CANCELLED;
    }
    clear_last_error();
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
struct ActiveInvocationBidi {
    owner: EasynetHandle,
    ability: String,
    up_tx: tokio::sync::mpsc::Sender<easynet_axon::pb::axon::v1::InvokeBidiUp>,
    cancel: tokio_util::sync::CancellationToken,
    next_sequence: AtomicU64,
}

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
    set_last_error(format!("{context}: {err}"));
    code
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
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            message = stream.message() => match message {
                Ok(Some(chunk)) => {
                    let terminal = chunk.terminal;
                    let bytes = stream_chunk_json(chunk).to_string().into_bytes();
                    if tx.send(bytes).await.is_err() || terminal {
                        break;
                    }
                }
                Ok(None) => break,
                Err(status) => {
                    let bytes = stream_status_error_json(status).to_string().into_bytes();
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
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            message = down.message() => match message {
                Ok(Some(frame)) => {
                    let terminal = bidi_down_frame_is_terminal(&frame);
                    let bytes = bidi_down_frame_json(frame).to_string().into_bytes();
                    if tx.send(bytes).await.is_err() || terminal {
                        break;
                    }
                }
                Ok(None) => break,
                Err(status) => {
                    let bytes = stream_status_error_json(status).to_string().into_bytes();
                    let _ = tx.send(bytes).await;
                    break;
                }
            }
        }
    }
    let _ = remove_bidi(bidi_id);
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
struct BidiUpFrame {
    mac: Vec<u8>,
    payload: easynet_axon::pb::axon::v1::invoke_bidi_up::Payload,
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
    let key_id_hint = optional_string(signature_obj, "key_id_hint")?.unwrap_or_default();
    Ok(Some(easynet_axon::pb::axon::v1::CallerSignature {
        algorithm,
        signature,
        key_id_hint,
    }))
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
fn invocation_output_json(
    spec: &InvocationJson,
    response: easynet_axon::pb::axon::v1::InvokeResponse,
) -> serde_json::Value {
    use base64::Engine;
    let result_base64 = base64::engine::general_purpose::STANDARD.encode(&response.result);
    let result_json = if response.result_content_type == "application/json" {
        serde_json::from_slice::<serde_json::Value>(&response.result).ok()
    } else {
        None
    };
    serde_json::json!({
        "ok": true,
        "caller_ura": spec.caller_ura,
        "callee_ura": spec.callee_ura,
        "descriptor_ref": spec.descriptor_ref,
        "subject_ura": spec.subject_ura,
        "nonce_base64": base64::engine::general_purpose::STANDARD.encode(spec.nonce),
        "state": response.state,
        "selected_node_id": response.selected_node_id,
        "scheduling_reason": response.scheduling_reason,
        "elapsed_ms": response.elapsed_ms,
        "result_content_type": response.result_content_type,
        "result_base64": result_base64,
        "result_json": result_json,
        "admission_receipt": response.admission_receipt.as_ref().map(receipt_summary_json),
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
fn stream_status_error_json(status: tonic::Status) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "event": "error",
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
    use std::ffi::{c_void, CString};

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
        CString::new(
            serde_json::json!({
                "caller_ura": "ura://device/test/caller",
                "callee_ura": "ura://device/test/callee",
                "descriptor_ref": "ura://device/test/callee/ability/observe.health@2.4.0",
                "subject_ura": "ura://device/test/callee",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "args": {"ping": true}
            })
            .to_string(),
        )
        .unwrap()
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
                "caller_ura": "ura://device/test/caller",
                "callee_ura": "ura://device/test/callee",
                "descriptor_ref": "ura://device/test/callee/ability/observe.health@2.4.0",
                "subject_ura": "ura://device/test/callee",
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

    /// Canonical-URA invocation JSON for tests that go past parse into
    /// `into_daemon_invocation` (the builder's `checked_ura` rejects
    /// the legacy `ura://` fixture shapes that parse-only tests use).
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
                    "caller_ura": "ura://device/test/caller",
                    "callee_ura": "ura://device/test/callee",
                    "descriptor_ref": "ura://device/test/callee/ability/observe.health@2.4.0",
                    "subject_ura": "ura://device/test/callee",
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
    fn parse_invocation_json_rejects_zero_nonce() {
        let err = InvocationJson::parse(
            r#"{
                "caller_ura": "ura://device/test/caller",
                "callee_ura": "ura://device/test/callee",
                "descriptor_ref": "ura://device/test/callee/ability/observe.health@2.4.0",
                "subject_ura": "ura://device/test/callee",
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
                "caller_ura": "ura://device/test/caller",
                "callee_ura": "ura://device/test/callee",
                "descriptor_ref": "ura://device/test/callee/ability/observe.health@2.4.0",
                "subject_ura": "ura://device/test/callee",
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

    #[test]
    fn invocation_invoke_rejects_invalid_handle_after_zeroing_out_pointer() {
        let raw = valid_invocation_json();
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code = unsafe { easynet_invocation_invoke(9_999_999, raw.as_ptr(), &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
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
    }

    #[test]
    fn invocation_bidi_cancel_is_idempotent_for_unknown_session() {
        let (handle, _) = alloc(test_session());
        let code = unsafe { easynet_invocation_bidi_cancel(handle, 9_999_999) };
        assert_eq!(code, EASYNET_OK);
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
        let (up_tx, _up_rx) = tokio::sync::mpsc::channel(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let bidi_id = insert_bidi(ActiveInvocationBidi {
            owner: 41,
            ability: "device.pty.attach".to_string(),
            up_tx,
            cancel: cancel.clone(),
            next_sequence: AtomicU64::new(1),
        });
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

        let (owned_up_tx, _owned_up_rx) = tokio::sync::mpsc::channel(1);
        let (other_up_tx, _other_up_rx) = tokio::sync::mpsc::channel(1);
        let owned_bidi_cancel = tokio_util::sync::CancellationToken::new();
        let other_bidi_cancel = tokio_util::sync::CancellationToken::new();
        let owned_bidi_id = insert_bidi(ActiveInvocationBidi {
            owner: 41,
            ability: "device.pty.attach".to_string(),
            up_tx: owned_up_tx,
            cancel: owned_bidi_cancel.clone(),
            next_sequence: AtomicU64::new(1),
        });
        let other_bidi_id = insert_bidi(ActiveInvocationBidi {
            owner: 42,
            ability: "device.pty.attach".to_string(),
            up_tx: other_up_tx,
            cancel: other_bidi_cancel.clone(),
            next_sequence: AtomicU64::new(1),
        });

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
        let (up_tx, _up_rx) = tokio::sync::mpsc::channel(1);
        let bidi_id = insert_bidi(ActiveInvocationBidi {
            owner: 101,
            ability: "device.pty.attach".to_string(),
            up_tx,
            cancel: tokio_util::sync::CancellationToken::new(),
            next_sequence: AtomicU64::new(1),
        });

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

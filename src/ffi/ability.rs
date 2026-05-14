// EasyNet CLI — Generic ability invoke/subscribe ABI
// ====================================================
//
// File: src/ffi/ability.rs
// Description: The two ability helpers every Client FFI binding
//              uses: `easynet_ability_invoke` (RPC) and
//              `easynet_ability_subscribe` (streaming). Feature PRs
//              extend the daemon side; the ABI stays fixed at this
//              pair of generic functions so Clients never need a
//              lib rebuild when a new ability is added.
//
// Why only two functions, not one per ability
// -------------------------------------------
// Every ability call has the same shape: "name + JSON args → JSON
// result (RPC)" or "name + JSON args → stream of JSON frames
// (Subscribe)". Keeping the ABI at two functions means:
//
//   (a) Adding `device.session.attach` doesn't bump the ABI.
//   (b) Client bindings can be auto-generated from `.proto` files
//       without a per-ability C wrapper layer.
//   (c) The ABI stability contract has exactly two functions to
//       review for breakage.
//
// v1 status (PR-DAEMON Commit 4)
// -------------------------------
// `easynet_ability_invoke` is wired through `IpcClient::round_trip`
// to the daemon. The daemon's `AbilityProxy` still returns the v1
// skeleton `Error` envelope (PR-INVOCATION-EXEC-UNITY swaps that
// for real Kernel::invoke dispatch), so a successful round-trip
// today returns `ERR_ABILITY_FAILED` with the daemon's message in
// the last-error slot — distinct from "transport broke".
//
// `easynet_ability_subscribe` remains a skeleton: streaming needs
// a long-lived reader task per subscription plus a bounded channel
// pumping bytes back to the FFI callback. That lands in a focused
// follow-up commit; shipping it half-built would create a confusing
// state where some subscriptions deliver frames and others silently
// drop.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::os::raw::{c_char, c_void};

use crate::ffi::client::IpcClient;
use crate::ffi::errors::{
    set_last_error, ERR_ABILITY_FAILED, ERR_DAEMON_DOWN, ERR_GENERIC, ERR_INVALID_HANDLE,
    ERR_NULL_POINTER,
};
use crate::ffi::handle::{get, lib_runtime, EasynetHandle};
use crate::ffi::strings::{alloc_output_cstring, read_cstr};
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};
#[cfg(unix)]
use crate::services::control::transport;

/// Subscription id type returned from `easynet_ability_subscribe`.
/// 0 means "no subscription allocated" (error path).
pub type SubscriptionId = u64;

/// Frame callback: invoked once per streaming `Frame` arriving
/// from the daemon. The `frame_json` pointer is borrowed — valid
/// only for the duration of the callback; the Client must copy it
/// if retention is needed.
///
/// `user_data` is opaque; the lib does not dereference it.
pub type FrameCallback = unsafe extern "C" fn(user_data: *mut c_void, frame_json: *const c_char);

/// Generate a request id for an Invoke frame the FFI synthesises.
///
/// Why FFI-side and not Client-side: the Client doesn't see the
/// IPC layer; the FFI is the only place that can guarantee
/// uniqueness within a single library load. v1 uses a process-
/// monotonic counter (good enough — `request_id` is a v1 dedupe
/// key, not a security claim). The follow-up commit that wires
/// real Invocation envelopes will replace this with the AXIOM §2
/// nonce + caller-side canonical bytes.
fn next_invoke_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("ffi-{n}")
}

/// Invoke an ability synchronously. Writes the result JSON string
/// to `*out_result` (caller frees via `easynet_string_free`) and
/// returns `EASYNET_OK`. On failure returns a non-zero code and
/// `*out_result` is set to NULL.
///
/// Mapping of daemon responses to ABI codes:
///   - `OutgoingFrame::Result` → `EASYNET_OK`, `*out_result` =
///     newly-allocated CString of the `value` JSON.
///   - `OutgoingFrame::Error`  → `ERR_ABILITY_FAILED`, last-error
///     set to the daemon's message; `*out_result` = NULL.
///   - Transport failure (peer closed, decode error) →
///     `ERR_DAEMON_DOWN`; last-error carries the I/O reason.
///
/// # Safety
/// - `handle` must be a valid handle from a successful `easynet_init`.
/// - `ability` and `args_json` must be valid UTF-8 C strings.
/// - `out_result` must be a non-null pointer to a `*mut c_char`
///   the caller owns.
#[no_mangle]
pub unsafe extern "C" fn easynet_ability_invoke(
    handle: EasynetHandle,
    ability: *const c_char,
    args_json: *const c_char,
    out_result: *mut *mut c_char,
) -> i32 {
    if out_result.is_null() {
        set_last_error("easynet_ability_invoke: out_result pointer is null");
        return ERR_NULL_POINTER;
    }
    // Initialise the out pointer to NULL so the caller can safely
    // check it on the failure path.
    unsafe { *out_result = std::ptr::null_mut() };

    let session = match get(handle) {
        Some(s) => s,
        None => {
            set_last_error(format!(
                "easynet_ability_invoke: handle {handle} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
    };

    let ability_name = match read_cstr(ability) {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_error("easynet_ability_invoke: ability name is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };
    let args_raw = match read_cstr(args_json) {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_error("easynet_ability_invoke: args_json is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };

    // Parse args JSON into a serde_json::Value. `args_json` is a
    // string of JSON, not a JSON-encoded string of JSON — passing
    // `{}` literal is correct, passing `"{}"` (with extra quoting)
    // is a Client bug we surface as ERR_NULL_POINTER.
    let args_value: serde_json::Value = match serde_json::from_str(&args_raw) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(format!(
                "easynet_ability_invoke: args_json is not valid JSON: {e}"
            ));
            return ERR_NULL_POINTER;
        }
    };

    let req = IncomingFrame::Invoke {
        request_id: next_invoke_request_id(),
        ability: ability_name,
        args: args_value,
        subject: None,
    };

    // Build / fetch the lib's tokio runtime; hold the IPC client
    // mutex across the round-trip so concurrent FFI calls on the
    // same handle serialise (single framed stream cannot interleave
    // request/response pairs).
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(format!("easynet_ability_invoke: {e}"));
            return ERR_GENERIC;
        }
    };

    let client_mutex = match session.client.as_ref() {
        Some(m) => m,
        None => {
            // A test session created without a real IPC client
            // somehow ended up with a registered handle — only
            // possible from inside the test suite. Surface it as a
            // generic failure rather than panic.
            set_last_error(
                "easynet_ability_invoke: handle has no IPC client \
                 (test-only session?); call easynet_init first",
            );
            return ERR_GENERIC;
        }
    };

    let resp_result = {
        let mut client = client_mutex.lock().expect("ipc client mutex not poisoned");
        rt.block_on(client.round_trip(req))
    };

    let resp = match resp_result {
        Ok(r) => r,
        Err(e) => {
            set_last_error(format!("easynet_ability_invoke: {e:#}"));
            return ERR_DAEMON_DOWN;
        }
    };

    match resp {
        OutgoingFrame::Result { value, .. } => {
            // Serialise the value back to a JSON string the caller
            // will read out via `*out_result`. The string is heap-
            // allocated; the caller must `easynet_string_free` it.
            let json = match serde_json::to_string(&value) {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!(
                        "easynet_ability_invoke: encode result JSON failed: {e}"
                    ));
                    return ERR_GENERIC;
                }
            };
            let ptr = alloc_output_cstring(json);
            if ptr.is_null() {
                set_last_error("easynet_ability_invoke: out-of-memory allocating result string");
                return ERR_GENERIC;
            }
            unsafe { *out_result = ptr };
            crate::ffi::errors::clear_last_error();
            crate::ffi::errors::EASYNET_OK
        }
        OutgoingFrame::Error { code, message, .. } => {
            set_last_error(format!(
                "easynet_ability_invoke: daemon returned error \"{code}\": {message}"
            ));
            ERR_ABILITY_FAILED
        }
        // The daemon should not send Frame/Terminal in response to
        // an Invoke (those are subscription replies). Treat as a
        // protocol violation — fail loud, don't paper over.
        other => {
            set_last_error(format!(
                "easynet_ability_invoke: daemon sent unexpected frame for an Invoke: {other:?}"
            ));
            ERR_DAEMON_DOWN
        }
    }
}

/// Subscribe to a streaming ability. Spawns a reader task on the
/// lib's tokio runtime; that task dials a fresh UDS connection,
/// sends a `Subscribe` frame, then loops decoding response frames
/// and invoking `on_frame` once per data frame. A `Terminal` frame
/// (or a transport error) ends the loop and the task exits.
///
/// Why a fresh connection per subscription
/// ---------------------------------------
/// The session's RPC socket already runs a one-frame-in /
/// one-frame-out contract behind a Mutex. Multiplexing a long-lived
/// subscription stream onto it would require a per-connection
/// reader/writer split mirroring the daemon's, which doubles the
/// FFI complexity. A fresh socket per subscription keeps each path
/// simple; the cost is one extra UDS file handle per active
/// subscription, which on a desktop is irrelevant.
///
/// Returns a non-zero subscription id the caller uses to cancel.
/// The id is local to the FFI registry — it is NOT the same as
/// the wire-level subscription_id sent in the Subscribe frame
/// (which the lib generates separately).
///
/// # Safety
/// - `handle` must be a valid handle from a successful `easynet_init`.
/// - `ability` and `args_json` must be valid UTF-8 C strings.
/// - `on_frame` must not be null. It is invoked from a tokio
///   worker thread inside the lib; the callback must not block
///   indefinitely. `user_data` is opaque; the lib does not
///   dereference it but does pass it back verbatim on every
///   callback.
#[no_mangle]
pub unsafe extern "C" fn easynet_ability_subscribe(
    handle: EasynetHandle,
    ability: *const c_char,
    args_json: *const c_char,
    on_frame: Option<FrameCallback>,
    user_data: *mut c_void,
    out_subscription_id: *mut SubscriptionId,
) -> i32 {
    if out_subscription_id.is_null() {
        set_last_error("easynet_ability_subscribe: out_subscription_id is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_subscription_id = 0 };

    let cb = match on_frame {
        Some(cb) => cb,
        None => {
            set_last_error("easynet_ability_subscribe: on_frame callback is null");
            return ERR_NULL_POINTER;
        }
    };

    let session = match get(handle) {
        Some(s) => s,
        None => {
            set_last_error(format!(
                "easynet_ability_subscribe: handle {handle} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
    };

    let ability = match read_cstr(ability) {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_error("easynet_ability_subscribe: ability name is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };
    let args = match read_cstr(args_json) {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_error("easynet_ability_subscribe: args_json is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };

    let args_value: serde_json::Value = match serde_json::from_str(&args) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(format!("easynet_ability_subscribe: args_json invalid: {e}"));
            return ERR_GENERIC;
        }
    };

    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(format!("easynet_ability_subscribe: {e}"));
            return ERR_GENERIC;
        }
    };

    // Allocate a local subscription id + cancellation token.
    let sub_id = next_subscription_id();
    let token = tokio_util::sync::CancellationToken::new();
    {
        let mut g = session
            .subscriptions
            .lock()
            .expect("subscriptions registry lock");
        g.insert(sub_id, token.clone());
    }

    // user_data is *mut c_void — not Send by default. Wrap it in a
    // newtype that asserts Send because the Client guarantees the
    // pointer outlives the subscription (or sets it to null).
    let user_data = UserDataPtr(user_data);

    let control_path = std::path::PathBuf::from(&session.control_path);
    let session_for_cleanup = session.clone();
    rt.spawn(async move {
        let _ = run_subscription(
            control_path,
            ability,
            args_value,
            cb,
            user_data,
            token.clone(),
        )
        .await;
        // De-register on exit.
        let mut g = session_for_cleanup
            .subscriptions
            .lock()
            .expect("subscriptions registry lock");
        g.remove(&sub_id);
    });

    unsafe { *out_subscription_id = sub_id };
    crate::ffi::errors::clear_last_error();
    crate::ffi::errors::EASYNET_OK
}

fn next_subscription_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The actual reader task. Dials a fresh UDS connection at
/// `control.sock` (resolved from the daemon's `control.json`),
/// sends one `Subscribe` frame, then loops decoding `Frame` /
/// `Terminal` / `Error` envelopes and invoking `on_frame`.
///
/// Returns Ok when the task exits cleanly (Terminal / cancel /
/// peer close). Errors are recorded but not propagated to the C
/// caller — the caller already returned from
/// `easynet_ability_subscribe` before this task started.
/// Send-asserting wrapper around `*mut c_void`. The Client owns
/// the pointer and guarantees it outlives the subscription; the
/// lib never dereferences it.
struct UserDataPtr(*mut c_void);
unsafe impl Send for UserDataPtr {}

impl UserDataPtr {
    /// Read the wrapped raw pointer. Done via a method (not field
    /// projection) so closure capture sees `&UserDataPtr` instead
    /// of `&*mut c_void` — Rust 2021's disjoint-field capture would
    /// otherwise drop the Send guarantee on the spawned thread.
    fn raw(&self) -> *mut c_void {
        self.0
    }
}

async fn run_subscription(
    control_path: std::path::PathBuf,
    ability: String,
    args: serde_json::Value,
    on_frame: FrameCallback,
    user_data: UserDataPtr,
    token: tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    use bytes::Bytes;
    use futures::SinkExt;

    // Dial control.json → resolve socket path → connect a fresh
    // UnixStream. We deliberately do NOT reuse the session's
    // existing IpcClient: that one runs a Mutex-guarded round-trip
    // contract, and a long-lived stream would block every
    // concurrent ability_invoke on the same handle.
    let disc = match crate::services::control::discovery::read(&control_path) {
        Ok(Some(d)) => d,
        Ok(None) => anyhow::bail!("control.json missing at {}", control_path.display()),
        Err(e) => anyhow::bail!("read control.json: {e}"),
    };
    #[cfg(unix)]
    let stream = {
        let socket_path = disc
            .socket_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("control.json has no socket_path"))?;
        let _ = transport::default_socket_path(); // re-export keeps the import "used"
        tokio::net::UnixStream::connect(&socket_path).await?
    };
    #[cfg(windows)]
    let stream = {
        let pipe_name = disc
            .pipe_name
            .clone()
            .ok_or_else(|| anyhow::anyhow!("control.json has no pipe_name"))?;
        crate::support::named_pipe::connect_with_retry(
            &pipe_name,
            std::time::Duration::from_secs(5),
        )
        .await?
    };
    let codec = tokio_util::codec::LengthDelimitedCodec::builder()
        .little_endian()
        .new_codec();
    let mut framed = tokio_util::codec::Framed::new(stream, codec);

    // Wire-level subscription_id is independent of the FFI-level
    // one; the daemon needs it to route Cancel frames. We use a
    // UUID so two subscriptions on the same connection (which we
    // do not multiplex today, but a future change might) don't
    // collide.
    let wire_sub_id = uuid::Uuid::new_v4().to_string();
    let req = IncomingFrame::Subscribe {
        subscription_id: wire_sub_id.clone(),
        ability,
        args,
    };
    let bytes = serde_json::to_vec(&req)?;
    framed.send(Bytes::from(bytes)).await?;

    // Dedicated OS thread for delivering frames to the C callback.
    //
    // Why an OS thread and not "just call on_frame here":
    // ---------------------------------------------------
    // Bindings like koffi (Node) implement the callback round-trip
    // synchronously: when a non-V8-thread calls the registered
    // function pointer, koffi's native trampoline blocks the caller
    // on a condvar until the V8 main thread has executed the JS
    // callback. If we invoke on_frame from a tokio worker, that
    // worker is parked for the entire JS dispatch. With the lib's
    // 2-worker runtime, two concurrent frame deliveries can park
    // both workers while V8 main is parked in `block_on` for the
    // next `easynet_ability_invoke` — three-way deadlock.
    //
    // Punting the synchronous on_frame call to a dedicated, non-
    // tokio thread keeps every tokio worker free. The tokio reader
    // only does a non-blocking mpsc send; the dispatcher thread
    // owns the callback's blocking semantics.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    // Move the Send-asserting wrapper itself into the closure (NOT
    // the raw pointer; destructuring inside the closure would re-
    // introduce the *mut c_void into the captured set and trip Send).
    let cb_user_data: UserDataPtr = user_data;
    let dispatcher = std::thread::Builder::new()
        .name(format!("easynet-cb-{wire_sub_id}"))
        .spawn(move || {
            // Use the accessor method instead of field projection
            // so the closure captures `cb_user_data: UserDataPtr`
            // (Send) rather than the raw pointer (not Send).
            let ud = cb_user_data.raw();
            while let Ok(json_bytes) = rx.recv() {
                let cstr = match std::ffi::CString::new(json_bytes) {
                    Ok(c) => c,
                    Err(_) => continue, // contained NUL; skip
                };
                // SAFETY: on_frame was validated non-null at the
                // FFI call site; the C string lives for the call.
                unsafe {
                    on_frame(ud, cstr.as_ptr());
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("spawn dispatcher thread: {e}"))?;

    let result = run_subscription_loop(&mut framed, &token, &wire_sub_id, &tx).await;

    // Dropping the sender wakes the dispatcher's `rx.recv()` with
    // Err and lets the thread exit. Joining is best-effort: the
    // dispatcher thread blocks on the user's callback, so a runaway
    // user callback would also park us — detach instead.
    drop(tx);
    let _ = dispatcher; // detach; thread exits on channel close

    result
}

async fn run_subscription_loop<T>(
    framed: &mut tokio_util::codec::Framed<T, tokio_util::codec::LengthDelimitedCodec>,
    token: &tokio_util::sync::CancellationToken,
    wire_sub_id: &str,
    tx: &std::sync::mpsc::Sender<Vec<u8>>,
) -> anyhow::Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use bytes::Bytes;
    use futures::{SinkExt, StreamExt};

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let cancel = IncomingFrame::Cancel { subscription_id: wire_sub_id.to_string() };
                if let Ok(b) = serde_json::to_vec(&cancel) {
                    let _ = framed.send(Bytes::from(b)).await;
                }
                break;
            }
            recv = framed.next() => match recv {
                None => break, // connection closed
                Some(Err(e)) => return Err(anyhow::anyhow!("framed read: {e}")),
                Some(Ok(bytes)) => {
                    let outgoing: OutgoingFrame = serde_json::from_slice(&bytes)?;
                    match outgoing {
                        OutgoingFrame::Frame { frame, .. } => {
                            let json = serde_json::to_vec(&frame)?;
                            // Hand the JSON off to the dispatcher
                            // thread. If the dispatcher has died the
                            // send fails — treat like terminal.
                            if tx.send(json).is_err() { break; }
                        }
                        OutgoingFrame::Terminal { .. } => break,
                        OutgoingFrame::Error { message, .. } => {
                            let v = serde_json::json!({
                                "kind": "error",
                                "message": message,
                            });
                            if let Ok(json) = serde_json::to_vec(&v) {
                                let _ = tx.send(json);
                            }
                            break;
                        }
                        OutgoingFrame::Result { .. } => continue,
                        // Bidi-only outbound variants must never appear on a
                        // Subscribe response stream — the server only emits
                        // them in reply to OpenBidi, which this code path
                        // never sends. Treat as a server protocol violation:
                        // surface as an error to the dispatcher and end the
                        // stream so a buggy server can't silently pump bidi
                        // frames into a stream subscriber's queue.
                        OutgoingFrame::RecvBidi { .. }
                        | OutgoingFrame::TerminalBidi { .. }
                        | OutgoingFrame::ErrorBidi { .. } => {
                            let v = serde_json::json!({
                                "kind": "error",
                                "message": "server emitted a bidi frame on a Subscribe stream",
                            });
                            if let Ok(json) = serde_json::to_vec(&v) {
                                let _ = tx.send(json);
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Cancel an in-flight subscription. Looks up the per-handle
/// registry; if the subscription_id is known, fires its
/// CancellationToken — the reader task observes the cancel via
/// tokio::select!, sends a `Cancel` frame to the daemon, and
/// exits. Idempotent: cancelling an unknown id returns OK without
/// reaching the wire.
///
/// # Safety
/// `handle` must be valid; `subscription_id` may refer to an
/// unknown subscription (idempotent).
#[no_mangle]
pub unsafe extern "C" fn easynet_subscription_cancel(
    handle: EasynetHandle,
    subscription_id: SubscriptionId,
) -> i32 {
    let session = match get(handle) {
        Some(s) => s,
        None => {
            set_last_error(format!(
                "easynet_subscription_cancel: handle {handle} is not registered"
            ));
            return ERR_INVALID_HANDLE;
        }
    };
    let token = {
        let mut g = session
            .subscriptions
            .lock()
            .expect("subscriptions registry lock");
        g.remove(&subscription_id)
    };
    if let Some(tok) = token {
        tok.cancel();
    }
    crate::ffi::errors::clear_last_error();
    crate::ffi::errors::EASYNET_OK
}

/// Marker — `IpcClient` is referenced by tests but unused in this
/// module's production path. Suppress the unused-import lint.
#[allow(dead_code)]
fn _mark_ipc_client_used() -> Option<&'static IpcClient> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::handle::{alloc, test_session};
    use std::ffi::CString;

    /// A handle to something alive in the registry. Tests that
    /// need "a valid handle" use this; tests exercising invalid
    /// handles pass literal `0` or a large unknown value.
    fn live_handle_no_client() -> EasynetHandle {
        // Test session has no IPC client. Real wire tests live in
        // src/ffi/client.rs (which has the server-harness setup);
        // here we exercise the validation paths only.
        let (h, _) = alloc(test_session());
        h
    }

    #[test]
    fn invoke_rejects_null_out_result_before_touching_handle() {
        // The null-check on out_result comes before the handle
        // lookup so a null-pointer error takes precedence over
        // an invalid-handle error. Pin this ordering: swapping
        // them would hide real null-pointer bugs behind "invalid
        // handle" errors.
        let ability = CString::new("observe.health").unwrap();
        let args = CString::new("{}").unwrap();
        let code = unsafe {
            easynet_ability_invoke(0, ability.as_ptr(), args.as_ptr(), std::ptr::null_mut())
        };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn invoke_with_invalid_handle_returns_invalid_handle_code() {
        let ability = CString::new("observe.health").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code =
            unsafe { easynet_ability_invoke(9_999_999, ability.as_ptr(), args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        // out_result must be set to NULL on the failure path so the
        // caller's pre-allocated `*mut c_char` does not carry a
        // stale pointer.
        assert!(out.is_null());
    }

    #[test]
    fn invoke_rejects_non_json_args() {
        // A handle with no IPC client gets us through the handle
        // validation and into the args-decode path. Pin: a Client
        // that passes a malformed args string sees ERR_NULL_POINTER
        // (the closest "your arg is wrong" code we have today),
        // never a panic.
        let h = live_handle_no_client();
        let ability = CString::new("observe.health").unwrap();
        // Trailing comma, not legal JSON.
        let bad_args = CString::new("{,}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code =
            unsafe { easynet_ability_invoke(h, ability.as_ptr(), bad_args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_NULL_POINTER);
        assert!(out.is_null());
    }

    #[test]
    fn invoke_on_test_session_without_client_returns_generic_error() {
        // A test-only session has `client: None`; reaching the IPC
        // dispatch branch must surface as a generic error with a
        // distinctive message, not a panic. This pins the safety
        // net for misuse from inside the crate's own tests.
        let h = live_handle_no_client();
        let ability = CString::new("observe.health").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { easynet_ability_invoke(h, ability.as_ptr(), args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_GENERIC);
    }

    #[test]
    fn subscribe_requires_frame_callback() {
        // A null callback would be impossible to deliver frames
        // through; reject at the ABI boundary rather than crashing
        // later when the first frame arrives.
        let h = live_handle_no_client();
        let ability = CString::new("device.session.attach").unwrap();
        let args = CString::new("{}").unwrap();
        let mut sub: SubscriptionId = 42;
        let code = unsafe {
            easynet_ability_subscribe(
                h,
                ability.as_ptr(),
                args.as_ptr(),
                None,
                std::ptr::null_mut(),
                &mut sub,
            )
        };
        assert_eq!(code, ERR_NULL_POINTER);
        assert_eq!(sub, 0, "out_subscription_id must be zeroed on error");
    }

    #[test]
    fn subscription_cancel_is_idempotent_for_unknown_id() {
        // v1 skeleton always returns OK for valid handles. Pin this
        // because the real implementation will preserve the same
        // idempotency — a Client re-sending a cancel must never
        // fail.
        let h = live_handle_no_client();
        let code = unsafe { easynet_subscription_cancel(h, 999_999) };
        assert_eq!(code, crate::ffi::errors::EASYNET_OK);
    }
}

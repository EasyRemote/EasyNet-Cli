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
//   (a) Adding `system.session.attach` doesn't bump the ABI.
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

use crate::ffi::errors::{
    set_last_error, ERR_ABILITY_FAILED, ERR_DAEMON_DOWN, ERR_GENERIC, ERR_INVALID_HANDLE,
    ERR_NOT_IMPLEMENTED, ERR_NULL_POINTER,
};
use crate::ffi::handle::{get, lib_runtime, EasynetHandle};
use crate::ffi::strings::{alloc_output_cstring, read_cstr};
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};

/// Subscription id type returned from `easynet_ability_subscribe`.
/// 0 means "no subscription allocated" (error path).
pub type SubscriptionId = u64;

/// Frame callback: invoked once per streaming `Frame` arriving
/// from the daemon. The `frame_json` pointer is borrowed — valid
/// only for the duration of the callback; the Client must copy it
/// if retention is needed.
///
/// `user_data` is opaque; the lib does not dereference it.
pub type FrameCallback =
    unsafe extern "C" fn(user_data: *mut c_void, frame_json: *const c_char);

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
        let mut client = client_mutex
            .lock()
            .expect("ipc client mutex not poisoned");
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
                set_last_error(
                    "easynet_ability_invoke: out-of-memory allocating result string",
                );
                return ERR_GENERIC;
            }
            unsafe { *out_result = ptr };
            crate::ffi::errors::clear_last_error();
            crate::ffi::errors::EASYNET_OK
        }
        OutgoingFrame::Error {
            code, message, ..
        } => {
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

/// Subscribe to a streaming ability. The `on_frame` callback is
/// invoked once per streaming frame delivered by the daemon.
/// Returns a non-zero subscription id the caller uses to cancel
/// via `easynet_subscription_cancel`.
///
/// v1 status: skeleton — returns `ERR_NOT_IMPLEMENTED` after
/// validating the inputs. The follow-up commit lands the per-
/// subscription reader task + frame channel + cancel semantics.
/// Validating inputs here means a Client misuse (null callback,
/// invalid handle) is surfaced today, instead of being shadowed by
/// the "not implemented" error.
///
/// # Safety
/// - `handle` must be a valid handle from a successful `easynet_init`.
/// - `ability` and `args_json` must be valid UTF-8 C strings.
/// - `on_frame` must not be null. It is invoked on the lib's
///   internal I/O thread; the Client is responsible for marshalling
///   back to its own thread if needed.
#[no_mangle]
pub unsafe extern "C" fn easynet_ability_subscribe(
    handle: EasynetHandle,
    ability: *const c_char,
    args_json: *const c_char,
    on_frame: Option<FrameCallback>,
    _user_data: *mut c_void,
    out_subscription_id: *mut SubscriptionId,
) -> i32 {
    if out_subscription_id.is_null() {
        set_last_error("easynet_ability_subscribe: out_subscription_id is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_subscription_id = 0 };

    if on_frame.is_none() {
        set_last_error("easynet_ability_subscribe: on_frame callback is null");
        return ERR_NULL_POINTER;
    }

    if get(handle).is_none() {
        set_last_error(format!(
            "easynet_ability_subscribe: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    let _ability = match read_cstr(ability) {
        Ok(s) => s,
        Err(_) => {
            set_last_error("easynet_ability_subscribe: ability name is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };
    let _args = match read_cstr(args_json) {
        Ok(s) => s,
        Err(_) => {
            set_last_error("easynet_ability_subscribe: args_json is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };

    set_last_error(
        "easynet_ability_subscribe is a skeleton in v1 of PR-DAEMON; \
         the streaming reader task lands in a follow-up commit",
    );
    ERR_NOT_IMPLEMENTED
}

/// Cancel an in-flight subscription. v1 skeleton: always returns
/// `EASYNET_OK` for a valid handle regardless of whether the
/// subscription is known. The follow-up commit looks up the
/// subscription registry and sends a `Cancel` frame, but keeps the
/// "unknown id is not an error" idempotency.
///
/// # Safety
/// `handle` must be valid; `subscription_id` may refer to an
/// unknown subscription (idempotent).
#[no_mangle]
pub unsafe extern "C" fn easynet_subscription_cancel(
    handle: EasynetHandle,
    _subscription_id: SubscriptionId,
) -> i32 {
    if get(handle).is_none() {
        set_last_error(format!(
            "easynet_subscription_cancel: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }
    crate::ffi::errors::clear_last_error();
    crate::ffi::errors::EASYNET_OK
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
        let ability = CString::new("system.ping").unwrap();
        let args = CString::new("{}").unwrap();
        let code = unsafe {
            easynet_ability_invoke(0, ability.as_ptr(), args.as_ptr(), std::ptr::null_mut())
        };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn invoke_with_invalid_handle_returns_invalid_handle_code() {
        let ability = CString::new("system.ping").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe {
            easynet_ability_invoke(9_999_999, ability.as_ptr(), args.as_ptr(), &mut out)
        };
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
        let ability = CString::new("system.ping").unwrap();
        // Trailing comma, not legal JSON.
        let bad_args = CString::new("{,}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe {
            easynet_ability_invoke(h, ability.as_ptr(), bad_args.as_ptr(), &mut out)
        };
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
        let ability = CString::new("system.ping").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code =
            unsafe { easynet_ability_invoke(h, ability.as_ptr(), args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_GENERIC);
    }

    #[test]
    fn subscribe_requires_frame_callback() {
        // A null callback would be impossible to deliver frames
        // through; reject at the ABI boundary rather than crashing
        // later when the first frame arrives.
        let h = live_handle_no_client();
        let ability = CString::new("system.session.attach").unwrap();
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

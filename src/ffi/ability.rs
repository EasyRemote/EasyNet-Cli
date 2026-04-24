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
// The shape "generic invoke + typed wrappers in Client-side code"
// is deliberate. Per-ability *convenience* functions (e.g. a
// Go `easynet_session_attach`) can be added on top, but every
// one of them is a thin wrapper over the generic helper — so the
// stability contract stays on the two functions in this file.
//
// v1 status — skeleton
// --------------------
// Both exported functions return `ERR_NOT_IMPLEMENTED` after
// setting a clear last-error message. Wiring them to the real
// `IpcClient` is the next PR-DAEMON commit. Shipping the symbols
// now means Client builds against `libeasynet_cli` link
// successfully — the functions just fail at runtime until the
// transport lands.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::os::raw::{c_char, c_void};

use crate::ffi::errors::{
    set_last_error, ERR_INVALID_HANDLE, ERR_NOT_IMPLEMENTED, ERR_NULL_POINTER,
};
use crate::ffi::handle::{get, EasynetHandle};
use crate::ffi::strings::read_cstr;

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

/// Invoke an ability synchronously. Writes the result JSON string
/// to `*out_result` (caller frees via `easynet_string_free`) and
/// returns `EASYNET_OK`. On failure returns a non-zero code and
/// `*out_result` is set to NULL.
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

    if get(handle).is_none() {
        set_last_error(format!(
            "easynet_ability_invoke: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    let _ability = match read_cstr(ability) {
        Ok(s) => s,
        Err(_) => {
            set_last_error("easynet_ability_invoke: ability name is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };
    let _args = match read_cstr(args_json) {
        Ok(s) => s,
        Err(_) => {
            set_last_error("easynet_ability_invoke: args_json is null or non-UTF-8");
            return ERR_NULL_POINTER;
        }
    };

    set_last_error(
        "easynet_ability_invoke is a skeleton in v1 of PR-DAEMON; \
         the follow-up commit wires it through IpcClient to the daemon",
    );
    ERR_NOT_IMPLEMENTED
}

/// Subscribe to a streaming ability. The `on_frame` callback is
/// invoked once per streaming frame delivered by the daemon.
/// Returns a non-zero subscription id the caller uses to cancel
/// via `easynet_subscription_cancel`.
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
         the follow-up commit wires the streaming path through IpcClient",
    );
    ERR_NOT_IMPLEMENTED
}

/// Cancel an in-flight subscription. v1 skeleton: always returns
/// `EASYNET_OK` regardless of whether the subscription is known;
/// the follow-up commit looks up the subscription registry and
/// sends a `Cancel` frame.
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
    fn live_handle() -> EasynetHandle {
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
    fn invoke_with_valid_handle_returns_not_implemented_in_v1_skeleton() {
        // The v1 skeleton surfaces a distinct error code for "wired
        // to the skeleton" vs "programmer error". This test pins
        // that distinction so the Client's error-handling code can
        // branch on `ERR_NOT_IMPLEMENTED` and tell a user "feature
        // not ready yet".
        let h = live_handle();
        let ability = CString::new("system.ping").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code =
            unsafe { easynet_ability_invoke(h, ability.as_ptr(), args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_NOT_IMPLEMENTED);
    }

    #[test]
    fn subscribe_requires_frame_callback() {
        // A null callback would be impossible to deliver frames
        // through; reject at the ABI boundary rather than crashing
        // later when the first frame arrives.
        let h = live_handle();
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
        let h = live_handle();
        let code = unsafe { easynet_subscription_cancel(h, 999_999) };
        assert_eq!(code, crate::ffi::errors::EASYNET_OK);
    }
}

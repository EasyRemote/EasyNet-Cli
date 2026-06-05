// EasyNet CLI — retired ability+args ABI
// ======================================
//
// File: src/ffi/ability.rs
// Description: Legacy ability+args C ABI symbols kept as explicit
//              rejection points while the product surface moves to
//              complete Axon Invocation.
//
// Boundary
// --------
// This module intentionally does not construct JSON control
// `IncomingFrame::{Invoke,Subscribe,Cancel}`. Public language
// bindings must use `ffi::invocation::easynet_invocation_invoke`
// for unary product calls, and future stream/cancel entry points
// must carry the complete Invocation tuple as well.
//
// What this module is NOT
// -----------------------
// - It is not a compatibility transport.
// - It is not allowed to fill `subject`, `nonce`, or
//   `causal_context` implicitly for callers.
// - It is not a place for control-plane fallback logic.

use std::os::raw::{c_char, c_void};

use crate::ffi::errors::{
    set_last_error, ERR_INVALID_HANDLE, ERR_NOT_IMPLEMENTED, ERR_NULL_POINTER,
};
use crate::ffi::handle::{get, EasynetHandle};

/// Subscription id type returned from the retired
/// `easynet_ability_subscribe` entry point.
///
/// A value of 0 means no subscription was allocated. New stream
/// APIs must define their own complete-Invocation handle type
/// rather than reusing JSON-control subscription ids.
pub type SubscriptionId = u64;

/// Frame callback shape used by the retired ability+args streaming
/// symbol. New stream APIs may reuse the callback ABI only if the
/// payload is explicitly documented as an Axon stream frame, not a
/// JSON-control `OutgoingFrame`.
pub type FrameCallback = unsafe extern "C" fn(user_data: *mut c_void, frame_json: *const c_char);

const LEGACY_INVOKE_REMOVED: &str =
    "easynet_ability_invoke has been removed; use easynet_invocation_invoke with a complete Invocation JSON object";
const LEGACY_SUBSCRIBE_REMOVED: &str =
    "easynet_ability_subscribe has been removed; use easynet_invocation_stream_open";
const LEGACY_CANCEL_REMOVED: &str =
    "easynet_subscription_cancel has been removed with the JSON-control subscription ABI";

/// Retired ability+args unary entry point.
///
/// This function validates only the ABI safety basics, zeros the
/// output pointer, and then returns `ERR_NOT_IMPLEMENTED`. It does
/// not parse ability args and does not reach the daemon.
///
/// # Safety
/// - `out_result` must be a non-null pointer owned by the caller.
/// - `handle` may be any value; unknown handles are rejected before
///   the retirement error is returned.
#[no_mangle]
pub unsafe extern "C" fn easynet_ability_invoke(
    handle: EasynetHandle,
    ability: *const c_char,
    args_json: *const c_char,
    out_result: *mut *mut c_char,
) -> i32 {
    let _ = (ability, args_json);
    if out_result.is_null() {
        set_last_error("easynet_ability_invoke: out_result pointer is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_result = std::ptr::null_mut() };

    if get(handle).is_none() {
        set_last_error(format!(
            "easynet_ability_invoke: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    set_last_error(LEGACY_INVOKE_REMOVED);
    ERR_NOT_IMPLEMENTED
}

/// Retired ability+args streaming entry point.
///
/// This function does not allocate a subscription id and does not
/// spawn any background task. Complete stream Invocation needs a
/// separate lifecycle handle because cancellation belongs to the
/// Axon stream, not to JSON control.
///
/// # Safety
/// - `out_subscription_id` must be non-null.
/// - `handle` may be any value; unknown handles are rejected before
///   the retirement error is returned.
#[no_mangle]
pub unsafe extern "C" fn easynet_ability_subscribe(
    handle: EasynetHandle,
    ability: *const c_char,
    args_json: *const c_char,
    on_frame: Option<FrameCallback>,
    user_data: *mut c_void,
    out_subscription_id: *mut SubscriptionId,
) -> i32 {
    let _ = (ability, args_json, on_frame, user_data);
    if out_subscription_id.is_null() {
        set_last_error("easynet_ability_subscribe: out_subscription_id is null");
        return ERR_NULL_POINTER;
    }
    unsafe { *out_subscription_id = 0 };

    if get(handle).is_none() {
        set_last_error(format!(
            "easynet_ability_subscribe: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    set_last_error(LEGACY_SUBSCRIBE_REMOVED);
    ERR_NOT_IMPLEMENTED
}

/// Retired JSON-control subscription cancellation entry point.
///
/// The function validates the handle and returns
/// `ERR_NOT_IMPLEMENTED`; there is no retired subscription registry to
/// inspect because `easynet_ability_subscribe` no longer starts a
/// JSON-control reader task.
///
/// # Safety
/// `handle` may be any value; the function does not dereference
/// caller memory.
#[no_mangle]
pub unsafe extern "C" fn easynet_subscription_cancel(
    handle: EasynetHandle,
    subscription_id: SubscriptionId,
) -> i32 {
    let _ = subscription_id;
    if get(handle).is_none() {
        set_last_error(format!(
            "easynet_subscription_cancel: handle {handle} is not registered"
        ));
        return ERR_INVALID_HANDLE;
    }

    set_last_error(LEGACY_CANCEL_REMOVED);
    ERR_NOT_IMPLEMENTED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::handle::{alloc, test_session};
    use std::ffi::CString;

    unsafe extern "C" fn ignore_frame(_: *mut c_void, _: *const c_char) {}

    fn live_handle_no_client() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    #[test]
    fn invoke_rejects_null_out_result_before_touching_handle() {
        let ability = CString::new("observe.health").unwrap();
        let args = CString::new("{}").unwrap();
        let code = unsafe {
            easynet_ability_invoke(0, ability.as_ptr(), args.as_ptr(), std::ptr::null_mut())
        };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn invoke_rejects_unknown_handle_after_zeroing_out_pointer() {
        let ability = CString::new("observe.health").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();
        let code =
            unsafe { easynet_ability_invoke(9_999_999, ability.as_ptr(), args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn invoke_legacy_entry_returns_not_implemented_for_live_handle() {
        let handle = live_handle_no_client();
        let ability = CString::new("observe.health").unwrap();
        let args = CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code =
            unsafe { easynet_ability_invoke(handle, ability.as_ptr(), args.as_ptr(), &mut out) };
        assert_eq!(code, ERR_NOT_IMPLEMENTED);
        assert!(out.is_null());
    }

    #[test]
    fn subscribe_rejects_null_out_subscription_before_touching_handle() {
        let ability = CString::new("device.session.attach").unwrap();
        let args = CString::new("{}").unwrap();
        let code = unsafe {
            easynet_ability_subscribe(
                0,
                ability.as_ptr(),
                args.as_ptr(),
                Some(ignore_frame),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn subscribe_legacy_entry_allocates_no_subscription() {
        let handle = live_handle_no_client();
        let ability = CString::new("device.session.attach").unwrap();
        let args = CString::new("{}").unwrap();
        let mut sub: SubscriptionId = 42;
        let code = unsafe {
            easynet_ability_subscribe(
                handle,
                ability.as_ptr(),
                args.as_ptr(),
                Some(ignore_frame),
                std::ptr::null_mut(),
                &mut sub,
            )
        };
        assert_eq!(code, ERR_NOT_IMPLEMENTED);
        assert_eq!(sub, 0);
    }

    #[test]
    fn cancel_rejects_unknown_handle() {
        let code = unsafe { easynet_subscription_cancel(9_999_999, 1) };
        assert_eq!(code, ERR_INVALID_HANDLE);
    }

    #[test]
    fn cancel_legacy_entry_returns_not_implemented_for_live_handle() {
        let handle = live_handle_no_client();
        let code = unsafe { easynet_subscription_cancel(handle, 1) };
        assert_eq!(code, ERR_NOT_IMPLEMENTED);
    }
}

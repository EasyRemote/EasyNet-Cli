// EasyNet CLI — Events C ABI projection
// ======================================
//
// File: src/ffi/events/mod.rs
// Description: C ABI EventClient projection helpers for daemon SDK directory
//              stream carriers and typed event frames.
//
// Protocol Responsibility
// -----------------------
// Expose Events DTO construction without letting language facades parse daemon
// `DirectoryEvent` variants independently. This module does not own stream
// dispatch, backend fanout, or retry policy.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and allocation mechanics at the exported
// boundary. Delegate carrier and frame semantics to
// `protocol::events_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Events profile projection. Runtime Core remains the only
// stream open/close path for returned Invocation carriers and stream chunks.

use std::os::raw::c_char;

use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};
use crate::protocol::events_contract::{
    build_device_event_history_invocation, build_device_subscription_invocation,
    build_directory_subscription_invocation, build_invocation_subscription_invocation,
    build_session_subscription_invocation, project_device_event_page, project_directory_event,
    project_drop_report, project_live_event, project_terminal, EventsError,
};

/// Build a complete Invocation JSON carrier for daemon
/// `federation.subscribe_directory_v2`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_build_directory_subscription_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_events_build_directory_subscription_invocation",
        "out_invocation_json",
        "request_json",
        build_directory_subscription_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon device events.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_build_device_subscription_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_events_build_device_subscription_invocation",
        "out_invocation_json",
        "request_json",
        build_device_subscription_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `session.attach`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_build_session_subscription_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_events_build_session_subscription_invocation",
        "out_invocation_json",
        "request_json",
        build_session_subscription_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon Invocation events.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_build_invocation_subscription_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_events_build_invocation_subscription_invocation",
        "out_invocation_json",
        "request_json",
        build_invocation_subscription_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon device event history.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_build_device_event_history_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_events_build_device_event_history_invocation",
        "out_invocation_json",
        "request_json",
        build_device_event_history_invocation,
    )
}

/// Project daemon device event history output into an SDK page DTO.
///
/// # Safety
/// `page_json` must be a valid UTF-8 C string and `out_page_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_project_device_event_page(
    handle: EasynetHandle,
    page_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        page_json,
        out_page_json,
        "easynet_events_project_device_event_page",
        "out_page_json",
        "page_json",
        project_device_event_page,
    )
}

/// Project a daemon `DirectoryEvent` frame plus explicit cursor into the SDK
/// EventFrame DTO.
///
/// # Safety
/// `event_json` must be a valid UTF-8 C string and `out_event_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_project_directory_event(
    handle: EasynetHandle,
    event_json: *const c_char,
    out_event_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        event_json,
        out_event_json,
        "easynet_events_project_directory_event",
        "out_event_json",
        "event_json",
        project_directory_event,
    )
}

/// Project a daemon raw live event frame plus explicit cursor into the SDK
/// EventFrame DTO.
///
/// # Safety
/// `event_json` must be a valid UTF-8 C string and `out_event_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_project_live_event(
    handle: EasynetHandle,
    event_json: *const c_char,
    out_event_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        event_json,
        out_event_json,
        "easynet_events_project_live_event",
        "out_event_json",
        "event_json",
        project_live_event,
    )
}

/// Project an explicit terminal frame for a directory event stream.
///
/// # Safety
/// `terminal_json` must be a valid UTF-8 C string and `out_event_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_project_terminal(
    handle: EasynetHandle,
    terminal_json: *const c_char,
    out_event_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        terminal_json,
        out_event_json,
        "easynet_events_project_terminal",
        "out_event_json",
        "terminal_json",
        project_terminal,
    )
}

/// Project an explicit dropped-event report for a directory event stream.
///
/// # Safety
/// `drop_json` must be a valid UTF-8 C string and `out_event_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_events_project_drop_report(
    handle: EasynetHandle,
    drop_json: *const c_char,
    out_event_json: *mut *mut c_char,
) -> i32 {
    project_events_json(
        handle,
        drop_json,
        out_event_json,
        "easynet_events_project_drop_report",
        "out_event_json",
        "drop_json",
        project_drop_report,
    )
}

fn project_events_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, EventsError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "events",
        },
        project,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
    use crate::ffi::errors::{EASYNET_OK, ERR_INVALID_ARG, ERR_INVALID_HANDLE};
    use serde_json::Value;
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

    fn nonce() -> &'static str {
        "AQIDBAUGBwgJCgsMDQ4PEA=="
    }

    fn base_request(extra: Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"},
        });
        let Value::Object(extra) = extra else {
            return CString::new(request.to_string()).unwrap();
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        CString::new(request.to_string()).unwrap()
    }

    #[test]
    fn events_build_directory_subscription_invocation_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "resume_cursor": "directory:7"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_events_build_directory_subscription_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "federation.subscribe_directory_v2"
        );
        assert_eq!(value["args"]["resume_cursor"], "directory:7");
        release(handle);
    }

    #[test]
    fn events_build_session_subscription_invocation_projects_attach_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "stream": "session",
            "session_id": "run-1",
            "resume_cursor": {"stream": "session", "sequence": 4}
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_events_build_session_subscription_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "session.attach");
        assert_eq!(value["args"]["session_id"], "run-1");
        assert_eq!(value["args"]["since_seq"], 4);
        release(handle);
    }

    #[test]
    fn events_build_device_subscription_invocation_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "stream": "device",
            "device_ura": "easynet:///r/example/device/dev-a",
            "resume_cursor": {"stream": "device", "sequence": 2}
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_events_build_device_subscription_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "events.device.subscribe"
        );
        assert_eq!(
            value["args"]["device_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(value["args"]["resume_cursor"], "device:2");
        release(handle);
    }

    #[test]
    fn events_build_invocation_subscription_invocation_projects_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "stream": "invocation",
            "invocation_id": "inv-1"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_events_build_invocation_subscription_invocation(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(
            value["metadata"]["system_ability"],
            "events.invocation.subscribe"
        );
        assert_eq!(value["args"]["invocation_id"], "inv-1");
        release(handle);
    }

    #[test]
    fn events_project_device_event_page_projects_history() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "limit": 1,
                "result": {
                    "events": [
                        {
                            "sequence": 8,
                            "device_ura": "easynet:///r/example/device/dev-a",
                            "occurred_unix_ms": 1783100000123i64,
                            "kind": "device.status_changed"
                        },
                        {
                            "sequence": 9,
                            "device_ura": "easynet:///r/example/device/dev-a",
                            "occurred_unix_ms": 1783100001123i64
                        }
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_events_project_device_event_page(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["stream"], "device");
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["next_cursor"], "device:1");
        release(handle);
    }

    #[test]
    fn events_project_directory_event_projects_sdk_frame() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "cursor": {"stream": "directory", "sequence": 8},
                "event": {
                    "type": "agent_revoked",
                    "agent_ura": "easynet:///r/example/agent/alice.main",
                    "was_active": true,
                    "reason": "stream_closed",
                    "unix_ms": 1783100000123i64
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_events_project_directory_event(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "directory.agent_revoked");
        assert_eq!(value["cursor"]["token"], "directory:8");
        assert_eq!(value["subject_ref"]["role"], "agent");
        release(handle);
    }

    #[test]
    fn events_project_live_event_projects_device_and_invocation_frames() {
        let handle = handle();
        let raw_device = CString::new(
            serde_json::json!({
                "cursor": {"stream": "device", "sequence": 8},
                "event": {
                    "sequence": 8,
                    "device_ura": "easynet:///r/example/device/dev-a",
                    "occurred_unix_ms": 1783100000123i64,
                    "kind": "device.status_changed",
                    "payload": {"state": "online"}
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out_device: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_events_project_live_event(handle, raw_device.as_ptr(), &mut out_device)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out_device);
        assert_eq!(value["stream"], "device");
        assert_eq!(value["kind"], "device.status_changed");
        assert_eq!(
            value["metadata"]["stream_ability"],
            "events.device.subscribe"
        );

        let raw_invocation = CString::new(
            serde_json::json!({
                "cursor": {"stream": "invocation", "sequence": 4},
                "event": {
                    "sequence": 4,
                    "invocation_id": "inv-1",
                    "occurred_unix_ms": 1783100001123i64,
                    "kind": "invocation.completed",
                    "payload": {"terminal_state": "Completed"}
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out_invocation: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_events_project_live_event(handle, raw_invocation.as_ptr(), &mut out_invocation)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out_invocation);
        assert_eq!(value["stream"], "invocation");
        assert_eq!(value["subject_ref"]["kind"], "invocation");
        assert_eq!(value["subject_ref"]["invocation_id"], "inv-1");
        release(handle);
    }

    #[test]
    fn events_project_drop_report_rejects_zero_dropped_count() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "cursor": "directory:9",
                "occurred_unix_ms": 1783100000123i64,
                "dropped_count": 0
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_events_project_drop_report(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn events_projection_rejects_invalid_handle_after_zeroing_output() {
        let raw = base_request(serde_json::json!({}));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_events_build_directory_subscription_invocation(
                9_999_999,
                raw.as_ptr(),
                &mut out,
            )
        };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

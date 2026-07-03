// EasyNet CLI — Convenience Wrapper C ABI projection
// ==================================================
//
// File: src/ffi/wrappers/mod.rs
// Description: C ABI Convenience Wrapper projection helpers for SDK wrapper
//              records.
//
// Protocol Responsibility
// -----------------------
// Expose wrapper record DTO construction without letting language facades own
// file/session/media record semantics. This module does not start sessions,
// open product WebSockets, execute abilities, or own Runtime Core stream/bidi
// terminal state.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at the exported
// boundary. Delegate wrapper record validation and normalization to
// `daemon::wrapper_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Convenience Wrapper profile projection. Runtime Core remains
// the only submit/observe path for future wrapper execution helpers.

use std::os::raw::c_char;

use crate::daemon::wrapper_contract::{
    project_browser_session, project_file_record, project_media_session,
    project_remote_desktop_session, project_terminal_session, WrapperError,
};
use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};

/// Project daemon/resource file facts into a wrapper FileRecord DTO.
///
/// # Safety
/// `file_json` must be a valid UTF-8 C string and `out_file_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_wrappers_project_file_record(
    handle: EasynetHandle,
    file_json: *const c_char,
    out_file_json: *mut *mut c_char,
) -> i32 {
    project_wrappers_json(
        handle,
        file_json,
        out_file_json,
        "easynet_wrappers_project_file_record",
        "out_file_json",
        "file_json",
        project_file_record,
    )
}

/// Project daemon terminal session facts into a TerminalSessionRecord DTO.
///
/// # Safety
/// `session_json` must be a valid UTF-8 C string and `out_session_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_wrappers_project_terminal_session(
    handle: EasynetHandle,
    session_json: *const c_char,
    out_session_json: *mut *mut c_char,
) -> i32 {
    project_wrappers_json(
        handle,
        session_json,
        out_session_json,
        "easynet_wrappers_project_terminal_session",
        "out_session_json",
        "session_json",
        project_terminal_session,
    )
}

/// Project daemon remote desktop session facts into a
/// RemoteDesktopSessionRecord DTO.
///
/// # Safety
/// `session_json` must be a valid UTF-8 C string and `out_session_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_wrappers_project_remote_desktop_session(
    handle: EasynetHandle,
    session_json: *const c_char,
    out_session_json: *mut *mut c_char,
) -> i32 {
    project_wrappers_json(
        handle,
        session_json,
        out_session_json,
        "easynet_wrappers_project_remote_desktop_session",
        "out_session_json",
        "session_json",
        project_remote_desktop_session,
    )
}

/// Project daemon browser session facts into a BrowserSessionRecord DTO.
///
/// # Safety
/// `session_json` must be a valid UTF-8 C string and `out_session_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_wrappers_project_browser_session(
    handle: EasynetHandle,
    session_json: *const c_char,
    out_session_json: *mut *mut c_char,
) -> i32 {
    project_wrappers_json(
        handle,
        session_json,
        out_session_json,
        "easynet_wrappers_project_browser_session",
        "out_session_json",
        "session_json",
        project_browser_session,
    )
}

/// Project daemon media session facts into a MediaSessionRecord DTO.
///
/// # Safety
/// `session_json` must be a valid UTF-8 C string and `out_session_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_wrappers_project_media_session(
    handle: EasynetHandle,
    session_json: *const c_char,
    out_session_json: *mut *mut c_char,
) -> i32 {
    project_wrappers_json(
        handle,
        session_json,
        out_session_json,
        "easynet_wrappers_project_media_session",
        "out_session_json",
        "session_json",
        project_media_session,
    )
}

fn project_wrappers_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, WrapperError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "wrappers",
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

    #[test]
    fn wrappers_project_file_record_projects_resource_facts() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "file_ref": "easynet:///r/example/resource/alice.files/report.txt",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "content_type": "text/plain",
                "size_bytes": 42
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_wrappers_project_file_record(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["profile"], "wrappers");
        assert_eq!(value["kind"], "file_record");
        assert_eq!(value["size_bytes"], 42);
        release(handle);
    }

    #[test]
    fn wrappers_project_terminal_session_rejects_missing_state_after_zeroing_output() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "session_id": "term-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_wrappers_project_terminal_session(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn wrappers_project_remote_desktop_session_projects_display_ref() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "session_id": "rdp-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "active",
                "display_ref": "display-main"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            easynet_wrappers_project_remote_desktop_session(handle, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "remote_desktop_session");
        assert_eq!(value["display_ref"], "display-main");
        release(handle);
    }

    #[test]
    fn wrappers_project_browser_session_rejects_invalid_handle_after_zeroing_output() {
        let raw = CString::new(
            serde_json::json!({
                "session_id": "browser-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "active"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_wrappers_project_browser_session(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn wrappers_project_media_session_projects_media_kind() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "session_id": "media-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "active",
                "media_kind": "voice",
                "stream_ref": "stream-voice-1"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_wrappers_project_media_session(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "media_session");
        assert_eq!(value["media_kind"], "voice");
        release(handle);
    }
}

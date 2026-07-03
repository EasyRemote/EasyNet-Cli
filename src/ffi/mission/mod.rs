// EasyNet CLI — Mission C ABI projection
// =======================================
//
// File: src/ffi/mission/mod.rs
// Description: C ABI MissionClient projection helpers for daemon SDK
//              Mission/EAL carriers and typed status DTOs.
//
// Protocol Responsibility
// -----------------------
// Expose Mission/EAL submission carriers and status projection without letting
// language facades own daemon transport, mission run directory parsing, or
// child receipt interpretation.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at this exported
// boundary. Delegate Mission carrier and status semantics to
// `daemon::mission_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Mission profile projection. Runtime Core remains the only
// submit/observe path for returned Invocation carriers.

use std::os::raw::c_char;

use crate::daemon::mission_contract::{
    build_cancel_invocation, build_run_eal_invocation, build_run_file_invocation,
    build_track_invocation, project_status, MissionError,
};
use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};

/// Build a complete Invocation JSON carrier for daemon `mission.run` from EAL
/// source text.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_mission_build_run_eal_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_mission_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_mission_build_run_eal_invocation",
        "out_invocation_json",
        "request_json",
        build_run_eal_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `mission.run` by
/// reading an absolute local EAL source file.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_mission_build_run_file_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_mission_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_mission_build_run_file_invocation",
        "out_invocation_json",
        "request_json",
        build_run_file_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `mission.track`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_mission_build_track_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_mission_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_mission_build_track_invocation",
        "out_invocation_json",
        "request_json",
        build_track_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `mission.cancel`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_mission_build_cancel_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_mission_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_mission_build_cancel_invocation",
        "out_invocation_json",
        "request_json",
        build_cancel_invocation,
    )
}

/// Project daemon `mission.run`, `mission.track`, or `mission.cancel` JSON into
/// the SDK MissionStatus DTO.
///
/// # Safety
/// `status_json` must be a valid UTF-8 C string and `out_status_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_mission_project_status(
    handle: EasynetHandle,
    status_json: *const c_char,
    out_status_json: *mut *mut c_char,
) -> i32 {
    project_mission_json(
        handle,
        status_json,
        out_status_json,
        "easynet_mission_project_status",
        "out_status_json",
        "status_json",
        project_status,
    )
}

fn project_mission_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, MissionError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "mission",
        },
        project,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::client::handle::{alloc, release, test_session};
    use crate::ffi::errors::{EASYNET_OK, ERR_INVALID_ARG, ERR_INVALID_HANDLE};
    use std::ffi::{CStr, CString};
    use std::io::Write;

    fn handle() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    fn read_json(ptr: *mut c_char) -> serde_json::Value {
        let value = unsafe { serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(ptr) };
        value
    }

    fn nonce() -> &'static str {
        "AQIDBAUGBwgJCgsMDQ4PEA=="
    }

    fn base_request(extra: serde_json::Value) -> CString {
        let mut obj = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"}
        });
        obj.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        CString::new(obj.to_string()).unwrap()
    }

    #[test]
    fn mission_build_run_eal_invocation_projects_complete_tuple() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "source": "mission demo",
            "label": "demo"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_mission_build_run_eal_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "mission.run");
        assert_eq!(value["args"]["source"], "mission demo");
        release(handle);
    }

    #[test]
    fn mission_build_run_file_invocation_reads_source_file() {
        let handle = handle();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flow.eal");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "mission demo").unwrap();
        let raw = base_request(serde_json::json!({
            "path": path.display().to_string()
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_mission_build_run_file_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert!(value["args"]["source"]
            .as_str()
            .unwrap()
            .contains("mission demo"));
        release(handle);
    }

    #[test]
    fn mission_project_status_projects_child_receipts() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "run_id": "2026-07-04_010203_demo",
                "run_dir": "/tmp/easynet/missions/runs/2026-07-04_010203_demo",
                "running": false,
                "meta": {
                    "trace_id": "2026-07-04_010203_demo",
                    "status": "ok",
                    "steps_failed": 0,
                    "ability_graph_traces": [{
                        "step_id": "s1",
                        "ability": "observe.health",
                        "invocation_ura": "easynet:///r/example/invocation/req-1",
                        "receipt": {
                            "anchor": {
                                "receipt_ura": "easynet:///r/example/receipt/child",
                                "receipt_hash": "ab"
                            }
                        }
                    }]
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_mission_project_status(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["state"], "ok");
        assert_eq!(value["terminal"], true);
        assert_eq!(
            value["child_receipts"][0]["receipt_ura"],
            "easynet:///r/example/receipt/child"
        );
        release(handle);
    }

    #[test]
    fn mission_build_cancel_invocation_rejects_path_like_mission_id() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "mission_id": "../bad"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_mission_build_cancel_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn mission_projection_rejects_invalid_handle_after_zeroing_output() {
        let raw = base_request(serde_json::json!({
            "source": "mission demo"
        }));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_mission_build_run_eal_invocation(9_999_999, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }
}

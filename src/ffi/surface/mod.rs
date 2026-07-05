// EasyNet CLI — Surface C ABI projection
// =======================================
//
// File: src/ffi/surface/mod.rs
// Description: C ABI SurfaceClient helpers for daemon SDK page carriers and
//              page DTO projections.
//
// Protocol Responsibility
// -----------------------
// Expose Surface DTO construction without letting language facades hand-build
// page system-ability Invocation carriers, page records, manifests, or public
// refs. Backend rendering, auth, CDN, and browser route policy stay outside
// this ABI surface.
//
// Implementation Approach
// -----------------------
// Keep pointer, handle, UTF-8, JSON, and string allocation at the exported
// boundary. Delegate Surface carrier and projection semantics to
// `protocol::surface_contract`.
//
// Usage Contract
// --------------
// Callers must pass a live `EasynetHandle`, valid UTF-8 JSON, and a non-null
// output pointer. Returned strings are caller-owned and freed through
// `easynet_string_free`.
//
// Architectural Position
// ----------------------
// EasyNet-Cli SDK Surface profile projection. Runtime Core remains the submit
// path for returned Invocation carriers; EasyNet backend owns public HTTP
// rendering and presentation.

use std::os::raw::c_char;

use crate::ffi::client::handle::EasynetHandle;
use crate::ffi::profile_json::{project_profile_json, ProfileJsonSpec};
use crate::protocol::surface_contract::{
    build_create_page_invocation, build_delete_page_invocation, build_health_invocation,
    build_list_pages_invocation, build_manifest_invocation, project_mutation_result,
    project_page_page, project_page_record, project_public_page_ref, project_surface_health,
    project_surface_manifest, SurfaceError,
};

/// Build a complete Invocation JSON carrier for daemon `pages.list`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_build_list_pages_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_surface_build_list_pages_invocation",
        "out_invocation_json",
        "request_json",
        build_list_pages_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `pages.publish`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_build_create_page_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_surface_build_create_page_invocation",
        "out_invocation_json",
        "request_json",
        build_create_page_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `pages.unpublish`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_build_delete_page_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_surface_build_delete_page_invocation",
        "out_invocation_json",
        "request_json",
        build_delete_page_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `pages.get`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_build_manifest_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_surface_build_manifest_invocation",
        "out_invocation_json",
        "request_json",
        build_manifest_invocation,
    )
}

/// Build a complete Invocation JSON carrier for daemon `pages.health`.
///
/// # Safety
/// `request_json` must be a valid UTF-8 C string and `out_invocation_json`
/// must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_build_health_invocation(
    handle: EasynetHandle,
    request_json: *const c_char,
    out_invocation_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        request_json,
        out_invocation_json,
        "easynet_surface_build_health_invocation",
        "out_invocation_json",
        "request_json",
        build_health_invocation,
    )
}

/// Project one daemon page fact object into a Surface PageRecord DTO.
///
/// # Safety
/// `page_json` must be a valid UTF-8 C string and `out_page_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_project_page_record(
    handle: EasynetHandle,
    page_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        page_json,
        out_page_json,
        "easynet_surface_project_page_record",
        "out_page_json",
        "page_json",
        project_page_record,
    )
}

/// Project daemon `pages.list` output into a bounded Surface page DTO.
///
/// # Safety
/// `pages_json` must be a valid UTF-8 C string and `out_page_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_project_page_page(
    handle: EasynetHandle,
    pages_json: *const c_char,
    out_page_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        pages_json,
        out_page_json,
        "easynet_surface_project_page_page",
        "out_page_json",
        "pages_json",
        project_page_page,
    )
}

/// Project daemon `pages.get` output into a SurfaceManifest DTO.
///
/// # Safety
/// `page_json` must be a valid UTF-8 C string and `out_manifest_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_project_manifest(
    handle: EasynetHandle,
    page_json: *const c_char,
    out_manifest_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        page_json,
        out_manifest_json,
        "easynet_surface_project_manifest",
        "out_manifest_json",
        "page_json",
        project_surface_manifest,
    )
}

/// Project explicit page facts into a PublicPageRef DTO.
///
/// # Safety
/// `page_json` must be a valid UTF-8 C string and `out_ref_json` must be a
/// non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_project_public_page_ref(
    handle: EasynetHandle,
    page_json: *const c_char,
    out_ref_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        page_json,
        out_ref_json,
        "easynet_surface_project_public_page_ref",
        "out_ref_json",
        "page_json",
        project_public_page_ref,
    )
}

/// Project daemon `pages.unpublish` output into a mutation result DTO.
///
/// # Safety
/// `result_json` must be a valid UTF-8 C string and `out_result_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_project_mutation_result(
    handle: EasynetHandle,
    result_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        result_json,
        out_result_json,
        "easynet_surface_project_mutation_result",
        "out_result_json",
        "result_json",
        project_mutation_result,
    )
}

/// Project daemon `pages.health` output into a SurfaceHealth DTO.
///
/// # Safety
/// `health_json` must be a valid UTF-8 C string and `out_health_json` must be
/// a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_surface_project_health(
    handle: EasynetHandle,
    health_json: *const c_char,
    out_health_json: *mut *mut c_char,
) -> i32 {
    project_surface_json(
        handle,
        health_json,
        out_health_json,
        "easynet_surface_project_health",
        "out_health_json",
        "health_json",
        project_surface_health,
    )
}

fn project_surface_json(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    function: &'static str,
    output_name: &'static str,
    input_name: &'static str,
    project: fn(&serde_json::Value) -> Result<serde_json::Value, SurfaceError>,
) -> i32 {
    project_profile_json(
        handle,
        input,
        output,
        ProfileJsonSpec {
            function,
            output_name,
            input_name,
            profile: "surface",
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

    fn handle() -> EasynetHandle {
        let (handle, _) = alloc(test_session());
        handle
    }

    fn read_json(ptr: *mut c_char) -> serde_json::Value {
        let value = unsafe { serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(ptr) };
        value
    }

    fn base_request(extra: serde_json::Value) -> CString {
        let mut request = serde_json::json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/agent/alice.pages",
            "subject_ura": "easynet:///r/example/agent/alice.pages",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "surface-ffi-1"}
        });
        let serde_json::Value::Object(extra) = extra else {
            return CString::new(request.to_string()).unwrap();
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        CString::new(request.to_string()).unwrap()
    }

    fn page_fact() -> CString {
        CString::new(
            serde_json::json!({
                "user": "alice",
                "project_id": "docs",
                "project_ura": "easynet:///r/example/resource/alice.docs",
                "url_root": "https://example/web/alice/docs/",
                "visibility": "public"
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn surface_build_create_page_projects_pages_publish_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "project_id": "docs",
            "folder": "/tmp/docs",
            "visibility": "public"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_surface_build_create_page_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "pages.publish");
        assert_eq!(value["args"]["project_id"], "docs");
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0"
        );
        release(handle);
    }

    #[test]
    fn surface_build_health_projects_pages_health_carrier() {
        let handle = handle();
        let raw = base_request(serde_json::json!({
            "surface_ref": "easynet:///r/example/resource/alice.docs"
        }));
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_surface_build_health_invocation(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["metadata"]["system_ability"], "pages.health");
        assert_eq!(
            value["descriptor_ref"],
            "easynet:///r/example/ability/alice.pages.pages.health@1.0.0"
        );
        assert_eq!(
            value["args"]["surface_ref"],
            "easynet:///r/example/resource/alice.docs"
        );
        release(handle);
    }

    #[test]
    fn surface_build_delete_rejects_invalid_handle_after_zeroing_output() {
        let raw = base_request(serde_json::json!({"project_id": "docs"}));
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code = unsafe {
            easynet_surface_build_delete_page_invocation(9_999_999, raw.as_ptr(), &mut out)
        };

        assert_eq!(code, ERR_INVALID_HANDLE);
        assert!(out.is_null());
    }

    #[test]
    fn surface_project_page_record_projects_refs() {
        let handle = handle();
        let raw = page_fact();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_surface_project_page_record(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["page_id"], "docs");
        assert_eq!(value["owner_ura"], "easynet:///r/example/agent/alice.pages");
        assert_eq!(value["public_ref"], "https://example/web/alice/docs/");
        release(handle);
    }

    #[test]
    fn surface_project_page_page_projects_bounded_items() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "owner_ura": "easynet:///r/example/agent/alice.pages",
                "realm": "example",
                "limit": 1,
                "result": {
                    "projects": [
                        {"user": "alice", "project_id": "docs", "url_root": "https://example/web/alice/docs/"},
                        {"user": "alice", "project_id": "blog", "url_root": "https://example/web/alice/blog/"}
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_surface_project_page_page(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["next_cursor"], "1");
        release(handle);
    }

    #[test]
    fn surface_project_public_page_ref_requires_public_ref() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "user": "alice",
                "project_id": "docs",
                "project_ura": "easynet:///r/example/resource/alice.docs"
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::dangling_mut();

        let code =
            unsafe { easynet_surface_project_public_page_ref(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, ERR_INVALID_ARG);
        assert!(out.is_null());
        release(handle);
    }

    #[test]
    fn surface_project_manifest_wraps_entrypoint() {
        let handle = handle();
        let raw = page_fact();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_surface_project_manifest(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "surface_manifest");
        assert_eq!(
            value["entrypoint"]["href"],
            "https://example/web/alice/docs/"
        );
        release(handle);
    }

    #[test]
    fn surface_project_mutation_result_projects_delete_state() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "user": "alice",
                "project_id": "docs",
                "removed": true
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { easynet_surface_project_mutation_result(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["state"], "deleted");
        release(handle);
    }

    #[test]
    fn surface_project_health_projects_ready_status() {
        let handle = handle();
        let raw = CString::new(
            serde_json::json!({
                "callee_ura": "easynet:///r/example/agent/alice.pages",
                "descriptor_version": "1.0.0",
                "surface_ref": "easynet:///r/example/resource/alice.docs",
                "result": {
                    "state": "ready",
                    "ready": true,
                    "owner_ura": "easynet:///r/example/agent/alice.pages",
                    "page_count": 1,
                    "checks": [
                        {"name": "manifest", "state": "ready", "ready": true, "latency_ms": 3}
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let code = unsafe { easynet_surface_project_health(handle, raw.as_ptr(), &mut out) };

        assert_eq!(code, EASYNET_OK);
        let value = read_json(out);
        assert_eq!(value["kind"], "surface_health");
        assert_eq!(value["ready"], true);
        assert_eq!(value["checks"][0]["name"], "manifest");
        release(handle);
    }
}

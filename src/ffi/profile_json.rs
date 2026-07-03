// EasyNet CLI — FFI profile JSON projection helper
// =================================================
//
// File: src/ffi/profile_json.rs
// Description: Shared C ABI boundary helper for daemon SDK profile DTOs.
//
// Protocol Responsibility
// -----------------------
// Enforce the binding-facing C ABI preconditions common to pure JSON profile
// projections: live `EasynetHandle`, valid input C string, non-null output
// pointer, and caller-owned output allocation.
//
// Implementation Approach
// -----------------------
// Keep pointer and JSON mechanics here, then delegate profile semantics to a
// small `fn(&Value) -> Result<Value, E>`. This prevents profile modules from
// duplicating C boundary checks while keeping domain validation outside FFI.
//
// Usage Contract
// --------------
// Exported profile functions must pass their exact public symbol name and
// parameter labels so `easynet_last_error_json` remains actionable for
// language bindings.
//
// Architectural Position
// ----------------------
// C ABI adapter layer. Daemon SDK contract modules own profile DTO semantics;
// this helper owns only the C-compatible transport envelope around those DTOs.

use std::fmt;
use std::os::raw::c_char;

use serde_json::Value;

use crate::ffi::client::handle::{get, EasynetHandle};
use crate::ffi::errors::{
    clear_last_error, set_last_error_code, EASYNET_OK, ERR_GENERIC, ERR_INVALID_ARG,
    ERR_INVALID_HANDLE, ERR_INVALID_UTF8, ERR_NULL_POINTER,
};
use crate::ffi::strings::{alloc_output_cstring, read_cstr, StringError};

pub(crate) fn project_profile_json<E>(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    spec: ProfileJsonSpec,
    project: fn(&Value) -> Result<Value, E>,
) -> i32
where
    E: fmt::Display,
{
    let raw = match read_profile_args(handle, input, output, spec) {
        Ok(raw) => raw,
        Err(code) => return code,
    };
    let input = match parse_json_value(raw, spec.function, spec.input_name) {
        Ok(value) => value,
        Err(code) => return code,
    };
    match project(&input) {
        Ok(value) => write_json_output(spec.function, spec.profile, output, value),
        Err(err) => {
            set_last_error_code(ERR_INVALID_ARG, format!("{}: {err}", spec.function));
            ERR_INVALID_ARG
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ProfileJsonSpec {
    pub(crate) function: &'static str,
    pub(crate) output_name: &'static str,
    pub(crate) input_name: &'static str,
    pub(crate) profile: &'static str,
}

fn read_profile_args<'a>(
    handle: EasynetHandle,
    input: *const c_char,
    output: *mut *mut c_char,
    spec: ProfileJsonSpec,
) -> Result<&'a str, i32> {
    if output.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            format!("{}: {} pointer is null", spec.function, spec.output_name),
        );
        return Err(ERR_NULL_POINTER);
    }
    unsafe { *output = std::ptr::null_mut() };

    if get(handle).is_none() {
        set_last_error_code(
            ERR_INVALID_HANDLE,
            format!("{}: handle {handle} is not registered", spec.function),
        );
        return Err(ERR_INVALID_HANDLE);
    }

    match read_cstr(input) {
        Ok(raw) => Ok(raw),
        Err(StringError::Null) => {
            set_last_error_code(
                ERR_NULL_POINTER,
                format!("{}: {} pointer is null", spec.function, spec.input_name),
            );
            Err(ERR_NULL_POINTER)
        }
        Err(StringError::NotUtf8) => {
            set_last_error_code(
                ERR_INVALID_UTF8,
                format!("{}: {} is not valid UTF-8", spec.function, spec.input_name),
            );
            Err(ERR_INVALID_UTF8)
        }
    }
}

fn parse_json_value(
    raw: &str,
    function: &'static str,
    input_name: &'static str,
) -> Result<Value, i32> {
    serde_json::from_str(raw).map_err(|err| {
        set_last_error_code(
            ERR_INVALID_ARG,
            format!("{function}: decode {input_name} failed: {err}"),
        );
        ERR_INVALID_ARG
    })
}

fn write_json_output(
    function: &'static str,
    profile: &'static str,
    output: *mut *mut c_char,
    value: Value,
) -> i32 {
    let ptr = alloc_output_cstring(value.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating {profile} JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *output = ptr };
    clear_last_error();
    EASYNET_OK
}

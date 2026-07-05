// EasyNet CLI — FFI error codes + last-error TLS
// ================================================
//
// File: src/ffi/errors.rs
// Description: Integer error codes returned across the C ABI,
//              plus a thread-local "last error message" buffer the
//              caller queries via `easynet_last_error()` when an
//              exported function returns a non-zero code.
//
// Why an i32 + TLS, not exceptions / Result
// -----------------------------------------
// C ABI cannot carry Rust panics or `Result`s. The lingua franca
// between Go / Python / Node / Swift / Java bindings is: integer
// return code → "zero means success, anything else means look at
// the last-error buffer". This file owns both sides of that
// contract.
//
// Thread-local rather than handle-local: the last-error read must
// succeed even when the error happened *before* a handle was
// obtained (e.g. `easynet_init` failed), so a per-thread slot is
// the right granularity. Every exported function that can fail
// writes its error message here before returning a non-zero code.
//
// Stability
// ---------
// Error codes are part of the ABI. Renaming or renumbering any
// `pub const ERR_*` requires a `EASYNET_ABI_VERSION` bump.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Success. Exported function completed without error.
pub const EASYNET_OK: i32 = 0;

/// Generic / unclassified error. Prefer a more specific code when
/// possible; this is the catch-all for programmer error paths.
pub const ERR_GENERIC: i32 = 1;

/// A required pointer argument was null.
pub const ERR_NULL_POINTER: i32 = 2;

/// A string argument was not valid UTF-8.
pub const ERR_INVALID_UTF8: i32 = 3;

/// The Client passed a handle that was never issued or has been
/// freed. Callers should treat this as a programming error, not a
/// retriable condition.
pub const ERR_INVALID_HANDLE: i32 = 4;

/// The library has not been initialised (or was shut down). Call
/// `easynet_init()` first.
pub const ERR_NOT_INITIALIZED: i32 = 5;

/// The library was initialised twice from the same process. Use
/// the existing handle.
pub const ERR_ALREADY_INIT: i32 = 6;

/// The daemon's `control.json` / socket could not be reached.
pub const ERR_DAEMON_DOWN: i32 = 7;

/// IPC version negotiation failed (daemon and lib ranges do not
/// overlap).
pub const ERR_VERSION_INCOMPATIBLE: i32 = 8;

/// The ability call returned an error; the message carries the
/// daemon-side reason verbatim.
pub const ERR_ABILITY_FAILED: i32 = 9;

/// A symbol is intentionally unavailable in this build or has been
/// retired from the current ABI surface. Client bindings should treat
/// this as a non-retriable capability mismatch, not daemon downtime.
pub const ERR_NOT_IMPLEMENTED: i32 = 10;

/// The caller supplied a syntactically valid ABI value whose domain
/// contents are invalid: malformed JSON, missing required fields,
/// invalid URA shape, invalid base64, or daemon-side
/// `InvalidArgument`.
pub const ERR_INVALID_ARG: i32 = 11;

/// The daemon rejected the call because the caller is not authorised
/// for the requested Invocation.
pub const ERR_PERMISSION_DENIED: i32 = 12;

/// The requested ability, subject, stream, or resource was not found.
pub const ERR_NOT_FOUND: i32 = 13;

/// The operation was cancelled or the local stream/session was
/// already closed.
pub const ERR_CANCELLED: i32 = 14;

/// The daemon returned a malformed or semantically impossible
/// protocol response.
pub const ERR_PROTOCOL: i32 = 15;

/// The operation exceeded its deadline.
pub const ERR_TIMEOUT: i32 = 16;

thread_local! {
    /// Per-thread last-error message. Rust storage owns the
    /// `CString`; the pointer returned by `easynet_last_error()` is
    /// borrowed from this storage and is only valid until the next
    /// call on the same thread that writes a new error.
    static LAST_ERROR: RefCell<Option<LastErrorRecord>> = const { RefCell::new(None) };
}

struct LastErrorRecord {
    message: CString,
    code: Option<i32>,
}

/// Record an error message for later retrieval by
/// `easynet_last_error`. Called internally from exported functions
/// immediately before returning a non-zero code.
#[cfg(test)]
pub(crate) fn set_last_error(msg: impl Into<String>) {
    set_last_error_record(None, msg);
}

/// Record an error code plus message for typed error projection.
/// Existing C ABI callers still branch on the returned integer; this
/// helper lets newer bindings also retrieve a schema-backed JSON DTO
/// without parsing the human-readable last-error string.
pub(crate) fn set_last_error_code(code: i32, msg: impl Into<String>) {
    set_last_error_record(Some(code), msg);
}

fn set_last_error_record(code: Option<i32>, msg: impl Into<String>) {
    let s = msg.into();
    // Strip any interior NULs so the CString construction cannot
    // fail on well-formed Rust strings that happen to contain a \0.
    let sanitized: String = s.chars().filter(|c| *c != '\0').collect();
    let c = CString::new(sanitized).unwrap_or_else(|_| CString::new("(unrepresentable)").unwrap());
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(LastErrorRecord { message: c, code });
    });
}

/// Clear the per-thread last-error slot. Exported functions call
/// this on the success path so a Client that reuses the same thread
/// cannot accidentally read a stale message.
pub(crate) fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// Return a borrowed pointer to the thread-local last-error
/// message. The pointer is valid until the next error-recording
/// call on the same thread. Returns NULL when no error is recorded.
///
/// # Safety
/// The caller must not free the returned pointer nor use it across
/// a subsequent call that records a new error on the same thread.
#[no_mangle]
pub extern "C" fn easynet_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| match &*slot.borrow() {
        Some(record) => record.message.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Return the current thread's typed last-error JSON.
///
/// The returned string is caller-owned and must be released with
/// `easynet_string_free`. When no error is recorded, this returns the
/// JSON literal `null`.
///
/// # Safety
/// `out_error_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_last_error_json(out_error_json: *mut *mut c_char) -> i32 {
    if out_error_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_last_error_json: out_error_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_error_json = std::ptr::null_mut() };

    let json = LAST_ERROR.with(|slot| match &*slot.borrow() {
        Some(record) => typed_error_json(record.code, record.message.to_string_lossy().as_ref()),
        None => serde_json::Value::Null,
    });
    write_json_output("easynet_last_error_json", out_error_json, json)
}

/// Project a stable C ABI error code and optional message into the
/// shared `DaemonError` JSON DTO.
///
/// Bindings that already have a non-zero return code can call this
/// directly and branch on `code` in the returned JSON without parsing
/// `easynet_last_error()` text.
///
/// # Safety
/// `message` may be null; if non-null it must be a valid UTF-8 C string.
/// `out_error_json` must be a non-null caller-owned pointer.
#[no_mangle]
pub unsafe extern "C" fn easynet_error_json(
    code: i32,
    message: *const c_char,
    out_error_json: *mut *mut c_char,
) -> i32 {
    if out_error_json.is_null() {
        set_last_error_code(
            ERR_NULL_POINTER,
            "easynet_error_json: out_error_json pointer is null",
        );
        return ERR_NULL_POINTER;
    }
    unsafe { *out_error_json = std::ptr::null_mut() };

    let message = if message.is_null() {
        last_error_message().unwrap_or_default()
    } else {
        match unsafe { CStr::from_ptr(message) }.to_str() {
            Ok(value) => value.to_string(),
            Err(_) => {
                set_last_error_code(
                    ERR_INVALID_UTF8,
                    "easynet_error_json: message is not valid UTF-8",
                );
                return ERR_INVALID_UTF8;
            }
        }
    };

    let json = if code == EASYNET_OK {
        serde_json::Value::Null
    } else {
        typed_error_json(Some(code), &message)
    };
    write_json_output("easynet_error_json", out_error_json, json)
}

fn last_error_message() -> Option<String> {
    LAST_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|record| record.message.to_string_lossy().into_owned())
    })
}

fn write_json_output(
    function: &'static str,
    out_error_json: *mut *mut c_char,
    json: serde_json::Value,
) -> i32 {
    let ptr = alloc_output_cstring(json.to_string());
    if ptr.is_null() {
        set_last_error_code(
            ERR_GENERIC,
            format!("{function}: out-of-memory allocating error JSON"),
        );
        return ERR_GENERIC;
    }
    unsafe { *out_error_json = ptr };
    EASYNET_OK
}

fn alloc_output_cstring(s: impl Into<String>) -> *mut c_char {
    let sanitized: String = s.into().chars().filter(|c| *c != '\0').collect();
    match CString::new(sanitized) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn typed_error_json(code: Option<i32>, message: &str) -> serde_json::Value {
    let metadata = error_metadata(code.unwrap_or(ERR_GENERIC));
    serde_json::json!({
        "code": metadata.code,
        "stage": metadata.stage,
        "message": message,
        "retry": metadata.retry,
        "source": "c_abi",
        "invocation_id": null,
        "receipt_ura": null,
        "details": {
            "abi_code": code.unwrap_or(ERR_GENERIC),
            "abi_symbol": metadata.abi_symbol,
            "legacy_untyped": code.is_none(),
        },
    })
}

struct ErrorMetadata {
    code: &'static str,
    abi_symbol: &'static str,
    stage: &'static str,
    retry: &'static str,
}

fn error_metadata(code: i32) -> ErrorMetadata {
    match code {
        EASYNET_OK => ErrorMetadata {
            code: "OK",
            abi_symbol: "EASYNET_OK",
            stage: "sdk",
            retry: "never",
        },
        ERR_NULL_POINTER => ErrorMetadata {
            code: "NULL_POINTER",
            abi_symbol: "ERR_NULL_POINTER",
            stage: "sdk",
            retry: "never",
        },
        ERR_INVALID_UTF8 => ErrorMetadata {
            code: "INVALID_UTF8",
            abi_symbol: "ERR_INVALID_UTF8",
            stage: "sdk",
            retry: "never",
        },
        ERR_INVALID_HANDLE => ErrorMetadata {
            code: "INVALID_HANDLE",
            abi_symbol: "ERR_INVALID_HANDLE",
            stage: "sdk",
            retry: "never",
        },
        ERR_NOT_INITIALIZED => ErrorMetadata {
            code: "NOT_INITIALIZED",
            abi_symbol: "ERR_NOT_INITIALIZED",
            stage: "sdk",
            retry: "never",
        },
        ERR_ALREADY_INIT => ErrorMetadata {
            code: "ALREADY_INIT",
            abi_symbol: "ERR_ALREADY_INIT",
            stage: "sdk",
            retry: "never",
        },
        ERR_DAEMON_DOWN => ErrorMetadata {
            code: "DAEMON_OFFLINE",
            abi_symbol: "ERR_DAEMON_DOWN",
            stage: "transport",
            retry: "after_backoff",
        },
        ERR_VERSION_INCOMPATIBLE => ErrorMetadata {
            code: "VERSION_MISMATCH",
            abi_symbol: "ERR_VERSION_INCOMPATIBLE",
            stage: "sdk",
            retry: "never",
        },
        ERR_ABILITY_FAILED => ErrorMetadata {
            code: "ADMISSION_DENIED",
            abi_symbol: "ERR_ABILITY_FAILED",
            stage: "runtime",
            retry: "unknown",
        },
        ERR_NOT_IMPLEMENTED => ErrorMetadata {
            code: "NOT_IMPLEMENTED",
            abi_symbol: "ERR_NOT_IMPLEMENTED",
            stage: "sdk",
            retry: "never",
        },
        ERR_INVALID_ARG => ErrorMetadata {
            code: "INVALID_ARGUMENT",
            abi_symbol: "ERR_INVALID_ARG",
            stage: "sdk",
            retry: "never",
        },
        ERR_PERMISSION_DENIED => ErrorMetadata {
            code: "PERMISSION_DENIED",
            abi_symbol: "ERR_PERMISSION_DENIED",
            stage: "runtime",
            retry: "never",
        },
        ERR_NOT_FOUND => ErrorMetadata {
            code: "ABILITY_NOT_FOUND",
            abi_symbol: "ERR_NOT_FOUND",
            stage: "runtime",
            retry: "never",
        },
        ERR_CANCELLED => ErrorMetadata {
            code: "CANCELLED",
            abi_symbol: "ERR_CANCELLED",
            stage: "client",
            retry: "never",
        },
        ERR_PROTOCOL => ErrorMetadata {
            code: "PROTOCOL_MISMATCH",
            abi_symbol: "ERR_PROTOCOL",
            stage: "protocol",
            retry: "never",
        },
        ERR_TIMEOUT => ErrorMetadata {
            code: "TIMEOUT",
            abi_symbol: "ERR_TIMEOUT",
            stage: "transport",
            retry: "safe",
        },
        _ => ErrorMetadata {
            code: "GENERIC",
            abi_symbol: "ERR_GENERIC",
            stage: "sdk",
            retry: "unknown",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_error_null_before_any_error() {
        // A thread that has not produced an error must see NULL.
        // Run in a fresh thread so prior test ordering cannot stain
        // the TLS slot.
        std::thread::spawn(|| {
            let p = easynet_last_error();
            assert!(p.is_null());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn set_then_read_last_error_round_trips_message() {
        std::thread::spawn(|| {
            set_last_error("boom");
            let p = easynet_last_error();
            assert!(!p.is_null());
            let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap();
            assert_eq!(s, "boom");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn clear_last_error_makes_pointer_null_again() {
        std::thread::spawn(|| {
            set_last_error("oops");
            clear_last_error();
            let p = easynet_last_error();
            assert!(p.is_null());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn last_error_json_returns_null_when_clean() {
        std::thread::spawn(|| {
            clear_last_error();
            let mut out: *mut c_char = std::ptr::null_mut();
            let code = unsafe { easynet_last_error_json(&mut out) };
            assert_eq!(code, EASYNET_OK);
            assert!(!out.is_null());
            let value: serde_json::Value =
                unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
            unsafe { crate::ffi::strings::easynet_string_free(out) };
            assert_eq!(value, serde_json::Value::Null);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn last_error_json_projects_coded_tls_error() {
        std::thread::spawn(|| {
            set_last_error_code(ERR_INVALID_HANDLE, "bad handle");
            let mut out: *mut c_char = std::ptr::null_mut();
            let code = unsafe { easynet_last_error_json(&mut out) };
            assert_eq!(code, EASYNET_OK);
            let value: serde_json::Value =
                unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
            unsafe { crate::ffi::strings::easynet_string_free(out) };
            assert_eq!(value["code"], "INVALID_HANDLE");
            assert_eq!(value["stage"], "sdk");
            assert_eq!(value["retry"], "never");
            assert_eq!(value["message"], "bad handle");
            assert_eq!(value["details"]["abi_code"], ERR_INVALID_HANDLE);
            assert_eq!(value["details"]["legacy_untyped"], false);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn last_error_json_projects_legacy_message_as_generic() {
        std::thread::spawn(|| {
            set_last_error("legacy text");
            let mut out: *mut c_char = std::ptr::null_mut();
            let code = unsafe { easynet_last_error_json(&mut out) };
            assert_eq!(code, EASYNET_OK);
            let value: serde_json::Value =
                unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
            unsafe { crate::ffi::strings::easynet_string_free(out) };
            assert_eq!(value["code"], "GENERIC");
            assert_eq!(value["message"], "legacy text");
            assert_eq!(value["details"]["legacy_untyped"], true);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn error_json_maps_explicit_code_without_parsing_message() {
        let message = CString::new("deadline elapsed").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = unsafe { easynet_error_json(ERR_TIMEOUT, message.as_ptr(), &mut out) };
        assert_eq!(code, EASYNET_OK);
        let value: serde_json::Value =
            unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
        unsafe { crate::ffi::strings::easynet_string_free(out) };
        assert_eq!(value["code"], "TIMEOUT");
        assert_eq!(value["stage"], "transport");
        assert_eq!(value["retry"], "safe");
        assert_eq!(value["message"], "deadline elapsed");
        assert_eq!(value["details"]["abi_symbol"], "ERR_TIMEOUT");
    }

    #[test]
    fn error_json_projects_abi_codes_to_canonical_runtime_codes() {
        for (abi_code, expected_code, expected_symbol) in [
            (ERR_DAEMON_DOWN, "DAEMON_OFFLINE", "ERR_DAEMON_DOWN"),
            (
                ERR_VERSION_INCOMPATIBLE,
                "VERSION_MISMATCH",
                "ERR_VERSION_INCOMPATIBLE",
            ),
            (ERR_ABILITY_FAILED, "ADMISSION_DENIED", "ERR_ABILITY_FAILED"),
            (ERR_NOT_FOUND, "ABILITY_NOT_FOUND", "ERR_NOT_FOUND"),
            (ERR_PROTOCOL, "PROTOCOL_MISMATCH", "ERR_PROTOCOL"),
        ] {
            let message = CString::new("typed projection").unwrap();
            let mut out: *mut c_char = std::ptr::null_mut();
            let code = unsafe { easynet_error_json(abi_code, message.as_ptr(), &mut out) };
            assert_eq!(code, EASYNET_OK);
            let value: serde_json::Value =
                unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
            unsafe { crate::ffi::strings::easynet_string_free(out) };
            assert_eq!(value["code"], expected_code);
            assert_eq!(value["details"]["abi_code"], abi_code);
            assert_eq!(value["details"]["abi_symbol"], expected_symbol);
        }
    }

    #[test]
    fn last_error_json_null_output_records_typed_null_pointer() {
        std::thread::spawn(|| {
            let code = unsafe { easynet_last_error_json(std::ptr::null_mut()) };
            assert_eq!(code, ERR_NULL_POINTER);

            let mut out: *mut c_char = std::ptr::null_mut();
            let code = unsafe { easynet_last_error_json(&mut out) };
            assert_eq!(code, EASYNET_OK);
            let value: serde_json::Value =
                unsafe { serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap() };
            unsafe { crate::ffi::strings::easynet_string_free(out) };
            assert_eq!(value["code"], "NULL_POINTER");
            assert_eq!(value["details"]["abi_code"], ERR_NULL_POINTER);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn interior_nul_in_message_is_stripped_not_fatal() {
        // An error message that includes a \0 (e.g. accidentally
        // concatenated binary data) must not break the last-error
        // path. The stripping behaviour is a defensive measure
        // because losing an error message is worse than losing
        // a \0 inside it.
        std::thread::spawn(|| {
            set_last_error("a\0b\0c");
            let p = easynet_last_error();
            let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap();
            assert_eq!(s, "abc");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn error_codes_are_distinct_integers() {
        // Cheap guard against a copy-paste bug that assigns two
        // symbolic names to the same integer. Client bindings
        // switch on these; duplicates would route an error to the
        // wrong branch.
        let codes = [
            EASYNET_OK,
            ERR_GENERIC,
            ERR_NULL_POINTER,
            ERR_INVALID_UTF8,
            ERR_INVALID_HANDLE,
            ERR_NOT_INITIALIZED,
            ERR_ALREADY_INIT,
            ERR_DAEMON_DOWN,
            ERR_VERSION_INCOMPATIBLE,
            ERR_ABILITY_FAILED,
            ERR_NOT_IMPLEMENTED,
            ERR_INVALID_ARG,
            ERR_PERMISSION_DENIED,
            ERR_NOT_FOUND,
            ERR_CANCELLED,
            ERR_PROTOCOL,
            ERR_TIMEOUT,
        ];
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(codes[i], codes[j], "duplicate error code at {i} vs {j}");
            }
        }
    }
}

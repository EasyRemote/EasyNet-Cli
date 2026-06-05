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
use std::ffi::CString;
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
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Record an error message for later retrieval by
/// `easynet_last_error`. Called internally from exported functions
/// immediately before returning a non-zero code.
pub(crate) fn set_last_error(msg: impl Into<String>) {
    let s = msg.into();
    // Strip any interior NULs so the CString construction cannot
    // fail on well-formed Rust strings that happen to contain a \0.
    let sanitized: String = s.chars().filter(|c| *c != '\0').collect();
    let c = CString::new(sanitized).unwrap_or_else(|_| CString::new("(unrepresentable)").unwrap());
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(c);
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
        Some(c) => c.as_ptr(),
        None => std::ptr::null(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

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

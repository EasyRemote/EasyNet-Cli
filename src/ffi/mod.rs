// EasyNet CLI — C ABI Surface
// ============================
//
// File: src/ffi/mod.rs
// Description: C-ABI-compatible functions exported by
//              `libeasynet_cli.{so,dylib,dll,a}` for consumption by
//              non-Rust Client libraries (Go cgo, Python cffi, Node
//              N-API, Swift C interop, Java JNI). This module is the
//              public face of the library crate.
//
// Stability contract
// ------------------
// Every symbol exported from this module with `#[no_mangle]` and
// `extern "C"` is part of the ABI stability contract. Breaking a
// symbol (rename / signature change) requires bumping
// `easynet_abi_version()`. Downstream Client bindings refuse to
// initialise when the lib's reported ABI version does not match the
// one they were compiled against.
//
// `cbindgen` generates `include/easynet_cli.h` from this file tree.
// The generated header is checked into the repo; CI asserts that a
// fresh `cbindgen` run produces the same file (detects "I changed
// a signature but forgot to regenerate").
//
// Module layout
// -------------
//   mod.rs     — top-level functions (ABI version, init, shutdown).
//   handle.rs  — opaque handle types + registry + lib runtime.
//   client.rs  — the lib's internal IPC client (UDS + framed JSON).
//   errors.rs  — i32 error codes + thread-local last-error message.
//   strings.rs — UTF-8 C string ↔ Rust &str conversion helpers.
//   ability.rs — generic `easynet_ability_invoke` /
//                `easynet_ability_subscribe` helpers every feature
//                PR's FFI binding maps onto.
//
// v1 status (PR-DAEMON Commit 4)
// -------------------------------
// - ABI version + handle registry + last-error TLS: shipped (Commit 1).
// - `easynet_init` / `easynet_shutdown` + `easynet_ability_invoke`
//   wired through the real IPC client to the daemon UDS: this commit.
// - `easynet_ability_subscribe` still returns ERR_NOT_IMPLEMENTED
//   because streaming requires a long-lived reader task + frame
//   channel back to the FFI callback; that lands in a follow-up.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability;
pub mod client;
pub mod errors;
pub mod handle;
pub mod strings;

use std::os::raw::c_char;

use crate::ffi::client as ipc_client;
use crate::ffi::errors::{
    clear_last_error, set_last_error, EASYNET_OK, ERR_ALREADY_INIT, ERR_DAEMON_DOWN,
    ERR_GENERIC, ERR_INVALID_HANDLE, ERR_NULL_POINTER, ERR_VERSION_INCOMPATIBLE,
};
use crate::ffi::handle::{alloc, lib_runtime, release, ClientSession, EasynetHandle};
use crate::ffi::strings::read_cstr;
use crate::services::control::discovery;

/// Current ABI version. Every breaking change to an exported
/// `#[no_mangle] extern "C"` function bumps this integer; the CI
/// header diff + the cbindgen regeneration guard catch renames
/// that forget to bump.
///
/// v1 = 1. First value; no deprecation path to a prior value.
pub const EASYNET_ABI_VERSION: u32 = 1;

/// Report the ABI version of this library build. Client bindings
/// call this first thing at dlopen time and refuse to proceed when
/// the value disagrees with the one they were compiled against.
///
/// # Safety
/// No pointer parameters; no preconditions; always safe to call.
#[no_mangle]
pub extern "C" fn easynet_abi_version() -> u32 {
    EASYNET_ABI_VERSION
}

/// Open an IPC connection to the local daemon and return a handle.
///
/// `control_path` is an optional UTF-8 path to the daemon's
/// `control.json`. Pass NULL to use the default
/// (`~/.easynet/control.json`) — the path the daemon writes when
/// `services::control::server::run` boots.
///
/// On success: writes the new handle to `*out_handle`, returns
/// `EASYNET_OK`, clears the last-error slot.
///
/// On failure: writes 0 to `*out_handle`, returns one of:
///   - `ERR_NULL_POINTER`         — `out_handle` is null.
///   - `ERR_DAEMON_DOWN`          — control.json missing or socket
///                                  unreachable.
///   - `ERR_VERSION_INCOMPATIBLE` — IPC version overlap empty.
///   - `ERR_GENERIC`              — runtime construction failed.
/// and records a human-readable message via `set_last_error`.
///
/// # Safety
/// - `control_path` may be NULL; if non-null it must point to a
///   valid UTF-8 C string.
/// - `out_handle` must be a non-null pointer to a `u64` the caller
///   owns.
#[no_mangle]
pub unsafe extern "C" fn easynet_init(
    control_path: *const c_char,
    out_handle: *mut EasynetHandle,
) -> i32 {
    if out_handle.is_null() {
        set_last_error("easynet_init: out_handle pointer is null");
        return ERR_NULL_POINTER;
    }
    // Initialise the out value first so the caller can safely read it
    // on the failure path.
    unsafe { *out_handle = 0 };

    // Resolve control.json path: caller-supplied or default.
    let path = if control_path.is_null() {
        discovery::default_path()
    } else {
        match read_cstr(control_path) {
            Ok(s) => std::path::PathBuf::from(s),
            Err(_) => {
                set_last_error(
                    "easynet_init: control_path is not a valid UTF-8 C string",
                );
                return ERR_NULL_POINTER;
            }
        }
    };

    // Build / fetch the lib's tokio runtime. Failure here is fatal —
    // the lib cannot drive any I/O without a runtime.
    let rt = match lib_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(format!("easynet_init: {e}"));
            return ERR_GENERIC;
        }
    };

    // Block on the connect; the runtime is single-thread so this is
    // a serial dial, which is what we want — `easynet_init` is a
    // setup call, not on the hot path.
    let connect_result = rt.block_on(ipc_client::connect(&path));
    let client = match connect_result {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("{e:#}");
            set_last_error(format!("easynet_init: {msg}"));
            // Distinguish version-incompat from "no daemon" so Client
            // bindings can branch on the right code. Fall back to
            // ERR_DAEMON_DOWN for everything else (refused connect,
            // missing control.json, IO error).
            if msg.contains("version negotiation failed") {
                return ERR_VERSION_INCOMPATIBLE;
            }
            return ERR_DAEMON_DOWN;
        }
    };

    let session = ClientSession::with_client(path.to_string_lossy().into_owned(), client);
    let (id, _arc) = alloc(session);
    unsafe { *out_handle = id };
    clear_last_error();
    EASYNET_OK
}

/// Release a handle previously returned from `easynet_init`. The
/// IPC connection inside the session is closed when the registry
/// drops the last `Arc<ClientSession>`.
///
/// Returns `EASYNET_OK` if the handle was known and removed,
/// `ERR_INVALID_HANDLE` if it was not (or had already been freed —
/// double-shutdown is a programmer error, not a retriable
/// condition, but it does not crash).
///
/// # Safety
/// `handle` may be any value, including 0; the function does not
/// dereference any pointers.
#[no_mangle]
pub extern "C" fn easynet_shutdown(handle: EasynetHandle) -> i32 {
    if release(handle) {
        clear_last_error();
        EASYNET_OK
    } else {
        set_last_error(format!(
            "easynet_shutdown: handle {handle} is not registered (already shut down?)"
        ));
        ERR_INVALID_HANDLE
    }
}

/// Reject a second initialisation that targets the same control
/// path. The intended Client usage is "one handle per Client
/// process"; a second `easynet_init` is almost always a bug, but
/// returning a distinct code (`ERR_ALREADY_INIT`) lets the binding
/// surface "you already have a handle" instead of crashing.
///
/// v1 implementation: check whether any session in the registry
/// already references the requested control path. The path
/// comparison is by string, so `~/.easynet/control.json` and
/// `/Users/.../.easynet/control.json` are not deduplicated; a
/// follow-up commit can canonicalise the path. Today's behaviour
/// is documented in the ABI spec.
///
/// Note: this is not currently called from `easynet_init` because
/// the v1 contract is "init returns a fresh handle". Reserved for a
/// follow-up commit that adds the dedupe behaviour explicitly.
#[doc(hidden)]
#[allow(dead_code)]
pub(crate) fn _reserved_already_init_marker() -> i32 {
    ERR_ALREADY_INIT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_reports_constant() {
        // The function must return the const verbatim — no extra
        // logic, no environment-based branching. A regression that
        // returned a runtime value would silently break every
        // Client's version-match check.
        assert_eq!(easynet_abi_version(), EASYNET_ABI_VERSION);
    }

    #[test]
    fn abi_version_is_nonzero_to_distinguish_from_uninitialized_memory() {
        // Client bindings sometimes check `ver != 0` as a cheap
        // "did the symbol load?" test. If the ABI version ever
        // became 0, that idiom would silently pass; this pins it.
        assert!(EASYNET_ABI_VERSION >= 1);
    }

    #[test]
    fn init_rejects_null_out_handle_pointer() {
        // The null-check on out_handle must precede any I/O. A
        // regression that started I/O before validating the output
        // pointer would attempt to write into memory the caller did
        // not provide; pin the early return here.
        let code = unsafe { easynet_init(std::ptr::null(), std::ptr::null_mut()) };
        assert_eq!(code, ERR_NULL_POINTER);
    }

    #[test]
    fn init_returns_daemon_down_when_control_json_missing() {
        // Use a path that cannot exist; the FFI client returns the
        // "control.json not found" error which init maps to
        // ERR_DAEMON_DOWN. This pins the operator-visible message
        // for that path.
        let bogus = std::ffi::CString::new("/tmp/eznt-no-such-file.json").unwrap();
        let mut h: EasynetHandle = 999;
        let code = unsafe { easynet_init(bogus.as_ptr(), &mut h) };
        assert_eq!(code, ERR_DAEMON_DOWN);
        assert_eq!(h, 0, "out_handle must be zeroed on the failure path");
    }

    #[test]
    fn shutdown_with_unknown_handle_returns_invalid_handle() {
        // The handle 9_999_999 was never issued; shutdown must report
        // it as invalid rather than silently succeeding.
        let code = easynet_shutdown(9_999_999);
        assert_eq!(code, ERR_INVALID_HANDLE);
    }

    #[test]
    fn shutdown_zero_handle_is_invalid_not_silent_ok() {
        // 0 is the sentinel "null handle"; passing it to shutdown
        // is a programming error and must surface as
        // ERR_INVALID_HANDLE — silently returning OK would mask
        // double-shutdown bugs in Client code.
        let code = easynet_shutdown(0);
        assert_eq!(code, ERR_INVALID_HANDLE);
    }
}

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
//   mod.rs     — this file (top-level functions, ABI version).
//   handle.rs  — opaque handle types + DashMap registry.
//   client.rs  — the lib's internal IPC client: dial the daemon's
//                UDS/Named-Pipe and exchange `IncomingFrame` /
//                `OutgoingFrame`s.
//   errors.rs  — i32 error codes + thread-local last-error message.
//   strings.rs — UTF-8 C string ↔ Rust &str conversion helpers.
//   ability.rs — generic `easynet_ability_invoke` /
//                `easynet_ability_subscribe` helpers every feature
//                PR's FFI binding maps onto.
//
// v1 status
// ---------
// v1 ships the shape of the ABI (version, handle constructors,
// error surface). The generic ability helpers land in the next
// PR-DAEMON commit once `services::control` transport is wired.
// Shipping the ABI version function now lets the Client bindings'
// build systems link against the cdylib before any feature is
// wired end-to-end.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability;
pub mod client;
pub mod errors;
pub mod handle;
pub mod strings;

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
}

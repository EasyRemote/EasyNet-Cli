// EasyNet library crate root
// ==========================
//
// File: src/lib.rs
// Description: Library crate surface for `easynet_cli`.
//
// v10.5 R1: PR-DAEMON splits the historical binary-only crate into
// three targets sharing one library:
//
//   [lib]     easynet_cli (cdylib + rlib + staticlib) — this file
//   [[bin]]   easynet         — thin CLI wrapper in src/bin/easynet.rs
//   [[bin]]   easynet-daemon  — long-running daemon in src/bin/easynet-daemon.rs
//
// The lib carries all the business modules (core / eal / facade /
// persistence / registry / runtime / support) plus a forthcoming
// `ffi` module that exposes a C ABI for non-Rust clients loading
// `libeasynet_cli.{so,dylib,dll,a}`.
//
// Why a crate-type cdylib when we also have rlib + staticlib
// ----------------------------------------------------------
// - `rlib` keeps Rust-to-Rust internal linking available (bins in
//   this workspace consume the library as a regular Rust crate).
// - `cdylib` produces a shared library loadable from C FFI callers
//   (Go cgo, Python cffi, Swift C interop, Node N-API, Java JNI).
// - `staticlib` produces an `.a` archive for platforms that prefer
//   static linking (notably iOS).
//
// Not adding `dylib` because that carries Rust-specific symbol
// mangling; the goal here is a stable C ABI, not Rust ABI.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

// Crate-level lint policy — matches src/bin/easynet.rs for consistency.
#![allow(
    clippy::needless_pass_by_value,
    clippy::struct_excessive_bools,
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::module_name_repetitions
)]
// Ratchet (F-005): categories cleared to zero are DENIED so they
// cannot creep back; the two still-open categories
// (`result_large_err` — needs AxonError boxing/SDK slim, and
// `too_many_arguments` — the F-002 transport arg-struct refactor)
// stay at warn until their tracked fixes land. Lower the warn set,
// never raise it.
#![deny(
    clippy::needless_borrow,
    clippy::needless_return,
    clippy::redundant_closure,
    clippy::useless_format,
    clippy::missing_safety_doc
)]

pub mod core;
pub mod daemon;
pub mod eal;
pub mod facade;
pub mod ffi;
pub mod persistence;
pub mod plugins;
pub mod registry;
pub mod runtime;
pub mod services;
pub mod support;
pub mod ura;

#[cfg(feature = "axon-pb")]
pub mod pb {
    pub use easynet_axon::pb::axon;
}

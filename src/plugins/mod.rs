// EasyNet CLI — built-in plugin packages
// ======================================
//
// File: src/plugins/mod.rs
// Description: Runtime plugin packages compiled into the daemon.
//
// Architectural Position:
// - This module is the package namespace for device plugins. Runtime loading
//   policy belongs to `daemon::plugins`; feature implementation belongs here.

/// Compiled builtin plugin bindings.
pub mod builtin;

/// Compatibility path for the builtin remote desktop binding.
///
/// Runtime loading policy is in `daemon::plugins`; this re-export exists
/// so the migrated remote desktop modules keep their existing crate path while
/// the source files live under `src/plugins/builtin/remote_desktop`.
#[cfg(feature = "remote-desktop")]
pub use builtin::remote_desktop;

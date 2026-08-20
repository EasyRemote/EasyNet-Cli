// EasyNet remote desktop plugin crate
// ===================================
//
// File: plugins/remote-desktop/src/lib.rs
// Description: Public provider export for the remote desktop plugin package.
//
// Protocol Responsibility:
// - None. This crate exports a daemon plugin provider. Axon owns Invocation,
//   receipt, admission, stream/bidi, signing, and URA semantics.
//
// Implementation Approach:
// - The implementation source is package-owned under this project and mounted
//   by the daemon build for native-static linking.
// - The crate export keeps the manifest entrypoint stable as
//   `easynet_plugin_remote_desktop::provider`.
//
// Usage Contract:
// - Product callers use daemon plugin loading and public ability names, not
//   Rust module paths.
//
// Architectural Position:
// - Native-static provider package export.

pub use easynet_cli::daemon::plugins::remote_desktop::provider;

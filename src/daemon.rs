// EasyNet CLI — daemon SDK surface
// =================================
//
// File: src/daemon.rs
// Description: Rust SDK facade for starting, inspecting, stopping,
//              and invoking the local `easynet-daemon`.
//
// Responsibility boundary
// -----------------------
// This module is the public Rust API for the EasyNet product daemon.
// It owns process lifecycle and local endpoint discovery for
// `easynet-daemon`; it does not own Axon protocol semantics. When the
// `axon-pb` feature is enabled, `DaemonClient` submits complete Axon
// Invocation requests to the daemon-hosted Invocation transport.
//
// What this module is NOT
// -----------------------
// - It is not `persistence::daemon_config::DaemonConfig`. That type is
//   the validated representation of `~/.easynet/daemon-config.toml`;
//   this module's `DaemonStartConfig` is a launch request object for
//   the SDK call that starts or attaches to a process.
// - It is not the gRPC service implementation. That lives in
//   `services::invocation_transport`.
// - It is not an Axon runtime lifecycle API. `axon-runtime` remains
//   owned by the Axon SDK; this module starts only `easynet-daemon`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(feature = "axon-pb")]
mod client;
mod error;
#[cfg(feature = "axon-pb")]
mod invocation;
mod process;

#[cfg(feature = "axon-pb")]
pub use client::{DaemonBidiSession, DaemonClient};
pub use error::DaemonError;
#[cfg(feature = "axon-pb")]
pub use invocation::{DaemonInvocation, DaemonInvocationBuilder};
pub use process::{
    start_daemon, stop_daemon, DaemonEndpoints, DaemonHandle, DaemonStartConfig, DaemonStatus,
};

/// Result alias for the daemon SDK surface.
pub type Result<T> = std::result::Result<T, DaemonError>;

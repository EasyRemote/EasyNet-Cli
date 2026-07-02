// EasyNet CLI — daemon SDK surface
// =================================
//
// File: src/daemon/mod.rs
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
//   `daemon::invocation`, whose submodules own the daemon-side
//   Invocation transport, routing, and session dispatch.
// - It is not a generic service bucket. Local boot/status IPC lives
//   in `daemon::control`; product ability execution reaches the
//   daemon through Invocation, not through control frames.
// - It is not an Axon runtime lifecycle API. `axon-runtime` remains
//   owned by the Axon SDK; this module starts only `easynet-daemon`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability;
pub mod axon_bridge;
#[cfg(feature = "axon-pb")]
mod client;
pub mod context;
pub mod control;
mod error;
pub mod federation;
pub mod identity;
#[cfg(feature = "axon-pb")]
pub mod invocation;
pub mod keyring;
pub mod plugins;
mod process;
pub mod trust;

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

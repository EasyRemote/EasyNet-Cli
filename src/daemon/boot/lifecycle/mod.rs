//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/mod.rs
//! Description: daemon-local runtime lifecycle authority.
//!
//! Protocol Responsibility:
//! - Separates local process facts, CLI session projection, and product
//!   presence observation without redefining Axon Invocation semantics.
//!
//! Implementation Approach:
//! - Uses narrow value objects for discovery, projection, start preflight,
//!   status classification, stop planning, and read-only product presence.
//!
//! Usage Contract:
//! - CLI commands call the service facade and render reports; they do not own
//!   daemon lifecycle state machines.
//!
//! Architectural Position:
//! - `daemon::boot` process lifecycle layer, re-exported as
//!   `daemon::lifecycle` for domain-language callers.
//!
//! This module reconciles the three runtime signals that migration
//! temporarily allowed to drift apart:
//! 1. `runtime.json`, the CLI session projection.
//! 2. `control.json`, the daemon discovery projection.
//! 3. pid/socket probes, the current process facts.
//!
//! The daemon process remains the authority. `runtime.json` is useful
//! operator context, never proof that the runtime is alive.

pub mod discovery;
pub mod errors;
pub mod presence;
pub mod projection;
pub mod service;
pub mod start;
pub mod status;
pub mod stop;

pub use discovery::{DaemonDiscoveryObserver, DaemonDiscoverySnapshot};
pub use errors::RuntimeLifecycleError;
pub use presence::{ProductPresenceObserver, ProductPresenceSnapshot, ProductPresenceStatus};
pub use projection::{RuntimeProjectionStore, RuntimeSessionProjection};
pub use service::RuntimeLifecycleService;
pub use start::{RuntimeStartPreflightAction, RuntimeStartPreflightReport, RuntimeStartRequest};
pub use status::{RuntimeLifecycleStatus, RuntimeStatusReport};
pub use stop::{RuntimeStopPlan, RuntimeStopShape};

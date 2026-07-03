//! Daemon boot sequencing and startup state projections.

mod error;
#[cfg(feature = "axon-pb")]
pub mod invocation;
pub mod join_connection_state;
pub mod kernel;
pub mod lifecycle;
mod process;

pub use error::DaemonError;
pub use process::{
    start_daemon, stop_daemon, DaemonEndpoints, DaemonHandle, DaemonStartConfig, DaemonStatus,
};

/// Result alias for daemon boot and process-lifecycle operations.
pub type Result<T> = std::result::Result<T, DaemonError>;

//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/errors.rs
//! Description: typed errors for lifecycle boundary operations.
//!
//! Protocol Responsibility:
//! - Surfaces daemon lifecycle refusal reasons without collapsing them into
//!   generic CLI errors.
//!
//! Implementation Approach:
//! - Uses domain variants at lifecycle boundaries and carries operator-fixable
//!   details.
//!
//! Usage Contract:
//! - CLI commands may add context, but they must preserve the domain error
//!   message.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` error boundary.

use crate::daemon::DaemonError;
use std::path::PathBuf;

use super::RuntimeLifecycleStatus;

/// Errors emitted by lifecycle operations that perform side effects.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeLifecycleError {
    /// Start refused because a daemon control endpoint responds but
    /// Invocation is down.
    #[error(
        "daemon control endpoint is accepting but Invocation is down (control: {control}, invocation: {invocation})"
    )]
    StartRefusedControlOnlyInvocationDown {
        control: PathBuf,
        invocation: PathBuf,
    },

    /// Start refused because a live daemon has no discovery identity,
    /// so attach would be ambiguous.
    #[error("daemon is live but control.json has no daemon identity; refusing ambiguous attach")]
    StartRefusedMissingDaemonIdentity,

    /// Start refused because a live daemon advertises a different
    /// requested product identity.
    #[error("daemon identity mismatch for {field}: requested {requested}, discovered {actual}")]
    StartRefusedIdentityMismatch {
        field: &'static str,
        requested: String,
        actual: String,
    },

    /// Removing a stale runtime projection failed during start preflight.
    #[error("remove stale runtime projection failed: {message}")]
    ProjectionRemoveFailed { message: String },

    /// Reading `runtime.json` failed before lifecycle classification.
    #[error("load runtime projection failed: {message}")]
    ProjectionLoadFailed { message: String },

    /// Persisting `runtime.json` failed while attaching to an existing
    /// daemon; no rollback was attempted because this process does not
    /// own that daemon.
    #[error("persist runtime projection failed: {message}")]
    ProjectionPersistFailed { message: String },

    /// Persisting `runtime.json` failed after this command spawned a
    /// daemon, and rollback successfully stopped that daemon.
    #[error("persist runtime projection failed: {message}; newly started daemon was stopped")]
    ProjectionPersistRolledBack { message: String },

    /// Persisting `runtime.json` failed after this command spawned a
    /// daemon, and rollback could not stop that daemon.
    #[error(
        "persist runtime projection failed: {message}; rollback stop of newly started daemon failed: {rollback}"
    )]
    ProjectionPersistRollbackFailed {
        message: String,
        #[source]
        rollback: DaemonError,
    },
}

impl RuntimeLifecycleError {
    /// State-machine status implied by this boundary error.
    pub fn status_hint(&self) -> Option<RuntimeLifecycleStatus> {
        match self {
            Self::StartRefusedIdentityMismatch { .. } => {
                Some(RuntimeLifecycleStatus::IdentityMismatch)
            }
            Self::ProjectionPersistFailed { .. }
            | Self::ProjectionPersistRolledBack { .. }
            | Self::ProjectionPersistRollbackFailed { .. } => {
                Some(RuntimeLifecycleStatus::StartProjectionCommitFailed)
            }
            Self::ProjectionLoadFailed { .. } => None,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_persist_error_maps_to_start_commit_failed_status() {
        let err = RuntimeLifecycleError::ProjectionPersistFailed {
            message: "permission denied".to_string(),
        };

        assert_eq!(
            err.status_hint(),
            Some(RuntimeLifecycleStatus::StartProjectionCommitFailed)
        );
    }
}

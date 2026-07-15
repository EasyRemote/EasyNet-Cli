//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/projection.rs
//! Description: runtime session projection handling.
//!
//! Protocol Responsibility:
//! - Names `runtime.json` as operator/session projection, never as daemon
//!   process authority.
//!
//! Implementation Approach:
//! - Wraps the existing persisted `RuntimeState` and exposes lifecycle-domain
//!   names without changing the on-disk compatibility shape.
//!
//! Usage Contract:
//! - Mutations go through `RuntimeLifecycleService` so projection commit and
//!   rollback semantics stay centralized.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` projection object over daemon persistence.
//!
//! `RuntimeSessionProjection` wraps `runtime.json` so lifecycle code
//! speaks explicitly about a projection instead of confusing persisted
//! CLI state with process authority.

use serde_json::{json, Value};

use crate::daemon::persistence::config;

/// Concrete store for the CLI runtime session projection.
///
/// Invariants:
/// 1. Loading this store observes `runtime.json` as metadata only.
/// 2. Saves and removals are process-local filesystem side effects
///    owned by the lifecycle service, not by pure state classifiers.
/// 3. The on-disk wire shape remains `RuntimeState` until the public
///    compatibility contract is intentionally revised.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeProjectionStore;

impl RuntimeProjectionStore {
    /// Load the current session projection, if it exists.
    pub fn load(&self) -> Option<RuntimeSessionProjection> {
        RuntimeSessionProjection::load_current()
    }

    /// Persist the current session projection.
    pub fn save(&self, state: &config::RuntimeState) -> anyhow::Result<()> {
        config::save(state)
    }

    /// Remove the current session projection.
    pub fn remove(&self) -> anyhow::Result<()> {
        config::remove()
    }
}

/// Parsed `runtime.json` as lifecycle input.
///
/// Invariants:
/// 1. The inner `RuntimeState` is a projection produced by CLI start,
///    not proof that a process exists.
/// 2. Callers that need process truth must pair this object with a
///    `DaemonDiscoverySnapshot`.
/// 3. This wrapper never mutates the projection; writes go through the
///    lifecycle service so rollback semantics stay centralized.
#[derive(Debug, Clone)]
pub struct RuntimeSessionProjection {
    state: config::RuntimeState,
}

impl RuntimeSessionProjection {
    /// Wrap an already parsed runtime state.
    pub fn from_state(state: config::RuntimeState) -> Self {
        Self { state }
    }

    /// Read `runtime.json` from the current state directory.
    pub fn load_current() -> Option<Self> {
        config::load().ok().map(Self::from_state)
    }

    /// Borrow the underlying projection for legacy CLI renderers.
    pub fn as_runtime_state(&self) -> &config::RuntimeState {
        &self.state
    }

    /// Consume the wrapper and return the raw runtime projection.
    pub fn into_runtime_state(self) -> config::RuntimeState {
        self.state
    }

    /// Runtime shape declared by the projection.
    pub fn runtime_kind(&self) -> config::RuntimeKind {
        self.state.runtime_kind
    }

    /// JSON representation used by lifecycle status reports.
    pub fn to_json(&self) -> Value {
        let state = &self.state;
        json!({
            "endpoint": state.endpoint,
            "process_kind": "easynet_daemon",
            "runtime_kind": "daemon_only",
            "pid": state.pid,
            "hub": state.hub,
            "realm": state.tenant,
            "tenant": state.tenant,
            "label": state.label,
            "started_at": state.started_at,
            "credential_verified": state.credential_verified,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_json_preserves_runtime_kind_wire_name() {
        let projection = RuntimeSessionProjection::from_state(config::RuntimeState {
            endpoint: "/tmp/easynet.sock".to_string(),
            runtime_kind: config::RuntimeKind::DaemonOnly,
            pid: Some(42),
            hub: None,
            tenant: Some("tenant-test".to_string()),
            label: Some("node-test".to_string()),
            started_at: None,
            credential_verified: Some(true),
        });

        assert_eq!(projection.to_json()["runtime_kind"], "daemon_only");
        assert_eq!(projection.to_json()["process_kind"], "easynet_daemon");
    }
}

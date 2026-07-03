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

/// Lifecycle-domain process kind for `runtime.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeProcessKind {
    /// Current EasyNet product daemon process.
    EasynetDaemon,
    /// Historical raw Axon bridge runtime.
    LegacyAxonBridge,
}

impl RuntimeProcessKind {
    /// Stable lifecycle wire string.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::EasynetDaemon => "easynet_daemon",
            Self::LegacyAxonBridge => "legacy_axon_bridge",
        }
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

    /// Lifecycle-domain process kind declared by this projection.
    pub fn process_kind(&self) -> RuntimeProcessKind {
        match self.state.runtime_kind {
            config::RuntimeKind::DaemonOnly => RuntimeProcessKind::EasynetDaemon,
            config::RuntimeKind::AxonBridge => RuntimeProcessKind::LegacyAxonBridge,
        }
    }

    /// Whether this projection describes the legacy raw Axon bridge.
    pub fn uses_bridge(&self) -> bool {
        self.state.uses_bridge()
    }

    /// JSON representation used by lifecycle status reports.
    pub fn to_json(&self) -> Value {
        let state = &self.state;
        json!({
            "endpoint": state.endpoint,
            "process_kind": self.process_kind().as_wire_str(),
            "runtime_kind": match state.runtime_kind {
                config::RuntimeKind::DaemonOnly => "daemon_only",
                config::RuntimeKind::AxonBridge => "axon_bridge",
            },
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

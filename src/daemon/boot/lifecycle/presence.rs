//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/presence.rs
//! Description: read-only product presence projection for lifecycle reports.
//!
//! Protocol Responsibility:
//! - Keeps product online/offline observation separate from local daemon
//!   process facts.
//!
//! Implementation Approach:
//! - Projects the persisted join/session snapshot into a small lifecycle DTO.
//!   It never infers product `Online` from PID, socket, or `runtime.json`.
//!
//! Usage Contract:
//! - Treat `Unknown` and `Suspect` as diagnostic facts, not as permission to
//!   mark a device permanently offline.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` observer; authoritative Hub/backend directory
//!   state remains outside this local CLI layer.

use serde_json::{json, Value};

use crate::daemon::boot::join_connection_state::{self, JoinConnectionSnapshot};

/// Read-only observer for product presence projections available to
/// the local CLI.
///
/// Invariants:
/// 1. It never probes pidfiles or sockets to decide product presence.
/// 2. It only projects already-recorded session/join facts; Hub/backend
///    directory authority remains outside this local lifecycle layer.
/// 3. Missing local evidence is represented as `None`, not as `Online`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProductPresenceObserver;

impl ProductPresenceObserver {
    /// Capture the current local product-presence projection.
    pub fn capture(&self) -> Option<ProductPresenceSnapshot> {
        ProductPresenceSnapshot::capture_current()
    }
}

/// Product-facing presence status observed by the local lifecycle layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductPresenceStatus {
    /// Hub/session evidence says the product can route to this device.
    Online,
    /// The local snapshot records a liveness doubt or retryable failure.
    Suspect,
    /// Shutdown or removal is in progress.
    Draining,
    /// The product-side state is removed or explicitly offline.
    Removed,
    /// No authoritative product presence fact is available locally.
    Unknown,
}

impl ProductPresenceStatus {
    /// Stable JSON status string.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Suspect => "suspect",
            Self::Draining => "draining",
            Self::Removed => "removed",
            Self::Unknown => "unknown",
        }
    }
}

/// Product presence observation attached to lifecycle reports.
///
/// Invariants:
/// 1. `session_admitted` is true only for `Online`.
/// 2. `directory_status` is derived from the product connection snapshot, not
///    from local daemon process facts.
/// 3. `last_heartbeat_unix_ms` is absent unless a real heartbeat/directory
///    lease timestamp is observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductPresenceSnapshot {
    device_ura: Option<String>,
    session_admitted: bool,
    directory_status: ProductPresenceStatus,
    last_heartbeat_unix_ms: Option<i64>,
    dispatch_probe: Option<String>,
}

impl ProductPresenceSnapshot {
    /// Capture the current local product-presence projection.
    pub fn capture_current() -> Option<Self> {
        Self::from_join_snapshot(join_connection_state::latest_snapshot())
    }

    /// Build from a persisted join/session snapshot.
    pub fn from_join_snapshot(snapshot: JoinConnectionSnapshot) -> Option<Self> {
        let device_ura = non_empty(&snapshot.device_ura);
        if device_ura.is_none() {
            return None;
        }
        let directory_status = status_from_join_snapshot(&snapshot);
        Some(Self {
            device_ura,
            session_admitted: matches!(directory_status, ProductPresenceStatus::Online),
            directory_status,
            last_heartbeat_unix_ms: None,
            dispatch_probe: None,
        })
    }

    /// Device URA whose product presence is being observed.
    pub fn device_ura(&self) -> Option<&str> {
        self.device_ura.as_deref()
    }

    /// Whether local evidence says `session.open` has been admitted.
    pub fn session_admitted(&self) -> bool {
        self.session_admitted
    }

    /// Directory/presence status projected from product state.
    pub fn directory_status(&self) -> ProductPresenceStatus {
        self.directory_status
    }

    /// JSON representation for CLI/API status output.
    pub fn to_json(&self) -> Value {
        json!({
            "device_ura": self.device_ura,
            "session_admitted": self.session_admitted,
            "directory_status": self.directory_status.as_wire_str(),
            "last_heartbeat_unix_ms": self.last_heartbeat_unix_ms,
            "dispatch_probe": self.dispatch_probe,
        })
    }
}

fn status_from_join_snapshot(snapshot: &JoinConnectionSnapshot) -> ProductPresenceStatus {
    match snapshot.state.as_str() {
        "FRONTEND_CONNECTED" => ProductPresenceStatus::Online,
        "DEGRADED" => ProductPresenceStatus::Suspect,
        "OFFLINE" if snapshot.state_code == "J800" => ProductPresenceStatus::Draining,
        "OFFLINE" => ProductPresenceStatus::Removed,
        "FAILED" => {
            if snapshot
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retryable)
            {
                ProductPresenceStatus::Suspect
            } else {
                ProductPresenceStatus::Unknown
            }
        }
        "SESSION_CONNECTING" | "DAEMON_BOOT" | "HUB_PREFLIGHT" => ProductPresenceStatus::Unknown,
        _ => ProductPresenceStatus::Unknown,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(state: &str, state_code: &str) -> JoinConnectionSnapshot {
        JoinConnectionSnapshot {
            state: state.to_string(),
            state_code: state_code.to_string(),
            transition_id: None,
            interrupted_transition: None,
            failure: None,
            realm: "realm".to_string(),
            node_id: "node".to_string(),
            device_ura: "easynet:///r/realm/device/node".to_string(),
            hub_endpoint: Some("http://127.0.0.1:50051".to_string()),
            source: "test".to_string(),
            observed_at_unix_ms: 0,
        }
    }

    #[test]
    fn online_requires_product_connected_snapshot() {
        let presence =
            ProductPresenceSnapshot::from_join_snapshot(snapshot("FRONTEND_CONNECTED", "J800"))
                .expect("presence");

        assert!(presence.session_admitted());
        assert_eq!(presence.directory_status(), ProductPresenceStatus::Online);
    }

    #[test]
    fn daemon_boot_snapshot_is_unknown_not_online() {
        let presence = ProductPresenceSnapshot::from_join_snapshot(snapshot("DAEMON_BOOT", "J400"))
            .expect("presence");

        assert!(!presence.session_admitted());
        assert_eq!(presence.directory_status(), ProductPresenceStatus::Unknown);
    }
}

//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/discovery.rs
//! Description: daemon-side process discovery for lifecycle decisions.
//!
//! Protocol Responsibility:
//! - Observes daemon-local process facts without treating session projection
//!   as runtime authority.
//!
//! Implementation Approach:
//! - Reads `control.json`, pidfile, endpoint probes, and PID process identity
//!   into one immutable snapshot.
//!
//! Usage Contract:
//! - Callers must combine this with `RuntimeSessionProjection` only after
//!   preserving the distinction between process facts and projection metadata.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` discovery object.
//!
//! This file deliberately does not read `runtime.json`. It observes
//! only daemon-owned facts: `control.json`, the daemon pidfile, and
//! endpoint accept probes. The caller may combine this with a runtime
//! projection, but this snapshot itself stays process-authoritative.

use std::path::Path;

use serde_json::{json, Value};

use crate::daemon::boot::DaemonEndpoints;
use crate::daemon::control::discovery::{self, ControlDiscovery, DaemonIdentity};
use crate::daemon::persistence::config;
use crate::support::platform::{local_daemon_grpc, net};

/// Concrete observer for daemon-owned discovery facts.
///
/// Invariants:
/// 1. It reads only process/discovery inputs, never `runtime.json`.
/// 2. Each call returns a fresh point-in-time snapshot; callers must
///    not cache it across start/stop side effects.
/// 3. It is concrete, not trait-based, because production has one
///    discovery source in this crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct DaemonDiscoveryObserver;

impl DaemonDiscoveryObserver {
    /// Capture the current daemon process facts.
    pub fn capture(&self) -> DaemonDiscoverySnapshot {
        DaemonDiscoverySnapshot::capture_current()
    }
}

/// Point-in-time daemon process facts.
///
/// Invariants:
/// 1. `endpoints` are the exact endpoints probed to compute
///    `control_accepting` and `invocation_accepting`.
/// 2. `pid_alive` describes the selected `pid`; endpoint liveness can
///    still prove a daemon is alive when the PID is unknown.
/// 3. `control_discovery` is carried as evidence, not as authority:
///    stale discovery with dead pid and closed sockets does not make
///    the daemon live.
#[derive(Debug, Clone)]
pub struct DaemonDiscoverySnapshot {
    control_discovery: Option<ControlDiscovery>,
    pid: Option<u32>,
    pid_alive: bool,
    pid_matches_easynet: bool,
    control_accepting: bool,
    invocation_accepting: bool,
    endpoints: DaemonEndpoints,
}

impl DaemonDiscoverySnapshot {
    /// Capture the daemon process facts for the current EasyNet state
    /// directory.
    pub fn capture_current() -> Self {
        let default_endpoints = DaemonEndpoints::current();
        let control_discovery = discovery::read(&discovery::default_path()).ok().flatten();
        let endpoints = endpoints_from_discovery(&default_endpoints, control_discovery.as_ref());
        let discovery_pid = control_discovery.as_ref().map(|disc| disc.pid);
        let pidfile_pid = read_pidfile(&config::easynet_daemon_pid_path());
        let pid = choose_pid(discovery_pid, pidfile_pid);
        let pid_alive = pid.is_some_and(net::is_pid_alive);
        let pid_matches_easynet = pid.is_some_and(net::is_easynet_process);
        let control_accepting = local_daemon_grpc::probe_accepting(endpoints.control());
        let invocation_accepting = local_daemon_grpc::probe_accepting(endpoints.invocation());

        Self {
            control_discovery,
            pid,
            pid_alive,
            pid_matches_easynet,
            control_accepting,
            invocation_accepting,
            endpoints,
        }
    }

    /// Whether any process-level fact proves there is daemon work to
    /// inspect or clean up.
    pub fn has_daemon_fact(&self) -> bool {
        (self.pid_alive && self.pid_matches_easynet)
            || self.control_accepting
            || self.invocation_accepting
    }

    /// Discovery file read from `control.json`, when it exists and is
    /// parseable.
    pub fn control_discovery(&self) -> Option<&ControlDiscovery> {
        self.control_discovery.as_ref()
    }

    /// Product identity declared by the daemon, when advertised.
    pub fn identity(&self) -> Option<&DaemonIdentity> {
        self.control_discovery
            .as_ref()
            .and_then(|disc| disc.daemon_identity.as_ref())
    }

    /// Best-known daemon PID.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Whether the best-known daemon PID is alive.
    pub fn pid_alive(&self) -> bool {
        self.pid_alive
    }

    /// Whether the selected live PID still belongs to an EasyNet
    /// process. A reused PID is reported but not treated as daemon
    /// authority.
    pub fn pid_matches_easynet(&self) -> bool {
        self.pid_matches_easynet
    }

    /// Whether the lifecycle control endpoint accepts connections.
    pub fn control_accepting(&self) -> bool {
        self.control_accepting
    }

    /// Whether the daemon Invocation endpoint accepts connections.
    pub fn invocation_accepting(&self) -> bool {
        self.invocation_accepting
    }

    /// Endpoints used for this snapshot's probes.
    pub fn endpoints(&self) -> &DaemonEndpoints {
        &self.endpoints
    }

    /// JSON representation used by CLI status and FFI-facing reports.
    pub fn to_json(&self) -> Value {
        if self.control_discovery.is_none() && !self.has_daemon_fact() {
            return Value::Null;
        }
        json!({
            "pid": self.pid,
            "pid_alive": self.pid_alive,
            "pid_matches_easynet": self.pid_matches_easynet,
            "control_accepting": self.control_accepting,
            "invocation_accepting": self.invocation_accepting,
            "control_socket": self.endpoints.control().display().to_string(),
            "invocation_endpoint": self.endpoints.invocation().display().to_string(),
            "identity": self.identity().map(|identity| json!({
                "mode": identity.mode,
                "realm": identity.realm,
                "node_id": identity.node_id,
            })),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        control_discovery: Option<ControlDiscovery>,
        pid: Option<u32>,
        pid_alive: bool,
        control_accepting: bool,
        invocation_accepting: bool,
        endpoints: DaemonEndpoints,
    ) -> Self {
        Self::from_parts_with_pid_match(
            control_discovery,
            pid,
            pid_alive,
            pid_alive,
            control_accepting,
            invocation_accepting,
            endpoints,
        )
    }

    #[cfg(test)]
    pub(crate) fn from_parts_with_pid_match(
        control_discovery: Option<ControlDiscovery>,
        pid: Option<u32>,
        pid_alive: bool,
        pid_matches_easynet: bool,
        control_accepting: bool,
        invocation_accepting: bool,
        endpoints: DaemonEndpoints,
    ) -> Self {
        Self {
            control_discovery,
            pid,
            pid_alive,
            pid_matches_easynet,
            control_accepting,
            invocation_accepting,
            endpoints,
        }
    }
}

fn endpoints_from_discovery(
    default_endpoints: &DaemonEndpoints,
    discovery: Option<&ControlDiscovery>,
) -> DaemonEndpoints {
    DaemonEndpoints {
        control: discovery
            .and_then(|disc| disc.socket_path.clone())
            .unwrap_or_else(|| default_endpoints.control().to_path_buf()),
        invocation: discovery
            .and_then(|disc| disc.invocation_endpoint.clone())
            .unwrap_or_else(|| default_endpoints.invocation().to_path_buf()),
    }
}

fn choose_pid(discovery_pid: Option<u32>, pidfile_pid: Option<u32>) -> Option<u32> {
    discovery_pid
        .filter(|pid| net::is_pid_alive(*pid))
        .or_else(|| pidfile_pid.filter(|pid| net::is_pid_alive(*pid)))
        .or(discovery_pid)
        .or(pidfile_pid)
}

fn read_pidfile(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn endpoints() -> DaemonEndpoints {
        DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-lifecycle-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-lifecycle-daemon.sock"),
        }
    }

    #[test]
    fn has_daemon_fact_accepts_endpoint_liveness_without_pid() {
        let snapshot =
            DaemonDiscoverySnapshot::from_parts(None, None, false, true, false, endpoints());

        assert!(
            snapshot.has_daemon_fact(),
            "Invariant 2: accepting control socket proves daemon work even without a PID"
        );
    }

    #[test]
    fn has_daemon_fact_rejects_reused_non_easynet_pid() {
        let snapshot = DaemonDiscoverySnapshot::from_parts_with_pid_match(
            None,
            Some(12_345),
            true,
            false,
            false,
            false,
            endpoints(),
        );

        assert!(
            !snapshot.has_daemon_fact(),
            "Invariant 3: PID reuse is evidence to report, not daemon liveness authority"
        );
    }

    #[test]
    fn json_is_null_when_no_daemon_fact_exists() {
        let snapshot =
            DaemonDiscoverySnapshot::from_parts(None, None, false, false, false, endpoints());

        assert!(snapshot.to_json().is_null());
    }
}

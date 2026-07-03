// EasyNet CLI — GatewayApi (daemon kernel ↔ federation boundary)
// ==============================================================
//
// File: src/daemon/federation/gateway_api.rs
// Description: The trait that is the *only* surface by which the
//              daemon kernel reaches federation lifecycle/discovery
//              surfaces. Invocation dispatch itself goes through
//              daemon Invocation, not this trait.
//
// Why two trait boundaries, not one
// ---------------------------------
// KernelApi is "who is allowed to ask the daemon kernel to do things".
// GatewayApi is "how does the daemon kernel talk to federation". Keeping
// them separate makes mocking trivial for tests: a test that
// exercises `Kernel::invoke` can inject an in-memory GatewayApi
// mock without spinning up Axon; a test that exercises the
// Gateway can do so without instantiating a full KernelApi impl.
//
// v1 surface is small on purpose
// ------------------------------
// The plan does not try to enumerate every Axon SDK call the
// daemon kernel might make — only lifecycle/discovery calls that cross
// the daemon-kernel/federation boundary today: publish an ability, list
// peers, and send a heartbeat. Anything else stays inside
// `daemon::federation::gateway.rs` where the DendriteBridge actually lives.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::core::domain::NodeId;
use serde_json::Value;

/// A peer node as seen by this daemon's Axon registration.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node: NodeId,
    /// Free-form labels the peer advertises (from
    /// `a2a.agents_json`, `a2a.system_skills`, etc.). Callers match
    /// on label keys the registry layer already documents.
    pub labels: std::collections::BTreeMap<String, Value>,
}

/// GatewayApi surface — non-dispatch Axon-facing methods used by
/// the Execution layer.
///
/// Joint-plan phase 4 (海峰 + 凉冰, 2026-05-03) removed
/// `invoke_remote_ability` / `subscribe_remote_ability` along with
/// the `TargetScope::Remote` dispatch branch they backed. Cross-
/// device dispatch now flows through the daemon's
/// `federation.forward_invoke` ability instead — one path, one
/// helper (`daemon::invocation::routing::federation_invoke::invoke_via_federation_forward`).
/// The remaining trait methods (publish_ability / list_peers /
/// send_heartbeat) describe lifecycle / discovery surfaces that
/// are unrelated to remote dispatch and stay.
pub trait GatewayApi: Send + Sync {
    /// Publish a local ability on the Axon adapter so federated
    /// peers can discover and invoke it.
    fn publish_ability(&self, name: &str, description: &str, schema: &Value) -> anyhow::Result<()>;

    /// List currently-reachable peers (this node excluded).
    fn list_peers(&self) -> anyhow::Result<Vec<PeerInfo>>;

    /// Send one heartbeat. The existing heartbeat daemon loop
    /// already owns this behaviour; exposing it on the trait lets
    /// a test Gateway tick without reaching for the real
    /// ReconnectingBridge.
    fn send_heartbeat(&self) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_gateway_can_be_used_through_gateway_api_trait_object() {
        let gateway = crate::daemon::federation::gateway::NoopGateway::new();
        let object: &dyn GatewayApi = &gateway;

        assert!(object.list_peers().unwrap().is_empty());
    }
}

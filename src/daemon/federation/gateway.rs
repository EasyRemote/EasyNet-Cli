// EasyNet CLI — Gateway (Axon-facing implementation)
// ===================================================
//
// File: src/daemon/federation/gateway.rs
// Description: Concrete `GatewayApi` implementation backed by a
//              DendriteBridge. v1 ships a no-op placeholder so the
//              Kernel can be constructed without a live Axon
//              connection.
//
// Joint-plan phase 4 (海峰 + 凉冰, 2026-05-03)
// --------------------------------------------
// `invoke_remote_ability` and `subscribe_remote_ability` came down
// with the legacy abstraction cull. Cross-device dispatch now flows
// through the daemon's `federation.forward_invoke` ability — one
// path, one helper (`daemon::invocation::routing::federation_invoke`). What survives on
// `GatewayApi` are lifecycle / discovery surfaces unrelated to
// remote dispatch: `publish_ability`, `list_peers`, `send_heartbeat`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

use crate::daemon::federation::gateway_api::{GatewayApi, PeerInfo};

/// v1 no-op Gateway. Every method returns an empty / success value.
/// Suitable for tests and for CLI tooling that needs a Kernel but
/// does not need to reach the network.
#[derive(Debug, Default)]
pub struct NoopGateway;

impl NoopGateway {
    pub fn new() -> Self {
        Self
    }
}

impl GatewayApi for NoopGateway {
    fn publish_ability(
        &self,
        _name: &str,
        _description: &str,
        _schema: &Value,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn list_peers(&self) -> anyhow::Result<Vec<PeerInfo>> {
        Ok(Vec::new())
    }

    fn send_heartbeat(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_publish_is_success() {
        let g = NoopGateway::new();
        assert!(g.publish_ability("x.y", "doc", &Value::Null).is_ok());
    }

    #[test]
    fn noop_list_peers_is_empty() {
        let g = NoopGateway::new();
        assert!(g.list_peers().unwrap().is_empty());
    }

    #[test]
    fn noop_heartbeat_is_success() {
        let g = NoopGateway::new();
        assert!(g.send_heartbeat().is_ok());
    }
}

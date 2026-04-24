// EasyNet CLI — Gateway (Axon-facing implementation)
// ===================================================
//
// File: src/runtime/gateway.rs
// Description: Concrete `GatewayApi` implementation backed by a
//              DendriteBridge. v1 ships a no-op placeholder so the
//              Kernel can be constructed without a live Axon
//              connection; PR-DAEMON's daemon bin wires the real
//              AxonGateway later.
//
// Why v1 ships a no-op alongside the eventual real impl
// -----------------------------------------------------
// Tests and CLI tooling that exercise the KernelApi surface without
// reaching Axon (e.g. `easynet doctor`, schema-only operations,
// dry-run planners) need a Gateway they can construct cheaply. The
// no-op placeholder does not dial out to anything and returns
// sensible "empty" responses for every method. The full AxonGateway
// impl comes online when a daemon process registers with a Hub.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

use crate::runtime::gateway_api::{GatewayApi, PeerInfo, RemoteTarget};

/// v1 no-op Gateway. Every method returns an empty / success value.
/// Suitable for tests and for CLI tooling that needs a Kernel but
/// does not need to reach the network. PR-DAEMON wires the full
/// AxonGateway impl alongside this.
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

    fn invoke_remote_ability(
        &self,
        target: &RemoteTarget,
        _args: &Value,
    ) -> anyhow::Result<Value> {
        anyhow::bail!(
            "NoopGateway cannot invoke remote ability {}: no Axon bridge connected \
             (daemon not running in this process)",
            target.ability
        )
    }

    fn subscribe_remote_ability(
        &self,
        target: &RemoteTarget,
        _args: &Value,
        _on_frame: Box<dyn FnMut(Value) + Send>,
    ) -> anyhow::Result<()> {
        anyhow::bail!(
            "NoopGateway cannot subscribe to remote ability {}: no Axon bridge connected",
            target.ability
        )
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
    fn noop_invoke_returns_clear_error() {
        // The error message mentions "NoopGateway" so an operator
        // staring at a failing mission run understands the cause is
        // "no daemon", not "remote node declined".
        let g = NoopGateway::new();
        let t = RemoteTarget {
            node: crate::runtime::domain::NodeId::new("b"),
            ability: "demo.x".into(),
        };
        let err = g.invoke_remote_ability(&t, &Value::Null).unwrap_err();
        assert!(format!("{err}").contains("NoopGateway"));
    }
}

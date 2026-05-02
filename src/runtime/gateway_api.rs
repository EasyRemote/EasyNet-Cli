// EasyNet CLI — GatewayApi (runtime ↔ network boundary)
// ======================================================
//
// File: src/runtime/gateway_api.rs
// Description: The trait that is the *only* surface by which the
//              Execution layer reaches the Axon network — pair,
//              heartbeat, remote ability invocation, stream
//              subscription. All federation concerns live behind
//              this trait.
//
// Why two trait boundaries, not one
// ---------------------------------
// KernelApi is "who is allowed to ask the runtime to do things".
// GatewayApi is "how does the runtime talk to the world". Keeping
// them separate makes mocking trivial for tests: a test that
// exercises `Kernel::invoke` can inject an in-memory GatewayApi
// mock without spinning up Axon; a test that exercises the
// Gateway can do so without instantiating a full KernelApi impl.
//
// v1 surface is small on purpose
// ------------------------------
// The plan does not try to enumerate every Axon SDK call the
// runtime might make — only the ones that cross the runtime
// boundary today: publish an ability, invoke a remote ability,
// subscribe to a remote ability stream, list peers, and send a
// heartbeat. Anything else stays inside `runtime::gateway.rs`
// where the DendriteBridge actually lives.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::NodeId;
use serde_json::Value;

/// Remote-ability target. v1 always carries explicit `node_id` +
/// `ability` fields; v2 will fold these into the Invocation's
/// URA-based `callee` when signed invocation ships.
#[derive(Debug, Clone)]
pub struct RemoteTarget {
    pub node: NodeId,
    pub ability: String,
}

/// A peer node as seen by this daemon's Axon registration.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node: NodeId,
    /// Free-form labels the peer advertises (from
    /// `a2a.agents_json`, `a2a.system_skills`, etc.). Callers match
    /// on label keys the registry layer already documents.
    pub labels: std::collections::BTreeMap<String, Value>,
}

/// v1 GatewayApi surface. Every Axon-facing method the Execution
/// layer uses goes through here.
///
/// v1 signatures return `Result<_>` synchronously; v2 will add
/// async variants. The DendriteBridge owner (`runtime::gateway`)
/// is the sole concrete implementor.
pub trait GatewayApi: Send + Sync {
    /// Publish a local ability on the Axon adapter so federated
    /// peers can discover and invoke it.
    fn publish_ability(&self, name: &str, description: &str, schema: &Value) -> anyhow::Result<()>;

    /// Invoke a remote ability via `send_a2a_task`. Returns the
    /// response value when the remote side responds RPC-style. For
    /// streaming abilities, see `subscribe_remote_ability`.
    fn invoke_remote_ability(&self, target: &RemoteTarget, args: &Value) -> anyhow::Result<Value>;

    /// Subscribe to a streaming remote ability. v1 returns a boxed
    /// callback installer; v2 will switch to a proper Stream type
    /// once stream routing stability is confirmed (see plan §Axon
    /// SDK 能力 unstable-stream fallback).
    fn subscribe_remote_ability(
        &self,
        target: &RemoteTarget,
        args: &Value,
        on_frame: Box<dyn FnMut(Value) + Send>,
    ) -> anyhow::Result<()>;

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

    #[allow(dead_code)]
    fn _gateway_api_is_object_safe(_g: &dyn GatewayApi) {}
}

// EasyNet CLI — InvocationTarget Resolver (dispatch stage 1)
// ===========================================================
//
// File: src/runtime/invocation_target.rs
// Description: The *explicit* resolver stage that turns a caller's
//              `InvocationPlan` (ability name + args + hints) into
//              an `InvocationTarget` (scope = Local | Remote,
//              call mode = Rpc | Stream). The downstream dispatch
//              executor (`ability_dispatch.rs`) consumes the
//              resolved target and nothing else.
//
// Why this exists as its own file
// -------------------------------
// Plan v10.1 makes target resolution a first-class stage, not a
// one-liner inside dispatch. The reason is that the future planner
// / capability router / locality preference layer will all hang off
// this resolver — putting it in one place means there is exactly one
// call site to update when those land.
//
// A second reason is the CI grep rule in
// `scripts/check-dispatch-boundary.sh`: handlers under
// `src/runtime/system/*_ability.rs` are forbidden from branching on
// `self.node_id` or `target_node == self`. All those checks happen
// once, here.
//
// v1 state
// --------
// The resolver here is the trait + basic plan/target types. The
// concrete `resolve()` implementation lives in the feature PR that
// first needs it (PR-SYS); v1 ships the skeleton so downstream PRs
// can import the types without touching each other.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::NodeId;
use serde_json::Value;

/// Caller's request *before* the resolver has decided scope or
/// call mode. Built by the IPC layer (or by a future planner) from
/// Client-supplied parameters.
#[derive(Debug, Clone)]
pub struct InvocationPlan {
    /// Fully-qualified ability name (`<agent>.chat`,
    /// `system.session.attach`, etc.).
    pub ability: String,

    /// Raw JSON args. v1 uses serde JSON; v2 will switch to
    /// proto-encoded bytes once schemas/ is wired.
    pub args: Value,

    /// Optional routing hint. When the Client explicitly names a
    /// target node (`node: "workstation-B"` in the args), the IPC
    /// layer surfaces it here so the resolver can honour it without
    /// re-parsing args.
    pub target_node_hint: Option<NodeId>,

    /// Streaming vs single-shot RPC.
    pub call_mode: CallMode,
}

/// Resolved target. Feature PR handlers consume this type; they are
/// forbidden (by CI grep) from inspecting raw `target_node` fields
/// or making local-vs-remote decisions themselves.
#[derive(Debug, Clone)]
pub struct InvocationTarget {
    pub scope: TargetScope,
    pub ability: String,
    pub normalized_args: Value,
    pub call_mode: CallMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetScope {
    /// Ability runs in-process on this daemon. The executor calls
    /// the local AbilityToolAdapter handler.
    Local,
    /// Ability runs on a remote node via Axon `send_a2a_task`.
    Remote { node: NodeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallMode {
    /// Single-shot request/response.
    Rpc,
    /// Streaming: one request, multiple response frames, explicit
    /// terminal frame at the end.
    Stream,
}

/// Trait for the resolver. Concrete impl lives in PR-SYS.
pub trait TargetResolver: Send + Sync {
    fn resolve(&self, plan: InvocationPlan) -> anyhow::Result<InvocationTarget>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_scope_distinguishes_local_from_remote_by_equality() {
        // Equality check is load-bearing: PR-SYS's dispatch executor
        // pattern-matches on scope; a regression that made every
        // Local compare equal to every other Local (or worse, to
        // Remote variants) would route every call one direction.
        let local = TargetScope::Local;
        let remote_a = TargetScope::Remote {
            node: NodeId::new("A"),
        };
        let remote_b = TargetScope::Remote {
            node: NodeId::new("B"),
        };
        assert_eq!(local, TargetScope::Local);
        assert_ne!(local, remote_a);
        assert_ne!(remote_a, remote_b);
    }
}

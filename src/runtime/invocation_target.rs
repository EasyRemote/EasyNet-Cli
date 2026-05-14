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
    /// `device.session.attach`, etc.).
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

    /// AXIOM 7-tuple `subject` — the resource URA the invocation
    /// acts on. `None` for legacy abilities and for the degenerate
    /// `subject = callee` case (per INV-META-SUBJECT-EXEMPT). Per
    /// **INV-SUBJECT-ENVELOPE**: when set, this MUST come from the
    /// invocation envelope (signed cross-process bytes), NEVER from
    /// args. The IPC translator that builds this plan reads the
    /// signed envelope's subject field; future in-process callers
    /// supply it explicitly via the `with_subject` builder on the
    /// resolved target.
    pub subject: Option<String>,
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
    /// Resolved AXIOM 7-tuple `subject`. Carried through
    /// dispatch so handlers registered via `register_*_with_envelope`
    /// can read it. Per **INV-SUBJECT-ENVELOPE**: handlers that
    /// need a subject MUST consume it from this field; they MUST
    /// NOT accept a `subject` key in `normalized_args`. The
    /// `reject_subject_in_args` guard in media_abilities enforces
    /// the negative half. Default `None` — every legacy site
    /// (struct literal in tests, resolvers built before this PR)
    /// gets `None` until it explicitly sets a subject.
    pub subject: Option<String>,
}

impl InvocationTarget {
    /// Builder: attach a subject to the resolved target. Used by
    /// callers that have envelope context (the IPC translator, or a
    /// future planner). In-process tests construct the literal
    /// directly with `subject: None`; they exercise the no-subject
    /// path which most handlers don't need.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }
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
    /// Bidirectional session: long-lived, both ends may push frames
    /// at any time until either closes. See C-M3a design doc and
    /// `LocalBidiHandler` for the contract.
    Bidi,
}

/// Trait for the resolver. Concrete impl: `LocalNodeResolver`.
pub trait TargetResolver: Send + Sync {
    fn resolve(&self, plan: InvocationPlan) -> anyhow::Result<InvocationTarget>;
}

/// PR-SYS concrete resolver. Knows the local node's id; resolves
/// `Local` when the plan's `target_node_hint` is absent or matches
/// the local id; resolves `Remote { node }` otherwise.
///
/// Resolution rules (single source of truth — no handler may
/// re-derive them):
///
///   1. `target_node_hint == None`           → `Local`
///   2. `target_node_hint == Some(local_id)` → `Local`  (loopback)
///   3. `target_node_hint == Some(other)`    → `Remote { node: other }`
///
/// The args are passed through unchanged in v1; v2 may add
/// proto-canonicalisation here.
pub struct LocalNodeResolver {
    local_node: NodeId,
}

impl LocalNodeResolver {
    pub fn new(local_node: NodeId) -> Self {
        Self { local_node }
    }

    /// The local node id this resolver was built with. Exposed for
    /// observability and for tests that assert "loopback fired".
    pub fn local_node(&self) -> &NodeId {
        &self.local_node
    }
}

impl TargetResolver for LocalNodeResolver {
    fn resolve(&self, plan: InvocationPlan) -> anyhow::Result<InvocationTarget> {
        let scope = match &plan.target_node_hint {
            None => TargetScope::Local,
            Some(node) if node == &self.local_node => TargetScope::Local,
            Some(node) => TargetScope::Remote { node: node.clone() },
        };
        Ok(InvocationTarget {
            scope,
            ability: plan.ability,
            normalized_args: plan.args,
            call_mode: plan.call_mode,
            subject: plan.subject,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    fn plan(hint: Option<&str>) -> InvocationPlan {
        InvocationPlan {
            ability: "observe.health".into(),
            args: json!({}),
            target_node_hint: hint.map(NodeId::new),
            call_mode: CallMode::Rpc,
            subject: None,
        }
    }

    #[test]
    fn resolver_no_hint_means_local() {
        // The most common case: Client did not name a target node.
        // Loopback is the right default — every operation defaults
        // to "do it here" rather than guessing a peer.
        let r = LocalNodeResolver::new(NodeId::new("self"));
        let t = r.resolve(plan(None)).unwrap();
        assert_eq!(t.scope, TargetScope::Local);
        assert_eq!(t.ability, "observe.health");
    }

    #[test]
    fn resolver_hint_equal_to_local_means_loopback() {
        // The "loopback shortcut" — Client explicitly named *this*
        // node, so the resolver returns Local instead of routing
        // back through Axon. This is the optimisation the plan
        // calls "本机 loopback" — ~10× lower latency than a remote
        // call to the same address.
        let r = LocalNodeResolver::new(NodeId::new("alpha"));
        let t = r.resolve(plan(Some("alpha"))).unwrap();
        assert_eq!(t.scope, TargetScope::Local);
    }

    #[test]
    fn resolver_hint_different_from_local_means_remote() {
        // The cross-machine case: Client named a peer. Resolver
        // surfaces a Remote scope; the executor will dispatch via
        // GatewayApi.
        let r = LocalNodeResolver::new(NodeId::new("alpha"));
        let t = r.resolve(plan(Some("beta"))).unwrap();
        assert_eq!(
            t.scope,
            TargetScope::Remote {
                node: NodeId::new("beta")
            }
        );
    }

    #[test]
    fn resolver_passes_args_through_unchanged_in_v1() {
        // v1 does not normalise args. A regression that started
        // mutating them (e.g. inserting a synthesised `node:` field)
        // would surprise downstream handlers.
        let r = LocalNodeResolver::new(NodeId::new("self"));
        let plan = InvocationPlan {
            ability: "x.y".into(),
            args: json!({"prompt": "hello", "count": 3}),
            target_node_hint: None,
            call_mode: CallMode::Rpc,
            subject: None,
        };
        let t = r.resolve(plan).unwrap();
        assert_eq!(t.normalized_args, json!({"prompt": "hello", "count": 3}));
    }

    #[test]
    fn resolver_threads_subject_from_plan_to_target() {
        // INV-SUBJECT-ENVELOPE: when the IPC translator built a
        // plan with a subject (read from the signed envelope), the
        // resolver MUST surface it on the resolved target so the
        // downstream `register_*_with_envelope` handler can read
        // it. Dropping it here would force handlers back to args
        // and break the invariant in flight.
        let r = LocalNodeResolver::new(NodeId::new("self"));
        let plan = InvocationPlan {
            ability: "camera.snapshot".into(),
            args: json!({}),
            target_node_hint: None,
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01CAM".into()),
        };
        let t = r.resolve(plan).unwrap();
        assert_eq!(
            t.subject.as_deref(),
            Some("easynet:///r/acme/resource/01CAM")
        );
    }

    #[test]
    fn target_with_subject_builder_attaches_uri() {
        // The builder is the dispatcher-side path: a caller that
        // already has a resolved target can attach a subject after
        // the fact (used by the IPC layer translator that resolves
        // first, then rebuilds with envelope context).
        let t = InvocationTarget {
            scope: TargetScope::Local,
            ability: "camera.snapshot".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
        };
        let with = t.with_subject("easynet:///r/acme/resource/01CAM");
        assert_eq!(
            with.subject.as_deref(),
            Some("easynet:///r/acme/resource/01CAM")
        );
    }
}

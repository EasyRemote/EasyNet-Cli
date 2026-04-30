// EasyNet CLI — axon_serve — federation.* thin wrappers
// =======================================================
//
// File: src/services/axon_serve/federation_wrappers.rs
// Description: Six thin wrappers for the `federation.*` ability
//              family that the new daemon binary serves over the
//              `Invocation::Invoke` (and one over `InvokeStream`)
//              RPC method, replacing the legacy axon-runtime
//              implementations while preserving wire surface.
//
// What this module is
// -------------------
// One file containing the small handler functions referenced by the
// `federation.*` arm of the dispatcher in
// `daemon_invocation_service`. Each handler is intentionally short:
// parse a JSON-encoded request shape from the `InvokeRequest.arguments`
// field, consult `PresenceRegistry` if needed, and return a typed
// `FederationResponse` value the dispatcher serialises into
// `InvokeResponse.result`.
//
// What it is NOT
// --------------
// - The dispatcher itself (lives in `daemon_invocation_service.rs`)
// - The admission gate (lives in `easynet-axon`'s `domain::admission`;
//   the dispatcher calls it before the wrapper runs)
// - The PresenceRegistry (lives in `services::presence_registry`;
//   wrappers borrow `&PresenceRegistry` to read membership state)
// - A re-implementation of the axon-runtime admission / membership /
//   delegation machinery — those are unchanged in axon and the
//   dispatcher delegates to them
//
// PR-1 staging: the wrappers' shapes
// ----------------------------------
// This commit lands all six wrapper functions with their full
// argument/response types and deterministic-field population. The
// handlers do not yet:
//
// - Verify caller URI against envelope signer (admission gate
//   integration arrives in commit 7/9 alongside the realm-trust
//   loader)
// - Push frames down a `<self>.session` reverse channel for
//   `federation.forward_invoke` (the PresenceRegistry lookup is
//   wired in commit 6/9 when the dispatcher injects the registry
//   into the service)
// - Pump the `subscribe_directory` server-stream from
//   `registry.subscribe_events()` (also commit 6/9)
//
// Until those wires connect, callers receive schema-compatible
// responses with default values for the non-deterministic fields and
// correctly-derived values for the deterministic fields. PR-4's
// schema-compat matrix accepts that distribution by design (spec
// §4.2: time-valued and freshly-minted-ID fields MAY differ).
//
// Wire surface contract
// ---------------------
// Each wrapper response is a JSON object encoded into
// `InvokeResponse.result` with `result_content_type =
// "application/json"`. The shapes here mirror what axon-runtime
// emits today; PR-4's baseline captures pin the byte-level expected
// output for SDK consumers across rust, go, python, java, node,
// swift, react. Field names are drawn from the proto definitions in
// `EasyNet-Axon/core/proto/axon/v1/federation.proto` so the JSON
// encoding is the canonical proto-JSON mapping.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::services::presence_registry::PresenceRegistry;

/// `federation.join` — caller's claimed URI is authoritative; no
/// hub-side `agent/a-X` minting (spec §5.1 URI scheme migration).
pub const ABILITY_FEDERATION_JOIN: &str = "federation.join";

/// `federation.advertise_agent` — no-op success when the caller's
/// `<self>.session` is already in the PresenceRegistry; the actual
/// directory entry is implicit in stream presence.
pub const ABILITY_FEDERATION_ADVERTISE_AGENT: &str = "federation.advertise_agent";

/// `federation.heartbeat` — warns that liveness is now stream-derived
/// and returns a typed no-op success so legacy callers see "active"
/// without us re-implementing the unary heartbeat path.
pub const ABILITY_FEDERATION_HEARTBEAT: &str = "federation.heartbeat";

/// `federation.resolve` — looks up agents in the PresenceRegistry by
/// prefix. Status always "active" because in-registry equals online.
pub const ABILITY_FEDERATION_RESOLVE: &str = "federation.resolve";

/// `federation.subscribe_directory` — the only federation.* ability
/// served via `InvokeStream` (server-stream); the others go through
/// unary `Invoke`.
pub const ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY: &str = "federation.subscribe_directory";

/// `federation.revoke` — operator-driven removal of an agent from
/// the registry via `PresenceRegistry::force_revoke`.
pub const ABILITY_FEDERATION_REVOKE: &str = "federation.revoke";

/// `federation.forward_invoke` — push an inner envelope down a
/// target agent's `<self>.session` reverse channel; correlate the
/// reply by call_id (same scheme MVP uses).
pub const ABILITY_FEDERATION_FORWARD_INVOKE: &str = "federation.forward_invoke";

/// All seven federation.* ability names in deterministic order.
/// Iteration order is the order PR-4's schema-compat matrix files
/// land on disk, so changing this slice without updating PR-4
/// fixtures is a wire-compat break.
pub const FEDERATION_ABILITIES: &[&str] = &[
    ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
    ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_FORWARD_INVOKE,
];

// ─── federation.join ───────────────────────────────────────────────

/// Request payload for `federation.join`.
///
/// Wire shape mirrors the JSON encoding of axon-runtime's
/// `JoinFederationRequest` proto. Only the deterministic fields are
/// captured here; production-only ergonomic fields (e.g. `node_label`)
/// are tolerated and ignored via `#[serde(default)]` on optional
/// fields.
#[derive(Debug, Clone, Deserialize)]
pub struct JoinRequest {
    /// Caller-claimed canonical URI (must match envelope signer per
    /// admission gate, verified in the dispatcher before this
    /// wrapper runs).
    pub canonical_agent_uri: String,
    /// Realm the caller is joining; must match the daemon's
    /// configured realm.
    pub realm: String,
}

/// Response payload for `federation.join`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JoinResponse {
    /// The caller's claimed URI, echoed back unchanged. Deterministic.
    pub canonical_agent_uri: String,
    /// The realm the caller has joined. Deterministic.
    pub realm: String,
    /// SHA-256 of `caller_uri || realm` as a 64-character lowercase
    /// hex string. Deterministic per spec §5.1 — different from
    /// axon-runtime's prior nonce-bearing receipt, MAY-differ under
    /// schema-compat.
    pub join_receipt_hash: String,
}

/// Handle a `federation.join` invocation. Pure function — no
/// PresenceRegistry interaction (the device's `<self>.session`
/// stream is what populates the registry; `join` just acknowledges
/// realm membership).
#[must_use]
pub fn handle_join(request: &JoinRequest) -> JoinResponse {
    JoinResponse {
        canonical_agent_uri: request.canonical_agent_uri.clone(),
        realm: request.realm.clone(),
        join_receipt_hash: derive_join_receipt_hash(&request.canonical_agent_uri, &request.realm),
    }
}

/// Derive the deterministic `join_receipt_hash` per spec §5.1:
/// SHA-256 of the byte concatenation `caller_uri || realm` rendered
/// as a 64-character lowercase hex string.
#[must_use]
pub fn derive_join_receipt_hash(caller_uri: &str, realm: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(caller_uri.as_bytes());
    hasher.update(realm.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

// ─── federation.advertise_agent ────────────────────────────────────

/// Request payload for `federation.advertise_agent`.
#[derive(Debug, Clone, Deserialize)]
pub struct AdvertiseAgentRequest {
    /// URI of the agent being advertised.
    pub agent_uri: String,
    /// Optional URI of a host that proxies for `agent_uri`. When
    /// present, the dispatcher verifies `host_uri` is in
    /// PresenceRegistry; PR-1 staging skips that verification.
    #[serde(default)]
    pub host_uri: Option<String>,
}

/// Response payload for `federation.advertise_agent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvertiseAgentResponse {
    /// Always `true` when this wrapper returns; rejection paths run
    /// in the admission gate before this function is called.
    pub ack: bool,
    /// Always `false` in the new architecture: directory entries are
    /// not replaced, they are stream-presence-driven. Kept in the
    /// shape for wire compat.
    pub replaced_prior: bool,
}

/// Handle a `federation.advertise_agent` invocation. PR-1 staging:
/// no-op success because directory state is implicit in
/// `<self>.session` membership rather than maintained as a
/// separate advertise table.
#[must_use]
pub fn handle_advertise_agent(_request: &AdvertiseAgentRequest) -> AdvertiseAgentResponse {
    AdvertiseAgentResponse {
        ack: true,
        replaced_prior: false,
    }
}

// ─── federation.heartbeat ──────────────────────────────────────────

/// Request payload for `federation.heartbeat`.
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatRequest {
    /// URI of the agent reporting in. Used only for log context now;
    /// liveness comes from the registry's stream membership.
    pub agent_uri: String,
}

/// Response payload for `federation.heartbeat`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatResponse {
    /// Always `"active"` in the new architecture. Deterministic.
    pub membership_status: String,
    /// Snapshot of registry size at response time. Deterministic
    /// for byte-identical state, MAY-differ field per spec §4.2
    /// (registry may have churned between identical-looking calls).
    pub realm_directory_size: usize,
}

/// Handle a `federation.heartbeat` invocation.
///
/// Logs a warning that the unary heartbeat is a no-op in the new
/// architecture (PresenceRegistry membership is the liveness signal),
/// then returns a typed success so legacy callers don't fail. The
/// `realm_directory_size` field is read from the registry snapshot
/// for transparency to operators reading audit logs.
#[must_use]
pub fn handle_heartbeat(
    _request: &HeartbeatRequest,
    registry: &PresenceRegistry,
) -> HeartbeatResponse {
    HeartbeatResponse {
        membership_status: "active".to_string(),
        realm_directory_size: registry.snapshot().len(),
    }
}

// ─── federation.resolve ────────────────────────────────────────────

/// Request payload for `federation.resolve`.
#[derive(Debug, Clone, Deserialize)]
pub struct ResolveRequest {
    /// Optional URI prefix to filter the registry on. When absent,
    /// returns every online agent.
    #[serde(default)]
    pub uri_prefix: Option<String>,
}

/// One agent in a resolve response. Mirrors
/// `FederatedNodeEntry`'s deterministic subset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSummary {
    /// The agent's URI; deterministic.
    pub canonical_agent_uri: String,
    /// Always `"active"` because in-registry equals online; spec §4.
    pub status: String,
}

/// Response payload for `federation.resolve`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolveResponse {
    /// Sorted ascending by `canonical_agent_uri` so byte-identical
    /// responses come from byte-identical state.
    pub agents: Vec<AgentSummary>,
}

/// Handle a `federation.resolve` invocation.
#[must_use]
pub fn handle_resolve(request: &ResolveRequest, registry: &PresenceRegistry) -> ResolveResponse {
    let snapshot = registry.snapshot();
    let agents = snapshot
        .into_iter()
        .filter(|uri| match &request.uri_prefix {
            Some(prefix) => uri.starts_with(prefix),
            None => true,
        })
        .map(|uri| AgentSummary {
            canonical_agent_uri: uri,
            status: "active".to_string(),
        })
        .collect();
    ResolveResponse { agents }
}

// ─── federation.revoke ─────────────────────────────────────────────

/// Request payload for `federation.revoke`.
#[derive(Debug, Clone, Deserialize)]
pub struct RevokeRequest {
    /// URI of the agent to revoke.
    pub target_uri: String,
}

/// Response payload for `federation.revoke`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeResponse {
    /// Always `true` when this wrapper returns; deterministic.
    pub ack: bool,
    /// Whether the target was online at revoke time. Deterministic
    /// for byte-identical input + state.
    pub was_active: bool,
}

/// Handle a `federation.revoke` invocation. Forces removal of the
/// target session and records whether the target was online at
/// revoke time so the caller can distinguish a real revoke from a
/// no-op.
#[must_use]
pub fn handle_revoke(request: &RevokeRequest, registry: &PresenceRegistry) -> RevokeResponse {
    let was_active = registry.lookup(&request.target_uri).is_some();
    let _displaced = registry.force_revoke(&request.target_uri);
    RevokeResponse {
        ack: true,
        was_active,
    }
}

// ─── federation.forward_invoke ─────────────────────────────────────

/// Request payload for `federation.forward_invoke` — the dispatcher
/// uses this to decide whether to push the inner envelope down a
/// target's `<self>.session` reverse channel.
#[derive(Debug, Clone, Deserialize)]
pub struct ForwardInvokeRequest {
    /// URI of the destination agent.
    pub target_uri: String,
    /// The inner invocation, encoded as the dispatcher serialised it.
    /// Opaque to this wrapper — passed through to the dispatch
    /// sender as-is.
    pub inner_envelope_b64: String,
}

/// Response payload for `federation.forward_invoke`.
///
/// **PR-1 staging shape only.** Spec §4 defines the final wire shape
/// as a *dispatch result* (the inner invocation's correlated reply,
/// carrying the target's response bytes plus the call_id used to
/// correlate). PR-1 ships this `target_online` shape during the
/// staging window because:
///
/// 1. The presence-registry lookup is implementable at PR-1 commit
///    5/9; the actual frame push down a `<self>.session` reverse
///    channel needs the broadcast-pump infrastructure that lands in
///    commit 8/9
/// 2. PR-4's schema-compat suite is *informational, not gating*
///    until commit 8/9 (see `checklists/PR-4-checklist.md §6.5`),
///    so an interim shape does not break the bisect-bisect-merge
///    plan
/// 3. Replacing the response shape later is one struct edit + a
///    test update — `ForwardInvokeResponse` is not on the path of
///    any consumer outside this commit's tests
///
/// Final shape (lands in commit 8/9): `result_bytes: Vec<u8>` +
/// `correlation_call_id: String` + a `target_offline` error variant
/// communicated via `Status::failed_precondition`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForwardInvokeResponse {
    /// PR-1 staging field; whether the target had an open
    /// `<self>.session` at lookup time. Replaced in commit 8/9.
    pub target_online: bool,
}

/// Handle a `federation.forward_invoke` invocation. PR-1 staging:
/// reports whether the target is online; the actual frame push
/// across the reverse channel arrives in a follow-up commit on the
/// same branch.
#[must_use]
pub fn handle_forward_invoke(
    request: &ForwardInvokeRequest,
    registry: &PresenceRegistry,
) -> ForwardInvokeResponse {
    ForwardInvokeResponse {
        target_online: registry.lookup(&request.target_uri).is_some(),
    }
}

// ─── federation.subscribe_directory ────────────────────────────────
//
// Server-stream wrapper. The dispatcher routes `InvokeStream` calls
// for `ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY` here. PR-1 staging
// returns the initial snapshot only; the broadcast pump is added in
// commit 6/9 when the dispatcher gains its long-lived task spawning
// surface.

/// Initial snapshot frame on a `federation.subscribe_directory`
/// stream. The first frame; subsequent frames (added in commit
/// 6/9) carry incremental events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscribeDirectoryInitial {
    /// Sorted ascending so byte-identical bytes land on the wire
    /// from byte-identical state.
    pub agents: Vec<AgentSummary>,
}

/// Build the initial snapshot frame. The dispatcher then attaches a
/// `subscribe_events` receiver to pump subsequent frames; that
/// wiring lives in `daemon_invocation_service.rs` once the registry
/// is injected.
#[must_use]
pub fn build_subscribe_directory_initial(
    registry: &PresenceRegistry,
) -> SubscribeDirectoryInitial {
    let agents = registry
        .snapshot()
        .into_iter()
        .map(|uri| AgentSummary {
            canonical_agent_uri: uri,
            status: "active".to_string(),
        })
        .collect();
    SubscribeDirectoryInitial { agents }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_dispatch_sender() -> crate::services::presence_registry::DispatchSender {
        let (tx, _rx) = mpsc::channel(256);
        tx
    }

    #[test]
    fn ability_name_constants_match_spec_section_4() {
        // These constants flow into PR-4's baseline capture file
        // names; changing them without updating PR-4 fixtures is
        // a wire-compat break.
        assert_eq!(ABILITY_FEDERATION_JOIN, "federation.join");
        assert_eq!(ABILITY_FEDERATION_ADVERTISE_AGENT, "federation.advertise_agent");
        assert_eq!(ABILITY_FEDERATION_HEARTBEAT, "federation.heartbeat");
        assert_eq!(ABILITY_FEDERATION_RESOLVE, "federation.resolve");
        assert_eq!(
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
            "federation.subscribe_directory"
        );
        assert_eq!(ABILITY_FEDERATION_REVOKE, "federation.revoke");
        assert_eq!(ABILITY_FEDERATION_FORWARD_INVOKE, "federation.forward_invoke");
        assert_eq!(FEDERATION_ABILITIES.len(), 7);
    }

    #[test]
    fn join_receipt_hash_is_deterministic() {
        let a = derive_join_receipt_hash("easynet:///r/realm/agent/n1", "realm");
        let b = derive_join_receipt_hash("easynet:///r/realm/agent/n1", "realm");
        assert_eq!(a, b, "byte-identical input must produce byte-identical hash");
        assert_eq!(a.len(), 64, "SHA-256 hex must be 64 characters");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn join_receipt_hash_pin() {
        // Pin the value directly so a future change to the
        // derivation algorithm requires updating both this test and
        // the spec §5.1 statement of `sha256(uri || realm)`.
        //
        // Equivalent in shell:
        //     printf 'agent-xrealm-y' | shasum -a 256
        //     → 61b246cc7cf71a25da55149a25953fe2caf85984ad125753dd8114875fb7c56a
        let hash = derive_join_receipt_hash("agent-x", "realm-y");
        assert_eq!(
            hash,
            "61b246cc7cf71a25da55149a25953fe2caf85984ad125753dd8114875fb7c56a"
        );
    }

    #[test]
    fn handle_join_echoes_uri_and_realm() {
        let req = JoinRequest {
            canonical_agent_uri: "easynet:///r/realm/agent/n1".to_string(),
            realm: "realm".to_string(),
        };
        let resp = handle_join(&req);
        assert_eq!(resp.canonical_agent_uri, req.canonical_agent_uri);
        assert_eq!(resp.realm, req.realm);
        assert_eq!(resp.join_receipt_hash.len(), 64);
    }

    #[test]
    fn handle_advertise_agent_returns_typed_ack() {
        let req = AdvertiseAgentRequest {
            agent_uri: "easynet:///r/realm/agent/n1".to_string(),
            host_uri: None,
        };
        let resp = handle_advertise_agent(&req);
        assert!(resp.ack);
        assert!(!resp.replaced_prior);
    }

    #[test]
    fn handle_heartbeat_reports_registry_size() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/agent/a".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/agent/b".to_string(),
            make_dispatch_sender(),
        );
        let req = HeartbeatRequest {
            agent_uri: "easynet:///r/realm/agent/a".to_string(),
        };
        let resp = handle_heartbeat(&req, &registry);
        assert_eq!(resp.membership_status, "active");
        assert_eq!(resp.realm_directory_size, 2);
    }

    #[test]
    fn handle_resolve_with_no_filter_returns_all_sorted() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/agent/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/agent/a".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/agent/b".to_string(),
            make_dispatch_sender(),
        );

        let resp = handle_resolve(&ResolveRequest { uri_prefix: None }, &registry);
        let uris: Vec<&str> = resp
            .agents
            .iter()
            .map(|a| a.canonical_agent_uri.as_str())
            .collect();
        assert_eq!(
            uris,
            vec![
                "easynet:///r/realm/agent/a",
                "easynet:///r/realm/agent/b",
                "easynet:///r/realm/agent/c",
            ]
        );
        for agent in &resp.agents {
            assert_eq!(agent.status, "active");
        }
    }

    #[test]
    fn handle_resolve_with_prefix_filters_correctly() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm-a/agent/x".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm-b/agent/y".to_string(),
            make_dispatch_sender(),
        );

        let resp = handle_resolve(
            &ResolveRequest {
                uri_prefix: Some("easynet:///r/realm-a".to_string()),
            },
            &registry,
        );
        assert_eq!(resp.agents.len(), 1);
        assert_eq!(resp.agents[0].canonical_agent_uri, "easynet:///r/realm-a/agent/x");
    }

    #[test]
    fn handle_revoke_reports_was_active_correctly() {
        let registry = PresenceRegistry::new();
        let uri = "easynet:///r/realm/agent/n1".to_string();
        registry.insert(uri.clone(), make_dispatch_sender());

        let resp = handle_revoke(
            &RevokeRequest {
                target_uri: uri.clone(),
            },
            &registry,
        );
        assert!(resp.ack);
        assert!(resp.was_active);
        assert!(registry.lookup(&uri).is_none(), "must be removed");
    }

    #[test]
    fn handle_revoke_on_unknown_uri_reports_was_active_false() {
        let registry = PresenceRegistry::new();
        let resp = handle_revoke(
            &RevokeRequest {
                target_uri: "easynet:///r/realm/agent/missing".to_string(),
            },
            &registry,
        );
        assert!(resp.ack);
        assert!(!resp.was_active);
    }

    #[test]
    fn handle_forward_invoke_reports_target_online() {
        let registry = PresenceRegistry::new();
        let uri = "easynet:///r/realm/agent/n1".to_string();
        registry.insert(uri.clone(), make_dispatch_sender());

        let resp = handle_forward_invoke(
            &ForwardInvokeRequest {
                target_uri: uri,
                inner_envelope_b64: String::new(),
            },
            &registry,
        );
        assert!(resp.target_online);
    }

    #[test]
    fn handle_forward_invoke_offline_target_reports_false() {
        let registry = PresenceRegistry::new();
        let resp = handle_forward_invoke(
            &ForwardInvokeRequest {
                target_uri: "easynet:///r/realm/agent/missing".to_string(),
                inner_envelope_b64: String::new(),
            },
            &registry,
        );
        assert!(!resp.target_online);
    }

    #[test]
    fn build_subscribe_directory_initial_snapshot_is_sorted() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/agent/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/agent/a".to_string(),
            make_dispatch_sender(),
        );

        let initial = build_subscribe_directory_initial(&registry);
        let uris: Vec<&str> = initial
            .agents
            .iter()
            .map(|a| a.canonical_agent_uri.as_str())
            .collect();
        assert_eq!(
            uris,
            vec!["easynet:///r/realm/agent/a", "easynet:///r/realm/agent/c"]
        );
    }
}

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

/// `federation.resolve_key` — peer-hub lookup of an agent URI's
/// Ed25519 public key, served from the local realm trust anchor.
/// PR-N2 commit 1/N's `FederatedKeyResolver` is the canonical
/// caller: when realm A's daemon receives a forwarded envelope
/// signed by an agent in realm B, A dials B's `federation.
/// resolve_key` to fetch the verifying key, runs the same RFC 001
/// §5.2 4-step verify, and admits or rejects identically to a
/// local-realm caller. Wire shape: request `{agent_uri}` → response
/// `{public_key_b64}`; `Status::not_found` when the URI is not in
/// this hub's trust set.
pub const ABILITY_FEDERATION_RESOLVE_KEY: &str = "federation.resolve_key";

/// `federation.discover` — cross-realm directory lookup
/// (PR-N3 N3-4). Reads the daemon's `SharedFederatedDirectoryView`
/// snapshot, fans out across every federated peer's view in lex
/// order on `peer_realm`, and returns the matching
/// `DirectoryEntry` (or every entry when no `agent_uri` filter
/// is supplied). Lex tie-break is deterministic (first peer in
/// alphabetical order wins). Returns the empty list when no peer
/// has the URI; never errors. The §2.4 `origin_realm` rewrite
/// chokepoint runs on the write side (`DirectoryView::apply_frame`)
/// so reads here are pure lookup.
pub const ABILITY_FEDERATION_DISCOVER: &str = "federation.discover";

/// All nine federation.* ability names in deterministic order.
/// Iteration order is the order PR-4's schema-compat matrix files
/// land on disk, so changing this slice without updating PR-4
/// fixtures is a wire-compat break.
pub const FEDERATION_ABILITIES: &[&str] = &[
    ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY,
    ABILITY_FEDERATION_DISCOVER,
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

// ─── federation.resolve_key ────────────────────────────────────────

/// Request payload for `federation.resolve_key`. PR-N2 commit 2/N
/// peer-side handler: the local trust anchor is consulted for the
/// supplied `agent_uri` and its base64-encoded Ed25519 public key
/// is returned (or `Status::not_found` when absent).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResolveKeyRequest {
    /// The canonical agent URI whose verifying key the caller
    /// needs. The peer hub returns its locally-known key
    /// regardless of who is asking; cross-realm trust gating is
    /// enforced caller-side by the FederatedKeyResolver before
    /// dialling, never here.
    pub agent_uri: String,
}

/// Response payload for `federation.resolve_key`. The 32-byte
/// Ed25519 verifying key is returned base64-encoded in the same
/// format `realm-trust.toml` and the local
/// `TrustAnchorKeyResolver` use, so callers can feed it directly
/// to `ed25519_dalek::VerifyingKey::from_bytes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolveKeyResponse {
    /// Base64 (standard alphabet) of 32 raw Ed25519 verifying-key
    /// bytes. The wire encoding is fixed; PR-4 schema fixtures
    /// pin this shape.
    pub public_key_b64: String,
}

/// Handle a `federation.resolve_key` invocation.
///
/// Looks up `agent_uri` in the supplied trust anchor snapshot and
/// returns its `public_key_b64` verbatim (the local trust file
/// already stores the canonical base64 form, so no re-encode is
/// needed here). On miss, returns `None`; the caller is responsible
/// for wrapping that as `Status::not_found` so the FederatedKey-
/// Resolver can distinguish "URI is not in this hub's trust set"
/// from a network-level failure.
#[must_use]
pub fn handle_resolve_key(
    request: &ResolveKeyRequest,
    trust_anchor: &crate::services::realm_trust_anchor::RealmTrustAnchor,
) -> Option<ResolveKeyResponse> {
    trust_anchor
        .lookup(&request.agent_uri)
        .map(|entry| ResolveKeyResponse {
            public_key_b64: entry.public_key_b64.clone(),
        })
}

// ─── federation.discover (PR-N3 N3-4) ──────────────────────────────

/// Request payload for `federation.discover`. PR-N3 N3-4 cross-
/// realm directory lookup. When `agent_uri` is `Some`, the
/// handler returns at most one entry (the lex-smallest peer's
/// view of that URI). When `None`, the handler returns the
/// flattened federated directory in deterministic order
/// (peers in lex order on `peer_realm`, entries within each
/// peer in lex order on `agent_uri`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoverRequest {
    /// Optional URI to filter on. Absent ⇒ return every entry
    /// in the federated directory.
    #[serde(default)]
    pub agent_uri: Option<String>,
}

/// Response payload for `federation.discover`. Each entry in
/// `entries` carries its `origin_realm` already stamped via the
/// §2.4 rewrite chokepoint, so callers can sort, group, or
/// filter by realm without trusting the wire bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoverResponse {
    /// Matching `DirectoryEntry`s. Empty when the `agent_uri`
    /// filter misses every peer; also empty when no peers are
    /// federated (single-realm daemons gracefully degrade).
    pub entries: Vec<crate::services::federation_directory::DirectoryEntry>,
}

/// Handle a `federation.discover` invocation. Pure read against
/// the supplied `SharedFederatedDirectoryView` snapshot — no I/O,
/// no async — so the dispatcher can call it inline.
#[must_use]
pub fn handle_discover(
    request: &DiscoverRequest,
    view: &crate::services::federation_directory::SharedFederatedDirectoryView,
) -> DiscoverResponse {
    let entries = match request.agent_uri.as_deref() {
        Some(uri) => crate::services::federation_directory::lookup_in_federated_view(view, uri)
            .map(|e| vec![e])
            .unwrap_or_default(),
        None => crate::services::federation_directory::flatten_federated_view(view),
    };
    DiscoverResponse { entries }
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
///
/// Wire shape per DEC-N4 §2.1:
/// - `target_uri` — destination agent URI.
/// - `inner_envelope_b64` — base64 of the caller-built inner
///   payload (`{ability, args, call_id}`); opaque to this wrapper.
/// - `causal_context_bytes` — opaque audit-chain bytes the caller's
///   `<self>.invoke_remote` initiator carries (PR-N5 §1: prior
///   ForwardReceipt hash list, possibly empty). The dispatcher
///   threads these verbatim into the target's session frame so the
///   target's InvocationReceipt can stamp `causal_context.list`
///   with the same values.
/// - `forward_deadline_ms` — caller-side deadline budget remaining
///   in milliseconds at the time the request was built. The peer
///   hub uses this to derive its own forward-call deadline (DEC-N5
///   §3); zero means "no caller-side deadline supplied" (peer
///   applies its own default).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForwardInvokeRequest {
    pub target_uri: String,
    pub inner_envelope_b64: String,
    /// Opaque audit-chain bytes; round-trips verbatim per DEC-N4
    /// §2.1 acceptance criterion. Empty when the caller's initiator
    /// has no prior receipts to chain (typical for the first call
    /// in a session).
    ///
    /// Wire shape accepts BOTH a JSON array of byte values
    /// (Rust-serde default for `Vec<u8>`) AND a base64-encoded
    /// string (Go's default `[]byte` JSON shape). PR-4 conformance
    /// captures across rust/go/python/java/node/swift/react each
    /// pick whichever shape is idiomatic for their language; the
    /// daemon's `deserialize_bytes_dual` collapses both to the same
    /// `Vec<u8>` value.
    #[serde(default, deserialize_with = "deserialize_bytes_dual")]
    pub causal_context_bytes: Vec<u8>,
    /// Caller-side remaining deadline in milliseconds. `0` is the
    /// sentinel for "no deadline supplied"; the peer applies its
    /// configured default in that case (DEC-N5 §3).
    #[serde(default)]
    pub forward_deadline_ms: u64,
}

/// Permissive bytes deserialiser accepting both the JSON-array
/// shape `[1, 2, 3]` (Rust serde default) and the base64-string
/// shape `"AQID"` (Go `[]byte` default JSON encoding). PR-4
/// SDK-conformance vectors regenerate cleanly across both
/// language families without forcing a single wire encoding.
fn deserialize_bytes_dual<'de, D>(d: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde::de::{Error as DeError, Visitor};

    struct DualVisitor;
    impl<'de> Visitor<'de> for DualVisitor {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a JSON array of byte values or a base64-encoded string")
        }
        fn visit_str<E: DeError>(self, v: &str) -> Result<Vec<u8>, E> {
            if v.is_empty() {
                return Ok(Vec::new());
            }
            STANDARD.decode(v).map_err(DeError::custom)
        }
        fn visit_string<E: DeError>(self, v: String) -> Result<Vec<u8>, E> {
            self.visit_str(&v)
        }
        fn visit_seq<A>(self, mut seq: A) -> Result<Vec<u8>, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(b) = seq.next_element::<u8>()? {
                out.push(b);
            }
            Ok(out)
        }
    }
    d.deserialize_any(DualVisitor)
}

/// Response payload for `federation.forward_invoke` (DEC-N4 §2.1
/// final shape).
///
/// `result_bytes` carries the target's ability-response bytes
/// end-to-end, opaque to the forwarding hub. `correlation_call_id`
/// is the call_id originally minted by the caller's
/// `<self>.invoke_remote` initiator (or, for the CLI bridge,
/// generated client-side at request build time); the receiving
/// daemon uses it to correlate the SessionDispatch::Result with
/// the awaiting bidi.
///
/// `target_offline` is NOT carried as an `Ok(ForwardInvokeResponse
/// { result_bytes: empty })`; per DEC-N4 §2.1 it surfaces as
/// `Status::failed_precondition` with reason text `target_offline`.
/// The previous staging field `target_online: bool` is removed
/// entirely; PR-4 baseline schema fixtures regenerate alongside
/// this shape change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForwardInvokeResponse {
    /// Target ability response bytes, opaque to the forwarder.
    pub result_bytes: Vec<u8>,
    /// Call-id minted by the caller; threaded back so the caller
    /// can correlate this response with its awaiting bidi.
    pub correlation_call_id: String,
}

/// Reason text emitted on `Status::failed_precondition` when the
/// target presence-registry lookup misses on the local-tenant
/// fast-path. Wire-stable per DEC-N4 §2.1.
pub const FORWARD_INVOKE_TARGET_OFFLINE_REASON: &str = "target_offline";

/// Handle a local-tenant `federation.forward_invoke` invocation.
///
/// Pure constructor for the DEC-N4 §2.1 `ForwardInvokeResponse`
/// shape. Threads the caller's `correlation_call_id` and the
/// target's `result_bytes` into the wire envelope. Presence-
/// registry lookup + target_offline behaviour live in the
/// dispatcher (`daemon_invocation_service::try_push_forward_
/// invoke_frame`); this wrapper exists for the test contract
/// pin and call sites that build the wire shape directly.
#[must_use]
pub fn handle_forward_invoke(
    request: &ForwardInvokeRequest,
    correlation_call_id: &str,
    result_bytes: Vec<u8>,
) -> ForwardInvokeResponse {
    let _ = request;
    ForwardInvokeResponse {
        result_bytes,
        correlation_call_id: correlation_call_id.to_string(),
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
pub fn build_subscribe_directory_initial(registry: &PresenceRegistry) -> SubscribeDirectoryInitial {
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
        assert_eq!(
            ABILITY_FEDERATION_ADVERTISE_AGENT,
            "federation.advertise_agent"
        );
        assert_eq!(ABILITY_FEDERATION_HEARTBEAT, "federation.heartbeat");
        assert_eq!(ABILITY_FEDERATION_RESOLVE, "federation.resolve");
        assert_eq!(
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
            "federation.subscribe_directory"
        );
        assert_eq!(ABILITY_FEDERATION_REVOKE, "federation.revoke");
        assert_eq!(
            ABILITY_FEDERATION_FORWARD_INVOKE,
            "federation.forward_invoke"
        );
        assert_eq!(ABILITY_FEDERATION_RESOLVE_KEY, "federation.resolve_key");
        assert_eq!(ABILITY_FEDERATION_DISCOVER, "federation.discover");
        assert_eq!(FEDERATION_ABILITIES.len(), 9);
    }

    #[test]
    fn join_receipt_hash_is_deterministic() {
        let a = derive_join_receipt_hash("easynet:///r/realm/agent/n1", "realm");
        let b = derive_join_receipt_hash("easynet:///r/realm/agent/n1", "realm");
        assert_eq!(
            a, b,
            "byte-identical input must produce byte-identical hash"
        );
        assert_eq!(a.len(), 64, "SHA-256 hex must be 64 characters");
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
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
        assert_eq!(
            resp.agents[0].canonical_agent_uri,
            "easynet:///r/realm-a/agent/x"
        );
    }

    #[test]
    fn handle_resolve_key_returns_pubkey_when_present_in_anchor() {
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };
        let entry = TrustedAgent {
            agent_uri: "easynet:///r/realm-a/agent/n1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_uri: "easynet:///r/realm-a/agent/n1".to_string(),
            },
            &anchor,
        )
        .expect("hit");
        assert_eq!(
            resp.public_key_b64,
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    #[test]
    fn handle_resolve_key_returns_none_when_uri_not_in_anchor() {
        use crate::services::realm_trust_anchor::RealmTrustAnchor;
        let anchor = RealmTrustAnchor::default();
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_uri: "easynet:///r/realm-a/agent/missing".to_string(),
            },
            &anchor,
        );
        assert!(resp.is_none(), "miss must surface as None for caller status mapping");
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
    fn handle_forward_invoke_threads_correlation_id_and_result_bytes() {
        // DEC-N4 §2.1 final shape: handle_forward_invoke is a
        // pure constructor that threads the caller's
        // correlation_call_id and the target's result_bytes
        // through. Presence-registry lookup + target_offline
        // surface live in the dispatcher's
        // try_push_forward_invoke_frame.
        let _registry = PresenceRegistry::new();
        let target_reply = b"hello from device-b".to_vec();
        let resp = handle_forward_invoke(
            &ForwardInvokeRequest {
                target_uri: "easynet:///r/realm/agent/n1".to_string(),
                inner_envelope_b64: String::new(),
                causal_context_bytes: Vec::new(),
                forward_deadline_ms: 0,
            },
            "call-id-7",
            target_reply.clone(),
        );
        assert_eq!(resp.correlation_call_id, "call-id-7");
        assert_eq!(resp.result_bytes, target_reply);
    }

    #[test]
    fn handle_forward_invoke_empty_result_bytes_is_legal_at_construction() {
        // Empty result_bytes is a legitimate shape after the
        // target dispatcher returns nothing. It is NOT how
        // target_offline is signalled (DEC-N4 §2.1 makes
        // target_offline a Status::failed_precondition).
        let resp = handle_forward_invoke(
            &ForwardInvokeRequest {
                target_uri: "easynet:///r/realm/agent/n1".to_string(),
                inner_envelope_b64: String::new(),
                causal_context_bytes: Vec::new(),
                forward_deadline_ms: 0,
            },
            "call-id-8",
            Vec::new(),
        );
        assert!(resp.result_bytes.is_empty());
        assert_eq!(resp.correlation_call_id, "call-id-8");
    }

    #[test]
    fn forward_invoke_request_round_trips_audit_chain_and_deadline() {
        // DEC-N4 §2.1 acceptance: `causal_context_bytes` and
        // `forward_deadline_ms` are wire fields on
        // `ForwardInvokeRequest` that round-trip verbatim from the
        // caller's `<self>.invoke_remote` initiator (or the CLI
        // bridge in `support::federation_invoke`) through the
        // dispatcher's JSON deserialise step. The dispatcher
        // surfaces these to the target's session frame so PR-N5's
        // InvocationReceipt can stamp `causal_context.list` and
        // DEC-N5 §3 can derive the inner deadline.
        let audit_bytes: Vec<u8> = vec![0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0xFF];
        let original = serde_json::json!({
            "target_uri": "easynet:///r/realm/agent/n1",
            "inner_envelope_b64": "",
            "causal_context_bytes": audit_bytes,
            "forward_deadline_ms": 12_345_u64,
        });
        let bytes = serde_json::to_vec(&original).unwrap();
        let parsed: ForwardInvokeRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.causal_context_bytes, audit_bytes);
        assert_eq!(parsed.forward_deadline_ms, 12_345);
    }

    #[test]
    fn forward_invoke_request_audit_fields_default_when_absent() {
        // Backwards-compat: a `ForwardInvokeRequest` produced
        // before C1a (no audit fields in the JSON) must still
        // deserialise — the new fields default to empty / zero
        // sentinels per the `#[serde(default)]` annotation.
        let pre_c1a = serde_json::json!({
            "target_uri": "easynet:///r/realm/agent/n1",
            "inner_envelope_b64": "",
        });
        let bytes = serde_json::to_vec(&pre_c1a).unwrap();
        let parsed: ForwardInvokeRequest = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.causal_context_bytes.is_empty());
        assert_eq!(parsed.forward_deadline_ms, 0);
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

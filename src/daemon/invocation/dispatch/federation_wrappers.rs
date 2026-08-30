// EasyNet Daemon — invocation_transport — federation wrappers
// ===========================================================
//
// File: src/daemon/invocation/federation_wrappers.rs
// Description: Small daemon-owned handlers for the Hub/Federation
//              baseline ability family served over Axon
//              `Invocation::{Invoke,InvokeStream}`. Ability names
//              come from `daemon::ability::conformance`; this module owns
//              transport request/response decoding only.
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
// - The PresenceRegistry (lives in `daemon::invocation::bidi::state::presence`;
//   wrappers borrow `&PresenceRegistry` to read membership state)
// - A re-implementation of the axon-runtime admission / membership /
//   delegation machinery — those are unchanged in axon and the
//   dispatcher delegates to them
//
// Wire surface contract
// ---------------------
// Each wrapper response is a JSON object encoded into
// `InvokeResponse.result` with `result_content_type =
// "application/json"`. The shapes here mirror what axon-runtime
// emits today; PR-4's baseline captures pin the byte-level expected
// output for product consumers. Field names are owned by the daemon's
// `federation::{wire_contract,resolver_contract}` modules; Axon transports
// their JSON bytes without contributing product semantics.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::ability::conformance;
use crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore;
use crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore;
#[cfg(test)]
use crate::daemon::federation::read_model::advertised_agents::{
    AdvertisedAgentRecord, AdvertisedAgentSigningAuthority,
};
use crate::daemon::federation::receipt_contract::{
    AdvertiseContract, AuthorityAbilitiesDiff, AuthorityAbilityEntry,
};
#[cfg(test)]
use crate::daemon::federation::resolver_contract::{GateResult, RecordType};
use crate::daemon::federation::resolver_contract::{
    NegativeReason, ResolveAnswerKind, ResolverReleaseProfile,
};
pub use crate::daemon::federation::wire_contract::{
    DiscoverRequest, DiscoverResponse, ListUserDevicesRequest, ListUserDevicesResponse,
    ResolveAgentSummary, ResolveFilterRequest, ResolveKeyRequest, ResolveKeyResponse,
    ResolveRequest, ResolveResponse,
};
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::invocation::routing::route_resolver::NamespaceResolveQuery;

const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// `federation.join` — caller's claimed URA is authoritative; no
/// hub-side `agent/a-X` minting (spec §5.1 URA scheme migration).
pub const ABILITY_FEDERATION_JOIN: &str = conformance::ABILITY_FEDERATION_JOIN;

/// `federation.advertise_agent` — records hosted-agent directory rows.
/// PresenceRegistry still owns transport liveness; resolve joins the
/// two so `/agent/<user>.<agent>` rows surface while online/offline
/// is derived from the host device's live `session.open`.
pub const ABILITY_FEDERATION_ADVERTISE_AGENT: &str =
    conformance::ABILITY_FEDERATION_ADVERTISE_AGENT;

/// `federation.heartbeat` — renews owner projection leases while liveness
/// remains stream-derived from the PresenceRegistry.
pub const ABILITY_FEDERATION_HEARTBEAT: &str = conformance::ABILITY_FEDERATION_HEARTBEAT;

/// `federation.resolve` — projects both live PresenceRegistry URAs
/// and hosted-agent rows whose host device is presently online.
pub const ABILITY_FEDERATION_RESOLVE: &str = conformance::ABILITY_FEDERATION_RESOLVE;

/// `namespace.resolve` — RFC-005 typed namespace resolver surface.
/// This is a daemon ability reached through `axon.v1.Invocation`; it
/// returns an Axon `ResolveAnswer` proto-JSON projection. Retired directory
/// row shapes are not accepted as an alternate read model.
pub const ABILITY_NAMESPACE_RESOLVE: &str = conformance::ABILITY_NAMESPACE_RESOLVE;

/// `namespace.proxy_resolve` — daemon-local typed namespace proxy.
/// The backend supplies the peer hub set, but the daemon owns trust
/// filtering, peer dialling, envelope signing, and typed
/// `ResolveAnswer` aggregation. This is the clean replacement for
/// backend product paths that previously consumed daemon-private directory
/// rows directly.
pub const ABILITY_NAMESPACE_PROXY_RESOLVE: &str = conformance::ABILITY_NAMESPACE_PROXY_RESOLVE;

/// `federation.revoke` — operator-driven removal of an agent from
/// the registry via `PresenceRegistry::force_revoke`.
pub const ABILITY_FEDERATION_REVOKE: &str = conformance::ABILITY_FEDERATION_REVOKE;

/// `federation.resolve_key` — peer-hub lookup of an agent URA's
/// Ed25519 public key, served from the local realm trust anchor.
/// PR-N2 commit 1/N's `FederatedKeyResolver` is the canonical
/// caller: when realm A's daemon receives a forwarded envelope
/// signed by an agent in realm B, A dials B's `federation.
/// resolve_key` to fetch the verifying key, runs the same RFC 001
/// §5.2 4-step verify, and admits or rejects identically to a
/// local-realm caller. Wire shape: request `{agent_ura}` → response
/// `{public_key_b64}`; `Status::not_found` when the URA is not in
/// this hub's trust set.
pub const ABILITY_FEDERATION_RESOLVE_KEY: &str = conformance::ABILITY_FEDERATION_RESOLVE_KEY;

/// `federation.discover` — cross-realm directory lookup
/// (PR-N3 N3-4). Reads the daemon's `SharedFederatedDirectoryView`
/// snapshot, fans out across every federated peer's view in lex
/// order on `peer_realm`, and returns the matching
/// `DirectoryEntry` (or every entry when no `agent_ura` filter
/// is supplied). Lex tie-break is deterministic (first peer in
/// alphabetical order wins). Returns the empty list when no peer
/// has the URA; never errors. The §2.4 `origin_realm` rewrite
/// chokepoint runs on the write side (`DirectoryView::apply_frame`)
/// so reads here are pure lookup.
pub const ABILITY_FEDERATION_DISCOVER: &str = conformance::ABILITY_FEDERATION_DISCOVER;

/// `federation.subscribe_directory_v2` — the canonical public federation
/// directory stream. It emits typed `DirectoryEvent` frames (Snapshot / Upsert /
/// Remove / Heartbeat) per PR-N3 spec §2.2-2.3.
pub const ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2: &str =
    conformance::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2;

/// `federation.list_user_devices` — peer-hub user-device
/// projection (PR-N3 N3-5). Backend on hub A invokes this on
/// hub B to merge B's view of the user's realm devices into the
/// `listDevices` response. Caller must authenticate as a
/// trusted hub-role peer (admission filter rejects backends
/// dialled directly from outside the federation). Hub-side
/// projects the local `PresenceRegistry` entries whose URA
/// matches the supplied realm prefix into `DirectoryEntry`s
/// with `origin_realm = None` (this hub speaks for its own
/// realm; the calling backend stamps the merge boundary's
/// realm at its end).
pub const ABILITY_FEDERATION_LIST_USER_DEVICES: &str =
    conformance::ABILITY_FEDERATION_LIST_USER_DEVICES;

/// `federation.proxy_list_user_devices` — daemon-local proxy
/// wrapper that fans `federation.list_user_devices` out across
/// the specific peer hubs the backend selected for the current
/// user. This ability is intentionally NOT a federation surface:
/// callers must be the local backend (or daemon loopback), and
/// the daemon owns the cross-hub dial + signing path so the Go
/// backend never grows a second transport stack.
pub const ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES: &str =
    conformance::ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES;

/// `federation.advertise_abilities` — backend self-registration
/// path. Backend on boot may publish product-facing ability descriptors
/// so they show up in `federation.resolve(prefix=hub)`. The handler must
/// write the owner projection read model; a missing catalog is a daemon
/// construction error, not a successful no-op.
pub const ABILITY_FEDERATION_ADVERTISE_ABILITIES: &str =
    conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES;

/// `runtime.bootstrap_self_identity` — runtime-self handshake.
///
/// Kept only as the ability-name constant. This module must not
/// provide a shadow handler for the contract; if the embedded Axon
/// runtime lacks the runtime-admin implementation, callers must see
/// that explicit missing-handler failure instead of a false ack.
pub const ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY: &str =
    conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY;

/// `federation.status` — read-only projection of the canonical join/session
/// state machine. No second process-global federation state is maintained.
pub const ABILITY_FEDERATION_STATUS: &str = conformance::ABILITY_FEDERATION_STATUS;

/// All federation.* ability names in deterministic order.
/// Iteration order is part of the canonical publication digest input, so
/// changing this slice requires an intentional descriptor/projection revision.
pub const FEDERATION_ABILITIES: &[&str] = &[
    ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY,
    ABILITY_FEDERATION_DISCOVER,
    ABILITY_FEDERATION_LIST_USER_DEVICES,
    ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_ADVERTISE_ABILITIES,
    ABILITY_FEDERATION_STATUS,
];

#[must_use]
pub fn handle_status() -> serde_json::Value {
    let snapshot = crate::daemon::boot::join_connection_state::latest_snapshot();
    let online = snapshot.state == "FRONTEND_CONNECTED";
    let code = if online {
        "installed"
    } else if snapshot.device_ura.is_empty() {
        "disabled"
    } else {
        "failed"
    };
    let failure_reason = snapshot
        .failure
        .as_ref()
        .map(|failure| failure.message.clone())
        .unwrap_or_else(|| "device session is not online".to_string());
    let outcome = match code {
        "installed" => serde_json::json!({
            "kind": "installed",
            "tenant": snapshot.realm.clone(),
            "realm": snapshot.realm.clone(),
            "device_ura": snapshot.device_ura.clone(),
            "connection": snapshot,
        }),
        "disabled" => serde_json::json!({
            "kind": "disabled",
            "reason": "device is not joined",
            "connection": snapshot,
        }),
        _ => serde_json::json!({
            "kind": "failed",
            "stage": "session_unavailable",
            "reason": failure_reason,
            "connection": snapshot,
        }),
    };
    serde_json::json!({
        "ok": online,
        "code": code,
        "outcome": outcome,
    })
}

// ─── federation.join ───────────────────────────────────────────────

/// Request payload for `federation.join`.
///
/// Wire shape is the canonical runtime join contract. Product pairing tokens
/// and ergonomic labels belong outside this descriptor-bound ability; unknown
/// fields fail closed instead of being ignored as compatibility data.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    /// Caller-claimed canonical URA (must match envelope signer per
    /// admission gate, verified in the dispatcher before this
    /// wrapper runs).
    pub membership_ura: String,
    /// Realm the caller is joining; must match the daemon's
    /// configured realm.
    pub realm: String,
    /// Lowercase hex-encoded Ed25519 public key the joining device
    /// will use for descriptor-bound membership calls after genesis.
    pub public_key_hex: String,
    /// Optional product-neutral PrincipalLifecycle proof used to bind this
    /// Device membership to a User Principal. The hub dispatcher validates the
    /// proof before mutating RuntimeTrust; the pure receipt wrapper only echoes
    /// membership facts.
    #[serde(default)]
    pub principal_enrollment: Option<PrincipalEnrollmentProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrincipalEnrollmentProof {
    pub principal_ura: String,
    pub proof: PrincipalProofRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrincipalProofRef {
    pub kind: String,
    pub reference: String,
}

/// Response payload for `federation.join`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JoinResponse {
    /// The caller's claimed URA, echoed back unchanged. Deterministic.
    pub membership_ura: String,
    /// The realm the caller has joined. Deterministic.
    pub realm: String,
    /// SHA-256 of `caller_ura || realm` as a 64-character lowercase
    /// hex string. Deterministic per spec §5.1 — different from
    /// axon-runtime's prior nonce-bearing receipt.
    pub join_receipt_hash: String,
    /// Explicit Authority-published ability catalog snapshot published at join time.
    pub authority_published_abilities: Vec<AuthorityAbilityEntry>,
    /// Monotonic revision for the Authority-published ability snapshot.
    pub authority_abilities_revision: u64,
    /// Explicit advertise policy fact for this membership.
    pub advertise_contract: AdvertiseContract,
}

/// Handle a `federation.join` invocation. Pure function — no
/// PresenceRegistry interaction (the device's `session.open`
/// stream is what populates the registry; `join` just acknowledges
/// realm membership).
#[must_use]
pub fn handle_join(request: &JoinRequest) -> JoinResponse {
    JoinResponse {
        membership_ura: request.membership_ura.clone(),
        realm: request.realm.clone(),
        join_receipt_hash: derive_join_receipt_hash(&request.membership_ura, &request.realm),
        authority_published_abilities: Vec::new(),
        authority_abilities_revision: 0,
        advertise_contract: AdvertiseContract::device_default(),
    }
}

/// Derive the deterministic `join_receipt_hash` per spec §5.1:
/// SHA-256 of the byte concatenation `caller_ura || realm` rendered
/// as a 64-character lowercase hex string.
#[must_use]
pub fn derive_join_receipt_hash(caller_ura: &str, realm: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(caller_ura.as_bytes());
    hasher.update(realm.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

// ─── federation.advertise_agent ────────────────────────────────────

/// Request payload for `federation.advertise_agent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdvertiseAgentRequest {
    /// URA of the agent being advertised.
    pub agent_ura: String,
    /// Device-persisted idempotency key for this hosted-Agent incarnation.
    pub incarnation_id:
        crate::daemon::federation::hosted_agent_publication::HostedAgentIncarnationId,
}

/// Response payload for `federation.advertise_agent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdvertiseAgentResponse {
    /// Always `true` when this wrapper returns; rejection paths run
    /// in the admission gate before this function is called.
    pub ack: bool,
    /// Exact Hub-owned generation assignment for the submitted incarnation.
    pub assignment:
        crate::daemon::federation::hosted_agent_publication::HostedAgentGenerationAssignment,
}

pub(crate) fn register_advertised_agent(
    command: crate::daemon::persistence::federation_revoke::HostedAgentRegistrationCommand,
) -> anyhow::Result<crate::daemon::persistence::federation_revoke::HostedAgentRegistrationResult> {
    crate::daemon::persistence::federation_revoke::register_agent(command)
}

pub(crate) fn advertise_agent_response(
    registration: crate::daemon::persistence::federation_revoke::HostedAgentRegistrationResult,
) -> AdvertiseAgentResponse {
    AdvertiseAgentResponse {
        ack: true,
        assignment: registration.assignment,
    }
}

// ─── federation.advertise_abilities ────────────────────────────────

/// Request payload for `federation.advertise_abilities`.
///
/// The current wire shape is RFC-005 owner projection publication:
/// the caller sends projection metadata plus bounded ability summaries.
pub(crate) type AdvertiseAbilitiesRequest =
    crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication;

/// Shared response payload for `federation.advertise_abilities`.
pub(crate) use crate::daemon::federation::advertise::AdvertiseAbilitiesResponse;

/// Handle a `federation.advertise_abilities` invocation by updating the
/// hub-side owner projection read model.
#[must_use]
pub(crate) fn handle_advertise_abilities(
    request: &AdvertiseAbilitiesRequest,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
) -> AdvertiseAbilitiesResponse {
    let count = request.ability_count();
    let outcome = catalog.upsert_projection(
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            request.owner_ura.clone(),
            request.host_device_ura.clone(),
            request.generation,
            request.projection_revision,
            request.projection_digest.clone(),
            request.lease_expires_unix_ms,
            request.ability_summaries.clone(),
        ),
    );
    let stored = outcome.is_stored();
    AdvertiseAbilitiesResponse {
        ack: stored,
        count: if stored { count } else { 0 },
        outcome: Some(outcome.as_wire_str().to_string()),
    }
}

// ─── federation.heartbeat ──────────────────────────────────────────

/// Request payload for `federation.heartbeat`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRequest {
    /// Device's last observed realm Authority-published ability revision. Until the realm Authority
    /// has a provider-backed diff source, the response explicitly echoes this
    /// revision with an empty diff instead of silently ignoring the field.
    pub since_abilities_revision: u64,
    /// Retained wire field for older clients. New event-driven owner
    /// projections are non-expiring and send this empty; admission restricts
    /// any legacy refresh request to the caller's own Device projection.
    pub refresh_owner_uras: Vec<String>,
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
    /// Explicit Authority-published ability catalog diff since the caller's last
    /// observed revision.
    pub authority_abilities_diff: AuthorityAbilitiesDiff,
}

/// Handle a `federation.heartbeat` invocation.
///
/// Renews owner projection leases while PresenceRegistry membership remains
/// the liveness signal. The ability catalog is required because heartbeat is
/// a read-model state transition, not an optional compatibility ping.
#[must_use]
pub(crate) fn handle_heartbeat(
    request: &HeartbeatRequest,
    registry: &PresenceRegistry,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
    now_unix_ms: i64,
) -> HeartbeatResponse {
    // Compatibility for a legacy same-Device projection lease. Current owner
    // projections publish lease=0 and `refresh_lease` deliberately preserves
    // that non-expiring state, so heartbeat is no longer an existence signal.
    let new_expiry =
        crate::daemon::federation::read_model::owner_projection::lease_expiry_from_now(now_unix_ms);
    for owner_ura in &request.refresh_owner_uras {
        let owner_ura = owner_ura.trim();
        if !owner_ura.is_empty() {
            catalog.refresh_lease(owner_ura, new_expiry);
        }
    }
    HeartbeatResponse {
        membership_status: "active".to_string(),
        realm_directory_size: registry.online_count(),
        authority_abilities_diff: AuthorityAbilitiesDiff::empty_at(
            request.since_abilities_revision,
        ),
    }
}

// ─── federation.resolve ────────────────────────────────────────────

/// Handle a `federation.resolve` invocation.
///
/// `catalog` is the mandatory owner projection read model the daemon
/// constructs at boot. When the canonical request filter asks for abilities
/// and the store has a row for an in-presence owner URA, the response
/// carries namespace-safe projection summaries in the historical
/// `abilities` output field. An empty catalog is the canonical "no published
/// abilities" fact; a missing catalog is a daemon construction error.
pub fn handle_resolve(
    request: &ResolveRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
    local_catalog: Option<&crate::daemon::ability::dispatch::AxonAbilityCatalog>,
) -> Result<ResolveResponse, String> {
    let local_publication = local_catalog.map(
        crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot::capture,
    );
    handle_resolve_at(
        request,
        registry,
        advertised_agents,
        catalog,
        local_publication.as_ref(),
        crate::daemon::federation::directory::now_unix_ms(),
    )
}

/// Deterministic variant of `handle_resolve` for tests and replay checks.
/// `now_unix_ms` is used only to filter expired owner projection read-model
/// rows; liveness still comes from `PresenceRegistry`.
pub(crate) fn handle_resolve_at(
    request: &ResolveRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
    local_publication: Option<
        &crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot,
    >,
    now_unix_ms: i64,
) -> Result<ResolveResponse, String> {
    let prefix = request.effective_ura_prefix();
    let want_abilities = request.wants_abilities();
    let mut agents = std::collections::BTreeMap::<String, ResolveAgentSummary>::new();
    let live_uras: std::collections::BTreeSet<String> = registry.snapshot().into_iter().collect();

    for ura in &live_uras {
        if prefix.is_some_and(|p| !ura.starts_with(p)) {
            continue;
        }
        let abilities = if want_abilities {
            resolved_owner_projection_values(catalog, local_publication, ura, now_unix_ms)?
        } else {
            Vec::new()
        };
        agents.insert(
            ura.clone(),
            ResolveAgentSummary {
                ura: ura.clone(),
                status: "active".to_string(),
                host_node_id: None,
                abilities,
            },
        );
    }

    if let Some(store) = advertised_agents {
        for record in store.snapshot() {
            let is_online = match record.host_ura() {
                Some(host_ura) => live_uras.contains(host_ura),
                None => live_uras.contains(&record.agent_ura),
            };
            if !is_online {
                continue;
            }
            if prefix.is_some_and(|p| !record.agent_ura.starts_with(p)) {
                continue;
            }
            let abilities = if want_abilities {
                resolved_owner_projection_values(
                    catalog,
                    local_publication,
                    &record.agent_ura,
                    now_unix_ms,
                )?
            } else {
                Vec::new()
            };
            agents
                .entry(record.agent_ura.clone())
                .and_modify(|summary| {
                    if summary.host_node_id.is_none() {
                        summary.host_node_id = record.host_node_id.clone();
                    }
                    if summary.abilities.is_empty() {
                        summary.abilities = abilities.clone();
                    }
                })
                .or_insert(ResolveAgentSummary {
                    ura: record.agent_ura,
                    status: "active".to_string(),
                    host_node_id: record.host_node_id,
                    abilities,
                });
        }
    }

    for row in catalog.projection_rows_for_live_hosts_at(&live_uras, now_unix_ms) {
        if prefix.is_some_and(|p| !row.owner_ura().starts_with(p)) {
            continue;
        }
        let Some(host_node_id) =
            device_sponsored_system_agent_host_node_id(row.owner_ura(), row.host_device_ura())
        else {
            continue;
        };
        let abilities = if want_abilities {
            resolved_owner_projection_values(
                catalog,
                local_publication,
                row.owner_ura(),
                now_unix_ms,
            )?
        } else {
            Vec::new()
        };
        agents
            .entry(row.owner_ura().to_string())
            .or_insert(ResolveAgentSummary {
                ura: row.owner_ura().to_string(),
                status: "active".to_string(),
                host_node_id: Some(host_node_id),
                abilities,
            });
    }

    Ok(ResolveResponse {
        agents: agents.into_values().collect(),
    })
}

fn device_sponsored_system_agent_host_node_id(
    owner_ura: &str,
    host_device_ura: &str,
) -> Option<String> {
    let owner = crate::core::ura::parse_ura(owner_ura).ok()?;
    let host = crate::core::ura::parse_ura(host_device_ura).ok()?;
    if host.kind != crate::core::ura::URAKind::Device || host.realm != owner.realm {
        return None;
    }
    let host_device_id = host.device_id()?;
    if owner.kind != crate::core::ura::URAKind::Agent {
        return None;
    }
    let (owner_device_id, system_agent_id) = owner.device_agent_ids()?;
    if !crate::daemon::ability::catalog::profiles::is_declared_daemon_native_system_agent_id(
        system_agent_id,
    ) {
        return None;
    }
    (host_device_id == owner_device_id).then(|| host_device_id.to_string())
}

/// Handle RFC-005 `namespace.resolve` using daemon-owned runtime state.
///
/// The returned value follows Axon proto-JSON field names and enum strings.
/// CLI owns the read model and route feasibility decision here; Axon owns the
/// generated enum/message vocabulary used to serialize the answer.
#[must_use]
pub fn handle_namespace_resolve(
    query: &Value,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
) -> Value {
    handle_namespace_resolve_at(
        query,
        registry,
        advertised_agents,
        catalog,
        crate::daemon::federation::directory::now_unix_ms(),
    )
}

#[must_use]
pub(crate) fn handle_namespace_resolve_at(
    query: &Value,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
    now_unix_ms: i64,
) -> Value {
    if let Err(error) = validate_namespace_resolve_query(query) {
        return namespace_resolve_input_failure(query, &error);
    }
    crate::daemon::invocation::routing::route_resolver::DaemonRouteResolver::new(
        registry,
        advertised_agents,
        catalog,
    )
    .at(now_unix_ms)
    .resolve_query_json(query)
}

fn validate_namespace_resolve_query(query: &Value) -> Result<(), String> {
    NamespaceResolveQuery::from_json(query).map(|_| ())
}

fn namespace_resolve_input_failure(query: &Value, detail: &str) -> Value {
    let query_name = query
        .get("query_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    serde_json::json!({
        "answer_kind": ResolveAnswerKind::Negative.as_str_name(),
        "next_hop": {
            "no_route": {}
        },
        "records": [],
        "release_profile": ResolverReleaseProfile::AuthoritativeLocal.as_str_name(),
        "authority": crate::daemon::invocation::routing::route_resolver::authority_for_query(query_name),
        "cache_policy": {
            "ttl_ms": 0,
            "shared_cacheable": false,
            "retry_after_unix_ms": 0,
        },
        "negative": {
            "reason": NegativeReason::Refused.as_str_name(),
            "query_name": query_name,
            "detail": detail,
        }
    })
}

/// Namespace-safe ability summaries for one in-presence owner. Local rows
/// come from the process's immutable live-catalog snapshot; remote rows come
/// from the lease-filtered owner projection store. The owner key controls the
/// merge, so a local snapshot cannot fabricate rows for a remote device.
fn resolved_owner_projection_values(
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
    local_publication: Option<
        &crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot,
    >,
    owner_ura: &str,
    now_unix_ms: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let mut by_public_name = std::collections::BTreeMap::<String, serde_json::Value>::new();
    let mut order = Vec::new();
    let mut push = |summary: serde_json::Value| -> Result<(), String> {
        let parsed =
            match crate::daemon::federation::read_model::owner_projection::summary_from_value(
                &summary,
            ) {
                Some(parsed) => parsed,
                None => {
                    return Err(owner_projection_summary_error(
                        owner_ura,
                        "contains invalid ability summary",
                        &summary,
                    ));
                }
            };
        let Some(key) =
            crate::daemon::federation::read_model::owner_projection::summary_public_name(&parsed)
        else {
            return Err(owner_projection_summary_error(
                owner_ura,
                "contains ability summary without public name",
                &summary,
            ));
        };
        if by_public_name.insert(key.clone(), summary).is_none() {
            order.push(key);
        }
        Ok(())
    };

    if let Some(local_publication) = local_publication {
        for summary in local_publication.owner_projection_values(owner_ura)? {
            push(summary)?;
        }
    }
    for summary in catalog.get_at(owner_ura, now_unix_ms).unwrap_or_default() {
        push(summary)?;
    }

    Ok(order
        .into_iter()
        .filter_map(|key| by_public_name.remove(&key))
        .collect())
}

fn owner_projection_summary_error(
    owner_ura: &str,
    reason: &str,
    summary: &serde_json::Value,
) -> String {
    format!("owner projection for `{owner_ura}` {reason}: {summary}")
}

// ─── federation.resolve_key ────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveKeyResponseError {
    InvalidPublicKeyBase64(String),
    InvalidPublicKeyLength(usize),
}

impl std::fmt::Display for ResolveKeyResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPublicKeyBase64(error) => {
                write!(f, "public_key_b64 is not valid base64: {error}")
            }
            Self::InvalidPublicKeyLength(len) => write!(
                f,
                "public_key_b64 must decode to exactly {ED25519_PUBLIC_KEY_LEN} bytes, got {len}"
            ),
        }
    }
}

impl std::error::Error for ResolveKeyResponseError {}

fn decode_resolve_key_public_key(public_key_b64: &str) -> Result<Vec<u8>, ResolveKeyResponseError> {
    let public_key = BASE64_STANDARD
        .decode(public_key_b64.as_bytes())
        .map_err(|error| ResolveKeyResponseError::InvalidPublicKeyBase64(error.to_string()))?;
    if public_key.len() != ED25519_PUBLIC_KEY_LEN {
        return Err(ResolveKeyResponseError::InvalidPublicKeyLength(
            public_key.len(),
        ));
    }
    Ok(public_key)
}

pub(crate) fn resolve_key_response(
    public_key_b64: &str,
    all_keys_b64: Vec<String>,
    principal_owner: Option<&crate::daemon::trust::anchor::TrustedPrincipalOwner>,
) -> Result<ResolveKeyResponse, ResolveKeyResponseError> {
    let public_key_hex = hex::encode(decode_resolve_key_public_key(public_key_b64)?);
    Ok(ResolveKeyResponse {
        public_key_b64: public_key_b64.to_string(),
        public_key_hex,
        public_keys_b64: if all_keys_b64.is_empty() {
            vec![public_key_b64.to_string()]
        } else {
            all_keys_b64
        },
        principal_owner_ura: principal_owner.map(|owner| owner.owner_ura.clone()),
        principal_owner_user_id: principal_owner.map(|owner| owner.owner_user_id.clone()),
    })
}

/// Every key registered under `agent_ura` when it is a multi-key user
/// URA; empty for single-key roles (device/backend/hub), letting
/// `resolve_key_response` fall back to the primary key.
fn all_user_keys_b64(
    trust_anchor: &crate::daemon::trust::anchor::RealmTrustAnchor,
    agent_ura: &str,
) -> Vec<String> {
    trust_anchor
        .lookup_user_all(agent_ura)
        .iter()
        .map(|e| e.public_key_b64.clone())
        .collect()
}

/// Handle a `federation.resolve_key` invocation.
///
/// Looks up `agent_ura` in the supplied trust anchor snapshot and
/// returns its `public_key_b64` verbatim (the local trust file
/// already stores the canonical base64 form, so no re-encode is
/// needed here). On miss, returns `None`; the caller is responsible
/// for wrapping that as `Status::not_found` so the FederatedKey-
/// Resolver can distinguish "URA is not in this hub's trust set"
/// from a network-level failure.
pub fn handle_resolve_key(
    request: &ResolveKeyRequest,
    trust_anchor: &crate::daemon::trust::anchor::RealmTrustAnchor,
) -> Result<Option<ResolveKeyResponse>, ResolveKeyResponseError> {
    // DEC-EU multi-device user URAs: caller supplies the pubkey it
    // observed on the envelope; we confirm it's in the user bucket.
    // Singleton roles (hub/backend/device) ignore this field and
    // fall through to exact singleton trust-anchor lookup below.
    let presented_pubkey_b64 = request
        .presented_pubkey_b64
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(pk) = presented_pubkey_b64.as_deref() {
        if let Some(entry) = trust_anchor.lookup_user_by_pubkey(&request.agent_ura, pk) {
            return resolve_key_response(
                &entry.public_key_b64,
                all_user_keys_b64(trust_anchor, &request.agent_ura),
                trust_anchor.lookup_principal_owner(&request.agent_ura),
            )
            .map(Some);
        }
        if matches!(
            crate::core::ura::parse_ura(&request.agent_ura).map(|parsed| parsed.kind),
            Ok(crate::core::ura::URAKind::User)
        ) {
            return Ok(None);
        }
    }
    trust_anchor
        .lookup(&request.agent_ura)
        .map_or(Ok(None), |entry| {
            if let Some(pk) = presented_pubkey_b64.as_deref() {
                if entry.public_key_b64 != pk {
                    return Ok(None);
                }
            }
            resolve_key_response(
                &entry.public_key_b64,
                all_user_keys_b64(trust_anchor, &request.agent_ura),
                trust_anchor.lookup_principal_owner(&request.agent_ura),
            )
            .map(Some)
        })
}

// ─── federation.discover (PR-N3 N3-4) ──────────────────────────────

/// Handle a `federation.discover` invocation. Pure read against
/// the supplied `SharedFederatedDirectoryView` snapshot — no I/O,
/// no async — so the dispatcher can call it inline.
#[must_use]
pub fn handle_discover(
    request: &DiscoverRequest,
    view: &crate::daemon::federation::directory::SharedFederatedDirectoryView,
) -> DiscoverResponse {
    let entries = match request.agent_ura.as_deref() {
        Some(ura) => crate::daemon::federation::directory::lookup_in_federated_view(view, ura)
            .map(|e| vec![e])
            .unwrap_or_default(),
        None => crate::daemon::federation::directory::flatten_federated_view(view),
    };
    DiscoverResponse { entries }
}

/// **PR-N4 N3-N4 bridge**. Variant of `handle_discover` that
/// filters cross-realm entries through a `FederatedUserResolver`.
/// Only entries whose URA either:
///   - matches the local realm (`FederatedUserOutcome::Local`), or
///   - has a recorded binding for the calling user
///     (`BoundLocalUser`)
/// pass through. Unbound (`NotBound`) and malformed
/// (`Malformed`) URAs are filtered out.
///
/// This realises PR-N4 spec §commit 4/N's INV-5 privacy default:
/// a calling user only sees cross-realm devices that have been
/// explicitly opted into by a `device.keyring.consume_federate_
/// user_token` round on this hub.
#[must_use]
pub fn handle_discover_with_user_filter(
    request: &DiscoverRequest,
    view: &crate::daemon::federation::directory::SharedFederatedDirectoryView,
    resolver: &crate::daemon::keyring::resolver::FederatedUserResolver,
) -> DiscoverResponse {
    use crate::daemon::keyring::resolver::FederatedUserOutcome;
    let raw = match request.agent_ura.as_deref() {
        Some(ura) => crate::daemon::federation::directory::lookup_in_federated_view(view, ura)
            .map(|e| vec![e])
            .unwrap_or_default(),
        None => crate::daemon::federation::directory::flatten_federated_view(view),
    };
    let entries = raw
        .into_iter()
        .filter(|entry| {
            matches!(
                resolver.resolve_user(&entry.agent_ura),
                FederatedUserOutcome::Local | FederatedUserOutcome::BoundLocalUser(_)
            )
        })
        .collect();
    DiscoverResponse { entries }
}

// ─── federation.list_user_devices (PR-N3 N3-5) ────────────────────

/// Request payload for `federation.proxy_list_user_devices`.
/// `realm` is the device realm to enumerate on each selected
/// peer; `peer_hub_urls` are the exact peer TLS listener URLs
/// selected by the caller's directory/read-model policy.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyListUserDevicesRequest {
    pub realm: String,
    #[serde(default)]
    pub peer_hub_urls: Vec<String>,
}

/// Response payload for `federation.proxy_list_user_devices`.
/// The daemon stamps each returned `DirectoryEntry` with the
/// peer's `origin_realm` and `hub_endpoint` before returning it
/// to the backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyListUserDevicesResponse {
    pub devices: Vec<crate::daemon::federation::directory::DirectoryEntry>,
}

/// Request payload for `namespace.proxy_resolve`.
///
/// `peer_hub_urls` is product-selected fanout scope. The remaining fields are
/// forwarded verbatim to each peer's daemon-local `namespace.resolve` surface;
/// the proxy does not reinterpret resolver semantics.
#[derive(Debug, Clone, Serialize)]
pub struct NamespaceProxyResolveRequest {
    #[serde(default)]
    pub peer_hub_urls: Vec<String>,
    #[serde(rename = "query_name")]
    pub query_name: String,
    #[serde(rename = "qtype")]
    pub qtype: String,
    #[serde(rename = "caller_ura")]
    pub caller_ura: String,
    #[serde(rename = "subject_ura")]
    pub subject_ura: String,
    #[serde(rename = "realm_hint")]
    pub realm_hint: String,
    #[serde(rename = "ability_name")]
    pub ability_name: ExplicitOptionalAbilityName,
}

impl<'de> Deserialize<'de> for NamespaceProxyResolveRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            #[serde(default)]
            peer_hub_urls: Vec<String>,
            #[serde(rename = "query_name")]
            query_name: String,
            #[serde(rename = "qtype")]
            qtype: String,
            #[serde(rename = "caller_ura")]
            caller_ura: String,
            #[serde(rename = "subject_ura")]
            subject_ura: String,
            #[serde(rename = "realm_hint")]
            realm_hint: String,
            #[serde(rename = "ability_name")]
            ability_name: serde_json::Value,
        }

        let fields = Fields::deserialize(deserializer)?;
        let ability_name = ExplicitOptionalAbilityName::deserialize(fields.ability_name)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            peer_hub_urls: fields.peer_hub_urls,
            query_name: fields.query_name,
            qtype: fields.qtype,
            caller_ura: fields.caller_ura,
            subject_ura: fields.subject_ura,
            realm_hint: fields.realm_hint,
            ability_name,
        })
    }
}

/// Required selector slot for `namespace.proxy_resolve`.
///
/// `None` is an explicit `null` selector for directory/listing queries. Missing
/// fields never deserialize into this type, so public ingress cannot silently
/// default a resolver selector. Empty strings are rejected; a caller that wants
/// no separate owner-local ability selector must send JSON `null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitOptionalAbilityName(Option<String>);

impl ExplicitOptionalAbilityName {
    #[must_use]
    pub fn peer_argument(&self) -> Option<String> {
        self.0.as_deref().map(str::trim).map(str::to_string)
    }
}

impl Serialize for ExplicitOptionalAbilityName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExplicitOptionalAbilityName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Option::<String>::deserialize(deserializer)?;
        if value.as_deref().is_some_and(|raw| raw.trim().is_empty()) {
            return Err(serde::de::Error::custom(
                "ability_name must be null or a non-empty string",
            ));
        }
        Ok(Self(value))
    }
}

/// Handle a `federation.list_user_devices` invocation. Reads
/// the supplied `PresenceRegistry` snapshot, filters URAs
/// whose realm component matches `request.realm`, and
/// projects each into a `DirectoryEntry`. PR-N3 spec §3.5: the
/// admission filter (caller must be a trusted hub-role peer)
/// is enforced by the dispatcher *before* this handler runs;
/// the handler is pure data shaping.
///
/// `display_name` / `hub_endpoint` / `last_seen_unix_ms` are
/// `None` in this baseline projection — the daemon's
/// PresenceRegistry only knows URAs and active/inactive state.
/// Enriching from the backend's device_pairing table (with
/// real display_name, last_seen) is N3-6 backend-Go territory.
///
/// Canonical device-session projection:
/// - Canonical v4.1.4 device sessions live under
///   `easynet:///r/<realm>/device/<node>`.
/// - Only canonical device-session URAs (`.../device/<node>`) are
///   surfaced here.
/// - Real agent-profile URAs (`.../agent/<user>.<agent>`) are not
///   device sessions and are ignored here.
pub fn handle_list_user_devices(
    request: &ListUserDevicesRequest,
    registry: &PresenceRegistry,
) -> Result<ListUserDevicesResponse, String> {
    let realm = request.realm.trim();
    if realm.is_empty() {
        return Err("federation.list_user_devices: realm is required".to_string());
    }
    let snapshot = registry.snapshot();
    let mut devices = Vec::new();
    for ura in snapshot {
        if let Some(entry) = list_user_devices_presence_entry(&ura, realm)? {
            devices.push(entry);
        }
    }
    Ok(ListUserDevicesResponse { devices })
}

pub(crate) fn validate_list_user_devices_response(
    response: &ListUserDevicesResponse,
    source: &str,
) -> Result<(), String> {
    for (index, device) in response.devices.iter().enumerate() {
        let parsed = crate::core::ura::parse_ura(&device.agent_ura).map_err(|error| {
            format!(
                "{source}: devices[{index}].agent_ura {:?} is not canonical: {error}",
                device.agent_ura
            )
        })?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            return Err(format!(
                "{source}: devices[{index}].agent_ura {:?} is not a Device URA",
                device.agent_ura
            ));
        }
        let node_id = parsed
            .device_id()
            .map(str::trim)
            .filter(|node_id| !node_id.is_empty())
            .ok_or_else(|| {
                format!(
                    "{source}: devices[{index}].agent_ura {:?} is missing canonical device id",
                    device.agent_ura
                )
            })?;
        let canonical_ura = crate::core::ura::device_ura(&parsed.realm, node_id);
        if canonical_ura != device.agent_ura {
            return Err(format!(
                "{source}: devices[{index}].agent_ura {:?} is not canonical; expected {:?}",
                device.agent_ura, canonical_ura
            ));
        }
        if device.node_id.trim() != node_id {
            return Err(format!(
                "{source}: devices[{index}].node_id {:?} does not match Device URA id {:?}",
                device.node_id, node_id
            ));
        }
        if device.status.trim().is_empty() {
            return Err(format!("{source}: devices[{index}].status is empty"));
        }
    }
    Ok(())
}

fn list_user_devices_presence_entry(
    ura: &str,
    requested_realm: &str,
) -> Result<Option<crate::daemon::federation::directory::DirectoryEntry>, String> {
    let parsed = match crate::core::ura::parse_ura(ura) {
        Ok(parsed) => parsed,
        Err(error) => {
            let realm_device_prefix = crate::core::ura::realm_device_prefix(requested_realm);
            if ura.starts_with(&realm_device_prefix) {
                return Err(format!(
                    "federation.list_user_devices: presence row {ura:?} matches realm device prefix but is not a canonical Device URA: {error}"
                ));
            }
            return Ok(None);
        }
    };
    if parsed.realm != requested_realm || parsed.kind != crate::core::ura::URAKind::Device {
        return Ok(None);
    }
    let node_id = parsed
        .device_id()
        .map(str::trim)
        .filter(|node_id| !node_id.is_empty())
        .ok_or_else(|| {
            format!(
                "federation.list_user_devices: presence row {ura:?} is missing canonical device id"
            )
        })?;
    let canonical_ura = crate::core::ura::device_ura(&parsed.realm, node_id);
    if canonical_ura != ura {
        return Err(format!(
            "federation.list_user_devices: presence row {ura:?} is not canonical; expected {canonical_ura:?}"
        ));
    }
    Ok(Some(crate::daemon::federation::directory::DirectoryEntry {
        agent_ura: ura.to_string(),
        node_id: node_id.to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }))
}

// ─── federation.revoke ─────────────────────────────────────────────

/// Request payload for `federation.revoke`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeRequest {
    /// Canonical URA of the Device/Agent/User membership to revoke.
    pub agent_ura: String,
    #[serde(default)]
    pub purge_transaction_id: Option<String>,
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub authority_ura: Option<String>,
    #[serde(default)]
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub delivery_fence: Option<u64>,
}

impl RevokeRequest {
    fn canonical_target_ura(&self) -> anyhow::Result<String> {
        let target = self.agent_ura.trim();
        if target.is_empty() {
            anyhow::bail!("federation.revoke agent_ura is required");
        }
        crate::core::ura::parse_ura(target)
            .map_err(|error| anyhow::anyhow!("federation.revoke agent_ura is invalid: {error}"))?;
        Ok(target.to_string())
    }

    fn resolve_intent(&self) -> anyhow::Result<ResolvedRevokeIntent> {
        let target_ura = self.canonical_target_ura()?;
        let Some(transaction_id) = self
            .purge_transaction_id
            .as_deref()
            .map(str::trim)
            .filter(|transaction_id| !transaction_id.is_empty())
        else {
            return Ok(ResolvedRevokeIntent::Immediate { target_ura });
        };
        Ok(ResolvedRevokeIntent::Purge {
            target_ura,
            transaction_id: transaction_id.to_string(),
            generation: require_purge_revoke_fact(self.generation, "generation")?,
            reason: require_purge_revoke_text(self.reason.as_deref(), "reason")?,
            authority_ura: require_purge_revoke_text(
                self.authority_ura.as_deref(),
                "authority_ura",
            )?,
            protocol_version: require_purge_revoke_fact(self.protocol_version, "protocol_version")?,
            delivery_fence: require_purge_revoke_fact(self.delivery_fence, "delivery_fence")?,
        })
    }

    pub(crate) fn bind_to_subject(self, subject_ura: &str) -> anyhow::Result<AdmittedRevokeIntent> {
        let target_ura = self.canonical_target_ura()?;
        let subject_ura = subject_ura.trim();
        if subject_ura.is_empty() || subject_ura != target_ura {
            anyhow::bail!(
                "federation.revoke envelope subject must equal the canonical request target"
            );
        }
        Ok(AdmittedRevokeIntent { request: self })
    }
}

/// A revoke command whose mutable target is bound to the exact subject that
/// passed envelope admission. Constructing this value is the only production
/// path into the revoke handler, preventing policy-on-A / mutation-on-B
/// confused-deputy calls.
#[derive(Debug, Clone)]
pub(crate) struct AdmittedRevokeIntent {
    request: RevokeRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedRevokeIntent {
    Immediate {
        target_ura: String,
    },
    Purge {
        target_ura: String,
        transaction_id: String,
        generation: u64,
        reason: String,
        authority_ura: String,
        protocol_version: u32,
        delivery_fence: u64,
    },
}

impl ResolvedRevokeIntent {
    fn target_ura(&self) -> &str {
        match self {
            Self::Immediate { target_ura } | Self::Purge { target_ura, .. } => target_ura,
        }
    }

    fn generation(&self) -> Option<u64> {
        match self {
            Self::Immediate { .. } => None,
            Self::Purge { generation, .. } => Some(*generation),
        }
    }
}

fn require_purge_revoke_fact<T: Copy>(value: Option<T>, field: &str) -> anyhow::Result<T> {
    value.ok_or_else(|| anyhow::anyhow!("federation.revoke purge requires {field}"))
}

fn require_purge_revoke_text(value: Option<&str>, field: &str) -> anyhow::Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("federation.revoke purge requires {field}"))
}

/// Response payload for `federation.revoke`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RevokeResponse {
    /// Always `true` when this wrapper returns; deterministic.
    pub ack: bool,
    /// Whether the target was online at revoke time. Deterministic
    /// for byte-identical input + state.
    pub was_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purge_transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition:
        Option<crate::daemon::persistence::federation_revoke::FederationRevokeDisposition>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Presence side-effect policy for `federation.revoke`.
///
/// The ordinary administrative path removes the target presence immediately.
/// A device revoking its own runtime over `session.open` is different: the same
/// session is the only carrier for the canonical response. Destroying that
/// transport before Axon returns the terminal checkpoint wedges the caller.
/// In that self-revoke case the durable/read-model rows are still removed, and
/// the session reverse-dispatch lifecycle removes presence after it has queued
/// the canonical response frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevokePresenceMode<'a> {
    Immediate,
    DeferCurrentCaller { caller_ura: &'a str },
}

impl<'a> RevokePresenceMode<'a> {
    #[must_use]
    pub(crate) fn defer_current_caller(caller_ura: &'a str) -> Self {
        Self::DeferCurrentCaller { caller_ura }
    }

    #[must_use]
    fn should_remove_presence(self, target_ura: &str) -> bool {
        match self {
            Self::Immediate => true,
            Self::DeferCurrentCaller { caller_ura } => caller_ura != target_ura,
        }
    }
}

/// Handle a `federation.revoke` invocation. Forces removal of the
/// target session and records whether the target was online at
/// revoke time so the caller can distinguish a real revoke from a
/// no-op.
#[cfg(test)]
fn handle_revoke(
    request: &RevokeRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    ability_catalog: &AbilityCatalogStore,
) -> anyhow::Result<RevokeResponse> {
    let admitted = request.clone().bind_to_subject(&request.agent_ura)?;
    handle_revoke_with_presence_mode(
        &admitted,
        registry,
        advertised_agents,
        ability_catalog,
        RevokePresenceMode::Immediate,
    )
}

pub(crate) fn handle_revoke_with_presence_mode(
    admitted: &AdmittedRevokeIntent,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    ability_catalog: &AbilityCatalogStore,
    presence_mode: RevokePresenceMode<'_>,
) -> anyhow::Result<RevokeResponse> {
    let request = &admitted.request;
    let intent = request.resolve_intent()?;
    let target_ura = intent.target_ura().to_string();
    let purge_generation = intent.generation();
    let advertised_record = advertised_agents
        .and_then(|store| store.get(&target_ura))
        .filter(|record| purge_generation.is_none() || purge_generation == Some(record.generation));
    let target_generation_is_current = purge_generation.is_none()
        || advertised_record
            .as_ref()
            .is_some_and(|record| purge_generation == Some(record.generation));
    let was_active = target_generation_is_current
        && (registry.contains(&target_ura)
            || advertised_record
                .as_ref()
                .map(|record| match record.host_ura() {
                    Some(host_ura) => registry.contains(host_ura),
                    None => registry.contains(&record.agent_ura),
                })
                .unwrap_or(false));
    let ResolvedRevokeIntent::Purge {
        transaction_id,
        generation,
        reason,
        authority_ura,
        protocol_version,
        delivery_fence,
        ..
    } = intent
    else {
        if crate::daemon::persistence::federation_revoke::active_inventory_record(&target_ura)?
            .is_some()
        {
            anyhow::bail!(
                "hosted Agent revoke requires a durable transaction, generation, authority, protocol version, and delivery fence"
            );
        }
        if presence_mode.should_remove_presence(&target_ura) {
            let _displaced = registry.force_revoke(&target_ura);
        }
        if let Some(store) = advertised_agents {
            let _removed = store.remove(&target_ura);
        }
        let _removed = ability_catalog.remove_owner(&target_ura);
        return Ok(RevokeResponse {
            ack: true,
            was_active,
            purge_transaction_id: None,
            replayed: false,
            disposition: None,
        });
    };
    let command = crate::daemon::persistence::federation_revoke::FederationRevokeCommand {
        protocol_version,
        transaction_id: transaction_id.clone(),
        agent_ura: target_ura.clone(),
        generation,
        reason,
        authority_ura,
        target_ura: target_ura.clone(),
    };
    let presence_session_id = target_generation_is_current
        .then(|| {
            registry
                .lookup_tracked(&target_ura)
                .map(|(session_id, _)| session_id)
        })
        .flatten();
    let now = checked_revoke_now_unix_ms()?;
    let prepared = crate::daemon::persistence::federation_revoke::prepare_revoke(
        &command,
        delivery_fence,
        was_active,
        presence_session_id,
        now,
    )?;
    let (outcome, replayed) = match prepared {
        crate::daemon::persistence::federation_revoke::PrepareRevokeOutcome::Applied(outcome) => {
            (outcome, true)
        }
        crate::daemon::persistence::federation_revoke::PrepareRevokeOutcome::Prepared => {
            crate::daemon::persistence::federation_revoke::apply_prepared_revoke(
                &transaction_id,
                delivery_fence,
                now,
            )?
        }
    };
    if outcome.disposition
        != crate::daemon::persistence::federation_revoke::FederationRevokeDisposition::SupersededByNewIncarnation
    {
        if let Some(store) = advertised_agents {
            let _removed = store.remove_generation(&target_ura, generation);
        }
        let _removed = ability_catalog.remove_generation(&target_ura, generation);
        if presence_mode.should_remove_presence(&target_ura) {
            if let Some(session_id) = outcome.presence_session_id {
            let _removed = registry.remove_if_session(
                &target_ura,
                session_id,
                crate::daemon::invocation::bidi::state::presence::OfflineReason::AdminRevoked,
            );
            }
        }
    }
    Ok(RevokeResponse {
        ack: true,
        was_active: outcome.was_active,
        purge_transaction_id: Some(transaction_id),
        replayed,
        disposition: Some(outcome.disposition),
    })
}

fn checked_revoke_now_unix_ms() -> anyhow::Result<u64> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("system clock precedes Unix epoch: {error}"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| anyhow::anyhow!("system clock milliseconds exceed durable u64 range"))
}

// Reason text emitted on `Status::failed_precondition` when the
// target presence-registry lookup misses on the local-realm fast-path.
// Wire-stable per DEC-N4 §2.1.

// Reason text emitted when the target device's dispatch channel is full. A
// full channel means the device is SLOW (its session drain is behind), not
// DEAD: the device stays in the presence registry and only the triggering call
// fails, retryable. Evicting on full — the pre-2026-06-13 policy — turned a
// load spike into a false offline plus a failure avalanche for every pending
// call (measured: one >256-frame burst killed 73% of 2048 in-flight
// invocations).

/// **PR-N3 N3-streaming-1**. Build the initial `Snapshot` frame
/// for the v2 subscribe stream from the local presence registry.
/// Each in-registry URA projects to a `DirectoryAgentSummary` via
/// the pure-data adapter; sorted iteration keeps deterministic bytes
/// for deterministic state.
pub fn build_subscribe_directory_v2_snapshot(
    registry: &PresenceRegistry,
) -> Result<crate::daemon::federation::directory::DirectoryEvent, String> {
    crate::daemon::federation::directory::presence_uras_to_directory_snapshot(
        registry.snapshot(),
        crate::daemon::federation::directory::now_unix_ms(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_dispatch_sender() -> crate::daemon::invocation::bidi::state::presence::DispatchSender {
        let (tx, _rx) = mpsc::channel(256);
        tx
    }

    fn insert_presence(registry: &PresenceRegistry, ura: impl Into<String>) {
        registry
            .insert_negotiated(
                ura.into(),
                make_dispatch_sender(),
                crate::daemon::invocation::bidi::state::presence::SessionContract::new(
                    crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
                    vec![0; 16],
                ),
            )
            .expect("canonical presence key");
    }

    fn projection_summary(
        owner_ura: &str,
        ability_ura: &str,
        namespace: &str,
        local_name: &str,
    ) -> crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
        let public_name = if namespace.is_empty() {
            local_name.to_string()
        } else {
            format!("{namespace}.{local_name}")
        };
        let descriptor_revision = format!("sha256:{}", "a".repeat(64));
        let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{ability_ura}@1.0.0#{}!read",
            "a".repeat(64)
        ))
        .expect("test descriptor_ref");
        let mut callable_summary =
            crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                public_name,
            );
        callable_summary.mode_geometry.push(
            crate::daemon::federation::read_model::owner_projection::AbilityCallModeGeometry {
                call_mode: crate::daemon::ability::CallMode::Rpc,
                descriptor_ref,
                descriptor_version: "1.0.0".to_string(),
                descriptor_revision: descriptor_revision.clone(),
                admission_action: "read".to_string(),
                schema_hash: format!("sha256:{}", "b".repeat(64)),
                policy_ref: "visibility:SCOPED".to_string(),
                policy_hash: format!("sha256:{}", "c".repeat(64)),
                description: local_name.to_string(),
                receipt_semantics: crate::daemon::ability::ReceiptSemantics::Operational,
                input_fields: Vec::new(),
                flags: crate::daemon::federation::read_model::owner_projection::AbilityCallableFlags::default(),
                tags: vec!["class:unary".to_string()],
            },
        );
        crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
            ability_ura: ability_ura.to_string(),
            owner_ura: owner_ura.to_string(),
            namespace: namespace.to_string(),
            local_name: local_name.to_string(),
            descriptor_revision,
            schema_ref: None,
            schema_hash: None,
            policy_ref: "visibility:SCOPED".to_string(),
            route_summary_ref: Some(format!("route-ref::{ability_ura}")),
            tags: vec!["class:unary".to_string()],
            callable_summary,
        }
    }

    fn projection_row_for(
        owner_ura: &str,
        summaries: Vec<
            crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary,
        >,
    ) -> crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow {
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            "easynet:///r/realm/device/dev-1".to_string(),
            1,
            7,
            "sha256:projection".to_string(),
            4_102_444_800_000,
            summaries,
        )
    }

    fn hosted_agent_registration(
        agent_ura: &str,
        host_device_ura: &str,
        incarnation_hex: &str,
    ) -> crate::daemon::persistence::federation_revoke::HostedAgentRegistrationCommand {
        crate::daemon::persistence::federation_revoke::HostedAgentRegistrationCommand {
            agent_ura: agent_ura.to_string(),
            incarnation_id: crate::daemon::federation::hosted_agent_publication::HostedAgentIncarnationId::parse(
                incarnation_hex,
            )
            .unwrap(),
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".to_string()),
            signing_authority:
                crate::daemon::persistence::federation_revoke::DurableSigningAuthority::HostedBy {
                    host_ura: host_device_ura.to_string(),
                },
        }
    }

    fn register_into_read_model(
        command: crate::daemon::persistence::federation_revoke::HostedAgentRegistrationCommand,
        store: &AdvertisedAgentStore,
    ) -> AdvertiseAgentResponse {
        let registration = register_advertised_agent(command).expect("Hub registration");
        let outcome = store.upsert(registration.record.clone().into());
        assert!(outcome.is_stored(), "authoritative result projection");
        advertise_agent_response(registration)
    }

    #[test]
    fn ability_name_constants_match_spec_section_4() {
        // These constants are canonical descriptor/publication identifiers;
        // changing them requires an intentional descriptor revision.
        assert_eq!(ABILITY_FEDERATION_JOIN, "federation.join");
        assert_eq!(
            ABILITY_FEDERATION_ADVERTISE_AGENT,
            "federation.advertise_agent"
        );
        assert_eq!(ABILITY_FEDERATION_HEARTBEAT, "federation.heartbeat");
        assert_eq!(ABILITY_FEDERATION_RESOLVE, "federation.resolve");
        assert_eq!(ABILITY_NAMESPACE_RESOLVE, "namespace.resolve");
        assert_eq!(ABILITY_NAMESPACE_PROXY_RESOLVE, "namespace.proxy_resolve");
        assert_eq!(ABILITY_FEDERATION_REVOKE, "federation.revoke");
        assert_eq!(ABILITY_FEDERATION_RESOLVE_KEY, "federation.resolve_key");
        assert_eq!(ABILITY_FEDERATION_DISCOVER, "federation.discover");
        assert_eq!(
            ABILITY_FEDERATION_LIST_USER_DEVICES,
            "federation.list_user_devices"
        );
        assert_eq!(
            ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
            "federation.proxy_list_user_devices"
        );
        assert_eq!(
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
            "federation.subscribe_directory_v2"
        );
        assert_eq!(
            ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            "federation.advertise_abilities"
        );
        assert_eq!(ABILITY_FEDERATION_STATUS, "federation.status");
        assert_eq!(
            ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            "runtime.bootstrap_self_identity"
        );
        // `namespace.*` resolver surfaces live outside the federation ability
        // set. `runtime.bootstrap_self_identity` is namespaced under
        // `runtime.*`, so it also stays outside this list.
        assert_eq!(FEDERATION_ABILITIES.len(), 12);
        assert!(
            !FEDERATION_ABILITIES.contains(&"aggregate.list_abilities_catalog"),
            "backend/product aggregate alias must not be advertised as federation baseline"
        );
    }

    #[test]
    fn federation_ability_list_matches_hub_baseline_federation_plane() {
        let expected: std::collections::BTreeSet<&str> =
            conformance::HubBaseline::required_abilities()
                .iter()
                .filter(|ability| ability.domain == conformance::BaselineDomain::HubFederation)
                .map(|ability| ability.name)
                .collect();
        let actual: std::collections::BTreeSet<&str> =
            FEDERATION_ABILITIES.iter().copied().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn federation_status_projects_unjoined_canonical_snapshot() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let status = handle_status();

        assert_eq!(status["ok"], false);
        assert_eq!(status["code"], "disabled");
        assert_eq!(status["outcome"]["kind"], "disabled");
        assert_eq!(status["outcome"]["connection"]["state"], "PAIRING_NONE");
    }

    #[test]
    fn federation_status_projects_connected_canonical_snapshot() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let snapshot =
            crate::daemon::boot::join_connection_state::JoinConnectionSnapshot::from_parts(
                crate::daemon::boot::join_connection_state::JoinConnectionState::ConnectedOnline,
                Some(crate::daemon::boot::join_connection_state::JoinTransition::AdmitPresence),
                "realm-a",
                "device-a",
                Some("https://hub.example:50443".to_string()),
                "test",
            );
        crate::daemon::boot::join_connection_state::save_snapshot(&snapshot)
            .expect("save canonical join snapshot");

        let status = handle_status();

        assert_eq!(status["ok"], true);
        assert_eq!(status["code"], "installed");
        assert_eq!(status["outcome"]["kind"], "installed");
        assert_eq!(
            status["outcome"]["connection"]["state"],
            "FRONTEND_CONNECTED"
        );
        assert_eq!(
            status["outcome"]["device_ura"],
            "easynet:///r/realm-a/device/device-a"
        );
    }

    #[test]
    fn join_receipt_hash_is_deterministic() {
        let a = derive_join_receipt_hash("easynet:///r/realm/device/n1", "realm");
        let b = derive_join_receipt_hash("easynet:///r/realm/device/n1", "realm");
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
        // the spec §5.1 statement of `sha256(ura || realm)`.
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
    fn handle_join_echoes_ura_and_realm() {
        let req = JoinRequest {
            membership_ura: "easynet:///r/realm/device/n1".to_string(),
            realm: "realm".to_string(),
            public_key_hex: hex::encode(
                ed25519_dalek::SigningKey::from_bytes(&[0x42; 32])
                    .verifying_key()
                    .to_bytes(),
            ),
            principal_enrollment: None,
        };
        let resp = handle_join(&req);
        assert_eq!(resp.membership_ura, req.membership_ura);
        assert_eq!(resp.realm, req.realm);
        assert_eq!(resp.join_receipt_hash.len(), 64);
        assert!(resp.authority_published_abilities.is_empty());
        assert_eq!(resp.authority_abilities_revision, 0);
        assert_eq!(
            resp.advertise_contract.allowed_owner_prefixes,
            vec!["device.".to_string()]
        );
        assert!(resp.advertise_contract.allows_hosted_agents);
    }

    #[test]
    fn join_request_rejects_retired_pairing_secret_field() {
        let args = serde_json::json!({
            "membership_ura": "easynet:///r/realm/device/n1",
            "realm": "realm",
            "public_key_hex": hex::encode(
                ed25519_dalek::SigningKey::from_bytes(&[0x42; 32])
                    .verifying_key()
                    .to_bytes()
            ),
            "pairing_secret": "retired-token-carrier"
        });

        let err = serde_json::from_value::<JoinRequest>(args)
            .expect_err("retired pairing_secret must fail closed at the join parser");

        assert!(
            err.to_string().contains("unknown field `pairing_secret`"),
            "join parser must name the retired field: {err}"
        );
    }

    #[test]
    fn register_advertised_agent_returns_exact_generation_assignment() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let store = AdvertisedAgentStore::new();
        let agent_ura = "easynet:///r/realm/agent/user.n1";
        let host_ura = "easynet:///r/realm/device/dev-1";
        let incarnation_id = "11111111111111111111111111111111";
        let resp = register_into_read_model(
            hosted_agent_registration(agent_ura, host_ura, incarnation_id),
            &store,
        );
        assert!(resp.ack);
        assert_eq!(resp.assignment.agent_ura, agent_ura);
        assert_eq!(resp.assignment.host_device_ura, host_ura);
        assert_eq!(resp.assignment.incarnation_id.as_str(), incarnation_id);
        assert_eq!(resp.assignment.generation, 1);
        let stored = store
            .get(agent_ura)
            .expect("advertised agent must be stored");
        assert_eq!(stored.host_ura(), Some(host_ura));
        assert_eq!(stored.generation, resp.assignment.generation);
    }

    #[test]
    fn advertise_agent_request_is_exactly_agent_and_incarnation() {
        let canonical = serde_json::json!({
            "agent_ura": "easynet:///r/realm/agent/user.n1",
            "incarnation_id": "11111111111111111111111111111111"
        });
        serde_json::from_value::<AdvertiseAgentRequest>(canonical.clone())
            .expect("canonical request");

        for retired in [
            "generation",
            "public_key_hex",
            "host_ura",
            "host_node_id",
            "signing_authority",
        ] {
            let mut legacy = canonical.clone();
            legacy
                .as_object_mut()
                .unwrap()
                .insert(retired.to_string(), serde_json::json!(1));
            let error = serde_json::from_value::<AdvertiseAgentRequest>(legacy)
                .expect_err("sender-assigned host/generation facts must fail closed");
            assert!(error.to_string().contains(retired));
        }
    }

    #[test]
    fn advertise_agent_request_requires_strict_incarnation_id() {
        let missing_incarnation = serde_json::json!({
            "agent_ura": "easynet:///r/realm/agent/user.n1",
        });
        let error = serde_json::from_value::<AdvertiseAgentRequest>(missing_incarnation)
            .expect_err("advertise_agent must require incarnation_id");
        assert!(error.to_string().contains("incarnation_id"));

        for invalid in ["A".repeat(32), "a".repeat(31), "g".repeat(32)] {
            let error = serde_json::from_value::<AdvertiseAgentRequest>(serde_json::json!({
                "agent_ura": "easynet:///r/realm/agent/user.n1",
                "incarnation_id": invalid
            }))
            .expect_err("invalid incarnation id must fail at wire decode");
            assert!(error.to_string().contains("lowercase hexadecimal"));
        }
    }

    #[test]
    fn advertise_agent_response_is_closed_and_reuses_the_assignment_value_object() {
        let canonical = serde_json::json!({
            "ack": true,
            "assignment": {
                "agent_ura": "easynet:///r/realm/agent/user.n1",
                "host_device_ura": "easynet:///r/realm/device/dev-1",
                "incarnation_id": "11111111111111111111111111111111",
                "generation": 1
            }
        });
        serde_json::from_value::<AdvertiseAgentResponse>(canonical.clone())
            .expect("canonical assignment response");

        let mut unknown = canonical.clone();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("generation".to_string(), serde_json::json!(1));
        assert!(serde_json::from_value::<AdvertiseAgentResponse>(unknown).is_err());

        let mut invalid_incarnation = canonical;
        invalid_incarnation["assignment"]["incarnation_id"] =
            serde_json::json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert!(serde_json::from_value::<AdvertiseAgentResponse>(invalid_incarnation).is_err());
    }

    #[test]
    fn advertise_agent_descriptor_pins_the_breaking_closed_wire_contract() {
        let descriptor: toml::Value = toml::from_str(include_str!(
            "../../../../ability-descriptors/system/federation/federation.advertise_agent.ability.toml"
        ))
        .expect("advertise_agent descriptor TOML");
        assert_eq!(descriptor["descriptor_version"].as_str(), Some("2.0.0"));

        let input = descriptor["input_schema"].as_table().unwrap();
        assert_eq!(input["additionalProperties"].as_bool(), Some(false));
        let required = input["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert_eq!(required[0].as_str(), Some("agent_ura"));
        assert_eq!(required[1].as_str(), Some("incarnation_id"));
        assert_eq!(input["properties"].as_table().unwrap().len(), 2);

        let output: serde_json::Value =
            serde_json::from_str(descriptor["output_receipt_schema_json"].as_str().unwrap())
                .expect("advertise_agent output schema JSON");
        assert_eq!(output["additionalProperties"], false);
        assert_eq!(
            output["properties"]["assignment"]["additionalProperties"],
            false
        );
        assert_eq!(
            output["properties"]["assignment"]["properties"]["incarnation_id"]["pattern"],
            "^[0-9a-f]{32}$"
        );
        assert_eq!(
            output["properties"]["assignment"]["properties"]["generation"]["minimum"],
            1
        );
    }

    #[test]
    fn handle_advertise_abilities_stores_owner_projection_row() {
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let owner_ura = "easynet:///r/realm/device/dev-1";
        let req = AdvertiseAbilitiesRequest {
            owner_ura: owner_ura.to_string(),
            host_device_ura: owner_ura.to_string(),
            generation: 1,
            projection_revision: 7,
            projection_digest: "sha256:projection".to_string(),
            lease_expires_unix_ms: 1_714_493_100_000,
            purge_delivery: None,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/device.dev-1.fs.read",
                "fs",
                "read",
            )],
        };

        let resp = handle_advertise_abilities(&req, &catalog);

        assert!(resp.ack);
        assert_eq!(resp.count, 1);
        let stored = catalog
            .projection_for_owner(owner_ura)
            .expect("owner projection row must be stored");
        assert_eq!(stored.owner_ura(), owner_ura);
        assert_eq!(stored.host_device_ura(), owner_ura);
        assert_eq!(stored.projection_revision(), 7);
        assert_eq!(stored.projection_digest(), "sha256:projection");
        assert_eq!(stored.lease_expires_unix_ms(), 1_714_493_100_000);
        assert_eq!(stored.ability_count(), 1);
        assert_eq!(stored.summaries_as_json()[0]["local_name"], "read");
    }

    #[test]
    fn handle_advertise_abilities_rejects_stale_projection_for_read_model() {
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let owner_ura = "easynet:///r/realm/device/dev-1";
        let newer = AdvertiseAbilitiesRequest {
            owner_ura: owner_ura.to_string(),
            host_device_ura: owner_ura.to_string(),
            generation: 1,
            projection_revision: 7,
            projection_digest: "sha256:newer".to_string(),
            lease_expires_unix_ms: 4_102_444_800_000,
            purge_delivery: None,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/device.dev-1.fs.write",
                "fs",
                "write",
            )],
        };
        let stale = AdvertiseAbilitiesRequest {
            owner_ura: owner_ura.to_string(),
            host_device_ura: owner_ura.to_string(),
            generation: 1,
            projection_revision: 6,
            projection_digest: "sha256:stale".to_string(),
            lease_expires_unix_ms: 4_102_444_800_000,
            purge_delivery: None,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/device.dev-1.fs.read",
                "fs",
                "read",
            )],
        };

        assert_eq!(
            handle_advertise_abilities(&newer, &catalog),
            AdvertiseAbilitiesResponse {
                ack: true,
                count: 1,
                outcome: Some("inserted".to_string()),
            }
        );
        assert_eq!(
            handle_advertise_abilities(&stale, &catalog),
            AdvertiseAbilitiesResponse {
                ack: false,
                count: 0,
                outcome: Some("ignored_stale".to_string()),
            }
        );

        let got = catalog.get_at(owner_ura, 1_714_493_100_000).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "write");
    }

    #[test]
    fn handle_advertise_abilities_reports_equal_revision_conflict_for_single_owner_row() {
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let owner_ura = "easynet:///r/realm/agent/alice.worker";
        let first = AdvertiseAbilitiesRequest {
            owner_ura: owner_ura.to_string(),
            host_device_ura: "easynet:///r/realm/device/provider".to_string(),
            generation: 1,
            projection_revision: 7,
            projection_digest: "sha256:provider".to_string(),
            lease_expires_unix_ms: 4_102_444_800_000,
            purge_delivery: None,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/alice.worker.project_list",
                "pages",
                "project_list",
            )],
        };
        let conflict = AdvertiseAbilitiesRequest {
            owner_ura: owner_ura.to_string(),
            host_device_ura: "easynet:///r/realm/device/caller".to_string(),
            generation: 1,
            projection_revision: 7,
            projection_digest: "sha256:caller".to_string(),
            lease_expires_unix_ms: 4_102_444_800_000,
            purge_delivery: None,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/alice.worker.project_create",
                "pages",
                "project_create",
            )],
        };

        assert_eq!(
            handle_advertise_abilities(&first, &catalog),
            AdvertiseAbilitiesResponse {
                ack: true,
                count: 1,
                outcome: Some("inserted".to_string()),
            }
        );
        assert_eq!(
            handle_advertise_abilities(&conflict, &catalog),
            AdvertiseAbilitiesResponse {
                ack: false,
                count: 0,
                outcome: Some("rejected_conflict".to_string()),
            }
        );

        let stored = catalog
            .projection_for_owner(owner_ura)
            .expect("first projection remains selected");
        assert_eq!(
            stored.host_device_ura(),
            "easynet:///r/realm/device/provider"
        );
        assert_eq!(stored.ability_count(), 1);
    }

    #[test]
    fn handle_heartbeat_reports_registry_size() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm/device/a".to_string());
        insert_presence(&registry, "easynet:///r/realm/device/b".to_string());
        let req = HeartbeatRequest {
            since_abilities_revision: 9,
            refresh_owner_uras: Vec::new(),
        };
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let resp = handle_heartbeat(&req, &registry, &catalog, 1_000);
        assert_eq!(resp.membership_status, "active");
        assert_eq!(resp.realm_directory_size, 2);
        assert_eq!(resp.authority_abilities_diff.revision, 9);
        assert!(resp.authority_abilities_diff.added.is_empty());
        assert!(resp.authority_abilities_diff.removed.is_empty());
        let wire = serde_json::to_value(&resp).expect("heartbeat response serializes");
        let mut keys = wire.as_object().expect("heartbeat response object").keys();
        assert_eq!(
            keys.next().map(String::as_str),
            Some("authority_abilities_diff")
        );
        assert_eq!(keys.next().map(String::as_str), Some("membership_status"));
        assert_eq!(
            keys.next().map(String::as_str),
            Some("realm_directory_size")
        );
        assert!(keys.next().is_none());
    }

    #[test]
    fn handle_heartbeat_renews_owner_projection_lease() {
        // The exact production bug: a DeviceProfileProjection row is published
        // with a lease, the lease expires, and migration/local projection
        // abilities silently drop out of `namespace.resolve` with NODATA.
        // Heartbeat must renew the lease so the projection stays resolvable
        // without a full re-advertise.
        let registry = PresenceRegistry::new();
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let host_device_ura = crate::core::ura::device_ura("realm", "a");
        let owner_ura = crate::core::ura::device_agent_ura("realm", "a", "test-runtime");
        insert_presence(&registry, host_device_ura.clone());

        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "device_profile.cursor")
            .expect("ability ura");
        let publish_at = 1_000_i64;
        let lease = crate::daemon::federation::read_model::owner_projection::lease_expiry_from_now(
            publish_at,
        );
        catalog.upsert_projection(
            crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                owner_ura.clone(),
                host_device_ura,
                1,
                1,
                "sha256:digest".to_string(),
                lease,
                vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
                    ability_ura: ability_ura.clone(),
                    owner_ura: owner_ura.clone(),
                    namespace: "device_profile".to_string(),
                    local_name: "cursor".to_string(),
                    descriptor_revision: "sha256:desc".to_string(),
                    schema_ref: None,
                    schema_hash: None,
                    policy_ref: "visibility:PUBLIC".to_string(),
                    route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                    tags: Vec::new(),
                    callable_summary:
                        crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                            "device_profile.cursor",
                        ),
                }],
            ),
        );

        // After the lease expires, the projection is filtered out.
        let after_expiry = lease + 1;
        assert!(catalog.get_at(&owner_ura, after_expiry).is_none());

        // A heartbeat at that moment renews the lease...
        let req = HeartbeatRequest {
            since_abilities_revision: 11,
            refresh_owner_uras: vec![owner_ura.clone()],
        };
        let resp = handle_heartbeat(&req, &registry, &catalog, after_expiry);
        assert_eq!(resp.authority_abilities_diff.revision, 11);

        // ...and the DeviceProfileProjection ability is resolvable again, with
        // its contents and revision unchanged (lease-only refresh).
        let got = catalog
            .get_at(&owner_ura, after_expiry)
            .expect("lease renewed");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "cursor");
        let row = catalog.projection_for_owner(&owner_ura).unwrap();
        assert_eq!(row.projection_revision(), 1);
        assert_eq!(row.projection_digest(), "sha256:digest");
    }

    #[test]
    fn handle_heartbeat_skips_unknown_owner() {
        let registry = PresenceRegistry::new();
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let req = HeartbeatRequest {
            since_abilities_revision: 0,
            refresh_owner_uras: vec!["easynet:///r/realm/device/never-published".to_string()],
        };
        let _ = handle_heartbeat(&req, &registry, &catalog, 5_000);
        assert!(
            catalog
                .projection_for_owner("easynet:///r/realm/device/never-published")
                .is_none(),
            "heartbeat must not synthesize missing owner projections"
        );
    }

    #[test]
    fn heartbeat_request_accepts_canonical_revision_and_refresh_batch() {
        let payload = serde_json::json!({
            "since_abilities_revision": 7,
            "refresh_owner_uras": ["easynet:///r/realm/device/a"],
        });
        let req: HeartbeatRequest =
            serde_json::from_value(payload).expect("device heartbeat payload must deserialize");
        assert_eq!(req.since_abilities_revision, 7);
        assert_eq!(req.refresh_owner_uras, vec!["easynet:///r/realm/device/a"]);
    }

    #[test]
    fn heartbeat_request_rejects_retired_identity_fields() {
        for (field, value) in [
            (
                "agent_ura",
                serde_json::json!("easynet:///r/realm/device/a"),
            ),
            ("node_id", serde_json::json!("device-a")),
            (
                "owner_ura",
                serde_json::json!("easynet:///r/realm/device/a"),
            ),
            ("generation", serde_json::json!(7)),
        ] {
            let mut payload = serde_json::json!({
                "since_abilities_revision": 7,
                "refresh_owner_uras": ["easynet:///r/realm/device/a"],
            });
            payload
                .as_object_mut()
                .expect("object")
                .insert(field.to_string(), value);
            let error = serde_json::from_value::<HeartbeatRequest>(payload)
                .expect_err("retired heartbeat field must fail closed");
            assert!(
                error.to_string().contains(field),
                "retired field {field} must be named in error: {error}"
            );
        }
    }

    #[test]
    fn handle_resolve_with_no_filter_returns_all_sorted() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm/device/c".to_string());
        insert_presence(&registry, "easynet:///r/realm/device/a".to_string());
        insert_presence(&registry, "easynet:///r/realm/device/b".to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let resp = handle_resolve(&ResolveRequest::all(), &registry, None, &catalog, None)
            .expect("resolve");
        let uras: Vec<&str> = resp.agents.iter().map(|a| a.ura.as_str()).collect();
        assert_eq!(
            uras,
            vec![
                "easynet:///r/realm/device/a",
                "easynet:///r/realm/device/b",
                "easynet:///r/realm/device/c",
            ]
        );
        for agent in &resp.agents {
            assert_eq!(agent.status, "active");
        }
    }

    #[test]
    fn handle_resolve_with_prefix_filters_correctly() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm-a/device/x".to_string());
        insert_presence(&registry, "easynet:///r/realm-b/device/y".to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let resp = handle_resolve(
            &ResolveRequest::with_filter(Some("easynet:///r/realm-a".to_string()), false),
            &registry,
            None,
            &catalog,
            None,
        )
        .expect("resolve");
        assert_eq!(resp.agents.len(), 1);
        assert_eq!(resp.agents[0].ura, "easynet:///r/realm-a/device/x");
    }

    #[test]
    fn handle_resolve_keeps_device_profile_projection_inventory_empty() {
        let registry = PresenceRegistry::new();
        let self_device_ura = "easynet:///r/realm/device/dev-1";
        insert_presence(&registry, self_device_ura.to_string());

        let local_publication =
            crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot::default(
            );
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let resp = handle_resolve_at(
            &ResolveRequest::with_filter(Some("easynet:///r/realm/device/".to_string()), true),
            &registry,
            None,
            &catalog,
            Some(&local_publication),
            1_000,
        )
        .expect("resolve");

        assert_eq!(resp.agents.len(), 1);
        let names: std::collections::BTreeSet<_> = resp.agents[0]
            .abilities
            .iter()
            .filter_map(|ability| {
                let namespace = ability.get("namespace")?.as_str()?;
                let local_name = ability.get("local_name")?.as_str()?;
                Some(format!("{namespace}.{local_name}"))
            })
            .collect();
        assert!(
            names.is_empty(),
            "DeviceProfileProjection is an empty migration cursor, got {names:?}"
        );
    }

    #[test]
    fn handle_resolve_does_not_fabricate_profile_for_remote_device() {
        // A hub resolving a remote device has no matching local catalog rows,
        // so it can only publish what that device advertised (here: nothing).
        let registry = PresenceRegistry::new();
        let remote_device = "easynet:///r/realm/device/dev-remote";
        insert_presence(&registry, remote_device.to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let resp = handle_resolve(
            &ResolveRequest::with_filter(Some("easynet:///r/realm/device/".to_string()), true),
            &registry,
            None,
            &catalog,
            None,
        )
        .expect("resolve");

        assert_eq!(resp.agents.len(), 1);
        assert!(
            resp.agents[0].abilities.is_empty(),
            "remote device profile must not be fabricated; got {:?}",
            resp.agents[0].abilities
        );
    }

    #[test]
    fn handle_resolve_includes_hosted_agent_when_host_device_is_online() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm/device/dev-1".to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(projection_row_for(
            "easynet:///r/realm/agent/user.alice",
            vec![projection_summary(
                "easynet:///r/realm/agent/user.alice",
                "easynet:///r/realm/ability/user.alice.chat",
                "",
                "chat",
            )],
        ));
        let advertised = AdvertisedAgentStore::new();
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/user.alice".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        });

        let resp = handle_resolve(
            &ResolveRequest::with_filter(Some("easynet:///r/realm/agent/".to_string()), true),
            &registry,
            Some(&advertised),
            &catalog,
            None,
        )
        .expect("resolve");
        assert_eq!(resp.agents.len(), 1);
        assert_eq!(resp.agents[0].ura, "easynet:///r/realm/agent/user.alice");
        assert_eq!(resp.agents[0].abilities.len(), 1);
        assert_eq!(resp.agents[0].abilities[0]["local_name"], "chat");
        assert_eq!(
            resp.agents[0].abilities[0]["ability_ura"],
            "easynet:///r/realm/ability/user.alice.chat"
        );
    }

    #[test]
    fn handle_resolve_includes_device_sponsored_system_agent_when_host_device_is_online() {
        let registry = PresenceRegistry::new();
        let host_device_ura = "easynet:///r/realm/device/dev-1";
        let owner_ura = crate::core::ura::device_agent_ura(
            "realm",
            "dev-1",
            crate::daemon::ability::names::governance::RUNTIME_HEALTH_SYSTEM_AGENT_ID,
        );
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "observe.health")
            .expect("runtime-health SystemAgent ability ura");
        insert_presence(&registry, host_device_ura.to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(projection_row_for(
            &owner_ura,
            vec![projection_summary(
                &owner_ura,
                &ability_ura,
                "observe",
                "health",
            )],
        ));

        let resp = handle_resolve_at(
            &ResolveRequest::with_filter(Some(owner_ura.clone()), true),
            &registry,
            None,
            &catalog,
            None,
            1_714_493_100_000,
        )
        .expect("resolve");

        assert_eq!(resp.agents.len(), 1);
        assert_eq!(resp.agents[0].ura, owner_ura);
        assert_eq!(resp.agents[0].host_node_id.as_deref(), Some("dev-1"));
        assert_eq!(resp.agents[0].abilities.len(), 1);
        assert_eq!(resp.agents[0].abilities[0]["local_name"], "health");
        assert_eq!(resp.agents[0].abilities[0]["ability_ura"], ability_ura);
        assert!(
            !registry.contains(&resp.agents[0].ura),
            "SystemAgent visibility must be driven by host Device presence, not an independent SystemAgent presence row"
        );
    }

    #[test]
    fn handle_resolve_does_not_surface_service_projection_as_device_hosted_agent() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm/device/dev-1".to_string());
        let service_ura = crate::core::ura::service_ura("realm", "user-dev", "pages");
        let ability_ura = crate::core::ura::owner_ability_ura(&service_ura, "project_list")
            .expect("legacy Service ability URA");
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(projection_row_for(
            &service_ura,
            vec![projection_summary(
                &service_ura,
                &ability_ura,
                "project",
                "list",
            )],
        ));

        let resp = handle_resolve_at(
            &ResolveRequest::with_filter(Some(service_ura), true),
            &registry,
            None,
            &catalog,
            None,
            1_714_493_100_000,
        )
        .expect("resolve");

        assert!(
            resp.agents.is_empty(),
            "Service owner projections are not Agent/SystemAgent presence rows"
        );
    }

    #[test]
    fn handle_resolve_does_not_surface_expired_owner_projection() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm/device/dev-1".to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(
            crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                "easynet:///r/realm/device/dev-1".to_string(),
                "easynet:///r/realm/device/dev-1".to_string(),
                1,
                7,
                "sha256:projection".to_string(),
                1_000,
                vec![projection_summary(
                    "easynet:///r/realm/device/dev-1",
                    "easynet:///r/realm/ability/device.dev-1.fs.read",
                    "fs",
                    "read",
                )],
            ),
        );

        let live = handle_resolve_at(
            &ResolveRequest::with_filter(Some("easynet:///r/realm/device/".to_string()), true),
            &registry,
            None,
            &catalog,
            None,
            999,
        )
        .expect("resolve");
        assert_eq!(live.agents.len(), 1);
        assert_eq!(live.agents[0].abilities.len(), 1);

        let expired = handle_resolve_at(
            &ResolveRequest::with_filter(Some("easynet:///r/realm/device/".to_string()), true),
            &registry,
            None,
            &catalog,
            None,
            1_000,
        )
        .expect("resolve");
        assert_eq!(expired.agents.len(), 1);
        assert!(expired.agents[0].abilities.is_empty());
    }

    #[test]
    fn namespace_resolve_returns_typed_final_route_for_system_agent_ability() {
        let registry = PresenceRegistry::new();
        let host_device_ura = "easynet:///r/realm/device/dev-1";
        let owner_ura = crate::core::ura::device_agent_ura(
            "realm",
            "dev-1",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "meta.list")
            .expect("SystemAgent ability ura");
        insert_presence(&registry, host_device_ura.to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(projection_row_for(
            &owner_ura,
            vec![projection_summary(&owner_ura, &ability_ura, "meta", "list")],
        ));

        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "query_name": owner_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
                "ability_name": "meta.list",
            }),
            &registry,
            None,
            &catalog,
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::FinalRoute.as_str_name(),
            "{answer:#}"
        );
        assert_eq!(answer["owner_ura"], owner_ura);
        assert_eq!(answer["ability_ura"], ability_ura);
        assert_eq!(
            answer["release_profile"],
            ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        assert_eq!(
            answer["next_hop"]["hosted_agent_via_device"]["dispatch_name"],
            "meta.list"
        );
        assert_eq!(
            answer["selected_route"]["gates"]["ability"],
            GateResult::Pass.as_str_name()
        );
        assert!(answer.get("negative").is_none());
    }

    #[test]
    fn namespace_resolve_returns_typed_negative_when_ability_absent() {
        let registry = PresenceRegistry::new();
        let host_device_ura = "easynet:///r/realm/device/dev-1";
        let owner_ura = crate::core::ura::device_agent_ura(
            "realm",
            "dev-1",
            crate::daemon::ability::names::agents::AGENT_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        insert_presence(&registry, host_device_ura.to_string());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let published_ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "agent.start")
            .expect("system-agent ability URA");
        catalog.upsert_projection(projection_row_for(
            &owner_ura,
            vec![projection_summary(
                &owner_ura,
                &published_ability_ura,
                "agent",
                "start",
            )],
        ));

        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "query_name": owner_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
                "ability_name": "agent.list",
            }),
            &registry,
            None,
            &catalog,
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            NegativeReason::Nodata.as_str_name()
        );
        assert_eq!(answer["next_hop"]["no_route"], serde_json::json!({}));
    }

    #[test]
    fn namespace_resolve_rejects_missing_qtype_without_guessing_route_shape() {
        let registry = PresenceRegistry::new();
        let host_device_ura = crate::core::ura::device_ura("realm", "dev-1");
        let owner_ura = crate::core::ura::device_agent_ura("realm", "dev-1", "test-runtime");
        insert_presence(&registry, host_device_ura.clone());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "agent.list")
            .expect("device ability ura");
        catalog.upsert_projection(projection_row_for(
            &owner_ura,
            vec![projection_summary(
                &owner_ura,
                &ability_ura,
                "agent",
                "list",
            )],
        ));

        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "query_name": owner_ura,
                "ability_name": "agent.list",
            }),
            &registry,
            None,
            &catalog,
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            NegativeReason::Refused.as_str_name()
        );
        assert!(answer["negative"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("missing canonical qtype")));
        assert!(answer.get("ability_ura").is_none());
    }

    #[test]
    fn namespace_resolve_input_failure_does_not_fabricate_localhost_authority() {
        let registry = PresenceRegistry::new();
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "query_name": "not-a-ura",
                "ability_name": "agent.list",
            }),
            &registry,
            None,
            &catalog,
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            NegativeReason::Refused.as_str_name()
        );
        assert_eq!(answer["authority"]["authority_ura"], "");
        assert_eq!(answer["authority"]["zone_ref"], "query_name_unavailable");
        assert_ne!(
            answer["authority"]["authority_ura"],
            crate::core::ura::hub_ura("localhost")
        );
    }

    #[test]
    fn namespace_resolve_rejects_short_qtype_aliases_at_public_ingress() {
        let registry = PresenceRegistry::new();
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "query_name": "easynet:///r/realm/device/dev-1",
                "qtype": "ROUTE",
                "ability_name": "agent.list",
            }),
            &registry,
            None,
            &catalog,
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert!(answer["negative"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("not a canonical ResolveType enum string")));
    }

    #[test]
    fn namespace_resolve_directory_includes_hosted_by_for_hosted_agents() {
        let registry = PresenceRegistry::new();
        let host_ura = "easynet:///r/realm/device/dev-1";
        let agent_ura = "easynet:///r/realm/agent/alice.remote";
        insert_presence(&registry, host_ura.to_string());
        let advertised = AdvertisedAgentStore::new();
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: agent_ura.to_string(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
        });
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "query_name": "easynet:///r/realm/agent/alice.",
                "qtype": "RESOLVE_TYPE_DIRECTORY_LISTING",
            }),
            &registry,
            Some(&advertised),
            &catalog,
            1_714_493_100_000,
        );

        let records = answer["records"]
            .as_array()
            .expect("typed answer must carry records");
        assert!(
            records.iter().any(|record| {
                record["record_type"] == RecordType::HostedBy.as_str_name()
                    && record["value"]["hosted_by"]["hosted_ura"] == agent_ura
                    && record["value"]["hosted_by"]["host_ura"] == host_ura
                    && record["value"]["hosted_by"]["host_node_id"] == "dev-1"
            }),
            "hosted agent namespace directory must include hosted_by placement; got {records:#?}"
        );
    }

    #[test]
    fn handle_resolve_key_returns_pubkey_when_present_in_anchor() {
        use crate::daemon::trust::anchor::{
            RealmTrustAnchor, TrustAnchorRole, TrustedAgent, TrustedPrincipalOwner,
        };
        let entry = TrustedAgent {
            agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            vec![entry],
            vec![TrustedPrincipalOwner {
                principal_ura: "easynet:///r/realm-a/device/n1".to_string(),
                owner_user_id: "alice".to_string(),
                owner_ura: "easynet:///r/realm-a/user/alice".to_string(),
                added_at_unix_ms: 1_700_000_000_001,
            }],
            Vec::new(),
        )
        .expect("anchor");
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
                presented_pubkey_b64: None,
            },
            &anchor,
        )
        .expect("resolve_key must not fail")
        .expect("hit");
        assert_eq!(
            resp.public_key_b64,
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
        assert_eq!(
            resp.public_key_hex,
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            resp.principal_owner_ura.as_deref(),
            Some("easynet:///r/realm-a/user/alice")
        );
        assert_eq!(resp.principal_owner_user_id.as_deref(), Some("alice"));
    }

    #[test]
    fn handle_resolve_key_returns_none_when_ura_not_in_anchor() {
        use crate::daemon::trust::anchor::RealmTrustAnchor;
        let anchor = RealmTrustAnchor::default();
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: "easynet:///r/realm-a/device/missing".to_string(),
                presented_pubkey_b64: None,
            },
            &anchor,
        );
        assert!(
            resp.expect("resolve_key must not fail").is_none(),
            "miss must surface as None for caller status mapping"
        );
    }

    #[test]
    fn resolve_key_response_rejects_invalid_base64_key_material() {
        let err = resolve_key_response("not-base64", Vec::new(), None)
            .expect_err("invalid public_key_b64 must not produce an empty public_key_hex");

        assert!(
            matches!(err, ResolveKeyResponseError::InvalidPublicKeyBase64(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_key_response_rejects_non_ed25519_key_length() {
        let err = resolve_key_response("AA==", Vec::new(), None)
            .expect_err("non-32-byte public_key_b64 must fail closed");

        assert_eq!(err, ResolveKeyResponseError::InvalidPublicKeyLength(1));
    }

    #[test]
    fn handle_resolve_key_user_role_pins_the_presented_pubkey() {
        use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole, TrustedAgent};

        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let alice = "easynet:///r/realm/user/alice";
        let entries = [pk_a, pk_b].into_iter().map(|pk| TrustedAgent {
            agent_ura: alice.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_714_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        });
        let anchor = RealmTrustAnchor::from_entries(entries.collect()).expect("anchor");

        // Presented = pk_a → resolves to pk_a.
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: alice.to_string(),
                presented_pubkey_b64: Some(pk_a.to_string()),
            },
            &anchor,
        )
        .expect("resolve_key must not fail")
        .expect("pk_a resolves");
        assert_eq!(resp.public_key_b64, pk_a);

        // Presented = pk_b → resolves to pk_b. Multi-device proof.
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: alice.to_string(),
                presented_pubkey_b64: Some(pk_b.to_string()),
            },
            &anchor,
        )
        .expect("resolve_key must not fail")
        .expect("pk_b resolves");
        assert_eq!(resp.public_key_b64, pk_b);

        // Presented = unknown → miss.
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: alice.to_string(),
                presented_pubkey_b64: Some(
                    "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC=".to_string(),
                ),
            },
            &anchor,
        );
        assert!(
            resp.expect("resolve_key must not fail").is_none(),
            "unknown pubkey under known user must miss"
        );
    }

    #[test]
    fn handle_resolve_key_single_key_roles_pin_presented_pubkey() {
        use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole, TrustedAgent};

        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let other = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let device = "easynet:///r/realm/device/node-a";
        let anchor = RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: device.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_714_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("anchor");

        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: device.to_string(),
                presented_pubkey_b64: Some(pk.to_string()),
            },
            &anchor,
        )
        .expect("resolve_key must not fail")
        .expect("matching presented device key resolves");
        assert_eq!(resp.public_key_b64, pk);

        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: device.to_string(),
                presented_pubkey_b64: Some(other.to_string()),
            },
            &anchor,
        );
        assert!(
            resp.expect("resolve_key must not fail").is_none(),
            "mismatched presented key must not resolve a stale same-URA device key"
        );
    }

    // ── N3-N4 bridge: handle_discover_with_user_filter ─────────

    fn populated_view_two_realms(
    ) -> crate::daemon::federation::directory::SharedFederatedDirectoryView {
        use crate::daemon::federation::directory::{
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        let cell = SharedFederatedDirectoryView::default();
        let mut realm_a = DirectoryView::new("realm-a".to_string());
        realm_a.replace_entries(vec![DirectoryEntry {
            agent_ura: "easynet:///r/realm-a/user/user-c".to_string(),
            node_id: "user-c".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        }]);
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.replace_entries(vec![DirectoryEntry {
            agent_ura: "easynet:///r/realm-c/user/unbound".to_string(),
            node_id: "unbound".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        }]);
        let mut peers = std::collections::BTreeMap::new();
        peers.insert("realm-a".to_string(), std::sync::Arc::new(realm_a));
        peers.insert("realm-c".to_string(), std::sync::Arc::new(realm_c));
        cell.replace(peers);
        cell
    }

    #[test]
    fn discover_with_user_filter_keeps_bound_and_drops_unbound() {
        use crate::daemon::keyring::federated_bindings::{
            FederatedBindingsStore, FederatedUserBinding,
        };
        use crate::daemon::keyring::resolver::FederatedUserResolver;
        use std::sync::Arc;

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        // Bind realm-a's user-c to local user-on-b. realm-c is
        // NOT bound — its entry must be filtered out.
        bindings
            .record_binding(
                FederatedUserBinding {
                    source_realm: "realm-a".to_string(),
                    source_user_ura: "easynet:///r/realm-a/user/user-c".to_string(),
                    source_user_pubkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                        .to_string(),
                    local_user_id: "user-on-b".to_string(),
                    bound_at_unix_ms: 1_714_500_000_000,
                },
                "n".to_string(),
            )
            .unwrap();
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let view = populated_view_two_realms();

        let resp = handle_discover_with_user_filter(
            &DiscoverRequest {
                agent_ura: None,
                ..Default::default()
            },
            &view,
            &resolver,
        );
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(
            resp.entries[0].agent_ura,
            "easynet:///r/realm-a/user/user-c"
        );
    }

    #[test]
    fn discover_with_user_filter_keeps_local_realm_unconditionally() {
        // Calling daemon's own realm = realm-a; the realm-a
        // entry surfaces as `Local` from the resolver and passes
        // the filter without needing a binding.
        use crate::daemon::keyring::federated_bindings::FederatedBindingsStore;
        use crate::daemon::keyring::resolver::FederatedUserResolver;
        use std::sync::Arc;

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-a", bindings);
        let view = populated_view_two_realms();

        let resp = handle_discover_with_user_filter(
            &DiscoverRequest {
                agent_ura: None,
                ..Default::default()
            },
            &view,
            &resolver,
        );
        // realm-a entry passes (Local), realm-c does not (NotBound).
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(
            resp.entries[0].agent_ura,
            "easynet:///r/realm-a/user/user-c"
        );
    }

    #[test]
    fn discover_with_user_filter_drops_all_unbound_when_no_local_realm_match() {
        // Resolver thinks we're realm-b; no binding exists.
        // Both directory entries are cross-realm and unbound;
        // result is empty.
        use crate::daemon::keyring::federated_bindings::FederatedBindingsStore;
        use crate::daemon::keyring::resolver::FederatedUserResolver;
        use std::sync::Arc;

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let view = populated_view_two_realms();
        let resp = handle_discover_with_user_filter(
            &DiscoverRequest {
                agent_ura: None,
                ..Default::default()
            },
            &view,
            &resolver,
        );
        assert!(
            resp.entries.is_empty(),
            "no bindings + no local-realm match ⇒ empty filtered result"
        );
    }

    #[test]
    fn discover_with_user_filter_ura_query_drops_when_unbound() {
        use crate::daemon::keyring::federated_bindings::FederatedBindingsStore;
        use crate::daemon::keyring::resolver::FederatedUserResolver;
        use std::sync::Arc;

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let view = populated_view_two_realms();
        // Direct URA query for realm-c's entry — exists in the
        // view but is unbound for the calling user. Filter out.
        let resp = handle_discover_with_user_filter(
            &DiscoverRequest {
                agent_ura: Some("easynet:///r/realm-c/user/unbound".to_string()),
                ..Default::default()
            },
            &view,
            &resolver,
        );
        assert!(resp.entries.is_empty());
    }

    #[test]
    fn handle_list_user_devices_returns_only_matching_realm() {
        // PR-N3 N3-5. Registry holds entries for two realms;
        // the handler must surface only the requested realm's
        // entries.
        let registry = PresenceRegistry::new();
        insert_presence(
            &registry,
            "easynet:///r/realm-a/device/device-1".to_string(),
        );
        insert_presence(
            &registry,
            "easynet:///r/realm-a/device/device-2".to_string(),
        );
        insert_presence(
            &registry,
            "easynet:///r/realm-b/device/device-3".to_string(),
        );

        let resp = handle_list_user_devices(
            &ListUserDevicesRequest {
                realm: "realm-a".to_string(),
            },
            &registry,
        )
        .expect("realm-a devices project");
        assert_eq!(resp.devices.len(), 2);
        let expected_prefix = crate::core::ura::realm_device_prefix("realm-a");
        for entry in &resp.devices {
            assert!(entry.agent_ura.starts_with(&expected_prefix));
            assert_eq!(entry.origin_realm, None, "speaks for own realm — None");
            assert_eq!(entry.status, "active");
        }
    }

    #[test]
    fn handle_list_user_devices_extracts_node_id_from_ura() {
        let registry = PresenceRegistry::new();
        insert_presence(
            &registry,
            "easynet:///r/realm-a/device/node-xyz".to_string(),
        );

        let resp = handle_list_user_devices(
            &ListUserDevicesRequest {
                realm: "realm-a".to_string(),
            },
            &registry,
        )
        .expect("device URA projects");
        assert_eq!(resp.devices.len(), 1);
        assert_eq!(resp.devices[0].node_id, "node-xyz");
    }

    #[test]
    fn handle_list_user_devices_filters_canonical_non_device_principals() {
        let registry = PresenceRegistry::new();
        insert_presence(
            &registry,
            "easynet:///r/realm-a/agent/alice.helper".to_string(),
        );
        insert_presence(
            &registry,
            "easynet:///r/realm-a/agent/alice.claude".to_string(),
        );

        let resp = handle_list_user_devices(
            &ListUserDevicesRequest {
                realm: "realm-a".to_string(),
            },
            &registry,
        )
        .expect("canonical non-device principals are filtered");
        assert!(
            resp.devices.is_empty(),
            "list_user_devices must only project Device principal presence"
        );
    }

    #[test]
    fn handle_list_user_devices_returns_empty_when_no_match() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm-a/device/device".to_string());

        let resp = handle_list_user_devices(
            &ListUserDevicesRequest {
                realm: "realm-missing".to_string(),
            },
            &registry,
        )
        .expect("unmatched realm is empty");
        assert!(resp.devices.is_empty());
    }

    #[test]
    fn presence_registry_rejects_prefix_matched_malformed_device_presence() {
        let registry = PresenceRegistry::new();
        let error = registry
            .insert_negotiated(
                "easynet:///r/realm-a/device/".to_string(),
                make_dispatch_sender(),
                crate::daemon::invocation::bidi::state::presence::SessionContract::new(
                    crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
                    vec![0; 16],
                ),
            )
            .expect_err("malformed device presence must fail closed before wrapper projection");

        assert!(
            error.contains("canonical URA"),
            "unexpected presence registry error: {error}"
        );
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn validate_list_user_devices_response_rejects_node_id_mismatch() {
        let response = ListUserDevicesResponse {
            devices: vec![crate::daemon::federation::directory::DirectoryEntry {
                agent_ura: "easynet:///r/realm-a/device/dev-1".to_string(),
                node_id: "other".to_string(),
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            }],
        };

        let error = validate_list_user_devices_response(&response, "test peer")
            .expect_err("peer device row must bind node_id to Device URA");

        assert!(
            error.contains("node_id") && error.contains("does not match"),
            "unexpected peer response validation error: {error}"
        );
    }

    #[test]
    fn list_user_devices_requests_reject_retired_product_directory_fields() {
        let list = serde_json::from_value::<ListUserDevicesRequest>(serde_json::json!({
            "tenant_id": "tenant-a",
        }));
        assert!(list.is_err(), "peer request must require `realm`");
        let list = serde_json::from_value::<ListUserDevicesRequest>(serde_json::json!({
            "realm": "tenant-a",
            "user_ura": "easynet:///r/tenant-a/user/alice",
        }));
        assert!(
            list.is_err(),
            "peer request must reject retired user_ura filter"
        );

        let proxy = serde_json::from_value::<ProxyListUserDevicesRequest>(serde_json::json!({
            "tenant_id": "tenant-a",
            "peer_hub_urls": ["https://peer.example:50443"],
        }));
        assert!(proxy.is_err(), "proxy request must require `realm`");
        let proxy = serde_json::from_value::<ProxyListUserDevicesRequest>(serde_json::json!({
            "realm": "tenant-a",
            "peers": ["https://peer.example:50443"],
        }));
        assert!(
            proxy.is_err(),
            "proxy request must reject retired peers alias"
        );
    }

    #[test]
    fn handle_revoke_reports_was_active_correctly() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let ura = "easynet:///r/realm/device/n1".to_string();
        insert_presence(&registry, ura.clone());

        let resp = handle_revoke(
            &RevokeRequest {
                agent_ura: ura.clone(),
                purge_transaction_id: None,
                generation: None,
                reason: None,
                authority_ura: None,
                protocol_version: None,
                delivery_fence: None,
            },
            &registry,
            None,
            &catalog,
        )
        .unwrap();
        assert!(resp.ack);
        assert!(resp.was_active);
        assert!(!registry.contains(&ura), "must be removed");
    }

    #[test]
    fn self_revoke_defers_presence_removal_until_response_carrier_closes() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let ura = "easynet:///r/realm/device/n1".to_string();
        insert_presence(&registry, ura.clone());
        catalog.upsert_projection(projection_row_for(&ura, Vec::new()));

        let admitted = RevokeRequest {
            agent_ura: ura.clone(),
            purge_transaction_id: None,
            generation: None,
            reason: None,
            authority_ura: None,
            protocol_version: None,
            delivery_fence: None,
        }
        .bind_to_subject(&ura)
        .expect("self-revoke target is bound to its admitted subject");
        let resp = handle_revoke_with_presence_mode(
            &admitted,
            &registry,
            None,
            &catalog,
            RevokePresenceMode::defer_current_caller(&ura),
        )
        .unwrap();

        assert!(resp.ack);
        assert!(resp.was_active);
        assert!(
            registry.contains(&ura),
            "self-revoke must not destroy the session carrying its own response"
        );
        assert!(
            catalog.get(&ura).is_none(),
            "self-revoke still removes the advertised ability owner projection"
        );
    }

    #[test]
    fn revoke_request_rejects_retired_target_ura_alias() {
        let error = serde_json::from_value::<RevokeRequest>(serde_json::json!({
            "target_ura": "easynet:///r/realm/device/n1"
        }))
        .expect_err("retired target_ura alias must fail closed");
        assert!(
            error.to_string().contains("unknown field `target_ura`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn revoke_request_serializes_only_the_canonical_agent_target() {
        let request = RevokeRequest {
            agent_ura: "easynet:///r/realm/agent/user.worker".to_string(),
            purge_transaction_id: Some("11111111111111111111111111111111".to_string()),
            generation: Some(2),
            reason: Some("agent.purge".to_string()),
            authority_ura: Some("easynet:///r/realm/device/dev-1".to_string()),
            protocol_version: Some(
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            ),
            delivery_fence: Some(3),
        };

        let wire = serde_json::to_vec(&request).expect("serialize canonical revoke request");
        let value: serde_json::Value = serde_json::from_slice(&wire).expect("revoke json");
        assert_eq!(
            value.get("agent_ura").and_then(serde_json::Value::as_str),
            Some("easynet:///r/realm/agent/user.worker")
        );
        assert!(value.get("target_ura").is_none());
        serde_json::from_slice::<RevokeRequest>(&wire)
            .expect("sender-produced revoke request must satisfy the receiver contract");
    }

    #[test]
    fn revoke_intent_binds_mutation_target_to_admitted_subject() {
        let request = RevokeRequest {
            agent_ura: "easynet:///r/realm/agent/user.worker".to_string(),
            purge_transaction_id: None,
            generation: None,
            reason: None,
            authority_ura: None,
            protocol_version: None,
            delivery_fence: None,
        };

        request
            .clone()
            .bind_to_subject("easynet:///r/realm/agent/user.worker")
            .expect("exact admitted subject binds the revoke target");
        let error = request
            .bind_to_subject("easynet:///r/realm/agent/user.other")
            .expect_err("policy-on-A / mutation-on-B must fail closed");
        assert!(error.to_string().contains("envelope subject must equal"));
    }

    #[test]
    fn purge_revoke_requires_complete_command_facts() {
        let request = RevokeRequest {
            agent_ura: "easynet:///r/realm/agent/user.alice".to_string(),
            purge_transaction_id: Some("11111111111111111111111111111111".to_string()),
            generation: Some(1),
            reason: None,
            authority_ura: Some("easynet:///r/realm/device/dev-1".to_string()),
            protocol_version: Some(
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            ),
            delivery_fence: Some(1),
        };

        let error = request
            .resolve_intent()
            .expect_err("purge revoke must reject incomplete durable command facts");
        assert!(
            error
                .to_string()
                .contains("federation.revoke purge requires reason"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn handle_revoke_requires_canonical_agent_ura() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let missing = handle_revoke(
            &RevokeRequest {
                agent_ura: String::new(),
                purge_transaction_id: None,
                generation: None,
                reason: None,
                authority_ura: None,
                protocol_version: None,
                delivery_fence: None,
            },
            &registry,
            None,
            &catalog,
        )
        .expect_err("missing agent_ura must fail closed");
        assert!(missing.to_string().contains("agent_ura is required"));

        let invalid = handle_revoke(
            &RevokeRequest {
                agent_ura: "not-a-ura".to_string(),
                purge_transaction_id: None,
                generation: None,
                reason: None,
                authority_ura: None,
                protocol_version: None,
                delivery_fence: None,
            },
            &registry,
            None,
            &catalog,
        )
        .expect_err("invalid agent_ura must fail closed");
        assert!(invalid.to_string().contains("agent_ura is invalid"));
    }

    #[test]
    fn handle_revoke_removes_hosted_agent_rows_too() {
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let agent_ura = "easynet:///r/realm/agent/user.alice";
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: agent_ura.into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        });
        catalog.upsert_projection(projection_row_for(agent_ura, Vec::new()));
        let resp = handle_revoke(
            &RevokeRequest {
                agent_ura: agent_ura.to_string(),
                purge_transaction_id: None,
                generation: None,
                reason: None,
                authority_ura: None,
                protocol_version: None,
                delivery_fence: None,
            },
            &registry,
            Some(&advertised),
            &catalog,
        )
        .unwrap();
        assert!(resp.ack);
        assert!(!resp.was_active);
        assert!(
            advertised.get(agent_ura).is_none(),
            "revoke must remove advertised hosted-agent rows"
        );
        assert!(
            catalog.get(agent_ura).is_none(),
            "revoke must remove owner projection rows"
        );
    }

    #[test]
    fn immediate_revoke_cannot_bypass_durable_hosted_agent_retirement() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let agent_ura = "easynet:///r/realm/agent/user.alice";
        let host_ura = "easynet:///r/realm/device/dev-1";
        crate::daemon::persistence::federation_revoke::register_agent(
            crate::daemon::persistence::federation_revoke::HostedAgentRegistrationCommand {
                agent_ura: agent_ura.to_string(),
                incarnation_id: crate::daemon::federation::hosted_agent_publication::HostedAgentIncarnationId::parse(
                    "1".repeat(32),
                )
                .unwrap(),
                public_key_hex: String::new(),
                host_node_id: Some("dev-1".to_string()),
                signing_authority: crate::daemon::persistence::federation_revoke::DurableSigningAuthority::HostedBy {
                    host_ura: host_ura.to_string(),
                },
            },
        )
        .unwrap();
        let error = handle_revoke(
            &RevokeRequest {
                agent_ura: agent_ura.to_string(),
                purge_transaction_id: None,
                generation: Some(1),
                reason: Some("agent.stop".to_string()),
                authority_ura: Some(host_ura.to_string()),
                protocol_version: Some(
                    crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
                ),
                delivery_fence: Some(1),
            },
            &PresenceRegistry::new(),
            None,
            &AbilityCatalogStore::new(),
        )
        .expect_err("hosted Agent revoke must use the durable transaction FSM");
        assert!(error.to_string().contains("requires a durable transaction"));
        assert!(
            crate::daemon::persistence::federation_revoke::active_inventory_record(agent_ura)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn purge_revoke_replay_returns_durable_result_and_reapplies_removal() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = "easynet:///r/realm/device/dev-1";
        let agent_ura = "easynet:///r/realm/agent/user.crash-window";
        insert_presence(&registry, host_ura.to_string());
        let record = AdvertisedAgentRecord {
            agent_ura: agent_ura.to_string(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
        };
        register_into_read_model(
            hosted_agent_registration(agent_ura, host_ura, "11111111111111111111111111111111"),
            &advertised,
        );
        catalog.upsert_projection(projection_row_for(agent_ura, Vec::new()));
        let request = RevokeRequest {
            agent_ura: agent_ura.to_string(),
            purge_transaction_id: Some("fedcba9876543210fedcba9876543210".to_string()),
            generation: Some(1),
            reason: Some("test purge".to_string()),
            authority_ura: Some(host_ura.to_string()),
            protocol_version: Some(
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            ),
            delivery_fence: Some(1),
        };

        let first = handle_revoke(&request, &registry, Some(&advertised), &catalog).unwrap();
        assert!(first.was_active);
        assert!(!first.replayed);
        assert!(advertised.get(agent_ura).is_none());
        assert!(catalog.get(agent_ura).is_none());

        registry.force_revoke(host_ura);
        advertised.upsert(record);
        catalog.upsert_projection(projection_row_for(agent_ura, Vec::new()));
        let replay = handle_revoke(&request, &registry, Some(&advertised), &catalog).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.was_active, first.was_active);
        assert_eq!(replay.purge_transaction_id, first.purge_transaction_id);
        assert!(
            advertised.get(agent_ura).is_none(),
            "replay after restart reapplies the committed removal"
        );
        assert!(
            catalog.get(agent_ura).is_none(),
            "replay after restart reapplies the committed owner projection removal"
        );
    }

    #[test]
    fn delayed_old_revoke_preserves_new_same_ura_incarnation_everywhere() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let agent_ura = "easynet:///r/realm/agent/user.aba";
        let host_ura = "easynet:///r/realm/device/dev-1";
        let advertise =
            |incarnation_hex| hosted_agent_registration(agent_ura, host_ura, incarnation_hex);
        register_into_read_model(advertise("11111111111111111111111111111111"), &advertised);
        let request = RevokeRequest {
            agent_ura: agent_ura.to_string(),
            purge_transaction_id: Some("11111111111111111111111111111111".into()),
            generation: Some(1),
            reason: Some("agent.purge".into()),
            authority_ura: Some(host_ura.into()),
            protocol_version: Some(
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            ),
            delivery_fence: Some(1),
        };
        let command = crate::daemon::persistence::federation_revoke::FederationRevokeCommand {
            protocol_version: request.protocol_version.unwrap(),
            transaction_id: request.purge_transaction_id.clone().unwrap(),
            agent_ura: agent_ura.into(),
            generation: 1,
            reason: request.reason.clone().unwrap(),
            authority_ura: host_ura.into(),
            target_ura: agent_ura.into(),
        };
        crate::daemon::persistence::federation_revoke::prepare_revoke(&command, 1, false, None, 1)
            .unwrap();
        let (durable_old_revoke, replayed) =
            crate::daemon::persistence::federation_revoke::apply_prepared_revoke(
                &command.transaction_id,
                1,
                2,
            )
            .unwrap();
        assert!(!replayed);
        assert_eq!(
            durable_old_revoke.disposition,
            crate::daemon::persistence::federation_revoke::FederationRevokeDisposition::Retired,
            "the old incarnation must be durably retired before a new one can register"
        );

        register_into_read_model(advertise("22222222222222222222222222222222"), &advertised);
        catalog.upsert_projection(
            crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                agent_ura.into(),
                host_ura.into(),
                2,
                1,
                "sha256:new-incarnation".into(),
                0,
                Vec::new(),
            ),
        );
        insert_presence(&registry, agent_ura.to_string());

        let response = handle_revoke(&request, &registry, Some(&advertised), &catalog)
            .expect("old applied revoke replays through generation-fenced read-model cleanup");
        assert!(response.replayed);
        assert_eq!(
            response.disposition,
            Some(
                crate::daemon::persistence::federation_revoke::FederationRevokeDisposition::Retired
            )
        );
        assert_eq!(advertised.get(agent_ura).unwrap().generation, 2);
        assert!(catalog.get(agent_ura).is_some());
        assert!(registry.contains(agent_ura));
    }

    #[test]
    fn build_subscribe_directory_v2_snapshot_is_sorted() {
        let registry = PresenceRegistry::new();
        insert_presence(&registry, "easynet:///r/realm/device/c".to_string());
        insert_presence(&registry, "easynet:///r/realm/device/a".to_string());

        let initial = build_subscribe_directory_v2_snapshot(&registry)
            .expect("canonical device snapshot builds");
        let crate::daemon::federation::directory::DirectoryEvent::Snapshot { agents, .. } = initial
        else {
            panic!("v2 initial frame must be a DirectoryEvent::Snapshot");
        };
        let uras: Vec<&str> = agents.iter().map(|a| a.agent_ura.as_str()).collect();
        assert_eq!(
            uras,
            vec!["easynet:///r/realm/device/a", "easynet:///r/realm/device/c"]
        );
    }

    #[test]
    fn build_subscribe_directory_v2_snapshot_rejects_non_device_presence_row() {
        let registry = PresenceRegistry::new();
        insert_presence(
            &registry,
            "easynet:///r/realm/agent/user.device-carryover".to_string(),
        );

        let err = build_subscribe_directory_v2_snapshot(&registry)
            .expect_err("agent URA must not publish as a directory device row");
        assert!(
            err.contains("not a canonical Device URA"),
            "unexpected error: {err}"
        );
    }
}

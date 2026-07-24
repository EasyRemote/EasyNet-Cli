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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::ability::conformance;
use crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore;
use crate::daemon::federation::read_model::advertised_agents::{
    AdvertisedAgentRecord, AdvertisedAgentSigningAuthority, AdvertisedAgentStore,
};
use crate::daemon::federation::receipt_contract::{
    AdvertiseContract, HubAbilitiesDiff, HubAbilityEntry,
};
#[cfg(test)]
use crate::daemon::federation::resolver_contract::{GateResult, RecordType};
use crate::daemon::federation::resolver_contract::{
    NegativeReason, ResolveAnswerKind, ResolveType, ResolverReleaseProfile,
};
pub use crate::daemon::federation::wire_contract::{
    DiscoverRequest, DiscoverResponse, ListUserDevicesRequest, ListUserDevicesResponse,
    ResolveAgentSummary, ResolveFilterRequest, ResolveKeyRequest, ResolveKeyResponse,
    ResolveRequest, ResolveResponse,
};
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;

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
/// returns an Axon `ResolveAnswer` proto-JSON projection, not legacy
/// retired directory row shapes.
pub const ABILITY_NAMESPACE_RESOLVE: &str = conformance::ABILITY_NAMESPACE_RESOLVE;

/// `namespace.proxy_resolve` — daemon-local typed namespace proxy.
/// The backend supplies the peer hub set, but the daemon owns trust
/// filtering, peer dialling, envelope signing, and typed
/// `ResolveAnswer` aggregation. This is the clean replacement for
/// backend product paths that previously consumed
/// legacy federation directory rows.
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
    /// axon-runtime's prior nonce-bearing receipt, MAY-differ under
    /// schema-compat.
    pub join_receipt_hash: String,
    /// Explicit hub-owned ability catalog snapshot published at join time.
    pub hub_published_abilities: Vec<HubAbilityEntry>,
    /// Monotonic revision for `hub_published_abilities`.
    pub hub_abilities_revision: u64,
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
        hub_published_abilities: Vec::new(),
        hub_abilities_revision: 0,
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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvertiseAgentRequest {
    /// URA of the agent being advertised.
    pub agent_ura: String,
    /// Durable owner-cursor generation for this Agent incarnation.
    pub generation: u64,
    /// Canonical authority carrying hosted-agent signing custody.
    pub signing_authority: AdvertiseSigningAuthorityRequest,
    #[serde(default)]
    pub public_key_hex: String,
    /// Runtime node hosting the agent's canonical invocation endpoint.
    #[serde(default)]
    pub host_node_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdvertiseSigningAuthorityRequest {
    SelfSigned,
    HostedBy { host_ura: String },
}

impl AdvertiseAgentRequest {
    #[must_use]
    pub(crate) fn signing_host_ura(&self) -> Option<&str> {
        match &self.signing_authority {
            AdvertiseSigningAuthorityRequest::HostedBy { host_ura } => Some(host_ura),
            AdvertiseSigningAuthorityRequest::SelfSigned => None,
        }
    }

    #[must_use]
    fn to_record(&self) -> AdvertisedAgentRecord {
        let signing_authority = match &self.signing_authority {
            AdvertiseSigningAuthorityRequest::SelfSigned => {
                AdvertisedAgentSigningAuthority::SelfSigned
            }
            AdvertiseSigningAuthorityRequest::HostedBy { host_ura } => {
                AdvertisedAgentSigningAuthority::HostedBy {
                    host_ura: host_ura.clone(),
                }
            }
        };
        AdvertisedAgentRecord {
            agent_ura: self.agent_ura.clone(),
            generation: self.generation,
            public_key_hex: self.public_key_hex.clone(),
            host_node_id: self.host_node_id.clone(),
            signing_authority,
        }
    }

    fn to_durable_record(
        &self,
    ) -> crate::daemon::persistence::federation_revoke::HostedAgentInventoryRecord {
        use crate::daemon::persistence::federation_revoke::{
            DurableSigningAuthority, HostedAgentInventoryRecord, InventoryLifecycle,
        };
        let signing_authority = match self.signing_host_ura() {
            Some(host_ura) => DurableSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
            None => DurableSigningAuthority::SelfSigned,
        };
        HostedAgentInventoryRecord {
            agent_ura: self.agent_ura.clone(),
            generation: self.generation,
            public_key_hex: self.public_key_hex.clone(),
            host_node_id: self.host_node_id.clone(),
            signing_authority,
            lifecycle: InventoryLifecycle::Active,
        }
    }
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

/// Handle a `federation.advertise_agent` invocation. Presence still
/// owns liveness; the store just captures the host-device linkage so
/// resolve can surface hosted agents.
pub fn handle_advertise_agent(
    request: &AdvertiseAgentRequest,
    store: Option<&AdvertisedAgentStore>,
) -> anyhow::Result<AdvertiseAgentResponse> {
    crate::daemon::persistence::federation_revoke::register_agent(request.to_durable_record())?;
    if let Some(store) = store {
        store.upsert(request.to_record());
    }
    Ok(AdvertiseAgentResponse {
        ack: true,
        replaced_prior: false,
    })
}

// ─── federation.advertise_abilities ────────────────────────────────

/// Request payload for `federation.advertise_abilities`.
///
/// The current wire shape is RFC-005 owner projection publication:
/// the caller sends projection metadata plus bounded ability summaries.
pub(crate) type AdvertiseAbilitiesRequest =
    crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication;

/// Response payload for `federation.advertise_abilities`. Matches the
/// daemon-backed wrapper contract (`ack` + `count`), where `ack` is true
/// only when the owner projection read model accepted the publication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvertiseAbilitiesResponse {
    pub ack: bool,
    pub count: usize,
}

/// Handle a `federation.advertise_abilities` invocation by updating the
/// hub-side owner projection read model.
#[must_use]
pub(crate) fn handle_advertise_abilities(
    request: &AdvertiseAbilitiesRequest,
    catalog: &crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
) -> AdvertiseAbilitiesResponse {
    let count = request.ability_summaries.len();
    let stored = catalog
        .upsert_projection(
            crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                request.owner_ura.clone(),
                request.host_device_ura.clone(),
                request.generation,
                request.projection_revision,
                request.projection_digest.clone(),
                request.lease_expires_unix_ms,
                request.ability_summaries.clone(),
            ),
        )
        .is_stored();
    AdvertiseAbilitiesResponse {
        ack: stored,
        count: if stored { count } else { 0 },
    }
}

// ─── federation.heartbeat ──────────────────────────────────────────

/// Request payload for `federation.heartbeat`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRequest {
    /// Device's last observed hub-published ability revision. Until the hub
    /// has a provider-backed diff source, the response explicitly echoes this
    /// revision with an empty diff instead of silently ignoring the field.
    pub since_abilities_revision: u64,
    /// Owner URAs whose ability projection leases this heartbeat renews.
    /// The device batches its own owners (device + hosted agents) here so
    /// the hub keeps their projections live without a full re-advertise.
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
    /// Number of `refresh_owner_uras` whose projection lease this
    /// heartbeat actually renewed (owners without a stored projection are
    /// skipped). Lets the device detect when it must re-advertise.
    #[serde(default)]
    pub refreshed_owner_count: usize,
    /// Explicit hub-owned ability catalog diff since the caller's last
    /// observed revision.
    pub hub_abilities_diff: HubAbilitiesDiff,
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
    // RFC-005: heartbeat renews the owner projection lease only; it must
    // not mutate projection contents, revision, or digest. Extend the
    // lease for every owner the device batched into `refresh_owner_uras`
    // so its device/hosted-agent abilities stay resolvable between full
    // re-advertise cycles. Unknown owners are skipped (the device must
    // `advertise_abilities` before its first projection exists).
    let mut refreshed_owner_count = 0_usize;
    let new_expiry =
        crate::daemon::federation::read_model::owner_projection::lease_expiry_from_now(now_unix_ms);
    for owner_ura in &request.refresh_owner_uras {
        let owner_ura = owner_ura.trim();
        if !owner_ura.is_empty() && catalog.refresh_lease(owner_ura, new_expiry) {
            refreshed_owner_count += 1;
        }
    }
    HeartbeatResponse {
        membership_status: "active".to_string(),
        realm_directory_size: registry.online_count(),
        refreshed_owner_count,
        hub_abilities_diff: HubAbilitiesDiff::empty_at(request.since_abilities_revision),
    }
}

// ─── federation.resolve ────────────────────────────────────────────

/// Handle a `federation.resolve` invocation.
///
/// `catalog` is the mandatory owner projection read model the daemon
/// constructs at boot. When `request.include_abilities` is true and
/// the store has a row for an in-presence owner URA, the response
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

    for ura in registry.snapshot() {
        if prefix.is_some_and(|p| !ura.starts_with(p)) {
            continue;
        }
        let abilities = if want_abilities {
            resolved_owner_projection_values(catalog, local_publication, &ura, now_unix_ms)?
        } else {
            Vec::new()
        };
        agents.insert(
            ura.clone(),
            ResolveAgentSummary {
                ura,
                status: "active".to_string(),
                host_node_id: None,
                abilities,
            },
        );
    }

    if let Some(store) = advertised_agents {
        for record in store.snapshot() {
            let is_online = match record.host_ura() {
                Some(host_ura) => registry.lookup(host_ura).is_some(),
                None => registry.lookup(&record.agent_ura).is_some(),
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

    Ok(ResolveResponse {
        agents: agents.into_values().collect(),
    })
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
    let object = query
        .as_object()
        .ok_or_else(|| "namespace.resolve request must be a JSON object".to_string())?;
    let qtype = object
        .get("qtype")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|qtype| !qtype.is_empty())
        .ok_or_else(|| "namespace.resolve request missing canonical qtype".to_string())?;
    let parsed = ResolveType::from_str_name(qtype).ok_or_else(|| {
        format!("namespace.resolve qtype {qtype:?} is not a canonical ResolveType enum string")
    })?;
    if parsed == ResolveType::Unspecified {
        return Err("namespace.resolve qtype must not be RESOLVE_TYPE_UNSPECIFIED".to_string());
    }
    Ok(())
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
        principal_owner_username: principal_owner.and_then(|owner| owner.owner_username.clone()),
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
#[must_use]
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
        .map(str::to_string)
        .or_else(|| {
            request
                .presented_pubkey_hex
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|hex| hex::decode(hex).ok())
                .map(|raw| BASE64_STANDARD.encode(raw))
        });
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default, rename = "ability_name")]
    pub ability_name: String,
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
/// URA compatibility:
/// - Canonical v4.1.4 device sessions live under
///   `easynet:///r/<realm>/device/<node>`.
/// - Only canonical device-session URAs (`.../device/<node>`) are
///   surfaced here.
/// - Real agent-profile URAs (`.../agent/<user>.<agent>`) are not
///   device sessions and are ignored here.
#[must_use]
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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeRequest {
    /// Canonical URA of the Device/Agent/User membership to revoke.
    pub agent_ura: String,
    #[serde(default)]
    pub purge_transaction_id: Option<String>,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub authority_ura: String,
    #[serde(default)]
    pub protocol_version: u32,
    #[serde(default)]
    pub delivery_fence: u64,
}

impl RevokeRequest {
    fn canonical_target_ura(&self) -> anyhow::Result<&str> {
        let target = self.agent_ura.trim();
        if target.is_empty() {
            anyhow::bail!("federation.revoke agent_ura is required");
        }
        crate::core::ura::parse_ura(target)
            .map_err(|error| anyhow::anyhow!("federation.revoke agent_ura is invalid: {error}"))?;
        Ok(target)
    }
}

/// Response payload for `federation.revoke`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Handle a `federation.revoke` invocation. Forces removal of the
/// target session and records whether the target was online at
/// revoke time so the caller can distinguish a real revoke from a
/// no-op.
pub fn handle_revoke(
    request: &RevokeRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    ability_catalog: &AbilityCatalogStore,
) -> anyhow::Result<RevokeResponse> {
    let target_ura = request.canonical_target_ura()?;
    let advertised_record = advertised_agents
        .and_then(|store| store.get(target_ura))
        .filter(|record| {
            request.purge_transaction_id.is_none() || record.generation == request.generation
        });
    let target_generation_is_current = request.purge_transaction_id.is_none()
        || advertised_record
            .as_ref()
            .is_some_and(|record| record.generation == request.generation);
    let was_active = target_generation_is_current
        && (registry.lookup(target_ura).is_some()
            || advertised_record
                .as_ref()
                .map(|record| match record.host_ura() {
                    Some(host_ura) => registry.lookup(host_ura).is_some(),
                    None => registry.lookup(&record.agent_ura).is_some(),
                })
                .unwrap_or(false));
    let Some(transaction_id) = request.purge_transaction_id.as_deref() else {
        let _displaced = registry.force_revoke(target_ura);
        if let Some(store) = advertised_agents {
            let _removed = store.remove(target_ura);
        }
        let _removed = ability_catalog.remove_owner(target_ura);
        return Ok(RevokeResponse {
            ack: true,
            was_active,
            purge_transaction_id: None,
            replayed: false,
            disposition: None,
        });
    };
    let command = crate::daemon::persistence::federation_revoke::FederationRevokeCommand {
        protocol_version: request.protocol_version,
        transaction_id: transaction_id.to_string(),
        agent_ura: request.agent_ura.clone(),
        generation: request.generation,
        reason: request.reason.clone(),
        authority_ura: request.authority_ura.clone(),
        target_ura: target_ura.to_string(),
    };
    let presence_session_id = target_generation_is_current
        .then(|| {
            registry
                .lookup_tracked(target_ura)
                .map(|(session_id, _)| session_id)
        })
        .flatten();
    let now = checked_revoke_now_unix_ms()?;
    let prepared = crate::daemon::persistence::federation_revoke::prepare_revoke(
        &command,
        request.delivery_fence,
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
                transaction_id,
                request.delivery_fence,
                now,
            )?
        }
    };
    if outcome.disposition
        != crate::daemon::persistence::federation_revoke::FederationRevokeDisposition::SupersededByNewIncarnation
    {
        if let Some(store) = advertised_agents {
            let _removed = store.remove_generation(target_ura, request.generation);
        }
        let _removed = ability_catalog.remove_generation(target_ura, request.generation);
        if let Some(session_id) = outcome.presence_session_id {
            let _removed = registry.remove_if_session(
                target_ura,
                session_id,
                crate::daemon::invocation::bidi::state::presence::OfflineReason::AdminRevoked,
            );
        }
    }
    Ok(RevokeResponse {
        ack: true,
        was_active: outcome.was_active,
        purge_transaction_id: Some(transaction_id.to_string()),
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
#[must_use]
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

    fn projection_summary(
        owner_ura: &str,
        ability_ura: &str,
        namespace: &str,
        local_name: &str,
    ) -> crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
        crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
            ability_ura: ability_ura.to_string(),
            owner_ura: owner_ura.to_string(),
            namespace: namespace.to_string(),
            local_name: local_name.to_string(),
            descriptor_revision: "sha256:descriptor".to_string(),
            schema_ref: None,
            schema_hash: None,
            policy_ref: "visibility:SCOPED".to_string(),
            route_summary_ref: Some(format!("route-ref::{ability_ura}")),
            tags: vec!["class:unary".to_string()],
            callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                if namespace.is_empty() {
                    local_name.to_string()
                } else {
                    format!("{namespace}.{local_name}")
                },
            ),
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
        assert!(resp.hub_published_abilities.is_empty());
        assert_eq!(resp.hub_abilities_revision, 0);
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
    fn handle_advertise_agent_returns_typed_ack() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let store = AdvertisedAgentStore::new();
        let req = AdvertiseAgentRequest {
            agent_ura: "easynet:///r/realm/agent/user.n1".to_string(),
            generation: 1,
            signing_authority: AdvertiseSigningAuthorityRequest::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".to_string(),
            },
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".to_string()),
        };
        let resp = handle_advertise_agent(&req, Some(&store)).expect("advertise agent succeeds");
        assert!(resp.ack);
        assert!(!resp.replaced_prior);
        let stored = store
            .get("easynet:///r/realm/agent/user.n1")
            .expect("advertised agent must be stored");
        assert_eq!(stored.host_ura(), Some("easynet:///r/realm/device/dev-1"));
    }

    #[test]
    fn advertise_agent_request_rejects_retired_top_level_host_ura() {
        let legacy = serde_json::json!({
            "agent_ura": "easynet:///r/realm/agent/user.n1",
            "generation": 1,
            "public_key_hex": "",
            "host_ura": "easynet:///r/realm/device/dev-1",
            "host_node_id": "dev-1"
        });

        let error = serde_json::from_value::<AdvertiseAgentRequest>(legacy)
            .expect_err("retired top-level host_ura must not be repaired");
        assert!(
            error.to_string().contains("host_ura"),
            "rejection must name retired host_ura field: {error}"
        );
    }

    #[test]
    fn advertise_agent_request_requires_signing_authority() {
        let missing_authority = serde_json::json!({
            "agent_ura": "easynet:///r/realm/agent/user.n1",
            "generation": 1,
            "public_key_hex": "",
            "host_node_id": "dev-1"
        });

        let error = serde_json::from_value::<AdvertiseAgentRequest>(missing_authority)
            .expect_err("advertise_agent must require signing_authority");
        assert!(
            error.to_string().contains("signing_authority"),
            "rejection must name required signing_authority: {error}"
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
                count: 1
            }
        );
        assert_eq!(
            handle_advertise_abilities(&stale, &catalog),
            AdvertiseAbilitiesResponse {
                ack: false,
                count: 0
            }
        );

        let got = catalog.get_at(owner_ura, 1_714_493_100_000).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "write");
    }

    #[test]
    fn handle_heartbeat_reports_registry_size() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/device/a".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/b".to_string(),
            make_dispatch_sender(),
        );
        let req = HeartbeatRequest {
            since_abilities_revision: 9,
            refresh_owner_uras: Vec::new(),
        };
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let resp = handle_heartbeat(&req, &registry, &catalog, 1_000);
        assert_eq!(resp.membership_status, "active");
        assert_eq!(resp.realm_directory_size, 2);
        assert_eq!(resp.refreshed_owner_count, 0);
        assert_eq!(resp.hub_abilities_diff.revision, 9);
        assert!(resp.hub_abilities_diff.added.is_empty());
        assert!(resp.hub_abilities_diff.removed.is_empty());
    }

    #[test]
    fn handle_heartbeat_renews_owner_projection_lease() {
        // The exact production bug: a device's projection is published with
        // a lease, the lease expires, and `terminal.list` (and every other
        // device-owned ability) silently drops out of `namespace.resolve`
        // with NODATA. Heartbeat must renew the lease so the projection
        // stays resolvable without a full re-advertise.
        let registry = PresenceRegistry::new();
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let owner_ura = "easynet:///r/realm/device/a";
        registry.insert(owner_ura.to_string(), make_dispatch_sender());

        let ability_ura =
            crate::core::ura::owner_ability_ura(owner_ura, "terminal.list").expect("ability ura");
        let publish_at = 1_000_i64;
        let lease = crate::daemon::federation::read_model::owner_projection::lease_expiry_from_now(
            publish_at,
        );
        catalog.upsert_projection(
            crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                owner_ura.to_string(),
                owner_ura.to_string(),
                1,
                1,
                "sha256:digest".to_string(),
                lease,
                vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
                    ability_ura: ability_ura.clone(),
                    owner_ura: owner_ura.to_string(),
                    namespace: "terminal".to_string(),
                    local_name: "list".to_string(),
                    descriptor_revision: "sha256:desc".to_string(),
                    schema_ref: None,
                    schema_hash: None,
                    policy_ref: "visibility:PUBLIC".to_string(),
                    route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                    tags: Vec::new(),
                    callable_summary:
                        crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                            "terminal.list",
                        ),
                }],
            ),
        );

        // After the lease expires, the projection is filtered out.
        let after_expiry = lease + 1;
        assert!(catalog.get_at(owner_ura, after_expiry).is_none());

        // A heartbeat at that moment renews the lease...
        let req = HeartbeatRequest {
            since_abilities_revision: 11,
            refresh_owner_uras: vec![owner_ura.to_string()],
        };
        let resp = handle_heartbeat(&req, &registry, &catalog, after_expiry);
        assert_eq!(resp.refreshed_owner_count, 1);
        assert_eq!(resp.hub_abilities_diff.revision, 11);

        // ...and the device-owned ability is resolvable again, with its
        // contents and revision unchanged (lease-only refresh).
        let got = catalog
            .get_at(owner_ura, after_expiry)
            .expect("lease renewed");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "list");
        let row = catalog.projection_for_owner(owner_ura).unwrap();
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
        let resp = handle_heartbeat(&req, &registry, &catalog, 5_000);
        assert_eq!(resp.refreshed_owner_count, 0);
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
        registry.insert(
            "easynet:///r/realm/device/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/a".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/b".to_string(),
            make_dispatch_sender(),
        );
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: None,
                include_abilities: false,
                filter: None,
            },
            &registry,
            None,
            &catalog,
            None,
        )
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
        registry.insert(
            "easynet:///r/realm-a/device/x".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm-b/device/y".to_string(),
            make_dispatch_sender(),
        );
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm-a".to_string()),
                include_abilities: false,
                filter: None,
            },
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
    fn handle_resolve_includes_device_owned_ability_routes_for_live_devices() {
        let registry = PresenceRegistry::new();
        let self_device_ura = "easynet:///r/realm/device/dev-1";
        registry.insert(self_device_ura.to_string(), make_dispatch_sender());

        let local_publication = crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot::from_owner_public_names(
            self_device_ura,
            &["agent.list", "skill.list", "plugin.dynamic"],
        );
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let resp = handle_resolve_at(
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/device/".to_string()),
                include_abilities: true,
                filter: None,
            },
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
            names.contains("agent.list"),
            "device route summary must include agent.list; got {names:?}"
        );
        assert!(
            names.contains("skill.list"),
            "device route summary must include skill.list; got {names:?}"
        );
        assert!(
            names.contains("plugin.dynamic"),
            "live dynamic publication must be visible; got {names:?}"
        );
    }

    #[test]
    fn handle_resolve_does_not_fabricate_profile_for_remote_device() {
        // A hub resolving a remote device has no matching local catalog rows,
        // so it can only publish what that device advertised (here: nothing).
        let registry = PresenceRegistry::new();
        let remote_device = "easynet:///r/realm/device/dev-remote";
        registry.insert(remote_device.to_string(), make_dispatch_sender());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/device/".to_string()),
                include_abilities: true,
                filter: None,
            },
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
        registry.insert(
            "easynet:///r/realm/device/dev-1".to_string(),
            make_dispatch_sender(),
        );
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
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/agent/".to_string()),
                include_abilities: true,
                filter: None,
            },
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
    fn handle_resolve_does_not_surface_expired_owner_projection() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/device/dev-1".to_string(),
            make_dispatch_sender(),
        );
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
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/device/".to_string()),
                include_abilities: true,
                filter: None,
            },
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
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/device/".to_string()),
                include_abilities: true,
                filter: None,
            },
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
    fn namespace_resolve_returns_typed_final_route_for_device_ability() {
        let registry = PresenceRegistry::new();
        let owner_ura = "easynet:///r/realm/device/dev-1";
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, "agent.list")
            .expect("device ability ura");
        registry.insert(owner_ura.to_string(), make_dispatch_sender());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(projection_row_for(
            owner_ura,
            vec![projection_summary(owner_ura, &ability_ura, "agent", "list")],
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
            ResolveAnswerKind::FinalRoute.as_str_name()
        );
        assert_eq!(answer["owner_ura"], owner_ura);
        assert_eq!(answer["ability_ura"], ability_ura);
        assert_eq!(
            answer["release_profile"],
            ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        assert_eq!(
            answer["next_hop"]["local_device_ability"]["dispatch_name"],
            "agent.list"
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
        let owner_ura = "easynet:///r/realm/device/dev-1";
        registry.insert(owner_ura.to_string(), make_dispatch_sender());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();

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
        let owner_ura = "easynet:///r/realm/device/dev-1";
        registry.insert(owner_ura.to_string(), make_dispatch_sender());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, "agent.list")
            .expect("device ability ura");
        catalog.upsert_projection(projection_row_for(
            owner_ura,
            vec![projection_summary(owner_ura, &ability_ura, "agent", "list")],
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
        registry.insert(host_ura.to_string(), make_dispatch_sender());
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
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole, TrustedPrincipalOwner,
        };
        let entry = TrustedAgent {
            agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
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
                owner_username: Some("alice".to_string()),
                added_at_unix_ms: 1_700_000_000_001,
            }],
            Vec::new(),
        )
        .expect("anchor");
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
                presented_pubkey_b64: None,
                presented_pubkey_hex: None,
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
        assert_eq!(resp.principal_owner_username.as_deref(), Some("alice"));
    }

    #[test]
    fn handle_resolve_key_returns_none_when_ura_not_in_anchor() {
        use crate::daemon::trust::anchor::RealmTrustAnchor;
        let anchor = RealmTrustAnchor::default();
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: "easynet:///r/realm-a/device/missing".to_string(),
                presented_pubkey_b64: None,
                presented_pubkey_hex: None,
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
        use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let alice = "easynet:///r/realm/user/alice";
        let entries = [pk_a, pk_b].into_iter().map(|pk| TrustedAgent {
            agent_ura: alice.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustedAgentRole::User,
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
                presented_pubkey_hex: None,
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
                presented_pubkey_hex: None,
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
                presented_pubkey_hex: None,
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
        use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let other = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let device = "easynet:///r/realm/device/node-a";
        let anchor = RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: device.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustedAgentRole::Device,
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
                presented_pubkey_hex: None,
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
                presented_pubkey_hex: None,
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
        registry.insert(
            "easynet:///r/realm-a/device/device-1".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm-a/device/device-2".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm-b/device/device-3".to_string(),
            make_dispatch_sender(),
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
        registry.insert(
            "easynet:///r/realm-a/device/node-xyz".to_string(),
            make_dispatch_sender(),
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
    fn handle_list_user_devices_ignores_legacy_agent_device_shape() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm-a/agent/node-legacy".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm-a/agent/alice.claude".to_string(),
            make_dispatch_sender(),
        );

        let resp = handle_list_user_devices(
            &ListUserDevicesRequest {
                realm: "realm-a".to_string(),
            },
            &registry,
        )
        .expect("legacy agent shapes are ignored");
        assert!(
            resp.devices.is_empty(),
            "legacy and hosted agent URAs must be ignored"
        );
    }

    #[test]
    fn handle_list_user_devices_returns_empty_when_no_match() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm-a/device/device".to_string(),
            make_dispatch_sender(),
        );

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
    fn handle_list_user_devices_rejects_prefix_matched_malformed_device_presence() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm-a/device/".to_string(),
            make_dispatch_sender(),
        );

        let error = handle_list_user_devices(
            &ListUserDevicesRequest {
                realm: "realm-a".to_string(),
            },
            &registry,
        )
        .expect_err("malformed device presence must fail closed");

        assert!(
            error.contains("matches realm device prefix")
                || error.contains("missing canonical device id"),
            "unexpected list_user_devices error: {error}"
        );
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
        registry.insert(ura.clone(), make_dispatch_sender());

        let resp = handle_revoke(
            &RevokeRequest {
                agent_ura: ura.clone(),
                purge_transaction_id: None,
                generation: 0,
                reason: String::new(),
                authority_ura: String::new(),
                protocol_version: 0,
                delivery_fence: 0,
            },
            &registry,
            None,
            &catalog,
        )
        .unwrap();
        assert!(resp.ack);
        assert!(resp.was_active);
        assert!(registry.lookup(&ura).is_none(), "must be removed");
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
    fn handle_revoke_requires_canonical_agent_ura() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let missing = handle_revoke(
            &RevokeRequest {
                agent_ura: String::new(),
                purge_transaction_id: None,
                generation: 0,
                reason: String::new(),
                authority_ura: String::new(),
                protocol_version: 0,
                delivery_fence: 0,
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
                generation: 0,
                reason: String::new(),
                authority_ura: String::new(),
                protocol_version: 0,
                delivery_fence: 0,
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
                generation: 0,
                reason: String::new(),
                authority_ura: String::new(),
                protocol_version: 0,
                delivery_fence: 0,
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
    fn purge_revoke_replay_returns_durable_result_and_reapplies_removal() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = "easynet:///r/realm/device/dev-1";
        let agent_ura = "easynet:///r/realm/agent/user.crash-window";
        registry.insert(host_ura.to_string(), make_dispatch_sender());
        let record = AdvertisedAgentRecord {
            agent_ura: agent_ura.to_string(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
        };
        handle_advertise_agent(
            &AdvertiseAgentRequest {
                agent_ura: agent_ura.to_string(),
                generation: 1,
                signing_authority: AdvertiseSigningAuthorityRequest::HostedBy {
                    host_ura: host_ura.to_string(),
                },
                public_key_hex: String::new(),
                host_node_id: Some("dev-1".into()),
            },
            Some(&advertised),
        )
        .unwrap();
        catalog.upsert_projection(projection_row_for(agent_ura, Vec::new()));
        let request = RevokeRequest {
            agent_ura: agent_ura.to_string(),
            purge_transaction_id: Some("fedcba9876543210fedcba9876543210".to_string()),
            generation: 1,
            reason: "test purge".to_string(),
            authority_ura: host_ura.to_string(),
            protocol_version:
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            delivery_fence: 1,
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
        let advertise = |generation| AdvertiseAgentRequest {
            agent_ura: agent_ura.to_string(),
            generation,
            signing_authority: AdvertiseSigningAuthorityRequest::HostedBy {
                host_ura: host_ura.to_string(),
            },
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
        };
        handle_advertise_agent(&advertise(1), Some(&advertised)).unwrap();
        let request = RevokeRequest {
            agent_ura: agent_ura.to_string(),
            purge_transaction_id: Some("11111111111111111111111111111111".into()),
            generation: 1,
            reason: "agent.purge".into(),
            authority_ura: host_ura.into(),
            protocol_version:
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            delivery_fence: 1,
        };
        let command = crate::daemon::persistence::federation_revoke::FederationRevokeCommand {
            protocol_version: request.protocol_version,
            transaction_id: request.purge_transaction_id.clone().unwrap(),
            agent_ura: agent_ura.into(),
            generation: 1,
            reason: request.reason.clone(),
            authority_ura: host_ura.into(),
            target_ura: agent_ura.into(),
        };
        crate::daemon::persistence::federation_revoke::prepare_revoke(&command, 1, false, None, 1)
            .unwrap();

        handle_advertise_agent(&advertise(2), Some(&advertised)).unwrap();
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
        registry.insert(agent_ura.to_string(), make_dispatch_sender());

        let response = handle_revoke(&request, &registry, Some(&advertised), &catalog)
            .expect("old prepared revoke completes as superseded");
        assert_eq!(
            response.disposition,
            Some(crate::daemon::persistence::federation_revoke::FederationRevokeDisposition::SupersededByNewIncarnation)
        );
        assert_eq!(advertised.get(agent_ura).unwrap().generation, 2);
        assert!(catalog.get(agent_ura).is_some());
        assert!(registry.lookup(agent_ura).is_some());
    }

    #[test]
    fn build_subscribe_directory_v2_snapshot_is_sorted() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/device/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/a".to_string(),
            make_dispatch_sender(),
        );

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
        registry.insert(
            "easynet:///r/realm/agent/user.device-carryover".to_string(),
            make_dispatch_sender(),
        );

        let err = build_subscribe_directory_v2_snapshot(&registry)
            .expect_err("agent URA must not publish as a directory device row");
        assert!(
            err.contains("not a canonical Device URA"),
            "unexpected error: {err}"
        );
    }
}

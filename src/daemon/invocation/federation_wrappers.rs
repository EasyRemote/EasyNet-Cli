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
// - The PresenceRegistry (lives in `daemon::invocation::state::presence`;
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
// output for SDK consumers across rust, go, python, java, node,
// swift, react. Field names are drawn from the proto definitions in
// `EasyNet-Axon/core/proto/axon/v1/federation.proto` so the JSON
// encoding is the canonical proto-JSON mapping.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::ability::conformance;
use crate::daemon::federation::read_model::advertised_agents::{
    AdvertisedAgentRecord, AdvertisedAgentSigningAuthority, AdvertisedAgentStore,
};
use crate::daemon::invocation::state::presence::PresenceRegistry;
#[cfg(test)]
use easynet_axon::pb::axon::v1 as axon_pb;
pub use easynet_axon::{
    DiscoverRequest, DiscoverResponse, ForwardInvokeRequest, ForwardInvokeResponse,
    ListUserDevicesRequest, ListUserDevicesResponse, ResolveAgentSummary, ResolveFilterRequest,
    ResolveKeyRequest, ResolveKeyResponse, ResolveRequest, ResolveResponse,
};

/// `federation.join` — caller's claimed URA is authoritative; no
/// hub-side `agent/a-X` minting (spec §5.1 URA scheme migration).
pub const ABILITY_FEDERATION_JOIN: &str = conformance::ABILITY_FEDERATION_JOIN;

/// `federation.advertise_agent` — records hosted-agent directory rows.
/// PresenceRegistry still owns transport liveness; resolve joins the
/// two so `/agent/<user>.<agent>` rows surface while online/offline
/// is derived from the host device's live `session.open`.
pub const ABILITY_FEDERATION_ADVERTISE_AGENT: &str =
    conformance::ABILITY_FEDERATION_ADVERTISE_AGENT;

/// `federation.heartbeat` — warns that liveness is now stream-derived
/// and returns a typed no-op success so legacy callers see "active"
/// without us re-implementing the unary heartbeat path.
pub const ABILITY_FEDERATION_HEARTBEAT: &str = conformance::ABILITY_FEDERATION_HEARTBEAT;

/// `federation.resolve` — projects both live PresenceRegistry URAs
/// and hosted-agent rows whose host device is presently online.
pub const ABILITY_FEDERATION_RESOLVE: &str = conformance::ABILITY_FEDERATION_RESOLVE;

/// `namespace.resolve` — RFC-005 typed namespace resolver surface.
/// This is a daemon ability reached through `axon.v1.Invocation`; it
/// returns an Axon `ResolveAnswer` proto-JSON projection, not legacy
/// directory rows.
pub const ABILITY_NAMESPACE_RESOLVE: &str = conformance::ABILITY_NAMESPACE_RESOLVE;

/// `namespace.proxy_resolve` — daemon-local typed namespace proxy.
/// The backend supplies the peer hub set, but the daemon owns trust
/// filtering, peer dialling, envelope signing, and typed
/// `ResolveAnswer` aggregation. This is the clean replacement for
/// backend product paths that previously consumed
/// legacy federation directory rows.
pub const ABILITY_NAMESPACE_PROXY_RESOLVE: &str = conformance::ABILITY_NAMESPACE_PROXY_RESOLVE;

/// `federation.subscribe_directory` — the only federation.* ability
/// served via `InvokeStream` (server-stream); the others go through
/// unary `Invoke`.
pub const ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY: &str =
    conformance::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY;

/// `federation.revoke` — operator-driven removal of an agent from
/// the registry via `PresenceRegistry::force_revoke`.
pub const ABILITY_FEDERATION_REVOKE: &str = conformance::ABILITY_FEDERATION_REVOKE;

/// `federation.forward_invoke` — push an inner envelope down a
/// target agent's `session.open` reverse channel; correlate the
/// reply by call_id (same scheme MVP uses).
pub const ABILITY_FEDERATION_FORWARD_INVOKE: &str = conformance::ABILITY_FEDERATION_FORWARD_INVOKE;

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

/// `federation.subscribe_directory_v2` — server-stream variant
/// of `subscribe_directory` that emits `DirectoryEvent` frames
/// (Snapshot / Upsert / Remove / Heartbeat) per PR-N3 spec
/// §2.2-2.3. Distinct from the legacy `federation.subscribe_
/// directory` (which emits `SubscribeDirectoryInitial` +
/// `PresenceEventDelta` shapes); the daemon serves both during
/// the v1→v2 migration so subscriber-side rollout can ramp
/// independently of hub upgrades.
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
/// so they show up in `federation.resolve(prefix=hub)`. PR-1 staging
/// accepts the call as a no-op success — the directory is presence-driven
/// via `session.open` membership, so the descriptors don't need separate
/// persistence. Without the handler the backend's boot path errors
/// `Unimplemented` and the realm directory is silently missing every
/// backend-owned ability.
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

/// `federation.status` — read-only boot-state projection backed by
/// `runtime::federation_init::FederationStatusProbe`.
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
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_FORWARD_INVOKE,
    ABILITY_FEDERATION_ADVERTISE_ABILITIES,
    ABILITY_FEDERATION_STATUS,
];

#[must_use]
pub fn handle_status() -> serde_json::Value {
    crate::runtime::federation_init::FederationStatusProbe::render()
}

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
pub struct AdvertiseAgentRequest {
    /// URA of the agent being advertised.
    pub agent_ura: String,
    /// New wire shape used by the publisher. Legacy callers may still
    /// send a top-level `host_ura`, so we accept both.
    #[serde(default)]
    pub signing_authority: Option<AdvertiseSigningAuthorityRequest>,
    #[serde(default)]
    pub public_key_hex: String,
    #[serde(default)]
    pub host_ura: Option<String>,
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
    fn to_record(&self) -> AdvertisedAgentRecord {
        let signing_authority = match &self.signing_authority {
            Some(AdvertiseSigningAuthorityRequest::SelfSigned) => {
                AdvertisedAgentSigningAuthority::SelfSigned
            }
            Some(AdvertiseSigningAuthorityRequest::HostedBy { host_ura }) => {
                AdvertisedAgentSigningAuthority::HostedBy {
                    host_ura: host_ura.clone(),
                }
            }
            None => match &self.host_ura {
                Some(host_ura) => AdvertisedAgentSigningAuthority::HostedBy {
                    host_ura: host_ura.clone(),
                },
                None => AdvertisedAgentSigningAuthority::SelfSigned,
            },
        };
        AdvertisedAgentRecord {
            agent_ura: self.agent_ura.clone(),
            public_key_hex: self.public_key_hex.clone(),
            host_node_id: self.host_node_id.clone(),
            signing_authority,
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
#[must_use]
pub fn handle_advertise_agent(
    request: &AdvertiseAgentRequest,
    store: Option<&AdvertisedAgentStore>,
) -> AdvertiseAgentResponse {
    if let Some(store) = store {
        store.upsert(request.to_record());
    }
    AdvertiseAgentResponse {
        ack: true,
        replaced_prior: false,
    }
}

// ─── federation.advertise_abilities ────────────────────────────────

/// Request payload for `federation.advertise_abilities`.
///
/// The current wire shape is RFC-005 owner projection publication:
/// the caller sends projection metadata plus bounded ability summaries.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AdvertiseAbilitiesRequest {
    pub owner_ura: String,
    pub host_device_ura: String,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub lease_expires_unix_ms: i64,
    #[serde(default)]
    pub ability_summaries: Vec<crate::runtime::owner_projection::AbilityProjectionSummary>,
}

/// Response payload for `federation.advertise_abilities`. Matches the
/// daemon-backed wrapper contract (`ack` + `count`). PR-1 staging always
/// returns `ack = true`; future PRs may surface partial-failure counts.
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
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
) -> AdvertiseAbilitiesResponse {
    let count = request.ability_summaries.len();
    let stored = if let Some(store) = catalog {
        store
            .upsert_projection(
                crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                    request.owner_ura.clone(),
                    request.host_device_ura.clone(),
                    request.projection_revision,
                    request.projection_digest.clone(),
                    request.lease_expires_unix_ms,
                    request.ability_summaries.clone(),
                ),
            )
            .is_stored()
    } else {
        true
    };
    AdvertiseAbilitiesResponse {
        ack: stored,
        count: if stored { count } else { 0 },
    }
}

// ─── federation.heartbeat ──────────────────────────────────────────

/// Request payload for `federation.heartbeat`.
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatRequest {
    /// URA of the agent reporting in. Used only for log context now;
    /// liveness comes from the registry's stream membership. The device's
    /// heartbeat payload (see `runtime/advertise.rs`) does not send this
    /// field, so it must deserialize as optional — a missing `agent_ura`
    /// is a valid heartbeat, not a wire error.
    #[serde(default)]
    pub agent_ura: String,
    /// Owner URAs whose ability projection leases this heartbeat renews.
    /// The device batches its own owners (device + hosted agents) here so
    /// the hub keeps their projections live without a full re-advertise.
    #[serde(default)]
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
}

/// Handle a `federation.heartbeat` invocation.
///
/// Logs a warning that the unary heartbeat is a no-op in the new
/// architecture (PresenceRegistry membership is the liveness signal),
/// then returns a typed success so legacy callers don't fail. The
/// `realm_directory_size` field is read from the registry snapshot
/// for transparency to operators reading audit logs.
#[must_use]
pub(crate) fn handle_heartbeat(
    request: &HeartbeatRequest,
    registry: &PresenceRegistry,
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    now_unix_ms: i64,
) -> HeartbeatResponse {
    // RFC-005: heartbeat renews the owner projection lease only; it must
    // not mutate projection contents, revision, or digest. Extend the
    // lease for every owner the device batched into `refresh_owner_uras`
    // so its device/hosted-agent abilities stay resolvable between full
    // re-advertise cycles. Unknown owners are skipped (the device must
    // `advertise_abilities` before its first projection exists).
    let mut refreshed_owner_count = 0_usize;
    if let Some(catalog) = catalog {
        let new_expiry = crate::runtime::owner_projection::lease_expiry_from_now(now_unix_ms);
        for owner_ura in &request.refresh_owner_uras {
            let owner_ura = owner_ura.trim();
            if !owner_ura.is_empty() && catalog.refresh_lease(owner_ura, new_expiry) {
                refreshed_owner_count += 1;
            }
        }
    }
    HeartbeatResponse {
        membership_status: "active".to_string(),
        realm_directory_size: registry.online_count(),
        refreshed_owner_count,
    }
}

// ─── federation.resolve ────────────────────────────────────────────

/// Legacy v1 directory-stream projection. Kept separate from
/// `ResolveAgentSummary` because `subscribe_directory` still speaks
/// the historical `membership_ura` field while
/// `federation.resolve` now matches the backend helper's `ura`
/// field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSummary {
    pub membership_ura: String,
    pub status: String,
}

/// Handle a `federation.resolve` invocation.
///
/// `catalog` is the optional owner projection read model the daemon
/// constructs at boot. When `request.include_abilities` is true and
/// the store has a row for an in-presence owner URA, the response
/// carries namespace-safe projection summaries in the historical
/// `abilities` output field. Hub-mode daemons in production always
/// wire a catalog; build-without-catalog paths pass `None` and the
/// abilities slot stays empty.
#[must_use]
pub fn handle_resolve(
    request: &ResolveRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    self_device_ura: Option<&str>,
) -> ResolveResponse {
    handle_resolve_at(
        request,
        registry,
        advertised_agents,
        catalog,
        self_device_ura,
        crate::daemon::federation::directory::now_unix_ms(),
    )
}

/// Deterministic variant of `handle_resolve` for tests and replay checks.
/// `now_unix_ms` is used only to filter expired owner projection read-model
/// rows; liveness still comes from `PresenceRegistry`.
#[must_use]
pub(crate) fn handle_resolve_at(
    request: &ResolveRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    self_device_ura: Option<&str>,
    now_unix_ms: i64,
) -> ResolveResponse {
    let prefix = request.effective_ura_prefix();
    let want_abilities = request.wants_abilities();
    let mut agents = std::collections::BTreeMap::<String, ResolveAgentSummary>::new();

    for ura in registry.snapshot() {
        if prefix.is_some_and(|p| !ura.starts_with(p)) {
            continue;
        }
        let abilities = if want_abilities {
            resolved_owner_projection_values(catalog, &ura, self_device_ura, now_unix_ms)
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
                catalog
                    .and_then(|c| c.get_at(&record.agent_ura, now_unix_ms))
                    .unwrap_or_default()
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

    ResolveResponse {
        agents: agents.into_values().collect(),
    }
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
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
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
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    now_unix_ms: i64,
) -> Value {
    crate::daemon::invocation::route_resolver::DaemonRouteResolver::new(
        registry,
        advertised_agents,
        catalog,
    )
    .at(now_unix_ms)
    .resolve_query_json(query)
}

/// Namespace-safe ability summaries for one in-presence owner, merging
/// two authorities:
///
/// 1. The owner's **static device profile** (RFC-005 §4 / D105): a
///    device is the authority for its own control-plane surface
///    (`terminal.*`, `agent.*`, `skill.*`, `fs.*`, `meta.*`). This is
///    derived from the live registry and **never expires**, so a device's
///    own abilities stay resolvable on its own daemon even though the
///    daemon never receives its projection back into its local catalog
///    (it advertises that projection up to the hub, not to itself), and
///    regardless of any hub-side lease.
/// 2. The **hub projection catalog** (lease-filtered): dynamic projection
///    summaries for hosted agents and any owner this daemon is the hub
///    for. These overlay the static profile by public name.
///
/// The static device profile is included only when `owner_ura` is THIS
/// daemon's own device URA (`self_device_ura`): a daemon is the authority
/// for its own device surface, but a hub resolving a *remote* device must
/// not fabricate that device's profile — it only knows what the remote
/// device advertised into the catalog.
///
/// Without (1), the device daemon's own catalog is empty and every
/// device-owned ability lists as NODATA — the production bug behind the
/// empty Abilities page and `terminal.list`/`agent.list`/`skill.list`
/// "owner is online but does not publish" failures.
fn resolved_owner_projection_values(
    catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    owner_ura: &str,
    self_device_ura: Option<&str>,
    now_unix_ms: i64,
) -> Vec<serde_json::Value> {
    let mut by_public_name = std::collections::BTreeMap::<String, serde_json::Value>::new();
    let mut order = Vec::new();
    let mut push = |summary: serde_json::Value| {
        let Some(key) = crate::runtime::owner_projection::summary_from_value(&summary)
            .and_then(|parsed| crate::runtime::owner_projection::summary_public_name(&parsed))
        else {
            return;
        };
        if by_public_name.insert(key.clone(), summary).is_none() {
            order.push(key);
        }
    };

    if self_device_ura.is_some_and(|self_ura| self_ura == owner_ura) {
        for summary in device_owner_projection_values(owner_ura) {
            push(summary);
        }
    }
    if let Some(catalog) = catalog {
        for summary in catalog.get_at(owner_ura, now_unix_ms).unwrap_or_default() {
            push(summary);
        }
    }

    order
        .into_iter()
        .filter_map(|key| by_public_name.remove(&key))
        .collect()
}

/// Static device-profile ability summaries for a device-owned URA. Empty
/// for non-device owners (agents/hubs publish through the catalog).
fn device_owner_projection_values(owner_ura: &str) -> Vec<serde_json::Value> {
    if !crate::ura::parse_ura(owner_ura)
        .map(|parsed| parsed.kind == crate::ura::URAKind::Device)
        .unwrap_or(false)
    {
        return Vec::new();
    }
    crate::daemon::ability::catalog::profiles::device::descriptors_for(owner_ura)
        .iter()
        .filter_map(|descriptor| {
            crate::runtime::owner_projection::summary_from_descriptor(descriptor).ok()
        })
        .filter_map(|summary| serde_json::to_value(summary).ok())
        .collect()
}

// ─── federation.resolve_key ────────────────────────────────────────

fn resolve_key_response(public_key_b64: &str, all_keys_b64: Vec<String>) -> ResolveKeyResponse {
    let public_key_hex = BASE64_STANDARD
        .decode(public_key_b64.as_bytes())
        .map(hex::encode)
        .unwrap_or_default();
    ResolveKeyResponse {
        public_key_b64: public_key_b64.to_string(),
        public_key_hex,
        public_keys_b64: if all_keys_b64.is_empty() {
            vec![public_key_b64.to_string()]
        } else {
            all_keys_b64
        },
    }
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
) -> Option<ResolveKeyResponse> {
    // DEC-EU multi-device user URAs: caller supplies the pubkey it
    // observed on the envelope; we confirm it's in the user bucket.
    // Single-value roles (hub/backend/device) ignore this field and
    // fall through to the legacy lookup below.
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
            return Some(resolve_key_response(
                &entry.public_key_b64,
                all_user_keys_b64(trust_anchor, &request.agent_ura),
            ));
        }
        if matches!(
            crate::ura::parse_ura(&request.agent_ura).map(|parsed| parsed.kind),
            Ok(crate::ura::URAKind::User)
        ) {
            return None;
        }
    }
    trust_anchor.lookup(&request.agent_ura).map(|entry| {
        resolve_key_response(
            &entry.public_key_b64,
            all_user_keys_b64(trust_anchor, &request.agent_ura),
        )
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
    resolver: &crate::runtime::keyring::resolver::FederatedUserResolver,
) -> DiscoverResponse {
    use crate::runtime::keyring::resolver::FederatedUserOutcome;
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
/// `realm` is the user-owned device realm to enumerate on each
/// peer; `peer_hub_urls` are the exact peer TLS listener
/// URLs the backend selected from its `user_peer_hubs` table.
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
pub struct NamespaceProxyResolveRequest {
    #[serde(default)]
    pub peer_hub_urls: Vec<String>,
    #[serde(default, rename = "queryName", alias = "query_name")]
    pub query_name: String,
    #[serde(default, rename = "qtype", alias = "qType")]
    pub qtype: String,
    #[serde(default, rename = "callerUra", alias = "caller_ura")]
    pub caller_ura: String,
    #[serde(default, rename = "subjectUra", alias = "subject_ura")]
    pub subject_ura: String,
    #[serde(default, rename = "realmHint", alias = "realm_hint")]
    pub realm_hint: String,
    #[serde(default, rename = "abilityName", alias = "ability_name")]
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
) -> ListUserDevicesResponse {
    let realm_device_prefix = crate::ura::realm_device_prefix(&request.realm);
    let snapshot = registry.snapshot();
    let devices = snapshot
        .into_iter()
        .filter(|ura| ura.starts_with(&realm_device_prefix))
        .map(|ura| {
            // Canonical v4.1.4 device URAs are the only input
            // that should survive the prefix filter above.
            let node_id = match crate::ura::parse_ura(&ura) {
                Ok(parsed) if parsed.kind == crate::ura::URAKind::Device => {
                    parsed.device_id().map(str::to_string).unwrap_or_default()
                }
                _ => String::new(),
            };
            crate::daemon::federation::directory::DirectoryEntry {
                agent_ura: ura,
                node_id,
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            }
        })
        .collect();
    ListUserDevicesResponse { devices }
}

// ─── federation.revoke ─────────────────────────────────────────────

/// Request payload for `federation.revoke`.
#[derive(Debug, Clone, Deserialize)]
pub struct RevokeRequest {
    /// Modern callers send `agent_ura`; older callers send
    /// `target_ura`. We accept both.
    #[serde(default)]
    pub target_ura: String,
    #[serde(default)]
    pub agent_ura: String,
}

impl RevokeRequest {
    #[must_use]
    fn effective_target_ura(&self) -> &str {
        if !self.target_ura.is_empty() {
            &self.target_ura
        } else {
            &self.agent_ura
        }
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
}

/// Handle a `federation.revoke` invocation. Forces removal of the
/// target session and records whether the target was online at
/// revoke time so the caller can distinguish a real revoke from a
/// no-op.
#[must_use]
pub fn handle_revoke(
    request: &RevokeRequest,
    registry: &PresenceRegistry,
    advertised_agents: Option<&AdvertisedAgentStore>,
) -> RevokeResponse {
    let target_ura = request.effective_target_ura();
    let was_active = registry.lookup(target_ura).is_some()
        || advertised_agents
            .and_then(|store| store.get(target_ura))
            .map(|record| match record.host_ura() {
                Some(host_ura) => registry.lookup(host_ura).is_some(),
                None => registry.lookup(&record.agent_ura).is_some(),
            })
            .unwrap_or(false);
    let _displaced = registry.force_revoke(target_ura);
    if let Some(store) = advertised_agents {
        let _removed = store.remove(target_ura);
    }
    RevokeResponse {
        ack: true,
        was_active,
    }
}

// ─── federation.forward_invoke ─────────────────────────────────────

/// Reason text emitted on `Status::failed_precondition` when the
/// target presence-registry lookup misses on the local-realm
/// fast-path. Wire-stable per DEC-N4 §2.1.
pub const FORWARD_INVOKE_TARGET_OFFLINE_REASON: &str = "target_offline";

/// Reason text emitted when the target device's dispatch channel is
/// full. A full channel means the device is SLOW (its session drain
/// is behind), not DEAD: the device stays in the presence registry
/// and only the triggering call fails, retryable. Evicting on full
/// — the pre-2026-06-13 policy — turned a load spike into a false
/// offline plus a failure avalanche for every pending call
/// (measured: one >256-frame burst killed 73% of 2048 in-flight
/// invocations).
pub const FORWARD_INVOKE_TARGET_BUSY_REASON: &str = "target_busy_retry";

/// Handle a local-realm `federation.forward_invoke` invocation.
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
        .map(|ura| AgentSummary {
            membership_ura: ura,
            status: "active".to_string(),
        })
        .collect();
    SubscribeDirectoryInitial { agents }
}

/// **PR-N3 N3-streaming-1**. Build the initial `Snapshot` frame
/// for the v2 subscribe stream from the local presence registry.
/// Each in-registry URA projects to a `DirectoryAgentSummary` via
/// the pure-data adapter; sorted iteration mirrors v1's
/// deterministic-bytes-from-deterministic-state contract.
#[must_use]
pub fn build_subscribe_directory_v2_snapshot(
    registry: &PresenceRegistry,
) -> crate::daemon::federation::directory::DirectoryEvent {
    crate::daemon::federation::directory::presence_uras_to_directory_snapshot(
        registry.snapshot(),
        crate::daemon::federation::directory::now_unix_ms(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_dispatch_sender() -> crate::daemon::invocation::state::presence::DispatchSender {
        let (tx, _rx) = mpsc::channel(256);
        tx
    }

    fn projection_summary(
        owner_ura: &str,
        ability_ura: &str,
        namespace: &str,
        local_name: &str,
    ) -> crate::runtime::owner_projection::AbilityProjectionSummary {
        crate::runtime::owner_projection::AbilityProjectionSummary {
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
            callable_summary: crate::runtime::owner_projection::AbilityCallableSummary::minimal(
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
        summaries: Vec<crate::runtime::owner_projection::AbilityProjectionSummary>,
    ) -> crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow {
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            "easynet:///r/realm/device/dev-1".to_string(),
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
        assert_eq!(FEDERATION_ABILITIES.len(), 14);
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
        };
        let resp = handle_join(&req);
        assert_eq!(resp.membership_ura, req.membership_ura);
        assert_eq!(resp.realm, req.realm);
        assert_eq!(resp.join_receipt_hash.len(), 64);
    }

    #[test]
    fn handle_advertise_agent_returns_typed_ack() {
        let store = AdvertisedAgentStore::new();
        let req = AdvertiseAgentRequest {
            agent_ura: "easynet:///r/realm/agent/user.n1".to_string(),
            signing_authority: Some(AdvertiseSigningAuthorityRequest::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".to_string(),
            }),
            public_key_hex: String::new(),
            host_ura: None,
            host_node_id: Some("dev-1".to_string()),
        };
        let resp = handle_advertise_agent(&req, Some(&store));
        assert!(resp.ack);
        assert!(!resp.replaced_prior);
        let stored = store
            .get("easynet:///r/realm/agent/user.n1")
            .expect("advertised agent must be stored");
        assert_eq!(stored.host_ura(), Some("easynet:///r/realm/device/dev-1"));
    }

    #[test]
    fn handle_advertise_abilities_stores_owner_projection_row() {
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        let owner_ura = "easynet:///r/realm/device/dev-1";
        let req = AdvertiseAbilitiesRequest {
            owner_ura: owner_ura.to_string(),
            host_device_ura: owner_ura.to_string(),
            projection_revision: 7,
            projection_digest: "sha256:projection".to_string(),
            lease_expires_unix_ms: 1_714_493_100_000,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/device.dev-1.fs.read",
                "fs",
                "read",
            )],
        };

        let resp = handle_advertise_abilities(&req, Some(&catalog));

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
            projection_revision: 7,
            projection_digest: "sha256:newer".to_string(),
            lease_expires_unix_ms: 4_102_444_800_000,
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
            projection_revision: 6,
            projection_digest: "sha256:stale".to_string(),
            lease_expires_unix_ms: 4_102_444_800_000,
            ability_summaries: vec![projection_summary(
                owner_ura,
                "easynet:///r/realm/ability/device.dev-1.fs.read",
                "fs",
                "read",
            )],
        };

        assert_eq!(
            handle_advertise_abilities(&newer, Some(&catalog)),
            AdvertiseAbilitiesResponse {
                ack: true,
                count: 1
            }
        );
        assert_eq!(
            handle_advertise_abilities(&stale, Some(&catalog)),
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
            agent_ura: "easynet:///r/realm/device/a".to_string(),
            refresh_owner_uras: Vec::new(),
        };
        let resp = handle_heartbeat(&req, &registry, None, 1_000);
        assert_eq!(resp.membership_status, "active");
        assert_eq!(resp.realm_directory_size, 2);
        assert_eq!(resp.refreshed_owner_count, 0);
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
            crate::ura::owner_ability_ura(owner_ura, "terminal.list").expect("ability ura");
        let publish_at = 1_000_i64;
        let lease = crate::runtime::owner_projection::lease_expiry_from_now(publish_at);
        catalog.upsert_projection(
            crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
                owner_ura.to_string(),
                owner_ura.to_string(),
                1,
                "sha256:digest".to_string(),
                lease,
                vec![crate::runtime::owner_projection::AbilityProjectionSummary {
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
                        crate::runtime::owner_projection::AbilityCallableSummary::minimal(
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
            agent_ura: owner_ura.to_string(),
            refresh_owner_uras: vec![owner_ura.to_string()],
        };
        let resp = handle_heartbeat(&req, &registry, Some(&catalog), after_expiry);
        assert_eq!(resp.refreshed_owner_count, 1);

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
            agent_ura: "easynet:///r/realm/device/a".to_string(),
            refresh_owner_uras: vec!["easynet:///r/realm/device/never-published".to_string()],
        };
        let resp = handle_heartbeat(&req, &registry, Some(&catalog), 5_000);
        assert_eq!(resp.refreshed_owner_count, 0);
    }

    #[test]
    fn heartbeat_request_deserializes_device_payload_without_agent_ura() {
        // The device's actual heartbeat payload (runtime/advertise.rs) sends
        // `since_abilities_revision` + `refresh_owner_uras` and NO
        // `agent_ura`. The wrapper request must accept it — a missing
        // `agent_ura` is a valid heartbeat, not a deserialization error.
        let payload = serde_json::json!({
            "since_abilities_revision": 7,
            "refresh_owner_uras": ["easynet:///r/realm/device/a"],
        });
        let req: HeartbeatRequest =
            serde_json::from_value(payload).expect("device heartbeat payload must deserialize");
        assert!(req.agent_ura.is_empty());
        assert_eq!(req.refresh_owner_uras, vec!["easynet:///r/realm/device/a"]);
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

        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: None,
                include_abilities: false,
                filter: None,
            },
            &registry,
            None,
            None,
            None,
        );
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

        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm-a".to_string()),
                include_abilities: false,
                filter: None,
            },
            &registry,
            None,
            None,
            None,
        );
        assert_eq!(resp.agents.len(), 1);
        assert_eq!(resp.agents[0].ura, "easynet:///r/realm-a/device/x");
    }

    #[test]
    fn handle_resolve_includes_device_owned_ability_routes_for_live_devices() {
        let registry = PresenceRegistry::new();
        let self_device_ura = "easynet:///r/realm/device/dev-1";
        registry.insert(self_device_ura.to_string(), make_dispatch_sender());

        // This daemon IS dev-1: its own device profile is the authority for
        // its control-plane surface, included even with an empty catalog.
        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/device/".to_string()),
                include_abilities: true,
                filter: None,
            },
            &registry,
            None,
            None,
            Some(self_device_ura),
        );

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
    }

    #[test]
    fn handle_resolve_does_not_fabricate_profile_for_remote_device() {
        // A hub resolving a DIFFERENT device must not synthesize that
        // device's profile from descriptors_for — it only knows what the
        // remote device advertised (here: nothing). Self gate is another
        // device's URA, so the static profile must not leak in.
        let registry = PresenceRegistry::new();
        let remote_device = "easynet:///r/realm/device/dev-remote";
        registry.insert(remote_device.to_string(), make_dispatch_sender());

        let resp = handle_resolve(
            &ResolveRequest {
                ura_prefix: Some("easynet:///r/realm/device/".to_string()),
                include_abilities: true,
                filter: None,
            },
            &registry,
            None,
            None,
            Some("easynet:///r/realm/device/dev-self"),
        );

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
            Some(&catalog),
            None,
        );
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
            Some(&catalog),
            None,
            999,
        );
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
            Some(&catalog),
            None,
            1_000,
        );
        assert_eq!(expired.agents.len(), 1);
        assert!(expired.agents[0].abilities.is_empty());
    }

    #[test]
    fn namespace_resolve_returns_typed_final_route_for_device_ability() {
        let registry = PresenceRegistry::new();
        let owner_ura = "easynet:///r/realm/device/dev-1";
        let ability_ura =
            crate::ura::owner_ability_ura(owner_ura, "agent.list").expect("device ability ura");
        registry.insert(owner_ura.to_string(), make_dispatch_sender());
        let catalog =
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new();
        catalog.upsert_projection(projection_row_for(
            owner_ura,
            vec![projection_summary(owner_ura, &ability_ura, "agent", "list")],
        ));

        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "queryName": owner_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
                "abilityName": "agent.list",
            }),
            &registry,
            None,
            Some(&catalog),
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answerKind"],
            axon_pb::ResolveAnswerKind::FinalRoute.as_str_name()
        );
        assert_eq!(answer["ownerUra"], owner_ura);
        assert_eq!(answer["abilityUra"], ability_ura);
        assert_eq!(
            answer["releaseProfile"],
            axon_pb::ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        assert_eq!(
            answer["nextHop"]["localDeviceAbility"]["dispatchName"],
            "agent.list"
        );
        assert_eq!(
            answer["selectedRoute"]["gates"]["ability"],
            axon_pb::GateResult::Pass.as_str_name()
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
                "queryName": owner_ura,
                "qtype": "ROUTE",
                "abilityName": "agent.list",
            }),
            &registry,
            None,
            Some(&catalog),
            1_714_493_100_000,
        );

        assert_eq!(
            answer["answerKind"],
            axon_pb::ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            axon_pb::NegativeReason::Nodata.as_str_name()
        );
        assert_eq!(answer["nextHop"]["noRoute"], serde_json::json!({}));
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
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
        });

        let answer = handle_namespace_resolve_at(
            &serde_json::json!({
                "queryName": "easynet:///r/realm/agent/alice.",
                "qtype": "RESOLVE_TYPE_DIRECTORY_LISTING",
            }),
            &registry,
            Some(&advertised),
            None,
            1_714_493_100_000,
        );

        let records = answer["records"]
            .as_array()
            .expect("typed answer must carry records");
        assert!(
            records.iter().any(|record| {
                record["recordType"] == axon_pb::RecordType::HostedBy.as_str_name()
                    && record["value"]["hostedBy"]["hostedUra"] == agent_ura
                    && record["value"]["hostedBy"]["hostUra"] == host_ura
                    && record["value"]["hostedBy"]["hostNodeId"] == "dev-1"
            }),
            "hosted agent namespace directory must include hosted_by placement; got {records:#?}"
        );
    }

    #[test]
    fn handle_resolve_key_returns_pubkey_when_present_in_anchor() {
        use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
        let entry = TrustedAgent {
            agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        let resp = handle_resolve_key(
            &ResolveKeyRequest {
                agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
                presented_pubkey_b64: None,
                presented_pubkey_hex: None,
            },
            &anchor,
        )
        .expect("hit");
        assert_eq!(
            resp.public_key_b64,
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
        assert_eq!(
            resp.public_key_hex,
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
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
            resp.is_none(),
            "miss must surface as None for caller status mapping"
        );
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
        assert!(resp.is_none(), "unknown pubkey under known user must miss");
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
        use crate::runtime::keyring::federated_bindings::{
            FederatedBindingsStore, FederatedUserBinding,
        };
        use crate::runtime::keyring::resolver::FederatedUserResolver;
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
        use crate::runtime::keyring::federated_bindings::FederatedBindingsStore;
        use crate::runtime::keyring::resolver::FederatedUserResolver;
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
        use crate::runtime::keyring::federated_bindings::FederatedBindingsStore;
        use crate::runtime::keyring::resolver::FederatedUserResolver;
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
        use crate::runtime::keyring::federated_bindings::FederatedBindingsStore;
        use crate::runtime::keyring::resolver::FederatedUserResolver;
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
        );
        assert_eq!(resp.devices.len(), 2);
        let expected_prefix = crate::ura::realm_device_prefix("realm-a");
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
        );
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
        );
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
        );
        assert!(resp.devices.is_empty());
    }

    #[test]
    fn list_user_devices_requests_reject_retired_tenant_id_field() {
        let list = serde_json::from_value::<ListUserDevicesRequest>(serde_json::json!({
            "tenant_id": "tenant-a",
        }));
        assert!(list.is_err(), "peer request must require `realm`");

        let proxy = serde_json::from_value::<ProxyListUserDevicesRequest>(serde_json::json!({
            "tenant_id": "tenant-a",
            "peer_hub_urls": ["https://peer.example:50443"],
        }));
        assert!(proxy.is_err(), "proxy request must require `realm`");
    }

    #[test]
    fn handle_revoke_reports_was_active_correctly() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/n1".to_string();
        registry.insert(ura.clone(), make_dispatch_sender());

        let resp = handle_revoke(
            &RevokeRequest {
                target_ura: ura.clone(),
                agent_ura: String::new(),
            },
            &registry,
            None,
        );
        assert!(resp.ack);
        assert!(resp.was_active);
        assert!(registry.lookup(&ura).is_none(), "must be removed");
    }

    #[test]
    fn handle_revoke_removes_hosted_agent_rows_too() {
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/user.alice".into(),
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        });
        let resp = handle_revoke(
            &RevokeRequest {
                target_ura: String::new(),
                agent_ura: "easynet:///r/realm/agent/user.alice".to_string(),
            },
            &registry,
            Some(&advertised),
        );
        assert!(resp.ack);
        assert!(!resp.was_active);
        assert!(
            advertised
                .get("easynet:///r/realm/agent/user.alice")
                .is_none(),
            "revoke must remove advertised hosted-agent rows"
        );
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
                target_ura: "easynet:///r/realm/device/n1".to_string(),
                inner_envelope_b64: String::new(),
                causal_context_bytes: Vec::new(),
                forward_deadline_ms: 0,
                origin_caller: None,
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
                target_ura: "easynet:///r/realm/device/n1".to_string(),
                inner_envelope_b64: String::new(),
                causal_context_bytes: Vec::new(),
                forward_deadline_ms: 0,
                origin_caller: None,
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
        // caller's `runtime.invoke_remote` initiator (or the CLI
        // bridge in `daemon::invocation::federation_invoke`) through the
        // dispatcher's JSON deserialise step. The dispatcher
        // surfaces these to the target's session frame so PR-N5's
        // InvocationReceipt can stamp `causal_context.list` and
        // DEC-N5 §3 can derive the inner deadline.
        //
        // `causal_context_bytes` rides the wire as a base64 STRING,
        // not a JSON byte-array — the Axon deserializer rejects a
        // sequence with `invalid type: sequence, expected a
        // base64-encoded string`. Serialise the canonical struct (the
        // exact shape a producer emits) so this test exercises the
        // real wire contract rather than a hand-rolled object that can
        // drift from it.
        let audit_bytes: Vec<u8> = vec![0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0xFF];
        let original = ForwardInvokeRequest {
            target_ura: "easynet:///r/realm/device/n1".to_string(),
            inner_envelope_b64: String::new(),
            causal_context_bytes: audit_bytes.clone(),
            forward_deadline_ms: 12_345,
            origin_caller: None,
        };
        let bytes = serde_json::to_vec(&original).unwrap();

        // Pin the wire shape: the audit field must be a base64 string.
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            wire["causal_context_bytes"].as_str(),
            Some(BASE64_STANDARD.encode(&audit_bytes).as_str()),
            "causal_context_bytes must serialise as a base64 string, not a JSON array"
        );

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
            "target_ura": "easynet:///r/realm/device/n1",
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
            "easynet:///r/realm/device/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/a".to_string(),
            make_dispatch_sender(),
        );

        let initial = build_subscribe_directory_initial(&registry);
        let uras: Vec<&str> = initial
            .agents
            .iter()
            .map(|a| a.membership_ura.as_str())
            .collect();
        assert_eq!(
            uras,
            vec!["easynet:///r/realm/device/a", "easynet:///r/realm/device/c"]
        );
    }
}

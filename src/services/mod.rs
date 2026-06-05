// EasyNet CLI — Services Layer
// =============================
//
// File: src/services/mod.rs
// Description: Long-running, non-CLI surfaces hosted by the daemon
//              process. Today that means the local `control` plane
//              that Client FFI libraries dial into and the daemon
//              Invocation transport that product paths use for Axon
//              calls, with room for future sibling services
//              (IPC-over-vsock for a future hypervisor mode, a
//              planner service, etc.) to sit without polluting
//              `facade/` or `runtime/`.
//
// Layering rule
// -------------
// `services/` sits at the same layer as `facade/` and reaches the
// runtime through hard trait/type boundaries. The exact allowlist
// is per-subtree because the three subdirs play different roles:
//
//   * `services/control/` — Control-plane wire adapter. Narrow
//     allowlist: kernel_api, invocation, invocation_target, domain,
//     ability_dispatch, gateway_api, gateway, system,
//     local_runtime_invoker, hosted_receipt. Every entry is a
//     syscall-boundary type.
//
//   * `services/invocation_transport/` — daemon-owned gRPC
//     Invocation transport.
//     Wider allowlist (adds agents, keyring, publish, execution,
//     advertise, federation_client, abilities, axon_bridge,
//     ability_wire, plugin_host) because the gRPC surface legitimately
//     translates each of those concerns. `ability_wire` is read-only
//     codec metadata for local bidi abilities; `plugin_host` is only
//     the boot-injected runtime manager for already-loaded plugin
//     abilities. Package ownership remains in `runtime/plugin_host`.
//
//   * `services/trust_anchor_key_resolver` — the adapter from the
//     daemon-owned trust-anchor cell to Axon's `KeyResolver` trait.
//     It stays in services so `runtime/axon_bridge` receives only a
//     trait object and never imports services internals.
//
// `scripts/check-kernel-boundary.sh` is the CI gate. Adding a new
// permitted import requires updating both the allowlist there AND
// the rationale comment next to it — the script's own header
// documents the convention.
//
// Why a separate top-level layer instead of a sibling under facade/
// -----------------------------------------------------------------
// `facade/cli` and `facade/mcp` are user-or-agent surfaces hit from
// *outside* the daemon process: the CLI user's terminal, a remote
// MCP client. `services/control` is an *intra-machine* surface
// hit by a Client FFI library loaded into another process on the
// same host. The trust model, transport, and lifetime are all
// different enough that keeping them in peer namespaces prevents
// accidental conflation (e.g. a CLI subcommand reaching into the
// IPC server's state, or vice versa).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

// Admission observability contract (post Phase-5a)
// -------------------------------------------------
// `services/receipt_store.rs` (the in-memory `SharedReceiptStore`
// ring buffer) was deleted in Phase 5a. The current contract:
//
//   * Successful admission of a daemon Invocation
//     emits NO standalone audit artefact. The call proceeds.
//     When (and only when) the invocation reaches a terminal
//     state, the `LedgerSink` writes one `InvocationLedger` row
//     (redb-backed, on-disk).
//
//   * Rejected admission emits an `op_event!`
//     (`component = daemon_invocation`, `kind = admission_rejected_*`)
//     to stderr and surfaces the typed `tonic::Status` to the
//     caller. Nothing is persisted server-side.
//
//   * Forward-invoke (cross-hub) outcomes are observable via the
//     two channels above plus `kind = forward_invoke_*` op_events
//     for transport-level miss/fail diagnostics.
//
// Known gap (DEC-011, scheduled Week-5+): an invocation that
// passes admission but never reaches a terminal state (handler
// hang, daemon crash mid-call) leaves NO record at all. Closing
// this gap with an admission-time ledger sentinel
// (`state = Admitted`) is tracked separately from the Phase 5a
// deletion. Until that lands, the operator log is the audit
// source for non-terminal calls.

pub mod control;

/// Daemon-owned Axon `Invocation` transport hosted by
/// `easynet-daemon`.
#[cfg(feature = "axon-pb")]
pub mod invocation_transport;

/// In-memory registry of live `<self>.session` reverse-channel
/// senders keyed by caller URI. RFC-003 PR-1 spec §3 — hub-side
/// liveness model that replaces unary heartbeats. Gated on `axon-pb`
/// because the dispatch sender type holds the proto-generated
/// `InvokeBidiDown` frame and the rejection path uses `tonic::Status`;
/// neither is available off-feature.
#[cfg(feature = "axon-pb")]
pub mod presence_registry;

/// On-disk trust set (`/etc/easynet/realm-trust.toml`) the daemon
/// admission gate consults to answer "is this caller permitted to
/// join this realm". RFC-003 PR-1 spec §5.2 — PR-1 reads, PR-7
/// authors via the device-pairing flow. Always built into the
/// library since the data structure is pure data + std crates;
/// only consumers in `invocation_transport` need the proto plumbing the
/// `axon-pb` feature gates.
pub mod realm_trust_anchor;

/// Cross-call correlation table for `<self>.invoke_remote` (PR-3
/// writer) and `<self>.session` (PR-2 completer). Outside
/// `presence_registry` because the concerns differ: presence is
/// "who is online right now"; pending_dispatch is "outstanding
/// cross-device calls awaiting reply". Pure data — no feature gate.
pub mod pending_dispatch;

/// Reload-friendly cell holding the daemon's current
/// operator-curated `tenant → hub_endpoint` map (PR-N1 commit 10/N —
/// closes the LB-37 §2.3 fallback Scope A defer note). Mirrors
/// `trust_anchor_cell` so SIGHUP-triggered `daemon-config.toml`
/// reloads republish the federated_peers map without daemon
/// restart. Pure data; no feature gate.
pub mod federated_peers_cell;

/// Per-agent ability catalog the daemon stores when devices land
/// `federation.advertise_abilities`. Read by `federation.resolve`
/// when the caller sets `include_abilities = true` so the backend's
/// `/api/v1/abilities` page projects every device's published
/// catalog. Without this store the wrapper acked + dropped, and
/// the catalog page rendered empty despite advertised data.
pub mod ability_catalog_store;

/// Hosted-agent directory rows published through
/// `federation.advertise_agent`. PresenceRegistry owns transport
/// liveness for device URIs; this store maps hosted agent URIs back
/// to their host device URI so `federation.resolve` can surface
/// `/agent/<user>.<agent>` rows while deriving online/offline from
/// the host's live `<self>.session`.
pub mod advertised_agent_store;

/// AXON-RFC-001 v4.1.7 hub-broadcast contract — caches hub-owned
/// ability descriptors that the realm hub pushes via
/// `federation.join` (full snapshot) + `federation.heartbeat`
/// (incremental diff). `meta.list_abilities scope=realm` merges
/// this cache with the device-local registry so users see both
/// device-owned and hub-owned abilities through one query.
pub mod hub_published_ability_store;

/// Per-daemon nonce replay store (RFC 001 §5.2 step 4). Wraps the
/// axon SDK's time-wheel store in `Arc<Mutex<…>>` so a single
/// instance is shared across every concurrent invoke through the
/// admission gate. PR-7 commit 4/N introduces the wrapper alongside
/// the admission upgrade. DEC-011 confirms RFC-003 ships in-memory
/// only; persistence is a Week-5+ topic.
pub mod nonce_replay_store;

/// Per-(consumer-URA, ability) usage quota counter (#185). Meters an
/// already-admitted caller against a per-window cap and surfaces the
/// result as the Axon `RateLimitInfo` contract on invoke responses.
/// In-memory tumbling-window state, peer to `nonce_replay_store`;
/// enforcement is serving-node runtime state, the wire shape stays
/// Axon's.
pub mod usage_quota_store;

/// Reload-friendly wrapper around the daemon's `RealmTrustAnchor`.
/// `<self>.register_device_pubkey` (PR-7 commit 5/N) appends an
/// entry, persists via atomic rename, then `replace`s the cell so
/// the admission gate's next read reflects the new entry. Built
/// once at boot and shared by clone between the admission facade
/// and the register-pubkey handler. DEC-010 mechanism layer.
pub mod trust_anchor_cell;
pub mod trust_anchor_key_resolver;

/// EasyNet-native device identity vault (RFC-001 plan v4.1.5
/// Phase 3A). Process-external Ed25519 keypair store sealed under
/// an Argon2id-derived AES-GCM key. Backend / daemon / CLI on the
/// same host all reach this module's `Vault` (directly in-process
/// or over the `easynet-keyring` daemon's UDS) to obtain
/// signatures without ever holding the raw seed bytes. Role-overlay
/// lookup means the same keypair signs as both `HubURI(realm)` and
/// `DeviceURI(realm, uuid)` — the host's identity is unitary across
/// roles.
pub mod keyring;

/// SelfIdentity client (RFC-001 plan v4.1.5 Phase 3B). Typed
/// `sign(self_ura, canonical_bytes) -> Signature` handle backed
/// by the `easynet-keyring` daemon's UDS or an in-process
/// `Vault`. Boot wiring picks the impl; callsites take an
/// `Arc<dyn SelfIdentity>` and never touch raw seed bytes.
pub mod self_identity;

/// Cross-hub federation transport (RFC-N PR-N1 onwards). The
/// outbound dial counterpart to PR-10 commit 1/N's inbound
/// TCP+TLS listener — when a `federation.forward_invoke`
/// targets a peer realm, this module's `FederationClient` carries
/// the call across the hub-to-hub TLS channel. PR-N1 commit 1/N
/// lands the skeleton (trait + types + channel cache);
/// commits 2-5/N wire real I/O, `handle_forward_invoke`
/// integration, and the timeout / circuit-breaker / e2e suite
/// per `pr-drafts/PR-N1-spec-hub-to-hub-grpc-outbound.md`.
#[cfg(feature = "axon-pb")]
pub mod federation_client;

/// Cross-realm directory federation wire shapes (RFC-N PR-N3).
/// PR-N3 commit N3-1 lands `DirectoryEntry` with schema-B
/// `origin_realm` / `hub_endpoint` / `last_seen_unix_ms` fields
/// per `pr-drafts/PR-N3-spec-cross-realm-directory-v2.md §2.1`.
/// `DirectoryEvent` (the event-stream tagged enum) lands in
/// N3-2; the `RemoteDirectoryClient` per-peer FSM and
/// `SharedFederatedDirectoryView` cell land in N3-3. Pure data
/// + serde, but the current file also hosts the streaming
/// supervisor path that depends on proto-generated request types.
/// Keep it behind `axon-pb` until the pure-data portion is split
/// out.
#[cfg(feature = "axon-pb")]
pub mod federation_directory;

// `axon_bridge` moved to `crate::runtime::axon_bridge` per the
// 2026-05-29 industrial-textbook review: its imports go almost
// entirely to `crate::runtime::*` (`ability_dispatch`, `agents::*`,
// `invocation_target`); it carries no `services/`-layer
// concern of its own. Keeping a re-export here would only
// reintroduce the false hierarchy. Search by name (`axon_bridge`)
// or by symbol — every public type is unchanged.

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    #[test]
    fn quota_meters_user_abilities_but_exempts_control_plane() {
        let meters =
            crate::services::invocation_transport::daemon_invocation_service::quota_meters_function;

        assert!(meters("device.observe.health"));
        assert!(meters("agent.todo.run"));

        assert!(!meters("federation.heartbeat"));
        assert!(!meters("federation.forward_invoke"));
        assert!(!meters("<self>.register_device_pubkey"));
        assert!(!meters("<self>.session"));
    }
}

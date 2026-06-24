// EasyNet CLI — invocation_transport — transport policy gate facade
// ==================================================
//
// File: src/services/invocation_transport/admission_facade.rs
// Description: Per-RPC transport/product policy gate the dispatcher
//              consults before routing into a federation wrapper or
//              any ability handler.
//
// Boundary note
// -------------
// This facade is not an Axon LocalRuntime "already admitted" boundary.
// A successful `verify_*` result only means the daemon transport accepted
// the caller for routing, quota, delegation, and wrapper compatibility.
// Any request that enters Axon `LocalRuntime` must still be reconstructed
// as a descriptor-bound public request and admitted by Axon through
// `DescriptorBoundInvocationRequest::{externally_signed,signed}`.
//
// What this module does
// ---------------------
// 1. Reads the `Envelope` from an inbound `pb::axon::v1::InvokeRequest`
//    (or its server-stream / bidi counterpart)
// 2. **Loopback bypass**: callers presenting the daemon's own URA
//    are accepted without crypto on trusted local transports
// 3. **Trust-anchor membership** (always): unknown caller URAs are
//    rejected with `permission_denied` before any structural work,
//    so unrelated callers cannot push entries into the replay store
// 4. **Strict signed admission by `TrustedAgent.role`**:
//      - **Device / Backend / Hub / User** → strict 4-step §5.2 pipeline
//          a. `validate_envelope`            (RFC 001 §5.2 step 1)
//          b. `validate_signature_structure` (RFC 001 §5.2 step 2)
//          c. `verify_signature` against the trust-anchor-backed
//             `KeyResolver`                  (RFC 001 §5.2 step 3)
//          d. `NonceReplayStore::check_and_record` against the
//             daemon-shared store            (RFC 001 §5.2 step 4)
// 5. Returns `Ok(())` for accept and a `tonic::Status` for reject —
//    the only outcomes the dispatcher needs

// Why strict-everywhere for public callers
// ----------------------------------------
// The clean architecture has no unsigned Device compatibility arm.
// Devices, backends, hubs, and users all enter
// through signed Axon envelopes. The daemon-local loopback bypass is
// reserved for the daemon's own URA on trusted local transports; it is
// not a public caller compatibility mechanism.
//
// What this module does NOT do (yet)
// ----------------------------------
// **Receipt emission** — RFC 001 §5.3 admission-emits-receipt. Per
// DEC-012, receipt minting is deferred to PR-10 (production canary)
// where the receipt store and the signing key are wired together.
// PR-7 (this commit) intentionally leaves admission as a yes/no gate
// — receipts on the InvokeResponse remain `None` for now.
//
// Invariants
// ----------
// **Invariant 1 (caller URA required)**: Every inbound RPC must
// carry an `Envelope` with a non-empty `caller.ura`. The dispatcher
// receives `Status::invalid_argument` for any RPC missing this; it
// is a wire-level requirement, not a policy choice.
//
// **Invariant 2 (loopback bypass)**: When the caller URA matches
// the daemon's configured URA, admission accepts without consulting
// the trust anchor or the replay store. The daemon trusts itself —
// `<self>.*` abilities and admin RPCs originate from the daemon's
// own process and need not sign.
//
// **Invariant 3 (strict public crypto)**: Every external caller role
// (`Device`, `Backend`, `Hub`, and `User`) runs the full §5.2 pipeline
// end-to-end: a missing/malformed `caller_signature` rejects with
// `CALLER_SIGNATURE_INVALID`; a signature that fails to verify against
// the trust anchor's public-key entry rejects with the same reason; a
// nonce already observed inside the dedup window rejects with
// `CALLER_NONCE_REPLAYED`.
//
// **Invariant 4 (replay store mutation discipline)**: The replay
// store is mutated only after `validate_envelope` and
// `validate_signature_structure` both pass — malformed callers
// can never pollute the store. This is a property of
// `easynet_axon::invocation::admission::run_descriptor_bound_admission`, which
// orders the four steps so structure failures short-circuit before
// the nonce hits the map.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::{collections::HashMap, sync::Arc};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use tonic::Status;

use easynet_axon::invocation::admission::{
    now_ms as axon_now_ms, run_descriptor_bound_admission, REASON_CALLER_SIGNATURE_INVALID,
    REASON_ENVELOPE_INCOMPLETE, REASON_NONCE_REPLAY,
};
use easynet_axon::invocation::axiom::{CallerSignature as AxiomCallerSignature, KeyResolver};
use easynet_axon::invocation::{
    AxonError as InvocationError, AxonErrorKind as InvocationErrorKind,
};

use crate::runtime::ability::canonical_json_bytes;
use crate::runtime::ability::HOSTED_AGENT_DELEGATION_METADATA_KEY;
use crate::runtime::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireCallerIdentity,
};
use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::invocation_transport::federated_key_resolver::{
    FederatedKeyResolver, SharedFederatedKeyCache,
};
use crate::services::invocation_transport::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_AGENT, ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
};
use crate::services::invocation_transport::list_user_pubkeys::ABILITY_SELF_LIST_USER_PUBKEYS;
use crate::services::invocation_transport::register_device_pubkey::ABILITY_SELF_REGISTER_DEVICE_PUBKEY;
use crate::services::invocation_transport::revoke_user_pubkey::ABILITY_SELF_REVOKE_USER_PUBKEY;
use crate::services::invocation_transport::session_initiator::SessionSigningSeed;
use crate::services::nonce_replay_store::SharedNonceReplayStore;
use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgentRole};
use crate::services::trust_anchor_cell::SharedTrustAnchor;
use crate::services::usage_quota_store::{QuotaDenyReason, SharedUsageQuotaGate};
use easynet_axon::pb::axon::v1::{
    Envelope, EnvelopeOpen, InvokeRequest, InvokeServerStreamRequest, RateLimitInfo,
};

const DELEGATION_METADATA_KEY: &str = "x-easynet-delegation";
const SESSION_AUTHORITY_METADATA_KEY: &str = "x-easynet-session-authority";
const REASON_AUTHORITY_REQUIRED: &str = "AUTHORITY_REQUIRED";
const REASON_AUTHORITY_FORMAT_INVALID: &str = "AUTHORITY_FORMAT_INVALID";
const REASON_AUTHORITY_SIGNATURE_INVALID: &str = "AUTHORITY_SIGNATURE_INVALID";
const REASON_AUTHORITY_CALLER_MISMATCH: &str = "AUTHORITY_CALLER_MISMATCH";
const REASON_AUTHORITY_SUBJECT_MISMATCH: &str = "AUTHORITY_SUBJECT_MISMATCH";
const REASON_AUTHORITY_AUDIENCE_VIOLATION: &str = "AUTHORITY_AUDIENCE_VIOLATION";
const REASON_AUTHORITY_SCOPE_VIOLATION: &str = "AUTHORITY_SCOPE_VIOLATION";
const REASON_AUTHORITY_EXPIRED: &str = "AUTHORITY_EXPIRED";
const REASON_AUTHORITY_ISSUER_UNKNOWN: &str = "AUTHORITY_ISSUER_UNKNOWN";
const REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND: &str = "AUTHORITY_ISSUER_KEY_NOT_FOUND";
const REASON_HOSTED_AGENT_DELEGATION_LOCAL_ONLY: &str = "HOSTED_AGENT_DELEGATION_LOCAL_ONLY";
const REASON_CALLER_UNKNOWN: &str = "CALLER_UNKNOWN";

/// Per-RPC transport/product policy gate consulted by
/// `DaemonInvocationService` before routing into a federation wrapper or
/// fallthrough handler.
///
/// Holds:
/// - `Arc<RealmTrustAnchor>` — the trust set authored by PR-7's
///   pairing flow and read at boot by the daemon binary
/// - `daemon_ura` — the daemon's own canonical URA (loopback bypass)
/// - `replay_store` — the daemon-shared `SharedNonceReplayStore` used by
///   legacy transport-wrapper strict policy checks. It is not a token that
///   lets callers bypass Axon's LocalRuntime admission.
///
/// Constructed once per daemon process; cloned into per-request
/// dispatcher tasks (clone is cheap — all fields are `Arc` or
/// `Option<String>`).
#[derive(Clone)]
pub struct AdmissionFacade {
    trust_anchor: SharedTrustAnchor,
    daemon_ura: Option<String>,
    replay_store: SharedNonceReplayStore,
    /// **PR-N2 commit 1/N**. Cross-hub federation client used by
    /// `FederatedKeyResolver` to dial a peer hub's
    /// `federation.resolve_key` ability when the local trust
    /// anchor has no entry for a cross-realm caller URA. `None`
    /// in single-realm/test builds — the resolver collapses to
    /// local-only behavior in that case (mirrors PR-7's
    /// `TrustAnchorKeyResolver`).
    federation_client: Option<Arc<dyn FederationClient>>,
    /// **PR-N2 commit 1/N**. Snapshot cell for the operator-
    /// curated `[daemon.federated_peers]` table. Threaded into
    /// `FederatedKeyResolver` so a SIGHUP-driven reload (PR-N1
    /// commit 10/N) is visible to the next admission. Defaults
    /// to an empty cell when no federation client is wired.
    federated_peers: SharedFederatedPeers,
    /// **PR-N2 commit 1/N**. The local realm string used for
    /// the same-realm-vs-cross-realm decision in
    /// `FederatedKeyResolver`. Derived from `daemon_ura` when
    /// not supplied directly. `None` in test builds with no
    /// daemon URA wired.
    self_realm: Option<String>,
    /// **C3b** TTL cache shared across every per-admission
    /// `FederatedKeyResolver` instance. Without this share, the
    /// per-call resolver would build a fresh empty cache and
    /// the TTL would deliver zero savings. Boot-time SIGHUP
    /// handler holds a clone too so a trust-anchor reload can
    /// flush all cached cross-realm pubkeys atomically.
    federated_key_cache: SharedFederatedKeyCache,
    /// Hub identity seed used only for outbound cross-hub
    /// `federation.resolve_key` calls emitted by FederatedKeyResolver.
    /// `None` keeps local-only/test facades fail-closed instead of
    /// fabricating a deterministic hub identity.
    hub_signing_seed: Option<SessionSigningSeed>,
    /// Whether the loopback bypass (Invariant 2) is honoured for
    /// this facade. The bypass is a pure URA string-match
    /// (`caller_ura == daemon_ura`), so any caller that can reach
    /// the listener and spoof the daemon's own URA would otherwise
    /// skip the trust anchor, signature, and replay checks. That is
    /// only safe on a genuinely loopback-only transport: the daemon
    /// serves the *same* `InvocationServer` over both a 0600 UDS and
    /// a TCP+TLS socket (see `boot::spawn_tcp_tls_listener`), and the
    /// TCP socket is off-box reachable. So the UDS-fed facade keeps
    /// `loopback_trusted = true` and the TCP-fed facade sets it to
    /// `false`, forcing every TCP caller — including a daemon-URA
    /// spoofer — through the full strict pipeline. Defaults to `true`
    /// so existing single-listener / test wiring is unchanged.
    loopback_trusted: bool,
    /// #185: reloadable per-consumer usage-quota gate. The gate is
    /// always present so SIGHUP can enable quota after boot; it is
    /// disabled internally when `[daemon.quota]` is absent. Loopback
    /// self calls remain exempt here because the daemon must not
    /// throttle its own `<self>.*` administrative surface.
    quota: SharedUsageQuotaGate,
}

impl std::fmt::Debug for AdmissionFacade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionFacade")
            .field("trust_anchor", &self.trust_anchor)
            .field("daemon_ura", &self.daemon_ura)
            .field("replay_store", &self.replay_store)
            .field(
                "federation_client",
                &self
                    .federation_client
                    .as_ref()
                    .map(|_| "<dyn FederationClient>"),
            )
            .field("federated_peers", &self.federated_peers)
            .field("self_realm", &self.self_realm)
            .field(
                "hub_signing_seed",
                &self.hub_signing_seed.as_ref().map(|_| "<seed>"),
            )
            .field("loopback_trusted", &self.loopback_trusted)
            .field("quota_configured", &self.quota.policy().is_some())
            .finish()
    }
}

impl AdmissionFacade {
    /// Construct a facade against the supplied trust anchor and
    /// daemon URA. Production callers thread the daemon's
    /// `credentials.json`-derived URA through; tests typically pass
    /// `None`.
    ///
    /// The trust anchor is wrapped in a fresh `SharedTrustAnchor`
    /// cell — every `verify_*` call snapshots the current anchor,
    /// so a future writer (`<self>.register_device_pubkey`,
    /// PR-7 commit 5/N) that holds a clone of the cell can publish
    /// updates without restarting the facade. Callers that already
    /// hold a `SharedTrustAnchor` and need to share it with the
    /// register handler should use `with_trust_anchor_cell` instead.
    ///
    /// The replay store is created fresh per facade — production
    /// builds one facade per daemon process, so this is also one
    /// store per daemon process (RFC 001 §5.2 step 4 invariant: one
    /// shared dedup window across the daemon's lifetime).
    #[must_use]
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>, daemon_ura: Option<String>) -> Self {
        Self::with_trust_anchor_cell(SharedTrustAnchor::new(trust_anchor), daemon_ura)
    }

    /// Construct a facade against a shared trust-anchor cell. Used
    /// by `start_daemon_invocation_transport` so the same cell is shared
    /// with the `<self>.register_device_pubkey` handler — a
    /// successful register publishes the new anchor and the next
    /// admission snapshot reflects it without daemon restart.
    #[must_use]
    pub fn with_trust_anchor_cell(
        trust_anchor: SharedTrustAnchor,
        daemon_ura: Option<String>,
    ) -> Self {
        let self_realm = daemon_ura.as_deref().and_then(
            crate::services::invocation_transport::register_device_pubkey::parse_realm_from_ura,
        );
        Self {
            trust_anchor,
            daemon_ura,
            replay_store: SharedNonceReplayStore::new(),
            federation_client: None,
            federated_peers: SharedFederatedPeers::new(std::collections::BTreeMap::new()),
            self_realm,
            federated_key_cache: SharedFederatedKeyCache::new(),
            hub_signing_seed: None,
            loopback_trusted: true,
            quota: SharedUsageQuotaGate::disabled(),
        }
    }

    /// Snapshot the SharedTrustAnchor cell. PR-N2 commit 2/N's
    /// `federation.resolve_key` handler consults this at dispatch
    /// time so a SIGHUP-driven `realm-trust.toml` reload (PR-7
    /// commit 5/N) is visible without restart. Returns the
    /// current `Arc<RealmTrustAnchor>`; callers pass it directly
    /// to `federation_wrappers::handle_resolve_key`.
    #[must_use]
    pub fn trust_anchor_snapshot(&self) -> Arc<RealmTrustAnchor> {
        self.trust_anchor.snapshot()
    }

    /// Verify delegation metadata against an already-constructed
    /// envelope without re-running caller signature / nonce policy checks.
    ///
    /// Used by `<self>.invoke_remote` for the inner ability request:
    /// the outer invoke_remote frame has already passed strict
    /// admission, and the inner JSON carries the user/resource
    /// subject plus non-AXIOM metadata. This method verifies only the
    /// authority proof binding `(caller, subject, callee, ability)`.
    pub fn verify_delegation_for_envelope(
        &self,
        envelope: &Envelope,
        ability: &str,
        metadata: &HashMap<String, String>,
    ) -> Result<(), Status> {
        reject_public_hosted_agent_delegation_metadata(Some(metadata))?;
        verify_delegation_metadata(
            envelope,
            ability,
            Some(metadata),
            self.trust_anchor_snapshot().as_ref(),
            axon_now_ms(),
        )
    }

    /// The daemon's own canonical URA. Used by per-ability
    /// admission filters that need to recognise the loopback
    /// caller (eg. `federation.list_user_devices` in N3-5
    /// admits the daemon talking to itself without requiring
    /// a Hub trust entry for its own URA).
    #[must_use]
    pub fn daemon_ura(&self) -> Option<&str> {
        self.daemon_ura.as_deref()
    }

    /// Set whether this facade honours the loopback bypass
    /// (Invariant 2). Boot wires the UDS-fed service with `true`
    /// (the daemon's own process reaches itself over the 0600 socket
    /// and need not sign) and the TCP+TLS-fed service with `false`,
    /// so an off-box caller that spoofs the daemon URA cannot skip
    /// the strict trust-anchor / signature / replay pipeline.
    #[must_use]
    pub fn with_loopback_trusted(mut self, loopback_trusted: bool) -> Self {
        self.loopback_trusted = loopback_trusted;
        self
    }

    /// #185: attach the reloadable per-consumer usage-quota gate.
    /// Boot wires the same gate into the SIGHUP reload coordinator so
    /// `[daemon.quota]` edits can affect the next admission without a
    /// daemon restart.
    #[must_use]
    pub fn with_quota_gate(mut self, gate: SharedUsageQuotaGate) -> Self {
        self.quota = gate;
        self
    }

    /// #185: meter a caller that already passed this transport policy
    /// gate. MUST be called only after `verify_invoke` has returned `Ok`
    /// for this request.
    ///
    /// Returns:
    /// - `Ok(None)` — no metering applies (quota off, the caller is
    ///   the daemon's own loopback/self URA, or the caller is
    ///   unmetered by policy). The response carries no `RateLimitInfo`.
    /// - `Ok(Some(info))` — the call is within budget; `info` is the
    ///   post-decrement quota status to surface on the response.
    /// - `Err(status)` — the window budget is exhausted, the bounded
    ///   store is saturated, or the key material violates the quota
    ///   key-size contract. Exhaustion/saturation use
    ///   `ResourceExhausted`; key contract violations use
    ///   `InvalidArgument`.
    pub fn check_quota(&self, request: &InvokeRequest) -> Result<Option<RateLimitInfo>, Status> {
        self.check_quota_for_ability(request, &request.function_name)
    }

    /// #185: meter a transport-policy-accepted unary caller against an
    /// explicit ability name. `federation.forward_invoke` uses this to
    /// charge the caller for the inner user ability while keeping the
    /// top-level federation wrapper itself exempt as control-plane traffic.
    pub fn check_quota_for_ability(
        &self,
        request: &InvokeRequest,
        ability: &str,
    ) -> Result<Option<RateLimitInfo>, Status> {
        let Some(envelope) = request.envelope.as_ref() else {
            return Ok(None);
        };
        let caller_ura = caller_ura_required(envelope)?;

        // The daemon never meters itself: loopback/self calls
        // (`<self>.*` abilities, admin RPCs) bypass quota exactly as
        // they bypass the trust anchor.
        if self.is_loopback(caller_ura) {
            return Ok(None);
        }

        let Some(decision) = self
            .quota
            .check_and_record(caller_ura, ability, axon_now_ms())
        else {
            return Ok(None);
        };

        if decision.allowed {
            let info = RateLimitInfo {
                quota_remaining: decision.quota_remaining,
                quota_limit: decision.quota_limit,
                reset_at_unix_ms: decision.reset_at_unix_ms,
                retry_after_ms: decision.retry_after_ms,
            };
            return Ok(Some(info));
        }
        match decision.deny_reason {
            Some(QuotaDenyReason::KeyTooLarge) => Err(Status::invalid_argument(format!(
                "REQUEST_METADATA_INVALID: quota key too large caller={caller_ura} ability={ability}"
            ))),
            Some(QuotaDenyReason::StoreSaturated) => Err(Status::resource_exhausted(format!(
                "RESOURCE_EXHAUSTED: quota store saturated caller={caller_ura} ability={ability} retry_after_ms={}",
                decision.retry_after_ms
            ))),
            Some(QuotaDenyReason::BudgetExhausted) | None => {
                Err(Status::resource_exhausted(format!(
                    "QUOTA_EXCEEDED: caller={caller_ura} ability={ability} retry_after_ms={}",
                    decision.retry_after_ms
                )))
            }
        }
    }

    /// Snapshot the shared federated-key cache. Boot-time SIGHUP
    /// handler clones this and calls `.flush()` after a
    /// trust-anchor reload so a key rotation propagates without
    /// waiting for the per-entry TTL to elapse.
    #[must_use]
    pub fn federated_key_cache(&self) -> SharedFederatedKeyCache {
        self.federated_key_cache.clone()
    }

    /// Construct a facade with a caller-supplied replay store. Used
    /// by tests that need to drive multiple facades against a single
    /// shared store, and reserved for the eventual PR-10 work that
    /// might split admission across listeners but keep one daemon-
    /// wide replay window.
    #[must_use]
    pub fn with_replay_store(
        trust_anchor: Arc<RealmTrustAnchor>,
        daemon_ura: Option<String>,
        replay_store: SharedNonceReplayStore,
    ) -> Self {
        let self_realm = daemon_ura.as_deref().and_then(
            crate::services::invocation_transport::register_device_pubkey::parse_realm_from_ura,
        );
        Self {
            trust_anchor: SharedTrustAnchor::new(trust_anchor),
            daemon_ura,
            replay_store,
            federation_client: None,
            federated_peers: SharedFederatedPeers::new(std::collections::BTreeMap::new()),
            self_realm,
            federated_key_cache: SharedFederatedKeyCache::new(),
            hub_signing_seed: None,
            loopback_trusted: true,
            quota: SharedUsageQuotaGate::disabled(),
        }
    }

    /// **PR-N2 commit 1/N**. Builder seam: wire the cross-hub
    /// federation client + operator-curated federated_peers
    /// cell. When set, the strict admission path constructs a
    /// `FederatedKeyResolver` instead of `TrustAnchorKeyResolver`,
    /// which means a cross-realm caller whose URA is missing
    /// from the local trust anchor falls through to a
    /// `federation.resolve_key` ability call against the peer
    /// hub mapped by `federated_peers[caller_realm]`.
    ///
    /// Production daemons call this in
    /// `start_daemon_invocation_transport` after wiring the dialer; test
    /// / smoke setups omit it and behave as PR-7
    /// `TrustAnchorKeyResolver` did (local-only).
    #[must_use]
    pub fn with_federation(
        mut self,
        federation_client: Arc<dyn FederationClient>,
        federated_peers: SharedFederatedPeers,
    ) -> Self {
        self.federation_client = Some(federation_client);
        self.federated_peers = federated_peers;
        self
    }

    /// Attach the local hub identity seed used when admission has to
    /// ask a peer hub for a cross-realm caller key. The key-resolver
    /// path is part of strict admission, so the resolve-key request
    /// itself must be a signed hub-to-hub invocation.
    #[must_use]
    pub fn with_hub_signing_seed(mut self, seed: SessionSigningSeed) -> Self {
        self.hub_signing_seed = Some(seed);
        self
    }

    /// Verify a unary `InvokeRequest`. Returns `Ok(())` when the
    /// caller passes this daemon transport policy gate; otherwise a
    /// `tonic::Status` mapped per the rule set in
    /// `run_transport_policy_gate`.
    pub fn verify_invoke(&self, request: &InvokeRequest) -> Result<(), Status> {
        let envelope = request
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Invoke request missing envelope"))?;
        self.run_transport_policy_gate(
            envelope,
            &request.function_name,
            &request.arguments,
            Some(&request.metadata),
        )
    }

    /// Verify a server-stream `InvokeServerStreamRequest`. Same transport
    /// policy rule set as `verify_invoke`; the differing wrapper is just
    /// the proto type.
    pub fn verify_invoke_stream(&self, request: &InvokeServerStreamRequest) -> Result<(), Status> {
        let envelope = request
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("InvokeStream request missing envelope"))?;
        self.run_transport_policy_gate(
            envelope,
            &request.function_name,
            &request.arguments,
            Some(&request.metadata),
        )
    }

    /// Verify the frame-0 `EnvelopeOpen` of an InvokeBidi stream.
    /// The bidi path's "ability" is `target.ability_name`, and the
    /// "args" feed for `args_digest` is `initial_args`. If either
    /// is missing the gate rejects with `invalid_argument`.
    pub fn verify_envelope_for_bidi(&self, open: &EnvelopeOpen) -> Result<(), Status> {
        let envelope = open
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("InvokeBidi frame 0 missing envelope"))?;
        let ability = open
            .target
            .as_ref()
            .map(|t| t.ability_name.as_str())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "InvokeBidi frame 0 missing target.ability_name; cannot dispatch",
                )
            })?;
        self.run_transport_policy_gate(envelope, ability, &open.initial_args, Some(&open.metadata))
    }

    // ── Internal pipeline ────────────────────────────────────────

    /// Strict public transport policy gate.
    ///
    /// Order:
    /// 1. Caller URA required (Invariant 1).
    /// 2. Loopback bypass (Invariant 2).
    /// 3. Trust-anchor membership: unknown URA → `permission_denied`.
    /// 4. Device, Backend, Hub, and User roles all run the strict
    ///    4-step §5.2 signature/replay pipeline.
    fn run_transport_policy_gate(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
        metadata: Option<&HashMap<String, String>>,
    ) -> Result<(), Status> {
        let caller_ura = caller_ura_required(envelope)?;

        // Invariant 2: loopback bypass. Daemon trusts itself.
        if self.is_loopback(caller_ura) {
            return Ok(());
        }

        // Snapshot the trust anchor once per RPC. The snapshot is a
        // cheap `Arc` clone; concurrent writers to the cell don't
        // disturb the view we hold for the lookup + downstream
        // resolver use. This is the structural reason the cell
        // exists: a register-pubkey call mid-RPC can't make the
        // current admission see a half-applied state.
        let snapshot = self.trust_anchor.snapshot();

        // Trust-anchor membership precedes any structural check.
        // Unknown callers reject with `permission_denied` — the
        // DEC-013 entry contract says "URA-not-in-trust-set" surfaces
        // before any attempt at envelope/signature parsing, so an
        // unrelated caller cannot waste structure-validation cycles
        // and never has its (possibly malformed) nonce considered.
        //
        // **PR-N2 commit 1/N — cross-realm extension**. When the
        // caller is in a *federated* realm (its realm is mapped in
        // `federated_peers` AND differs from `self_realm`), the
        // local trust anchor will NOT have an entry: the caller's
        // identity is gated by the peer hub, not by us. In that
        // case we route to the strict admission path with the
        // FederatedKeyResolver, which dials the peer's
        // `federation.resolve_key` to fetch the verifying key and
        // runs the same RFC 001 §5.2 4-step verify. This preserves
        // INV-1 (federated trust gate is *operator-explicit*: only
        // realms the operator listed in `federated_peers` can
        // bypass the local-membership reject) while opening the
        // cross-realm signed-admission door.
        let trusted = match snapshot.lookup(caller_ura) {
            Some(entry) => entry,
            None => {
                if self.is_federated_caller(caller_ura) {
                    reject_public_hosted_agent_delegation_metadata(metadata)?;
                    self.run_strict_signature_gate(envelope, ability, args, metadata, snapshot)?;
                    return Ok(());
                }
                return Err(permission_denied_unknown_caller(caller_ura));
            }
        };

        match trusted.role {
            // Public callers all run strict 4-step admission. The
            // strict path resolves the caller's public key against
            // the same snapshot we just consulted for membership —
            // keeping "membership hit" and "key resolved" tied to
            // the same trust-anchor version.
            TrustedAgentRole::Device
            | TrustedAgentRole::Backend
            | TrustedAgentRole::Hub
            | TrustedAgentRole::User => {
                reject_public_hosted_agent_delegation_metadata(metadata)?;
                self.run_strict_signature_gate(envelope, ability, args, metadata, snapshot)?;
                Ok(())
            }
        }
    }

    /// Strict §5.2 signature/replay policy for public caller roles.
    /// Bridges proto → descriptor-bound Axon domain types and dispatches
    /// into `easynet_axon::invocation::admission::run_descriptor_bound_admission`
    /// with a snapshot-backed `KeyResolver` and the daemon-shared replay
    /// store. This must verify the same canonical bytes later consumed by
    /// LocalRuntime; the daemon does not maintain a second signature dialect.
    fn run_strict_signature_gate(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
        metadata: Option<&HashMap<String, String>>,
        trust_anchor: Arc<RealmTrustAnchor>,
    ) -> Result<(), Status> {
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "envelope present but ability/function_name is empty; cannot run admission",
            ));
        }

        let descriptor_bound = descriptor_bound_from_wire_parts(
            envelope.clone(),
            ability.to_string(),
            args,
            WireCallerIdentity::FromEnvelope,
        )
        .map_err(axon_error_to_status)?;
        let axiom_signature = build_axiom_signature(envelope.caller_signature.as_ref())
            .map_err(axon_error_to_status)?;

        // DEC-EU §multi-device. User URAs are 1:N, so the resolver must
        // key local and cross-realm lookup on the envelope-presented
        // public key. FederatedKeyResolver now owns both paths: same-realm
        // user calls pin against `lookup_user_by_pubkey`; cross-realm user
        // calls forward `presented_pubkey_b64` to the peer hub's
        // `federation.resolve_key` handler. Non-user callers keep the
        // existing one-key local-first behavior.
        let mut federated_resolver = FederatedKeyResolver::new(
            Arc::clone(&trust_anchor),
            self.federation_client.clone(),
            self.federated_peers.snapshot(),
            self.self_realm.clone(),
        )
        .with_cache(self.federated_key_cache.clone())
        .with_hub_signing_seed(self.hub_signing_seed.as_ref());
        if envelope_caller_is_user(envelope) {
            federated_resolver = federated_resolver
                .with_presented_pubkey_b64(envelope_presented_pubkey_b64(envelope));
        }
        let resolver: Box<dyn KeyResolver> = Box::new(federated_resolver);

        let result = self.replay_store.with_inner(|store| {
            run_descriptor_bound_admission(
                &descriptor_bound.envelope,
                &axiom_signature,
                Some(resolver.as_ref()),
                store,
                axon_now_ms(),
            )
        });

        match result {
            // Phase 5a: SharedReceiptStore is gone. Successful
            // admission no longer drops a synthetic
            // `InvocationReceipt` into an in-memory ring buffer;
            // operators observe accepted invocations via the
            // `LedgerSink`-installed `InvocationLedger` at
            // terminal time, and rejected invocations via
            // `op_event!` audit lines + the wire-level error
            // their gRPC client sees.
            Ok(()) if bootstrap_authority_ability(ability) => Ok(()),
            Ok(()) => verify_delegation_metadata(
                envelope,
                ability,
                metadata,
                trust_anchor.as_ref(),
                axon_now_ms(),
            ),
            Err(err) => Err(axon_error_to_status(err)),
        }
    }

    fn is_loopback(&self, caller_ura: &str) -> bool {
        // Off-box transports never get the bypass, even on an exact
        // daemon-URA match: the same URA an attacker can put in
        // `caller.ura` would otherwise skip the entire strict
        // pipeline. Only the loopback-only (UDS) listener wires a
        // facade with `loopback_trusted = true`.
        if !self.loopback_trusted {
            return false;
        }
        if caller_ura == crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA {
            return true;
        }
        match self.daemon_ura.as_deref() {
            Some(self_ura) => caller_ura == self_ura,
            None => false,
        }
    }

    /// **PR-N2 commit 1/N**. Decide whether `caller_ura` belongs to
    /// a federated peer realm — i.e. a realm the operator has
    /// explicitly opted into by adding a `[daemon.federated_peers]`
    /// map entry mapping `realm → hub_endpoint`.
    ///
    /// Returns `true` iff:
    ///   - the URA parses to a non-self realm
    ///   - the federated_peers cell holds an entry for that realm
    ///   - a federation client is wired (without one, the strict
    ///     path's FederatedKeyResolver has no way to dial the peer
    ///     and would just fail closed — short-circuit here)
    fn is_federated_caller(&self, caller_ura: &str) -> bool {
        let Some(client) = self.federation_client.as_ref() else {
            return false;
        };
        let _ = client; // presence-only check; resolver does the dial
        let Some(caller_realm) = parse_realm_from_ura(caller_ura) else {
            return false;
        };
        if let Some(self_realm) = self.self_realm.as_deref() {
            if caller_realm == self_realm {
                return false;
            }
        }
        let peers = self.federated_peers.snapshot();
        peers.contains_key(&caller_realm)
    }
}

/// **PR-N2 commit 1/N**. Parse the realm component from a canonical
/// EasyNet URA (`easynet:///r/<realm>/...`). Returns the realm slice
/// when the shape matches, `None` otherwise. Shared by
/// `is_federated_caller` and the cross-realm gate.
///
/// Important: federated callers are not uniformly `.../agent/...`;
/// peer hubs use Axon's canonical hub identity shape and device sessions
/// register under `.../device/<id>`. Reuse the same realm parser as
/// `<self>.register_device_pubkey` so all canonical role tails stay
/// accepted and retired aliases stay rejected.
fn parse_realm_from_ura(ura: &str) -> Option<String> {
    crate::services::invocation_transport::register_device_pubkey::parse_realm_from_ura(ura)
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Bootstrap authority abilities mutate identity or presence roots.
/// They still require the caller to pass strict admission above; this
/// gate only keeps trust-anchor bootstrap out of normal user-delegation
/// semantics so stale backend issuer keys cannot deadlock key repair.
fn bootstrap_authority_ability(ability: &str) -> bool {
    matches!(
        ability,
        ABILITY_SELF_REGISTER_DEVICE_PUBKEY
            | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
            | ABILITY_FEDERATION_ADVERTISE_AGENT
            | ABILITY_SELF_LIST_USER_PUBKEYS
            | ABILITY_SELF_REVOKE_USER_PUBKEY
    )
}

#[derive(Debug, Deserialize)]
struct DelegationProofRaw {
    #[serde(default)]
    payload: serde_json::Value,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DelegationPayload {
    issuer_ura: String,
    subject_ura: String,
    caller_ura: String,
    audience: String,
    scopes: Vec<String>,
    issued_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SessionAuthorityRaw {
    #[serde(default)]
    payload: serde_json::Value,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct SessionAuthorityPayload {
    backend_ura: String,
    user_ura: String,
    session_id: String,
    scopes: Vec<String>,
    audiences: Vec<String>,
    issued_at_ms: i64,
    expires_at_ms: i64,
}

fn canonical_authority_payload_bytes<T: Serialize>(payload: &T) -> Result<Vec<u8>, Status> {
    let value = serde_json::to_value(payload).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority payload canonical value marshal failed: {err}"
        ))
    })?;
    Ok(canonical_json_bytes(&value))
}

fn verify_delegation_metadata(
    envelope: &Envelope,
    ability: &str,
    metadata: Option<&HashMap<String, String>>,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<(), Status> {
    let raw_delegation = metadata.and_then(|m| {
        m.get(DELEGATION_METADATA_KEY)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
    });
    let raw_session = metadata.and_then(|m| {
        m.get(SESSION_AUTHORITY_METADATA_KEY)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
    });

    match (raw_delegation, raw_session) {
        (Some(_), Some(_)) => {
            Err(Status::invalid_argument(format!(
                "{REASON_AUTHORITY_FORMAT_INVALID}: invocation carries both `{DELEGATION_METADATA_KEY}` \
                 and `{SESSION_AUTHORITY_METADATA_KEY}`"
            )))
        }
        (Some(raw_proof), None) => {
            let payload = parse_and_verify_delegation_proof(raw_proof, trust_anchor, now_ms)?;
            verify_delegation_bindings(&payload, envelope, ability)
        }
        (None, Some(raw_session)) => {
            let payload = parse_and_verify_session_authority(raw_session, trust_anchor, now_ms)?;
            verify_session_authority_bindings(&payload, envelope, ability)
        }
        (None, None) => {
            if envelope_requires_authority(envelope) {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_REQUIRED}: envelope subject differs from caller and is a user/session authority subject; \
                     missing `{DELEGATION_METADATA_KEY}` or `{SESSION_AUTHORITY_METADATA_KEY}` metadata"
                )));
            }
            Ok(())
        }
    }
}

fn parse_and_verify_session_authority(
    raw_authority: &str,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<SessionAuthorityPayload, Status> {
    let wire = BASE64_STANDARD.decode(raw_authority).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority base64 decode failed: {err}"
        ))
    })?;

    let raw: SessionAuthorityRaw = serde_json::from_slice(&wire).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: raw session authority JSON parse failed: {err}"
        ))
    })?;

    let payload: SessionAuthorityPayload = serde_json::from_value(raw.payload).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority payload parse failed: {err}"
        ))
    })?;
    validate_session_authority_payload_shape(&payload, now_ms)?;

    let payload_bytes = canonical_authority_payload_bytes(&payload)?;
    let signature = BASE64_STANDARD.decode(&raw.signature).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority signature base64 decode failed: {err}"
        ))
    })?;

    let backend = trust_anchor.lookup(&payload.backend_ura).ok_or_else(|| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_UNKNOWN}: session authority backend `{}` is not in the realm \
             trust anchor",
            payload.backend_ura
        ))
    })?;
    verify_delegation_signature(&backend.public_key_b64, &payload_bytes, &signature)?;

    Ok(payload)
}

fn validate_session_authority_payload_shape(
    payload: &SessionAuthorityPayload,
    now_ms: i64,
) -> Result<(), Status> {
    if payload.backend_ura.is_empty()
        || payload.user_ura.is_empty()
        || payload.session_id.is_empty()
        || payload.scopes.is_empty()
        || payload.audiences.is_empty()
    {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority must carry backend, user, \
             session_id, at least one audience, and at least one scope"
        )));
    }
    if authority_subject_kind(&payload.user_ura) != AuthoritySubjectKind::User {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority user_ura `{}` must be a canonical \
             user subject",
            payload.user_ura
        )));
    }
    if payload.expires_at_ms <= payload.issued_at_ms {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority expires_at_ms must be greater than \
             issued_at_ms"
        )));
    }
    if now_ms >= payload.expires_at_ms {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_EXPIRED}: session authority expired at {}ms (now {}ms)",
            payload.expires_at_ms, now_ms
        )));
    }
    Ok(())
}

fn verify_session_authority_bindings(
    payload: &SessionAuthorityPayload,
    envelope: &Envelope,
    ability: &str,
) -> Result<(), Status> {
    let caller = envelope
        .caller
        .as_ref()
        .map(|c| c.ura.as_str())
        .unwrap_or("");
    if payload.backend_ura != caller {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_CALLER_MISMATCH}: session backend `{}` does not match envelope \
             caller `{caller}`",
            payload.backend_ura
        )));
    }

    let subject = envelope
        .subject
        .as_ref()
        .map(|s| s.ura.as_str())
        .unwrap_or("");
    match authority_subject_kind(subject) {
        AuthoritySubjectKind::User if payload.user_ura != subject => {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_SUBJECT_MISMATCH}: session user `{}` does not match envelope \
                 subject `{subject}`",
                payload.user_ura
            )));
        }
        AuthoritySubjectKind::Session
            if !session_subject_matches_session_id(subject, &payload.session_id) =>
        {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_SUBJECT_MISMATCH}: session authority id `{}` does not bind \
                 envelope subject `{subject}`",
                payload.session_id
            )));
        }
        AuthoritySubjectKind::Other => {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_SUBJECT_MISMATCH}: session authority can only bind user or \
                 session subjects; got `{subject}`"
            )));
        }
        AuthoritySubjectKind::User | AuthoritySubjectKind::Session => {}
    }

    let callee = envelope
        .callee
        .as_ref()
        .map(|c| c.ura.as_str())
        .unwrap_or("");
    if !payload
        .audiences
        .iter()
        .any(|audience| audience_admits(audience, callee))
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_AUDIENCE_VIOLATION}: session audiences {:?} do not admit \
             envelope callee `{callee}`",
            payload.audiences
        )));
    }

    if !payload
        .scopes
        .iter()
        .any(|pattern| scope_matches(pattern, ability))
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SCOPE_VIOLATION}: session scopes {:?} do not admit ability \
             `{ability}`",
            payload.scopes
        )));
    }

    Ok(())
}

fn parse_and_verify_delegation_proof(
    raw_proof: &str,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<DelegationPayload, Status> {
    let wire = BASE64_STANDARD.decode(raw_proof).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: metadata base64 decode failed: {err}"
        ))
    })?;

    let raw: DelegationProofRaw = serde_json::from_slice(&wire).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: raw proof JSON parse failed: {err}"
        ))
    })?;

    let payload: DelegationPayload = serde_json::from_value(raw.payload).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority payload parse failed: {err}"
        ))
    })?;
    validate_delegation_payload_shape(&payload, now_ms)?;

    let payload_bytes = canonical_authority_payload_bytes(&payload)?;
    let signature = BASE64_STANDARD.decode(&raw.signature).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority signature base64 decode failed: {err}"
        ))
    })?;

    let issuer = trust_anchor.lookup(&payload.issuer_ura).ok_or_else(|| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_UNKNOWN}: authority issuer `{}` is not in the realm \
             trust anchor",
            payload.issuer_ura
        ))
    })?;
    verify_delegation_signature(&issuer.public_key_b64, &payload_bytes, &signature)?;

    Ok(payload)
}

fn envelope_requires_authority(envelope: &Envelope) -> bool {
    let Some(caller) = envelope.caller.as_ref().map(|c| c.ura.as_str()) else {
        return false;
    };
    let Some(subject) = envelope.subject.as_ref().map(|s| s.ura.as_str()) else {
        return false;
    };
    if caller == subject {
        return false;
    }
    matches!(
        authority_subject_kind(subject),
        AuthoritySubjectKind::User | AuthoritySubjectKind::Session
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoritySubjectKind {
    User,
    Session,
    Other,
}

fn authority_subject_kind(subject_ura: &str) -> AuthoritySubjectKind {
    match top_level_subject_role(subject_ura) {
        Some("user" | "users") => AuthoritySubjectKind::User,
        Some("session" | "sessions") => AuthoritySubjectKind::Session,
        _ => AuthoritySubjectKind::Other,
    }
}

fn top_level_subject_role(ura: &str) -> Option<&str> {
    let rest = ura.trim().strip_prefix(crate::ura::URA_SCHEME)?;
    let mut segments = rest.split('/');
    let realm = segments.next()?;
    let role = segments.next()?;
    if realm.is_empty() || role.is_empty() {
        return None;
    }
    Some(role)
}

fn session_subject_matches_session_id(subject_ura: &str, session_id: &str) -> bool {
    if session_id.trim().is_empty() {
        return false;
    }
    let Some(rest) = subject_ura.trim().strip_prefix(crate::ura::URA_SCHEME) else {
        return false;
    };
    let mut segments = rest.split('/');
    let Some(realm) = segments.next() else {
        return false;
    };
    let Some(role) = segments.next() else {
        return false;
    };
    let Some(id) = segments.next() else {
        return false;
    };
    segments.next().is_none()
        && !realm.is_empty()
        && matches!(role, "session" | "sessions")
        && id == session_id
}

fn validate_delegation_payload_shape(
    payload: &DelegationPayload,
    now_ms: i64,
) -> Result<(), Status> {
    if payload.issuer_ura.is_empty()
        || payload.subject_ura.is_empty()
        || payload.caller_ura.is_empty()
        || payload.audience.is_empty()
        || payload.scopes.is_empty()
    {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority payload must carry issuer, subject, caller, \
             audience, and at least one scope"
        )));
    }
    if payload.expires_at_ms <= payload.issued_at_ms {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority expires_at_ms must be greater than \
             issued_at_ms"
        )));
    }
    if now_ms >= payload.expires_at_ms {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_EXPIRED}: authority proof expired at {}ms (now {}ms)",
            payload.expires_at_ms, now_ms
        )));
    }
    Ok(())
}

fn verify_delegation_signature(
    issuer_public_key_b64: &str,
    payload_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), Status> {
    let public_key = BASE64_STANDARD
        .decode(issuer_public_key_b64)
        .map_err(|err| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: issuer public key is not valid base64: {err}"
            ))
        })?;
    let key_bytes: [u8; ed25519_dalek::PUBLIC_KEY_LENGTH] =
        public_key.as_slice().try_into().map_err(|_| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: issuer public key wrong size, want {} got {}",
                ed25519_dalek::PUBLIC_KEY_LENGTH,
                public_key.len()
            ))
        })?;
    let signature_bytes: [u8; ed25519_dalek::SIGNATURE_LENGTH] =
        signature_bytes.try_into().map_err(|_| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_SIGNATURE_INVALID}: signature wrong size, want {} got {}",
                ed25519_dalek::SIGNATURE_LENGTH,
                signature_bytes.len()
            ))
        })?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes).map_err(|err| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: issuer public key rejected: {err}"
        ))
    })?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(payload_bytes, &signature)
        .map_err(|err| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_SIGNATURE_INVALID}: authority signature does not verify: {err}"
            ))
        })
}

fn verify_delegation_bindings(
    payload: &DelegationPayload,
    envelope: &Envelope,
    ability: &str,
) -> Result<(), Status> {
    let caller = envelope
        .caller
        .as_ref()
        .map(|c| c.ura.as_str())
        .unwrap_or("");
    if payload.caller_ura != caller {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_CALLER_MISMATCH}: authority caller `{}` does not match envelope \
             caller `{caller}`",
            payload.caller_ura
        )));
    }

    let subject = envelope
        .subject
        .as_ref()
        .map(|s| s.ura.as_str())
        .unwrap_or("");
    if payload.subject_ura != subject {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SUBJECT_MISMATCH}: authority subject `{}` does not match envelope \
             subject `{subject}`",
            payload.subject_ura
        )));
    }

    let callee = envelope
        .callee
        .as_ref()
        .map(|c| c.ura.as_str())
        .unwrap_or("");
    if !audience_admits(&payload.audience, callee) {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_AUDIENCE_VIOLATION}: authority audience `{}` does not admit \
             envelope callee `{callee}`",
            payload.audience
        )));
    }

    if !payload
        .scopes
        .iter()
        .any(|pattern| scope_matches(pattern, ability))
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SCOPE_VIOLATION}: authority scopes {:?} do not admit ability \
             `{ability}`",
            payload.scopes
        )));
    }

    Ok(())
}

fn audience_admits(audience: &str, callee: &str) -> bool {
    audience == "*" || audience == callee || audience.ends_with('/') && callee.starts_with(audience)
}

fn scope_matches(pattern: &str, ability: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return !prefix.is_empty() && ability.starts_with(prefix);
    }
    pattern == ability
}

// Phase 5a deleted `hex_lower` — its sole caller was the
// `record_admission_receipt` helper, which built the
// `invocation_id` string for SharedReceiptStore entries. The
// store + helper are gone; nothing else needed lowercase-hex
// of the 16-byte nonce.

/// Extract `caller.ura` and reject as `invalid_argument` if absent
/// or empty. Shared by every entrypoint so the wire-level
/// "caller URA required" message is identical across surfaces.
fn caller_ura_required(envelope: &Envelope) -> Result<&str, Status> {
    envelope
        .caller
        .as_ref()
        .map(|c| c.ura.as_str())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{REASON_ENVELOPE_INCOMPLETE}: envelope.caller.ura is required \
                 (Invariant 1: caller URA required)"
            ))
        })
}

fn permission_denied_unknown_caller(caller_ura: &str) -> Status {
    Status::permission_denied(format!(
        "{REASON_CALLER_UNKNOWN}: caller URA `{caller_ura}` is not in the realm trust anchor; \
         pairing-flow registration via `<self>.register_device_pubkey` \
         (PR-7 commit 5/N) populates the trust set",
    ))
}

fn reject_public_hosted_agent_delegation_metadata(
    metadata: Option<&HashMap<String, String>>,
) -> Result<(), Status> {
    let Some(_) = metadata
        .and_then(|m| m.get(HOSTED_AGENT_DELEGATION_METADATA_KEY))
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };

    Err(Status::permission_denied(format!(
        "{REASON_HOSTED_AGENT_DELEGATION_LOCAL_ONLY}: `{HOSTED_AGENT_DELEGATION_METADATA_KEY}` \
         is local daemon control metadata and is only accepted on trusted loopback ingress"
    )))
}

/// Map an axon-SDK invocation `AxonError` (the kind admission
/// emits) to a `tonic::Status`. The mapping preserves the canonical
/// reason (e.g. `CALLER_SIGNATURE_INVALID`) inside the status
/// message so audit pipelines and client-side metrics that grep on
/// those strings continue to work.
fn axon_error_to_status(err: InvocationError) -> Status {
    let detail = if err.message.is_empty() {
        err.reason.clone()
    } else {
        format!("{}:{}", err.reason, err.message)
    };
    match err.reason.as_str() {
        REASON_ENVELOPE_INCOMPLETE => Status::invalid_argument(detail),
        REASON_CALLER_SIGNATURE_INVALID => Status::invalid_argument(detail),
        REASON_NONCE_REPLAY => Status::invalid_argument(detail),
        _ => match err.kind {
            InvocationErrorKind::InvalidArgument => Status::invalid_argument(detail),
            InvocationErrorKind::PermissionDenied => Status::permission_denied(detail),
            InvocationErrorKind::ResourceExhausted => Status::resource_exhausted(detail),
            InvocationErrorKind::Unavailable => Status::unavailable(detail),
            InvocationErrorKind::DeadlineExceeded => Status::deadline_exceeded(detail),
            InvocationErrorKind::Cancelled => Status::cancelled(detail),
            InvocationErrorKind::Internal => Status::internal(detail),
        },
    }
}

/// Build the axiom-side `CallerSignature` from the proto field. A
/// missing field is the "no signature carried" case — admission
/// step 2 (`validate_signature_structure`) will reject it with
/// `signature_algorithm_empty`, which is the correct wire-visible
/// outcome.
fn build_axiom_signature(
    proto: Option<&easynet_axon::pb::axon::v1::CallerSignature>,
) -> Result<AxiomCallerSignature, InvocationError> {
    Ok(match proto {
        Some(sig) => AxiomCallerSignature {
            algorithm: sig.algorithm.clone(),
            signature: sig.signature.clone(),
            key_id_hint: sig.key_id_hint.clone(),
        },
        None => AxiomCallerSignature {
            algorithm: String::new(),
            signature: Vec::new(),
            key_id_hint: String::new(),
        },
    })
}

/// DEC-EU §multi-device: returns true iff `envelope.caller.ura`
/// parses to a User-kind URA. Gates whether `run_strict_signature_gate`
/// pins the `FederatedKeyResolver` to the envelope-presented pubkey.
fn envelope_caller_is_user(envelope: &Envelope) -> bool {
    let Some(caller) = envelope.caller.as_ref() else {
        return false;
    };
    matches!(
        crate::ura::parse_ura(&caller.ura).map(|p| p.kind),
        Ok(crate::ura::URAKind::User)
    )
}

/// DEC-EU §multi-device: read the public key the envelope presented
/// via `caller_signature.key_id_hint`. The backend encodes the
/// signer's raw 32-byte Ed25519 verifying key as base64 and stores
/// it in this field; the daemon admission gate trims and returns it
/// verbatim so `FederatedKeyResolver` can pin the verify key to
/// exactly the one the browser used to sign.
///
/// The pubkey hint is `key_id_hint` not a new proto field because
/// `types.proto` already documents `key_id_hint` as "non-trustworthy
/// hint, verifiers MUST resolve independently" — exactly our use:
/// the daemon doesn't trust the hint blindly, it confirms the hint's
/// pubkey is registered under the Caller URA in `realm-trust.toml`
/// before treating it as the verify key.
///
/// Empty when the envelope is hub / device / backend-signed (those
/// callers don't need pubkey disambiguation), or when the caller
/// neglected to set the hint (a programming error; downstream
/// `FederatedKeyResolver` surfaces `CALLER_KEY_NOT_FOUND` and admission
/// rejects).
fn envelope_presented_pubkey_b64(envelope: &Envelope) -> String {
    envelope
        .caller_signature
        .as_ref()
        .map(|sig| sig.key_id_hint.trim().to_string())
        .unwrap_or_default()
}

// (PR-N2 commit 1/N) The local-only `TrustAnchorKeyResolver`
// was deleted in favour of `FederatedKeyResolver`, which
// short-circuits to identical local behavior when no
// federation client is wired and falls through to a peer
// hub's `federation.resolve_key` ability when it is. See
// `services::invocation_transport::federated_key_resolver`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
    use easynet_axon::pb::axon::v1::CallerSignature as PbCallerSignature;
    use ed25519_dalek::{Signer, SigningKey};

    fn hub_ura(realm: &str) -> String {
        crate::ura::hub_ura(realm)
    }

    fn envelope_with_caller(ura: &str) -> Envelope {
        let daemon_ura = hub_ura("realm");
        let subject_ura = default_subject_for(&daemon_ura, "self.echo");
        let mut envelope = crate::services::invocation_transport::ProtoEnvelope::targeted(
            ura,
            &daemon_ura,
            &subject_ura,
        )
        .expect("valid unsigned test envelope")
        .into_inner();
        envelope.invocation_nonce = vec![0x11u8; 16];
        envelope.caller_signature = None;
        envelope
    }

    fn invoke_request(envelope: Option<Envelope>) -> InvokeRequest {
        InvokeRequest {
            envelope,
            function_name: "self.echo".to_string(),
            arguments: b"{}".to_vec(),
            ..InvokeRequest::default()
        }
    }

    fn entry_with_role(ura: &str, public_key_b64: String, role: TrustedAgentRole) -> TrustedAgent {
        TrustedAgent {
            agent_ura: ura.to_string(),
            public_key_b64,
            role,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }
    }

    fn backend_entry(ura: &str, public_key_b64: String) -> TrustedAgent {
        entry_with_role(ura, public_key_b64, TrustedAgentRole::Backend)
    }

    fn device_entry(ura: &str, public_key_b64: String) -> TrustedAgent {
        entry_with_role(ura, public_key_b64, TrustedAgentRole::Device)
    }

    /// Anchor populated with `Backend`-role entries (zero-bytes
    /// public key — tests that exercise the strict path supply a
    /// real key separately). Backend role keeps the strict §5.2
    /// pipeline live for these tests after DEC-013.
    fn backend_anchor(uras: &[&str]) -> Arc<RealmTrustAnchor> {
        Arc::new(
            RealmTrustAnchor::from_entries(
                uras.iter()
                    .map(|u| {
                        backend_entry(
                            u,
                            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                        )
                    })
                    .collect(),
            )
            .expect("test anchor"),
        )
    }

    fn default_subject_for(callee_ura: &str, ability: &str) -> String {
        crate::ura::owner_ability_ura(callee_ura, ability)
            .expect("test callee must own the signed ability")
    }

    /// Build an envelope+signature pair that admits cleanly. `nonce`
    /// is variable so distinct tests don't collide on the daemon-
    /// shared replay store.
    fn signed_request_with_nonce(
        caller_ura: &str,
        callee_ura: &str,
        ability: &str,
        args: &[u8],
        signing_key: &SigningKey,
        nonce: [u8; 16],
    ) -> InvokeRequest {
        let subject_ura = default_subject_for(callee_ura, ability);
        signed_request_with_subject_and_nonce(
            caller_ura,
            callee_ura,
            &subject_ura,
            ability,
            args,
            signing_key,
            nonce,
        )
    }

    fn signed_request_with_subject_and_nonce(
        caller_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
        ability: &str,
        args: &[u8],
        signing_key: &SigningKey,
        nonce: [u8; 16],
    ) -> InvokeRequest {
        let mut envelope = crate::services::invocation_transport::ProtoEnvelope::targeted(
            caller_ura,
            callee_ura,
            subject_ura,
        )
        .expect("valid signed test envelope")
        .into_inner();
        envelope.invocation_nonce = nonce.to_vec();
        let descriptor_bound = descriptor_bound_from_wire_parts(
            envelope.clone(),
            ability.to_string(),
            args,
            WireCallerIdentity::FromEnvelope,
        )
        .expect("descriptor-bound signed test envelope");
        let sig = signing_key.sign(&descriptor_bound.envelope.canonical_bytes());
        envelope.caller_signature = Some(PbCallerSignature {
            algorithm: "ed25519".to_string(),
            signature: sig.to_bytes().to_vec(),
            key_id_hint: String::new(),
        });
        InvokeRequest {
            envelope: Some(envelope),
            function_name: ability.to_string(),
            arguments: args.to_vec(),
            ..InvokeRequest::default()
        }
    }

    // ── URA/loopback gate (preserved from PR-1) ────────────────────

    #[test]
    fn empty_anchor_rejects_external_caller_with_permission_denied() {
        // DEC-013: trust-anchor membership is the first non-loopback
        // check, so a URA not in the anchor short-circuits to
        // permission_denied without ever exercising the §5.2
        // pipeline.
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(envelope_with_caller(
            "easynet:///r/realm/agent/test.external",
        )));
        let err = facade.verify_invoke(&req).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("not in the realm trust anchor"),
            "rejection message must call out membership miss, got: {}",
            err.message()
        );
    }

    #[test]
    fn missing_envelope_returns_invalid_argument() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(None);
        let err = facade.verify_invoke(&req).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("missing envelope"));
    }

    #[test]
    fn missing_caller_ura_returns_invalid_argument() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(Envelope::default()));
        let err = facade.verify_invoke(&req).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("caller URA required"));
    }

    #[test]
    fn daemon_ura_loopback_bypasses_anchor_and_replay() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(hub_ura("realm")),
        );
        let daemon_ura = hub_ura("realm");
        let req = invoke_request(Some(envelope_with_caller(&daemon_ura)));
        facade
            .verify_invoke(&req)
            .expect("daemon loopback admitted without crypto");
        // Loopback must not pollute the replay store.
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn local_system_loopback_bypasses_anchor_and_replay() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(hub_ura("realm")),
        );
        let req = invoke_request(Some(envelope_with_caller(
            crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
        )));

        facade
            .verify_invoke(&req)
            .expect("UDS-origin local system caller admitted without realm trust anchor");
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn loopback_repeat_remains_admitted() {
        // A daemon may invoke `<self>.foo` many times with the same
        // body; loopback bypass is unconditional, so repeated calls
        // never trigger the replay path.
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(hub_ura("realm")),
        );
        let daemon_ura = hub_ura("realm");
        let req = invoke_request(Some(envelope_with_caller(&daemon_ura)));
        for _ in 0..3 {
            facade.verify_invoke(&req).expect("every loopback admitted");
        }
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn tcp_origin_facade_does_not_honour_local_system_bypass() {
        let req = invoke_request(Some(envelope_with_caller(
            crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
        )));
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(hub_ura("realm")),
        )
        .with_loopback_trusted(false);

        let err = facade
            .verify_invoke(&req)
            .expect_err("TCP-origin facade must not honour local system bypass");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("not in the realm trust anchor"),
            "{}",
            err.message()
        );
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn tcp_origin_facade_does_not_honour_loopback_bypass_for_daemon_ura_spoof() {
        // #66: the same DaemonInvocationService is served over a
        // loopback-only UDS and an off-box TCP+TLS socket. The UDS-fed
        // facade trusts the loopback bypass; the TCP-fed facade must
        // not. An unsigned envelope spoofing the daemon's own URA is
        // admitted by the former and rejected (forced through the
        // strict pipeline) by the latter — with no replay pollution on
        // either path.
        let daemon_ura = hub_ura("realm");
        let req = invoke_request(Some(envelope_with_caller(&daemon_ura)));

        let uds_facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(daemon_ura.clone()),
        );
        uds_facade
            .verify_invoke(&req)
            .expect("UDS-origin loopback bypass still admits the daemon's own URA");

        let tcp_facade =
            AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), Some(daemon_ura))
                .with_loopback_trusted(false);
        let err = tcp_facade
            .verify_invoke(&req)
            .expect_err("TCP-origin facade must not honour the loopback bypass");
        // The spoofed daemon URA is not in the (empty) trust anchor, so
        // the strict path rejects it as an unknown caller rather than
        // silently admitting it.
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            tcp_facade.replay_store.is_empty(),
            "an unknown-caller reject must never touch the replay store"
        );
    }

    // ── #185 usage quota ───────────────────────────────────────────

    #[test]
    fn quota_off_leaves_response_unmetered() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/agent/a.b")));
        assert_eq!(
            facade.check_quota(&req).expect("no metering configured"),
            None,
            "without [daemon.quota] the gate must not attach RateLimitInfo"
        );
    }

    #[test]
    fn quota_meters_then_exhausts_external_caller() {
        use crate::persistence::daemon_config::QuotaConfig;
        use crate::services::usage_quota_store::SharedUsageQuotaGate;

        let caller = "easynet:///r/realm/agent/a.b";
        let config = QuotaConfig::new(2, 10_000, std::collections::BTreeMap::new());
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None)
            .with_quota_gate(SharedUsageQuotaGate::from_policy(Some(config)));
        let req = invoke_request(Some(envelope_with_caller(caller)));

        // First two calls are within the cap and surface decreasing
        // remaining budget.
        let first = facade.check_quota(&req).expect("first within cap");
        assert_eq!(first.as_ref().map(|i| i.quota_remaining), Some(1));
        let second = facade.check_quota(&req).expect("second within cap");
        assert_eq!(second.as_ref().map(|i| i.quota_remaining), Some(0));

        // Third exceeds the cap → ResourceExhausted with the wire
        // reason and a retry hint.
        let err = facade.check_quota(&req).expect_err("third over cap");
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
        assert!(
            err.message().contains("QUOTA_EXCEEDED"),
            "wire reason must be QUOTA_EXCEEDED, got: {}",
            err.message()
        );
    }

    #[test]
    fn quota_exempts_loopback_self_caller() {
        use crate::persistence::daemon_config::QuotaConfig;
        use crate::services::usage_quota_store::SharedUsageQuotaGate;

        let daemon_ura = hub_ura("realm");
        // A cap of 1, but the daemon calling itself must never be
        // metered — it would otherwise self-throttle its own `<self>.*`
        // abilities.
        let config = QuotaConfig::new(1, 10_000, std::collections::BTreeMap::new());
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(daemon_ura.clone()),
        )
        .with_quota_gate(SharedUsageQuotaGate::from_policy(Some(config)));
        let req = invoke_request(Some(envelope_with_caller(&daemon_ura)));

        for _ in 0..5 {
            assert_eq!(
                facade.check_quota(&req).expect("loopback never throttled"),
                None,
                "the daemon's own URA is exempt from quota"
            );
        }
    }

    #[test]
    fn quota_rejects_oversized_ability_key_as_invalid_argument() {
        use crate::persistence::daemon_config::QuotaConfig;
        use crate::services::usage_quota_store::{
            SharedUsageQuotaGate, MAX_QUOTA_ABILITY_NAME_BYTES,
        };

        let caller = "easynet:///r/realm/agent/a.b";
        let config = QuotaConfig::new(1, 10_000, std::collections::BTreeMap::new());
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None)
            .with_quota_gate(SharedUsageQuotaGate::from_policy(Some(config)));
        let req = invoke_request(Some(envelope_with_caller(caller)));
        let ability = "a".repeat(MAX_QUOTA_ABILITY_NAME_BYTES + 1);

        let err = facade
            .check_quota_for_ability(&req, &ability)
            .expect_err("oversized quota key must be rejected");

        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("REQUEST_METADATA_INVALID"),
            "wire reason must name quota key bound, got: {}",
            err.message()
        );
    }

    // ── Full §5.2 pipeline ─────────────────────────────────────────

    #[test]
    fn unsigned_external_caller_rejected_with_signature_missing_reason() {
        // PR-7 LB-05 callout: this is the new wire-visible behaviour
        // that breaks the unsigned-envelope PR-6 e2e until commit
        // 7/N restores it with a signed payload.
        //
        // Caller URA shape note: trust-anchor entries under the
        // Backend role MUST be hub URAs (`from_entries` enforces
        // role-URA canonicality). We model the external caller as
        // a peer-realm hub so the URA shape is contract-valid while
        // the realm distinction keeps it outside the daemon's
        // loopback bypass.
        let peer_hub = hub_ura("peer-realm");
        let facade =
            AdmissionFacade::new(backend_anchor(&[peer_hub.as_str()]), Some(hub_ura("realm")));
        let req = invoke_request(Some(envelope_with_caller(&peer_hub)));
        let err = facade.verify_invoke(&req).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_CALLER_SIGNATURE_INVALID),
            "wire reason must be AXON_CALLER_SIGNATURE_INVALID, got: {}",
            err.message()
        );
    }

    #[test]
    fn signed_caller_with_trust_anchor_entry_admitted() {
        let signing_key = SigningKey::from_bytes(&[0x42u8; 32]);
        let pub_key = signing_key.verifying_key();
        let pub_key_b64 = BASE64_STANDARD.encode(pub_key.to_bytes());

        // Backend-role trust entries MUST be hub URAs per
        // `canonical_ura_for_role`. We use distinct peer-realm hub
        // URAs across tests so the daemon-shared replay store sees
        // distinct (caller, nonce) pairs even when tests interleave.
        let caller_ura = hub_ura("peer-signer-a");
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(&caller_ura, pub_key_b64)])
                .expect("anchor"),
        );

        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));

        let req = signed_request_with_nonce(
            &caller_ura,
            &daemon_ura,
            "self.echo",
            b"{}",
            &signing_key,
            [0x11u8; 16],
        );
        facade.verify_invoke(&req).expect("signed caller admitted");
        // Replay store retains exactly this nonce.
        assert_eq!(facade.replay_store.len(), 1);
    }

    #[test]
    fn public_signed_caller_cannot_inject_hosted_agent_delegation_metadata() {
        let signing_key = SigningKey::from_bytes(&[0x43u8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let caller_ura = hub_ura("peer-hosted-delegation-spoof");
        let daemon_ura = hub_ura("realm");
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(&caller_ura, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));
        let mut req = signed_request_with_nonce(
            &caller_ura,
            &daemon_ura,
            "meta.teach",
            b"{}",
            &signing_key,
            [0x43u8; 16],
        );
        req.metadata.insert(
            HOSTED_AGENT_DELEGATION_METADATA_KEY.to_string(),
            r#"{"kind":"hosted_agent","agent_ura":"easynet:///r/realm/agent/a","signing_authority":"host_device","wire_caller_ura":"ignored","wire_callee_ura":"ignored","wire_subject_ura":"ignored","ability":"meta.teach"}"#.to_string(),
        );

        let err = facade
            .verify_invoke(&req)
            .expect_err("public ingress must reject hosted-agent delegation metadata");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message()
                .contains(REASON_HOSTED_AGENT_DELEGATION_LOCAL_ONLY),
            "{}",
            err.message()
        );
        assert!(
            facade.replay_store.is_empty(),
            "local-only hosted delegation rejection must happen before nonce recording"
        );
    }

    #[test]
    fn loopback_may_carry_hosted_agent_delegation_metadata() {
        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), Some(daemon_ura));
        let mut req = invoke_request(Some(envelope_with_caller(
            crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
        )));
        req.metadata.insert(
            HOSTED_AGENT_DELEGATION_METADATA_KEY.to_string(),
            r#"{"kind":"hosted_agent"}"#.to_string(),
        );

        facade
            .verify_invoke(&req)
            .expect("trusted loopback may carry local hosted-agent delegation metadata");
        assert!(facade.replay_store.is_empty());
    }

    // Phase 5a removed the four receipt-store local tests
    // (`strict_admission_records_receipt`,
    //  `loopback_admission_does_not_record_receipt`,
    //  `replay_rejection_records_rejected_receipt`,
    //  `device_ura_only_records_annotated_receipt`).
    // Successful admission is now observed via the
    // `LedgerSink`-installed `InvocationLedger` at terminal time;
    // rejected admission is observed via the wire-level gRPC
    // error their caller sees. The behavioural contracts those
    // tests pinned ("admission accepts signed callers", "loopback
    // is a no-op bypass", "replay is rejected with
    // `NONCE_REPLAY`") remain covered by the
    // signed-caller accept/reject sites in
    // the other test functions below — those still exercise the
    // accept / reject paths, they just stop asserting on the now-
    // deleted ring buffer.

    #[test]
    fn signed_caller_replay_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x99u8; 32]);
        let pub_key = signing_key.verifying_key();
        let pub_key_b64 = BASE64_STANDARD.encode(pub_key.to_bytes());

        let caller_ura = hub_ura("peer-replay");
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(&caller_ura, pub_key_b64)])
                .expect("anchor"),
        );
        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));

        let req = signed_request_with_nonce(
            &caller_ura,
            &daemon_ura,
            "self.echo",
            b"{}",
            &signing_key,
            [0x22u8; 16],
        );
        facade.verify_invoke(&req).expect("first admitted");
        let err = facade.verify_invoke(&req).expect_err("replay must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_NONCE_REPLAY),
            "wire reason must be CALLER_NONCE_REPLAYED, got: {}",
            err.message()
        );
    }

    #[test]
    fn signed_caller_with_wrong_key_rejected() {
        // Trust anchor lists a different public key than the
        // signer's. verify_signature fails; admission propagates
        // CALLER_SIGNATURE_INVALID.
        let signing_key = SigningKey::from_bytes(&[0x55u8; 32]);
        let other_key = SigningKey::from_bytes(&[0x66u8; 32]);
        let other_pub_b64 = BASE64_STANDARD.encode(other_key.verifying_key().to_bytes());

        let caller_ura = hub_ura("peer-wrong-key");
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(&caller_ura, other_pub_b64)])
                .expect("anchor"),
        );
        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));

        let req = signed_request_with_nonce(
            &caller_ura,
            &daemon_ura,
            "self.echo",
            b"{}",
            &signing_key,
            [0x33u8; 16],
        );
        let err = facade
            .verify_invoke(&req)
            .expect_err("wrong-key signature must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains(REASON_CALLER_SIGNATURE_INVALID));
        // Failed crypto verify must not record the nonce.
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn signed_caller_unknown_ura_rejected_with_permission_denied() {
        // DEC-013: a caller URA absent from the trust anchor never
        // reaches the §5.2 pipeline; membership miss short-circuits
        // to permission_denied. The signature is valid in shape but
        // we never bother verifying it — the trust-anchor lookup is
        // the gating check.
        let signing_key = SigningKey::from_bytes(&[0x77u8; 32]);
        let trust = Arc::new(RealmTrustAnchor::default());
        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));

        let req = signed_request_with_nonce(
            "easynet:///r/realm/agent/test.uninvited",
            &daemon_ura,
            "self.echo",
            b"{}",
            &signing_key,
            [0x44u8; 16],
        );
        let err = facade
            .verify_invoke(&req)
            .expect_err("unknown caller URA must reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("not in the realm trust anchor"));
    }

    #[test]
    fn invoke_stream_uses_same_pipeline() {
        let signing_key = SigningKey::from_bytes(&[0x88u8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let caller_ura = hub_ura("peer-streamer");
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(&caller_ura, pub_key_b64)])
                .expect("anchor"),
        );
        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));

        let req = signed_request_with_nonce(
            &caller_ura,
            &daemon_ura,
            "federation.subscribe_directory",
            b"{}",
            &signing_key,
            [0x55u8; 16],
        );
        let stream_req = InvokeServerStreamRequest {
            envelope: req.envelope.clone(),
            function_name: req.function_name.clone(),
            arguments: req.arguments.clone(),
            ..InvokeServerStreamRequest::default()
        };
        facade
            .verify_invoke_stream(&stream_req)
            .expect("admitted on stream too");
    }

    #[test]
    fn shared_replay_store_serialises_dual_facades() {
        // Two facades sharing one replay store reject each other's
        // replays — the daemon-wide dedup window holds across any
        // listener split that PR-10 might introduce.
        let signing_key = SigningKey::from_bytes(&[0xAAu8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let caller_ura = hub_ura("peer-shared");
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(&caller_ura, pub_key_b64)])
                .expect("anchor"),
        );

        let store = SharedNonceReplayStore::new();
        let daemon_ura = hub_ura("realm");
        let facade_a = AdmissionFacade::with_replay_store(
            Arc::clone(&trust),
            Some(daemon_ura.clone()),
            store.clone(),
        );
        let facade_b = AdmissionFacade::with_replay_store(
            Arc::clone(&trust),
            Some(daemon_ura.clone()),
            store.clone(),
        );

        let req = signed_request_with_nonce(
            &caller_ura,
            &daemon_ura,
            "self.echo",
            b"{}",
            &signing_key,
            [0x66u8; 16],
        );
        facade_a.verify_invoke(&req).expect("facade A admits first");
        let err = facade_b
            .verify_invoke(&req)
            .expect_err("facade B must reject the replayed nonce");
        assert!(err.message().contains(REASON_NONCE_REPLAY));
    }

    // ── Strict public admission by role ───────────────────────────

    /// Anchor with one Device-role entry. The all-zero public key is
    /// enough for unsigned-rejection tests because structure validation
    /// fails before public-key resolution.
    fn device_anchor(ura: &str) -> Arc<RealmTrustAnchor> {
        Arc::new(
            RealmTrustAnchor::from_entries(vec![device_entry(
                ura,
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            )])
            .expect("anchor"),
        )
    }

    struct NoopFederationClient;

    #[async_trait::async_trait]
    impl crate::services::federation_client::FederationClient for NoopFederationClient {
        async fn forward_invoke(
            &self,
            _target_hub: &crate::services::federation_client::HubUri,
            _request: InvokeRequest,
        ) -> Result<
            easynet_axon::pb::axon::v1::InvokeResponse,
            crate::services::federation_client::FederationClientError,
        > {
            Err(
                crate::services::federation_client::FederationClientError::DialFailed {
                    hub: "https://noop.test:50443".to_string(),
                    detail: "noop test client".to_string(),
                },
            )
        }
    }

    #[test]
    fn parse_realm_from_ura_accepts_canonical_hub_and_device_shapes() {
        assert_eq!(
            parse_realm_from_ura(&hub_ura("peer-realm")),
            Some("peer-realm".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/peer-realm/device/device-123"),
            Some("peer-realm".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/peer-realm/hub"),
            Some("peer-realm".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/peer-realm/hub/extra"),
            None
        );
    }

    #[test]
    fn is_federated_caller_accepts_canonical_hub_ura() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(hub_ura("local-realm")),
        )
        .with_federation(
            Arc::new(NoopFederationClient),
            SharedFederatedPeers::new(std::collections::BTreeMap::from([(
                "peer-realm".to_string(),
                "https://peer.example:50443".to_string(),
            )])),
        );
        assert!(facade.is_federated_caller(&hub_ura("peer-realm")));
        assert!(facade.is_federated_caller("easynet:///r/peer-realm/hub"));
        assert!(!facade.is_federated_caller("easynet:///r/peer-realm/hub/extra"));
    }

    #[test]
    fn device_role_rejects_unsigned_envelope() {
        let caller_ura = "easynet:///r/realm/device/device-A";
        let facade = AdmissionFacade::new(device_anchor(caller_ura), Some(hub_ura("realm")));
        let req = invoke_request(Some(envelope_with_caller(caller_ura)));

        let err = facade
            .verify_invoke(&req)
            .expect_err("device path must reject unsigned envelopes");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_CALLER_SIGNATURE_INVALID),
            "wire reason must be AXON_CALLER_SIGNATURE_INVALID, got: {}",
            err.message(),
        );
        assert!(
            facade.replay_store.is_empty(),
            "structure failures must not record nonces",
        );
    }

    #[test]
    fn device_role_rejects_repeated_unsigned_envelopes_without_nonce_mutation() {
        let caller_ura = "easynet:///r/realm/device/device-B";
        let facade = AdmissionFacade::new(device_anchor(caller_ura), Some(hub_ura("realm")));
        let req = invoke_request(Some(envelope_with_caller(caller_ura)));
        for _ in 0..3 {
            let err = facade
                .verify_invoke(&req)
                .expect_err("each unsigned device call rejects before replay");
            assert_eq!(err.code(), tonic::Code::InvalidArgument);
            assert!(err.message().contains(REASON_CALLER_SIGNATURE_INVALID));
        }
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn device_role_uses_strict_path_when_signature_is_present() {
        let signing_key = SigningKey::from_bytes(&[0xAB_u8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let caller_ura = "easynet:///r/realm/device/device-signed";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![device_entry(caller_ura, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some(hub_ura("realm")));

        let req = signed_request_with_nonce(
            caller_ura,
            caller_ura,
            // Wire-pinned self-session spelling until EasyNet-Axon
            // ships device.session acceptance (RFC-001 v4.1.6).
            "<self>.session",
            b"",
            &signing_key,
            [0x5Au8; 16],
        );
        facade
            .verify_invoke(&req)
            .expect("signed device caller admitted");
        assert_eq!(facade.replay_store.len(), 1);

        let err = facade
            .verify_invoke(&req)
            .expect_err("replayed signed device nonce must reject");
        assert!(err.message().contains(REASON_NONCE_REPLAY));
    }

    #[test]
    fn role_dispatch_keeps_backend_and_device_strict() {
        // Two callers in the same anchor: one Backend, one Device.
        // Both callers go through strict §5.2; role dispatch no
        // longer carries a Device-specific unsigned compatibility arm.
        let backend_signing = SigningKey::from_bytes(&[0xC0u8; 32]);
        let backend_pub_b64 = BASE64_STANDARD.encode(backend_signing.verifying_key().to_bytes());
        let device_signing = SigningKey::from_bytes(&[0xC1u8; 32]);
        let device_pub_b64 = BASE64_STANDARD.encode(device_signing.verifying_key().to_bytes());
        // Backend role demands a hub URA per `canonical_ura_for_role`,
        // and the strict pipeline must NOT be short-circuited by the
        // loopback bypass — so we route the caller through a
        // peer-realm hub URA. The daemon's self URA (set below as
        // the second `AdmissionFacade::new` arg) stays in the local
        // `realm` so caller_ura != self_ura and the strict path runs.
        let backend_ura = hub_ura("peer-role-dispatch");
        let device_ura = "easynet:///r/realm/device/device-C";

        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![
                backend_entry(&backend_ura, backend_pub_b64),
                device_entry(device_ura, device_pub_b64),
            ])
            .expect("anchor"),
        );
        let daemon_ura = hub_ura("realm");
        let facade = AdmissionFacade::new(trust, Some(daemon_ura.clone()));

        // Device caller, unsigned: rejects strict.
        let device_req = invoke_request(Some(envelope_with_caller(device_ura)));
        let err = facade
            .verify_invoke(&device_req)
            .expect_err("device arm rejects unsigned");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        // Device caller, properly signed: admits strict and records
        // the nonce in the replay store.
        let device_signed = signed_request_with_nonce(
            device_ura,
            &daemon_ura,
            "self.echo",
            b"{}",
            &device_signing,
            [0xC2u8; 16],
        );
        facade
            .verify_invoke(&device_signed)
            .expect("device arm admits signed");

        // Backend caller, unsigned: rejects strict.
        let backend_unsigned = invoke_request(Some(envelope_with_caller(&backend_ura)));
        let err = facade
            .verify_invoke(&backend_unsigned)
            .expect_err("backend arm rejects unsigned");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        // Backend caller, properly signed: admits strict and records
        // the nonce in the replay store.
        let backend_signed = signed_request_with_nonce(
            &backend_ura,
            &daemon_ura,
            "self.echo",
            b"{}",
            &backend_signing,
            [0xD0u8; 16],
        );
        facade
            .verify_invoke(&backend_signed)
            .expect("backend arm admits signed");
        assert_eq!(
            facade.replay_store.len(),
            2,
            "both signed public callers should hit the replay store",
        );
    }
}

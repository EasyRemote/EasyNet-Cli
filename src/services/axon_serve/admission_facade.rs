// EasyNet CLI — axon_serve — admission gate facade
// ==================================================
//
// File: src/services/axon_serve/admission_facade.rs
// Description: Per-RPC admission check the dispatcher consults
//              before routing into a federation wrapper or any
//              ability handler.
//
// What this module does (PR-7 commit 4/N — DEC-013 path-conditional)
// ------------------------------------------------------------------
// 1. Reads the `Envelope` from an inbound `pb::axon::v1::InvokeRequest`
//    (or its server-stream / bidi counterpart)
// 2. **Loopback bypass**: callers presenting the daemon's own URI
//    are admitted without crypto — the daemon trusts itself
// 3. **Trust-anchor membership** (always): unknown caller URIs are
//    rejected with `permission_denied` before any structural work,
//    so unrelated callers cannot push entries into the replay store
// 4. **Path-conditional admission by `TrustedAgent.role`** (DEC-013
//    Option D, see below):
//      - **Backend** → strict 4-step §5.2 pipeline
//          a. `validate_envelope`           (RFC 001 §5.2 step 1)
//          b. `validate_signature_structure` (RFC 001 §5.2 step 2)
//          c. `verify_signature` against the trust-anchor-backed
//             `KeyResolver`                  (RFC 001 §5.2 step 3)
//          d. `NonceReplayStore::check_and_record` against the
//             daemon-shared store            (RFC 001 §5.2 step 4)
//      - **Device** → URI-only for legacy unsigned callers; if the
//        device already carries a real signature+nonce the same
//        strict 4-step pipeline runs immediately. This keeps
//        deployed unsigned devices alive while letting PR-2/PR-7
//        device-session frame 0 exercise the real crypto path.
//      - **Hub** → strict 4-step (cross-realm federation)
// 5. Returns `Ok(())` for accept and a `tonic::Status` for reject —
//    the only outcomes the dispatcher needs

// Why path-conditional, not strict-everywhere
// -------------------------------------------
// PR-7 commit 4/N upgrades the gate from URI-only to strict crypto.
// `kernel.rs:609/689/742/774` show 4 device-side call sites that
// emit unsigned envelopes today; an unconditional strict gate
// PermissionDenies every deployed device immediately, forcing
// re-pair on every host. DEC-013 keeps the strict semantics on the
// Backend/Hub paths (which do sign — PR-7 commit 2/N landed
// backend signing) while leaving the Device path at the PR-1 URI-
// only behaviour until PR-8 introduces device sign-on-send. The
// `TrustedAgent.role` field (set at pairing time per PR-7 commit
// 5/N's `<self>.register_device_pubkey`) is the dispatch axis;
// the gate writes no new state to make path selection work.
//
// What this module does NOT do (yet)
// ----------------------------------
// **Receipt emission** — RFC 001 §5.3 admission-emits-receipt. Per
// DEC-012, receipt minting is deferred to PR-10 (production canary)
// where the receipt store and the signing key are wired together.
// PR-7 (this commit) intentionally leaves admission as a yes/no gate
// — receipts on the InvokeResponse remain `None` for now.
//
// Cross-PR coupling note
// ----------------------
// **This commit makes the existing PR-6 e2e test (`go test
// -tags=e2e`) fail until PR-7 commit 6/N lands.** The e2e test
// formerly exercised an unsigned envelope and observed a
// `PermissionDenied` from the URI-not-in-trust-anchor branch. With
// the upgraded gate, an unsigned envelope from a non-loopback URI
// fails `validate_signature_structure` (signature_algorithm_empty)
// and the wire-visible reason changes from `permission_denied` to
// `invalid_argument` with reason `AXON_CALLER_SIGNATURE_INVALID`.
// The flip is by design — commit 6/N teaches the EasyNet backend's
// `verifyCredentialLogic` to invoke the new `<self>.register_device_pubkey`
// ability (PR-7 commit 5/N) so the trust set carries a real public
// key the gate can verify against, and commit 7/N updates the e2e
// to drive a signed envelope through the now-strict gate.
//
// Invariants
// ----------
// **Invariant 1 (caller URI required)**: Every inbound RPC must
// carry an `Envelope` with a non-empty `caller.uri`. The dispatcher
// receives `Status::invalid_argument` for any RPC missing this; it
// is a wire-level requirement, not a policy choice.
//
// **Invariant 2 (loopback bypass)**: When the caller URI matches
// the daemon's configured URI, admission accepts without consulting
// the trust anchor or the replay store. The daemon trusts itself —
// `<self>.*` abilities and admin RPCs originate from the daemon's
// own process and need not sign.
//
// **Invariant 3 (path-conditional strict crypto)**: External
// callers split into two paths by `TrustedAgent.role`. The
// `Backend` and `Hub` paths run the full §5.2 pipeline end-to-end:
// a missing/malformed `caller_signature` rejects with
// `AXON_CALLER_SIGNATURE_INVALID`; a signature that fails to
// verify against the trust anchor's public-key entry rejects with
// the same reason; a nonce already observed inside the dedup
// window rejects with `AXON_NONCE_REPLAY`. The `Device` path keeps
// URI-only admission for unsigned legacy callers, but any device
// envelope that already carries signature material runs the same
// strict pipeline immediately.
//
// **Invariant 4 (replay store mutation discipline)**: The replay
// store is mutated only after `validate_envelope` and
// `validate_signature_structure` both pass — malformed callers
// can never pollute the store. This is a property of
// `easynet_axon::invocation::admission::run_admission`, which
// orders the four steps so structure failures short-circuit before
// the nonce hits the map.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

#[cfg(test)]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use sha2::{Digest, Sha256};
use tonic::Status;

use easynet_axon::invocation::admission::{
    now_ms as axon_now_ms, run_admission, REASON_CALLER_SIGNATURE_INVALID,
    REASON_ENVELOPE_INCOMPLETE, REASON_NONCE_REPLAY,
};
use easynet_axon::invocation::axiom::{
    AgentIdentity as AxiomAgentIdentity, CallerSignature as AxiomCallerSignature, CausalContext,
    InvocationEnvelope, KeyResolver, ReceiptRef, SubjectIdentity, UriProfile,
};
use easynet_axon::invocation::{
    AxonError as InvocationError, AxonErrorKind as InvocationErrorKind,
};

use crate::pb::axon::v1::{
    causal_context, CausalContext as PbCausalContext, Envelope, EnvelopeOpen, InvocationState,
    InvokeRequest, InvokeServerStreamRequest,
};
use crate::services::axon_serve::federated_key_resolver::{
    FederatedKeyResolver, SharedFederatedKeyCache,
};
use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::nonce_replay_store::SharedNonceReplayStore;
use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgentRole};
use crate::services::receipt_store::SharedReceiptStore;
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// Per-RPC admission gate consulted by `DaemonInvocationService`
/// before routing into a federation wrapper or fallthrough handler.
///
/// Holds:
/// - `Arc<RealmTrustAnchor>` — the trust set authored by PR-7's
///   pairing flow and read at boot by the daemon binary
/// - `daemon_uri` — the daemon's own canonical URI (loopback bypass)
/// - `replay_store` — the daemon-shared `SharedNonceReplayStore` so
///   replay windows hold across all admissions
///
/// Constructed once per daemon process; cloned into per-request
/// dispatcher tasks (clone is cheap — all fields are `Arc` or
/// `Option<String>`).
#[derive(Clone)]
pub struct AdmissionFacade {
    trust_anchor: SharedTrustAnchor,
    daemon_uri: Option<String>,
    replay_store: SharedNonceReplayStore,
    /// Bounded ring buffer where the strict-success path records a
    /// signed `InvocationReceipt` per accepted call (PR-10 commit
    /// 3/N — RFC 001 §5.3 + DEC-012 close). Production daemons
    /// thread one shared store from `start_axon_serve_sidecar`;
    /// tests / smoke runs default to a fresh empty store. INV-5
    /// is honoured by construction: `record` never errors.
    receipt_store: SharedReceiptStore,
    /// **PR-N2 commit 1/N**. Cross-hub federation client used by
    /// `FederatedKeyResolver` to dial a peer hub's
    /// `federation.resolve_key` ability when the local trust
    /// anchor has no entry for a cross-realm caller URI. `None`
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
    /// `FederatedKeyResolver`. Derived from `daemon_uri` when
    /// not supplied directly. `None` in test builds with no
    /// daemon URI wired.
    self_realm: Option<String>,
    /// **C3b** TTL cache shared across every per-admission
    /// `FederatedKeyResolver` instance. Without this share, the
    /// per-call resolver would build a fresh empty cache and
    /// the TTL would deliver zero savings. Boot-time SIGHUP
    /// handler holds a clone too so a trust-anchor reload can
    /// flush all cached cross-realm pubkeys atomically.
    federated_key_cache: SharedFederatedKeyCache,
}

impl std::fmt::Debug for AdmissionFacade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionFacade")
            .field("trust_anchor", &self.trust_anchor)
            .field("daemon_uri", &self.daemon_uri)
            .field("replay_store", &self.replay_store)
            .field("receipt_store", &self.receipt_store)
            .field(
                "federation_client",
                &self
                    .federation_client
                    .as_ref()
                    .map(|_| "<dyn FederationClient>"),
            )
            .field("federated_peers", &self.federated_peers)
            .field("self_realm", &self.self_realm)
            .finish()
    }
}

impl AdmissionFacade {
    /// Construct a facade against the supplied trust anchor and
    /// daemon URI. Production callers thread the daemon's
    /// `credentials.json`-derived URI through; tests typically pass
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
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>, daemon_uri: Option<String>) -> Self {
        Self::with_trust_anchor_cell(SharedTrustAnchor::new(trust_anchor), daemon_uri)
    }

    /// Construct a facade against a shared trust-anchor cell. Used
    /// by `start_axon_serve_sidecar` so the same cell is shared
    /// with the `<self>.register_device_pubkey` handler — a
    /// successful register publishes the new anchor and the next
    /// admission snapshot reflects it without daemon restart.
    #[must_use]
    pub fn with_trust_anchor_cell(
        trust_anchor: SharedTrustAnchor,
        daemon_uri: Option<String>,
    ) -> Self {
        let self_realm = daemon_uri
            .as_deref()
            .and_then(crate::services::axon_serve::register_device_pubkey::parse_realm_from_uri)
            .map(str::to_string);
        Self {
            trust_anchor,
            daemon_uri,
            replay_store: SharedNonceReplayStore::new(),
            receipt_store: SharedReceiptStore::new(),
            federation_client: None,
            federated_peers: SharedFederatedPeers::new(std::collections::BTreeMap::new()),
            self_realm,
            federated_key_cache: SharedFederatedKeyCache::new(),
        }
    }

    /// Builder seam: set the daemon-shared receipt store
    /// (PR-10 commit 3/N). Production callers thread one store
    /// per daemon process from `start_axon_serve_sidecar`; tests
    /// pass a tighter-bounded store to exercise eviction paths.
    #[must_use]
    pub fn with_receipt_store(mut self, receipt_store: SharedReceiptStore) -> Self {
        self.receipt_store = receipt_store;
        self
    }

    /// Snapshot the daemon-shared receipt store. Used by PR-10
    /// commit 5/N's e2e to assert that a signed-success
    /// admission produced a recorded receipt; future audit
    /// query (RFC-N PR-N5) will replace this borrow with a
    /// richer subscription API.
    #[must_use]
    pub fn receipt_store(&self) -> &SharedReceiptStore {
        &self.receipt_store
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

    /// The daemon's own canonical URI. Used by per-ability
    /// admission filters that need to recognise the loopback
    /// caller (eg. `federation.list_user_devices` in N3-5
    /// admits the daemon talking to itself without requiring
    /// a Hub trust entry for its own URI).
    #[must_use]
    pub fn daemon_uri(&self) -> Option<&str> {
        self.daemon_uri.as_deref()
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
        daemon_uri: Option<String>,
        replay_store: SharedNonceReplayStore,
    ) -> Self {
        let self_realm = daemon_uri
            .as_deref()
            .and_then(crate::services::axon_serve::register_device_pubkey::parse_realm_from_uri)
            .map(str::to_string);
        Self {
            trust_anchor: SharedTrustAnchor::new(trust_anchor),
            daemon_uri,
            replay_store,
            receipt_store: SharedReceiptStore::new(),
            federation_client: None,
            federated_peers: SharedFederatedPeers::new(std::collections::BTreeMap::new()),
            self_realm,
            federated_key_cache: SharedFederatedKeyCache::new(),
        }
    }

    /// **PR-N2 commit 1/N**. Builder seam: wire the cross-hub
    /// federation client + operator-curated federated_peers
    /// cell. When set, the strict admission path constructs a
    /// `FederatedKeyResolver` instead of `TrustAnchorKeyResolver`,
    /// which means a cross-realm caller whose URI is missing
    /// from the local trust anchor falls through to a
    /// `federation.resolve_key` ability call against the peer
    /// hub mapped by `federated_peers[caller_tenant]`.
    ///
    /// Production daemons call this in
    /// `start_axon_serve_sidecar` after wiring the dialer; test
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

    /// Verify a unary `InvokeRequest`. Returns `Ok(())` when the
    /// caller is admitted; otherwise a `tonic::Status` mapped per
    /// the rule set in `run_full_admission`.
    pub fn verify_invoke(&self, request: &InvokeRequest) -> Result<(), Status> {
        let envelope = request
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Invoke request missing envelope"))?;
        self.run_full_admission(envelope, &request.function_name, &request.arguments)
    }

    /// Verify a server-stream `InvokeServerStreamRequest`. Same rule
    /// set as `verify_invoke`; the differing wrapper is just the
    /// proto type.
    pub fn verify_invoke_stream(&self, request: &InvokeServerStreamRequest) -> Result<(), Status> {
        let envelope = request
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("InvokeStream request missing envelope"))?;
        self.run_full_admission(envelope, &request.function_name, &request.arguments)
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
        self.run_full_admission(envelope, ability, &open.initial_args)
    }

    /// Direct-envelope entrypoint reserved for the PR-2 InvokeBidi
    /// path that does NOT carry an EnvelopeOpen — surface kept stable
    /// so existing callers compile. Defers to the loopback-only
    /// fast path; full admission requires the (ability, args) tuple
    /// the other entrypoints supply.
    ///
    /// PR-7 note: this is the URI-only legacy gate. The bidi path
    /// has migrated to `verify_envelope_for_bidi`. Remove once
    /// PR-2's session bidi handler also supplies (ability, args).
    pub fn verify_envelope_uri_only(&self, envelope: &Envelope) -> Result<(), Status> {
        let caller_uri = caller_uri_required(envelope)?;
        if self.is_loopback(caller_uri) {
            return Ok(());
        }
        let snapshot = self.trust_anchor.snapshot();
        if snapshot.lookup(caller_uri).is_some() {
            return Ok(());
        }
        Err(permission_denied_unknown_caller(caller_uri))
    }

    // ── Internal pipeline ────────────────────────────────────────

    /// Path-conditional admission per DEC-013 Option D.
    ///
    /// Order:
    /// 1. Caller URI required (Invariant 1).
    /// 2. Loopback bypass (Invariant 2).
    /// 3. Trust-anchor membership: unknown URI → `permission_denied`.
    /// 4. Path by `TrustedAgent.role`:
    ///    - `Backend`    → strict 4-step §5.2 pipeline (signs envelopes)
    ///    - `Device`     → URI-only no-op (devices don't sign yet —
    ///                     PR-8 flips this arm to strict once
    ///                     device-side sign-on-send lands)
    ///    - `Hub`        → strict 4-step (cross-realm federation)
    ///
    /// The Device arm is the temporary boundary disclosed in
    /// DEC-013: PR-7 commit 4/N upgrades backend↔daemon to strict
    /// crypto without invalidating already-deployed devices that
    /// were never taught to sign. The arm collapses to strict in
    /// PR-8 (one source-line flip, no feature flag, no ramp).
    fn run_full_admission(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
    ) -> Result<(), Status> {
        let caller_uri = caller_uri_required(envelope)?;

        // Invariant 2: loopback bypass. Daemon trusts itself.
        if self.is_loopback(caller_uri) {
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
        // DEC-013 entry contract says "URI-not-in-trust-set" surfaces
        // before any attempt at envelope/signature parsing, so an
        // unrelated caller cannot waste structure-validation cycles
        // and never has its (possibly malformed) nonce considered.
        //
        // **PR-N2 commit 1/N — cross-realm extension**. When the
        // caller is in a *federated* realm (its tenant is mapped in
        // `federated_peers` AND differs from `self_realm`), the
        // local trust anchor will NOT have an entry: the caller's
        // identity is gated by the peer hub, not by us. In that
        // case we route to the strict admission path with the
        // FederatedKeyResolver, which dials the peer's
        // `federation.resolve_key` to fetch the verifying key and
        // runs the same RFC 001 §5.2 4-step verify. This preserves
        // INV-1 (federated trust gate is *operator-explicit*: only
        // tenants the operator listed in `federated_peers` can
        // bypass the local-membership reject) while opening the
        // cross-realm signed-admission door.
        let trusted = match snapshot.lookup(caller_uri) {
            Some(entry) => entry,
            None => {
                if self.is_federated_caller(caller_uri) {
                    return self.run_strict_admission(envelope, ability, args, snapshot);
                }
                return Err(permission_denied_unknown_caller(caller_uri));
            }
        };

        match trusted.role {
            // Device path: URI-only admission until PR-8 device-side
            // sign-on-send lands. No envelope/signature/replay work —
            // device runtime today emits unsigned envelopes (kernel.rs
            // 4 sites: caller_signature: None) and DEC-013 explicitly
            // refuses to break already-deployed devices.
            //
            // PR-10 commit 4/N: emit a receipt even on this URI-only
            // path so the audit pipeline sees the call happen. The
            // `reason` annotation `"unsigned_caller_uri_admitted"`
            // distinguishes this from a strict-path admit; PR-8
            // flips Device to strict and the annotation becomes
            // dead in a follow-up.
            TrustedAgentRole::Device => {
                if envelope_carries_signature_material(envelope) {
                    self.run_strict_admission(envelope, ability, args, snapshot)
                } else {
                    self.record_admission_receipt(
                        envelope,
                        ability,
                        args,
                        "admitted",
                        InvocationState::Completed,
                        "unsigned_caller_uri_admitted",
                    );
                    Ok(())
                }
            }

            // Backend & Hub: strict 4-step admission. Backends sign
            // canonical bytes per PR-7 commit 2/N; hubs sign per
            // PR-10's federation surface. The strict path resolves
            // the caller's public key against the same snapshot we
            // just consulted for membership — keeps "membership
            // hit" and "key resolved" referring to the same anchor
            // version.
            TrustedAgentRole::Backend | TrustedAgentRole::Hub => {
                self.run_strict_admission(envelope, ability, args, snapshot)
            }
        }
    }

    /// Strict §5.2 admission for the Backend / Hub arms of DEC-013.
    /// Bridges proto → axiom domain types and dispatches into
    /// `easynet_axon::invocation::admission::run_admission` with a
    /// snapshot-backed `KeyResolver` and the daemon-shared replay
    /// store. On success, records an `InvocationReceipt` into the
    /// daemon-shared receipt store (PR-10 commit 3/N — RFC 001
    /// §5.3 + DEC-012 close). Receipt emission is best-effort per
    /// PR-10 spec INV-5: a poisoned-lock recovery never bubbles
    /// up as an admission failure.
    fn run_strict_admission(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
        trust_anchor: Arc<RealmTrustAnchor>,
    ) -> Result<(), Status> {
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "envelope present but ability/function_name is empty; cannot run admission",
            ));
        }

        let axiom_envelope =
            build_axiom_envelope(envelope, ability, args).map_err(axon_error_to_status)?;
        let axiom_signature = build_axiom_signature(envelope.caller_signature.as_ref())
            .map_err(axon_error_to_status)?;

        // **PR-N2 commit 1/N**. Build a `FederatedKeyResolver`
        // that wraps the per-call snapshot trust anchor with
        // the daemon-shared federation client + federated_peers
        // cell. Same-realm callers short-circuit on the local
        // anchor lookup (zero added latency); cross-realm
        // callers fall through to a peer hub's
        // `federation.resolve_key` ability iff the operator
        // marked their tenant as federated. The
        // `FederatedKeyResolver` mirrors the
        // `TrustAnchorKeyResolver` shape on the local-only
        // path, so single-realm setups are byte-identical to
        // PR-7 commit 4/N.
        let resolver: Box<dyn KeyResolver> = Box::new(
            FederatedKeyResolver::new(
                trust_anchor,
                self.federation_client.clone(),
                self.federated_peers.snapshot(),
                self.self_realm.clone(),
            )
            .with_cache(self.federated_key_cache.clone()),
        );

        let result = self.replay_store.with_inner(|store| {
            run_admission(
                &axiom_envelope,
                &axiom_signature,
                Some(resolver.as_ref()),
                store,
                axon_now_ms(),
            )
        });

        match result {
            Ok(()) => {
                self.record_admission_receipt(
                    envelope,
                    ability,
                    args,
                    "admitted",
                    InvocationState::Completed,
                    "",
                );
                Ok(())
            }
            Err(err) => {
                // PR-10 commit 4/N: reject-path receipts. Every
                // wire-visible admission outcome — including the
                // three RFC 001 §5.2 reject reasons
                // (AXON_AXIOM_ENVELOPE_INCOMPLETE /
                // AXON_CALLER_SIGNATURE_INVALID / AXON_NONCE_REPLAY)
                // — emits a `receipt_type = "rejected"` receipt
                // with the canonical reason in `reason`. Audit
                // pipelines see byte-symmetric coverage: every
                // call attempt is observable, not just the
                // accepted ones.
                self.record_admission_receipt(
                    envelope,
                    ability,
                    args,
                    "rejected",
                    InvocationState::Failed,
                    &err.reason,
                );
                Err(axon_error_to_status(err))
            }
        }
    }

    /// PR-10 commit 3/N + 4/N: build and record an
    /// `InvocationReceipt` for an admission outcome. Called from
    /// three sites:
    /// - strict-success path → `("admitted", Completed, "")`
    /// - strict-reject path → `("rejected", Failed,
    ///   AXON_AXIOM_ENVELOPE_INCOMPLETE | AXON_CALLER_SIGNATURE_INVALID
    ///   | AXON_NONCE_REPLAY)`
    /// - Device URI-only-no-op path →
    ///   `("admitted", Completed, "unsigned_caller_uri_admitted")`
    ///
    /// Field shape:
    /// - `receipt_type` / `state` / `reason` come from the call
    ///   site (audit pipelines grep on these; the wire-string
    ///   reasons are byte-stable per RFC 001 §5.2)
    /// - identity bindings (caller / callee / subject) cloned
    ///   from the envelope — proves which call this receipt
    ///   attests to even on reject paths
    /// - `invocation_nonce` echoed for audit-side dedup
    /// - `payload_digest = sha256(args)` — same digest the
    ///   admission gate computed for §5.2 step 1
    /// - `timestamp_unix_ms = axon_now_ms()`
    ///
    /// What this receipt does NOT carry yet (deferred to follow-up):
    /// - `callee_signature` — receipt signing key wiring lands
    ///   in a follow-up; v1 emits unsigned receipts
    /// - `prev_receipt_hash` chain — root-only for v1
    /// - `causal_binding` — copied verbatim today; will be
    ///   richer once RFC-N PR-N5 chains kick in
    ///
    /// INV-5 honoured: this method never errors. A poisoned
    /// receipt-store lock recovers via `into_inner` inside
    /// `record`.
    fn record_admission_receipt(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
        receipt_type: &str,
        state: InvocationState,
        reason: &str,
    ) {
        use crate::pb::axon::v1::InvocationReceipt;
        use sha2::Digest;

        let mut hasher = Sha256::new();
        hasher.update(args);
        let _payload_digest: Vec<u8> = hasher.finalize().to_vec();

        let invocation_id = format!(
            "{}:{}:{}",
            envelope
                .caller
                .as_ref()
                .map(|c| c.uri.as_str())
                .unwrap_or(""),
            ability,
            hex_lower(&envelope.invocation_nonce),
        );

        let receipt = InvocationReceipt {
            index: 0,
            invocation_id,
            receipt_type: receipt_type.to_string(),
            state: state as i32,
            timestamp_unix_ms: axon_now_ms(),
            prev_receipt_hash: vec![0u8; 32],
            self_hash: Vec::new(),
            payload: Vec::new(),
            payload_content_type: String::new(),
            cleanup_complete: true,
            reason: reason.to_string(),
            child_invocation_id: String::new(),
            caller_binding: envelope.caller.clone(),
            callee_binding: envelope.callee.clone(),
            subject_binding: envelope.subject.clone(),
            invocation_nonce: envelope.invocation_nonce.clone(),
            causal_binding: envelope.causal_context.clone(),
            callee_signature: None,
            ..InvocationReceipt::default()
        };
        self.receipt_store.record(receipt);
    }

    fn is_loopback(&self, caller_uri: &str) -> bool {
        match self.daemon_uri.as_deref() {
            Some(self_uri) => caller_uri == self_uri,
            None => false,
        }
    }

    /// **PR-N2 commit 1/N**. Decide whether `caller_uri` belongs to
    /// a federated peer realm — i.e. a realm the operator has
    /// explicitly opted into by adding a `[daemon.federated_peers]`
    /// map entry mapping `tenant → hub_uri`.
    ///
    /// Returns `true` iff:
    ///   - the URI parses to a non-self tenant
    ///   - the federated_peers cell holds an entry for that tenant
    ///   - a federation client is wired (without one, the strict
    ///     path's FederatedKeyResolver has no way to dial the peer
    ///     and would just fail closed — short-circuit here)
    fn is_federated_caller(&self, caller_uri: &str) -> bool {
        let Some(client) = self.federation_client.as_ref() else {
            return false;
        };
        let _ = client; // presence-only check; resolver does the dial
        let Some(caller_tenant) = parse_realm_from_uri(caller_uri) else {
            return false;
        };
        if let Some(self_realm) = self.self_realm.as_deref() {
            if caller_tenant == self_realm {
                return false;
            }
        }
        let peers = self.federated_peers.snapshot();
        peers.contains_key(caller_tenant)
    }
}

/// **PR-N2 commit 1/N**. Parse the realm component from a canonical
/// EasyNet URI (`easynet:///r/<realm>/...`). Returns the realm slice
/// when the shape matches, `None` otherwise. Shared by
/// `is_federated_caller` and the cross-realm gate.
///
/// Important: federated callers in v4.1.4 are no longer uniformly
/// `.../agent/...`; peer hubs use the singleton `.../hub` shape and
/// device sessions register under `.../device/<id>`. Reuse the same
/// realm parser as `<self>.register_device_pubkey` so all canonical
/// role tails stay accepted.
fn parse_realm_from_uri(uri: &str) -> Option<&str> {
    crate::services::axon_serve::register_device_pubkey::parse_realm_from_uri(uri)
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Lowercase-hex a byte slice. Receipt invocation_id construction
/// uses this to render the 16-byte invocation nonce. Inlined here
/// rather than pulling `hex` as a dep because it's a one-off use.
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Extract `caller.uri` and reject as `invalid_argument` if absent
/// or empty. Shared by every entrypoint so the wire-level
/// "caller URI required" message is identical across surfaces.
fn caller_uri_required(envelope: &Envelope) -> Result<&str, Status> {
    envelope
        .caller
        .as_ref()
        .map(|c| c.uri.as_str())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "envelope.caller.uri is required (Invariant 1: caller URI required)",
            )
        })
}

fn permission_denied_unknown_caller(caller_uri: &str) -> Status {
    Status::permission_denied(format!(
        "caller URI `{caller_uri}` is not in the realm trust anchor; \
         pairing-flow registration via `<self>.register_device_pubkey` \
         (PR-7 commit 5/N) populates the trust set",
    ))
}

/// Map an axon-SDK invocation `AxonError` (the kind admission
/// emits) to a `tonic::Status`. The mapping preserves the canonical
/// reason (e.g. `AXON_CALLER_SIGNATURE_INVALID`) inside the status
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

/// Bridge a proto `Envelope` (+ ability + args bytes) into an
/// `InvocationEnvelope` ready for `run_admission`. The proto wire
/// schema does NOT carry `args_digest` or `ability` directly — they
/// derive from the surrounding `InvokeRequest.function_name` and
/// `arguments`, so the bridge takes them as separate inputs.
///
/// `args_digest = SHA-256(arguments)` — this matches
/// `daemon_grpc/canonical.go::CanonicalInvocationBytes` (Go side)
/// and `axon::canonical_invocation_bytes` (Rust side) per DEC-009
/// (verbatim SHA-256, no JCS).
fn build_axiom_envelope(
    envelope: &Envelope,
    ability: &str,
    args: &[u8],
) -> Result<InvocationEnvelope, InvocationError> {
    let caller = envelope
        .caller
        .as_ref()
        .ok_or_else(|| reject_envelope("caller_missing"))?;
    let callee = envelope
        .callee
        .as_ref()
        .ok_or_else(|| reject_envelope("callee_missing"))?;
    let subject = envelope
        .subject
        .as_ref()
        .ok_or_else(|| reject_envelope("subject_missing"))?;

    let caller_profile = parse_profile_or_default(&caller.profile)?;
    let callee_profile = parse_profile_or_default(&callee.profile)?;
    let subject_profile = parse_profile_or_default(&subject.profile)?;

    let invocation_nonce: [u8; 16] = envelope
        .invocation_nonce
        .as_slice()
        .try_into()
        .map_err(|_| reject_envelope("invocation_nonce_wrong_length"))?;

    let causal_context = match envelope.causal_context.as_ref() {
        Some(ctx) => bridge_causal_context(ctx)?,
        None => CausalContext::None,
    };

    let mut hasher = Sha256::new();
    hasher.update(args);
    let args_digest: [u8; 32] = hasher.finalize().into();

    Ok(InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller.uri.clone(), caller_profile),
        callee: AxiomAgentIdentity::new(callee.uri.clone(), callee_profile),
        subject: SubjectIdentity::new(subject.uri.clone(), subject_profile),
        ability: ability.to_string(),
        args_digest,
        invocation_nonce,
        causal_context,
    })
}

/// Default a missing/empty profile field to the RFC 001 default
/// (`easynet-strict-v2`). This mirrors the Go canonical encoder's
/// behaviour — the proto wire allows the field to be empty when the
/// default is in effect, and admission must produce the same
/// canonical bytes either way.
fn parse_profile_or_default(profile: &str) -> Result<UriProfile, InvocationError> {
    if profile.is_empty() {
        return Ok(UriProfile::EasynetStrictV2);
    }
    UriProfile::parse(profile)
}

fn bridge_causal_context(ctx: &PbCausalContext) -> Result<CausalContext, InvocationError> {
    let Some(form) = ctx.form.as_ref() else {
        return Ok(CausalContext::None);
    };
    match form {
        causal_context::Form::None(_) => Ok(CausalContext::None),
        causal_context::Form::Scalar(r) => {
            let receipt_hash = receipt_hash_from_bytes(&r.receipt_hash)?;
            Ok(CausalContext::Scalar(ReceiptRef {
                receipt_hash,
                receipt_uri: r.receipt_uri.clone(),
            }))
        }
        causal_context::Form::List(list) => {
            let mut out = Vec::with_capacity(list.prior.len());
            for r in &list.prior {
                out.push(ReceiptRef {
                    receipt_hash: receipt_hash_from_bytes(&r.receipt_hash)?,
                    receipt_uri: r.receipt_uri.clone(),
                });
            }
            Ok(CausalContext::List(out))
        }
        causal_context::Form::Merkle(m) => {
            let root = receipt_hash_from_bytes(&m.root)?;
            Ok(CausalContext::Merkle {
                root,
                proof_uri: m.proof_uri.clone(),
            })
        }
    }
}

fn receipt_hash_from_bytes(bytes: &[u8]) -> Result<[u8; 32], InvocationError> {
    bytes
        .try_into()
        .map_err(|_| reject_envelope("receipt_hash_wrong_length"))
}

/// Build the axiom-side `CallerSignature` from the proto field. A
/// missing field is the "no signature carried" case — admission
/// step 2 (`validate_signature_structure`) will reject it with
/// `signature_algorithm_empty`, which is the correct wire-visible
/// outcome.
fn build_axiom_signature(
    proto: Option<&crate::pb::axon::v1::CallerSignature>,
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

fn reject_envelope(detail: &str) -> InvocationError {
    InvocationError::invalid_argument(REASON_ENVELOPE_INCOMPLETE).with_message(detail.to_string())
}

fn envelope_carries_signature_material(envelope: &Envelope) -> bool {
    envelope
        .caller_signature
        .as_ref()
        .map(|sig| !sig.algorithm.trim().is_empty() || !sig.signature.is_empty())
        .unwrap_or(false)
}

// (PR-N2 commit 1/N) The local-only `TrustAnchorKeyResolver`
// was deleted in favour of `FederatedKeyResolver`, which
// short-circuits to identical local behavior when no
// federation client is wired and falls through to a peer
// hub's `federation.resolve_key` ability when it is. See
// `services::axon_serve::federated_key_resolver`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::axon::v1::{
        AgentIdentity as PbAgentIdentity, CallerSignature as PbCallerSignature,
        SubjectIdentity as PbSubjectIdentity,
    };
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
    use ed25519_dalek::{Signer, SigningKey};

    fn agent(uri: &str) -> PbAgentIdentity {
        PbAgentIdentity {
            uri: uri.to_string(),
            ..PbAgentIdentity::default()
        }
    }

    fn subject(uri: &str) -> PbSubjectIdentity {
        PbSubjectIdentity {
            uri: uri.to_string(),
            ..PbSubjectIdentity::default()
        }
    }

    fn envelope_with_caller(uri: &str) -> Envelope {
        Envelope {
            caller: Some(agent(uri)),
            callee: Some(agent("easynet:///r/realm/hub")),
            subject: Some(subject("easynet:///r/realm/hub")),
            invocation_nonce: vec![0x11u8; 16],
            ..Envelope::default()
        }
    }

    fn invoke_request(envelope: Option<Envelope>) -> InvokeRequest {
        InvokeRequest {
            envelope,
            function_name: "self.echo".to_string(),
            arguments: b"{}".to_vec(),
            ..InvokeRequest::default()
        }
    }

    fn entry_with_role(uri: &str, public_key_b64: String, role: TrustedAgentRole) -> TrustedAgent {
        TrustedAgent {
            agent_uri: uri.to_string(),
            public_key_b64,
            role,
            added_at_unix_ms: 1_714_492_800_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        }
    }

    fn backend_entry(uri: &str, public_key_b64: String) -> TrustedAgent {
        entry_with_role(uri, public_key_b64, TrustedAgentRole::Backend)
    }

    fn device_entry(uri: &str, public_key_b64: String) -> TrustedAgent {
        entry_with_role(uri, public_key_b64, TrustedAgentRole::Device)
    }

    /// Anchor populated with `Backend`-role entries (zero-bytes
    /// public key — tests that exercise the strict path supply a
    /// real key separately). Backend role keeps the strict §5.2
    /// pipeline live for these tests after DEC-013.
    fn backend_anchor(uris: &[&str]) -> Arc<RealmTrustAnchor> {
        Arc::new(
            RealmTrustAnchor::from_entries(
                uris.iter()
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

    /// Build an envelope+signature pair that admits cleanly. `nonce`
    /// is variable so distinct tests don't collide on the daemon-
    /// shared replay store.
    fn signed_request_with_nonce(
        caller_uri: &str,
        callee_uri: &str,
        ability: &str,
        args: &[u8],
        signing_key: &SigningKey,
        nonce: [u8; 16],
    ) -> (InvokeRequest, [u8; 32]) {
        // Build the canonical bytes the same way axon's encoder does
        // so we sign over what admission will verify against.
        use easynet_axon::invocation::axiom::canonical_invocation_bytes;

        let mut hasher = Sha256::new();
        hasher.update(args);
        let args_digest: [u8; 32] = hasher.finalize().into();

        let axiom_env = InvocationEnvelope {
            caller: AxiomAgentIdentity::new(caller_uri, UriProfile::EasynetStrictV2),
            callee: AxiomAgentIdentity::new(callee_uri, UriProfile::EasynetStrictV2),
            subject: SubjectIdentity::new(callee_uri, UriProfile::EasynetStrictV2),
            ability: ability.to_string(),
            args_digest,
            invocation_nonce: nonce,
            causal_context: CausalContext::None,
        };
        let bytes = canonical_invocation_bytes(&axiom_env);
        let sig = signing_key.sign(&bytes);

        let envelope = Envelope {
            caller: Some(PbAgentIdentity {
                uri: caller_uri.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(PbAgentIdentity {
                uri: callee_uri.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(PbSubjectIdentity {
                uri: callee_uri.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: nonce.to_vec(),
            caller_signature: Some(PbCallerSignature {
                algorithm: "ed25519".to_string(),
                signature: sig.to_bytes().to_vec(),
                key_id_hint: String::new(),
            }),
            ..Envelope::default()
        };
        (
            InvokeRequest {
                envelope: Some(envelope),
                function_name: ability.to_string(),
                arguments: args.to_vec(),
                ..InvokeRequest::default()
            },
            args_digest,
        )
    }

    // ── URI/loopback gate (preserved from PR-1) ────────────────────

    #[test]
    fn empty_anchor_rejects_external_caller_with_permission_denied() {
        // DEC-013: trust-anchor membership is the first non-loopback
        // check, so a URI not in the anchor short-circuits to
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
    fn missing_caller_uri_returns_invalid_argument() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(Envelope::default()));
        let err = facade.verify_invoke(&req).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("caller URI required"));
    }

    #[test]
    fn daemon_uri_loopback_bypasses_anchor_and_replay() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/realm/hub".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/hub")));
        facade
            .verify_invoke(&req)
            .expect("daemon loopback admitted without crypto");
        // Loopback must not pollute the replay store.
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn loopback_repeat_remains_admitted() {
        // A daemon may invoke `<self>.foo` many times with the same
        // body; loopback bypass is unconditional, so repeated calls
        // never trigger the replay path.
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/realm/hub".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/hub")));
        for _ in 0..3 {
            facade.verify_invoke(&req).expect("every loopback admitted");
        }
        assert!(facade.replay_store.is_empty());
    }

    // ── Full §5.2 pipeline ─────────────────────────────────────────

    #[test]
    fn unsigned_external_caller_rejected_with_signature_invalid_reason() {
        // PR-7 LB-05 callout: this is the new wire-visible behaviour
        // that breaks the unsigned-envelope PR-6 e2e until commit
        // 7/N restores it with a signed payload.
        let facade = AdmissionFacade::new(
            backend_anchor(&["easynet:///r/realm/agent/test.external"]),
            Some("easynet:///r/realm/hub".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller(
            "easynet:///r/realm/agent/test.external",
        )));
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

        let caller_uri = "easynet:///r/realm/agent/test.signer-a";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );

        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        let (req, _digest) = signed_request_with_nonce(
            caller_uri,
            "easynet:///r/realm/hub",
            "self.echo",
            b"{}",
            &signing_key,
            [0x11u8; 16],
        );
        facade.verify_invoke(&req).expect("signed caller admitted");
        // Replay store retains exactly this nonce.
        assert_eq!(facade.replay_store.len(), 1);
    }

    /// PR-10 commit 3/N receipt emission: every strict-path
    /// success records an `InvocationReceipt` into the daemon-
    /// shared receipt store. The receipt's identity bindings
    /// echo the envelope; the receipt_type is `"admitted"` so
    /// audit pipelines can grep.
    #[test]
    fn strict_admission_records_receipt() {
        let signing_key = SigningKey::from_bytes(&[0x55u8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());

        let caller_uri = "easynet:///r/realm/agent/test.receipt-emitter";
        let callee_uri = "easynet:///r/realm/hub";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some(callee_uri.to_string()));
        assert!(facade.receipt_store().is_empty());

        let (req, _digest) = signed_request_with_nonce(
            caller_uri,
            callee_uri,
            "self.echo",
            b"{}",
            &signing_key,
            [0x77u8; 16],
        );
        facade.verify_invoke(&req).expect("signed caller admitted");

        // Exactly one receipt recorded for the one accepted call.
        assert_eq!(facade.receipt_store().len(), 1);
        let recent = facade.receipt_store().snapshot_recent(1);
        let receipt = recent.into_iter().next().expect("one receipt");
        assert_eq!(receipt.receipt_type, "admitted");
        assert_eq!(
            receipt
                .caller_binding
                .as_ref()
                .expect("caller_binding present")
                .uri,
            caller_uri
        );
        assert_eq!(
            receipt
                .callee_binding
                .as_ref()
                .expect("callee_binding present")
                .uri,
            callee_uri
        );
        assert_eq!(receipt.invocation_nonce, vec![0x77u8; 16]);
        assert!(
            !receipt.invocation_id.is_empty(),
            "invocation_id derived from caller+ability+nonce"
        );
    }

    /// Loopback-bypass admissions DO NOT record receipts
    /// (loopback caller is the daemon talking to itself; the
    /// receipt would have no audit value). Pin the contract.
    #[test]
    fn loopback_admission_does_not_record_receipt() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/realm/hub".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/hub")));
        facade.verify_invoke(&req).expect("loopback admitted");
        assert!(
            facade.receipt_store().is_empty(),
            "loopback bypass must not pollute the receipt store"
        );
    }

    /// PR-10 commit 4/N: reject-path receipts. A signed caller
    /// admitted, then replayed → first call records "admitted",
    /// second records "rejected" with `reason = AXON_NONCE_REPLAY`.
    /// The audit pipeline sees both outcomes byte-symmetrically.
    #[test]
    fn replay_rejection_records_rejected_receipt() {
        let signing_key = SigningKey::from_bytes(&[0xCCu8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());

        let caller_uri = "easynet:///r/realm/agent/test.replay-receipt";
        let callee_uri = "easynet:///r/realm/hub";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some(callee_uri.to_string()));

        let (req, _digest) = signed_request_with_nonce(
            caller_uri,
            callee_uri,
            "self.echo",
            b"{}",
            &signing_key,
            [0xDDu8; 16],
        );
        facade.verify_invoke(&req).expect("first admission OK");
        let _ = facade
            .verify_invoke(&req)
            .expect_err("second is replay-rejected");

        // Two receipts: one admitted, one rejected.
        assert_eq!(facade.receipt_store().len(), 2);
        let recent = facade.receipt_store().snapshot_recent(2);
        let admitted = &recent[0];
        let rejected = &recent[1];
        assert_eq!(admitted.receipt_type, "admitted");
        assert_eq!(rejected.receipt_type, "rejected");
        assert!(
            rejected.reason.contains("NONCE_REPLAY"),
            "rejected receipt must carry the canonical reason; got {:?}",
            rejected.reason
        );
    }

    /// PR-10 commit 4/N: Device URI-only-no-op path also emits a
    /// receipt, with `reason = "unsigned_caller_uri_admitted"`.
    /// PR-8 will flip the Device arm strict and delete this
    /// annotation in a follow-up.
    #[test]
    fn device_uri_only_records_annotated_receipt() {
        let caller_uri = "easynet:///r/realm/device/unsigned-device";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![device_entry(
                caller_uri,
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            )])
            .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));
        let req = invoke_request(Some(envelope_with_caller(caller_uri)));
        facade
            .verify_invoke(&req)
            .expect("device URI-only admitted");

        assert_eq!(facade.receipt_store().len(), 1);
        let recent = facade.receipt_store().snapshot_recent(1);
        let receipt = &recent[0];
        assert_eq!(receipt.receipt_type, "admitted");
        assert_eq!(receipt.reason, "unsigned_caller_uri_admitted");
        assert_eq!(
            receipt
                .caller_binding
                .as_ref()
                .expect("caller_binding present")
                .uri,
            caller_uri
        );
    }

    #[test]
    fn signed_caller_replay_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x99u8; 32]);
        let pub_key = signing_key.verifying_key();
        let pub_key_b64 = BASE64_STANDARD.encode(pub_key.to_bytes());

        let caller_uri = "easynet:///r/realm/agent/test.replay";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        let (req, _) = signed_request_with_nonce(
            caller_uri,
            "easynet:///r/realm/hub",
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
            "wire reason must be AXON_NONCE_REPLAY, got: {}",
            err.message()
        );
    }

    #[test]
    fn signed_caller_with_wrong_key_rejected() {
        // Trust anchor lists a different public key than the
        // signer's. verify_signature fails; admission propagates
        // AXON_CALLER_SIGNATURE_INVALID.
        let signing_key = SigningKey::from_bytes(&[0x55u8; 32]);
        let other_key = SigningKey::from_bytes(&[0x66u8; 32]);
        let other_pub_b64 = BASE64_STANDARD.encode(other_key.verifying_key().to_bytes());

        let caller_uri = "easynet:///r/realm/agent/test.wrong-key";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, other_pub_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        let (req, _) = signed_request_with_nonce(
            caller_uri,
            "easynet:///r/realm/hub",
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
    fn signed_caller_unknown_uri_rejected_with_permission_denied() {
        // DEC-013: a caller URI absent from the trust anchor never
        // reaches the §5.2 pipeline; membership miss short-circuits
        // to permission_denied. The signature is valid in shape but
        // we never bother verifying it — the trust-anchor lookup is
        // the gating check.
        let signing_key = SigningKey::from_bytes(&[0x77u8; 32]);
        let trust = Arc::new(RealmTrustAnchor::default());
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        let (req, _) = signed_request_with_nonce(
            "easynet:///r/realm/agent/test.uninvited",
            "easynet:///r/realm/hub",
            "self.echo",
            b"{}",
            &signing_key,
            [0x44u8; 16],
        );
        let err = facade
            .verify_invoke(&req)
            .expect_err("unknown caller URI must reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("not in the realm trust anchor"));
    }

    #[test]
    fn invoke_stream_uses_same_pipeline() {
        let signing_key = SigningKey::from_bytes(&[0x88u8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let caller_uri = "easynet:///r/realm/agent/test.streamer";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        let (req, _) = signed_request_with_nonce(
            caller_uri,
            "easynet:///r/realm/hub",
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
        let caller_uri = "easynet:///r/realm/agent/test.shared";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![backend_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );

        let store = SharedNonceReplayStore::new();
        let facade_a = AdmissionFacade::with_replay_store(
            Arc::clone(&trust),
            Some("easynet:///r/realm/hub".to_string()),
            store.clone(),
        );
        let facade_b = AdmissionFacade::with_replay_store(
            Arc::clone(&trust),
            Some("easynet:///r/realm/hub".to_string()),
            store.clone(),
        );

        let (req, _) = signed_request_with_nonce(
            caller_uri,
            "easynet:///r/realm/hub",
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

    // ── DEC-013 Option D: path-conditional admission by role ───────

    /// Anchor with one Device-role entry. Public key is the all-zero
    /// byte string — Device-arm callers never have their key resolved
    /// (no signature verification under DEC-013), so the byte content
    /// is immaterial.
    fn device_anchor(uri: &str) -> Arc<RealmTrustAnchor> {
        Arc::new(
            RealmTrustAnchor::from_entries(vec![device_entry(
                uri,
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
            crate::pb::axon::v1::InvokeResponse,
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
    fn parse_realm_from_uri_accepts_hub_and_device_shapes() {
        assert_eq!(
            parse_realm_from_uri("easynet:///r/peer-realm/hub"),
            Some("peer-realm")
        );
        assert_eq!(
            parse_realm_from_uri("easynet:///r/peer-realm/device/device-123"),
            Some("peer-realm")
        );
    }

    #[test]
    fn is_federated_caller_accepts_v414_hub_uri() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/local-realm/hub".to_string()),
        )
        .with_federation(
            Arc::new(NoopFederationClient),
            SharedFederatedPeers::new(std::collections::BTreeMap::from([(
                "peer-realm".to_string(),
                "https://peer.example:50443".to_string(),
            )])),
        );
        assert!(facade.is_federated_caller("easynet:///r/peer-realm/hub"));
    }

    #[test]
    fn device_role_admits_unsigned_envelope_per_dec013() {
        // The DEC-013 boundary: a device URI in the trust anchor
        // admits without signature, without nonce recording, and
        // without crypto. PR-8 will flip this arm to strict — for
        // PR-7 ship, it preserves URI-only PR-1 semantics for
        // already-deployed devices.
        let caller_uri = "easynet:///r/realm/device/device-A";
        let facade = AdmissionFacade::new(
            device_anchor(caller_uri),
            Some("easynet:///r/realm/hub".to_string()),
        );
        // Bare envelope, no signature — the kind kernel.rs emits today.
        let req = invoke_request(Some(envelope_with_caller(caller_uri)));
        facade
            .verify_invoke(&req)
            .expect("device path admits unsigned envelope under DEC-013");
        assert!(
            facade.replay_store.is_empty(),
            "device path must not record nonces (PR-8 territory)",
        );
    }

    #[test]
    fn device_role_admits_repeated_unsigned_envelopes() {
        // Replay protection only kicks in on the strict path; the
        // device path is no-op so repeated identical envelopes admit
        // every time. Once PR-8 lands and devices sign, this test
        // flips its assertion (call 2 must reject as replay).
        let caller_uri = "easynet:///r/realm/device/device-B";
        let facade = AdmissionFacade::new(
            device_anchor(caller_uri),
            Some("easynet:///r/realm/hub".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller(caller_uri)));
        for _ in 0..3 {
            facade.verify_invoke(&req).expect("each device call admits");
        }
        assert!(facade.replay_store.is_empty());
    }

    #[test]
    fn device_role_uses_strict_path_when_signature_is_present() {
        let signing_key = SigningKey::from_bytes(&[0xAB_u8; 32]);
        let pub_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let caller_uri = "easynet:///r/realm/device/device-signed";
        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![device_entry(caller_uri, pub_key_b64)])
                .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        let (req, _) = signed_request_with_nonce(
            caller_uri,
            caller_uri,
            // Wire-pinned legacy until EasyNet-Axon ships
            // device.session acceptance (RFC-001 v4.1.6).
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
    fn role_dispatch_keeps_backend_strict_alongside_device_no_op() {
        // Two callers in the same anchor: one Backend, one Device.
        // The Backend caller still goes through strict §5.2; the
        // Device caller still no-ops. This is the dispatch axis
        // working — same trust anchor, two policies.
        let backend_signing = SigningKey::from_bytes(&[0xC0u8; 32]);
        let backend_pub_b64 = BASE64_STANDARD.encode(backend_signing.verifying_key().to_bytes());
        let backend_uri = "easynet:///r/realm/hub";
        let device_uri = "easynet:///r/realm/device/device-C";

        let trust = Arc::new(
            RealmTrustAnchor::from_entries(vec![
                backend_entry(backend_uri, backend_pub_b64),
                device_entry(
                    device_uri,
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                ),
            ])
            .expect("anchor"),
        );
        let facade = AdmissionFacade::new(trust, Some("easynet:///r/realm/hub".to_string()));

        // Device caller: unsigned, admitted.
        let device_req = invoke_request(Some(envelope_with_caller(device_uri)));
        facade
            .verify_invoke(&device_req)
            .expect("device arm admits unsigned");

        // Backend caller, unsigned: rejects strict.
        let backend_unsigned = invoke_request(Some(envelope_with_caller(backend_uri)));
        let err = facade
            .verify_invoke(&backend_unsigned)
            .expect_err("backend arm rejects unsigned");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        // Backend caller, properly signed: admits strict and records
        // the nonce in the replay store.
        let (backend_signed, _) = signed_request_with_nonce(
            backend_uri,
            "easynet:///r/realm/hub",
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
            1,
            "only the backend signed call should hit the replay store",
        );
    }
}

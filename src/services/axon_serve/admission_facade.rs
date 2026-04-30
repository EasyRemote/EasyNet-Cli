// EasyNet CLI — axon_serve — admission gate facade
// ==================================================
//
// File: src/services/axon_serve/admission_facade.rs
// Description: Per-RPC admission check the dispatcher consults
//              before routing into a federation wrapper or any
//              ability handler.
//
// What this module does
// ---------------------
// 1. Reads the `Envelope` from an inbound `pb::axon::v1::InvokeRequest`
//    (or its server-stream / bidi counterpart)
// 2. Confirms the caller URI is present in the daemon's
//    `RealmTrustAnchor`
// 3. Returns `Ok(())` for accept and a `tonic::Status` for reject —
//    the only outcomes the dispatcher needs
//
// What this module does NOT do (yet)
// ----------------------------------
// PR-1 spec §5 sequences admission verification across multiple
// PRs. This commit lands the URI-in-trust-set check only — the
// minimum gate that surfaces "caller is unknown" to operators
// while the trust set is still being populated by PR-7's pairing
// flow. Specifically:
//
// - **Full envelope canonical-bytes signature verification** —
//   `easynet_axon::invocation::admission::run_admission` requires
//   `InvocationEnvelope` + `CallerSignature` constructed from the
//   proto Envelope fields. Constructing those domain types from
//   the proto wire (especially the `CausalContext` oneof) is
//   straightforward but not free; PR-7 lands it alongside the
//   real signed payloads from the pairing flow
// - **Nonce replay protection** — needs a long-lived
//   `NonceReplayStore` shared across requests. Plumbing the
//   shared mutex through the dispatcher is mechanical; PR-7 also
//   adds this since real signed payloads are needed to test the
//   replay protection meaningfully
// - **Receipt emission** — RFC 001 §5.3 admission-emits-receipt.
//   PR-1 does not emit; PR-7 wires receipt minting alongside the
//   real signature verification
//
// This staging is consistent with spec §5 ("PR-1 to PR-7 admission
// is permissive but URI-in-trust-set; PR-7 makes it strict") and
// with DEC-002's runbook obligation that ties admission strictness
// to the production canary's pre-conditions.
//
// Invariants
// ----------
// **Invariant 1 (caller URI required)**: Every inbound RPC must
// carry an `Envelope` with a non-empty `caller.uri`. The dispatcher
// receives `Status::invalid_argument` for any RPC missing this; it
// is a wire-level requirement, not a policy choice.
//
// **Invariant 2 (trust set membership)**: When the trust anchor is
// non-empty, every `caller.uri` that is *not* in it is rejected
// with `Status::permission_denied`. When the trust anchor *is*
// empty (PR-1 fallback path), every external caller is rejected
// with the same status; only the daemon's own loopback callers
// (caller URI matching the daemon's configured URI) bypass the
// check. This is the empty-trust-rejects-everyone default that
// keeps the staging window safe.
//
// **Invariant 3 (no ambient state)**: Every method takes the
// envelope as an argument. The facade does not cache, persist, or
// observe anything across calls beyond the `Arc<RealmTrustAnchor>`
// it was constructed with. PR-7 will add a shared
// `NonceReplayStore` mutex to support replay protection; until
// then the facade is stateless modulo the trust set.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use tonic::Status;

use crate::pb::axon::v1::{Envelope, InvokeRequest, InvokeServerStreamRequest};
use crate::services::realm_trust_anchor::RealmTrustAnchor;

/// Per-RPC admission gate consulted by `DaemonInvocationService`
/// before routing into a federation wrapper or fallthrough handler.
///
/// Holds an `Arc<RealmTrustAnchor>` — the trust set authored by
/// PR-7's pairing flow and read at boot by the daemon binary.
/// Constructed once per daemon process; cloned into per-request
/// dispatcher tasks.
#[derive(Debug, Clone)]
pub struct AdmissionFacade {
    trust_anchor: Arc<RealmTrustAnchor>,
    /// Daemon's own canonical URI (from `credentials.json` per
    /// spec §5.1). Loopback callers presenting this URI bypass the
    /// trust-anchor membership check — the daemon trusts itself.
    /// `None` is permitted for tests; in that mode every external
    /// caller must be in the trust anchor.
    daemon_uri: Option<String>,
}

impl AdmissionFacade {
    /// Construct a facade against the supplied trust anchor and
    /// daemon URI. Production callers thread the daemon's
    /// `credentials.json`-derived URI through; tests typically pass
    /// `None`.
    #[must_use]
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>, daemon_uri: Option<String>) -> Self {
        Self {
            trust_anchor,
            daemon_uri,
        }
    }

    /// Verify a unary `InvokeRequest`. Returns `Ok(())` when the
    /// caller is admitted; otherwise a `tonic::Status` mapped by
    /// the rule set in `verify_envelope`.
    pub fn verify_invoke(&self, request: &InvokeRequest) -> Result<(), Status> {
        let envelope = request
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Invoke request missing envelope"))?;
        self.verify_envelope(envelope)
    }

    /// Verify a server-stream `InvokeServerStreamRequest`. Same
    /// rule set as `verify_invoke`; the differing wrapper is just
    /// the proto type.
    pub fn verify_invoke_stream(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<(), Status> {
        let envelope = request
            .envelope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("InvokeStream request missing envelope"))?;
        self.verify_envelope(envelope)
    }

    /// Core check applied to a proto `Envelope`. Public so the
    /// future bidi accept path (PR-2) can call the same rule set
    /// against the EnvelopeOpen first frame.
    pub fn verify_envelope(&self, envelope: &Envelope) -> Result<(), Status> {
        let caller_uri = envelope
            .caller
            .as_ref()
            .map(|caller| caller.uri.as_str())
            .filter(|uri| !uri.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "envelope.caller.uri is required (Invariant 1: caller URI required)",
                )
            })?;

        if let Some(daemon_uri) = self.daemon_uri.as_deref() {
            if caller_uri == daemon_uri {
                return Ok(());
            }
        }

        if self.trust_anchor.lookup(caller_uri).is_some() {
            return Ok(());
        }

        Err(Status::permission_denied(format!(
            "caller URI `{caller_uri}` is not in the realm trust anchor; \
             pairing-flow registration is the PR-7 deliverable that \
             populates the trust set",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::axon::v1::AgentIdentity;
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};

    fn agent(uri: &str) -> AgentIdentity {
        AgentIdentity {
            uri: uri.to_string(),
            ..AgentIdentity::default()
        }
    }

    fn envelope_with_caller(uri: &str) -> Envelope {
        Envelope {
            caller: Some(agent(uri)),
            ..Envelope::default()
        }
    }

    fn invoke_request(envelope: Option<Envelope>) -> InvokeRequest {
        InvokeRequest {
            envelope,
            ..InvokeRequest::default()
        }
    }

    fn entry(uri: &str) -> TrustedAgent {
        TrustedAgent {
            agent_uri: uri.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
        }
    }

    fn anchor_with(uris: &[&str]) -> Arc<RealmTrustAnchor> {
        Arc::new(
            RealmTrustAnchor::from_entries(uris.iter().map(|u| entry(u)).collect())
                .expect("test anchor"),
        )
    }

    #[test]
    fn empty_anchor_rejects_external_caller_with_permission_denied() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/r/agent/a")));
        match facade.verify_invoke(&req) {
            Err(err) => assert_eq!(err.code(), tonic::Code::PermissionDenied),
            Ok(()) => panic!("empty anchor must reject external caller"),
        }
    }

    #[test]
    fn anchor_with_caller_uri_admits_caller() {
        let anchor = anchor_with(&["easynet:///r/realm/agent/n1"]);
        let facade = AdmissionFacade::new(anchor, None);
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/agent/n1")));
        facade.verify_invoke(&req).expect("admitted");
    }

    #[test]
    fn anchor_without_caller_uri_rejects_with_permission_denied() {
        let anchor = anchor_with(&["easynet:///r/realm/agent/n1"]);
        let facade = AdmissionFacade::new(anchor, None);
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/agent/n2")));
        match facade.verify_invoke(&req) {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::PermissionDenied);
                assert!(err.message().contains("not in the realm trust anchor"));
            }
            Ok(()) => panic!("non-trusted caller must be rejected"),
        }
    }

    #[test]
    fn missing_envelope_returns_invalid_argument() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(None);
        match facade.verify_invoke(&req) {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::InvalidArgument);
                assert!(err.message().contains("missing envelope"));
            }
            Ok(()) => panic!("missing envelope must be rejected"),
        }
    }

    #[test]
    fn missing_caller_uri_returns_invalid_argument() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(Envelope::default()));
        match facade.verify_invoke(&req) {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::InvalidArgument);
                assert!(err.message().contains("caller URI required"));
            }
            Ok(()) => panic!("missing caller URI must be rejected"),
        }
    }

    #[test]
    fn empty_caller_uri_returns_invalid_argument() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);
        let req = invoke_request(Some(envelope_with_caller("")));
        match facade.verify_invoke(&req) {
            Err(err) => assert_eq!(err.code(), tonic::Code::InvalidArgument),
            Ok(()) => panic!("empty caller URI must be rejected"),
        }
    }

    #[test]
    fn daemon_uri_loopback_bypasses_empty_anchor() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/realm/agent/this-daemon".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller(
            "easynet:///r/realm/agent/this-daemon",
        )));
        facade.verify_invoke(&req).expect("daemon loopback admitted");
    }

    #[test]
    fn daemon_uri_loopback_rejects_other_callers_when_anchor_empty() {
        let facade = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/realm/agent/this-daemon".to_string()),
        );
        let req = invoke_request(Some(envelope_with_caller("easynet:///r/realm/agent/other")));
        match facade.verify_invoke(&req) {
            Err(err) => assert_eq!(err.code(), tonic::Code::PermissionDenied),
            Ok(()) => panic!("non-loopback caller must be rejected when anchor empty"),
        }
    }

    #[test]
    fn invoke_stream_uses_same_rule_set() {
        let anchor = anchor_with(&["easynet:///r/realm/agent/n1"]);
        let facade = AdmissionFacade::new(anchor, None);

        let admitted = InvokeServerStreamRequest {
            envelope: Some(envelope_with_caller("easynet:///r/realm/agent/n1")),
            ..InvokeServerStreamRequest::default()
        };
        facade.verify_invoke_stream(&admitted).expect("admitted");

        let rejected = InvokeServerStreamRequest {
            envelope: Some(envelope_with_caller("easynet:///r/realm/agent/n2")),
            ..InvokeServerStreamRequest::default()
        };
        match facade.verify_invoke_stream(&rejected) {
            Err(err) => assert_eq!(err.code(), tonic::Code::PermissionDenied),
            Ok(()) => panic!("non-trusted caller must be rejected on stream too"),
        }
    }

    #[test]
    fn verify_envelope_can_be_called_directly_for_bidi_path() {
        // PR-2 will call `verify_envelope` against the EnvelopeOpen
        // first frame of an InvokeBidi stream. Pin the surface here
        // so PR-2 reviewers see the contract.
        let anchor = anchor_with(&["easynet:///r/realm/agent/n1"]);
        let facade = AdmissionFacade::new(anchor, None);
        facade
            .verify_envelope(&envelope_with_caller("easynet:///r/realm/agent/n1"))
            .expect("admitted via direct verify_envelope");
    }
}

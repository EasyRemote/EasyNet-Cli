// EasyNet CLI — cross-hub gRPC outbound dialer (PR-N1 commit 1/N)
// =================================================================
//
// File: src/services/federation_client/cross_hub_dial.rs
//
// PR-N1 commit 1/N skeleton — `FederationClient` trait + tonic-
// backed `CrossHubDialer` skeleton + typed `FederationClientError`.
// No real outbound I/O yet; that lands in commits 2/N (TLS pin) +
// 3/N (`handle_forward_invoke` rewrite) + 4/N (timeout / circuit-
// breaker).
//
// Boot wiring (commit 3/N+) plumbs one `Arc<dyn FederationClient>`
// through `start_axon_serve_sidecar` to the `federation_wrappers`
// dispatcher so `handle_forward_invoke` can call the concrete
// dialer when target tenant != self realm. PR-N1 commit 1/N keeps
// `forward_invoke` returning `FederationClientError::DialFailed`
// so call sites can already type-check against the trait without
// the real network surface.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tonic::transport::Channel;

use crate::pb::axon::v1::{InvokeRequest, InvokeResponse};
use crate::services::realm_trust_anchor::RealmTrustAnchor;

/// Canonical hub URI string used as the federation peer key. We
/// intentionally do not introduce a newtype wrapper around
/// `String` for v1 — the URI is parsed by `tonic::transport::
/// Endpoint::from_shared` at dial time, and additional structure
/// at the type level would foreclose future URI-shape evolution
/// (DEC-N3 §"hub URI carrier").
///
/// Examples:
///   "https://hub-a.example.com:50443"
///   "https://10.0.0.7:50443"
pub type HubUri = String;

/// Outcome of a cross-hub `forward_invoke` attempt.
///
/// Each variant is a wire-stable identifier — audit pipelines and
/// metrics consumers grep on these values, so renaming any is a
/// protocol-level change that requires an RFC amendment.
#[derive(Debug, thiserror::Error)]
pub enum FederationClientError {
    /// The peer hub URI is not present in the local
    /// `RealmTrustAnchor` with the federation role + non-empty
    /// origin tenant id (DEC-N1 schema-B). Fail-closed:
    /// admission's federated trust set is the only authority on
    /// which peers we may dial.
    #[error("federation peer `{0}` is not in the realm trust anchor; cross-hub dial refused")]
    PeerNotTrusted(HubUri),

    /// `tonic::transport::Channel::connect` failed (TCP, TLS,
    /// HTTP/2 handshake — anything below the gRPC layer). The
    /// message carries the underlying tonic error verbatim so
    /// operators can grep without losing diagnostic detail.
    #[error("federation dial to `{hub}` failed: {detail}")]
    DialFailed { hub: HubUri, detail: String },

    /// The cross-hub channel exceeded the configured timeout
    /// (PR-N1 spec INV-4: 30s for `forward_invoke`, 10s for
    /// dial). Maps onto `target_offline` in the wire response so
    /// callers fall back to local cache / retry policy.
    #[error("federation channel to `{0}` timed out")]
    ChannelTimeout(HubUri),

    /// The peer hub returned a `tonic::Status` from the inner
    /// `Invoke`. Wrapping rather than collapsing preserves the
    /// peer's error code so the local caller can replay the
    /// peer's reject reason verbatim (e.g.
    /// `AXON_CALLER_SIGNATURE_INVALID` from cross-realm
    /// admission).
    #[error("federation peer `{hub}` returned: {status}")]
    InnerInvokeFailed { hub: HubUri, status: String },

    /// Circuit-breaker open — the peer has had ≥ 3 consecutive
    /// failures within the breaker window and we refuse new
    /// dials until half-open elapses. Avoids hammering an
    /// unreachable peer. Implementation lands in PR-N1 commit
    /// 4/N; commit 1/N reserves the variant.
    #[error("federation circuit-breaker open for `{0}`")]
    CircuitOpen(HubUri),
}

/// Abstract surface every `federation.forward_invoke` cross-hub
/// dispatcher consumes. Trait shape mirrors
/// `daemon_grpc::Client::Invoke` so audit pipelines and tests can
/// swap in mocks without touching call sites.
#[async_trait]
pub trait FederationClient: Send + Sync {
    /// Forward an `InvokeRequest` to `target_hub` and return its
    /// response. Implementations MUST:
    ///
    /// 1. Look up `target_hub` in the trust anchor. Reject with
    ///    `PeerNotTrusted` if the entry is missing or its
    ///    `origin_tenant_id` is `None` (DEC-N1).
    /// 2. Re-use a cached `tonic::transport::Channel` per peer
    ///    (PR-N1 spec INV-5) — fresh channel per call would
    ///    burn TLS handshakes.
    /// 3. NOT retry on `forward_invoke`. The user-facing call
    ///    has its own idempotency assumptions; only dial-level
    ///    transient failures are retried (PR-N1 commit 4/N).
    async fn forward_invoke(
        &self,
        target_hub: &HubUri,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError>;
}

/// tonic-backed concrete implementation. Holds:
/// - `trust_anchor` — read-only handle to the daemon's
///   `RealmTrustAnchor`. Used in commit 2/N for the
///   `PeerNotTrusted` gate; commit 1/N captures the field so the
///   constructor signature is stable.
/// - `channels` — `Arc<DashMap<HubUri, Channel>>` peer-channel
///   cache (PR-N1 spec INV-5). Lock-free — every commit beyond
///   1/N reads on the hot path so `RwLock<HashMap>` would be a
///   regression vs. PresenceRegistry's existing pattern.
///
/// Constructed once per daemon process at boot (alongside the
/// inbound `start_axon_serve_sidecar` listener) and cloned
/// cheaply into per-RPC dispatch tasks.
#[derive(Clone)]
pub struct CrossHubDialer {
    #[allow(dead_code)] // commit 2/N reads
    trust_anchor: Arc<RealmTrustAnchor>,
    #[allow(dead_code)] // commit 2/N populates
    channels: Arc<DashMap<HubUri, Channel>>,
}

impl std::fmt::Debug for CrossHubDialer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrossHubDialer")
            .field("cached_peers", &self.channels.len())
            .finish_non_exhaustive()
    }
}

impl CrossHubDialer {
    /// Construct a fresh dialer. PR-N1 commit 1/N: skeleton; no
    /// peer dial occurs until commit 2/N. The dialer's behaviour
    /// in this commit is "every call returns
    /// `FederationClientError::DialFailed("not implemented in PR-N1
    /// commit 1/N")`". The constructor + cache + types ship now
    /// so commit 3/N can wire the dispatcher against a stable
    /// trait surface without forward-references.
    #[must_use]
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>) -> Self {
        Self {
            trust_anchor,
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Number of cached peer channels. Test/observability only.
    #[must_use]
    pub fn cached_peer_count(&self) -> usize {
        self.channels.len()
    }
}

#[async_trait]
impl FederationClient for CrossHubDialer {
    async fn forward_invoke(
        &self,
        target_hub: &HubUri,
        _request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError> {
        // PR-N1 commit 1/N intentionally returns the typed
        // "skeleton" error. commit 2/N replaces this body with
        // real TLS-pinned dial + cached channel reuse + inner
        // tonic Invoke. The error variant + message are chosen so
        // operators / tests can grep on the literal string
        // "not implemented in PR-N1 commit 1/N" if a stray call
        // hits this code path during the rollout window.
        Err(FederationClientError::DialFailed {
            hub: target_hub.clone(),
            detail:
                "not implemented in PR-N1 commit 1/N — real cross-hub dial \
                 lands in commit 2/N (TLS pin) + 3/N (handle_forward_invoke \
                 rewrite). See pr-drafts/PR-N1-spec-hub-to-hub-grpc-outbound.md."
                    .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Trait-shape pin tests for PR-N1 commit 1/N. The dialer's
    //! real I/O is exercised in commit 2/N+ tests; this module
    //! only proves the trait API + types compile cleanly and
    //! that a `MockFederationClient` can be substituted for the
    //! concrete `CrossHubDialer` so call-site tests in commit
    //! 3/N (`handle_forward_invoke` rewrite) can stand on a
    //! stable foundation.

    use super::*;
    use crate::pb::axon::v1::{InvocationState, ResponseHeader};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Test-only canned-response client. Lookup key is
    /// `(target_hub, request.function_name)` so a single mock
    /// can answer different abilities differently. Calls not in
    /// the canned set return `FederationClientError::DialFailed`
    /// — the same variant the real skeleton would return, so
    /// the dispatcher under test cannot tell apart "no canned
    /// response set" from "real dialer not yet wired".
    pub(super) struct MockFederationClient {
        canned: Mutex<HashMap<(HubUri, String), InvokeResponse>>,
    }

    impl MockFederationClient {
        pub(super) fn new() -> Self {
            Self {
                canned: Mutex::new(HashMap::new()),
            }
        }

        pub(super) fn insert(
            &self,
            target_hub: HubUri,
            function_name: &str,
            response: InvokeResponse,
        ) {
            self.canned
                .lock()
                .expect("mock canned map poisoned")
                .insert((target_hub, function_name.to_string()), response);
        }
    }

    #[async_trait]
    impl FederationClient for MockFederationClient {
        async fn forward_invoke(
            &self,
            target_hub: &HubUri,
            request: InvokeRequest,
        ) -> Result<InvokeResponse, FederationClientError> {
            let key = (target_hub.clone(), request.function_name);
            self.canned
                .lock()
                .expect("mock canned map poisoned")
                .get(&key)
                .cloned()
                .ok_or_else(|| FederationClientError::DialFailed {
                    hub: target_hub.clone(),
                    detail: "MockFederationClient: no canned response".to_string(),
                })
        }
    }

    fn empty_anchor() -> Arc<RealmTrustAnchor> {
        Arc::new(RealmTrustAnchor::default())
    }

    fn sample_request(function_name: &str) -> InvokeRequest {
        InvokeRequest {
            function_name: function_name.to_string(),
            ..InvokeRequest::default()
        }
    }

    fn sample_response() -> InvokeResponse {
        InvokeResponse {
            header: Some(ResponseHeader {
                status: "completed".to_string(),
                ..ResponseHeader::default()
            }),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        }
    }

    #[tokio::test]
    async fn skeleton_dialer_returns_typed_dial_failed_with_commit_marker() {
        // PR-N1 commit 1/N contract: skeleton returns the typed
        // "not implemented" error with the literal commit marker
        // string so a stray call during rollout is debuggable
        // by grep, not by guessing.
        let dialer = CrossHubDialer::new(empty_anchor());
        let target = "https://peer-hub.example:50443".to_string();
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("commit 1/N skeleton must not succeed");
        match err {
            FederationClientError::DialFailed { hub, detail } => {
                assert_eq!(hub, target);
                assert!(
                    detail.contains("not implemented in PR-N1 commit 1/N"),
                    "skeleton error must carry the rollout marker; got: {detail}"
                );
            }
            other => panic!("expected DialFailed, got: {other:?}"),
        }
    }

    #[test]
    fn dialer_starts_with_zero_cached_peer_channels() {
        let dialer = CrossHubDialer::new(empty_anchor());
        assert_eq!(dialer.cached_peer_count(), 0);
    }

    #[test]
    fn dialer_clone_shares_channel_cache() {
        // PR-N1 spec INV-5: the channel cache is process-wide,
        // not per-clone. Two clones must observe the same
        // backing DashMap so the eventual commit 2/N TLS-pinned
        // channel inserted on one clone is visible to admission
        // RPCs holding a different clone.
        let dialer_a = CrossHubDialer::new(empty_anchor());
        let dialer_b = dialer_a.clone();
        // We can't insert a real `Channel` here without a tonic
        // endpoint, but we can check the Arc identity by
        // comparing pointer equality on the underlying DashMap.
        assert!(
            Arc::ptr_eq(&dialer_a.channels, &dialer_b.channels),
            "clones must share the channel cache by Arc identity"
        );
    }

    #[tokio::test]
    async fn mock_client_returns_canned_response_when_present() {
        let mock = MockFederationClient::new();
        let target = "https://peer-hub.example:50443".to_string();
        mock.insert(target.clone(), "test.echo", sample_response());

        let resp = mock
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect("canned response delivered");
        assert_eq!(resp.state, InvocationState::Completed as i32);
        assert_eq!(
            resp.header.as_ref().expect("header present").status,
            "completed"
        );
    }

    #[tokio::test]
    async fn mock_client_dial_failed_when_no_canned_response() {
        let mock = MockFederationClient::new();
        let target = "https://peer-hub.example:50443".to_string();
        let err = mock
            .forward_invoke(&target, sample_request("never.canned"))
            .await
            .expect_err("missing canned response must surface as DialFailed");
        match err {
            FederationClientError::DialFailed { hub, .. } => assert_eq!(hub, target),
            other => panic!("expected DialFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_client_canned_lookup_keyed_by_function_name() {
        // The canned map keys on `(hub, function_name)` so one
        // mock can answer multiple abilities differently. Pin
        // the contract so commit 3/N tests against this mock
        // can rely on the keying.
        let mock = MockFederationClient::new();
        let target = "https://peer-hub.example:50443".to_string();

        let mut completed_resp = sample_response();
        completed_resp.result = b"echo-payload".to_vec();
        let mut other_resp = sample_response();
        other_resp.result = b"other-payload".to_vec();

        mock.insert(target.clone(), "test.echo", completed_resp.clone());
        mock.insert(target.clone(), "test.other", other_resp.clone());

        let r1 = mock
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect("echo canned");
        let r2 = mock
            .forward_invoke(&target, sample_request("test.other"))
            .await
            .expect("other canned");
        assert_eq!(r1.result, b"echo-payload");
        assert_eq!(r2.result, b"other-payload");
    }
}

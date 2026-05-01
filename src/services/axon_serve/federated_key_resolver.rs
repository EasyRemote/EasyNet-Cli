// EasyNet CLI — FederatedKeyResolver (PR-N2 commit 1/N)
// =======================================================
//
// File: src/services/axon_serve/federated_key_resolver.rs
// Description: KeyResolver wrapping the local TrustAnchorKeyResolver
//              with cross-realm fall-through to a peer hub's
//              `federation.resolve_key` ability via the PR-N1
//              CrossHubDialer.
//
// Why this resolver exists
// ------------------------
// PR-7 commit 4/N's TrustAnchorKeyResolver only knows about
// agents in the local realm's `realm-trust.toml`. Cross-realm
// callers (a device in realm A signing an envelope that lands
// at hub B) hit `unknown_agent_uri` and admission rejects with
// `caller_signature_invalid` regardless of whether the
// signature is correct, because hub B has no way to fetch
// hub A's pubkey for verification.
//
// PR-N2 closes that gap. FederatedKeyResolver:
//
//   1. Tries the local trust anchor first (INV-2 local-first).
//      Same-realm callers short-circuit before any network I/O.
//   2. On local miss, parses the caller URI's tenant and checks
//      the `realm-trust.toml` for a `[[trusted_agent]]` entry
//      whose `origin_tenant_id` matches that tenant (INV-1
//      federated trust gate). Operators explicitly opt into
//      cross-realm verification by adding such entries.
//   3. Calls `federation.resolve_key` on the peer hub via the
//      PR-N1 CrossHubDialer, returning the peer's pubkey for
//      the caller URI (INV-3 same Ed25519 4-step verify).
//   4. Dial failure surfaces as `unknown_agent_uri` so the
//      admission gate's reject path runs identically to a
//      local trust miss (INV-4 fail-closed).
//
// Sync trait shape
// ----------------
// `easynet_axon::invocation::axiom::KeyResolver::resolve` is a
// sync method (called from inside the sync `run_admission`
// pipeline). The cross-hub dial is async (tonic). We bridge via
// `tokio::task::block_in_place` + `Handle::current().block_on`
// — safe because the admission gate runs on a `tokio::spawn`
// task in the gRPC server, and `block_in_place` is the
// canonical pattern when a sync call needs to await inside an
// async runtime.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::VerifyingKey;
use easynet_axon::invocation::axiom::KeyResolver;
use easynet_axon::invocation::{AxonError, AxonErrorKind};

use crate::pb::axon::v1::{Envelope, InvokeRequest};
use crate::services::federation_client::FederationClient;
use crate::services::realm_trust_anchor::RealmTrustAnchor;

/// Resolves an `agent_uri` to its Ed25519 verifying key, falling
/// through to a federated lookup when the local trust anchor has
/// no entry for the URI and the caller's tenant is one the
/// operator has marked as federated via DEC-N1 schema-B
/// `origin_tenant_id` on a `[[trusted_agent]]` entry.
pub struct FederatedKeyResolver {
    trust_anchor: Arc<RealmTrustAnchor>,
    federation_client: Option<Arc<dyn FederationClient>>,
    federated_peers: Arc<std::collections::BTreeMap<String, String>>,
    self_realm: Option<String>,
}

impl FederatedKeyResolver {
    /// Construct a resolver that always tries the local trust
    /// anchor first, then federation. The federation client is
    /// optional — daemons booted without one (device mode, or
    /// hub-mode pre-PR-N1-commit-6) get a local-only resolver
    /// equivalent to PR-7's `TrustAnchorKeyResolver`.
    #[must_use]
    pub fn new(
        trust_anchor: Arc<RealmTrustAnchor>,
        federation_client: Option<Arc<dyn FederationClient>>,
        federated_peers: Arc<std::collections::BTreeMap<String, String>>,
        self_realm: Option<String>,
    ) -> Self {
        Self {
            trust_anchor,
            federation_client,
            federated_peers,
            self_realm,
        }
    }

    /// Local-first lookup. Mirrors `TrustAnchorKeyResolver` shape
    /// so existing single-realm setups behave identically.
    fn resolve_local(&self, agent_uri: &str) -> Result<VerifyingKey, AxonError> {
        let entry = self.trust_anchor.lookup(agent_uri).ok_or_else(|| {
            AxonError::new(AxonErrorKind::InvalidArgument).with_reason("unknown_agent_uri")
                .with_message(format!("agent_uri:{agent_uri}"))
        })?;
        let raw = BASE64_STANDARD.decode(&entry.public_key_b64).map_err(|e| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("public_key_b64_decode_failed")
                .with_message(format!("agent_uri:{agent_uri}:{e}"))
        })?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("public_key_wrong_length")
                .with_message(format!(
                    "agent_uri:{agent_uri}:expected_32_got_{}",
                    raw.len()
                ))
        })?;
        VerifyingKey::from_bytes(&arr).map_err(|e| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("public_key_parse_failed")
                .with_message(format!("agent_uri:{agent_uri}:{e}"))
        })
    }

    /// Cross-realm fall-through. Decision tree per spec §commit 2/N:
    ///
    /// - `caller_tenant == self_realm` → local-only; do NOT dial
    ///   federated. Local miss is final.
    /// - operator did NOT mark caller_tenant as federated (no
    ///   `[[trusted_agent]] origin_tenant_id = "<tenant>"` entry
    ///   in `realm-trust.toml`) → local-only.
    /// - `federated_peers` map has no entry mapping
    ///   `caller_tenant → hub_uri` → cannot dial; return
    ///   unknown_agent_uri.
    /// - dial fails → unknown_agent_uri (INV-4 fail-closed).
    ///
    /// Returns `Ok(VerifyingKey)` only when the cross-hub resolve
    /// returns a valid base64 Ed25519 pubkey for the caller.
    fn resolve_federated(&self, agent_uri: &str) -> Result<VerifyingKey, AxonError> {
        let Some(client) = self.federation_client.as_ref() else {
            return Err(unknown_agent_uri(agent_uri, "no_federation_client"));
        };

        let caller_tenant =
            crate::services::axon_serve::daemon_invocation_service::parse_tenant_from_uri(
                agent_uri,
            )
            .ok_or_else(|| unknown_agent_uri(agent_uri, "malformed_uri"))?;

        // INV-1 federated trust gate: same-realm caller's local
        // miss is final. Returning unknown_agent_uri here is the
        // same surface as a normal trust-anchor miss for a local
        // URI — the admission gate emits
        // AXON_CALLER_SIGNATURE_INVALID, which is the right
        // operator signal ("the URI is not trusted in this
        // realm").
        if let Some(self_realm) = self.self_realm.as_deref() {
            if caller_tenant == self_realm {
                return Err(unknown_agent_uri(agent_uri, "same_realm_local_miss"));
            }
        }

        // INV-1 second clause: the operator must have explicitly
        // marked the caller's tenant as federated. This requires
        // EITHER a `[[trusted_agent]]` entry whose
        // `origin_tenant_id` matches the caller's tenant (the
        // canonical schema-B path) OR a `[daemon.federated_peers]`
        // entry for that tenant. Both signal "I trust this
        // tenant's hub". We accept both because the operator
        // workflow may have populated only one (e.g. via
        // `easynet join` auto-wire, which writes to
        // federated_peers but not to realm-trust.toml).
        let trust_entry_marked = self
            .trust_anchor
            .entries_sorted()
            .into_iter()
            .any(|e| e.origin_tenant_id.as_deref() == Some(caller_tenant));
        let peer_entry = self.federated_peers.get(caller_tenant);
        if !trust_entry_marked && peer_entry.is_none() {
            return Err(unknown_agent_uri(agent_uri, "tenant_not_federated"));
        }

        let Some(peer_hub_uri) = peer_entry else {
            return Err(unknown_agent_uri(agent_uri, "no_hub_uri_for_tenant"));
        };

        // Build the cross-hub `federation.resolve_key` request.
        // The peer-side ability handler is a thin RFC-002 wrap
        // around its local trust anchor; we forward the caller
        // URI verbatim and parse the response as a JSON
        // `{"public_key_b64": "<base64-32-bytes>"}` shape.
        let args = serde_json::json!({ "agent_uri": agent_uri });
        let args_bytes = serde_json::to_vec(&args).map_err(|e| {
            AxonError::new(AxonErrorKind::Internal)
                .with_reason("resolve_key_args_encode")
                .with_message(format!("agent_uri:{agent_uri}:{e}"))
        })?;
        let request = InvokeRequest {
            envelope: Some(Envelope::default()),
            function_name: "federation.resolve_key".to_string(),
            arguments: args_bytes,
            ..InvokeRequest::default()
        };

        // Bridge sync trait → async tonic call.
        let target_hub = peer_hub_uri.clone();
        let client_clone = Arc::clone(client);
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { client_clone.forward_invoke(&target_hub, request).await })
        })
        .map_err(|err| unknown_agent_uri(agent_uri, &format!("dial_failed:{err}")))?;

        let parsed: serde_json::Value = serde_json::from_slice(&response.result).map_err(|e| {
            unknown_agent_uri(agent_uri, &format!("resolve_key_response_parse:{e}"))
        })?;
        let pk_b64 = parsed
            .get("public_key_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                unknown_agent_uri(agent_uri, "resolve_key_response_missing_pubkey")
            })?;
        let raw = BASE64_STANDARD.decode(pk_b64).map_err(|e| {
            unknown_agent_uri(agent_uri, &format!("resolve_key_pubkey_b64_decode:{e}"))
        })?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            unknown_agent_uri(
                agent_uri,
                &format!("resolve_key_pubkey_wrong_length:{}", raw.len()),
            )
        })?;
        VerifyingKey::from_bytes(&arr)
            .map_err(|e| unknown_agent_uri(agent_uri, &format!("resolve_key_pubkey_parse:{e}")))
    }
}

impl KeyResolver for FederatedKeyResolver {
    fn resolve(&self, agent_uri: &str) -> Result<VerifyingKey, AxonError> {
        // Local-first per INV-2.
        match self.resolve_local(agent_uri) {
            Ok(key) => Ok(key),
            Err(_) => self.resolve_federated(agent_uri),
        }
    }
}

/// Wrap a federated-resolve failure as the same wire-shape the
/// local trust-miss path emits, so the admission gate's reject
/// reason is `AXON_CALLER_SIGNATURE_INVALID` regardless of
/// whether the URI was unknown locally or unreachable cross-
/// realm. Operators reading the reject log see the failure
/// detail in the AxonError message field.
fn unknown_agent_uri(agent_uri: &str, detail: &str) -> AxonError {
    AxonError::new(AxonErrorKind::InvalidArgument).with_reason("unknown_agent_uri")
        .with_message(format!("agent_uri:{agent_uri}:{detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
    use ed25519_dalek::SigningKey;
    use std::collections::BTreeMap;

    fn ed25519_pubkey_b64() -> (SigningKey, String) {
        // Deterministic test key (zero seed). Acceptable because
        // these tests verify resolution wire shape, not
        // cryptographic strength.
        let signing = SigningKey::from_bytes(&[1u8; 32]);
        let pk_bytes = signing.verifying_key().to_bytes();
        (signing, BASE64_STANDARD.encode(pk_bytes))
    }

    fn local_entry(uri: &str, pk_b64: &str) -> TrustedAgent {
        TrustedAgent {
            agent_uri: uri.to_string(),
            public_key_b64: pk_b64.to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        }
    }

    /// Test-only `FederationClient` that returns a canned JSON
    /// response. The dispatcher under test calls
    /// `federation.resolve_key` against this client and parses
    /// the result the same way it parses a real peer response.
    struct CannedFederationClient {
        canned_response: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl FederationClient for CannedFederationClient {
        async fn forward_invoke(
            &self,
            _target_hub: &crate::services::federation_client::HubUri,
            _request: InvokeRequest,
        ) -> Result<
            crate::pb::axon::v1::InvokeResponse,
            crate::services::federation_client::FederationClientError,
        > {
            Ok(crate::pb::axon::v1::InvokeResponse {
                result: self.canned_response.clone(),
                ..Default::default()
            })
        }
    }

    /// Test-only client that always errors out, simulating a
    /// peer hub that's down or breaker-open.
    struct DialFailedClient;

    #[async_trait::async_trait]
    impl FederationClient for DialFailedClient {
        async fn forward_invoke(
            &self,
            target_hub: &crate::services::federation_client::HubUri,
            _request: InvokeRequest,
        ) -> Result<
            crate::pb::axon::v1::InvokeResponse,
            crate::services::federation_client::FederationClientError,
        > {
            Err(crate::services::federation_client::FederationClientError::DialFailed {
                hub: target_hub.clone(),
                detail: "test-injected failure".to_string(),
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_hit_short_circuits_before_federated_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let local_uri = "easynet:///r/realm-a/agent/local-device";
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![local_entry(local_uri, &pk_b64)]).unwrap(),
        );

        // Wire a dial-failed client; if the resolver tried to
        // dial we'd get an error. The local-first short-circuit
        // means the dial must NOT happen.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        );

        let key = resolver.resolve(local_uri).expect("local hit");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_with_federated_peers_entry_resolves_via_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_uri = "easynet:///r/realm-b/agent/peer-device";

        // Local trust anchor has NO entry for cross_uri.
        let anchor = Arc::new(RealmTrustAnchor::default());

        // Operator wired a federated_peers entry for realm-b.
        // This is the post-`easynet join` shape (Track 3).
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        // Peer hub returns the canned pubkey.
        let response_json = serde_json::json!({
            "public_key_b64": pk_b64,
        });
        let client: Arc<dyn FederationClient> = Arc::new(CannedFederationClient {
            canned_response: serde_json::to_vec(&response_json).unwrap(),
        });

        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let key = resolver.resolve(cross_uri).expect("federated hit");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_without_federation_marker_returns_unknown() {
        let cross_uri = "easynet:///r/realm-b/agent/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());

        // No federated_peers entry, no origin_tenant_id-marked
        // trust entry. The resolver MUST NOT dial — operator did
        // not opt into cross-realm resolution for realm-b.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        );

        let err = resolver.resolve(cross_uri).expect_err("unmarked");
        assert!(
            format!("{err:?}").contains("unknown_agent_uri"),
            "expected unknown_agent_uri, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_dial_failure_surfaces_as_unknown() {
        let cross_uri = "easynet:///r/realm-b/agent/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        // INV-4 fail-closed: dial failure → unknown_agent_uri,
        // NOT a silent local fall-through.
        let err = resolver.resolve(cross_uri).expect_err("dial fail");
        assert!(
            format!("{err:?}").contains("unknown_agent_uri"),
            "expected unknown_agent_uri, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_realm_local_miss_does_not_dial_federated() {
        let same_realm_uri = "easynet:///r/realm-a/agent/missing-device";
        let anchor = Arc::new(RealmTrustAnchor::default());

        // Federated peer wired, but the missing URI is in the
        // SAME realm as `self_realm` — INV-1 says local miss is
        // final, do not federate.
        let mut peers = BTreeMap::new();
        peers.insert("realm-a".to_string(), "https://hub-a:50443".to_string());

        // DialFailedClient ensures any accidental federated dial
        // would surface as a different error variant.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let err = resolver.resolve(same_realm_uri).expect_err("same-realm miss");
        let err_str = format!("{err:?}");
        assert!(err_str.contains("unknown_agent_uri"));
        // The dial must NOT have fired; the failure detail
        // should reflect a same-realm-local-miss, not
        // dial_failed.
        assert!(
            err_str.contains("same_realm_local_miss"),
            "expected same_realm_local_miss in detail, got {err_str}"
        );
    }
}

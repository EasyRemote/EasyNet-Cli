// EasyNet CLI — federation_client (cross-hub federation transport)
// =================================================================
//
// File: src/services/federation_client/mod.rs
//
// PR-N1 commit 1/N (RFC-N hub-to-hub federation, master plan
// `letters/2026-05-01-49-haifeng-to-cto-mohao-liangbing-RFC-N-implement-master-plan.md`).
// The cross-hub outbound dial primitive — counterpart to PR-10
// commit 1/N's inbound TCP+TLS listener (`ee7123c`). When `realm A`
// receives a `federation.forward_invoke` for a target whose tenant
// belongs to a peer realm, it dials the peer hub via this module
// instead of returning the existing stub `target_online: false`.
//
// What lives here
// ---------------
// - `FederationClient` trait — the abstract surface
//   `invocation_transport::federation_wrappers::handle_forward_invoke` will
//   consume in PR-N1 commit 3/N. Sync trait method shape mirrors
//   `daemon_grpc::Client::Invoke` so the federation client can be
//   swapped for tests + future protocol versions without touching
//   call sites.
// - `CrossHubDialer` — the concrete tonic-backed implementation.
//   commit 1/N (this commit) lands the skeleton: channel cache
//   (`Arc<DashMap<HubUri, Channel>>`), constructors, typed errors,
//   and a `forward_invoke` body that returns
//   `FederationClientError::DialFailed("not implemented in PR-N1
//   commit 1/N")`. Real TLS pinning + dial lands in commit 2/N;
//   real `handle_forward_invoke` integration lands in commit 3/N;
//   timeout / circuit-breaker lands in commit 4/N.
// - `MockFederationClient` — `#[cfg(test)]`-only fixture that
//   returns canned `InvokeResponse` per `(target_hub, ability)`
//   pair. Lets PR-N1 commit 3/N's `handle_forward_invoke` rewrite
//   land its tests against a known-good `FederationClient` impl
//   without spawning a peer daemon.
//
// What this commit does NOT do
// ----------------------------
// - No real outbound I/O. `forward_invoke` returns the typed
//   "not implemented yet" error; tests assert the trait shape and
//   the dialer's constructor, not real TCP dial.
// - No `realm_trust_anchor.rs` schema-B `origin_realm` field
//   yet. Per PR-N1 spec §commit 2/N the field lands alongside TLS
//   pinning. PR-N1 commit 1/N treats `realm_trust_anchor` as
//   read-only and reserves the schema bump for commit 2/N.
// - No `DaemonConfig::federated_peers` map. Per PR-N1 spec
//   §commit 3/N the operator-configured `tenant → hub_endpoint` mapping
//   lands when the real handler rewrite needs it. PR-N1 commit 1/N
//   exposes the trait against a hub URI provided directly by the
//   caller; mapping resolution belongs upstream.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod cross_hub_dial;
pub mod peer_dial;

pub use cross_hub_dial::{
    CrossHubDialer, DirectoryEventStream, FederationClient, FederationClientError, HubUri,
};
pub use peer_dial::{pinned_tls_config, PinnedTlsError};

// EasyNet CLI — HubResolver
// =========================
//
// File: src/daemon/invocation/hub_resolver.rs
// Description: Resolves a remote execution target to the peer hub endpoint
//              used by canonical `Invocation::Invoke` transport.
//
// Why this module exists
// ----------------------
// Canonical invocation dispatch admits the caller, signs the peer envelope,
// and dials the selected peer. *How* the daemon picks the
// peer hub URA is a routing policy, and that policy must have one
// authority: operator-curated `federated_peers`.
//
// Embedding that growing chain as a nested `match` in the dispatch
// hot path produces three observable failure modes:
//
//   1. Every new source bloats the `match` arm with another
//      `eprintln!`/error branch.
//   2. Unit-testing routing forces tests to spin up the full
//      `DaemonInvocationService` builder, even when they only care
//      about the resolver's decision matrix.
//   3. Sources cannot share precedence rules (e.g. "always prefer
//      the operator-curated entry over directory observation") in
//      one place; the precedence is implicit in match-arm order.
//
// [`HubResolver`] inverts the dependency: it takes the two cells
// (static peers, federated directory) and returns a typed outcome.
// The dispatcher hands it the target identifiers and consumes the
// outcome. A new source becomes a new method or a new arm *inside*
// the resolver, not a new branch in the dispatcher.
//
// Precedence contract
// -------------------
// Source contract:
//
//   Static `federated_peers` (operator-declared in `daemon-config.toml`,
//   hot-reloadable via SIGHUP) is the only peer-hub dispatch endpoint
//   authority.
//
// Federated directory entries remain read-model observations. They are not
// route authority and are never used to synthesize a peer-hub endpoint for
// invocation dispatch.
//
// Why directory observations do not route
// ---------------------------------------
// `federated_directory.hub_endpoint` is a string the peer hub
// published about itself during hub-to-hub sync. Treating that observation
// as a dispatch endpoint creates a second route authority and lets a peer
// redirect outbound federation transport. Canonical runtime convergence keeps
// directory sync as observability only; dispatch uses only operator intent.
//
// **Threat model**: an attacker who controls any peer hub our directory sync
// trusts can stamp a malicious `hub_endpoint` into their snapshot. Because
// dispatch never consults the directory endpoint, the attacker can at most
// affect directory read-model freshness; they cannot redirect Invocation
// transport.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::daemon::federation::peers::SharedFederatedPeers;

/// Outcome of [`HubResolver::resolve`].
///
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubResolution {
    /// Static `federated_peers` entry — operator declared this
    /// `realm → hub_endpoint` mapping in `daemon-config.toml`.
    Static { hub_endpoint: String },
    /// Neither source knows where to dial. The caller must surface
    /// the standard `target_offline` outcome.
    Offline,
}

impl HubResolution {
    /// Convenience accessor for call sites that only need the URA
    /// and have already decided how to react to the variant.
    #[must_use]
    pub fn hub_endpoint(&self) -> Option<&str> {
        match self {
            HubResolution::Static { hub_endpoint } => Some(hub_endpoint),
            HubResolution::Offline => None,
        }
    }
}

/// Holds the operator-owned routing source. Cheap to construct per-call: the
/// source is an `Arc`-backed cell. Borrows the cell rather than owning it so
/// the dispatcher's existing hot-path access pattern (snapshot per call so
/// SIGHUP reloads are visible) is preserved unchanged.
pub struct HubResolver<'a> {
    static_peers: &'a SharedFederatedPeers,
}

impl<'a> HubResolver<'a> {
    #[must_use]
    pub fn new(static_peers: &'a SharedFederatedPeers) -> Self {
        Self { static_peers }
    }

    /// Pick a hub endpoint for `target_realm` from the operator-curated peer
    /// map. The caller decides what to do with the result; `Offline` is not
    /// coerced into an `Err` here so the dispatch service can attach its
    /// existing receipt-emission side effect on the offline branch.
    #[must_use]
    pub fn resolve(&self, target_realm: &str) -> HubResolution {
        let peers_snapshot = self.static_peers.snapshot();
        if let Some(ura) = peers_snapshot.get(target_realm) {
            return HubResolution::Static {
                hub_endpoint: ura.clone(),
            };
        }

        HubResolution::Offline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn static_peer_resolves_from_operator_map() {
        let mut peers = BTreeMap::new();
        peers.insert("realm-x".to_string(), "https://primary.example".to_string());
        let static_peers = SharedFederatedPeers::new(peers);

        let resolver = HubResolver::new(&static_peers);
        let outcome = resolver.resolve("realm-x");

        assert_eq!(
            outcome,
            HubResolution::Static {
                hub_endpoint: "https://primary.example".to_string(),
            }
        );
    }

    #[test]
    fn static_miss_returns_offline() {
        let static_peers = SharedFederatedPeers::default();

        let resolver = HubResolver::new(&static_peers);
        let outcome = resolver.resolve("realm-y");

        assert_eq!(outcome, HubResolution::Offline);
    }
}

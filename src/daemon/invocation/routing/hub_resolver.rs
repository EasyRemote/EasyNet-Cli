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
// peer hub URA is a routing policy, and a routing policy that has
// grown from one source (operator-curated `federated_peers`) to two
// (operator-curated map + observed federation directory) and will
// grow to three when capability-aware routing lands.
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
// Sources are consulted in this order; the first hit wins:
//
//   1. Static `federated_peers` (operator-declared in
//      `daemon-config.toml`, hot-reloadable via SIGHUP). Operator
//      intent is authoritative.
//   2. Federated directory entry with `hub_endpoint` populated
//      (observed via PR-N3 directory sync). Auto-route fallback —
//      **only consulted when [`HubResolver::allow_directory_fallback`]
//      is `true`**.
//
// Static entries take precedence over directory observations so
// operators retain control: if `daemon-config.toml` pins
// `realm-x → https://primary.example`, a directory-observed
// `https://backup.example` for the same realm never overrides it.
//
// Why directory fallback is opt-in (default-off)
// ----------------------------------------------
// `federated_directory.hub_endpoint` is a string the peer hub
// published about itself during hub-to-hub sync. Until the
// directory transport layer ratchets to "endpoints flow only from
// authenticated peers", trusting an arbitrary observed endpoint
// would let a compromised or impersonating peer redirect our
// outbound federation client. Operators who do trust the directory
// (e.g. closed-realm deployments where every hub is operator-owned)
// set `[daemon] allow_directory_auto_route = true` in
// `daemon-config.toml`; the boot path threads that flag into
// `HubResolver`. The default refuses to dial anything that was not
// explicitly declared in `federated_peers`.
//
// **Threat model the opt-in addresses**: an attacker who controls
// any peer hub our directory sync trusts can stamp a malicious
// `hub_endpoint` into their snapshot. With the default
// (`allow_directory_fallback = false`) we never look at that field
// and dispatch falls back to `target_offline` — the attacker can
// at best DoS routing for realms the operator never wired by
// hand, which is already the legacy behaviour.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::daemon::federation::directory::{
    lookup_in_federated_view, SharedFederatedDirectoryView,
};
use crate::daemon::federation::peers::SharedFederatedPeers;

/// Outcome of [`HubResolver::resolve`].
///
/// Distinguishing static-vs-fallback at the type level is
/// load-bearing: operators monitoring `federated_peers_miss`
/// telemetry need to know "did the operator-declared map carry
/// this realm, or did we fall through to directory observation."
/// A single `Option<String>` collapses the two cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubResolution {
    /// Static `federated_peers` entry — operator declared this
    /// `realm → hub_endpoint` mapping in `daemon-config.toml`.
    Static { hub_endpoint: String },
    /// Federated directory fallback — no static entry; the URA
    /// comes from the directory sync's observation of this device
    /// on a peer hub. `target_ura` is included so the caller can
    /// emit a telemetry event with the exact device that triggered
    /// the auto-route.
    DirectoryFallback {
        hub_endpoint: String,
        target_ura: String,
    },
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
            HubResolution::Static { hub_endpoint }
            | HubResolution::DirectoryFallback { hub_endpoint, .. } => Some(hub_endpoint),
            HubResolution::Offline => None,
        }
    }
}

/// Holds references to the two routing sources plus the opt-in
/// flag that gates the directory-fallback source. Cheap to
/// construct per-call: each cell field is an `Arc` clone. Borrows
/// the cells rather than owning them so the dispatcher's existing
/// hot-path access pattern (snapshot per call so SIGHUP reloads
/// are visible) is preserved unchanged.
pub struct HubResolver<'a> {
    static_peers: &'a SharedFederatedPeers,
    federated_directory: &'a SharedFederatedDirectoryView,
    /// When `false`, the resolver never consults
    /// `federated_directory` and returns [`HubResolution::Offline`]
    /// whenever the static-peers map has no entry. See the module
    /// header for the threat model. Wired from
    /// `DaemonConfig::allow_directory_auto_route()` at boot.
    allow_directory_fallback: bool,
}

impl<'a> HubResolver<'a> {
    /// Construct a resolver. `allow_directory_fallback = false` is
    /// the secure default and is what callers should pass unless
    /// the daemon's `[daemon] allow_directory_auto_route = true`.
    #[must_use]
    pub fn new(
        static_peers: &'a SharedFederatedPeers,
        federated_directory: &'a SharedFederatedDirectoryView,
        allow_directory_fallback: bool,
    ) -> Self {
        Self {
            static_peers,
            federated_directory,
            allow_directory_fallback,
        }
    }

    /// Pick a hub endpoint for `(target_realm, target_ura)` by
    /// consulting each source in the precedence order documented
    /// in the module header. The caller decides what to do with
    /// the result; `Offline` is not coerced into an `Err` here so
    /// the dispatch service can attach its existing receipt-emission
    /// side effect on the offline branch.
    #[must_use]
    pub fn resolve(&self, target_realm: &str, target_ura: &str) -> HubResolution {
        // Source 1: static, operator-declared. Always consulted.
        let peers_snapshot = self.static_peers.snapshot();
        if let Some(ura) = peers_snapshot.get(target_realm) {
            return HubResolution::Static {
                hub_endpoint: ura.clone(),
            };
        }

        // Source 2: federated directory observation — only when
        // explicitly enabled by the operator. The default is the
        // secure shape: no static entry → `Offline`, regardless of
        // what the directory has cached.
        if self.allow_directory_fallback {
            if let Some(endpoint) = lookup_in_federated_view(self.federated_directory, target_ura)
                .and_then(|entry| entry.hub_endpoint)
            {
                return HubResolution::DirectoryFallback {
                    hub_endpoint: endpoint,
                    target_ura: target_ura.to_string(),
                };
            }
        }

        HubResolution::Offline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::federation::directory::{DirectoryEntry, DirectoryView};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn directory_with_entry(
        peer_realm: &str,
        agent_ura: &str,
        hub_endpoint: Option<&str>,
    ) -> SharedFederatedDirectoryView {
        let cell = SharedFederatedDirectoryView::default();
        let mut view = DirectoryView::new(peer_realm.to_string());
        view.replace_entries(vec![DirectoryEntry {
            agent_ura: agent_ura.to_string(),
            node_id: "node-x".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: hub_endpoint.map(str::to_string),
            last_seen_unix_ms: Some(1_700_000_000_000),
        }]);
        let mut peers = BTreeMap::new();
        peers.insert(peer_realm.to_string(), Arc::new(view));
        cell.replace(peers);
        cell
    }

    #[test]
    fn static_peer_wins_over_directory_observation_even_when_fallback_enabled() {
        // Both sources carry the realm; static must win so operator
        // intent is authoritative. Even with `allow_directory_fallback
        // = true` the directory must not override an operator pin.
        let mut peers = BTreeMap::new();
        peers.insert("realm-x".to_string(), "https://primary.example".to_string());
        let static_peers = SharedFederatedPeers::new(peers);
        let directory = directory_with_entry(
            "realm-x",
            "easynet:///r/realm-x/device/d1",
            Some("https://backup.example"),
        );

        let resolver = HubResolver::new(&static_peers, &directory, true);
        let outcome = resolver.resolve("realm-x", "easynet:///r/realm-x/device/d1");

        assert_eq!(
            outcome,
            HubResolution::Static {
                hub_endpoint: "https://primary.example".to_string(),
            }
        );
    }

    #[test]
    fn directory_fills_in_when_static_missing_and_fallback_enabled() {
        let static_peers = SharedFederatedPeers::default();
        let directory = directory_with_entry(
            "realm-y",
            "easynet:///r/realm-y/device/d2",
            Some("https://auto.example"),
        );

        let resolver = HubResolver::new(&static_peers, &directory, true);
        let outcome = resolver.resolve("realm-y", "easynet:///r/realm-y/device/d2");

        assert_eq!(
            outcome,
            HubResolution::DirectoryFallback {
                hub_endpoint: "https://auto.example".to_string(),
                target_ura: "easynet:///r/realm-y/device/d2".to_string(),
            }
        );
    }

    #[test]
    fn directory_is_ignored_when_fallback_disabled_even_with_matching_entry() {
        // P0 pin: default-off semantics. The directory carries an
        // entry the resolver could in principle use, but the
        // operator did not opt in — so the resolver must return
        // `Offline` and the caller surfaces `target_offline` to the
        // peer rather than dialing an attacker-controllable URL.
        let static_peers = SharedFederatedPeers::default();
        let directory = directory_with_entry(
            "realm-untrusted",
            "easynet:///r/realm-untrusted/device/d-evil",
            Some("https://attacker.example"),
        );

        let resolver = HubResolver::new(&static_peers, &directory, false);
        let outcome = resolver.resolve(
            "realm-untrusted",
            "easynet:///r/realm-untrusted/device/d-evil",
        );

        assert_eq!(
            outcome,
            HubResolution::Offline,
            "directory_fallback must be opt-in; default-off blocks attacker-controllable endpoints"
        );
    }

    #[test]
    fn static_match_still_works_when_fallback_disabled() {
        // The opt-in only gates source 2 — operator-declared static
        // peers must continue to resolve regardless of the flag.
        let mut peers = BTreeMap::new();
        peers.insert("realm-x".to_string(), "https://primary.example".to_string());
        let static_peers = SharedFederatedPeers::new(peers);
        let directory = SharedFederatedDirectoryView::default();

        let resolver = HubResolver::new(&static_peers, &directory, false);
        let outcome = resolver.resolve("realm-x", "easynet:///r/realm-x/device/anywhere");

        assert_eq!(
            outcome,
            HubResolution::Static {
                hub_endpoint: "https://primary.example".to_string(),
            }
        );
    }

    #[test]
    fn directory_entry_without_endpoint_is_not_a_hit_when_fallback_enabled() {
        let static_peers = SharedFederatedPeers::default();
        let directory = directory_with_entry("realm-z", "easynet:///r/realm-z/device/d3", None);

        let resolver = HubResolver::new(&static_peers, &directory, true);
        let outcome = resolver.resolve("realm-z", "easynet:///r/realm-z/device/d3");

        assert_eq!(outcome, HubResolution::Offline);
    }

    #[test]
    fn both_sources_miss_returns_offline_regardless_of_flag() {
        for allow in [false, true] {
            let static_peers = SharedFederatedPeers::default();
            let directory = SharedFederatedDirectoryView::default();

            let resolver = HubResolver::new(&static_peers, &directory, allow);
            let outcome = resolver.resolve("unknown", "easynet:///r/unknown/device/?");

            assert_eq!(outcome, HubResolution::Offline, "allow={allow}");
        }
    }
}
